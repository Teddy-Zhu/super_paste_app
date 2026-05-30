use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InternalEvent {
    ClipboardChanged {
        content_id: String,
        primary_type: String,
    },
    OcrCompleted {
        content_id: String,
        text: String,
    },
    SyncStarted,
    SyncCompleted {
        merged_count: usize,
    },
    SyncFailed {
        error: String,
    },
}

impl InternalEvent {
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ClipboardChanged { .. } => "clipboard:changed",
            Self::OcrCompleted { .. } => "ocr:completed",
            Self::SyncStarted => "sync:started",
            Self::SyncCompleted { .. } => "sync:completed",
            Self::SyncFailed { .. } => "sync:failed",
        }
    }
}
