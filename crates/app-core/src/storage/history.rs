use std::{collections::{HashMap, HashSet}, sync::Arc};
use rusqlite::{OptionalExtension, Row, params, params_from_iter, types::Value};
use tokio_util::sync::CancellationToken;
use super_paste_format::*;
use crate::{Error, Result, images::EncodedImage};
use crate::{ocr::{OcrTicket, OcrAdmission}, configuration::ocr_revision_in};
use crate::{clipboard::{ClipboardSnapshot, RestoreMode}, images::ImageSource};
use super::{Storage, collect_orphan_images, revision};

pub struct PreparedRecord {
    pub digest: Digest, pub content: TextContent, pub image: Option<Arc<EncodedImage>>,
    pub timestamp: i64, pub source: Option<Arc<SourceApp>>,
}
#[derive(Clone, Debug)]
pub struct UpsertResult { pub id: RecordId, pub inserted: bool, pub last_used: i64 }

const PROJECTION: &str = "h.id,h.primary_type,h.formats,h.timestamp,h.last_used,h.content_revision,h.metadata_revision,h.ocr_revision,h.text_characters,h.files_count,h.text_excerpt,h.html_excerpt,h.rtf_excerpt,h.ocr_has_text,h.truncated,s.id AS source_id,s.application AS source_application,s.path AS source_path,i.id AS image_id,i.encoded_bytes AS image_bytes,i.width AS image_width,i.height AS image_height";

