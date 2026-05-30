pub mod app;
pub mod commands;
pub mod constants;
pub mod error;
pub mod infrastructure;
pub mod result;
pub mod services;
pub mod system;
pub mod types;
pub mod utils;

use std::sync::Arc;
use tauri::Manager;

use services::clipboard::manager::ClipboardManager;
use services::clipboard::watcher::ClipboardWatcherService;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle = app.handle().clone();

            // 初始化应用状态（数据库、配置、日志）
            let state = app::setup::initialize(&handle)?;

            // 创建系统托盘
            system::tray::create_tray(&handle)?;

            // 注册全局快捷键
            let shortcut = state.config.get().global_shortcut;
            match system::shortcuts::ShortcutManager::register_with_fallback(&handle, &shortcut) {
                Ok(registered) => {
                    tracing::info!("全局快捷键已注册: {}", registered);
                }
                Err(e) => {
                    tracing::warn!("全局快捷键注册失败: {:?}", e);
                }
            }

            // 启动剪贴板监听器
            let clipboard_manager = Arc::new(ClipboardManager::new(
                state.db_pool.clone(),
                state.event_bus.clone(),
                state.active_window.clone(),
            ));
            let _watcher = ClipboardWatcherService::start(clipboard_manager);

            // 注册应用状态
            app.manage(state);

            tracing::info!("应用初始化完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 剪贴板命令
            commands::clipboard::get_clipboard_history,
            commands::clipboard::get_clipboard_item,
            commands::clipboard::search_clipboard,
            commands::clipboard::delete_clipboard_item,
            commands::clipboard::clear_clipboard_history,
            commands::clipboard::update_clipboard_flags,
            commands::clipboard::get_clipboard_count,
            // 配置命令
            commands::config::get_config,
            commands::config::set_config_value,
            commands::config::get_config_value,
            // 系统命令
            commands::system::show_overlay_window,
            commands::system::hide_overlay_window,
            commands::system::toggle_overlay_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
