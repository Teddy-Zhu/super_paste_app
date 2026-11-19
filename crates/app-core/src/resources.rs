use std::{collections::HashMap, future::Future, hash::Hash, panic::AssertUnwindSafe, sync::{Arc, Mutex, Weak, atomic::{AtomicU64, Ordering}}};
use futures_util::FutureExt;
use tokio::{runtime::Handle, sync::watch};
use tokio_util::sync::CancellationToken;
use crate::{Error, Result};
mod priority;
pub(crate) use priority::WorkDemand;

#[derive(Clone)]
pub(crate) struct ByteBudget(Arc<BudgetInner>);
struct BudgetInner { limit: u64, used: AtomicU64, name: &'static str }
pub(crate) struct Reservation { inner: Arc<BudgetInner>, bytes: u64 }
impl Reservation {
    pub fn bytes(&self) -> u64 { self.bytes }
    pub fn grow(&mut self, additional: u64) -> Result<()> {
        self.inner.used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(additional).filter(|sum| *sum <= self.inner.limit)
        }).map_err(|_| Error::ResourceLimit(format!("{}预算不足", self.inner.name).into()))?;
        self.bytes += additional;
        Ok(())
    }
    pub fn shrink(&mut self, bytes: u64) {
        if bytes < self.bytes {
            self.inner.used.fetch_sub(self.bytes - bytes, Ordering::AcqRel);
            self.bytes = bytes;
        }
    }
}
impl ByteBudget {
    pub fn new(limit: u64, name: &'static str) -> Self { Self(Arc::new(BudgetInner { limit, used: AtomicU64::new(0), name })) }
    pub fn reserve(&self, bytes: u64) -> Result<Reservation> {
        self.0.used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(bytes).filter(|sum| *sum <= self.0.limit)
        }).map_err(|_| Error::ResourceLimit(format!("{}预算不足", self.0.name).into()))?;
        Ok(Reservation { inner: self.0.clone(), bytes })
    }
    pub fn used(&self) -> u64 { self.0.used.load(Ordering::Acquire) }
    pub fn limit(&self) -> u64 { self.0.limit }
}
impl Drop for Reservation {
    fn drop(&mut self) { self.inner.used.fetch_sub(self.bytes, Ordering::AcqRel); }
}

