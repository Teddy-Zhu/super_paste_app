use std::{collections::VecDeque, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}}};
use tokio::sync::Notify;
use crate::{Error, Result, resources::WorkDemand};

pub(super) struct WorkerPool { state: Mutex<State>, idle: Notify }
struct State { available: usize, closed: bool, waiting: VecDeque<Weak<Ticket>> }
struct Ticket { demand: Option<Arc<WorkDemand>>, granted: AtomicBool, ready: Notify }
pub(super) struct WorkerPermit { pool: Arc<WorkerPool>, ticket: Arc<Ticket> }

impl WorkerPool {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State { available: capacity, closed: false, waiting: VecDeque::new() }), idle: Notify::new() })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| Error::Task("图片任务队列状态不可用".into()))
    }
    // Assign capacity at release time, using the flight's current demand. A
    // joined detail reader can promote an existing queued dependency without
    // cancelling/restarting it or disturbing FIFO order within each class.
    fn dispatch(state: &mut State) -> Vec<Arc<Ticket>> {
        let mut ready = Vec::new();
        state.waiting.retain(|ticket| ticket.strong_count() > 0);
        if state.closed {
            return state.waiting.drain(..).filter_map(|ticket| ticket.upgrade()).collect();
        }
        while state.available > 0 && !state.waiting.is_empty() {
            let index = state.waiting.iter().position(|ticket| ticket.upgrade()
                .is_some_and(|ticket| ticket.demand.as_ref().is_some_and(|demand| demand.interactive()))).unwrap_or(0);
            let Some(ticket) = state.waiting.remove(index).and_then(|ticket| ticket.upgrade()) else { continue; };
            state.available -= 1;
            ticket.granted.store(true, Ordering::Release);
            ready.push(ticket);
        }
        ready
    }
    fn wake(ready: Vec<Arc<Ticket>>) { for ticket in ready { ticket.ready.notify_one(); } }
    pub async fn acquire_owned(self: Arc<Self>) -> Result<WorkerPermit> {
        let ticket = Arc::new(Ticket { demand: WorkDemand::current(), granted: AtomicBool::new(false), ready: Notify::new() });
        let permit = WorkerPermit { pool: self.clone(), ticket: ticket.clone() };
        let ready = {
            let mut state = self.lock()?;
            if state.closed { return Err(Error::Cancelled); }
            state.waiting.push_back(Arc::downgrade(&ticket));
            Self::dispatch(&mut state)
        };
        Self::wake(ready);
        loop {
            let ready = ticket.ready.notified();
            if self.lock()?.closed { return Err(Error::Cancelled); }
            if ticket.granted.load(Ordering::Acquire) { return Ok(permit); }
            ready.await;
        }
    }
    pub fn try_idle(self: &Arc<Self>) -> Result<Option<WorkerPermit>> {
        let mut state = self.lock()?;
        if state.closed { return Err(Error::Cancelled); }
        if state.available == 0 || !state.waiting.is_empty() { return Ok(None); }
        state.available -= 1;
        Ok(Some(WorkerPermit { pool: self.clone(), ticket: Arc::new(Ticket {
            demand: None, granted: AtomicBool::new(true), ready: Notify::new(),
        }) }))
    }
    pub fn idle_notification(&self) -> tokio::sync::futures::Notified<'_> { self.idle.notified() }
    pub fn close(&self) -> Result<()> {
        let mut state = self.lock()?;
        state.closed = true;
        let ready = Self::dispatch(&mut state);
        drop(state);
        Self::wake(ready);
        self.idle.notify_waiters();
        Ok(())
    }
    #[cfg(test)]
    pub fn available_permits(&self) -> usize { self.state.lock().expect("worker pool state").available }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        // Drop cannot return an error. On poison, close and wake the queue;
        // subsequent acquisition reports the poisoned state through lock().
        let mut state = match self.pool.state.lock() {
            Ok(state) => state,
            Err(poisoned) => { let mut state = poisoned.into_inner(); state.closed = true; state },
        };
        if self.ticket.granted.swap(false, Ordering::AcqRel) { state.available += 1; }
        else {
            let cancelled = Arc::downgrade(&self.ticket);
            state.waiting.retain(|ticket| !ticket.ptr_eq(&cancelled));
        }
        let ready = WorkerPool::dispatch(&mut state);
        let idle = state.available > 0;
        drop(state);
        WorkerPool::wake(ready);
        if idle { self.pool.idle.notify_one(); }
    }
}
