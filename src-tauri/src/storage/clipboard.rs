use crate::core::clipboard_content::plain_text;
use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};
use std::{collections::HashMap, io::Cursor};

use super::{
    Storage, now_ms,
    tags::{get_record_tags, record_tags_sync},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertClipboard {
    pub id: String,
    pub digest: String,
    pub is_new: bool,
    pub last_used: i64,
}

#[derive(Debug, Clone)]
pub struct HistoryMetadata {
    pub fields: Value,
    pub content: Value,
    pub image_bytes: Option<usize>,
}

#[derive(Debug)]
struct HistoryMetadataRow {
    id: String,
    content_text: Option<String>,
    content_html: Option<String>,
    content_rtf: Option<String>,
    content_files_json: Option<String>,
    content_ocr: Option<String>,
    primary_type: String,
    available_formats_json: String,
    timestamp: i64,
    last_used: i64,
    source_app: Option<String>,
    image_bytes: Option<i64>,
}

impl HistoryMetadataRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("content_id")?,
            content_text: row.get("content_text")?,
            content_html: row.get("content_html")?,
            content_rtf: row.get("content_rtf")?,
            content_files_json: row.get("content_files_json")?,
            content_ocr: row.get("content_ocr")?,
            primary_type: row.get("primary_type")?,
            available_formats_json: row.get("available_formats_json")?,
            timestamp: row.get("timestamp")?,
            last_used: row.get("last_used")?,
            source_app: row.get("source_app")?,
            image_bytes: row.get("image_bytes")?,
        })
    }

    fn into_metadata(self, connection: &rusqlite::Connection) -> rusqlite::Result<HistoryMetadata> {
        let mut content = serde_json::Map::new();
        if let Some(value) = self.content_text {
            content.insert("text".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_html {
            content.insert("html".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_rtf {
            content.insert("rtf".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_files_json {
            content.insert(
                "files".to_owned(),
                serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery)?,
            );
        }
        if let Some(value) = self.content_ocr {
            content.insert("ocr".to_owned(), Value::String(value));
        }
        let formats: Value = serde_json::from_str(&self.available_formats_json)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut fields = json!({
            "id": self.id,
            "primaryType": self.primary_type,
            "availableFormats": formats,
            "timestamp": self.timestamp,
            "lastUsed": self.last_used,
            "tags": record_tags_sync(connection, &self.id)?
        });
        if let Some(source) = self.source_app {
            fields["source_app"] = Value::String(source);
        }
        let image_bytes = self
            .image_bytes
            .map(|value| usize::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery))
            .transpose()?;
        Ok(HistoryMetadata {
            fields,
            content: Value::Object(content),
            image_bytes,
        })
    }
}

#[derive(Debug)]
struct HistoryRow {
    id: String,
    digest: String,
    content_text: Option<String>,
    content_html: Option<String>,
    content_rtf: Option<String>,
    content_files_json: Option<String>,
    content_ocr: Option<String>,
    primary_type: String,
    available_formats_json: String,
    timestamp: i64,
    last_used: i64,
    source_app: Option<String>,
    updated_at: i64,
    raw_image_bytes: Option<i64>,
}

impl HistoryRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("content_id")?,
            digest: row.get("digest")?,
            content_text: row.get("content_text")?,
            content_html: row.get("content_html")?,
            content_rtf: row.get("content_rtf")?,
            content_files_json: row.get("content_files_json")?,
            content_ocr: row.get("content_ocr")?,
            primary_type: row.get("primary_type")?,
            available_formats_json: row.get("available_formats_json")?,
            timestamp: row.get("timestamp")?,
            last_used: row.get("last_used")?,
            source_app: row.get("source_app")?,
            updated_at: row.get("updated_at")?,
            raw_image_bytes: row.get("raw_image_bytes")?,
        })
    }

    fn into_dto(self, tags: Vec<Value>, image_reference: Option<String>) -> Result<Value, String> {
        let mut content = serde_json::Map::new();
        let image_bytes = self
            .raw_image_bytes
            .and_then(|size| usize::try_from(size).ok());
        let mut files_value = self
            .content_files_json
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|error| format!("文件列表格式无效: {error}"))?;
        let files_count = files_value
            .as_ref()
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if let Some(image_reference) = image_reference {
            content.insert("image".to_owned(), Value::String(image_reference));
        }
        if let Some(value) = self.content_text {
            content.insert("text".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_html {
            content.insert("html".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_rtf {
            content.insert("rtf".to_owned(), Value::String(value));
        }
        if let Some(files) = files_value.take() {
            content.insert("files".to_owned(), files);
        }
        if let Some(value) = self.content_ocr {
            content.insert("ocr".to_owned(), Value::String(value));
        }
        let available_formats: Value = serde_json::from_str(&self.available_formats_json)
            .map_err(|error| format!("剪贴板格式列表无效: {error}"))?;
        let mut dto = json!({
            "id": self.id,
            "content": Value::Object(content),
            "primaryType": self.primary_type,
            "availableFormats": available_formats,
            "timestamp": self.timestamp,
            "lastUsed": self.last_used,
            "contentVersion": self.updated_at,
            "hasImage": self.raw_image_bytes.is_some(),
            "imageBytes": image_bytes,
            "filesCount": files_count,
            "tags": tags
        });
        if let Some(source) = self.source_app {
            dto["source_app"] = Value::String(source);
        }
        Ok(dto)
    }
}

struct PreviewCache {
    entries: HashMap<(String, String), Vec<u8>>,
    order: std::collections::VecDeque<(String, String)>,
    bytes: usize,
}

static PREVIEW_CACHE: std::sync::OnceLock<std::sync::Mutex<PreviewCache>> =
    std::sync::OnceLock::new();
static PREVIEW_SEMAPHORE: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();

fn preview_semaphore() -> std::sync::Arc<tokio::sync::Semaphore> {
    PREVIEW_SEMAPHORE
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(3)))
        .clone()
}

