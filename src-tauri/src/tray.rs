use crate::{platform::windows::on_main, runtime::Runtime};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const TRAY_ID: &str = "super-paste-tray";

fn menu(app: &tauri::AppHandle, settings: &Value) -> Result<Menu<tauri::Wry>, String> {
    let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let panel = MenuItem::with_id(app, "panel", "粘贴面板", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let mode = settings["colorMode"].as_str().ok_or("颜色模式无效")?;
    let system = CheckMenuItem::with_id(
        app,
        "theme:system",
        "跟随系统",
        true,
        mode == "system",
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let light = CheckMenuItem::with_id(
        app,
        "theme:light",
        "亮色模式",
        true,
        mode == "light",
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let dark = CheckMenuItem::with_id(
        app,
        "theme:dark",
        "暗黑模式",
        true,
        mode == "dark",
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let theme = Submenu::with_items(app, "颜色模式", true, &[&system, &light, &dark])
        .map_err(|e| e.to_string())?;
    let first_separator = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "开机自启",
        true,
        settings["autoStart"].as_bool().ok_or("自启配置无效")?,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let ocr = CheckMenuItem::with_id(
        app,
        "ocr",
        "自动 OCR",
        true,
        settings["enableLocalOCR"].as_bool().ok_or("OCR 配置无效")?,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    let second_separator = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let restart = MenuItem::with_id(app, "restart", "重启应用", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let quit =
        MenuItem::with_id(app, "quit", "退出", true, None::<&str>).map_err(|e| e.to_string())?;
    Menu::with_items(
        app,
        &[
            &settings_item,
            &panel,
            &theme,
            &first_separator,
            &autostart,
            &ocr,
            &second_separator,
            &restart,
            &quit,
        ],
    )
    .map_err(|e| e.to_string())
}

pub async fn create(runtime: &Arc<Runtime>) -> Result<(), String> {
    let settings = runtime.config.runtime_settings().await?;
    let app = runtime.app.clone();
    let weak = Arc::downgrade(runtime);
    on_main(&runtime.app, move || {
        let icon = app.default_window_icon().ok_or("应用托盘图标缺失")?.clone();
        let clicked = weak.clone();
        TrayIconBuilder::with_id(TRAY_ID)
            .icon(icon)
            .icon_as_template(cfg!(target_os = "macos"))
            .tooltip("Super Paste - 剪贴板管理工具")
            .menu(&menu(&app, &settings)?)
            .show_menu_on_left_click(false)
            .on_menu_event(move |_, event| {
                let Some(runtime) = weak.upgrade() else {
                    return;
                };
                let id = event.id.as_ref().to_owned();
                tracing::debug!(menu_item = %id, "tray menu action");
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = handle(runtime.clone(), &id).await {
                        tracing::error!(%error,"tray action failed");
                        if let Err(error) = refresh(&runtime).await {
                            tracing::error!(%error,"tray state restoration failed");
                        }
                    }
                });
            })
            .on_tray_icon_event(move |_, event| {
                let main_window = matches!(
                    event,
                    TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if (main_window
                    || matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ))
                    && let Some(runtime) = clicked.upgrade()
                {
                    tauri::async_runtime::spawn(async move {
                        let result = if main_window {
                            runtime.windows.open_main().await
                        } else {
                            runtime.windows.toggle(None).await
                        };
                        if let Err(error) = result {
                            tracing::error!(%error,"tray window activation failed");
                        }
                    });
                }
            })
            .build(&app)
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn refresh(runtime: &Arc<Runtime>) -> Result<(), String> {
    let settings = runtime.config.runtime_settings().await?;
    let app = runtime.app.clone();
    on_main(&runtime.app, move || {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            tray.set_menu(Some(menu(&app, &settings)?))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

async fn handle(runtime: Arc<Runtime>, id: &str) -> Result<(), String> {
    if runtime.is_quitting() {
        return Err("应用正在退出".into());
    }
    match id {
        "settings" => runtime.windows.open_main().await,
        "panel" => runtime.windows.toggle(None).await,
        "autostart" | "ocr" => {
            let settings = runtime.config.runtime_settings().await?;
            let key = if id == "autostart" {
                "autoStart"
            } else {
                "enableLocalOCR"
            };
            runtime
                .set_setting(
                    key,
                    json!(!settings[key].as_bool().ok_or("布尔配置无效")?),
                    "tray",
                )
                .await
        }
        "restart" | "quit" => {
            runtime.shutdown(id == "restart").await;
            Ok(())
        }
        value if value.starts_with("theme:") => {
            runtime
                .set_setting("colorMode", json!(&value[6..]), "tray")
                .await
        }
        _ => Err("未知托盘操作".into()),
    }
}
