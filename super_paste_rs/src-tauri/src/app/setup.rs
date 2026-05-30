use rusqlite::Connection;
use tauri::{AppHandle, Manager};

use crate::app::state::AppState;
use crate::error::AppError;
use crate::infrastructure::config::manager::ConfigManager;
use crate::infrastructure::database::migrations::MigrationManager;
use crate::infrastructure::database::pool::create_pool;
use crate::result::Result;

/// 初始化应用核心：数据目录、日志、数据库连接池、迁移、默认配置与应用状态。
pub fn initialize(app: &AppHandle) -> Result<AppState> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::config(format!("无法获取数据目录: {}", e)))?;
    std::fs::create_dir_all(&data_dir)?;

    // 日志（失败不阻断启动）
    let log_dir = data_dir.join("logs");
    if let Err(e) = crate::infrastructure::logging::init_logging(log_dir) {
        eprintln!("日志系统初始化失败: {}", e);
    }

    tracing::info!("数据目录: {}", data_dir.display());

    let db_path = data_dir.join(crate::constants::DB_FILE_NAME);
    let pool = create_pool(db_path.clone())?;

    {
        let conn = pool.get()?;
        MigrationManager::run_migrations(&conn)?;
        seed_default_config(&conn)?;
    }

    let config = {
        let conn = pool.get()?;
        ConfigManager::load(&conn)?
    };

    Ok(AppState::new(pool, config, db_path))
}

/// 将默认配置写入数据库（仅当对应键不存在时）。
fn seed_default_config(conn: &Connection) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let defaults: &[(&str, &str, &str)] = &[
        ("max_history_size", "1000", "number"),
        ("max_keep_days", "-1", "number"),
        ("enable_local_ocr", "false", "boolean"),
        ("ocr_mode", "local", "string"),
        ("ocr_remote_url", "", "string"),
        ("clipboard_listening", "true", "boolean"),
        ("overlay_position", "bottom", "string"),
        ("global_shortcut", "CmdOrCtrl+Shift+V", "string"),
        ("auto_start", "false", "boolean"),
        ("theme", "auto", "string"),
        ("language", "zh-CN", "string"),
        ("sync_enabled", "false", "boolean"),
        ("sync_server_url", "", "string"),
        ("sync_username", "", "string"),
        ("sync_password", "", "string"),
        ("sync_remote_path", "/superpaste/clipboard.db", "string"),
        ("sync_strategy", "merge_by_time", "string"),
    ];

    for (key, value, value_type) in defaults {
        conn.execute(
            "INSERT OR IGNORE INTO app_config (key, value, value_type, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![key, value, value_type, now],
        )?;
    }

    Ok(())
}
