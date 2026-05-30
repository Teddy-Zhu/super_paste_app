//! 应用生命周期辅助函数（窗口显示/隐藏、退出等）。
//! 具体的 Overlay/主窗口切换逻辑在阶段 2（系统集成）中实现于 `system::overlay`。

use tauri::{AppHandle, Manager};

/// 显示并聚焦主窗口。
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 隐藏主窗口。
pub fn hide_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}
