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
use tauri::{Emitter, Listener, Manager};
use tauri_plugin_opener::OpenerExt;

use app::state::AppState;
use services::clipboard::manager::ClipboardManager;
use services::clipboard::watcher::ClipboardWatcherService;
use system::{main_window, overlay};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二实例启动时显示主窗口（统一走 show_main_window，尺寸/居中一致）
            let _ = main_window::show_main_window(app);
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
                state.clipboard_listening.clone(),
            ));
            let _watcher = ClipboardWatcherService::start(clipboard_manager);

            // 注册应用状态
            app.manage(state);

            // 注册前端 -> 后端事件监听器（必须在 app.manage 之后，闭包内才能取 state）
            register_frontend_listeners(&handle);

            // Overlay 失焦自动关闭（仅生产环境，避免开发态 devtools 抢焦点误触）
            #[cfg(not(debug_assertions))]
            {
                if let Some(overlay_win) = app.get_webview_window("overlay") {
                    let h = handle.clone();
                    overlay_win.on_window_event(move |event| {
                        if let tauri::WindowEvent::Focused(false) = event {
                            if let Some(ov) = h.get_webview_window("overlay") {
                                if ov.is_visible().unwrap_or(false) {
                                    // 通知前端：播放滑出动画，动画结束再 emit close-overlay-window
                                    let _ = ov.emit("trigger-close-window", true);
                                }
                            }
                        }
                    });
                }
            }

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
            commands::system::show_main_window_cmd,
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

/// 集中注册前端 emit -> 后端 listen 的事件监听器。
fn register_frontend_listeners(handle: &tauri::AppHandle) {
    // 读取当前 Overlay 位置（闭包里用 try_state 不 panic）
    fn overlay_position(app: &tauri::AppHandle) -> String {
        app.try_state::<AppState>()
            .map(|s| s.config.get().overlay_position)
            .unwrap_or_else(|| "bottom".to_string())
    }

    // 1) close-overlay-window：滑出动画完成后真正隐藏 overlay（核心修复）
    {
        let app = handle.clone();
        handle.listen("close-overlay-window", move |_event| {
            let pos = overlay_position(&app);
            if let Err(e) = overlay::hide_overlay(&app, &pos) {
                tracing::warn!("close-overlay-window: 隐藏失败 {:?}", e);
            }
        });
    }

    // 2) open-main-window：显示主窗口
    {
        let app = handle.clone();
        handle.listen("open-main-window", move |_event| {
            if let Err(e) = main_window::show_main_window(&app) {
                tracing::warn!("open-main-window: 显示主窗口失败 {:?}", e);
            }
        });
    }

    // 3) open-overlay-window：显示 overlay + 滑入动画
    {
        let app = handle.clone();
        handle.listen("open-overlay-window", move |_event| {
            let pos = overlay_position(&app);
            match overlay::show_overlay(&app, &pos) {
                Ok(_) => {
                    if let Some(ov) = app.get_webview_window("overlay") {
                        let _ = ov.emit("trigger-slide-in-animation", &pos);
                    }
                }
                Err(e) => tracing::warn!("open-overlay-window: 显示失败 {:?}", e),
            }
        });
    }

    // 4) open-external-url：用 opener 插件打开外链
    {
        let app = handle.clone();
        handle.listen("open-external-url", move |event| {
            // payload 是 JSON 文本，字符串会带引号，需反序列化
            let raw = event.payload();
            let url = serde_json::from_str::<String>(raw)
                .unwrap_or_else(|_| raw.trim_matches('"').to_string());
            if url.is_empty() {
                tracing::warn!("open-external-url: 空 URL");
                return;
            }
            if let Err(e) = app.opener().open_url(url, None::<&str>) {
                tracing::warn!("open-external-url: 打开失败 {:?}", e);
            }
        });
    }

    // 5) open-overlay-finished：滑入完成信号，仅记日志
    handle.listen("open-overlay-finished", move |_event| {
        tracing::debug!("overlay 滑入动画完成");
    });
}
