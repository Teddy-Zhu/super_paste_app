use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: Option<i64>,
    pub content_id: String,
    pub content_hash: String,
    pub primary_type: String,
    pub data_size: i64,
    pub available_formats: Vec<String>,
    pub source_app_id: Option<String>,
    pub is_pinned: bool,
    pub is_favorite: bool,
    pub created_at: i64,
    pub last_used_at: i64,
    pub use_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardContent {
    pub content_id: String,
    pub text_content: Option<String>,
    pub html_content: Option<String>,
    pub rtf_content: Option<String>,
    pub image_data: Option<Vec<u8>>,
    pub files_json: Option<String>,
    pub metadata_json: Option<String>,
    pub compression_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub bundle_id: Option<String>,
    pub executable_path: String,
    pub icon_hash: Option<String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryOptions {
    pub offset: i64,
    pub limit: i64,
    pub primary_type: Option<String>,
    pub tag_id: Option<String>,
    pub is_pinned: Option<bool>,
    pub is_favorite: Option<bool>,
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 50,
            primary_type: None,
            tag_id: None,
            is_pinned: None,
            is_favorite: None,
            start_date: None,
            end_date: None,
        }
    }
}
