use tauri::{Emitter, Manager, State};

use super::SimpleResponse;
use crate::app::state::AppState;
use crate::system::{main_window, overlay};

/// 显示主窗口（动态计算大小）
#[tauri::command]
pub fn show_main_window_cmd(app: tauri::AppHandle) -> SimpleResponse {
    match main_window::show_main_window(&app) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("显示主窗口失败: {}", e)),
    }
}

/// 显示 Overlay 窗口。
#[tauri::command]
pub fn show_overlay_window(app: tauri::AppHandle, state: State<AppState>) -> SimpleResponse {
    let position = state.config.get().overlay_position;
    match overlay::show_overlay(&app, &position) {
        Ok(_) => {
            if let Some(overlay_win) = app.get_webview_window("overlay") {
                let _ = overlay_win.emit("trigger-slide-in-animation", &position);
            }
            SimpleResponse::ok()
        }
        Err(e) => SimpleResponse::err(format!("显示 Overlay 失败: {}", e)),
    }
}

/// 隐藏 Overlay 窗口。
#[tauri::command]
pub fn hide_overlay_window(app: tauri::AppHandle, state: State<AppState>) -> SimpleResponse {
    let position = state.config.get().overlay_position;
    match overlay::hide_overlay(&app, &position) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("隐藏 Overlay 失败: {}", e)),
    }
}

/// 切换 Overlay 窗口。
#[tauri::command]
pub fn toggle_overlay_window(app: tauri::AppHandle, state: State<AppState>) -> SimpleResponse {
    let position = state.config.get().overlay_position;
    match overlay::toggle_overlay(&app, &position) {
        Ok(_) => SimpleResponse::ok(),
        Err(e) => SimpleResponse::err(format!("切换 Overlay 失败: {}", e)),
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
