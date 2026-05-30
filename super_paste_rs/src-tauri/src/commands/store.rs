use serde::Deserialize;
use tauri::State;

use super::{IpcResponse, SimpleResponse};
use crate::app::state::AppState;
use crate::infrastructure::database::repositories::ConfigRepository;

#[derive(Deserialize)]
pub struct StoreGetParams {
    pub key: String,
    #[serde(rename = "defaultValue")]
    pub default_value: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct StoreSetParams {
    pub key: String,
    pub value: serde_json::Value,
}

/// 获取存储值
#[tauri::command]
pub fn store_get(
    state: State<AppState>,
    key: String,
    default_value: Option<serde_json::Value>,
) -> IpcResponse<serde_json::Value> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    match ConfigRepository::get(&conn, &key) {
        Ok(Some(value)) => {
            // 尝试解析为 JSON
            let json_value = serde_json::from_str::<serde_json::Value>(&value)
                .unwrap_or(serde_json::Value::String(value));
            IpcResponse::ok(json_value)
        }
        Ok(None) => {
            let val = default_value.unwrap_or(serde_json::Value::Null);
            IpcResponse::ok(val)
        }
        Err(e) => IpcResponse::err(format!("获取配置失败: {}", e)),
    }
}

/// 设置存储值
#[tauri::command]
pub fn store_set(
    state: State<AppState>,
    key: String,
    value: serde_json::Value,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    let value_str = match &value {
        serde_json::Value::String(s) => s.clone(),
        _ => value.to_string(),
    };

    let value_type = match &value {
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        _ => "json",
    };

    match ConfigRepository::set(&conn, &key, &value_str, value_type) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("设置配置失败: {}", e)),
    }
}

/// 获取单个配置值（简化的 store 读取）
#[tauri::command]
pub fn store_get_value(
    state: State<AppState>,
    key: String,
) -> IpcResponse<serde_json::Value> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    match ConfigRepository::get(&conn, &key) {
        Ok(Some(value)) => {
            let json_value = serde_json::from_str::<serde_json::Value>(&value)
                .unwrap_or(serde_json::Value::String(value));
            IpcResponse::ok(json_value)
        }
        Ok(None) => IpcResponse::ok(serde_json::Value::Null),
        Err(e) => IpcResponse::err(format!("获取配置失败: {}", e)),
    }
}
