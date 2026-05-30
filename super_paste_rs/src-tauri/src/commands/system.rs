use tauri::{Manager, State};

use crate::app::state::AppState;
use crate::result::Result;
use crate::system::overlay;

/// 显示 Overlay 窗口（根据配置的位置）。
#[tauri::command]
pub fn show_overlay_window(app: tauri::AppHandle, state: State<AppState>) -> Result<()> {
    let position = state.config.get().overlay_position;
    overlay::show_overlay(&app, &position)
}

/// 隐藏 Overlay 窗口。
#[tauri::command]
pub fn hide_overlay_window(app: tauri::AppHandle) -> Result<()> {
    overlay::hide_overlay(&app)
}

/// 切换 Overlay 窗口显示/隐藏。
#[tauri::command]
pub fn toggle_overlay_window(app: tauri::AppHandle, state: State<AppState>) -> Result<()> {
    if let Some(overlay) = app.get_webview_window("overlay") {
        if overlay.is_visible().unwrap_or(false) {
            overlay::hide_overlay(&app)?;
        } else {
            let position = state.config.get().overlay_position;
            overlay::show_overlay(&app, &position)?;
        }
    }
    Ok(())
}
