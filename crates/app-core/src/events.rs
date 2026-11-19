use std::sync::Arc;
use super_paste_format::{ConfigSnapshot, RecordId, TagId};
use tokio::sync::broadcast;
use crate::Error;

#[derive(Clone, Debug)]
pub enum RecordChange { Inserted, Metadata, Ocr { revision: u64 }, Removed }
#[derive(Clone, Debug)]
pub enum OcrTaskState { Queued, Running, Completed, Cancelled, Failed(Error) }
#[derive(Clone, Debug)]
pub struct OcrTaskStatus {
    pub record: RecordId, pub generation: u64, pub state: OcrTaskState,
    pub expires_at: Option<std::time::Instant>,
}
#[derive(Clone, Debug)]
pub enum AppEvent {
    ClipboardFailed(Error),
    SourceTrackingFailed(Error),
    HistoryChanged { ids: Arc<[RecordId]>, change: RecordChange },
    TagsChanged { id: Option<TagId> },
    ConfigurationCommitted(ConfigSnapshot),
    CredentialCleanupFailed(Error),
    CopyStatisticsChanged,
    AiUsageChanged,
    AiUsageFailed(Error),
    OcrTask(OcrTaskStatus),
    OcrModelChanged(Arc<crate::ocr::catalog::OcrModelStatus>),
    OcrServiceFailed(Error),
    SyncStatusChanged(Arc<super_paste_format::SyncStatus>),
    SyncFinished { manual: bool, result: crate::Result<Arc<super_paste_format::SyncOutcome>> },
    McpStatusChanged(Arc<super_paste_format::McpStatus>),
}
/// A bounded notification channel; consumers recover a snapshot after `Lagged`.
#[derive(Clone)]
pub struct EventBus { sender: broadcast::Sender<AppEvent> }
impl EventBus {
    pub fn new(capacity: usize) -> crate::Result<Self> {
        if capacity == 0 { return Err(Error::Invalid("事件队列容量必须大于零".into())); }
        let (sender, _) = broadcast::channel(capacity);
        Ok(Self { sender })
    }
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> { self.sender.subscribe() }
    pub fn publish(&self, event: AppEvent) {
        // No listeners is normal while windows are hidden. Authoritative state lives in services.
        let _ = self.sender.send(event);
    }
}

