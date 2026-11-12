mod apps;
mod clipboard;
mod commands;
mod settings;
mod statistics;
mod tags;
mod usage;

use std::{path::Path, sync::Arc};

use serde_json::Value;
use tokio_rusqlite::Connection;

pub use clipboard::{HistoryMetadata, UpsertClipboard};
pub use settings::SettingsSnapshot;

pub struct Storage {
    pub(crate) connection: Connection,
}

pub(crate) struct SourceApp {
    pub id: String,
    pub application: String,
    pub path: String,
    pub icon: Option<String>,
}

impl Storage {
    pub async fn open(data_dir: &Path) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|error| format!("无法创建应用数据目录: {error}"))?;
        let database_path = data_dir.join(data_format::DATABASE_FILE);
        let connection = Connection::open(database_path)
            .await
            .map_err(|error| format!("无法打开应用数据库: {error}"))?;
        connection
            .call(|connection| {
                let schema_version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
                if schema_version > data_format::FORMAT_VERSION as i64 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                connection.busy_timeout(std::time::Duration::from_secs(5))?;
                connection.pragma_update(None, "journal_mode", "WAL")?;
                connection.pragma_update(None, "synchronous", "NORMAL")?;
                connection.pragma_update(None, "foreign_keys", "ON")?;
                connection.execute_batch(data_format::INITIAL_SCHEMA)?;
                connection.pragma_update(None, "user_version", data_format::FORMAT_VERSION)?;
                let defaults = data_format::default_preferences();
                let now = now_ms();
                for root in ["settings", "sync"] {
                    let value = defaults
                        .get(root)
                        .ok_or(rusqlite::Error::InvalidQuery)?;
                    connection.execute(
                        "INSERT OR IGNORE INTO app_settings(key,value_json,updated_at) VALUES(?1,?2,?3)",
                        rusqlite::params![root, serde_json::to_string(value).map_err(|_| rusqlite::Error::InvalidQuery)?, now],
                    )?;
                    if let Some(fields) = value.as_object() {
                        for field in fields.keys() {
                            connection.execute(
                                "INSERT OR IGNORE INTO config_field_versions(key,version) VALUES(?1,0)",
                                [format!("{root}.{field}")],
                            )?;
                        }
                    }
                }
                connection.execute(
                    "INSERT OR IGNORE INTO app_meta(key,value_json) VALUES('format_id',?1)",
                    [serde_json::to_string(data_format::FORMAT_ID).expect("constant string serializes")],
                )?;
                let format_id: String = connection.query_row(
                    "SELECT value_json FROM app_meta WHERE key='format_id'",
                    [],
                    |row| row.get(0),
                )?;
                if format_id != serde_json::to_string(data_format::FORMAT_ID).expect("constant string serializes") {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(())
            })
            .await
            .map_err(|error| format!("初始化应用数据库失败: {error}"))?;
        Ok(Arc::new(Self { connection }))
    }

    pub async fn invoke(&self, channel: &str, args: &[Value]) -> Result<Value, String> {
        commands::invoke(self, channel, args).await
    }

    pub async fn upsert_clipboard_with_image(
        &self,
        item: &Value,
        image: Option<Vec<u8>>,
    ) -> Result<UpsertClipboard, String> {
        clipboard::upsert_clipboard_with_image(self, item, image).await
    }

    pub async fn get_clipboard(&self, content_id: &str) -> Result<Option<Value>, String> {
        clipboard::get_clipboard(self, content_id, true).await
    }

    pub async fn get_clipboard_image(&self, content_id: &str) -> Result<Option<Vec<u8>>, String> {
        clipboard::get_clipboard_image(self, content_id).await
    }

    pub async fn search_history_metadata(
        &self,
        search_text: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<HistoryMetadata>, String> {
        clipboard::search_history_metadata(self, search_text, limit, offset).await
    }

    pub async fn get_history_metadata(
        &self,
        content_id: &str,
    ) -> Result<Option<HistoryMetadata>, String> {
        clipboard::get_history_metadata(self, content_id).await
    }

    pub async fn touch_clipboard(&self, content_id: &str) -> Result<bool, String> {
        clipboard::touch_clipboard(self, content_id).await
    }

    pub async fn history_count(&self) -> Result<u64, String> {
        clipboard::history_count(self).await
    }

    pub async fn cleanup_history(
        &self,
        max_history_size: i64,
        max_keep_time_days: i64,
    ) -> Result<Vec<String>, String> {
        clipboard::cleanup_history(self, max_history_size, max_keep_time_days).await
    }

    pub async fn update_ocr(&self, content_id: &str, text: &str) -> Result<bool, String> {
        clipboard::update_ocr(self, content_id, text).await
    }

    pub async fn upsert_app(&self, app: &Value) -> Result<String, String> {
        apps::upsert_app(self, app).await
    }

    pub(crate) async fn source_apps(&self) -> Result<Vec<SourceApp>, String> {
        self.call(|connection| {
            let mut statement = connection.prepare("SELECT id,application,path,icon FROM apps")?;
            statement
                .query_map([], |row| {
                    Ok(SourceApp {
                        id: row.get(0)?,
                        application: row.get(1)?,
                        path: row.get(2)?,
                        icon: row.get(3)?,
                    })
                })?
                .collect()
        })
        .await
    }

    pub(crate) async fn replace_app_icon(
        &self,
        id: String,
        previous: Option<String>,
        icon: String,
    ) -> Result<bool, String> {
        self.call(move |connection| {
            // A live capture may have refreshed this row while the theme lookup was running.
            Ok(connection.execute(
                "UPDATE apps SET icon=?1,updated_at=?2 WHERE id=?3 AND icon IS ?4 AND icon IS NOT ?1",
                rusqlite::params![icon, now_ms(), id, previous],
            )? > 0)
        })
        .await
    }

    pub async fn record_copy(&self, event: &Value) -> Result<bool, String> {
        statistics::record_copy(self, event).await
    }

    pub async fn record_ai_usage(&self, entry: &Value) -> Result<bool, String> {
        usage::record_ai_usage(self, entry).await
    }

    pub async fn flush(&self) -> Result<(), String> {
        self.call(|connection| connection.execute_batch("PRAGMA wal_checkpoint(FULL)"))
            .await
    }

    pub(crate) async fn call<T, F>(&self, operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.connection
            .call(operation)
            .await
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
