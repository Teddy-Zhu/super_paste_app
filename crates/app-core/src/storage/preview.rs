use std::sync::Arc;
use rusqlite::{OptionalExtension, params, types::{FromSql, FromSqlError, FromSqlResult, ValueRef}};
use tokio_util::sync::CancellationToken;
use super_paste_format::{HistoryEntry, RecordId, TextContent};
use crate::{Error, Result, preview::{Input, PreviewScope, excerpt, rich}, resources::{ByteBudget, Reservation}};
use super::Storage;

const CARD_TEXT_UNITS: usize = 20_000;
// One UTF-16 unit needs at most three UTF-8 bytes, including embedded NUL.
const CARD_TEXT_BYTES: usize = CARD_TEXT_UNITS * 3;
const RICH_TEXT_BYTES: usize = rich::MAX_SOURCE_UNITS * 3;

struct CardText<const TRUNCATED: bool>(Arc<str>);
impl<const TRUNCATED: bool> FromSql for CardText<TRUNCATED> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let ValueRef::Text(bytes) = value else { return Err(FromSqlError::InvalidType); };
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            // SQLite's bounded byte prefix can end inside a character. At this
            // byte bound that incomplete suffix cannot fit the UTF-16 limit.
            Err(error) if TRUNCATED && bytes.len() == CARD_TEXT_BYTES && error.error_len().is_none() => {
                std::str::from_utf8(&bytes[..error.valid_up_to()]).map_err(|error| FromSqlError::Other(Box::new(error)))?
            }
            Err(error) => return Err(FromSqlError::Other(Box::new(error))),
        };
        Ok(Self(Arc::from(excerpt(text, CARD_TEXT_UNITS))))
    }
}

enum RichText { Included(Arc<str>), Omitted }
impl FromSql for RichText {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        Ok(if !rich::source_within_limit(text) {
            Self::Omitted
        } else { Self::Included(Arc::from(text)) })
    }
}
impl RichText {
    fn into_content(self) -> Option<Arc<str>> { match self { Self::Included(text) => Some(text), Self::Omitted => None } }
}