impl Storage {
    /// Save the captured record and enforce retention in one commit. Identity
    /// and decoding have already completed before entering the SQLite worker.
    pub async fn upsert_record(&self, prepared: PreparedRecord, limits: HistoryLimits) -> Result<(UpsertResult, Vec<RecordId>)> {
        if prepared.content.formats(prepared.image.is_some()).is_empty() {
            return Err(Error::Invalid("剪贴板没有可保存的内容".into()));
        }
        if limits.max_records < -1 || limits.keep_days < -1 {
            return Err(Error::Invalid("历史清理参数无效".into()));
        }
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let record = upsert_record_in(&transaction, &prepared)?;
            let removed = apply_history_limits(&transaction, limits, prepared.timestamp)?;
            transaction.commit()?;
            Ok((record, removed))
        }).await
    }

    pub async fn history(&self, query: HistoryQuery, cancel: &CancellationToken) -> Result<HistoryPage> {
        self.history_reusing(query, Arc::new(HashMap::new()), cancel).await
    }

    /// Probe ordering and revisions first when the caller already owns a window.
    /// Only new or revised rows hydrate excerpts, tags, files and source icons;
    /// unchanged rows retain their shared owners. No second cache is retained.
    pub async fn history_reusing(&self, query: HistoryQuery, known: Arc<HashMap<RecordId, Arc<HistoryEntry>>>, cancel: &CancellationToken) -> Result<HistoryPage> {
        if query.limit == 0 || query.limit > 1000 { return Err(Error::Invalid("历史分页大小无效".into())); }
        self.call_read(cancel, move |connection| {
            let order = match query.order { HistoryOrder::LastUsed => "last_used", HistoryOrder::Timestamp => "timestamp" };
            let projection = if known.is_empty() { PROJECTION } else { "h.id,h.content_revision,h.metadata_revision,h.ocr_revision" };
            let joins = if known.is_empty() { " LEFT JOIN sources s ON s.id=h.source_id LEFT JOIN images i ON i.id=h.image_id" } else { "" };
            let mut sql = format!("SELECT {projection} FROM history h{joins} WHERE 1=1");
            let mut values = Vec::<Value>::new();
            let search = query.search.trim();
            if !search.is_empty() {
                // Preserve LIKE wildcard/case semantics instead of silently replacing substring search with FTS.
                sql.push_str(" AND EXISTS(SELECT 1 FROM payloads p WHERE p.record_id=h.id AND (p.text LIKE ? OR p.ocr LIKE ?))");
                let pattern = format!("%{search}%"); values.push(pattern.clone().into()); values.push(pattern.into());
            }
            if let Some(kind) = query.primary_type { sql.push_str(" AND h.primary_type=?"); values.push(kind.as_str().to_owned().into()); }
            if !query.tag_ids.is_empty() {
                sql.push_str(" AND EXISTS(SELECT 1 FROM record_tags rt WHERE rt.record_id=h.id AND rt.tag_id IN (");
                sql.push_str(&placeholders(query.tag_ids.len())); sql.push_str("))");
                values.extend(query.tag_ids.iter().map(|id| Value::Text(id.to_string())));
            }
            if let Some(cursor) = &query.cursor {
                sql.push_str(&format!(" AND (h.{order},h.id)<(?,?)"));
                values.push(cursor.value.into()); values.push(cursor.id.to_string().into());
            }
            sql.push_str(&format!(" ORDER BY h.{order} DESC,h.id DESC LIMIT ?"));
            values.push(i64::from(query.limit + 1).into());
            let (entries, has_more) = if known.is_empty() {
                let mut entries = {
                    let mut statement = connection.prepare(&sql)?;
                    statement.query_map(params_from_iter(values), read_entry)?.collect::<rusqlite::Result<Vec<_>>>()?
                };
                let has_more = entries.len() > query.limit as usize;
                entries.truncate(query.limit as usize);
                hydrate_page(connection, &mut entries)?;
                // Do not retain the much larger row buffer through Vec's
                // in-place collect optimization when converting to pointers.
                let mut shared = Vec::with_capacity(entries.len());
                shared.extend(entries.into_iter().map(Arc::new));
                (shared, has_more)
            } else {
                let mut stamps = {
                    let mut statement = connection.prepare(&sql)?;
                    statement.query_map(params_from_iter(values), |row| Ok((
                        parse_sql_id::<RecordId>(&row.get::<_, String>(0)?)?,
                        row.get::<_, u64>(1)?, row.get::<_, u64>(2)?, row.get::<_, u64>(3)?,
                    )))?.collect::<rusqlite::Result<Vec<_>>>()?
                };
                let has_more = stamps.len() > query.limit as usize;
                stamps.truncate(query.limit as usize);
                let changed: Vec<_> = stamps.iter().filter(|(id, content, metadata, ocr)| {
                    known.get(id).is_none_or(|entry| (entry.content_revision, entry.metadata_revision, entry.ocr_revision) != (*content, *metadata, *ocr))
                }).map(|(id, ..)| id.to_string()).collect();
                let mut replacements = HashMap::new();
                if !changed.is_empty() {
                    let sql = format!("SELECT {PROJECTION} FROM history h LEFT JOIN sources s ON s.id=h.source_id LEFT JOIN images i ON i.id=h.image_id WHERE h.id IN ({})", placeholders(changed.len()));
                    let mut entries = {
                        let mut statement = connection.prepare(&sql)?;
                        statement.query_map(params_from_iter(&changed), read_entry)?.collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    hydrate_page(connection, &mut entries)?;
                    replacements.extend(entries.into_iter().map(|entry| (entry.id, Arc::new(entry))));
                }
                let mut entries = Vec::with_capacity(stamps.len());
                for (id, content, metadata, ocr) in stamps {
                    let entry = replacements.remove(&id).or_else(|| known.get(&id)
                        .filter(|entry| (entry.content_revision, entry.metadata_revision, entry.ocr_revision) == (content, metadata, ocr)).cloned())
                        .ok_or_else(|| Error::Database("历史记录在修订读取期间发生变化".into()))?;
                    entries.push(entry);
                }
                tracing::debug!(rows = entries.len(), hydrated = changed.len(), "历史窗口增量读取");
                (entries, has_more)
            };
            let next_cursor = if has_more { entries.last().map(|entry| HistoryCursor { value: match query.order { HistoryOrder::LastUsed => entry.last_used, HistoryOrder::Timestamp => entry.timestamp }, id: entry.id }) } else { None };
            Ok(HistoryPage { entries, next_cursor })
        }).await
    }

    pub async fn content(&self, id: RecordId) -> Result<FullContent> {
        self.call(move |connection| {
            let sql = format!("SELECT {PROJECTION},p.text AS full_text,p.html AS full_html,p.rtf AS full_rtf,p.ocr AS full_ocr FROM history h JOIN payloads p ON p.record_id=h.id LEFT JOIN sources s ON s.id=h.source_id LEFT JOIN images i ON i.id=h.image_id WHERE h.id=?1");
            let result = connection.query_row(&sql, [id.to_string()], |row| {
                Ok(FullContent { id, revision: row.get("content_revision")?, ocr_revision: row.get("ocr_revision")?,
                    content: TextContent { text: row.get("full_text")?, html: row.get("full_html")?, rtf: row.get("full_rtf")?, files: Arc::default() },
                    ocr: row.get("full_ocr")?, image: read_image_metadata(row)? })
            }).optional()?;
            let mut result = result.ok_or(Error::NotFound)?;
            let mut statement = connection.prepare_cached("SELECT path FROM files WHERE record_id=?1 ORDER BY position")?;
            result.content.files = statement.query_map([id.to_string()], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?.into();
            Ok(result)
        }).await
    }

    /// Select only the formats needed by the requested restoration. Plain text
    /// keeps raw text only when it wins restoration priority. Otherwise retain
    /// every fallback: an HTML/RTF payload can produce no readable text. All
    /// reads share one snapshot, without duplicating Rust's Unicode trim in SQL.
    pub(crate) async fn restore_input(&self, id: RecordId, mode: RestoreMode, cancel: &CancellationToken) -> Result<ClipboardSnapshot> {
        let scan_cancel = cancel.clone();
        self.call_read(cancel, move |connection| {
            // The ordered worker owns this read-only snapshot. Cancellation
            // must retire its SQL scope before a later write can begin.
            let transaction = connection.unchecked_transaction()?;
            let sql = match mode {
                RestoreMode::Original => "SELECT p.text AS full_text,p.html AS full_html,p.rtf AS full_rtf,i.id AS image_id,i.encoded_bytes AS image_bytes,i.width AS image_width,i.height AS image_height FROM history h JOIN payloads p ON p.record_id=h.id LEFT JOIN images i ON i.id=h.image_id WHERE h.id=?1",
                RestoreMode::PlainText => "SELECT p.text AS full_text,NULL AS full_html,NULL AS full_rtf FROM history h JOIN payloads p ON p.record_id=h.id WHERE h.id=?1",
                RestoreMode::Ocr => "SELECT p.ocr AS full_text,NULL AS full_html,NULL AS full_rtf FROM history h JOIN payloads p ON p.record_id=h.id WHERE h.id=?1",
            };
            let mut snapshot = transaction.query_row(sql, [id.to_string()], |row| Ok(ClipboardSnapshot {
                content: TextContent { text: row.get("full_text")?, html: row.get("full_html")?, rtf: row.get("full_rtf")?, files: Arc::default() },
                image: if matches!(mode, RestoreMode::Original) { read_image_metadata(row)?.map(ImageSource::Stored) } else { None },
            })).optional()?.ok_or(Error::NotFound)?;
            if scan_cancel.is_cancelled() { return Err(Error::Cancelled); }
            let fallback = matches!(mode, RestoreMode::PlainText) && snapshot.content.original_plain_text().is_none();
            if fallback {
                if scan_cancel.is_cancelled() { return Err(Error::Cancelled); }
                snapshot.content.text = None;
                (snapshot.content.html, snapshot.content.rtf) = transaction.query_row("SELECT html,rtf FROM payloads WHERE record_id=?1", [id.to_string()], |row| {
                    Ok((row.get::<_, Option<Arc<str>>>(0)?, row.get::<_, Option<Arc<str>>>(1)?))
                })?;
            }
            if matches!(mode, RestoreMode::Original) || fallback {
                if scan_cancel.is_cancelled() { return Err(Error::Cancelled); }
                let mut statement = transaction.prepare_cached("SELECT path FROM files WHERE record_id=?1 ORDER BY position")?;
                snapshot.content.files = statement.query_map([id.to_string()], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?.into();
            }
            transaction.commit()?;
            Ok(snapshot)
        }).await
    }

    pub(crate) async fn ocr_input(&self, id: RecordId) -> Result<Option<(u64, ImageMetadata)>> {
        self.call(move |connection| {
            Ok(connection.query_row("SELECT h.content_revision,i.id AS image_id,i.encoded_bytes AS image_bytes,i.width AS image_width,i.height AS image_height FROM history h JOIN images i ON i.id=h.image_id WHERE h.id=?1", [id.to_string()], |row| {
                Ok((row.get(0)?, read_image_metadata(row)?.ok_or(rusqlite::Error::InvalidQuery)?))
            }).optional()?)
        }).await
    }
    /// Read only the authoritative QR source. An available text payload avoids
    /// fetching HTML, RTF, files, OCR text, source icons or image bytes.
    pub(crate) async fn qr_text_input(&self, id: RecordId, cancel: &CancellationToken) -> Result<Arc<str>> {
        let scan_cancel = cancel.clone();
        self.call_read(cancel, move |connection| {
            // 7089 bytes is an upper bound even for QR's most compact numeric
            // mode at its largest version. An oversized text cannot succeed,
            // so do not allocate it merely to discover DataTooLong afterward.
            let mut statement = connection.prepare("SELECT h.primary_type,COALESCE(length(CAST(p.text AS BLOB)),0),CASE WHEN length(CAST(p.text AS BLOB))<=?2 THEN p.text ELSE NULL END,CASE WHEN p.text IS NULL OR p.text='' THEN p.html ELSE NULL END FROM history h JOIN payloads p ON p.record_id=h.id WHERE h.id=?1")?;
            let mut rows = statement.query(params![id.to_string(), crate::qr::MAX_TEXT_BYTES as u32])?;
            let row = rows.next()?.ok_or(Error::NotFound)?;
            if !matches!(row.get::<_, String>(0)?.as_str(), "text" | "html" | "url") { return Err(Error::Invalid("只有文本类型的内容才能生成二维码".into())); }
            if row.get::<_, u64>(1)? > crate::qr::MAX_TEXT_BYTES as u64 { return Err(Error::Invalid("文本过长，无法生成二维码".into())); }
            for column in [2, 3] {
                match row.get_ref(column)? {
                    rusqlite::types::ValueRef::Null => {},
                    rusqlite::types::ValueRef::Text(bytes) => {
                        let text = std::str::from_utf8(bytes).map_err(|_| Error::Database("二维码源文本不是有效 UTF-8".into()))?;
                        if column == 2 && !text.is_empty() { return Ok(Arc::from(text)); }
                        if column == 3 { return crate::qr::text_from_html(text, &scan_cancel); }
                    },
                    _ => return Err(Error::Database("二维码源文本类型无效".into())),
                }
            }
            Ok(Arc::from(""))
        }).await
    }
    pub(crate) async fn begin_ocr(&self, id: RecordId, content_revision: u64, configuration_revision: u64) -> Result<OcrAdmission> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            if ocr_revision_in(&transaction)? != configuration_revision { return Ok(OcrAdmission::ConfigurationChanged); }
            let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM history WHERE id=?1 AND content_revision=?2 AND image_id IS NOT NULL)", params![id.to_string(), content_revision], |row| row.get(0))?;
            if !exists { return Ok(OcrAdmission::Missing); }
            let generation = revision(&transaction)? as u64;
            transaction.execute("UPDATE history SET ocr_generation=?1 WHERE id=?2", params![generation, id.to_string()])?;
            transaction.commit()?;
            Ok(OcrAdmission::Started(OcrTicket { record: id, generation, content_revision, configuration_revision }))
        }).await
    }
    pub async fn cancel_ocr(&self, ticket: OcrTicket) -> Result<bool> {
        self.call(move |connection| Ok(connection.execute("UPDATE history SET ocr_generation=0 WHERE id=?1 AND content_revision=?2 AND ocr_generation=?3", params![ticket.record.to_string(), ticket.content_revision, ticket.generation])? != 0)).await
    }
    pub async fn update_ocr(&self, ticket: OcrTicket, text: Arc<str>) -> Result<Option<u64>> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            if ocr_revision_in(&transaction)? != ticket.configuration_revision { return Ok(None); }
            let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM history WHERE id=?1 AND content_revision=?2 AND ocr_generation=?3)", params![ticket.record.to_string(), ticket.content_revision, ticket.generation], |row| row.get(0))?;
            if !exists { return Ok(None); }
            let revision = revision(&transaction)?;
            let has_text = text.chars().any(|character| !crate::preview::text_whitespace(character));
            transaction.execute("UPDATE payloads SET ocr=?1 WHERE record_id=?2", params![text.as_ref(), ticket.record.to_string()])?;
            transaction.execute("UPDATE history SET ocr_has_text=?1,ocr_revision=?2,ocr_generation=0 WHERE id=?3", params![has_text, revision, ticket.record.to_string()])?;
            transaction.commit()?;
            Ok(Some(revision as u64))
        }).await
    }

    pub async fn touch(&self, id: RecordId, timestamp: i64) -> Result<bool> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let revision = revision(&transaction)?;
            let changed = transaction.execute("UPDATE history SET last_used=MAX(last_used,?1),metadata_revision=?2 WHERE id=?3", params![timestamp, revision, id.to_string()])? > 0;
            transaction.commit()?; Ok(changed)
        }).await
    }

    pub async fn delete_record(&self, id: RecordId) -> Result<bool> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let deleted = transaction.execute("DELETE FROM history WHERE id=?1", [id.to_string()])? > 0;
            if deleted { collect_orphan_images(&transaction)?; revision(&transaction)?; }
            transaction.commit()?; Ok(deleted)
        }).await
    }

    /// User-facing clear preserves tagged records, matching the existing application.
    pub async fn clear_history(&self) -> Result<Vec<RecordId>> {
        self.call(|connection| {
            let transaction = connection.transaction()?;
            let ids = delete_returning(&transaction, "DELETE FROM history WHERE NOT EXISTS(SELECT 1 FROM record_tags rt WHERE rt.record_id=history.id) RETURNING id", [])?;
            if !ids.is_empty() { collect_orphan_images(&transaction)?; revision(&transaction)?; }
            transaction.commit()?; Ok(ids)
        }).await
    }

    pub async fn history_count(&self) -> Result<u64> {
        self.call(|connection| Ok(connection.query_row("SELECT count(*) FROM history", [], |row| row.get(0))?)).await
    }
    pub async fn record(&self, id: RecordId, cancel: &CancellationToken) -> Result<HistoryRecord> {
        self.call_read(cancel, move |connection| {
            full_records(connection, "h.id=?", vec![id.to_string().into()], " LIMIT 1")?.pop().ok_or(Error::NotFound)
        }).await
    }
    pub async fn image_metadata(&self, id: RecordId, cancel: &CancellationToken) -> Result<Option<ImageMetadata>> {
        self.call_read(cancel, move |connection| {
            connection.query_row("SELECT i.id AS image_id,i.encoded_bytes AS image_bytes,i.width AS image_width,i.height AS image_height FROM history h LEFT JOIN images i ON i.id=h.image_id WHERE h.id=?1", [id.to_string()], read_image_metadata)
                .optional()?.ok_or(Error::NotFound)
        }).await
    }
    /// Offset is retained only at the MCP protocol boundary; the visible history uses stable cursors.
    pub async fn search_records(&self, search: String, limit: u32, offset: u32, cancel: &CancellationToken) -> Result<Vec<HistoryRecord>> {
        if !(1..=100).contains(&limit) || offset > 1_000_000 || search.chars().count() > 1000 { return Err(Error::Invalid("历史搜索参数无效".into())); }
        self.call_read(cancel, move |connection| {
            let pattern = format!("%{}%", search.trim());
            let filter = if search.trim().is_empty() { "1=1" } else { "(p.text LIKE ? OR p.ocr LIKE ?)" };
            let mut values = Vec::new();
            if !search.trim().is_empty() { values.push(pattern.clone().into()); values.push(pattern.into()); }
            values.push(i64::from(limit).into()); values.push(i64::from(offset).into());
            full_records(connection, filter, values, " ORDER BY h.timestamp DESC,h.id DESC LIMIT ? OFFSET ?")
        }).await
    }
}

