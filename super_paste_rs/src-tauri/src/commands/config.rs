use tauri::State;

use crate::app::state::AppState;
use crate::infrastructure::config::manager::AppConfig;
use crate::result::Result;

/// 获取完整配置。
#[tauri::command]
pub fn get_config(state: State<AppState>) -> Result<AppConfig> {
    Ok(state.config.get())
}

/// 更新单个配置项。
#[tauri::command]
pub fn set_config_value(state: State<AppState>, key: String, value: String) -> Result<()> {
    let conn = state.db_pool.get()?;
    state.config.set(&conn, &key, &value)
}

/// 获取单个配置项。
#[tauri::command]
pub fn get_config_value(state: State<AppState>, key: String) -> Result<Option<String>> {
    Ok(state.config.get_value(&key))
}
