use rusqlite::{Transaction, params, types::ValueRef};
use crate::{Error, Result, preview::text_whitespace};

/// Upgrade this application's v1 derived OCR metadata. Identity validation and
/// the enclosing transaction belong to Storage::open; failures roll back DDL
/// and data together. Only bounded ID/boolean batches cross statement scopes.
pub(super) fn ocr_presence(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute_batch("ALTER TABLE history ADD COLUMN ocr_has_text INTEGER NOT NULL DEFAULT 0 CHECK(ocr_has_text IN (0,1))")?;
    let mut after = String::new();
    loop {
        let batch = {
            let mut statement = transaction.prepare_cached("SELECT record_id,ocr FROM payloads WHERE record_id>?1 AND ocr IS NOT NULL ORDER BY record_id LIMIT 128")?;
            let mut rows = statement.query([&after])?;
            let mut batch = Vec::new();
            while let Some(row) = rows.next()? {
                let has_text = match row.get_ref(1)? {
                    ValueRef::Text(bytes) => std::str::from_utf8(bytes)
                        .map_err(|_| Error::Database("OCR 原文不是有效 UTF-8".into()))?
                        .chars().any(|character| !text_whitespace(character)),
                    _ => return Err(Error::Database("OCR 原文类型无效".into())),
                };
                batch.push((row.get::<_, String>(0)?, has_text));
            }
            batch
        };
        if batch.is_empty() { break; }
        let mut update = transaction.prepare_cached("UPDATE history SET ocr_has_text=?2 WHERE id=?1")?;
        for (id, has_text) in batch {
            update.execute(params![id, has_text])?;
            after = id;
        }
    }
    // Both columns contain only rebuildable excerpts; the authoritative OCR
    // payload, record revisions, indexes and references remain intact.
    transaction.execute_batch("ALTER TABLE history DROP COLUMN ocr_excerpt; ALTER TABLE history DROP COLUMN ocr_truncated;")?;
    Ok(())
}
