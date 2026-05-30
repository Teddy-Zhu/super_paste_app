use tauri::State;

use super::{IpcResponse, SimpleResponse};
use crate::app::state::AppState;
use crate::infrastructure::database::repositories::ClipboardRepository;
use crate::types::clipboard::{ClipboardContent, ClipboardItem, QueryOptions};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ClipboardHistoryResult {
    pub items: Vec<ClipboardItem>,
    pub apps: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ClipboardHistoryQueryOptions {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    #[serde(rename = "primaryType")]
    pub primary_type: Option<String>,
    #[serde(rename = "searchText")]
    pub search_text: Option<String>,
    #[serde(rename = "orderBy")]
    pub order_by: Option<String>,
    #[serde(rename = "orderDirection")]
    pub order_direction: Option<String>,
    #[serde(rename = "tagIds")]
    pub tag_ids: Option<Vec<String>>,
}

/// 获取剪贴板历史（兼容原项目接口格式）
#[tauri::command]
pub fn get_clipboard_history(
    state: State<AppState>,
    options: Option<ClipboardHistoryQueryOptions>,
) -> IpcResponse<ClipboardHistoryResult> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    let opts = options.unwrap_or(ClipboardHistoryQueryOptions {
        limit: Some(200),
        offset: None,
        primary_type: None,
        search_text: None,
        order_by: Some("last_used".to_string()),
        order_direction: Some("DESC".to_string()),
        tag_ids: None,
    });

    let query_opts = QueryOptions {
        offset: opts.offset.unwrap_or(0),
        limit: opts.limit.unwrap_or(200),
        primary_type: opts.primary_type.clone(),
        tag_id: opts.tag_ids.as_ref().and_then(|ids| ids.first().cloned()),
        is_pinned: None,
        is_favorite: None,
        start_date: None,
        end_date: None,
    };

    // 如果有搜索文本，使用搜索接口
    let items = if let Some(ref search_text) = opts.search_text {
        if !search_text.is_empty() {
            match ClipboardRepository::search(&conn, search_text, opts.limit.unwrap_or(200)) {
                Ok(items) => items,
                Err(e) => return IpcResponse::err(format!("搜索失败: {}", e)),
            }
        } else {
            match ClipboardRepository::list(&conn, &query_opts) {
                Ok(items) => items,
                Err(e) => return IpcResponse::err(format!("查询失败: {}", e)),
            }
        }
    } else {
        match ClipboardRepository::list(&conn, &query_opts) {
            Ok(items) => items,
            Err(e) => return IpcResponse::err(format!("查询失败: {}", e)),
        }
    };

    let result = ClipboardHistoryResult {
        items,
        apps: std::collections::HashMap::new(),
    };

    IpcResponse::ok(result)
}

/// 还原剪贴板条目（复制到系统剪贴板）
#[tauri::command]
pub fn restore_clipboard_item(
    state: State<AppState>,
    item_id: String,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    let id: i64 = match item_id.parse() {
        Ok(id) => id,
        Err(_) => return SimpleResponse::err("无效的条目 ID"),
    };

    let (item, _content) = match ClipboardRepository::get_by_id(&conn, id) {
        Ok(Some(data)) => data,
        Ok(None) => return SimpleResponse::err("条目不存在"),
        Err(e) => return SimpleResponse::err(format!("查询失败: {}", e)),
    };

    // TODO: 将内容写入系统剪贴板（需要 clipboard-rs）
    tracing::info!("还原剪贴板条目: {}", item.content_id);
    SimpleResponse::ok()
}

/// 还原为纯文本
#[tauri::command]
pub fn restore_clipboard_as_text(
    state: State<AppState>,
    item_id: String,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    let id: i64 = match item_id.parse() {
        Ok(id) => id,
        Err(_) => return SimpleResponse::err("无效的条目 ID"),
    };

    let (_item, content) = match ClipboardRepository::get_by_id(&conn, id) {
        Ok(Some(data)) => data,
        Ok(None) => return SimpleResponse::err("条目不存在"),
        Err(e) => return SimpleResponse::err(format!("查询失败: {}", e)),
    };

    if let Some(_text) = content.text_content {
        // TODO: 将纯文本写入系统剪贴板
        SimpleResponse::ok()
    } else {
        SimpleResponse::err("该条目没有文本内容")
    }
}

/// 获取剪贴板状态
#[tauri::command]
pub fn get_clipboard_status(state: State<AppState>) -> IpcResponse<serde_json::Value> {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return IpcResponse::err(format!("数据库连接失败: {}", e)),
    };

    let count = ClipboardRepository::count(&conn).unwrap_or(0);
    let config = state.config.get();

    let status = serde_json::json!({
        "isListening": config.clipboard_listening,
        "config": {
            "maxHistorySize": config.max_history_size,
            "enableAutoSave": true,
            "enableLocalOCR": config.enable_local_ocr,
            "ocrMode": config.ocr_mode,
            "ocrRemoteConfig": {
                "url": config.ocr_remote_url,
                "extraParams": "{}",
                "imageFieldName": "image",
                "requestFormat": "json",
                "resultPath": "data.result"
            }
        },
        "historyCount": count
    });

    IpcResponse::ok(status)
}

/// 删除剪贴板条目（兼容原项目接口）
#[tauri::command]
pub fn delete_clipboard_item_compat(
    state: State<AppState>,
    item_id: String,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    let id: i64 = match item_id.parse() {
        Ok(id) => id,
        Err(_) => return SimpleResponse::err("无效的条目 ID"),
    };

    match ClipboardRepository::delete(&conn, id) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("删除失败: {}", e)),
    }
}

/// 清空剪贴板历史（兼容原项目接口）
#[tauri::command]
pub fn clear_clipboard_history_compat(state: State<AppState>) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    match ClipboardRepository::clear_all(&conn) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("清空失败: {}", e)),
    }
}

/// 开始监听剪贴板
#[tauri::command]
pub fn start_clipboard_listening() -> SimpleResponse {
    // TODO: 启动剪贴板监听
    SimpleResponse::ok()
}

/// 停止监听剪贴板
#[tauri::command]
pub fn stop_clipboard_listening() -> SimpleResponse {
    // TODO: 停止剪贴板监听
    SimpleResponse::ok()
}

/// 更新剪贴板配置
#[tauri::command]
pub fn update_clipboard_config(
    state: State<AppState>,
    config: serde_json::Value,
) -> SimpleResponse {
    let conn = match state.db_pool.get() {
        Ok(conn) => conn,
        Err(e) => return SimpleResponse::err(format!("数据库连接失败: {}", e)),
    };

    // 更新配置项
    if let Some(max_size) = config.get("maxHistorySize").and_then(|v| v.as_i64()) {
        let _ = state.config.set(&conn, "max_history_size", &max_size.to_string());
    }
    if let Some(enable_ocr) = config.get("enableLocalOCR").and_then(|v| v.as_bool()) {
        let _ = state.config.set(&conn, "enable_local_ocr", &enable_ocr.to_string());
    }
    if let Some(ocr_mode) = config.get("ocrMode").and_then(|v| v.as_str()) {
        let _ = state.config.set(&conn, "ocr_mode", ocr_mode);
    }

    SimpleResponse::ok()
}
