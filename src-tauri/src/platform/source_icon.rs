use super::active_window::ActiveWindow;
use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, RgbaImage, imageops::FilterType};
use std::io::Cursor;

// The existing UI renders 32 CSS pixels. Keep enough detail for a 3× display.
const ICON_SIZE: u32 = 96;
const PNG_PREFIX: &str = "data:image/png;base64,";

pub(crate) fn load(window: &ActiveWindow, app: &tauri::AppHandle) -> Result<String> {
    #[cfg(target_os = "linux")]
    let window_icon = if !clipboard_core::is_wayland_session() {
        window.native.as_ref().and_then(|native| {
            x11_icon(native.id)
                .inspect_err(|error| tracing::debug!(%error, "X11 window icon unavailable"))
                .ok()
        })
    } else {
        None
    };
    #[cfg(not(target_os = "linux"))]
    let window_icon: Option<String> = None;

    if let Some(icon) = window_icon.as_ref().filter(|icon| !needs_refresh(icon)) {
        return Ok(icon.clone());
    }
    match application_icon(&window.name, window.path.as_deref(), app) {
        Ok(icon) => Ok(match window_icon {
            Some(window_icon) if !should_replace(Some(&window_icon), &icon) => window_icon,
            _ => icon,
        }),
        Err(error) => {
            tracing::debug!(application = %window.name, %error, "system application icon unavailable; using window fallback");
            if let Some(icon) = window_icon {
                return Ok(icon);
            }
            // Do not re-enter x-win's first-image parser after our X11 parser rejected a property.
            #[cfg(target_os = "linux")]
            if !clipboard_core::is_wayland_session() {
                return Err(error);
            }
            window.native.as_ref().ok_or(error).and_then(native_icon)
        }
    }
}

fn encode_icon(pixels: RgbaImage) -> Result<String> {
    if pixels.width() == 0 || pixels.height() == 0 {
        return Err(anyhow!("来源应用图标尺寸无效"));
    }
    let image = DynamicImage::ImageRgba8(pixels);
    // Downsample oversized assets once, but never enlarge a low-resolution bitmap.
    let image = if image.width() > ICON_SIZE || image.height() > ICON_SIZE {
        image.resize(ICON_SIZE, ICON_SIZE, FilterType::Lanczos3)
    } else {
        image
    };
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .context("无法编码来源应用图标")?;
    Ok(format!("{PNG_PREFIX}{}", STANDARD.encode(png.into_inner())))
}

fn native_icon(window: &x_win::WindowInfo) -> Result<String> {
    let icon = x_win::get_window_icon(window)
        .map_err(|error| anyhow!("x-win 无法读取来源应用图标: {error}"))?;
    let data = icon
        .data
        .strip_prefix(PNG_PREFIX)
        .ok_or_else(|| anyhow!("来源应用窗口尚未提供有效的 PNG 图标"))?;
    let png = STANDARD.decode(data).context("来源应用图标编码无效")?;
    let pixels = image::load_from_memory_with_format(&png, ImageFormat::Png)
        .context("无法解码来源应用图标")?
        .into_rgba8();
    encode_icon(pixels)
}

fn resolution(icon: &str) -> u32 {
    let Some(data) = icon.strip_prefix(PNG_PREFIX) else {
        return 0;
    };
    // Only decode the PNG header; large historical icons need no pixel allocation.
    let Some(header) = data.get(..44).and_then(|s| STANDARD.decode(s).ok()) else {
        return 0;
    };
    if header.get(..8) != Some(b"\x89PNG\r\n\x1a\n") || header.get(8..16) != Some(b"\0\0\0\rIHDR") {
        return 0;
    }
    let width = u32::from_be_bytes(header[16..20].try_into().expect("PNG header width"));
    let height = u32::from_be_bytes(header[20..24].try_into().expect("PNG header height"));
    width.min(height)
}

pub(crate) fn needs_refresh(icon: &str) -> bool {
    resolution(icon) < ICON_SIZE
}

pub(crate) fn should_replace(previous: Option<&str>, next: &str) -> bool {
    let next_quality = resolution(next).min(ICON_SIZE);
    next_quality > 0 && next_quality >= previous.map(resolution).unwrap_or_default().min(ICON_SIZE)
}

#[cfg(target_os = "linux")]
fn x11_icon(window_id: u32) -> Result<String> {
    use xcb::{Xid, XidNew, x};

    let (connection, _) = xcb::Connection::connect(None).context("无法连接 X11 图标服务")?;
    let atom = connection
        .wait_for_reply(connection.send_request(&x::InternAtom {
            only_if_exists: true,
            name: b"_NET_WM_ICON",
        }))?
        .atom();
    if atom.is_none() {
        return Err(anyhow!("X11 窗口尚未提供图标属性"));
    }
    let reply = connection.wait_for_reply(connection.send_request(&x::GetProperty {
        delete: false,
        window: x::Window::new(window_id),
        property: atom,
        r#type: x::ATOM_CARDINAL,
        long_offset: 0,
        // Bound allocations from a foreign window property to 16 MiB.
        long_length: 4 * 1024 * 1024,
    }))?;
    if reply.format() != 32 || reply.r#type() != x::ATOM_CARDINAL {
        return Err(anyhow!("X11 窗口图标属性格式无效"));
    }
    encode_icon(select_x11_icon(reply.value::<u32>())?)
}

