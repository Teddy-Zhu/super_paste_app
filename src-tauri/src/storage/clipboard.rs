use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};

use super::{Storage, now_ms};

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
    content_text: Option<String>,
    content_html: Option<String>,
    content_image: Option<Vec<u8>>,
    content_rtf: Option<String>,
    content_files_json: Option<String>,
    content_ocr: Option<String>,
    primary_type: String,
    available_formats_json: String,
    timestamp: i64,
    last_used: i64,
    source_app: Option<String>,
}

impl HistoryRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("content_id")?,
            content_text: row.get("content_text")?,
            content_html: row.get("content_html")?,
            content_image: row.get("content_image")?,
            content_rtf: row.get("content_rtf")?,
            content_files_json: row.get("content_files_json")?,
            content_ocr: row.get("content_ocr")?,
            primary_type: row.get("primary_type")?,
            available_formats_json: row.get("available_formats_json")?,
            timestamp: row.get("timestamp")?,
            last_used: row.get("last_used")?,
            source_app: row.get("source_app")?,
        })
    }

    fn into_dto(self, tags: Vec<Value>) -> Result<Value, String> {
        let mut content = serde_json::Map::new();
        if let Some(value) = self.content_text {
            content.insert("text".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_html {
            content.insert("html".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_image {
            content.insert(
                "image".to_owned(),
                Value::Array(value.into_iter().map(Value::from).collect()),
            );
        }
        if let Some(value) = self.content_rtf {
            content.insert("rtf".to_owned(), Value::String(value));
        }
        if let Some(value) = self.content_files_json {
            content.insert(
                "files".to_owned(),
                serde_json::from_str(&value)
                    .map_err(|error| format!("文件列表格式无效: {error}"))?,
            );
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
            "tags": tags
        });
        if let Some(source) = self.source_app {
            dto["source_app"] = Value::String(source);
        }
        Ok(dto)
    }
}

pub(super) async fn upsert_clipboard(
    storage: &Storage,
    item: &Value,
) -> Result<UpsertClipboard, String> {
    let image = item
        .get("content")
        .and_then(Value::as_object)
        .and_then(|content| content.get("image"))
        .filter(|value| !value.is_null())
        .map(data_format::image_bytes)
        .transpose()?;
    upsert_clipboard_with_image(storage, item, image).await
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
            let digest = data_format::content_digest_with_image(&content_value, content_image.as_deref())
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
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

pub(super) async fn get_clipboard(
    storage: &Storage,
    id: &str,
    with_tags: bool,
) -> Result<Option<Value>, String> {
    let id = id.to_owned();
    let row = storage
        .call(move |connection| {
            let mut statement =
                connection.prepare("SELECT * FROM clipboard_history WHERE content_id=?1")?;
            statement.query_row([id], HistoryRow::read).optional()
        })
        .await?;
    match row {
        Some(row) => {
            let tags = if with_tags {
                get_record_tags(storage, &row.id).await?
            } else {
                Vec::new()
            };
            row.into_dto(tags).map(Some)
        }
        None => Ok(None),
    }
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

pub(super) async fn get_tagged_record_ids(storage: &Storage) -> Result<Vec<String>, String> {
    storage
        .call(|connection| {
            let mut statement =
                connection.prepare("SELECT DISTINCT content_id FROM clipboard_history_tags")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect()
        })
        .await
}

pub(super) async fn upsert_app(storage: &Storage, value: &Value) -> Result<String, String> {
    let value = value.clone();
    storage
        .call(move |connection| {
            let application = value.get("application").and_then(Value::as_str).ok_or(rusqlite::Error::InvalidQuery)?;
            let path = value.get("path").and_then(Value::as_str).ok_or(rusqlite::Error::InvalidQuery)?;
            let icon = value.get("icon").and_then(Value::as_str);
            let now = now_ms();
            if let Some((id, old_icon)) = connection
                .query_row(
                    "SELECT id,icon FROM apps WHERE application=?1 AND path=?2",
                    params![application,path],
                    |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?)),
                )
                .optional()?
            {
                if icon.is_some_and(|icon| crate::platform::source_icon::should_replace(old_icon.as_deref(), icon)) && icon != old_icon.as_deref() {
                    connection.execute("UPDATE apps SET icon=?1,updated_at=?2 WHERE id=?3",params![icon,now,id])?;
                }
                return Ok(id);
            }
            let id = value.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let created_at=value.get("createdAt").and_then(Value::as_i64).unwrap_or(now);
            let updated_at=value.get("updatedAt").and_then(Value::as_i64).unwrap_or(now);
            connection.execute("INSERT INTO apps(id,application,path,icon,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,application,path,icon,created_at,updated_at])?;
            Ok(id)
        })
        .await
}

pub(super) async fn get_record_tags(storage: &Storage, id: &str) -> Result<Vec<Value>, String> {
    let id = id.to_owned();
    storage
        .call(move |connection| {
            let mut statement = connection.prepare("SELECT tg.id,tg.name,tg.color,tg.description,tg.created_at,tg.updated_at FROM tag_groups tg JOIN clipboard_history_tags cht ON cht.tag_id=tg.id WHERE cht.content_id=?1 ORDER BY cht.created_at ASC")?;
            let rows = statement.query_map([id], |row| {
                Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"color":row.get::<_,String>(2)?,"description":row.get::<_,Option<String>>(3)?,"createdAt":row.get::<_,i64>(4)?,"updatedAt":row.get::<_,i64>(5)?}))
            })?;
            rows.collect()
        })
        .await
}

