use std::{cell::RefCell, collections::BTreeMap, future::Future, panic::AssertUnwindSafe, str::FromStr, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}}};
use futures_util::FutureExt;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use super_paste_core::{Error, Result};
use super_paste_format::{ShortcutState, ShortcutStatus};
use tokio::{runtime::Handle, sync::{Mutex as AsyncMutex, OwnedMutexGuard, Semaphore, oneshot, watch}, task::JoinHandle};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;
use crate::main_thread::MainThread;
#[cfg(target_os = "linux")]
mod portal;

type Activate = Arc<dyn Fn(Option<String>) + Send + Sync>;
#[derive(Clone)]
struct PortalCommand { generation: u64, accelerator: String }
#[derive(Clone)]
pub struct ShortcutSnapshot { generation: u64, pub status: Arc<ShortcutStatus> }
#[derive(Default)]
struct Active { id: AtomicU64, down: AtomicBool }
impl Active {
    fn set(&self, id: Option<u32>) { self.down.store(false, Ordering::Release); self.id.store(id.map_or(0, |id| u64::from(id) + 1), Ordering::Release); }
}
pub struct ShortcutService {
    suppressed: Arc<AtomicUsize>,
    id: Uuid, main: MainThread, runtime: Handle, activate: Activate, active: Arc<Active>,
    state: watch::Sender<ShortcutSnapshot>, gate: Arc<AsyncMutex<()>>,
    portal: Option<watch::Sender<Option<PortalCommand>>>, portal_task: Mutex<Option<JoinHandle<Result<()>>>>,
    stop: CancellationToken, accepting: Mutex<bool>, permits: Arc<Semaphore>, tasks: TaskTracker,
}
pub struct ActivationSuppression {
    count: Arc<AtomicUsize>,
    #[cfg(target_os = "windows")]
    service: std::sync::Weak<ShortcutService>,
    // Windows registration belongs to the GUI thread. A guard cannot migrate
    // to a worker thread and restore registration from there on drop.
    _gui_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for ActivationSuppression {
    fn drop(&mut self) {
        let previous = self.count.fetch_sub(1, Ordering::AcqRel);
        #[cfg(target_os = "windows")]
        if previous == 1 {
            if let Some(service) = self.service.upgrade().filter(|service| !service.stop.is_cancelled()) {
                match service.synchronize_native_capture() {
                    Ok(Some(registered)) => {
                        let accelerator = service.snapshot().accelerator.clone();
                        service.publish(&accelerator, if registered { ShortcutState::Registered } else { ShortcutState::Disabled }, None);
                    }
                    Ok(None) => {},
                    Err(error) => service.publish_failure(&format!("恢复全局快捷键失败：{error}")),
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        let _ = previous;
    }
}
pub struct PreparedShortcutChange {
    service: Arc<ShortcutService>, _guard: OwnedMutexGuard<()>, before: Arc<ShortcutStatus>, next: String, changed: bool, attempted: bool,
}
impl ShortcutService {
    /// Global shortcuts follow the desktop session, including GPUI windows
    /// using XWayland inside a Wayland session.
    pub fn new(main: MainThread, runtime: Handle, activate: Activate) -> Arc<Self> {
        let suppressed = Arc::new(AtomicUsize::new(0)); let count = suppressed.clone();
        let activate: Activate = Arc::new(move |token| { if count.load(Ordering::Acquire) == 0 { activate(token); } });
        let (state, _) = watch::channel(ShortcutSnapshot { generation: 0, status: Arc::new(ShortcutStatus { accelerator: "".into(), state: ShortcutState::Disabled, error: None }) });
        #[cfg(target_os = "linux")]
        let portal = if clipboard_core::is_wayland_session() { Some(watch::channel(None).0) } else { None };
        #[cfg(not(target_os = "linux"))]
        let portal = None;
        let service = Arc::new(Self { suppressed, id: Uuid::new_v4(), main, runtime, activate, active: Arc::new(Active::default()), state, gate: Arc::new(AsyncMutex::new(())),
            portal, portal_task: Mutex::new(None), stop: CancellationToken::new(), accepting: Mutex::new(true), permits: Arc::new(Semaphore::new(4)), tasks: TaskTracker::new() });
        #[cfg(target_os = "linux")]
        service.start_portal_worker();
        service
    }
    #[cfg(target_os = "linux")]
    fn start_portal_worker(self: &Arc<Self>) {
        if let Some(portal) = &self.portal {
            // Subscribe with the current value already seen. Recovery must
            // process the next command, not replay a superseded accelerator.
            let commands = portal.subscribe();
            let weak = Arc::downgrade(self); let stop = self.stop.clone(); let activate = self.activate.clone(); let state = self.state.clone();
            let task = self.runtime.spawn(async move {
                let result = AssertUnwindSafe(portal::worker(commands, stop.clone(), activate, state)).catch_unwind().await
                    .map_err(|_| Error::Task("快捷键 portal 任务异常退出".into())).and_then(|result| result.map_err(|error| Error::Platform(error.into())));
                if !stop.is_cancelled() {
                    if let Some(service) = weak.upgrade() { service.publish_failure(&result.as_ref().err().map(ToString::to_string).unwrap_or_else(|| "快捷键 portal 任务已停止".into())); }
                }
                result
            });
            *self.portal_task.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(task);
        }
    }
    /// Called while the configuration/retry gate is held. Join the old worker
    /// before replacement so its final failure cannot overwrite the new state.
    #[cfg(target_os = "linux")]
    async fn ensure_portal_worker(self: &Arc<Self>) -> Result<()> {
        self.check_open()?;
        let Some(portal) = &self.portal else { return Ok(()); };
        let previous = {
            let mut task = self.portal_task.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if !portal.is_closed() && task.as_ref().is_some_and(|task| !task.is_finished()) { return Ok(()); }
            task.take()
        };
        if let Some(previous) = previous {
            if let Err(error) = previous.await { tracing::warn!(%error, "快捷键 portal 任务收尾失败"); }
        }
        self.check_open()?;
        self.start_portal_worker();
        Ok(())
    }
    pub fn snapshot(&self) -> Arc<ShortcutStatus> { self.state.borrow().status.clone() }
    pub fn uses_portal(&self) -> bool { self.portal.is_some() }
    pub fn suppress_activation(self: &Arc<Self>) -> Result<ActivationSuppression> {
        self.check_open()?;
        if !self.main.is_current_thread() { return Err(Error::Task("快捷键录制只能在桌面线程启动".into())); }
        self.suppressed.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| count.checked_add(1)).map_err(|_| Error::ResourceLimit("快捷键录制会话过多".into()))?;
        let guard = ActivationSuppression { count: self.suppressed.clone(), _gui_thread: std::marker::PhantomData,
            #[cfg(target_os = "windows")]
            service: Arc::downgrade(self),
        };
        #[cfg(target_os = "windows")]
        self.synchronize_native_capture()?;
        Ok(guard)
    }
    #[cfg(target_os = "windows")]
    fn synchronize_native_capture(&self) -> Result<Option<bool>> {
        NATIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(native) = slot.as_mut() else { return Ok(None); };
            if native.owner != self.id { return Err(Error::Conflict("快捷键宿主已变化".into())); }
            native.set_suspended(self.suppressed.load(Ordering::Acquire) > 0)?;
            Ok(Some(native.desired.is_some()))
        })
    }
    pub fn subscribe(&self) -> watch::Receiver<ShortcutSnapshot> { self.state.subscribe() }
    fn check_open(&self) -> Result<()> { if self.stop.is_cancelled() { Err(Error::Cancelled) } else { Ok(()) } }
    fn publish(&self, accelerator: &str, status: ShortcutState, error: Option<String>) -> u64 {
        let value = Arc::new(ShortcutStatus { accelerator: accelerator.into(), state: status, error: error.map(Into::into) });
        let mut generation = 0;
        self.state.send_modify(|state| { state.generation = state.generation.wrapping_add(1); generation = state.generation; state.status = value; });
        generation
    }
    fn publish_failure(&self, error: &str) {
        tracing::error!(%error, "全局快捷键不可用");
        let accelerator = self.snapshot().accelerator.clone(); self.publish(&accelerator, ShortcutState::Failed, Some(error.to_owned()));
    }
    pub async fn prepare(self: &Arc<Self>, accelerator: &str) -> Result<PreparedShortcutChange> {
        self.check_open()?;
        if !accelerator.is_empty() { HotKey::from_str(accelerator).map_err(|error| Error::Invalid(format!("快捷键格式无效：{error}").into()))?; }
        let guard = self.gate.clone().lock_owned().await; self.check_open()?;
        let before = self.snapshot();
        let healthy = matches!(before.state, ShortcutState::Disabled | ShortcutState::Registered | ShortcutState::PortalRequested)
            && (self.portal.is_none() || self.portal_task.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().is_some_and(|task| !task.is_finished()));
        let changed = before.accelerator.as_ref() != accelerator || !healthy;
        Ok(PreparedShortcutChange { service: self.clone(), _guard: guard, before, next: accelerator.into(), changed, attempted: false })
    }
    async fn apply_value(self: &Arc<Self>, accelerator: &str) -> Result<()> {
        if let Some(portal) = &self.portal {
            // A worker can exit between the liveness check and send. Recover
            // that race once; permission denial remains an explicit status.
            for _ in 0..2 {
                #[cfg(target_os = "linux")]
                self.ensure_portal_worker().await?;
                let generation = self.publish(accelerator, if accelerator.is_empty() { ShortcutState::Disabled } else { ShortcutState::PortalRequested }, None);
                if portal.send(Some(PortalCommand { generation, accelerator: accelerator.into() })).is_ok() { return Ok(()); }
            }
            self.publish(accelerator, ShortcutState::Failed, Some("快捷键 portal 任务已关闭".into()));
            return Err(Error::Platform("快捷键 portal 任务已关闭".into()));
        }
        let next = if accelerator.is_empty() { None } else { Some(HotKey::from_str(accelerator).map_err(|error| Error::Invalid(error.to_string().into()))?) };
        let id = self.id; let active = self.active.clone(); let activate = self.activate.clone(); let stop = self.stop.clone();
        let service = self.clone(); let accelerator = accelerator.to_owned();
        // Publish on the same thread as the native mutation. A recorder's
        // synchronous drop must not be overwritten by an older async reply.
        let result = self.on_main(move || {
            let result = NATIVE.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.is_none() && next.is_none() { return Ok(()); }
                if slot.is_none() {
                    let suspended = cfg!(target_os = "windows") && service.suppressed.load(Ordering::Acquire) > 0;
                    *slot = Some(Native::new(id, active, activate, stop, suspended)?);
                }
                let native = slot.as_mut().ok_or_else(|| Error::Task("快捷键宿主未初始化".into()))?;
                if native.owner != id { return Err(Error::Conflict("已有其他快捷键宿主".into())); }
                native.replace(next)
            });
            match &result {
                Ok(()) => { service.publish(&accelerator, if next.is_some() { ShortcutState::Registered } else { ShortcutState::Disabled }, None); },
                Err(error) => { let previous = service.snapshot().accelerator.clone(); service.publish(&previous, ShortcutState::Failed, Some(error.to_string())); },
            }
            Ok(result)
        }).await;
        match result {
            Ok(result) => result,
            Err(error) => { self.publish_failure(&error.to_string()); Err(error) },
        }
    }
    /// A retry acquires the same gate as configuration effects, then verifies
    /// the captured configuration version without awaiting another service.
    pub async fn reconcile<F>(self: &Arc<Self>, accelerator: String, verify: F) -> Result<Arc<ShortcutStatus>>
    where F: Fn() -> Result<()> + Send + Sync + 'static {
        self.owned(move |service| async move {
            let result = async {
                verify()?;
                let mut change = service.prepare(&accelerator).await?; verify()?;
                if let Err(error) = change.apply().await { return Err(match change.rollback().await { Ok(()) => error, Err(rollback) => Error::Platform(format!("{error}；快捷键回滚失败：{rollback}").into()) }); }
                Ok(service.snapshot())
            }.await;
            if let Err(error) = &result { if !matches!(error, Error::Cancelled | Error::Conflict(_)) {
                let _guard = service.gate.lock().await;
                if verify().is_ok() && !service.stop.is_cancelled() {
                    service.state.send_modify(|snapshot| {
                        let status = &snapshot.status;
                        snapshot.status = Arc::new(ShortcutStatus { accelerator: status.accelerator.clone(), state: if matches!(status.state, ShortcutState::Registered | ShortcutState::PortalRequested) { status.state } else { ShortcutState::Failed }, error: Some(error.to_string().into()) });
                    });
                }
            } }
            result
        }).await
    }
    async fn on_main<T: Send + 'static>(&self, operation: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
        let (sender, receiver) = oneshot::channel();
        self.main.run_on_main_thread(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(operation)).unwrap_or_else(|_| Err(Error::Task("快捷键主线程任务异常退出".into())));
            let _ = sender.send(result);
        })?;
        receiver.await.map_err(|_| Error::Task("快捷键主线程任务未完成".into()))?
    }
    async fn owned<T, F, Fut>(self: &Arc<Self>, operation: F) -> Result<T>
    where T: Clone + Send + Sync + 'static, F: FnOnce(Arc<Self>) -> Fut + Send + 'static, Fut: Future<Output = Result<T>> + Send + 'static {
        let mut receiver = {
            let accepting = self.accepting.lock().map_err(|_| Error::Task("快捷键操作接入锁损坏".into()))?;
            if !*accepting { return Err(Error::Cancelled); }
            let permit = self.permits.clone().try_acquire_owned().map_err(|_| Error::ResourceLimit("快捷键操作过多，请稍后重试".into()))?;
            let (sender, receiver) = watch::channel(None); let service = self.clone();
            self.tasks.spawn_on(async move {
                let _permit = permit;
                let result = AssertUnwindSafe(operation(service)).catch_unwind().await.unwrap_or_else(|_| Err(Error::Task("快捷键操作异常退出".into())));
                if let Err(error) = &result { if !matches!(error, Error::Cancelled) { tracing::warn!(%error, "快捷键操作失败"); } }
                sender.send_replace(Some(result));
            }, &self.runtime);
            receiver
        };
        loop { if let Some(result) = receiver.borrow_and_update().clone() { return result; } receiver.changed().await.map_err(|_| Error::Task("快捷键操作结果通道关闭".into()))?; }
    }
    pub async fn shutdown(&self) -> Result<()> {
        *self.accepting.lock().map_err(|_| Error::Task("快捷键操作接入锁损坏".into()))? = false;
        self.stop.cancel(); self.active.set(None); self.tasks.close();
        let guard = self.gate.lock().await;
        let task = self.portal_task.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        let mut errors = Vec::new();
        if let Some(task) = task { match task.await { Ok(Ok(())) => {}, Ok(Err(error)) => errors.push(error.to_string()), Err(error) => errors.push(error.to_string()) } }
        if self.portal.is_none() {
            let id = self.id;
            if let Err(error) = self.on_main(move || NATIVE.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.as_ref().is_some_and(|native| native.owner != id) { return Err(Error::Conflict("快捷键宿主已变化".into())); }
                let Some(mut native) = slot.take() else { return Ok(()); };
                native.replace(None)
            })).await { errors.push(error.to_string()); }
        }
        self.publish("", ShortcutState::Disabled, if errors.is_empty() { None } else { Some(errors.join("；")) });
        drop(guard); self.tasks.wait().await;
        if errors.is_empty() { Ok(()) } else { Err(Error::Platform(errors.join("；").into())) }
    }
}
impl Drop for ShortcutService { fn drop(&mut self) { self.stop.cancel(); self.active.set(None); } }
impl PreparedShortcutChange {
    pub async fn apply(&mut self) -> Result<()> { if self.changed { self.attempted = true; self.service.apply_value(&self.next).await?; } Ok(()) }
    pub async fn rollback(&mut self) -> Result<()> { if self.attempted { self.service.apply_value(&self.before.accelerator).await?; } Ok(()) }
}
#[cfg(target_os = "linux")]
fn set_portal_state(state: &watch::Sender<ShortcutSnapshot>, command: &PortalCommand, status: ShortcutState, error: Option<String>) -> bool {
    state.send_if_modified(|snapshot| {
        if snapshot.generation != command.generation { return false; }
        snapshot.status = Arc::new(ShortcutStatus { accelerator: command.accelerator.as_str().into(), state: status, error: error.map(Into::into) }); true
    })
}

