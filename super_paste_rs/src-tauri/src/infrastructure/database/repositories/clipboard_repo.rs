use rusqlite::{params, Connection};

use crate::result::Result;
use crate::types::clipboard::{ClipboardContent, ClipboardItem, QueryOptions};

pub struct ClipboardRepository;

impl ClipboardRepository {
    pub fn insert(
        conn: &Connection,
        item: &ClipboardItem,
        content: &ClipboardContent,
    ) -> Result<i64> {
        let formats_json = serde_json::to_string(&item.available_formats)?;

        conn.execute(
            "INSERT INTO clipboard_items (
                content_id, content_hash, primary_type, data_size, available_formats,
                source_app_id, is_pinned, is_favorite, created_at, last_used_at, use_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                item.content_id,
                item.content_hash,
                item.primary_type,
                item.data_size,
                formats_json,
                item.source_app_id,
                item.is_pinned as i32,
                item.is_favorite as i32,
                item.created_at,
                item.last_used_at,
                item.use_count
            ],
        )?;

        let id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO clipboard_content (
                content_id, text_content, html_content, rtf_content,
                image_data, files_json, metadata_json, compression_type
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                content.content_id,
                content.text_content,
                content.html_content,
                content.rtf_content,
                content.image_data,
                content.files_json,
                content.metadata_json,
                content.compression_type
            ],
        )?;

        Ok(id)
    }

    pub fn list(conn: &Connection, options: &QueryOptions) -> Result<Vec<ClipboardItem>> {
        let mut sql = String::from(
            "SELECT id, content_id, content_hash, primary_type, data_size, available_formats,
                    source_app_id, is_pinned, is_favorite, created_at, last_used_at, use_count
             FROM clipboard_items WHERE 1=1",
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref t) = options.primary_type {
            sql.push_str(" AND primary_type = ?");
            params.push(Box::new(t.clone()));
        }

        if let Some(ref tag_id) = options.tag_id {
            sql.push_str(" AND content_id IN (SELECT content_id FROM item_tags WHERE tag_id = ?)");
            params.push(Box::new(tag_id.clone()));
        }

        if let Some(pinned) = options.is_pinned {
            sql.push_str(" AND is_pinned = ?");
            params.push(Box::new(pinned as i32));
        }

        if let Some(favorite) = options.is_favorite {
            sql.push_str(" AND is_favorite = ?");
            params.push(Box::new(favorite as i32));
        }

        if let Some(start) = options.start_date {
            sql.push_str(" AND created_at >= ?");
            params.push(Box::new(start));
        }

        if let Some(end) = options.end_date {
            sql.push_str(" AND created_at <= ?");
            params.push(Box::new(end));
        }

        sql.push_str(" ORDER BY is_pinned DESC, created_at DESC LIMIT ? OFFSET ?");
        params.push(Box::new(options.limit));
        params.push(Box::new(options.offset));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let items = stmt
            .query_map(param_refs.as_slice(), Self::map_item)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(items)
    }

    pub fn get_by_id(
        conn: &Connection,
        id: i64,
    ) -> Result<Option<(ClipboardItem, ClipboardContent)>> {
        let mut stmt = conn.prepare(
            "SELECT i.id, i.content_id, i.content_hash, i.primary_type, i.data_size,
                    i.available_formats, i.source_app_id, i.is_pinned, i.is_favorite,
                    i.created_at, i.last_used_at, i.use_count,
                    c.text_content, c.html_content, c.rtf_content, c.image_data,
                    c.files_json, c.metadata_json, c.compression_type
             FROM clipboard_items i
             LEFT JOIN clipboard_content c ON i.content_id = c.content_id
             WHERE i.id = ?",
        )?;

        let result = stmt.query_row([id], |row| {
            let formats_json: String = row.get(5)?;
            let formats: Vec<String> = serde_json::from_str(&formats_json).unwrap_or_default();

            let item = ClipboardItem {
                id: Some(row.get(0)?),
                content_id: row.get(1)?,
                content_hash: row.get(2)?,
                primary_type: row.get(3)?,
                data_size: row.get(4)?,
                available_formats: formats,
                source_app_id: row.get(6)?,
                is_pinned: row.get::<_, i32>(7)? != 0,
                is_favorite: row.get::<_, i32>(8)? != 0,
                created_at: row.get(9)?,
                last_used_at: row.get(10)?,
                use_count: row.get(11)?,
            };

            let content = ClipboardContent {
                content_id: row.get(1)?,
                text_content: row.get(12)?,
                html_content: row.get(13)?,
                rtf_content: row.get(14)?,
                image_data: row.get(15)?,
                files_json: row.get(16)?,
                metadata_json: row.get(17)?,
                compression_type: row.get(18)?,
            };

            Ok((item, content))
        });

        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn search(conn: &Connection, query: &str, limit: i64) -> Result<Vec<ClipboardItem>> {
        let mut stmt = conn.prepare(
            "SELECT i.id, i.content_id, i.content_hash, i.primary_type, i.data_size,
                    i.available_formats, i.source_app_id, i.is_pinned, i.is_favorite,
                    i.created_at, i.last_used_at, i.use_count
             FROM clipboard_items i
             INNER JOIN clipboard_fts f ON i.content_id = f.content_id
             WHERE clipboard_fts MATCH ?
             ORDER BY rank
             LIMIT ?",
        )?;

        let items = stmt
            .query_map(params![query, limit], Self::map_item)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(items)
    }

    pub fn update_last_used(conn: &Connection, id: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE clipboard_items SET last_used_at = ?, use_count = use_count + 1 WHERE id = ?",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn update_flags(
        conn: &Connection,
        id: i64,
        is_pinned: Option<bool>,
        is_favorite: Option<bool>,
    ) -> Result<()> {
        if let Some(pinned) = is_pinned {
            conn.execute(
                "UPDATE clipboard_items SET is_pinned = ? WHERE id = ?",
                params![pinned as i32, id],
            )?;
        }

        if let Some(favorite) = is_favorite {
            conn.execute(
                "UPDATE clipboard_items SET is_favorite = ? WHERE id = ?",
                params![favorite as i32, id],
            )?;
        }

        Ok(())
    }

    pub fn delete(conn: &Connection, id: i64) -> Result<()> {
        conn.execute("DELETE FROM clipboard_items WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn clear_all(conn: &Connection) -> Result<usize> {
        let count = conn.execute(
            "DELETE FROM clipboard_items WHERE is_pinned = 0 AND is_favorite = 0",
            [],
        )?;
        Ok(count)
    }

    pub fn delete_old(conn: &Connection, before_timestamp: i64) -> Result<usize> {
        let count = conn.execute(
            "DELETE FROM clipboard_items WHERE created_at < ? AND is_pinned = 0 AND is_favorite = 0",
            [before_timestamp],
        )?;
        Ok(count)
    }

    pub fn count(conn: &Connection) -> Result<i64> {
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM clipboard_items", [], |row| row.get(0))?;
        Ok(count)
    }

    pub fn get_by_hash(conn: &Connection, hash: &str) -> Result<Option<i64>> {
        let result = conn.query_row(
            "SELECT id FROM clipboard_items WHERE content_hash = ? ORDER BY created_at DESC LIMIT 1",
            [hash],
            |row| row.get(0),
        );

        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn map_item(row: &rusqlite::Row) -> rusqlite::Result<ClipboardItem> {
        let formats_json: String = row.get(5)?;
        let formats: Vec<String> = serde_json::from_str(&formats_json).unwrap_or_default();

        Ok(ClipboardItem {
            id: Some(row.get(0)?),
            content_id: row.get(1)?,
            content_hash: row.get(2)?,
            primary_type: row.get(3)?,
            data_size: row.get(4)?,
            available_formats: formats,
            source_app_id: row.get(6)?,
            is_pinned: row.get::<_, i32>(7)? != 0,
            is_favorite: row.get::<_, i32>(8)? != 0,
            created_at: row.get(9)?,
            last_used_at: row.get(10)?,
            use_count: row.get(11)?,
        })
    }
}
