use std::sync::{Arc, Mutex};
use super_paste_format::{AiUsageQuery, AiUsageStats, CopyStats, CopyStatsQuery};
use tokio::{runtime::Handle, sync::Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use crate::{Error, Result, events::{AppEvent, EventBus}, now_ms, storage::Storage};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatisticsKind { Copy, Ai }

pub struct StatisticsService {
    storage: Arc<Storage>, events: EventBus, runtime: Handle,
    accepting: Mutex<bool>, pending: Arc<Semaphore>, tasks: TaskTracker,
}
impl StatisticsService {
    pub fn new(storage: Arc<Storage>, events: EventBus, runtime: Handle) -> Arc<Self> {
        Arc::new(Self { storage, events, runtime, accepting: Mutex::new(true), pending: Arc::new(Semaphore::new(4)), tasks: TaskTracker::new() })
    }
    pub async fn copy(&self, query: CopyStatsQuery, cancel: &CancellationToken) -> Result<CopyStats> { self.storage.copy_stats(query, now_ms()?, cancel).await }
    pub async fn ai(&self, query: AiUsageQuery, cancel: &CancellationToken) -> Result<AiUsageStats> { self.storage.ai_usage(query, cancel).await }
    /// Accepted clears belong to the service, so dismissing a page cannot abandon a write.
    pub async fn clear(self: &Arc<Self>, kind: StatisticsKind) -> Result<()> {
        let task = {
            let accepting = self.accepting.lock().map_err(|_| Error::Task("统计接入锁损坏".into()))?;
            if !*accepting { return Err(Error::Cancelled); }
            let permit = self.pending.clone().try_acquire_owned().map_err(|error| match error {
                tokio::sync::TryAcquireError::Closed => Error::Cancelled,
                tokio::sync::TryAcquireError::NoPermits => Error::ResourceLimit("统计操作队列已满".into()),
            })?;
            let service = self.clone();
            self.tasks.spawn_on(async move {
                let _permit = permit;
                let result = match kind {
                    StatisticsKind::Copy => service.storage.clear_copy_stats(now_ms()?).await,
                    StatisticsKind::Ai => service.storage.clear_ai_usage(now_ms()?).await,
                };
                match &result {
                    Ok(()) => service.events.publish(match kind { StatisticsKind::Copy => AppEvent::CopyStatisticsChanged, StatisticsKind::Ai => AppEvent::AiUsageChanged }),
                    Err(error) => tracing::warn!(%error, "清空统计失败"),
                }
                result
            }, &self.runtime)
        };
        task.await?
    }
    pub async fn shutdown(&self) -> Result<()> {
        let failure = match self.accepting.lock() {
            Ok(mut accepting) => { *accepting = false; None }
            Err(poisoned) => { *poisoned.into_inner() = false; Some(Error::Task("统计接入锁损坏".into())) }
        };
        self.pending.close(); self.tasks.close(); self.tasks.wait().await;
        failure.map_or(Ok(()), Err)
    }
}