/// Counts allocated capacity, and retains the lease when transferred into an HTTP body.
pub(crate) struct BoundedWriter {
    data: Vec<u8>, limit: usize, reservation: Reservation, failure: Option<Error>,
    reclaim: Option<Box<dyn Fn() -> Result<()> + Send + Sync>>,
}
impl BoundedWriter {
    pub fn new(reservation: Reservation, limit: u64) -> Result<Self> {
        let limit = usize::try_from(limit).map_err(|_| Error::ResourceLimit("输出数据超过地址空间".into()))?;
        Ok(Self { data: Vec::new(), limit, reservation, failure: None, reclaim: None })
    }
    pub fn with_reclaim(mut self, reclaim: impl Fn() -> Result<()> + Send + Sync + 'static) -> Self { self.reclaim = Some(Box::new(reclaim)); self }
    pub fn into_parts(self) -> Result<(Vec<u8>, Reservation)> {
        if let Some(error) = self.failure { return Err(error); }
        Ok((self.data, self.reservation))
    }
    pub fn into_bytes(self) -> Result<bytes::Bytes> {
        if let Some(error) = self.failure.clone() { return Err(error); }
        Ok(bytes::Bytes::from_owner(self))
    }
    fn grow_required(&mut self, additional: u64) -> Result<()> {
        match self.reservation.grow(additional) {
            Ok(()) => Ok(()),
            Err(error) => match self.reclaim.take() {
                Some(reclaim) => { reclaim()?; self.reservation.grow(additional) },
                None => Err(error),
            },
        }
    }
    fn write_inner(&mut self, bytes: &[u8]) -> Result<usize> {
        if let Some(error) = &self.failure { return Err(error.clone()); }
        let required = self.data.len().checked_add(bytes.len()).filter(|length| *length <= self.limit)
            .ok_or_else(|| Error::ResourceLimit("输出数据超过大小限制".into()))?;
        let capacity = self.data.capacity();
        if required > capacity {
            let preferred = required.max(capacity.saturating_mul(2)).min(self.limit);
            let target = match self.reservation.grow((preferred - capacity) as u64) {
                Ok(()) => preferred,
                Err(_) => { self.grow_required((required - capacity) as u64)?; required }
            };
            if let Err(error) = self.data.try_reserve_exact(target - self.data.len()) {
                self.reservation.shrink(capacity as u64);
                return Err(Error::ResourceLimit(error.to_string().into()));
            }
            let actual = self.data.capacity() as u64;
            if actual > self.reservation.bytes() { self.grow_required(actual - self.reservation.bytes())?; }
        }
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }
}
impl AsRef<[u8]> for BoundedWriter { fn as_ref(&self) -> &[u8] { &self.data } }
impl std::io::Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.write_inner(bytes).map_err(|error| { self.failure = Some(error.clone()); std::io::Error::other(error) })
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

pub(crate) trait Weighted { fn weight(&self) -> u64; }
type SharedResult<V> = Option<Result<Arc<V>>>;
struct Entry<V> { weak: Weak<V>, retained: Option<Arc<V>>, touched: u64, weight: u64 }
struct Flight<V> { sender: watch::Sender<SharedResult<V>>, demand: Option<Arc<WorkDemand>> }
struct State<K, V> {
    entries: HashMap<K, Entry<V>>,
    // The cache must not itself count as a waiting consumer or keep a dead
    // producer's channel open after its runtime has stopped.
    flights: HashMap<K, Weak<Flight<V>>>,
    retained_bytes: u64,
    clock: u64,
    epoch: u64,
}

/// One operation per key, byte-bounded strong retention, and reuse of externally held results.
pub(crate) struct WorkCache<K, V> { state: Mutex<State<K, V>>, capacity: u64 }
impl<K, V> WorkCache<K, V>
where K: Eq + Hash + Clone + Send + Sync + 'static, V: Weighted + Send + Sync + 'static {
    pub fn new(capacity: u64) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State { entries: HashMap::new(), flights: HashMap::new(), retained_bytes: 0, clock: 0, epoch: 0 }), capacity })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State<K, V>>> {
        self.state.lock().map_err(|_| Error::Task("共享资源状态锁已损坏".into()))
    }
    pub fn trim(&self) -> Result<()> {
        let mut state = self.lock()?;
        for entry in state.entries.values_mut() { entry.retained = None; }
        state.retained_bytes = 0;
        state.entries.retain(|_, entry| entry.weak.strong_count() > 0);
        state.epoch = state.epoch.wrapping_add(1);
        Ok(())
    }
    pub fn invalidate_where(&self, mut matches: impl FnMut(&K) -> bool) -> Result<()> {
        let mut state = self.lock()?;
        let keys = state.entries.keys().filter(|key| matches(key)).cloned().collect::<Vec<_>>();
        for key in keys { if let Some(entry) = state.entries.remove(&key) { if entry.retained.is_some() { state.retained_bytes -= entry.weight; } } }
        // Keep shared flights attached, but prevent work started before this
        // invalidation from acquiring a fresh strong retention afterwards.
        state.epoch = state.epoch.wrapping_add(1);
        Ok(())
    }
    pub fn share(&self, key: K, value: Arc<V>) -> Result<Arc<V>> {
        let mut state = self.lock()?;
        state.clock = state.clock.wrapping_add(1);
        let clock = state.clock;
        if let Some(entry) = state.entries.get_mut(&key) {
            if let Some(existing) = entry.weak.upgrade() { entry.touched = clock; return Ok(existing); }
        }
        self.insert(&mut state, key, &value, true);
        Ok(value)
    }
    /// Associate another identity without extending the allocation's lifetime.
    pub fn share_weak(&self, key: K, value: &Arc<V>) -> Result<()> {
        let mut state = self.lock()?;
        self.insert(&mut state, key, value, false);
        Ok(())
    }
    /// Reuse a completed result or join existing work without creating a new
    /// producer. Consumers can avoid reloading a larger upstream resource.
    pub async fn get_existing(&self, key: &K, cancel: &CancellationToken) -> Result<Option<Arc<V>>> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let receiver = {
            let mut state = self.lock()?;
            state.clock = state.clock.wrapping_add(1);
            let clock = state.clock;
            if let Some(entry) = state.entries.get_mut(key) {
                if let Some(value) = entry.weak.upgrade() { entry.touched = clock; return Ok(Some(value)); }
            }
            match state.flights.get(key).and_then(Weak::upgrade) {
                Some(flight) => {
                    if let (Some(parent), Some(child)) = (WorkDemand::current(), &flight.demand) { parent.link(child)?; }
                    Some(flight.sender.subscribe())
                }
                None => None,
            }
        };
        match receiver {
            Some(receiver) => wait_result(receiver, cancel).await.map(Some),
            None => Ok(None),
        }
    }
    fn insert(&self, state: &mut State<K, V>, key: K, value: &Arc<V>, retain: bool) {
        // Direct PNG/pixel associations do not launch a producer. Reclaim dead
        // aliases here so every insertion path has the same lifetime boundary.
        state.entries.retain(|_, entry| entry.weak.strong_count() > 0);
        if let Some(previous) = state.entries.remove(&key) {
            if previous.retained.is_some() { state.retained_bytes -= previous.weight; }
        }
        let weight = value.weight();
        let retain = retain && weight <= self.capacity;
        if retain {
            while state.retained_bytes > self.capacity - weight {
                let oldest = state.entries.iter().filter(|(_, entry)| entry.retained.is_some())
                    .min_by_key(|(_, entry)| entry.touched).map(|(key, _)| key.clone());
                let Some(oldest) = oldest else { break };
                if let Some(entry) = state.entries.get_mut(&oldest) {
                    entry.retained = None;
                    let removed = entry.weight;
                    state.retained_bytes -= removed;
                    if entry.weak.strong_count() == 0 { state.entries.remove(&oldest); }
                }
            }
            state.retained_bytes += weight;
        }
        state.clock = state.clock.wrapping_add(1);
        let touched = state.clock;
        state.entries.insert(key, Entry { weak: Arc::downgrade(value), retained: retain.then(|| value.clone()), touched, weight });
    }
    /// Retain successful results, but stop unfinished work after the last
    /// waiter leaves. The producer token belongs to the shared flight rather
    /// than any one caller; blocking work must keep its own permits and check it.
    pub async fn get_or_load_observed<F, Fut>(self: &Arc<Self>, key: K, runtime: &Handle, cancel: &CancellationToken, load: F) -> Result<Arc<V>>
    where F: FnOnce(CancellationToken) -> Fut + Send + 'static, Fut: Future<Output = Result<V>> + Send + 'static {
        self.load_shared(key, runtime, cancel, true, move || async move {
            let producer_cancel = CancellationToken::new();
            let _cancel_on_drop = producer_cancel.clone().drop_guard();
            load(producer_cancel).await
        }).await
    }
    /// Coalesce transient work while consumers own its result. Success must not
    /// populate the retained LRU (for example, successive animation frames).
    /// The last waiting consumer cancels unfinished transient work; already
    /// running blocking operations still own their buffers until they finish.
    pub async fn get_or_load_weak<F, Fut>(self: &Arc<Self>, key: K, runtime: &Handle, cancel: &CancellationToken, load: F) -> Result<Arc<V>>
    where F: FnOnce() -> Fut + Send + 'static, Fut: Future<Output = Result<V>> + Send + 'static {
        self.load_shared(key, runtime, cancel, false, load).await
    }
    async fn load_shared<F, Fut>(self: &Arc<Self>, key: K, runtime: &Handle, cancel: &CancellationToken, retain_result: bool, load: F) -> Result<Arc<V>>
    where F: FnOnce() -> Fut + Send + 'static, Fut: Future<Output = Result<V>> + Send + 'static {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let mut load = Some(load);
        let receiver = {
            let mut state = self.lock()?;
            state.clock = state.clock.wrapping_add(1);
            let clock = state.clock;
            if let Some(entry) = state.entries.get_mut(&key) {
                if let Some(value) = entry.weak.upgrade() { entry.touched = clock; return Ok(value); }
            }
            let pending = state.flights.get(&key).and_then(Weak::upgrade);
            if let Some(flight) = pending {
                if let (Some(parent), Some(child)) = (WorkDemand::current(), &flight.demand) { parent.link(child)?; }
                flight.sender.subscribe()
            }
            else {
                state.flights.retain(|_, flight| flight.strong_count() > 0);
                if state.flights.len() >= 64 { return Err(Error::ResourceLimit("共享资源等待队列已满".into())); }
                let (sender, receiver) = watch::channel(None);
                let demand = WorkDemand::current();
                let producer = Arc::new(Flight { sender, demand: demand.clone() });
                let flight = Arc::downgrade(&producer);
                state.flights.insert(key.clone(), flight.clone());
                let cache = self.clone();
                let epoch = state.epoch;
                let load = load.take().expect("new shared operation");
                runtime.spawn(async move {
                    let result = {
                        let work = AssertUnwindSafe(WorkDemand::scope(demand, async move { load().await })).catch_unwind();
                        tokio::pin!(work);
                        loop {
                            tokio::select! { biased;
                                _ = producer.sender.closed() => {
                                    // Subscribe and removal use the same lock. A new
                                    // waiter may have arrived after closed() woke.
                                    let abandoned = match cache.lock() {
                                        Ok(mut state) => {
                                            if producer.sender.is_closed() {
                                                if state.flights.get(&key).is_some_and(|current| current.ptr_eq(&flight)) { state.flights.remove(&key); }
                                                true
                                            } else { false }
                                        }
                                        Err(error) => break Err(error),
                                    };
                                    if abandoned { break Err(Error::Cancelled); }
                                }
                                result = &mut work => break result.map_err(|_| Error::Task("共享资源任务意外终止".into())).and_then(|value| value).map(Arc::new),
                            }
                        }
                    };
                    let result = match cache.lock() {
                        Ok(mut state) => {
                            if state.flights.get(&key).is_some_and(|current| current.ptr_eq(&flight)) {
                                state.flights.remove(&key);
                                if let Ok(value) = &result {
                                    let retain = retain_result && state.epoch == epoch;
                                    cache.insert(&mut state, key, value, retain);
                                }
                            }
                            result
                        }
                        Err(error) => Err(error),
                    };
                    producer.sender.send_replace(Some(result));
                });
                receiver
            }
        };
        // A joined producer already owns its input. Do not keep a duplicate
        // payload or its budget leases in the unused loader while awaiting it.
        drop(load);
        wait_result(receiver, cancel).await
    }
}

