use tauri::{Manager, State};

use super::SimpleResponse;
use crate::app::state::AppState;
use crate::system::overlay;

/// 显示 Overlay 窗口（根据配置的位置）。
#[tauri::command]
pub fn show_overlay_window(app: tauri::AppHandle, state: State<AppState>) -> SimpleResponse {
    let position = state.config.get().overlay_position;
    match overlay::show_overlay(&app, &position) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("显示 Overlay 失败: {}", e)),
    }
}

/// 隐藏 Overlay 窗口。
#[tauri::command]
pub fn hide_overlay_window(app: tauri::AppHandle) -> SimpleResponse {
    match overlay::hide_overlay(&app) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("隐藏 Overlay 失败: {}", e)),
    }
}

/// 切换 Overlay 窗口显示/隐藏。
#[tauri::command]
pub fn toggle_overlay_window(app: tauri::AppHandle, state: State<AppState>) -> SimpleResponse {
    if let Some(overlay_win) = app.get_webview_window("overlay") {
        if overlay_win.is_visible().unwrap_or(false) {
            match overlay::hide_overlay(&app) {
                Ok(_) => SimpleResponse::ok(),
                Err(e) => SimpleResponse::err(format!("隐藏 Overlay 失败: {}", e)),
            }
        } else {
            let position = state.config.get().overlay_position;
            match overlay::show_overlay(&app, &position) {
                Ok(_) => SimpleResponse::ok(),
                Err(e) => SimpleResponse::err(format!("显示 Overlay 失败: {}", e)),
            }
        }
    } else {
        SimpleResponse::err("Overlay 窗口不存在")
    }
}

/// 最小化窗口
#[tauri::command]
pub fn minimize_window(app: tauri::AppHandle) -> SimpleResponse {
    if let Some(window) = app.get_webview_window("main") {
        match window.minimize() {
            Ok(_) => SimpleResponse::ok(),
            Err(e) => SimpleResponse::err(format!("最小化失败: {}", e)),
        }
    } else {
        SimpleResponse::err("主窗口不存在")
    }
}

/// 关闭窗口（隐藏到托盘）
#[tauri::command]
pub fn close_window(app: tauri::AppHandle) -> SimpleResponse {
    if let Some(window) = app.get_webview_window("main") {
        match window.hide() {
            Ok(_) => SimpleResponse::ok(),
            Err(e) => SimpleResponse::err(format!("关闭失败: {}", e)),
        }
    } else {
        SimpleResponse::err("主窗口不存在")
    }
}

/// 重启应用
#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) {
    app.restart();
}