fn full_records(connection: &rusqlite::Connection, filter: &str, values: Vec<Value>, suffix: &str) -> Result<Vec<HistoryRecord>> {
    let sql = format!("SELECT {PROJECTION},p.text AS full_text,p.html AS full_html,p.rtf AS full_rtf,p.ocr AS full_ocr FROM history h JOIN payloads p ON p.record_id=h.id LEFT JOIN sources s ON s.id=h.source_id LEFT JOIN images i ON i.id=h.image_id WHERE {filter}{suffix}");
    let mut entries = Vec::new(); let mut contents = Vec::new();
    {
        let mut statement = connection.prepare(&sql)?; let mut rows = statement.query(params_from_iter(values))?;
        while let Some(row) = rows.next()? {
            let entry = read_entry(row)?;
            contents.push(FullContent { id: entry.id, revision: entry.content_revision, ocr_revision: entry.ocr_revision,
                content: TextContent { text: row.get("full_text")?, html: row.get("full_html")?, rtf: row.get("full_rtf")?, files: Arc::default() },
                ocr: row.get("full_ocr")?, image: entry.image.clone() });
            entries.push(entry);
        }
    }
    hydrate_page(connection, &mut entries)?;
    let mut records = entries.into_iter().zip(contents).map(|(entry, mut content)| {
        content.content.files = entry.excerpt.files.clone(); HistoryRecord { entry, content }
    }).collect::<Vec<_>>();
    let indices = records.iter().enumerate().filter(|(_, record)| record.entry.files_count > 4)
        .map(|(index, record)| (record.entry.id.to_string(), index)).collect::<HashMap<_, _>>();
    if !indices.is_empty() {
        let ids = indices.keys().collect::<Vec<_>>();
        let sql = format!("SELECT record_id,path FROM files WHERE record_id IN ({}) AND position>=4 ORDER BY record_id,position", placeholders(ids.len()));
        let mut tails = HashMap::<usize, Vec<String>>::new();
        let mut statement = connection.prepare(&sql)?; let mut rows = statement.query(params_from_iter(ids))?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let index = *indices.get(&id).ok_or_else(|| Error::Database("历史文件所属记录不一致".into()))?;
            tails.entry(index).or_default().push(row.get(1)?);
        }
        for (index, tail) in tails { let files = &mut records[index].content.content.files; *files = files.iter().cloned().chain(tail).collect::<Vec<_>>().into(); }
    }
    Ok(records)
}