async fn wait_result<V>(mut receiver: watch::Receiver<SharedResult<V>>, cancel: &CancellationToken) -> Result<Arc<V>> {
    loop {
        if let Some(result) = receiver.borrow_and_update().clone() { return result; }
        tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            changed = receiver.changed() => {
                changed.map_err(|_| Error::Task("共享资源任务未返回结果".into()))?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Value(u8);
    impl Weighted for Value { fn weight(&self) -> u64 { 1 } }

    #[tokio::test]
    async fn direct_registration_reclaims_dead_indices_and_refreshes_lru() {
        let cache = WorkCache::new(2);
        let pinned = cache.share(0_u16, Arc::new(Value(0))).expect("consumer-owned pixels");
        for key in 1..=64_u16 {
            cache.share(key, Arc::new(Value(key as u8))).expect("register next encoding");
        }
        {
            let state = cache.lock().expect("cache metadata");
            assert_eq!(state.retained_bytes, 2);
            assert_eq!(state.entries.len(), 3, "only retained entries and the live external consumer need indices");
        }
        let cancel = CancellationToken::new();
        let reused = cache.get_existing(&0, &cancel).await.expect("pinned lookup").expect("external consumer survives eviction");
        assert!(Arc::ptr_eq(&pinned, &reused));
        drop(reused);
        let reused = cache.share(63, Arc::new(Value(255))).expect("reuse canonical value");
        assert_eq!(reused.0, 63);
        drop(reused);
        cache.share(65, Arc::new(Value(65))).expect("evict least recently used encoding");
        assert!(cache.get_existing(&64, &cancel).await.expect("cold lookup").is_none());
        assert_eq!(cache.get_existing(&63, &cancel).await.expect("hot lookup").expect("hot value retained").0, 63);
        drop(pinned);
        cache.share(66, Arc::new(Value(66))).expect("reclaim departed consumer");
        assert_eq!(cache.lock().expect("reclaimed metadata").entries.len(), 2);

        let aliases = WorkCache::new(0);
        let first = Arc::new(Value(1)); let second = Arc::new(Value(2));
        aliases.share_weak(1, &first).expect("first alias");
        drop(first);
        aliases.share_weak(2, &second).expect("next alias");
        assert_eq!(aliases.lock().expect("alias metadata").entries.len(), 1);
        assert!(Arc::ptr_eq(&second, &aliases.get_existing(&2, &cancel).await.expect("alias lookup").expect("live alias")));
    }

    #[tokio::test]
    async fn completing_producer_replaces_direct_registration_without_evicting_unrelated_data() {
        let cache = WorkCache::new(2); let runtime = Handle::current(); let cancel = CancellationToken::new();
        cache.share(1, Arc::new(Value(1))).expect("older value");
        cache.share(2, Arc::new(Value(2))).expect("unrelated value");
        let (started, running) = tokio::sync::oneshot::channel();
        let release = Arc::new(tokio::sync::Notify::new()); let ready = release.clone();
        let load = cache.get_or_load_observed(3, &runtime, &cancel, move |_| async move {
            let _ = started.send(());
            ready.notified().await;
            Ok(Value(3))
        });
        tokio::pin!(load);
        assert!(futures_util::poll!(load.as_mut()).is_pending());
        running.await.expect("producer started");
        // A normalized PNG can register the same decoded identity while an
        // earlier decoder still owns its accepted work.
        cache.share(3, Arc::new(Value(3))).expect("direct identity association");
        release.notify_one();
        assert_eq!(load.await.expect("producer completion").0, 3);
        assert_eq!(cache.get_existing(&2, &cancel).await.expect("unrelated lookup").expect("unrelated retention preserved").0, 2);
        let state = cache.lock().expect("cache accounting");
        assert_eq!(state.retained_bytes, 2);
        assert_eq!(state.entries.len(), 2);
    }

    #[tokio::test]
    async fn observed_work_keeps_other_waiters_and_retains_success() {
        let cache = WorkCache::new(1); let runtime = Handle::current();
        let first_cancel = CancellationToken::new(); let second_cancel = CancellationToken::new();
        let (started, running) = tokio::sync::oneshot::channel();
        let release = Arc::new(tokio::sync::Notify::new()); let ready = release.clone();
        let first = cache.get_or_load_observed(1, &runtime, &first_cancel, move |producer| async move {
            let _ = started.send(producer);
            ready.notified().await; Ok(Value(31))
        });
        tokio::pin!(first); assert!(futures_util::poll!(first.as_mut()).is_pending());
        let producer = running.await.expect("shared producer started");
        let second = cache.get_or_load_observed(1, &runtime, &second_cancel, |_| async { panic!("joined waiter must not start another producer") });
        tokio::pin!(second); assert!(futures_util::poll!(second.as_mut()).is_pending());
        first_cancel.cancel(); assert!(matches!(first.await, Err(Error::Cancelled)));
        assert!(!producer.is_cancelled(), "one departed waiter must not cancel other consumers");
        release.notify_one(); let result = second.await.expect("remaining waiter");
        assert_eq!(result.0, 31); drop(result);
        assert_eq!(cache.get_existing(&1, &second_cancel).await.expect("retained lookup").expect("successful result retained").0, 31);
    }

    #[tokio::test]
    async fn observed_work_cancels_last_waiter_and_allows_same_key_restart() {
        let cache = WorkCache::new(1); let runtime = Handle::current(); let cancel = CancellationToken::new();
        let (started, running) = tokio::sync::oneshot::channel();
        let first = cache.get_or_load_observed(1, &runtime, &cancel, move |producer| async move {
            let _ = started.send(producer);
            std::future::pending::<Result<Value>>().await
        });
        tokio::pin!(first); assert!(futures_util::poll!(first.as_mut()).is_pending());
        let producer = running.await.expect("shared producer started");
        cancel.cancel(); assert!(matches!(first.await, Err(Error::Cancelled)));
        tokio::time::timeout(std::time::Duration::from_secs(5), producer.cancelled()).await.expect("last waiter cancels producer");
        let result = cache.get_or_load_observed(1, &runtime, &CancellationToken::new(), |_| async { Ok(Value(47)) }).await.expect("same key can restart after abandonment");
        assert_eq!(result.0, 47);
    }

    #[tokio::test]
    async fn joined_ingress_drops_unused_payload_before_waiting_for_shared_work() {
        let cache = WorkCache::new(1);
        let budget = ByteBudget::new(64, "duplicate ingress");
        let ready = Arc::new(tokio::sync::Notify::new());
        let producer = ready.clone();
        let runtime = Handle::current();
        let cancel = CancellationToken::new();
        let first = cache.get_or_load_observed(1, &runtime, &cancel, move |_| async move {
            producer.notified().await;
            Ok(Value(17))
        });
        tokio::pin!(first);
        assert!(futures_util::poll!(first.as_mut()).is_pending());

        let duplicate = budget.reserve(64).expect("duplicate input lease");
        let joined = cache.get_or_load_observed(1, &runtime, &cancel, move |_| async move {
            drop(duplicate);
            Ok(Value(99))
        });
        tokio::pin!(joined);
        assert!(futures_util::poll!(joined.as_mut()).is_pending());
        assert_eq!(budget.used(), 0, "unused producer must release its input before the accepted producer finishes");
        ready.notify_one();
        let first = first.await.expect("accepted producer");
        let joined = joined.await.expect("joined producer");
        assert_eq!(first.0, 17);
        assert!(Arc::ptr_eq(&first, &joined));
    }
}
