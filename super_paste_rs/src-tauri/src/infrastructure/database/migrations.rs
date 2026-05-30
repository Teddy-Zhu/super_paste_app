use rusqlite::Connection;

use crate::result::Result;

pub struct MigrationManager;

impl MigrationManager {
    pub fn run_migrations(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            ) STRICT;",
        )?;

        let current_version = Self::get_current_version(conn)?;
        let migrations = Self::get_migrations();

        for (version, sql) in migrations {
            if version > current_version {
                tracing::info!("应用迁移 v{}", version);
                conn.execute_batch(sql)?;
                Self::record_migration(conn, version)?;
            }
        }

        tracing::info!(
            "数据库迁移完成，当前版本: v{}",
            Self::get_current_version(conn)?
        );
        Ok(())
    }

    fn get_current_version(conn: &Connection) -> Result<i32> {
        let version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(version)
    }

    fn record_migration(conn: &Connection, version: i32) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![version, now],
        )?;
        Ok(())
    }

    fn get_migrations() -> Vec<(i32, &'static str)> {
        vec![
            (1, include_str!("../../../migrations/001_initial_schema.sql")),
            (2, include_str!("../../../migrations/002_add_indexes.sql")),
            (3, include_str!("../../../migrations/003_add_fts.sql")),
        ]
    }
}
