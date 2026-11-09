use crate::{
    core::clipboard::ClipboardService,
    mcp::{McpBackend, McpClipboardData, McpHistoryEntry, McpHistorySearch},
    ocr::{OcrPurpose, OcrService},
    storage::{HistoryMetadata, Storage},
};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct Backend {
    pub clipboard: Arc<ClipboardService>,
    pub storage: Arc<Storage>,
    pub ocr: Arc<OcrService>,
}

#[async_trait]
impl McpBackend for Backend {
    async fn clipboard_is_running(&self) -> Result<bool, String> {
        Ok(self.clipboard.is_running())
    }

    async fn clipboard_is_listening(&self) -> Result<bool, String> {
        Ok(self.clipboard.is_listening())
    }

    async fn current_clipboard(&self) -> Result<McpClipboardData, String> {
        let snapshot = self.clipboard.read_current().await?;
        let mut data = Map::new();
        for (key, value) in [
            ("text", snapshot.text),
            ("html", snapshot.html),
            ("rtf", snapshot.rtf),
        ] {
            if let Some(value) = value {
                data.insert(key.to_owned(), Value::String(value));
            }
        }
        if !snapshot.files.is_empty() {
            data.insert("files".to_owned(), json!(snapshot.files));
        }
        Ok(McpClipboardData {
            data: Value::Object(data),
            image: snapshot.image,
        })
    }

    async fn search_history(
        &self,
        search: McpHistorySearch,
    ) -> Result<Vec<McpHistoryEntry>, String> {
        self.storage
            .search_history_metadata(search.search_text.as_deref(), search.limit, search.offset)
            .await
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| history_entry(record, None))
                    .collect()
            })
    }

    async fn history_item(&self, item_id: &str) -> Result<McpHistoryEntry, String> {
        let record = self
            .storage
            .get_history_metadata(item_id)
            .await?
            .ok_or_else(|| "历史记录不存在".to_owned())?;
        let image = match record.image_bytes {
            Some(bytes) if bytes > 0 => Some(
                self.storage
                    .get_clipboard_image(item_id)
                    .await?
                    .ok_or_else(|| "历史图片在读取时已删除".to_owned())?,
            ),
            _ => None,
        };
        Ok(history_entry(record, image))
    }

    async fn set_clipboard_text(&self, text: &str) -> Result<(), String> {
        self.clipboard.write_text(text.to_owned()).await
    }

    async fn recognize_image(
        &self,
        image: Vec<u8>,
        cancellation: CancellationToken,
    ) -> Result<Value, String> {
        let result = self
            .ocr
            .recognize_image(image, OcrPurpose::Mcp, cancellation)
            .await?;
        serde_json::to_value(result).map_err(|error| format!("序列化 OCR 结果失败: {error}"))
    }
}

fn history_entry(record: HistoryMetadata, image: Option<Vec<u8>>) -> McpHistoryEntry {
    let mut data = record.content;
    if image.is_none() {
        if let Some(bytes) = record.image_bytes.filter(|bytes| *bytes > 0) {
            data["image"] = json!({"bytes":bytes});
        }
    }
    McpHistoryEntry {
        fields: record.fields,
        content: McpClipboardData { data, image },
    }
}
