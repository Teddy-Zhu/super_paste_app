use tauri::{Manager, PhysicalPosition, PhysicalSize};

use crate::error::AppError;
use crate::result::Result;

pub struct OverlayPositioner;

impl OverlayPositioner {
    /// 根据配置位置（top/bottom/left/right/center）计算 Overlay 窗口坐标。
    pub fn calculate_position(
        position: &str,
        window_size: PhysicalSize<u32>,
        screen_size: PhysicalSize<u32>,
    ) -> PhysicalPosition<i32> {
        let margin = 50i32;
        match position {
            "top" => PhysicalPosition::new(
                ((screen_size.width - window_size.width) / 2) as i32,
                margin,
            ),
            "bottom" => PhysicalPosition::new(
                ((screen_size.width - window_size.width) / 2) as i32,
                (screen_size.height - window_size.height) as i32 - margin,
            ),
            "left" => PhysicalPosition::new(
                margin,
                ((screen_size.height - window_size.height) / 2) as i32,
            ),
            "right" => PhysicalPosition::new(
                (screen_size.width - window_size.width) as i32 - margin,
                ((screen_size.height - window_size.height) / 2) as i32,
            ),
            _ => PhysicalPosition::new(
                ((screen_size.width - window_size.width) / 2) as i32,
                ((screen_size.height - window_size.height) / 2) as i32,
            ),
        }
    }
}

/// 显示 Overlay 窗口并定位到指定位置。
pub fn show_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    let overlay = app
        .get_webview_window("overlay")
        .ok_or_else(|| AppError::not_found("Overlay 窗口"))?;

    let monitor = overlay
        .current_monitor()?
        .ok_or_else(|| AppError::config("无法获取显示器"))?;

    let screen_size = monitor.size();
    let window_size = overlay.outer_size()?;

    // 将 window_size 转换为 PhysicalSize<u32>
    let window_size_u32 = PhysicalSize::new(window_size.width, window_size.height);
    let pos = OverlayPositioner::calculate_position(position, window_size_u32, *screen_size);

    overlay.set_position(pos)?;
    overlay.show()?;
    overlay.set_focus()?;

    tracing::debug!("Overlay 窗口已显示于: {}", position);
    Ok(())
}

/// 隐藏 Overlay 窗口。
pub fn hide_overlay(app: &tauri::AppHandle) -> Result<()> {
    if let Some(overlay) = app.get_webview_window("overlay") {
        overlay.hide()?;
        tracing::debug!("Overlay 窗口已隐藏");
    }
    Ok(())
}
