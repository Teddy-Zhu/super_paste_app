use std::sync::Arc;
use crate::{Digest, Formats, PrimaryType, RecordId, SourceId, TagId, TextContent};

#[derive(Clone, Debug)]
pub struct SourceApp { pub id: SourceId, pub application: Arc<str>, pub path: Arc<str>, pub icon: Option<bytes::Bytes> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub id: TagId, pub name: Arc<str>, pub color: Arc<str>, pub description: Option<Arc<str>>,
    pub created_at: i64, pub updated_at: i64, pub count: u64,
}

#[derive(Clone, Debug)]
pub struct ImageMetadata { pub id: Digest, pub encoded_bytes: u64, pub width: Option<u32>, pub height: Option<u32> }

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub id: RecordId,
    pub content_revision: u64,
    pub metadata_revision: u64,
    pub ocr_revision: u64,
    pub primary_type: PrimaryType,
    pub formats: Formats,
    pub timestamp: i64,
    pub last_used: i64,
    pub source: Option<Arc<SourceApp>>,
    pub tags: Arc<[Tag]>,
    pub image: Option<ImageMetadata>,
    pub excerpt: TextContent,
    /// Whether the complete OCR result contains text under the UI's whitespace rules.
    pub has_ocr: bool,
    pub text_characters: u64,
    pub files_count: u64,
    pub truncated: Formats,
}

#[derive(Clone, Debug)]
pub struct FullContent {
    pub id: RecordId, pub revision: u64, pub ocr_revision: u64,
    pub content: TextContent, pub ocr: Option<Arc<str>>, pub image: Option<ImageMetadata>,
}
/// A consistent metadata and full-content snapshot for details and protocol consumers.
#[derive(Clone, Debug)]
pub struct HistoryRecord { pub entry: HistoryEntry, pub content: FullContent }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryOrder { #[default] LastUsed, Timestamp }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCursor { pub value: i64, pub id: RecordId }
#[derive(Clone, Debug)]
pub struct HistoryQuery {
    pub search: String, pub tag_ids: Vec<TagId>, pub primary_type: Option<PrimaryType>,
    pub cursor: Option<HistoryCursor>, pub limit: u32, pub order: HistoryOrder,
}
impl Default for HistoryQuery {
    fn default() -> Self { Self { search: String::new(), tag_ids: Vec::new(), primary_type: None, cursor: None, limit: 24, order: HistoryOrder::LastUsed } }
}
#[derive(Clone, Debug)]
pub struct HistoryPage { pub entries: Vec<Arc<HistoryEntry>>, pub next_cursor: Option<HistoryCursor> }
#[derive(Clone, Copy, Debug)]
pub struct HistoryLimits { pub max_records: i64, pub keep_days: i64 }

