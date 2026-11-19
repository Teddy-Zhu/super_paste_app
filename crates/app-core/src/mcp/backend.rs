use std::sync::Arc;
use super_paste_format::{RecordId, TextContent};
use tokio_util::sync::CancellationToken;
use crate::{clipboard::{ClipboardService, ClipboardSnapshot}, images::{ImageService, ImageSource}, ocr::{OcrService, OcrRecognition}, storage::Storage};
use super::types::{McpClipboardData, McpHistoryEntry, McpHistorySearch};

pub(super) struct Backend {
    pub clipboard: Arc<ClipboardService>, pub storage: Arc<Storage>, pub images: Arc<ImageService>, pub ocr: Arc<OcrService>,
    pub shutdown: CancellationToken,
}
impl Backend {
    pub async fn clipboard_is_running(&self) -> Result<bool, String> { Ok(!self.shutdown.is_cancelled()) }
    pub async fn clipboard_is_listening(&self) -> Result<bool, String> { Ok(!self.shutdown.is_cancelled() && self.clipboard.is_listening()) }
    pub async fn current_clipboard(&self, cancel: &CancellationToken) -> Result<McpClipboardData, String> {
        let snapshot = self.clipboard.read(cancel).await.map_err(error)?;
        Ok(McpClipboardData { content: snapshot.content, ocr: None, image: snapshot.image, image_bytes: None })
    }
    pub async fn search_history(&self, search: McpHistorySearch, cancel: &CancellationToken) -> Result<Vec<McpHistoryEntry>, String> {
        self.storage.search_records(search.search_text.unwrap_or_default(), search.limit, search.offset, cancel).await.map_err(error)
            .map(|records| records.into_iter().map(history_entry).collect())
    }
    pub async fn history_item(&self, item_id: &str, cancel: &CancellationToken) -> Result<McpHistoryEntry, String> {
        self.storage.record(parse_id(item_id)?, cancel).await.map(history_entry).map_err(error)
    }
    pub async fn history_image(&self, item_id: &str, cancel: &CancellationToken) -> Result<ImageSource, String> {
        self.storage.image_metadata(parse_id(item_id)?, cancel).await.map_err(error)?
            .map(ImageSource::Stored).ok_or_else(|| "剪贴板内容不包含图片".into())
    }
    pub async fn set_clipboard_text(&self, text: &str, cancel: &CancellationToken) -> Result<(), String> {
        if cancel.is_cancelled() { return Err("请求已取消".into()); }
        self.clipboard.write(ClipboardSnapshot { content: TextContent { text: Some(text.into()), ..Default::default() }, image: None }, cancel).await.map_err(error)
    }
    pub async fn recognize_image(&self, image: super::codec::ValidatedImage, cancellation: CancellationToken) -> Result<Arc<OcrRecognition>, String> {
        let result = self.ocr.recognize_image(image.original.clone(), cancellation).await.map_err(error);
        // Keep validated pixels alive through recognition, including buffers too
        // large for the retained cache. Local OCR obtains this same allocation.
        drop(image);
        result
    }
}
fn parse_id(value: &str) -> Result<RecordId, String> { value.parse().map_err(|_| "剪贴板历史记录不存在".into()) }
fn history_entry(record: super_paste_format::HistoryRecord) -> McpHistoryEntry {
    let image_bytes = record.content.image.as_ref().map(|image| image.encoded_bytes);
    McpHistoryEntry { metadata: record.entry, content: McpClipboardData { content: record.content.content, ocr: record.content.ocr,
        image: record.content.image.map(ImageSource::Stored), image_bytes } }
}
fn error(error: crate::Error) -> String { if matches!(error, crate::Error::NotFound) { "剪贴板历史记录不存在".into() } else { error.to_string() } }