struct Native {
    owner: Uuid, manager: GlobalHotKeyManager, registered: BTreeMap<u32, HotKey>, active: Arc<Active>,
    desired: Option<HotKey>, suspended: bool,
}
thread_local! { static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) }; }
impl Native {
    fn new(owner: Uuid, active: Arc<Active>, activate: Activate, stop: CancellationToken, suspended: bool) -> Result<Self> {
        let manager = GlobalHotKeyManager::new().map_err(|error| Error::Platform(format!("初始化快捷键宿主失败：{error}").into()))?;
        let target = active.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if stop.is_cancelled() || target.id.load(Ordering::Acquire) != u64::from(event.id) + 1 { return; }
            match event.state { HotKeyState::Released => target.down.store(false, Ordering::Release), HotKeyState::Pressed => { if !target.down.swap(true, Ordering::AcqRel) { activate(None); } } }
        }));
        Ok(Self { owner, manager, registered: BTreeMap::new(), active, desired: None, suspended })
    }
    #[cfg(target_os = "windows")]
    fn set_suspended(&mut self, suspended: bool) -> Result<()> {
        self.suspended = suspended;
        if suspended {
            self.active.set(None);
            // Do not try to register a previously failed binding just to
            // release it. Recording must allow choosing an alternative key.
            self.retire(None)
        } else { self.replace(self.desired) }
    }
    fn replace(&mut self, target: Option<HotKey>) -> Result<()> {
        let previous = self.desired.map_or(0, |key| u64::from(key.id()) + 1);
        if let Some(key) = target { if !self.registered.contains_key(&key.id()) {
            self.manager.register(key).map_err(|error| Error::Platform(format!("注册快捷键失败：{error}").into()))?; self.registered.insert(key.id(), key);
        } }
        // During recording validate a new binding with the OS, then release
        // it as well. The recorder keeps receiving its physical down/up events.
        match self.retire(if self.suspended { None } else { target }) {
            Ok(()) => { self.desired = target; self.active.set(if self.suspended { None } else { target.map(|key| key.id()) }); Ok(()) }
            Err(error) => {
                if self.suspended || previous == 0 || !self.registered.keys().any(|id| u64::from(*id) + 1 == previous) { self.active.set(None); }
                Err(error)
            }
        }
    }
    fn retire(&mut self, keep: Option<HotKey>) -> Result<()> {
        let retired = self.registered.values().copied().filter(|key| keep.is_none_or(|keep| keep.id() != key.id())).collect::<Vec<_>>();
        let mut errors = Vec::new();
        for key in retired { match self.manager.unregister(key) { Ok(()) => { self.registered.remove(&key.id()); }, Err(error) => errors.push(format!("注销快捷键失败：{error}")) } }
        if errors.is_empty() { Ok(()) } else { Err(Error::Platform(errors.join("；").into())) }
    }
}
