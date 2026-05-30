use rusqlite::Connection;

use crate::infrastructure::database::repositories::ClipboardRepository;
use crate::result::Result;

pub struct Deduplicator;

impl Deduplicator {
    /// 若相同哈希的条目已存在，则更新其 last_used 并返回 true（命中去重）。
    pub fn update_if_exists(&self, conn: &Connection, hash: &str) -> Result<bool> {
        if let Some(id) = ClipboardRepository::get_by_hash(conn, hash)? {
            ClipboardRepository::update_last_used(conn, id)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