fn cached_preview(id: &str, digest: &str) -> Option<Vec<u8>> {
    PREVIEW_CACHE
        .get()?
        .lock()
        .ok()?
        .entries
        .get(&(id.to_owned(), digest.to_owned()))
        .cloned()
}

fn cache_preview(id: String, digest: String, image: Vec<u8>) {
    const MAX_ENTRIES: usize = 32;
    const MAX_BYTES: usize = 16 * 1024 * 1024;
    if image.len() > MAX_BYTES {
        return;
    }
    let cache = PREVIEW_CACHE.get_or_init(|| {
        std::sync::Mutex::new(PreviewCache {
            entries: HashMap::new(),
            order: std::collections::VecDeque::new(),
            bytes: 0,
        })
    });
    let Ok(mut cache) = cache.lock() else { return };
    let key = (id, digest);
    if let Some(previous) = cache.entries.remove(&key) {
        cache.bytes -= previous.len();
    }
    cache.order.retain(|entry| entry != &key);
    cache.bytes += image.len();
    cache.entries.insert(key.clone(), image);
    cache.order.push_back(key);
    while cache.order.len() > MAX_ENTRIES || cache.bytes > MAX_BYTES {
        if let Some(oldest) = cache.order.pop_front() {
            if let Some(previous) = cache.entries.remove(&oldest) {
                cache.bytes -= previous.len();
            }
        }
    }
}

fn forget_preview_id(id: &str) {
    let Some(cache) = PREVIEW_CACHE.get() else {
        return;
    };
    let Ok(mut cache) = cache.lock() else { return };
    let keys = cache
        .order
        .iter()
        .filter(|key| key.0 == id)
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        cache.order.retain(|entry| entry != &key);
        if let Some(image) = cache.entries.remove(&key) {
            cache.bytes -= image.len();
        }
    }
}

fn clear_preview_cache() {
    let Some(cache) = PREVIEW_CACHE.get() else {
        return;
    };
    let Ok(mut cache) = cache.lock() else { return };
    cache.entries.clear();
    cache.order.clear();
    cache.bytes = 0;
}

fn make_preview(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use image::{ImageFormat, ImageReader, Limits};
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(65_536);
    limits.max_image_height = Some(65_536);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("图片预览解码失败: {error}"))?;
    let preview = decoded.thumbnail(640, 640);
    let mut output = Cursor::new(Vec::new());
    preview
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(output.into_inner())
}

#[derive(Debug)]
struct HistoryListRow {
    id: String,
    digest: String,
    content_text: Option<String>,
    content_html: Option<String>,
    content_rtf: Option<String>,
    content_files_json: Option<String>,
    content_ocr: Option<String>,
    primary_type: String,
    available_formats_json: String,
    timestamp: i64,
    last_used: i64,
    source_app: Option<String>,
    updated_at: i64,
    text_length: Option<i64>,
    text_truncated: bool,
    html_truncated: bool,
    rtf_truncated: bool,
    ocr_truncated: bool,
    has_image: bool,
    image_bytes: Option<i64>,
}

impl HistoryListRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("content_id")?,
            digest: row.get("digest")?,
            content_text: row.get("content_text")?,
            content_html: row.get("content_html")?,
            content_rtf: row.get("content_rtf")?,
            content_files_json: row.get("content_files_json")?,
            content_ocr: row.get("content_ocr")?,
            primary_type: row.get("primary_type")?,
            available_formats_json: row.get("available_formats_json")?,
            timestamp: row.get("timestamp")?,
            last_used: row.get("last_used")?,
            source_app: row.get("source_app")?,
            updated_at: row.get("updated_at")?,
            text_length: row.get("text_length")?,
            text_truncated: row.get("text_truncated")?,
            html_truncated: row.get("html_truncated")?,
            rtf_truncated: row.get("rtf_truncated")?,
            ocr_truncated: row.get("ocr_truncated")?,
            has_image: row.get("has_image")?,
            image_bytes: row.get("image_bytes")?,
        })
    }
}

struct HistoryPageData {
    records: Vec<HistoryListRow>,
    tags_by_record: HashMap<String, Vec<Value>>,
    apps: serde_json::Map<String, Value>,
    known_versions: HashMap<String, i64>,
    has_more: bool,
    next_cursor: Option<Value>,
}

