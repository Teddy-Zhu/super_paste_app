use rusqlite::{params, Connection};

use crate::result::Result;

pub struct ConfigRepository;

impl ConfigRepository {
    pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
        let result = conn.query_row(
            "SELECT value FROM app_config WHERE key = ?",
            [key],
            |row| row.get(0),
        );

        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set(conn: &Connection, key: &str, value: &str, value_type: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO app_config (key, value, value_type, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value = ?2, value_type = ?3, updated_at = ?4",
            params![key, value, value_type, now],
        )?;
        Ok(())
    }

    pub fn delete(conn: &Connection, key: &str) -> Result<()> {
        conn.execute("DELETE FROM app_config WHERE key = ?", [key])?;
        Ok(())
    }

    pub fn get_all(conn: &Connection) -> Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare("SELECT key, value FROM app_config")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
