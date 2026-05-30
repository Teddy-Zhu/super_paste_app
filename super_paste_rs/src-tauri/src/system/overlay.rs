use tauri::{Manager, PhysicalPosition, PhysicalSize};

use crate::error::AppError;
use crate::result::Result;

/// Overlay 窗口高度常量（与原项目 WINDOW_HEIGHT = 350 一致）
const WINDOW_HEIGHT: i32 = 350;

/// 计算 Overlay 可见位置（屏幕边缘）
fn calculate_visible_position(
    work_x: i32,
    work_y: i32,
    work_w: i32,
    work_h: i32,
    position: &str,
) -> PhysicalPosition<i32> {
    match position {
        "top" => PhysicalPosition::new(work_x, work_y),
        "bottom" => PhysicalPosition::new(work_x, work_y + work_h - WINDOW_HEIGHT),
        "left" => PhysicalPosition::new(work_x, work_y),
        "right" => PhysicalPosition::new(work_x + work_w - WINDOW_HEIGHT, work_y),
        _ => PhysicalPosition::new(work_x, work_y + work_h - WINDOW_HEIGHT),
    }
}

/// 计算 Overlay 隐藏位置（屏幕外）
fn calculate_hidden_position(
    work_x: i32,
    work_y: i32,
    work_w: i32,
    work_h: i32,
    position: &str,
) -> PhysicalPosition<i32> {
    match position {
        "top" => PhysicalPosition::new(work_x, work_y - WINDOW_HEIGHT - 10),
        "bottom" => PhysicalPosition::new(work_x, work_y + work_h + 10),
        "left" => PhysicalPosition::new(work_x - WINDOW_HEIGHT - 10, work_y),
        "right" => PhysicalPosition::new(work_x + work_w + 10, work_y),
        _ => PhysicalPosition::new(work_x, work_y + work_h + 10),
    }
}

/// 获取显示器工作区域
fn get_work_area(app: &tauri::AppHandle) -> Result<(i32, i32, i32, i32)> {
    let overlay = app
        .get_webview_window("overlay")
        .ok_or_else(|| AppError::not_found("Overlay 窗口"))?;

    let monitor = overlay
        .current_monitor()?
        .ok_or_else(|| AppError::config("无法获取显示器"))?;

    let work_area = monitor.work_area();
    Ok((
        work_area.position.x,
        work_area.position.y,
        work_area.size.width as i32,
        work_area.size.height as i32,
    ))
}

/// 显示 Overlay 窗口（移动到屏幕边缘位置）
pub fn show_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    let overlay = app
        .get_webview_window("overlay")
        .ok_or_else(|| AppError::not_found("Overlay 窗口"))?;

    let (work_x, work_y, work_w, work_h) = get_work_area(app)?;
    let pos = calculate_visible_position(work_x, work_y, work_w, work_h, position);

    overlay.set_position(pos)?;
    overlay.show()?;
    overlay.set_focus()?;

    tracing::debug!("Overlay 已显示于 {} 位置: {:?}", position, pos);
    Ok(())
}

/// 隐藏 Overlay 窗口（移动到屏幕外）
pub fn hide_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    if let Some(overlay) = app.get_webview_window("overlay") {
        let (work_x, work_y, work_w, work_h) = get_work_area(app)?;
        let hidden_pos = calculate_hidden_position(work_x, work_y, work_w, work_h, position);

        overlay.set_position(hidden_pos)?;
        overlay.hide()?;

        tracing::debug!("Overlay 已隐藏到屏幕外: {:?}", hidden_pos);
    }
    Ok(())
}
