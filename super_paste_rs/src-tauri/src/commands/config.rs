use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::app::state::AppState;
use crate::infrastructure::config::manager::AppConfig;
use crate::result::Result;
use crate::system::shortcuts::ShortcutManager;

/// 获取完整配置。
#[tauri::command]
pub fn get_config(state: State<AppState>) -> Result<AppConfig> {
    Ok(state.config.get())
}

/// 更新单个配置项，并按 key 应用相应副作用（生效）。
#[tauri::command]
pub fn set_config_value(
    app: AppHandle,
    state: State<AppState>,
    key: String,
    value: String,
) -> Result<()> {
    {
        let conn = state.db_pool.get()?;
        state.config.set(&conn, &key, &value)?;
    }

    // 按配置项派发副作用，使更改即时生效
    match key.as_str() {
        // Overlay 位置：通知 overlay 窗口实时更新滑入方向
        "overlay_position" => {
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.emit("overlay-position-updated", &value);
            }
        }
        // 全局快捷键：注销旧的并重新注册
        "global_shortcut" => {
            let _ = app.global_shortcut().unregister_all();
            if let Err(e) = ShortcutManager::register_with_fallback(&app, &value) {
                tracing::warn!("重新注册全局快捷键失败: {:?}", e);
            }
        }
        // 开机自启：调用 autostart 插件
        "auto_start" => {
            let manager = app.autolaunch();
            let result = if value == "true" {
                manager.enable()
            } else {
                manager.disable()
            };
            if let Err(e) = result {
                tracing::warn!("切换开机自启失败: {:?}", e);
            }
        }
        // 剪贴板监听开关：翻转共享标志
        "clipboard_listening" => {
            state
                .clipboard_listening
                .store(value == "true", std::sync::atomic::Ordering::Relaxed);
        }
        _ => {}
    }

    Ok(())
}

/// 获取单个配置项。
#[tauri::command]
pub fn get_config_value(state: State<AppState>, key: String) -> Result<Option<String>> {
    Ok(state.config.get_value(&key))
}