impl Storage {
    pub(crate) async fn external_link_input(&self, id: RecordId, memory: ByteBudget) -> Result<(u64, Arc<str>, Reservation)> {
        self.call(move |connection| {
            let mut statement = connection.prepare("SELECT h.content_revision,COALESCE(length(CAST(p.text AS BLOB)),0),p.text FROM history h JOIN payloads p ON p.record_id=h.id WHERE h.id=?1")?;
            let mut rows = statement.query([id.to_string()])?;
            let row = rows.next()?.ok_or(Error::NotFound)?;
            let bytes: u64 = row.get(1)?;
            // Cover original text and temporary URL normalization/IDNA work
            // before making a Rust allocation. The returned link shrinks this
            // estimate after parsing and keeps only the original shared text.
            let lease = memory.reserve(bytes.saturating_mul(4).saturating_add(4096))?;
            let text = row.get::<_, Option<Arc<str>>>(2)?.ok_or_else(|| Error::Invalid("链接内容无效".into()))?;
            Ok((row.get(0)?, text, lease))
        }).await
    }
    pub async fn record_revision(&self, id: RecordId, cancel: &CancellationToken) -> Result<Option<u64>> {
        self.call_read(cancel, move |connection| Ok(connection.query_row("SELECT content_revision FROM history WHERE id=?1", [id.to_string()], |row| row.get(0)).optional()?)).await
    }
    pub(crate) async fn preview_input(&self, entry: Arc<HistoryEntry>, scope: PreviewScope, memory: ByteBudget, cancel: &CancellationToken) -> Result<Input> {
        let requested = cancel.clone();
        self.call_read(cancel, move |connection| {
            let transaction = connection.unchecked_transaction()?;
            let id = entry.id.to_string(); let card = scope == PreviewScope::Card;
            // Inspect byte lengths before creating Rust payload allocations.
            // The rich byte bound excludes definitely oversized values; the
            // borrowed row then checks the actual UTF-16 limit before sharing.
            let header = transaction.query_row("SELECT h.content_revision,COALESCE(length(CAST(p.text AS BLOB)),0),COALESCE(length(CAST(p.html AS BLOB)),0),COALESCE(length(CAST(p.rtf AS BLOB)),0) FROM history h JOIN payloads p ON p.record_id=h.id WHERE h.id=?1", [&id], |row| {
                Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?, row.get::<_, u64>(2)?, row.get::<_, u64>(3)?))
            }).optional()?.ok_or(Error::NotFound)?;
            if header.0 != entry.content_revision { return Err(Error::Cancelled); }
            if requested.is_cancelled() { return Err(Error::Cancelled); }
            let html_too_large = header.2 > RICH_TEXT_BYTES as u64; let rtf_too_large = header.3 > RICH_TEXT_BYTES as u64;
            // Canonical URI/path deduplication must precede the four-row
            // display limit. Reserve raw input, normalization keys and output
            // scratch before reading; only the resulting visible rows persist.
            let (file_bytes, file_count): (u64, u64) = transaction.query_row("SELECT COALESCE(SUM(length(CAST(path AS BLOB))),0),COUNT(*) FROM files WHERE record_id=?1", [&id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let file_bytes = file_bytes.saturating_mul(8).saturating_add(file_count.saturating_mul(256));
            let bytes = (if card { header.1.min(CARD_TEXT_BYTES as u64) } else { header.1 }).saturating_add(if html_too_large { 0 } else { header.2 }).saturating_add(if rtf_too_large { 0 } else { header.3 }).saturating_add(file_bytes).saturating_add(4096);
            let lease = memory.reserve(bytes)?;
            if requested.is_cancelled() { return Err(Error::Cancelled); }
            let (mut content, html_omitted, rtf_omitted) = transaction.query_row("SELECT CASE WHEN ?2 AND typeof(text)='text' THEN CAST(substr(CAST(text AS BLOB),1,?3) AS TEXT) ELSE text END,CASE WHEN length(CAST(html AS BLOB))<=?4 THEN html ELSE NULL END,CASE WHEN length(CAST(rtf AS BLOB))<=?4 THEN rtf ELSE NULL END FROM payloads WHERE record_id=?1", params![id, card, CARD_TEXT_BYTES as u32, RICH_TEXT_BYTES as u32], |row| {
                let text = if !card { row.get(0)? }
                    else if header.1 > CARD_TEXT_BYTES as u64 { row.get::<_, Option<CardText<true>>>(0)?.map(|text| text.0) }
                    else { row.get::<_, Option<CardText<false>>>(0)?.map(|text| text.0) };
                let html: Option<RichText> = row.get(1)?; let rtf: Option<RichText> = row.get(2)?;
                let html_omitted = html_too_large || matches!(&html, Some(RichText::Omitted));
                let rtf_omitted = rtf_too_large || matches!(&rtf, Some(RichText::Omitted));
                Ok((TextContent { text, html: html.and_then(RichText::into_content), rtf: rtf.and_then(RichText::into_content), files: Arc::default() }, html_omitted, rtf_omitted))
            })?;
            if requested.is_cancelled() { return Err(Error::Cancelled); }
            {
                let mut statement = transaction.prepare("SELECT path FROM files WHERE record_id=?1 ORDER BY position")?;
                content.files = statement.query_map([&id], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?.into();
            }
            let text_clipped = card && content.text.as_ref().is_some_and(|text| (text.chars().count() as u64) < entry.text_characters);
            transaction.commit()?;
            Ok(Input { content, primary_type: entry.primary_type, image: entry.image.clone(), text_characters: entry.text_characters, text_clipped, html_omitted, rtf_omitted, scope, lease })
        }).await
    }
}
