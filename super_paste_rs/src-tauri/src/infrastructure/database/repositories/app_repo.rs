use rusqlite::{params, Connection};

use crate::result::Result;
use crate::types::clipboard::AppInfo;

pub struct AppRepository;

impl AppRepository {
    pub fn upsert(conn: &Connection, app: &AppInfo) -> Result<()> {
        conn.execute(
            "INSERT INTO apps (id, name, bundle_id, executable_path, icon_hash, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                name = ?2,
                last_seen_at = ?7",
            params![
                app.id,
                app.name,
                app.bundle_id,
                app.executable_path,
                app.icon_hash,
                app.first_seen_at,
                app.last_seen_at
            ],
        )?;
        Ok(())
    }

    pub fn get_by_id(conn: &Connection, id: &str) -> Result<Option<AppInfo>> {
        let result = conn.query_row(
            "SELECT id, name, bundle_id, executable_path, icon_hash, first_seen_at, last_seen_at
             FROM apps WHERE id = ?",
            [id],
            Self::map_app,
        );

        match result {
            Ok(app) => Ok(Some(app)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list(conn: &Connection) -> Result<Vec<AppInfo>> {
        let mut stmt = conn.prepare(
            "SELECT id, name, bundle_id, executable_path, icon_hash, first_seen_at, last_seen_at
             FROM apps ORDER BY last_seen_at DESC",
        )?;

        let apps = stmt
            .query_map([], Self::map_app)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(apps)
    }

    fn map_app(row: &rusqlite::Row) -> rusqlite::Result<AppInfo> {
        Ok(AppInfo {
            id: row.get(0)?,
            name: row.get(1)?,
            bundle_id: row.get(2)?,
            executable_path: row.get(3)?,
            icon_hash: row.get(4)?,
            first_seen_at: row.get(5)?,
            last_seen_at: row.get(6)?,
        })
    }
}
