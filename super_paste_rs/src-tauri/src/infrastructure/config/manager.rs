use parking_lot::RwLock;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::result::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub max_history_size: usize,
    pub max_keep_days: i64,
    pub enable_local_ocr: bool,
    pub ocr_mode: String,
    pub ocr_remote_url: String,
    pub ocr_remote_extra_params: String,
    pub ocr_remote_image_field_name: String,
    pub ocr_remote_request_format: String,
    pub ocr_remote_result_path: String,
    pub clipboard_listening: bool,
    pub overlay_position: String,
    pub global_shortcut: String,
    pub auto_start: bool,
    pub theme: String,
    pub language: String,
    pub sync_enabled: bool,
    pub sync_server_url: String,
    pub sync_username: String,
    pub sync_password: String,
    pub sync_remote_path: String,
    pub sync_strategy: String,
    pub last_sync_at: Option<i64>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            max_history_size: 1000,
            max_keep_days: -1,
            enable_local_ocr: false,
            ocr_mode: "local".to_string(),
            ocr_remote_url: String::new(),
            ocr_remote_extra_params: "{}".to_string(),
            ocr_remote_image_field_name: "image".to_string(),
            ocr_remote_request_format: "json".to_string(),
            ocr_remote_result_path: "data.result".to_string(),
            clipboard_listening: true,
            overlay_position: "bottom".to_string(),
            global_shortcut: "CmdOrCtrl+Shift+V".to_string(),
            auto_start: false,
            theme: "auto".to_string(),
            language: "zh-CN".to_string(),
            sync_enabled: false,
            sync_server_url: String::new(),
            sync_username: String::new(),
            sync_password: String::new(),
            sync_remote_path: "/superpaste/clipboard.db".to_string(),
            sync_strategy: "merge_by_time".to_string(),
            last_sync_at: None,
        }
    }
}

pub struct ConfigManager {
    cache: RwLock<HashMap<String, String>>,
}

impl ConfigManager {
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut stmt = conn.prepare("SELECT key, value FROM app_config")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut map = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }

        Ok(Self {
            cache: RwLock::new(map),
        })
    }

    pub fn get(&self) -> AppConfig {
        let cache = self.cache.read();
        AppConfig {
            max_history_size: cache
                .get("max_history_size")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
            max_keep_days: cache
                .get("max_keep_days")
                .and_then(|v| v.parse().ok())
                .unwrap_or(-1),
            enable_local_ocr: cache
                .get("enable_local_ocr")
                .map(|v| v == "true")
                .unwrap_or(false),
            ocr_mode: cache
                .get("ocr_mode")
                .cloned()
                .unwrap_or_else(|| "local".to_string()),
            ocr_remote_url: cache.get("ocr_remote_url").cloned().unwrap_or_default(),
            ocr_remote_extra_params: cache
                .get("ocr_remote_extra_params")
                .cloned()
                .unwrap_or_else(|| "{}".to_string()),
            ocr_remote_image_field_name: cache
                .get("ocr_remote_image_field_name")
                .cloned()
                .unwrap_or_else(|| "image".to_string()),
            ocr_remote_request_format: cache
                .get("ocr_remote_request_format")
                .cloned()
                .unwrap_or_else(|| "json".to_string()),
            ocr_remote_result_path: cache
                .get("ocr_remote_result_path")
                .cloned()
                .unwrap_or_else(|| "data.result".to_string()),
            clipboard_listening: cache
                .get("clipboard_listening")
                .map(|v| v == "true")
                .unwrap_or(true),
            overlay_position: cache
                .get("overlay_position")
                .cloned()
                .unwrap_or_else(|| "bottom".to_string()),
            global_shortcut: cache
                .get("global_shortcut")
                .cloned()
                .unwrap_or_else(|| "CmdOrCtrl+Shift+V".to_string()),
            auto_start: cache
                .get("auto_start")
                .map(|v| v == "true")
                .unwrap_or(false),
            theme: cache
                .get("theme")
                .cloned()
                .unwrap_or_else(|| "auto".to_string()),
            language: cache
                .get("language")
                .cloned()
                .unwrap_or_else(|| "zh-CN".to_string()),
            sync_enabled: cache
                .get("sync_enabled")
                .map(|v| v == "true")
                .unwrap_or(false),
            sync_server_url: cache.get("sync_server_url").cloned().unwrap_or_default(),
            sync_username: cache.get("sync_username").cloned().unwrap_or_default(),
            sync_password: cache.get("sync_password").cloned().unwrap_or_default(),
            sync_remote_path: cache
                .get("sync_remote_path")
                .cloned()
                .unwrap_or_else(|| "/superpaste/clipboard.db".to_string()),
            sync_strategy: cache
                .get("sync_strategy")
                .cloned()
                .unwrap_or_else(|| "merge_by_time".to_string()),
            last_sync_at: cache.get("last_sync_at").and_then(|v| v.parse().ok()),
        }
    }

    pub fn get_value(&self, key: &str) -> Option<String> {
        self.cache.read().get(key).cloned()
    }

    pub fn set(&self, conn: &Connection, key: &str, value: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO app_config (key, value, value_type, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value = ?2, value_type = ?3, updated_at = ?4",
            rusqlite::params![key, value, Self::infer_type(value), now],
        )?;
        self.cache.write().insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn infer_type(value: &str) -> &'static str {
        if value == "true" || value == "false" {
            "boolean"
        } else if value.parse::<f64>().is_ok() {
            "number"
        } else if value.starts_with('{') || value.starts_with('[') {
            "json"
        } else {
            "string"
        }
    }
}