#[cfg(target_os = "linux")]
fn select_x11_icon(mut data: &[u32]) -> Result<RgbaImage> {
    let mut best: Option<(u32, u32, &[u32])> = None;
    while data.len() >= 2 {
        let (width, height) = (data[0], data[1]);
        data = &data[2..];
        let Some(length) = (width as usize).checked_mul(height as usize) else {
            break;
        };
        if length == 0 || length > data.len() {
            break;
        }
        let pixels = &data[..length];
        data = &data[length..];
        let adequate = width.min(height) >= ICON_SIZE;
        let replace = best.is_none_or(|(old_width, old_height, old_pixels)| {
            let old_adequate = old_width.min(old_height) >= ICON_SIZE;
            match (adequate, old_adequate) {
                (true, false) => true,
                (false, true) => false,
                (true, true) => length < old_pixels.len(),
                (false, false) => width.min(height) > old_width.min(old_height),
            }
        });
        if replace {
            best = Some((width, height, pixels));
        }
    }
    let (width, height, pixels) = best.ok_or_else(|| anyhow!("X11 窗口没有完整有效的图标"))?;
    let rgba = pixels
        .iter()
        .flat_map(|argb| {
            [
                (argb >> 16) as u8,
                (argb >> 8) as u8,
                *argb as u8,
                (argb >> 24) as u8,
            ]
        })
        .collect();
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| anyhow!("X11 图标像素数据无效"))
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn application_icon(
    _application: &str,
    path: Option<&str>,
    _app: &tauri::AppHandle,
) -> Result<String> {
    let path = path
        .filter(|path| !path.is_empty())
        .context("来源应用路径为空")?;
    let icon = file_icon_provider::get_file_icon(path, ICON_SIZE as u16)
        .context("无法读取来源应用的系统图标")?;
    let pixels = RgbaImage::from_raw(icon.width, icon.height, icon.pixels)
        .context("来源应用的系统图标像素数据无效")?;
    encode_icon(pixels)
}

#[cfg(target_os = "linux")]
fn application_icon(
    application: &str,
    path: Option<&str>,
    app: &tauri::AppHandle,
) -> Result<String> {
    use freedesktop_desktop_entry::{desktop_entries, find_app_by_id, unicase::Ascii};
    use std::{path::Path, sync::mpsc, time::Duration};

    let locales = std::env::var("LANG").ok().into_iter().collect::<Vec<_>>();
    let entries = desktop_entries(&locales);
    let app_id = application.trim();
    let path = path.map(|path| path.strip_suffix(" (deleted)").unwrap_or(path));
    let executable = path.and_then(|path| Path::new(path).file_name()?.to_str());
    let entry = find_app_by_id(&entries, Ascii::new(app_id))
        .or_else(|| {
            app_id
                .strip_suffix(".desktop")
                .and_then(|id| find_app_by_id(&entries, Ascii::new(id)))
        })
        .or_else(|| path.and_then(|path| find_app_by_id(&entries, Ascii::new(path))))
        .or_else(|| executable.and_then(|name| find_app_by_id(&entries, Ascii::new(name))))
        .ok_or_else(|| anyhow!("未找到来源应用对应的桌面条目"))?;
    let icon_name = entry
        .icon()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| anyhow!("来源应用桌面条目未声明图标"))?
        .to_owned();

    let (send, receive) = mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = send.send(themed_icon(&icon_name));
    })
    .context("无法调度系统图标主题查询")?;
    let png = receive
        .recv_timeout(Duration::from_secs(2))
        .context("等待系统图标主题查询超时或查询已取消")??;
    let pixels = image::load_from_memory_with_format(&png, ImageFormat::Png)
        .context("无法解码系统主题图标")?
        .into_rgba8();
    encode_icon(pixels)
}

#[cfg(target_os = "linux")]
fn themed_icon(icon_name: &str) -> Result<Vec<u8>> {
    use gtk::prelude::IconThemeExt;

    let theme = gtk::IconTheme::default().ok_or_else(|| anyhow!("系统图标主题不可用"))?;
    let icon = gio::Icon::for_string(icon_name).context("无法解析来源应用图标名称")?;
    let info = theme
        .lookup_by_gicon_for_scale(&icon, 32, 3, gtk::IconLookupFlags::empty())
        .ok_or_else(|| anyhow!("系统图标主题中未找到来源应用图标: {icon_name}"))?;
    let pixels = info.load_icon().context("无法加载来源应用图标")?;
    pixels
        .save_to_bufferv("png", &[])
        .context("无法编码来源应用图标")
}

