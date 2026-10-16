use tauri::menu::{Menu, MenuEvent, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::app::state::AppState;
use crate::result::Result;
use crate::system::{main_window, overlay};

/// 读取当前 Overlay 位置（取不到回退 bottom）。
fn overlay_position(app: &AppHandle) -> String {
    app.try_state::<AppState>()
        .map(|s| s.config.get().overlay_position)
        .unwrap_or_else(|| "bottom".to_string())
}

pub fn create_tray(app: &AppHandle) -> Result<()> {
    // 菜单项
    let settings_i = MenuItemBuilder::with_id("settings", "设置").build(app)?;
    let overlay_i = MenuItemBuilder::with_id("overlay", "粘贴面板").build(app)?;
    let separator1 = PredefinedMenuItem::separator(app)?;
    let auto_start_i = MenuItemBuilder::with_id("auto_start", "开机自启").build(app)?;
    let ocr_i = MenuItemBuilder::with_id("ocr", "OCR 识别").build(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    let quit_i = MenuItemBuilder::with_id("quit", "退出").build(app)?;

    let menu = Menu::with_items(
        app,
        &[&settings_i, &overlay_i, &separator1, &auto_start_i, &ocr_i, &separator2, &quit_i],
    )?;

    let _tray = TrayIconBuilder::new()
        .tooltip("Super Paste - 剪贴板管理工具")
        .menu(&menu)
        .on_menu_event(|app, event| {
            handle_menu_event(app, event);
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                // 左键单击切换 Overlay 粘贴面板（与原项目一致）
                let position = overlay_position(app);
                let _ = overlay::toggle_overlay(app, &position);
            }
        })
        .build(app)?;

    tracing::info!("系统托盘已创建");
    Ok(())
}

fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id.as_ref() {
        "settings" => {
            let _ = main_window::show_main_window(app);
        }
        "overlay" => {
            // 切换 overlay 显示/隐藏
            let position = overlay_position(app);
            let _ = overlay::toggle_overlay(app, &position);
        }
        "auto_start" => {
            tracing::info!("切换开机自启");
        }
        "ocr" => {
            tracing::info!("切换 OCR");
        }
        "quit" => {
            app.exit(0);
        }
        _ => {}
    }
}