pub(super) async fn upsert_clipboard_with_image(
    storage: &Storage,
    item: &Value,
    image: Option<Vec<u8>>,
) -> Result<UpsertClipboard, String> {
    let item = item.clone();
    let image = match image {
        Some(image) => Some(image),
        None => item
            .get("content")
            .and_then(Value::as_object)
            .and_then(|content| content.get("image"))
            .filter(|value| !value.is_null())
            .map(data_format::image_bytes)
            .transpose()?,
    };
    let content_for_digest = item
        .get("content")
        .cloned()
        .ok_or_else(|| "剪贴板内容无效".to_owned())?;
    let (digest, image) = tokio::task::spawn_blocking(move || {
        let digest = data_format::content_digest_with_image(&content_for_digest, image.as_deref())?;
        Ok::<_, String>((digest, image))
    })
    .await
    .map_err(|error| error.to_string())??;
    storage
        .call(move |connection| {
            let content = item
                .get("content")
                .and_then(Value::as_object)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            let content_value = Value::Object(content.clone());
            let content_text = content.get("text").and_then(Value::as_str);
            let content_html = content.get("html").and_then(Value::as_str);
            let content_rtf = content.get("rtf").and_then(Value::as_str);
            let content_ocr = content.get("ocr").and_then(Value::as_str);
            let content_image = image;
            let content_files = content.get("files").filter(|value| !value.is_null());
            if content_text.is_none()
                && content_html.is_none()
                && content_rtf.is_none()
                && content_image.is_none()
                && content_files.and_then(Value::as_array).map_or(true, Vec::is_empty)
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let now = now_ms();
            let last_used = item.get("lastUsed").and_then(Value::as_i64).unwrap_or(now);
            let source_app = item.get("source_app").and_then(Value::as_str);
            if let Some((existing, existing_last_used)) = connection
                .query_row(
                    "SELECT content_id,last_used FROM clipboard_history WHERE digest=?1",
                    [&digest],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?
            {
                connection.execute(
                    "UPDATE clipboard_history SET last_used=MAX(last_used,?1),source_app=COALESCE(?2,source_app),updated_at=?3 WHERE content_id=?4",
                    params![last_used, source_app, now, existing],
                )?;
                return Ok(UpsertClipboard { id: existing, digest, is_new: false, last_used: last_used.max(existing_last_used) });
            }

            let id = item
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let timestamp = item.get("timestamp").and_then(Value::as_i64).unwrap_or(now);
            let primary_type = item
                .get("primaryType")
                .and_then(Value::as_str)
                .unwrap_or("empty");
            let formats = item
                .get("availableFormats")
                .filter(|value| value.is_array())
                .cloned()
                .unwrap_or_else(|| infer_available_formats(&content_value, content_image.is_some()));
            let files_json = content_files
                .map(serde_json::to_string)
                .transpose()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            connection.execute(
                "INSERT INTO clipboard_history(content_id,digest,content_text,content_html,content_image,content_rtf,content_files_json,content_ocr,primary_type,available_formats_json,timestamp,last_used,source_app,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14)",
                params![
                    id,
                    digest,
                    content_text,
                    content_html,
                    content_image,
                    content_rtf,
                    files_json,
                    content_ocr,
                    primary_type,
                    serde_json::to_string(&formats).map_err(|_| rusqlite::Error::InvalidQuery)?,
                    timestamp,
                    last_used,
                    source_app,
                    now
                ],
            )?;
            Ok(UpsertClipboard { id, digest, is_new: true, last_used })
        })
        .await
}

fn infer_available_formats(content: &Value, has_image: bool) -> Value {
    Value::Array(
        ["text", "html", "rtf", "files"]
            .into_iter()
            .filter(|key| {
                content.get(*key).is_some_and(|value| match *key {
                    "files" => value.as_array().is_some_and(|files| !files.is_empty()),
                    _ => value.as_str().is_some_and(|value| !value.is_empty()),
                })
            })
            .chain(has_image.then_some("image"))
            .map(|format| Value::String(format.to_owned()))
            .collect(),
    )
}

pub(super) async fn get_clipboard_snapshot(
    storage: &Storage,
    id: &str,
    text_only: bool,
    ocr_only: bool,
) -> Result<Option<clipboard_core::ClipboardSnapshot>, String> {
    let id = id.to_owned();
    let record = storage.call(move |connection| {
        if ocr_only {
            return connection.query_row(
                "SELECT content_ocr FROM clipboard_history WHERE content_id=?1",
                [id],
                |row| row.get::<_, Option<String>>(0),
            ).optional().map(|value| value.map(|ocr| (None, None, None, None, None, ocr)));
        }
        connection.query_row(
            if text_only { "SELECT content_text,content_html,NULL,content_rtf,content_files_json,NULL FROM clipboard_history WHERE content_id=?1" } else { "SELECT content_text,content_html,content_image,content_rtf,content_files_json,NULL FROM clipboard_history WHERE content_id=?1" },
            [id],
            |row| {
                let text: Option<String> = row.get(0)?;
                let html: Option<String> = row.get(1)?;
                let image: Option<Vec<u8>> = if text_only { None } else { row.get(2)? };
                let rtf: Option<String> = row.get(3)?;
                let files: Option<String> = row.get(4)?;
                Ok((text, html, image, rtf, files, None))
            },
        ).optional()
    }).await?;
    let Some((text, html, image, rtf, files, ocr)) = record else {
        return Ok(None);
    };
    if ocr_only {
        return ocr
            .filter(|value| !value.is_empty())
            .map(|text| {
                Some(clipboard_core::ClipboardSnapshot {
                    text: Some(text),
                    ..Default::default()
                })
            })
            .ok_or_else(|| "该记录没有 OCR 文本".into());
    }
    let files = files
        .map(|value| serde_json::from_str::<Vec<String>>(&value))
        .transpose()
        .map_err(|error| format!("文件列表格式无效: {error}"))?
        .unwrap_or_default();
    let snapshot = clipboard_core::ClipboardSnapshot {
        text,
        html,
        image,
        rtf,
        files,
    };
    if text_only {
        let text = plain_text(&snapshot)?;
        return Ok(Some(clipboard_core::ClipboardSnapshot {
            text: Some(text),
            ..Default::default()
        }));
    }
    Ok(Some(snapshot))
}

pub(super) async fn get_clipboard_item(
    storage: &Storage,
    id: &str,
) -> Result<Option<Value>, String> {
    let id_owned = id.to_owned();
    let row = storage.call(move |connection| {
        connection.query_row("SELECT content_id,digest,content_text,content_html,content_rtf,content_files_json,content_ocr,primary_type,available_formats_json,timestamp,last_used,source_app,updated_at,length(content_image) AS raw_image_bytes FROM clipboard_history WHERE content_id=?1", [id_owned], HistoryRow::read).optional()
    }).await?;
    let Some(row) = row else { return Ok(None) };
    let tags = get_record_tags(storage, &row.id).await?;
    let image_reference = if row.raw_image_bytes.is_some() {
        Some(media_reference("original", &row.id, &row.digest))
    } else {
        None
    };
    let item = tokio::task::spawn_blocking(move || row.into_dto(tags, image_reference))
        .await
        .map_err(|error| error.to_string())??;
    Ok(Some(item))
}

fn media_reference(kind: &str, id: &str, digest: &str) -> String {
    format!("{kind}/{id}/{digest}")
}

pub(super) async fn get_clipboard_media(
    storage: &Storage,
    kind: &str,
    id: &str,
    digest: &str,
) -> Result<Option<Vec<u8>>, String> {
    if !matches!(kind, "preview" | "original") {
        return Err("剪贴板图片类型无效".to_owned());
    }
    if kind == "preview" {
        if let Some(preview) = cached_preview(id, digest) {
            return Ok(Some(preview));
        }
    }
    let permit = if kind == "preview" {
        Some(
            preview_semaphore()
                .acquire_owned()
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let (kind, id, digest) = (kind.to_owned(), id.to_owned(), digest.to_owned());
    let (cache_id, cache_digest) = (id.clone(), digest.clone());
    let loaded = storage.call(move |connection| {
        let exists = connection.query_row(
            "SELECT 1 FROM clipboard_history WHERE content_id=?1 AND digest=?2 AND content_image IS NOT NULL",
            params![id, digest],
            |_| Ok(()),
        ).optional()?.is_some();
        if !exists { return Ok(None); }
        if kind == "preview" {
            if let Some(preview) = cached_preview(&id, &digest) {
                return Ok(Some((false, preview)));
            }
        }
        let image = connection.query_row(
            "SELECT content_image FROM clipboard_history WHERE content_id=?1 AND digest=?2",
            params![id, digest],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        Ok(Some((kind == "preview", image)))
    }).await?;
    let Some((needs_preview, bytes)) = loaded else {
        return Ok(None);
    };
    if !needs_preview {
        return Ok(Some(bytes));
    }
    let permit = permit.ok_or_else(|| "图片预览并发许可不可用".to_owned())?;
    let (permit, preview) = tokio::task::spawn_blocking(move || (permit, make_preview(&bytes)))
        .await
        .map_err(|error| error.to_string())?;
    let preview = preview?;
    let cached_bytes = preview.clone();
    let still_exists = storage.call(move |connection| {
        let exists = connection.query_row(
            "SELECT 1 FROM clipboard_history WHERE content_id=?1 AND digest=?2 AND content_image IS NOT NULL",
            params![cache_id, cache_digest],
            |_| Ok(()),
        ).optional()?.is_some();
        if exists { cache_preview(cache_id, cache_digest, cached_bytes); }
        Ok(exists)
    }).await?;
    drop(permit);
    if !still_exists {
        return Ok(None);
    }
    Ok(Some(preview))
}

pub(super) async fn get_clipboard_image(
    storage: &Storage,
    id: &str,
) -> Result<Option<Vec<u8>>, String> {
    let id = id.to_owned();
    storage
        .call(move |connection| {
            connection
                .query_row(
                    "SELECT content_image FROM clipboard_history WHERE content_id=?1",
                    [id],
                    |row| row.get::<_, Option<Vec<u8>>>(0),
                )
                .optional()
                .map(Option::flatten)
        })
        .await
}

pub(super) async fn update_ocr(storage: &Storage, id: &str, text: &str) -> Result<bool, String> {
    let (id, text) = (id.to_owned(), text.to_owned());
    storage
        .call(move |connection| {
            Ok(connection.execute(
                "UPDATE clipboard_history SET content_ocr=?1,updated_at=?2 WHERE content_id=?3",
                params![text, now_ms(), id],
            )? > 0)
        })
        .await
}

pub(super) async fn query_history(storage: &Storage, options: &Value) -> Result<Value, String> {
    let options = options.clone();
    let mut page = storage.call(move |connection| {
        let limit = options.get("limit").and_then(Value::as_u64).unwrap_or(24).clamp(1, 100) as usize;
        let known_versions = options.get("knownVersions").and_then(Value::as_object).map(|known| {
            known.iter().filter_map(|(id, version)| version.as_i64().map(|version| (id.clone(), version))).collect::<HashMap<_, _>>()
        }).unwrap_or_default();
        let mut query_options = options.clone();
        query_options["limit"] = json!(limit + 1);
        let projection = "ch.content_id,ch.digest,substr(ch.content_text,1,2048) AS content_text,CASE WHEN length(ch.content_html)<=16384 THEN ch.content_html END AS content_html,CASE WHEN length(ch.content_rtf)<=16384 THEN ch.content_rtf END AS content_rtf,ch.content_files_json,substr(ch.content_ocr,1,8192) AS content_ocr,ch.primary_type,ch.available_formats_json,ch.timestamp,ch.last_used,ch.source_app,ch.updated_at,length(ch.content_text) AS text_length,COALESCE(length(ch.content_text)>2048,0) AS text_truncated,COALESCE(length(ch.content_html)>16384,0) AS html_truncated,COALESCE(length(ch.content_rtf)>16384,0) AS rtf_truncated,COALESCE(length(ch.content_ocr)>8192,0) AS ocr_truncated,(ch.content_image IS NOT NULL) AS has_image,length(ch.content_image) AS image_bytes";
        let (sql, values) = build_history_sql(&query_options, projection, None)?;
        let records = {
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(values), HistoryListRow::read)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let has_more = records.len() > limit;
        let records = records.into_iter().take(limit).collect::<Vec<_>>();
        let next_cursor = records.last().map(|row| {
            let order_by = options.get("orderBy").and_then(Value::as_str).unwrap_or("last_used");
            json!({"value": if order_by == "timestamp" { row.timestamp } else { row.last_used }, "id": row.id})
        });
        let ids = records.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
        let mut tags_by_record = HashMap::<String, Vec<Value>>::new();
        if !ids.is_empty() {
            let placeholders = vec!["?"; ids.len()].join(",");
            let mut statement = connection.prepare(&format!("SELECT cht.content_id,tg.id,tg.name,tg.color,tg.description,tg.created_at,tg.updated_at FROM clipboard_history_tags cht JOIN tag_groups tg ON tg.id=cht.tag_id WHERE cht.content_id IN ({placeholders}) ORDER BY cht.created_at ASC"))?;
            let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                let id: String = row.get(0)?;
                let tag = json!({"id":row.get::<_,String>(1)?,"name":row.get::<_,String>(2)?,"color":row.get::<_,String>(3)?,"description":row.get::<_,Option<String>>(4)?,"createdAt":row.get::<_,i64>(5)?,"updatedAt":row.get::<_,i64>(6)?});
                Ok((id, tag))
            })?;
            for row in rows { let (id, tag) = row?; tags_by_record.entry(id).or_default().push(tag); }
        }
        let source_ids = records.iter().filter_map(|row| row.source_app.as_deref()).collect::<std::collections::BTreeSet<_>>();
        let mut apps = serde_json::Map::new();
        if !source_ids.is_empty() {
            let placeholders = vec!["?"; source_ids.len()].join(",");
            let mut statement = connection.prepare(&format!("SELECT id,application,path,icon,created_at,updated_at FROM apps WHERE id IN ({placeholders})"))?;
            let rows = statement.query_map(rusqlite::params_from_iter(source_ids.iter()), |row| {
                Ok((row.get::<_,String>(0)?,json!({"id":row.get::<_,String>(0)?,"application":row.get::<_,String>(1)?,"path":row.get::<_,String>(2)?,"icon":row.get::<_,Option<String>>(3)?,"createdAt":row.get::<_,i64>(4)?,"updatedAt":row.get::<_,i64>(5)?})))
            })?;
            for row in rows { let (id, app) = row?; apps.insert(id, app); }
        }
        Ok(HistoryPageData { records, tags_by_record, apps, known_versions, has_more, next_cursor })
    }).await?;

    let mut items = Vec::with_capacity(page.records.len());
    for row in page.records {
        let content_unchanged = page.known_versions.get(&row.id) == Some(&row.updated_at);
        let mut content = serde_json::Map::new();
        let mut preview_truncated =
            row.text_truncated || row.html_truncated || row.rtf_truncated || row.ocr_truncated;
        let files = row
            .content_files_json
            .as_deref()
            .map(serde_json::from_str::<Vec<String>>)
            .transpose()
            .map_err(|error| format!("文件列表格式无效: {error}"))?;
        let files_count = files.as_ref().map_or(0, Vec::len);
        preview_truncated |= files_count > 20;
        if !content_unchanged {
            if let Some(text) = row.content_text {
                content.insert("text".into(), Value::String(text));
            }
            if let Some(html) = row.content_html {
                content.insert("html".into(), Value::String(html));
            }
            if let Some(rtf) = row.content_rtf {
                content.insert("rtf".into(), Value::String(rtf));
            }
            if let Some(ocr) = row.content_ocr {
                content.insert("ocr".into(), Value::String(ocr));
            }
            if let Some(files) = files {
                content.insert(
                    "files".into(),
                    json!(files.into_iter().take(20).collect::<Vec<_>>()),
                );
            }
            if row.has_image {
                content.insert(
                    "image".into(),
                    Value::String(media_reference("preview", &row.id, &row.digest)),
                );
            }
        }
        let image_bytes = row
            .image_bytes
            .and_then(|bytes| usize::try_from(bytes).ok());
        let mut item = json!({"id":row.id,"content":content,"primaryType":row.primary_type,"availableFormats":serde_json::from_str::<Value>(&row.available_formats_json).map_err(|error| format!("剪贴板格式列表无效: {error}"))?,"timestamp":row.timestamp,"lastUsed":row.last_used,"contentVersion":row.updated_at,"hasImage":row.has_image,"imageBytes":image_bytes,"filesCount":files_count,"tags":page.tags_by_record.remove(&row.id).unwrap_or_default()});
        if let Some(text_length) = row.text_length {
            item["textLength"] = json!(text_length);
        }
        if let Some(source) = row.source_app {
            item["source_app"] = json!(source);
        }
        if content_unchanged {
            item["contentUnchanged"] = Value::Bool(true);
        }
        if preview_truncated {
            item["previewTruncated"] = Value::Bool(true);
        }
        items.push(item);
    }
    Ok(
        json!({"success":true,"data":items,"apps":page.apps,"hasMore":page.has_more,"nextCursor":page.next_cursor}),
    )
}

pub(super) async fn search_history_metadata(
    storage: &Storage,
    search_text: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<HistoryMetadata>, String> {
    let search_text = search_text.map(str::to_owned);
    let options = json!({
        "searchText": search_text,
        "orderBy": "timestamp",
        "orderDirection": "DESC",
        "limit": limit,
        "offset": offset
    });
    storage
        .call(move |connection| {
            let (sql, values) = build_history_sql(
                &options,
                "ch.content_id,ch.content_text,ch.content_html,ch.content_rtf,ch.content_files_json,ch.content_ocr,ch.primary_type,ch.available_formats_json,ch.timestamp,ch.last_used,ch.source_app,length(ch.content_image) AS image_bytes",
                None,
            )?;
            let records = {
                let mut statement = connection.prepare(&sql)?;
                let rows = statement.query_map(rusqlite::params_from_iter(values), HistoryMetadataRow::read)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            records
                .into_iter()
                .map(|record| record.into_metadata(connection))
                .collect()
        })
        .await
}

pub(super) async fn get_history_metadata(
    storage: &Storage,
    content_id: &str,
) -> Result<Option<HistoryMetadata>, String> {
    let content_id = content_id.to_owned();
    let options = json!({"limit": 1, "offset": 0});
    storage
        .call(move |connection| {
            let (sql, values) = build_history_sql(
                &options,
                "ch.content_id,ch.content_text,ch.content_html,ch.content_rtf,ch.content_files_json,ch.content_ocr,ch.primary_type,ch.available_formats_json,ch.timestamp,ch.last_used,ch.source_app,length(ch.content_image) AS image_bytes",
                Some(&content_id),
            )?;
            let record = {
                let mut statement = connection.prepare(&sql)?;
                statement
                    .query_row(rusqlite::params_from_iter(values), HistoryMetadataRow::read)
                    .optional()?
            };
            record
                .map(|record| record.into_metadata(connection))
                .transpose()
        })
        .await
}

fn build_history_sql(
    options: &Value,
    projection: &str,
    content_id: Option<&str>,
) -> rusqlite::Result<(String, Vec<rusqlite::types::Value>)> {
    let order_by = match options
        .get("orderBy")
        .and_then(Value::as_str)
        .unwrap_or("last_used")
    {
        "timestamp" => "timestamp",
        "last_used" | "lastUsed" => "last_used",
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let direction = match options
        .get("orderDirection")
        .and_then(Value::as_str)
        .unwrap_or("DESC")
    {
        "ASC" => "ASC",
        "DESC" => "DESC",
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let mut sql = format!("SELECT DISTINCT {projection} FROM clipboard_history ch");
    let mut conditions = Vec::new();
    let mut values = Vec::<rusqlite::types::Value>::new();
    if let Some(content_id) = content_id {
        conditions.push("ch.content_id=?".to_owned());
        values.push(rusqlite::types::Value::Text(content_id.to_owned()));
    }
    let cursor = options.get("cursor");
    if let Some(cursor) = cursor {
        let value = cursor
            .get("value")
            .and_then(Value::as_i64)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let id = cursor
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let comparator = if direction == "DESC" { "<" } else { ">" };
        conditions.push(format!("(ch.{order_by},ch.content_id){comparator}(?,?)"));
        values.push(rusqlite::types::Value::Integer(value));
        values.push(rusqlite::types::Value::Text(id.to_owned()));
    }
    if let Some(tag_ids) = options
        .get("tagIds")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    {
        sql.push_str(" JOIN clipboard_history_tags cht ON ch.content_id=cht.content_id");
        conditions.push(format!(
            "cht.tag_id IN ({})",
            vec!["?"; tag_ids.len()].join(",")
        ));
        for id in tag_ids {
            values.push(rusqlite::types::Value::Text(
                id.as_str().ok_or(rusqlite::Error::InvalidQuery)?.to_owned(),
            ));
        }
    }
    if let Some(primary_type) = options.get("primaryType").and_then(Value::as_str) {
        conditions.push("ch.primary_type=?".to_owned());
        values.push(rusqlite::types::Value::Text(primary_type.to_owned()));
    }
    if let Some(search) = options
        .get("searchText")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        conditions.push("(ch.content_text LIKE ? OR ch.content_ocr LIKE ?)".to_owned());
        let pattern = format!("%{search}%");
        values.push(rusqlite::types::Value::Text(pattern.clone()));
        values.push(rusqlite::types::Value::Text(pattern));
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY ch.");
    sql.push_str(order_by);
    sql.push(' ');
    sql.push_str(direction);
    sql.push_str(",ch.content_id ");
    sql.push_str(direction);
    sql.push_str(" LIMIT ? OFFSET ?");
    let limit = options.get("limit").and_then(Value::as_i64).unwrap_or(100);
    let offset = if cursor.is_some() {
        0
    } else {
        options.get("offset").and_then(Value::as_i64).unwrap_or(0)
    };
    if limit < 0 || offset < 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    values.push(rusqlite::types::Value::Integer(limit));
    values.push(rusqlite::types::Value::Integer(offset));
    Ok((sql, values))
}

pub(super) async fn delete_clipboard(storage: &Storage, id: &str) -> Result<bool, String> {
    let cache_id = id.to_owned();
    let id = cache_id.clone();
    let deleted = storage
        .call(move |connection| {
            Ok(connection.execute("DELETE FROM clipboard_history WHERE content_id=?1", [id])? > 0)
        })
        .await?;
    if deleted {
        forget_preview_id(&cache_id);
    }
    Ok(deleted)
}

pub(super) async fn clear_clipboard(storage: &Storage) -> Result<Vec<String>, String> {
    let deleted = storage.call(|connection| {
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare("DELETE FROM clipboard_history WHERE content_id NOT IN (SELECT content_id FROM clipboard_history_tags) RETURNING content_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let deleted = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        transaction.commit()?;
        Ok(deleted)
    }).await?;
    if !deleted.is_empty() {
        clear_preview_cache();
    }
    Ok(deleted)
}

pub(super) async fn history_count(storage: &Storage) -> Result<u64, String> {
    storage
        .call(|connection| {
            connection
                .query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map(|count| count.max(0) as u64)
        })
        .await
}

pub(super) async fn touch_clipboard(storage: &Storage, id: &str) -> Result<bool, String> {
    let id = id.to_owned();
    storage.call(move|connection|Ok(connection.execute("UPDATE clipboard_history SET last_used=MAX(last_used,?1),updated_at=?1 WHERE content_id=?2",params![now_ms(),id])?>0)).await
}

pub(super) async fn cleanup_history(
    storage: &Storage,
    max_history_size: i64,
    max_keep_time_days: i64,
) -> Result<Vec<String>, String> {
    if max_history_size < -1 || max_keep_time_days < -1 {
        return Err("历史清理参数无效".to_owned());
    }
    let deleted = storage.call(move|connection|{
        let transaction=connection.transaction()?;
        let mut deleted=Vec::new();
        if max_keep_time_days>0{
            let cutoff=now_ms().saturating_sub(max_keep_time_days.saturating_mul(86_400_000));
            let mut statement=transaction.prepare("DELETE FROM clipboard_history WHERE timestamp<?1 AND content_id NOT IN (SELECT content_id FROM clipboard_history_tags) RETURNING content_id")?;
            let rows=statement.query_map([cutoff],|row|row.get::<_,String>(0))?;
            let expired=rows.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            deleted.extend(expired);
        }
        if max_history_size>0{
            let count:i64=transaction.query_row("SELECT COUNT(*) FROM clipboard_history",[],|row|row.get(0))?;
            let excess=count.saturating_sub(max_history_size);
            if excess>0{
                let mut statement=transaction.prepare("DELETE FROM clipboard_history WHERE content_id IN (SELECT content_id FROM clipboard_history WHERE content_id NOT IN (SELECT content_id FROM clipboard_history_tags) ORDER BY last_used DESC LIMIT ?1 OFFSET ?2) RETURNING content_id")?;
                let rows=statement.query_map(params![excess,max_history_size],|row|row.get::<_,String>(0))?;
                let candidates=rows.collect::<rusqlite::Result<Vec<_>>>()?;
                drop(statement);
                deleted.extend(candidates);
            }
        }
        transaction.commit()?;
        Ok(deleted)
    }).await?;
    for id in &deleted {
        forget_preview_id(id);
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(900, 700, image::Rgb([24, 96, 180]));
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }

    fn test_item(id: &str, text: String, files: Option<Vec<String>>) -> Value {
        let mut content = serde_json::Map::new();
        content.insert("text".to_owned(), Value::String(text));
        if id == "record-c" {
            content.insert(
                "html".to_owned(),
                Value::String("<p>rich preview</p>".to_owned()),
            );
            content.insert(
                "rtf".to_owned(),
                Value::String(r"{\rtf1 rich preview}".to_owned()),
            );
        }
        if let Some(files) = files {
            content.insert("files".to_owned(), json!(files));
        }
        json!({
            "id": id,
            "content": Value::Object(content),
            "primaryType": "text",
            "timestamp": 100,
            "lastUsed": 500,
            "source_app": "test-app"
        })
    }

    #[tokio::test]
    async fn history_pages_return_bounded_previews_and_validated_media_refs() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(directory.path()).await.unwrap();
        storage
            .upsert_app(&json!({
                "id": "test-app",
                "application": "Test App",
                "path": "/test/app"
            }))
            .await
            .unwrap();
        storage
            .invoke(
                "create-tag",
                &[json!({
                    "id": "test-tag",
                    "name": "Test",
                    "color": "#ff0000"
                })],
            )
            .await
            .unwrap();

        let original_image = test_image();
        let full_text = format!("{}long-tail-marker", "x".repeat(2048));
        let files = (0..25)
            .map(|index| format!("/tmp/file-{index}"))
            .collect::<Vec<_>>();
        let inserted = storage
            .upsert_clipboard_with_image(
                &test_item("record-c", full_text.clone(), Some(files)),
                Some(original_image.clone()),
            )
            .await
            .unwrap();
        storage
            .upsert_clipboard_with_image(&test_item("record-b", "second".into(), None), None)
            .await
            .unwrap();
        storage
            .upsert_clipboard_with_image(&test_item("record-a", "third".into(), None), None)
            .await
            .unwrap();
        storage
            .invoke(
                "bind-tag-to-record",
                &[json!({"contentId":"record-c","tagId":"test-tag"})],
            )
            .await
            .unwrap();

        let first = storage
            .invoke(
                "clipboard-get-history",
                &[json!({
                    "limit": 1,
                    "orderBy": "last_used",
                    "orderDirection": "DESC"
                })],
            )
            .await
            .unwrap();
        let first_item = &first["data"][0];
        assert_eq!(first_item["id"], "record-c");
        assert_eq!(first_item["content"]["text"].as_str().unwrap().len(), 2048);
        assert_eq!(
            first_item["textLength"].as_u64(),
            Some(full_text.len() as u64)
        );
        assert_eq!(first_item["content"]["html"], "<p>rich preview</p>");
        assert_eq!(first_item["content"]["rtf"], r"{\rtf1 rich preview}");
        assert_eq!(first_item["content"]["files"].as_array().unwrap().len(), 20);
        assert_eq!(first_item["filesCount"], 25);
        assert_eq!(first_item["previewTruncated"], true);
        assert_eq!(
            first_item["content"]["image"],
            format!("preview/record-c/{}", inserted.digest)
        );
        assert_eq!(
            first_item["imageBytes"].as_u64(),
            Some(original_image.len() as u64)
        );
        assert_eq!(first_item["tags"].as_array().unwrap().len(), 1);
        assert_eq!(first["apps"]["test-app"]["application"], "Test App");
        assert_eq!(first["hasMore"], true);

        let known = storage
            .invoke(
                "clipboard-get-history",
                &[json!({
                    "limit": 1,
                    "orderBy": "last_used",
                    "orderDirection": "DESC",
                    "knownVersions": {"record-c": first_item["contentVersion"]}
                })],
            )
            .await
            .unwrap();
        assert_eq!(known["data"][0]["content"], json!({}));
        assert_eq!(known["data"][0]["contentUnchanged"], true);
        assert_eq!(known["data"][0]["previewTruncated"], true);

        let second = storage
            .invoke(
                "clipboard-get-history",
                &[json!({
                    "limit": 1,
                    "orderBy": "last_used",
                    "orderDirection": "DESC",
                    "cursor": first["nextCursor"]
                })],
            )
            .await
            .unwrap();
        let third = storage
            .invoke(
                "clipboard-get-history",
                &[json!({
                    "limit": 1,
                    "orderBy": "last_used",
                    "orderDirection": "DESC",
                    "cursor": second["nextCursor"]
                })],
            )
            .await
            .unwrap();
        assert_eq!(second["data"][0]["id"], "record-b");
        assert_eq!(third["data"][0]["id"], "record-a");
        assert_eq!(third["hasMore"], false);

        let search = storage
            .invoke(
                "clipboard-get-history",
                &[json!({"searchText":"long-tail-marker"})],
            )
            .await
            .unwrap();
        assert_eq!(search["data"][0]["id"], "record-c");

        let preview = storage
            .get_clipboard_media("preview", "record-c", &inserted.digest)
            .await
            .unwrap()
            .unwrap();
        let decoded_preview = image::load_from_memory(&preview).unwrap();
        assert!(decoded_preview.width() <= 640 && decoded_preview.height() <= 640);
        let full_item = storage
            .get_clipboard_item("record-c")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(full_item["content"]["text"], full_text);
        assert_eq!(full_item["content"]["files"].as_array().unwrap().len(), 25);
        assert_eq!(
            full_item["content"]["image"],
            format!("original/record-c/{}", inserted.digest)
        );
        assert_eq!(
            full_item["imageBytes"].as_u64(),
            Some(original_image.len() as u64)
        );
        assert_eq!(
            storage
                .get_clipboard_media("original", "record-c", &inserted.digest)
                .await
                .unwrap()
                .unwrap(),
            original_image
        );
        assert!(
            storage
                .get_clipboard_media("original", "record-c", "wrong-digest")
                .await
                .unwrap()
                .is_none()
        );
        let text_snapshot = storage
            .get_clipboard_snapshot("record-c", true, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text_snapshot.text.as_deref(), Some(full_text.as_str()));
        assert!(text_snapshot.image.is_none());
    }
}