fn read_entry(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    let primary_type: String = row.get("primary_type")?;
    let primary_type = PrimaryType::parse(&primary_type).ok_or(rusqlite::Error::InvalidQuery)?;
    let source = row.get::<_, Option<String>>("source_id")?.map(|id| {
        Ok::<_, rusqlite::Error>(Arc::new(SourceApp { id: parse_sql_id(&id)?, application: row.get::<_, String>("source_application")?.into(), path: row.get::<_, String>("source_path")?.into(), icon: None }))
    }).transpose()?;
    Ok(HistoryEntry {
        id: parse_sql_id(&row.get::<_, String>("id")?)?, primary_type,
        formats: Formats::from_bits(row.get("formats")?).ok_or(rusqlite::Error::InvalidQuery)?,
        timestamp: row.get("timestamp")?, last_used: row.get("last_used")?, content_revision: row.get("content_revision")?, metadata_revision: row.get("metadata_revision")?, ocr_revision: row.get("ocr_revision")?,
        text_characters: row.get("text_characters")?, files_count: row.get("files_count")?,
        excerpt: TextContent { text: row.get("text_excerpt")?, html: row.get("html_excerpt")?, rtf: row.get("rtf_excerpt")?, files: Arc::default() },
        has_ocr: row.get("ocr_has_text")?, truncated: Formats::from_bits(row.get("truncated")?).ok_or(rusqlite::Error::InvalidQuery)?,
        source, image: read_image_metadata(row)?, tags: Arc::default(),
    })
}

