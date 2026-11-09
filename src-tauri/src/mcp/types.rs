use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub const MCP_HOST: &str = "127.0.0.1";
pub const MCP_PATH: &str = "/mcp";
pub const MCP_DEFAULT_PORT: u16 = 19528;
pub const MCP_MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub const MCP_CLIPBOARD_RESOURCE_URI: &str = "super-paste://clipboard/current";
pub const MCP_CLIPBOARD_EVENT_META_KEY: &str = "super-paste/clipboard";

pub const MCP_MAX_BODY_BYTES: usize = 15 * 1024 * 1024;
pub const MCP_MAX_REQUESTS: usize = 32;
pub const MCP_MAX_SESSIONS: usize = 16;
pub const MCP_SESSION_IDLE_SECS: u64 = 5 * 60;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    pub auth_enabled: bool,
    pub api_key: String,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: MCP_DEFAULT_PORT,
            auth_enabled: true,
            api_key: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub running: bool,
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpState {
    pub settings: McpSettings,
    pub status: McpStatus,
}

/// `data` contains the serialized clipboard fields except for image bytes.
/// The optional image is the original encoded image, before MCP PNG conversion.
#[derive(Clone, Debug)]
pub struct McpClipboardData {
    pub data: Value,
    pub image: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct McpHistoryEntry {
    /// Record metadata, with its `content` member omitted.
    pub fields: Value,
    pub content: McpClipboardData,
}

#[derive(Clone, Debug)]
pub struct McpClipboardChange {
    pub timestamp: f64,
    /// Clipboard text formats and source metadata; images contain byte counts only.
    pub content: Value,
}

#[derive(Clone, Debug)]
pub struct McpHistorySearch {
    pub search_text: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

#[async_trait]
pub trait McpBackend: Send + Sync + 'static {
    async fn clipboard_is_running(&self) -> Result<bool, String>;
    async fn clipboard_is_listening(&self) -> Result<bool, String>;
    async fn current_clipboard(&self) -> Result<McpClipboardData, String>;
    async fn search_history(
        &self,
        search: McpHistorySearch,
    ) -> Result<Vec<McpHistoryEntry>, String>;
    async fn history_item(&self, item_id: &str) -> Result<McpHistoryEntry, String>;
    async fn set_clipboard_text(&self, text: &str) -> Result<(), String>;
    async fn recognize_image(
        &self,
        image: Vec<u8>,
        cancellation: CancellationToken,
    ) -> Result<Value, String>;
}