pub(super) async fn query_history(storage: &Storage, options: &Value) -> Result<Value, String> {
    let options = options.clone();
    storage
        .call(move |connection| {
            let (sql, values) = build_history_sql(&options, "ch.*", None)?;
            let records = {
                let mut statement = connection.prepare(&sql)?;
                let rows = statement.query_map(rusqlite::params_from_iter(values), HistoryRow::read)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut items = Vec::with_capacity(records.len());
            let mut apps = serde_json::Map::new();
            for row in records {
                if let Some(source_id) = &row.source_app {
                    if !apps.contains_key(source_id) {
                        if let Some(app) = connection.query_row(
                            "SELECT id,application,path,icon,created_at,updated_at FROM apps WHERE id=?1",
                            [source_id],
                            |app| Ok(json!({"id":app.get::<_,String>(0)?,"application":app.get::<_,String>(1)?,"path":app.get::<_,String>(2)?,"icon":app.get::<_,Option<String>>(3)?,"createdAt":app.get::<_,i64>(4)?,"updatedAt":app.get::<_,i64>(5)?})),
                        ).optional()? {
                            apps.insert(source_id.clone(), app);
                        }
                    }
                }
                let tags = record_tags_sync(connection, &row.id)?;
                items.push(row.into_dto(tags).map_err(|_| rusqlite::Error::InvalidQuery)?);
            }
            Ok(json!({"success":true,"data":items,"apps":apps}))
        })
        .await
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
    sql.push_str(" LIMIT ? OFFSET ?");
    let limit = options.get("limit").and_then(Value::as_i64).unwrap_or(100);
    let offset = options.get("offset").and_then(Value::as_i64).unwrap_or(0);
    if limit < 0 || offset < 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    values.push(rusqlite::types::Value::Integer(limit));
    values.push(rusqlite::types::Value::Integer(offset));
    Ok((sql, values))
}

fn record_tags_sync(connection: &rusqlite::Connection, id: &str) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare("SELECT tg.id,tg.name,tg.color,tg.description,tg.created_at,tg.updated_at FROM tag_groups tg JOIN clipboard_history_tags cht ON cht.tag_id=tg.id WHERE cht.content_id=?1 ORDER BY cht.created_at ASC")?;
    let rows = statement.query_map([id], |row| {
        Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"color":row.get::<_,String>(2)?,"description":row.get::<_,Option<String>>(3)?,"createdAt":row.get::<_,i64>(4)?,"updatedAt":row.get::<_,i64>(5)?}))
    })?;
    rows.collect()
}

pub(super) async fn delete_clipboard(storage: &Storage, id: &str) -> Result<bool, String> {
    let id = id.to_owned();
    storage
        .call(move |connection| {
            Ok(connection.execute("DELETE FROM clipboard_history WHERE content_id=?1", [id])? > 0)
        })
        .await
}

pub(super) async fn clear_clipboard(storage: &Storage) -> Result<Vec<String>, String> {
    storage.call(|connection| {
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare("DELETE FROM clipboard_history WHERE content_id NOT IN (SELECT content_id FROM clipboard_history_tags) RETURNING content_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let deleted = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        transaction.commit()?;
        Ok(deleted)
    }).await
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
    storage.call(move|connection|{
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
    }).await
}

pub(super) async fn get_all_tags(storage: &Storage) -> Result<Vec<Value>, String> {
    storage.call(|connection| {
        let mut statement = connection.prepare("SELECT id,name,color,description,created_at,updated_at FROM tag_groups ORDER BY created_at DESC")?;
        let rows=statement.query_map([],|row|Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"color":row.get::<_,String>(2)?,"description":row.get::<_,Option<String>>(3)?,"createdAt":row.get::<_,i64>(4)?,"updatedAt":row.get::<_,i64>(5)?})))?;
        rows.collect()
    }).await
}

