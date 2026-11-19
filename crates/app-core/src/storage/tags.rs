use std::sync::Arc;
use rusqlite::{OptionalExtension, Row, params};
use super_paste_format::{RecordId, Tag, TagId};
use tokio_util::sync::CancellationToken;
use crate::{Error, Result};
use super::{Storage, history::parse_sql_id, revision};

#[derive(Clone, Debug, Default)]
pub struct TagChanges {
    pub name: Option<String>, pub color: Option<String>, pub description: Option<Option<String>>,
}
impl Storage {
    pub async fn tags(&self, cancel: &CancellationToken) -> Result<Vec<Tag>> {
        let requested = cancel.clone();
        self.call_read(cancel, move |connection| {
            let mut statement = connection.prepare("SELECT t.id,t.name,t.color,t.description,t.created_at,t.updated_at,(SELECT count(*) FROM record_tags rt WHERE rt.tag_id=t.id) FROM tags t ORDER BY t.created_at DESC,t.id DESC")?;
            let mut rows = statement.query([])?;
            let mut tags = Vec::new();
            while let Some(row) = rows.next()? {
                if requested.is_cancelled() { return Err(Error::Cancelled); }
                tags.push(read_tag_at(row, 0, row.get(6)?)?);
            }
            Ok(tags)
        }).await
    }
    pub async fn create_tag(&self, name: String, color: String, description: Option<String>, timestamp: i64) -> Result<TagId> {
        validate_name(&name)?;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            ensure_unique_name(&transaction, &name, None)?;
            let id = TagId::new();
            transaction.execute("INSERT INTO tags(id,name,color,description,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)", params![id.to_string(), name, color, description, timestamp])?;
            revision(&transaction)?;
            transaction.commit()?;
            Ok(id)
        }).await
    }
    pub async fn update_tag(&self, id: TagId, changes: TagChanges, timestamp: i64) -> Result<bool> {
        if let Some(name) = &changes.name { validate_name(name)?; }
        if changes.name.is_none() && changes.color.is_none() && changes.description.is_none() { return Ok(false); }
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            if let Some(name) = &changes.name { ensure_unique_name(&transaction, name, Some(id))?; }
            let changed = transaction.execute("UPDATE tags SET name=COALESCE(?1,name),color=COALESCE(?2,color),description=CASE WHEN ?3 THEN ?4 ELSE description END,updated_at=?5 WHERE id=?6", params![changes.name, changes.color, changes.description.is_some(), changes.description.flatten(), timestamp, id.to_string()])? > 0;
            if changed {
                let version = revision(&transaction)?;
                transaction.execute("UPDATE history SET metadata_revision=?1 WHERE id IN (SELECT record_id FROM record_tags WHERE tag_id=?2)", params![version, id.to_string()])?;
            }
            transaction.commit()?;
            Ok(changed)
        }).await
    }
    pub async fn delete_tag(&self, id: TagId) -> Result<bool> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM tags WHERE id=?1)", [id.to_string()], |row| row.get(0))?;
            if !exists { return Ok(false); }
            let version = revision(&transaction)?;
            transaction.execute("UPDATE history SET metadata_revision=?1 WHERE id IN (SELECT record_id FROM record_tags WHERE tag_id=?2)", params![version, id.to_string()])?;
            transaction.execute("DELETE FROM tags WHERE id=?1", [id.to_string()])?;
            transaction.commit()?;
            Ok(true)
        }).await
    }
    pub async fn bind_tag(&self, record: RecordId, tag: TagId, timestamp: i64) -> Result<bool> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let valid: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM history WHERE id=?1) AND EXISTS(SELECT 1 FROM tags WHERE id=?2)", params![record.to_string(), tag.to_string()], |row| row.get(0))?;
            if !valid { return Err(Error::NotFound); }
            let changed = transaction.execute("INSERT OR IGNORE INTO record_tags(record_id,tag_id,created_at) VALUES(?1,?2,?3)", params![record.to_string(), tag.to_string(), timestamp])? > 0;
            if changed {
                let version = revision(&transaction)?;
                transaction.execute("UPDATE history SET metadata_revision=?1 WHERE id=?2", params![version, record.to_string()])?;
            }
            transaction.commit()?;
            Ok(changed)
        }).await
    }
    pub async fn unbind_tag(&self, record: RecordId, tag: TagId, timestamp: i64) -> Result<bool> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let changed = transaction.execute("DELETE FROM record_tags WHERE record_id=?1 AND tag_id=?2", params![record.to_string(), tag.to_string()])? > 0;
            if changed {
                let version = revision(&transaction)?;
                let remaining: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM record_tags WHERE record_id=?1)", [record.to_string()], |row| row.get(0))?;
                if remaining {
                    transaction.execute("UPDATE history SET metadata_revision=?1 WHERE id=?2", params![version, record.to_string()])?;
                } else {
                    // Removing the final protection re-enters normal history at the current time.
                    transaction.execute("UPDATE history SET timestamp=?1,last_used=?1,metadata_revision=?2 WHERE id=?3", params![timestamp, version, record.to_string()])?;
                }
            }
            transaction.commit()?;
            Ok(changed)
        }).await
    }
}

pub(super) fn read_tag_at(row: &Row<'_>, offset: usize, count: u64) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: parse_sql_id(&row.get::<_, String>(offset)?)?, name: Arc::from(row.get::<_, String>(offset + 1)?),
        color: Arc::from(row.get::<_, String>(offset + 2)?), description: row.get::<_, Option<String>>(offset + 3)?.map(Arc::from),
        created_at: row.get(offset + 4)?, updated_at: row.get(offset + 5)?, count,
    })
}
fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() { return Err(Error::Invalid("标签名称不能为空".into())); }
    Ok(())
}
fn ensure_unique_name(transaction: &rusqlite::Transaction<'_>, name: &str, excluding: Option<TagId>) -> Result<()> {
    let existing: Option<String> = transaction.query_row("SELECT id FROM tags WHERE name=?1", [name], |row| row.get(0)).optional()?;
    if existing.is_some_and(|id| Some(id) != excluding.map(|id| id.to_string())) { return Err(Error::Invalid("标签名称已存在".into())); }
    Ok(())
}