pub(crate) async fn refresh_saved(
    app: tauri::AppHandle,
    storage: std::sync::Arc<crate::storage::Storage>,
    stop: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let sources = storage.source_apps().await.map_err(anyhow::Error::msg)?;
    let mut updated = false;
    for source in sources {
        if stop.is_cancelled() {
            return Ok(());
        }
        if source
            .icon
            .as_deref()
            .is_some_and(|icon| !needs_refresh(icon))
        {
            continue;
        }
        let handle = app.clone();
        let application = source.application.clone();
        let icon = tokio::task::spawn_blocking(move || {
            application_icon(&application, Some(&source.path), &handle)
        })
        .await?;
        if stop.is_cancelled() {
            return Ok(());
        }
        match icon {
            Ok(icon) if should_replace(source.icon.as_deref(), &icon) => {
                updated |= storage
                    .replace_app_icon(source.id, source.icon, icon)
                    .await
                    .map_err(anyhow::Error::msg)?;
            }
            Err(error) => {
                tracing::debug!(application = %source.application, %error, "saved source icon refresh unavailable");
            }
            _ => {}
        }
    }
    if updated && !stop.is_cancelled() {
        (crate::events::sink(app))("clipboard-history-updated", serde_json::Value::Null);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    fn variants(sizes: &[u32]) -> Vec<u32> {
        let mut data = Vec::new();
        for size in sizes {
            data.extend([*size, *size]);
            data.extend(std::iter::repeat_n(0x8040_2010, (*size * *size) as usize));
        }
        data
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn konsole_uses_closest_sufficient_variant_instead_of_first() {
        let pixels = select_x11_icon(&variants(&[26, 36, 52, 78, 104, 208, 416])).unwrap();
        assert_eq!(pixels.dimensions(), (104, 104));
        assert_eq!(pixels.get_pixel(0, 0).0, [0x40, 0x20, 0x10, 0x80]);
        let icon = encode_icon(pixels).unwrap();
        assert_eq!(resolution(&icon), ICON_SIZE);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn fallback_keeps_largest_available_bitmap_without_upscaling() {
        let pixels = select_x11_icon(&variants(&[16, 48, 32])).unwrap();
        let icon = encode_icon(pixels).unwrap();
        assert_eq!(resolution(&icon), 48);
        assert!(needs_refresh(&icon));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn malformed_or_truncated_property_never_indexes_outside_valid_pixels() {
        for data in [vec![], vec![16], vec![0, 16], vec![u32::MAX, u32::MAX, 0]] {
            assert!(select_x11_icon(&data).is_err());
        }
        let mut data = variants(&[32]);
        data.extend([104, 104, 0]);
        assert_eq!(select_x11_icon(&data).unwrap().dimensions(), (32, 32));
    }

    #[test]
    fn cache_upgrade_does_not_replace_clear_icons_with_a_blurry_fallback() {
        let small = encode_icon(RgbaImage::new(16, 16)).unwrap();
        let clear = encode_icon(RgbaImage::new(96, 96)).unwrap();
        assert!(should_replace(Some(&small), &clear));
        assert!(!should_replace(Some(&clear), &small));
        assert!(!should_replace(Some(&clear), "not a PNG"));
        assert!(should_replace(Some("not a PNG"), &clear));
    }

    #[tokio::test]
    async fn saved_icon_refresh_preserves_live_updates_and_higher_resolution() {
        use crate::storage::Storage;
        use serde_json::json;

        let storage = Storage {
            connection: tokio_rusqlite::Connection::open_in_memory().await.unwrap(),
        };
        storage
            .call(|connection| connection.execute_batch(data_format::INITIAL_SCHEMA))
            .await
            .unwrap();
        let id = storage
            .upsert_app(&json!({"application":"konsole", "path":"/usr/bin/konsole"}))
            .await
            .unwrap();
        let small = encode_icon(RgbaImage::new(26, 26)).unwrap();
        let clear = encode_icon(RgbaImage::new(96, 96)).unwrap();
        assert!(
            storage
                .replace_app_icon(id.clone(), None, small.clone())
                .await
                .unwrap()
        );
        assert!(
            storage
                .replace_app_icon(id.clone(), Some(small.clone()), clear.clone())
                .await
                .unwrap()
        );
        // A delayed startup lookup cannot replace a newer capture, even with a valid icon.
        assert!(
            !storage
                .replace_app_icon(id.clone(), Some(small.clone()), small.clone())
                .await
                .unwrap()
        );
        storage
            .upsert_app(&json!({"application":"konsole", "path":"/usr/bin/konsole", "icon":small}))
            .await
            .unwrap();
        let apps = storage.source_apps().await.unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].icon.as_deref(), Some(clear.as_str()));
    }
}
