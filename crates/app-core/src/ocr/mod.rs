pub mod catalog;
mod models;
mod layout;
mod local;
mod remote;
mod wire;
mod ai;
pub use ai::{AiService, AiRequestConfig};
mod service;
mod statuses;
pub use service::{OcrService, OcrRecognition, OcrTrigger, OcrTaskHandle, OcrDiagnostics};
pub use statuses::{TASK_STATUS_RETENTION, MAX_RECENT_TASK_STATUSES};
pub(crate) use service::CapturedImage;
pub use models::{InstalledModel, ModelManager, ModelsDiagnostics};

#[derive(Clone, Copy, Debug)]
pub struct OcrTicket {
    pub record: super_paste_format::RecordId,
    pub generation: u64,
    pub content_revision: u64,
    pub configuration_revision: u64,
}
pub(crate) enum OcrAdmission { Started(OcrTicket), ConfigurationChanged, Missing }