pub(super) async fn create_tag(storage: &Storage, value: &Value) -> Result<String, String> {
    let value = value.clone();
    let result = storage.call(move |connection| {
        let result: rusqlite::Result<String> = (|| {
        let id=value.get("id").and_then(Value::as_str).filter(|value| !value.is_empty()).map(str::to_owned).unwrap_or_else(||uuid::Uuid::new_v4().to_string());
        let name=value.get("name").and_then(Value::as_str).filter(|value| !value.trim().is_empty()).ok_or(rusqlite::Error::InvalidQuery)?;
        let color=value.get("color").and_then(Value::as_str).ok_or(rusqlite::Error::InvalidQuery)?;
        let description=value.get("description").and_then(Value::as_str);
        let now=now_ms();
        connection.execute("INSERT INTO tag_groups(id,name,color,description,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,name,color,description,value.get("createdAt").and_then(Value::as_i64).unwrap_or(now),value.get("updatedAt").and_then(Value::as_i64).unwrap_or(now)])?;
        Ok(id)
        })();
        Ok(result)
    }).await.map_err(|error| {
        tracing::error!(error = %error, "failed to create tag");
        format!("创建标签失败: {error}")
    })?;
    result.map_err(|error| {
        if matches!(
            &error,
            rusqlite::Error::SqliteFailure(code, Some(message))
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    && message == "UNIQUE constraint failed: tag_groups.name"
        ) {
            return "标签名称已存在".to_owned();
        }
        tracing::error!(error = %error, "failed to create tag");
        format!("创建标签失败: {error}")
    })
}

pub(super) async fn update_tag(
    storage: &Storage,
    tag_id: &str,
    updates: &Value,
) -> Result<bool, String> {
    let (tag_id, updates) = (tag_id.to_owned(), updates.clone());
    storage.call(move |connection| {
        let name=updates.get("name").and_then(Value::as_str);
        let color=updates.get("color").and_then(Value::as_str);
        let description=updates.get("description").and_then(Value::as_str);
        if name.is_none() && color.is_none() && description.is_none() { return Ok(false); }
        Ok(connection.execute("UPDATE tag_groups SET name=COALESCE(?1,name),color=COALESCE(?2,color),description=CASE WHEN ?3 THEN ?4 ELSE description END,updated_at=?5 WHERE id=?6",params![name,color,updates.get("description").is_some(),description,now_ms(),tag_id])?>0)
    }).await
}

pub(super) async fn delete_tag(storage: &Storage, tag_id: &str) -> Result<bool, String> {
    let tag_id = tag_id.to_owned();
    storage
        .call(move |connection| {
            Ok(connection.execute("DELETE FROM tag_groups WHERE id=?1", [tag_id])? > 0)
        })
        .await
}

pub(super) async fn get_tag_stats(storage: &Storage) -> Result<Vec<Value>, String> {
    storage.call(|connection| {
        let mut statement=connection.prepare("SELECT tg.id,tg.name,COUNT(cht.content_id) FROM tag_groups tg LEFT JOIN clipboard_history_tags cht ON tg.id=cht.tag_id GROUP BY tg.id,tg.name ORDER BY COUNT(cht.content_id) DESC,tg.name ASC")?;
        let rows=statement.query_map([],|row|Ok(json!({"tagId":row.get::<_,String>(0)?,"tagName":row.get::<_,String>(1)?,"recordCount":row.get::<_,i64>(2)?})))?;
        rows.collect()
    }).await
}

pub(super) async fn bind_tag(
    storage: &Storage,
    content_id: &str,
    tag_id: &str,
) -> Result<bool, String> {
    let (content_id, tag_id) = (content_id.to_owned(), tag_id.to_owned());
    storage.call(move |connection|Ok(connection.execute("INSERT OR IGNORE INTO clipboard_history_tags(content_id,tag_id,created_at) VALUES(?1,?2,?3)",params![content_id,tag_id,now_ms()])?>0)).await
}

pub(super) async fn unbind_tag(
    storage: &Storage,
    content_id: &str,
    tag_id: &str,
) -> Result<bool, String> {
    let (content_id, tag_id) = (content_id.to_owned(), tag_id.to_owned());
    storage.call(move |connection|{
        let transaction=connection.transaction()?;
        let changed=transaction.execute("DELETE FROM clipboard_history_tags WHERE content_id=?1 AND tag_id=?2",params![content_id,tag_id])?>0;
        if changed {
            let has_tags:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM clipboard_history_tags WHERE content_id=?1)",[&content_id],|row|row.get(0))?;
            if !has_tags { let now=now_ms(); transaction.execute("UPDATE clipboard_history SET timestamp=?1,last_used=?1,updated_at=?1 WHERE content_id=?2",params![now,content_id])?; }
        }
        transaction.commit()?;
        Ok(changed)
    }).await
}
