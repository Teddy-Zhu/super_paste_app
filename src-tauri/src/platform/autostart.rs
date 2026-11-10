use glib::{KeyFile, KeyFileFlags};
use std::fs;
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;

pub fn enable(app: &tauri::AppHandle) -> Result<(), String> {
    let icon_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("icons");
    let icon = icon_dir.join("autostart.png");
    let icon_name = icon.to_str().ok_or("自启图标路径不是有效的 UTF-8")?;

    // AppImage resources live in a temporary mount; the login manager needs a persistent icon.
    fs::create_dir_all(&icon_dir).map_err(|e| format!("无法创建自启图标目录：{e}"))?;
    fs::write(&icon, include_bytes!("../../icons/128x128@2x.png"))
        .map_err(|e| format!("无法保存自启图标：{e}"))?;

    // Match auto-launch 0.6's directory, which does not use XDG_CONFIG_HOME.
    let entry = app
        .path()
        .home_dir()
        .map_err(|e| e.to_string())?
        .join(".config/autostart")
        .join(format!("{}.desktop", crate::AUTOSTART_APP_NAME));

    app.autolaunch().enable().map_err(|e| e.to_string())?;
    let desktop = KeyFile::new();
    desktop
        .load_from_file(
            &entry,
            KeyFileFlags::KEEP_COMMENTS | KeyFileFlags::KEEP_TRANSLATIONS,
        )
        .map_err(|e| format!("无法读取自启条目：{e}"))?;
    desktop.set_string("Desktop Entry", "Icon", icon_name);
    desktop
        .save_to_file(&entry)
        .map_err(|e| format!("无法保存自启条目图标：{e}"))
}
