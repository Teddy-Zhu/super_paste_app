use clipboard_rs::common::RustImage;
use clipboard_rs::{Clipboard, ClipboardContext};

use crate::error::AppError;
use crate::result::Result;
use crate::types::clipboard::{ClipboardContent, ClipboardItem};
use crate::utils::{hash, time};

pub struct ClipboardConverter;

impl ClipboardConverter {
    /// 读取系统剪贴板并转换为可入库的条目与内容。
    pub fn read_and_convert(
        source_app_id: Option<String>,
    ) -> Result<(ClipboardItem, ClipboardContent)> {
        let ctx = ClipboardContext::new().map_err(|e| AppError::clipboard(e.to_string()))?;
        let content_id = hash::generate_content_id();

        // 读取文本（空字符串视为无文本）
        let text = ctx.get_text().ok().filter(|t| !t.is_empty());

        // 读取图片并编码为 PNG
        let image_bytes: Option<Vec<u8>> = ctx
            .get_image()
            .ok()
            .and_then(|img| img.to_png().ok())
            .map(|buf| buf.get_bytes().to_vec());

        // 读取 HTML（可选）
        let html = ctx.get_rich_text().ok().filter(|t| !t.is_empty());

        // 确定主类型与哈希源
        let (primary_type, hash_src): (&str, Vec<u8>) = if let Some(t) = text.as_ref() {
            ("text", t.as_bytes().to_vec())
        } else if let Some(bytes) = image_bytes.as_ref() {
            ("image", bytes.clone())
        } else {
            ("unknown", Vec::new())
        };

        let content_hash = hash::hash_content(&hash_src);
        let data_size = hash_src.len() as i64;
        let now = time::now();

        let mut available_formats = vec![primary_type.to_string()];
        if html.is_some() && primary_type == "text" {
            available_formats.push("html".to_string());
        }

        let item = ClipboardItem {
            id: None,
            content_id: content_id.clone(),
            content_hash,
            primary_type: primary_type.to_string(),
            data_size,
            available_formats,
            source_app_id,
            is_pinned: false,
            is_favorite: false,
            created_at: now,
            last_used_at: now,
            use_count: 1,
        };

        let content = ClipboardContent {
            content_id,
            text_content: if primary_type == "text" { text } else { None },
            html_content: if primary_type == "text" { html } else { None },
            rtf_content: None,
            image_data: if primary_type == "image" {
                image_bytes
            } else {
                None
            },
            files_json: None,
            metadata_json: None,
            compression_type: Some("none".to_string()),
        };

        Ok((item, content))
    }
}
