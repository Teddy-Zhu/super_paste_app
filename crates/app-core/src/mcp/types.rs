use std::{ops::Deref, sync::Arc};
use super_paste_format::{HistoryEntry, McpSettings, TextContent};
use crate::{clipboard::ClipboardChange, credentials::CredentialReference, images::ImageSource};
pub(super) use super_paste_format::{MCP_HOST, MCP_PATH, MCP_MAX_IMAGE_BYTES, MCP_CLIPBOARD_RESOURCE_URI, MCP_CLIPBOARD_EVENT_META_KEY, MCP_MAX_BODY_BYTES, MCP_MAX_REQUESTS, MCP_MAX_SESSIONS, MCP_SESSION_IDLE_SECS};

#[derive(Clone)]
pub(super) struct SessionCancellation(pub tokio_util::sync::CancellationToken);

/// The HTTP server retains an authentication digest, not a second plaintext key.
#[derive(Clone)]
pub(super) struct RuntimeSettings {
    pub value: McpSettings,
    pub reference: Option<CredentialReference>,
    pub authorization_digest: Option<[u8; 32]>,
}
impl Deref for RuntimeSettings { type Target = McpSettings; fn deref(&self) -> &Self::Target { &self.value } }
impl RuntimeSettings {
    pub fn same_identity(&self, other: &Self) -> bool { self.value == other.value && self.reference == other.reference }
}
pub(super) struct McpClipboardData {
    pub content: TextContent, pub ocr: Option<Arc<str>>,
    pub image: Option<ImageSource>, pub image_bytes: Option<u64>,
}
pub(super) struct McpHistoryEntry { pub metadata: HistoryEntry, pub content: McpClipboardData }
pub(super) struct McpHistorySearch { pub search_text: Option<String>, pub limit: u32, pub offset: u32 }
#[derive(Clone)]
pub(super) struct McpChange { pub cursor: String, pub change: Arc<ClipboardChange> }