fn hydrate_page(connection: &rusqlite::Connection, entries: &mut [HistoryEntry]) -> Result<()> {
    if entries.is_empty() { return Ok(()); }
    // Fetch each source icon once per page, rather than copying its BLOB for
    // every history row. Entries share the complete source through one Arc.
    let source_ids: HashSet<_> = entries.iter().filter_map(|entry| entry.source.as_ref().map(|source| source.id)).collect();
    if !source_ids.is_empty() {
        let values: Vec<_> = source_ids.iter().map(ToString::to_string).collect();
        let sql = format!("SELECT id,application,path,icon FROM sources WHERE id IN ({})", placeholders(values.len()));
        let mut statement = connection.prepare(&sql)?;
        let sources = statement.query_map(params_from_iter(&values), |row| {
            let id: SourceId = parse_sql_id(&row.get::<_, String>(0)?)?;
            Ok((id, Arc::new(SourceApp { id, application: row.get::<_, String>(1)?.into(), path: row.get::<_, String>(2)?.into(), icon: row.get::<_, Option<Vec<u8>>>(3)?.map(bytes::Bytes::from) })))
        })?.collect::<rusqlite::Result<HashMap<_, _>>>()?;
        for entry in entries.iter_mut() {
            if let Some(source) = &entry.source { entry.source = Some(sources.get(&source.id).cloned().ok_or_else(|| Error::Database("来源应用记录不存在".into()))?); }
        }
    }
    let indices: HashMap<String, usize> = entries.iter().enumerate().map(|(i, entry)| (entry.id.to_string(), i)).collect();
    let ids: Vec<String> = entries.iter().map(|entry| entry.id.to_string()).collect();
    let mut tags = vec![Vec::new(); entries.len()];
    let mut files = vec![Vec::new(); entries.len()];
    {
        let sql = format!("SELECT rt.record_id,t.id,t.name,t.color,t.description,t.created_at,t.updated_at FROM record_tags rt JOIN tags t ON t.id=rt.tag_id WHERE rt.record_id IN ({}) ORDER BY rt.created_at ASC,t.id ASC", placeholders(ids.len()));
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(&ids))?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            if let Some(&index) = indices.get(&id) { tags[index].push(super::tags::read_tag_at(row, 1, 0)?); }
        }
    }
    {
        let sql = format!("SELECT record_id,path FROM files WHERE record_id IN ({}) AND position<4 ORDER BY record_id,position", placeholders(ids.len()));
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(&ids))?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            if let Some(&index) = indices.get(&id) { files[index].push(row.get::<_, String>(1)?); }
        }
    }
    for (index, entry) in entries.iter_mut().enumerate() {
        entry.tags = std::mem::take(&mut tags[index]).into();
        entry.excerpt.files = std::mem::take(&mut files[index]).into();
        entry.truncated.set(Formats::FILES, entry.files_count > 4);
    }
    Ok(())
}
fn read_image_metadata(row: &Row<'_>) -> rusqlite::Result<Option<ImageMetadata>> {
    row.get::<_, Option<Vec<u8>>>("image_id")?.map(|id| {
        let digest: [u8; 32] = id.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(ImageMetadata { id: Digest(digest), encoded_bytes: row.get("image_bytes")?, width: row.get("image_width")?, height: row.get("image_height")? })
    }).transpose()
}
pub(super) fn parse_sql_id<T: std::str::FromStr>(value: &str) -> rusqlite::Result<T> { value.parse().map_err(|_| rusqlite::Error::InvalidQuery) }
fn upsert_record_in(transaction: &rusqlite::Transaction<'_>, prepared: &PreparedRecord) -> Result<UpsertResult> {
    let revision = revision(transaction)?;
    let source_id = if let Some(source) = &prepared.source {
        let id = source.id.to_string();
        let changed = transaction.execute("INSERT INTO sources(id,application,path,icon) VALUES(?1,?2,?3,?4) ON CONFLICT(application,path) DO UPDATE SET icon=excluded.icon WHERE excluded.icon IS NOT NULL AND sources.icon IS NOT excluded.icon", params![id, source.application.as_ref(), source.path.as_ref(), source.icon.as_deref()])? > 0;
        let id: String = transaction.query_row("SELECT id FROM sources WHERE application=?1 AND path=?2", params![source.application.as_ref(), source.path.as_ref()], |row| row.get(0))?;
        // Shared source icons are part of hydrated summaries, so their changes
        // must invalidate every referring row's metadata identity as well.
        if changed { transaction.execute("UPDATE history SET metadata_revision=?1 WHERE source_id=?2", params![revision, id])?; }
        Some(id)
    } else { None };
    let existing: Option<(String, i64)> = transaction.query_row("SELECT id,last_used FROM history WHERE digest=?1", [prepared.digest.0.as_slice()], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    if let Some((id, last_used)) = existing {
        transaction.execute("UPDATE history SET last_used=MAX(last_used,?1),source_id=COALESCE(?2,source_id),metadata_revision=?3 WHERE id=?4", params![prepared.timestamp, source_id, revision, id])?;
        return Ok(UpsertResult { id: parse_id(id)?, inserted: false, last_used: last_used.max(prepared.timestamp) });
    }
    let id = RecordId::new();
    let id_text = id.to_string();
    if let Some(image) = &prepared.image {
        let dimensions = image.dimensions();
        transaction.execute("INSERT OR IGNORE INTO images(id,format,width,height,encoded_bytes,data) VALUES(?1,?2,?3,?4,?5,?6)", params![image.id().0.as_slice(), image.format_name(), dimensions.map(|v| v.0), dimensions.map(|v| v.1), image.bytes().len() as i64, image.bytes()])?;
    }
    let text = &prepared.content;
    let (text_excerpt, text_truncated) = excerpt(text.text.as_deref(), 2048);
    let (html_excerpt, html_truncated) = complete_if_short(text.html.as_deref(), 16384);
    let (rtf_excerpt, rtf_truncated) = complete_if_short(text.rtf.as_deref(), 16384);
    let mut truncated = Formats::empty();
    truncated.set(Formats::TEXT, text_truncated); truncated.set(Formats::HTML, html_truncated); truncated.set(Formats::RTF, rtf_truncated);
    let image_id = prepared.image.as_ref().map(|v| v.id().0.to_vec());
    transaction.execute("INSERT INTO history(id,digest,primary_type,formats,timestamp,last_used,content_revision,metadata_revision,source_id,image_id,text_characters,files_count,text_excerpt,html_excerpt,rtf_excerpt,truncated) VALUES(?1,?2,?3,?4,?5,?5,?6,?6,?7,?8,?9,?10,?11,?12,?13,?14)", params![id_text, prepared.digest.0.as_slice(), text.primary_type(prepared.image.is_some()).as_str(), text.formats(prepared.image.is_some()).bits(), prepared.timestamp, revision, source_id, image_id, text.text.as_deref().map_or(0, |v| v.chars().count()) as i64, text.files.len() as i64, text_excerpt, html_excerpt, rtf_excerpt, truncated.bits()])?;
    transaction.execute("INSERT INTO payloads(record_id,text,html,rtf) VALUES(?1,?2,?3,?4)", params![id_text, text.text.as_deref(), text.html.as_deref(), text.rtf.as_deref()])?;
    {
        let mut statement = transaction.prepare_cached("INSERT INTO files(record_id,position,path) VALUES(?1,?2,?3)")?;
        for (index, path) in text.files.iter().enumerate() { statement.execute(params![id_text, index as i64, path])?; }
    }
    Ok(UpsertResult { id, inserted: true, last_used: prepared.timestamp })
}

fn apply_history_limits(transaction: &rusqlite::Transaction<'_>, limits: HistoryLimits, timestamp: i64) -> Result<Vec<RecordId>> {
    let mut removed = Vec::new();
    if limits.keep_days > 0 {
        let cutoff = timestamp.saturating_sub(limits.keep_days.saturating_mul(86_400_000));
        removed.extend(delete_returning(transaction, "DELETE FROM history WHERE timestamp<?1 AND NOT EXISTS(SELECT 1 FROM record_tags rt WHERE rt.record_id=history.id) RETURNING id", [cutoff])?);
    }
    if limits.max_records > 0 {
        // The original limit excludes protected records from the candidate window.
        removed.extend(delete_returning(transaction, "DELETE FROM history WHERE id IN (SELECT id FROM history WHERE NOT EXISTS(SELECT 1 FROM record_tags rt WHERE rt.record_id=history.id) ORDER BY last_used DESC,id DESC LIMIT -1 OFFSET ?1) RETURNING id", [limits.max_records])?);
    }
    // Inserts and duplicate touches cannot orphan an image. Scan the image
    // table only when this transaction actually removed history references.
    if !removed.is_empty() { collect_orphan_images(transaction)?; }
    Ok(removed)
}

fn parse_id<T: std::str::FromStr>(value: String) -> Result<T> { value.parse().map_err(|_| Error::Database("数据库记录标识无效".into())) }
fn placeholders(count: usize) -> String { std::iter::repeat_n("?", count).collect::<Vec<_>>().join(",") }
fn excerpt(value: Option<&str>, max: usize) -> (Option<String>, bool) {
    let Some(value) = value else { return (None, false) };
    let end = value.char_indices().nth(max).map(|(position, _)| position);
    (Some(value[..end.unwrap_or(value.len())].to_owned()), end.is_some())
}
fn complete_if_short(value: Option<&str>, max: usize) -> (Option<&str>, bool) {
    let truncated = value.is_some_and(|value| value.chars().count() > max);
    (if truncated { None } else { value }, truncated)
}
fn delete_returning<P: rusqlite::Params>(transaction: &rusqlite::Transaction<'_>, sql: &str, params: P) -> Result<Vec<RecordId>> {
    let mut statement = transaction.prepare(sql)?;
    let ids = statement.query_map(params, |row| parse_sql_id(&row.get::<_, String>(0)?))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}
