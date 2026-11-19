use std::{future::Future, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}}};
use crate::{Error, Result};

tokio::task_local! { static CURRENT: Arc<WorkDemand>; }

/// Priority follows live work dependencies, independently of image/cache keys.
/// Promotion is monotonic for one flight. Weak edges retain no producer, input
/// or completed result, and cannot create ownership cycles.
pub(crate) struct WorkDemand {
    interactive: AtomicBool,
    children: Mutex<Vec<Weak<WorkDemand>>>,
}
impl WorkDemand {
    pub fn new(interactive: bool) -> Arc<Self> {
        Arc::new(Self { interactive: AtomicBool::new(interactive), children: Mutex::new(Vec::new()) })
    }
    pub fn current() -> Option<Arc<Self>> { CURRENT.try_with(Arc::clone).ok() }
    pub fn interactive(&self) -> bool { self.interactive.load(Ordering::Acquire) }
    pub fn link(self: &Arc<Self>, child: &Arc<Self>) -> Result<()> {
        if Arc::ptr_eq(self, child) { return Ok(()); }
        if self.interactive() { return child.promote(); }
        let mut children = self.children.lock().map_err(|_| Error::Task("共享任务优先级状态不可用".into()))?;
        children.retain(|entry| entry.strong_count() > 0);
        let edge = Arc::downgrade(child);
        if !children.iter().any(|entry| entry.ptr_eq(&edge)) { children.push(edge); }
        // Promotion may have raced with edge insertion. Its traversal either
        // sees the child, or this recheck propagates the already-latched demand.
        let promoted = self.interactive();
        drop(children);
        if promoted { child.promote()?; }
        Ok(())
    }
    fn promote(self: &Arc<Self>) -> Result<()> {
        let mut pending = vec![self.clone()];
        while let Some(demand) = pending.pop() {
            if demand.interactive.swap(true, Ordering::AcqRel) { continue; }
            let children = demand.children.lock().map_err(|_| Error::Task("共享任务优先级状态不可用".into()))?;
            pending.extend(children.iter().filter_map(Weak::upgrade));
        }
        Ok(())
    }
    pub async fn scope<F: Future>(demand: Option<Arc<Self>>, future: F) -> F::Output {
        match demand { Some(demand) => CURRENT.scope(demand, future).await, None => future.await }
    }
}
