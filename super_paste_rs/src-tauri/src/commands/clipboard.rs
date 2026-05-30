use tauri::State;

use crate::app::state::AppState;
use crate::infrastructure::database::repositories::ClipboardRepository;
use crate::result::Result;
use crate::types::clipboard::{ClipboardContent, ClipboardItem, QueryOptions};

/// 获取剪贴板历史列表（分页）。
#[tauri::command]
pub fn get_clipboard_history(
    state: State<AppState>,
    options: Option<QueryOptions>,
) -> Result<Vec<ClipboardItem>> {
    let conn = state.db_pool.get()?;
    let opts = options.unwrap_or_default();
    ClipboardRepository::list(&conn, &opts)
}

/// 根据 ID 获取剪贴板条目详情（含完整内容）。
#[tauri::command]
pub fn get_clipboard_item(
    state: State<AppState>,
    id: i64,
) -> Result<Option<(ClipboardItem, ClipboardContent)>> {
    let conn = state.db_pool.get()?;
    ClipboardRepository::get_by_id(&conn, id)
}

/// 搜索剪贴板历史（全文搜索）。
#[tauri::command]
pub fn search_clipboard(state: State<AppState>, query: String, limit: i64) -> Result<Vec<ClipboardItem>> {
    let conn = state.db_pool.get()?;
    ClipboardRepository::search(&conn, &query, limit)
}

/// 删除剪贴板条目。
#[tauri::command]
pub fn delete_clipboard_item(state: State<AppState>, id: i64) -> Result<()> {
    let conn = state.db_pool.get()?;
    ClipboardRepository::delete(&conn, id)
}

/// 清空所有非置顶/非收藏的剪贴板条目。
#[tauri::command]
pub fn clear_clipboard_history(state: State<AppState>) -> Result<usize> {
    let conn = state.db_pool.get()?;
    ClipboardRepository::clear_all(&conn)
}

/// 更新剪贴板条目的置顶/收藏状态。
#[tauri::command]
pub fn update_clipboard_flags(
    state: State<AppState>,
    id: i64,
    is_pinned: Option<bool>,
    is_favorite: Option<bool>,
) -> Result<()> {
    let conn = state.db_pool.get()?;
    ClipboardRepository::update_flags(&conn, id, is_pinned, is_favorite)
}

/// 获取剪贴板条目总数。
#[tauri::command]
pub fn get_clipboard_count(state: State<AppState>) -> Result<i64> {
    let conn = state.db_pool.get()?;
    ClipboardRepository::count(&conn)
}
