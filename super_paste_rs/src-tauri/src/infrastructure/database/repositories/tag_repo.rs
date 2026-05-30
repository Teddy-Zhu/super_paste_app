use rusqlite::{params, Connection};

use crate::result::Result;
use crate::types::clipboard::Tag;

pub struct TagRepository;

impl TagRepository {
    pub fn insert(conn: &Connection, tag: &Tag) -> Result<()> {
        conn.execute(
            "INSERT INTO tags (id, name, color, icon, description, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                tag.id,
                tag.name,
                tag.color,
                tag.icon,
                tag.description,
                tag.sort_order,
                tag.created_at,
                tag.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn list(conn: &Connection) -> Result<Vec<Tag>> {
        let mut stmt = conn.prepare(
            "SELECT id, name, color, icon, description, sort_order, created_at, updated_at
             FROM tags ORDER BY sort_order, name",
        )?;

        let tags = stmt
            .query_map([], Self::map_tag)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tags)
    }

    pub fn update(conn: &Connection, tag: &Tag) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE tags SET name = ?, color = ?, icon = ?, description = ?, sort_order = ?, updated_at = ?
             WHERE id = ?",
            params![
                tag.name,
                tag.color,
                tag.icon,
                tag.description,
                tag.sort_order,
                now,
                tag.id
            ],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, id: &str) -> Result<()> {
        conn.execute("DELETE FROM tags WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn add_to_item(conn: &Connection, content_id: &str, tag_id: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO item_tags (content_id, tag_id, created_at) VALUES (?1, ?2, ?3)",
            params![content_id, tag_id, now],
        )?;
        Ok(())
    }

    pub fn remove_from_item(conn: &Connection, content_id: &str, tag_id: &str) -> Result<()> {
        conn.execute(
            "DELETE FROM item_tags WHERE content_id = ? AND tag_id = ?",
            params![content_id, tag_id],
        )?;
        Ok(())
    }

    pub fn get_item_tags(conn: &Connection, content_id: &str) -> Result<Vec<Tag>> {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.name, t.color, t.icon, t.description, t.sort_order, t.created_at, t.updated_at
             FROM tags t
             INNER JOIN item_tags it ON t.id = it.tag_id
             WHERE it.content_id = ?
             ORDER BY t.sort_order, t.name",
        )?;

        let tags = stmt
            .query_map([content_id], Self::map_tag)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(tags)
    }

    fn map_tag(row: &rusqlite::Row) -> rusqlite::Result<Tag> {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            icon: row.get(3)?,
            description: row.get(4)?,
            sort_order: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}
