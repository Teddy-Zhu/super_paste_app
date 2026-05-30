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
            // 剪贴板命令（原项目兼容格式）
            commands::clipboard::get_clipboard_history,
            commands::clipboard::restore_clipboard_item,
            commands::clipboard::restore_clipboard_as_text,
            commands::clipboard::get_clipboard_status,
            commands::clipboard::delete_clipboard_item_compat,
            commands::clipboard::clear_clipboard_history_compat,
            commands::clipboard::start_clipboard_listening,
            commands::clipboard::stop_clipboard_listening,
            commands::clipboard::update_clipboard_config,
            // 剪贴板命令（原有 - 已合并到兼容命令中）
            // 存储命令
            commands::store::store_get,
            commands::store::store_set,
            commands::store::store_get_value,
            // 标签命令
            commands::tags::get_all_tags,
            commands::tags::create_tag,
            commands::tags::update_tag,
            commands::tags::delete_tag,
            commands::tags::get_tag_stats,
            commands::tags::bind_tag_to_record,
            commands::tags::unbind_tag_from_record,
            // 应用信息命令
            commands::apps::get_all_apps,
            commands::apps::get_app_cache_stats,
            // 配置命令
            commands::config::get_config,
            commands::config::set_config_value,
            commands::config::get_config_value,
            // 系统命令
            commands::system::show_overlay_window,
            commands::system::hide_overlay_window,
            commands::system::toggle_overlay_window,
            commands::system::minimize_window,
            commands::system::close_window,
            commands::system::restart_app,
            // 状态同步命令
            commands::sync::sync_state_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
