use tauri::{Manager, PhysicalPosition, PhysicalSize};

use crate::error::AppError;
use crate::result::Result;

/// Overlay 窗口高度常量（与原项目 WINDOW_HEIGHT = 350 一致）
const WINDOW_HEIGHT: u32 = 350;

/// Overlay 位置类型
pub type OverlayPosition = &'static str;

/// 计算 Overlay 可见位置（屏幕边缘，占满宽度/高度）
fn calculate_visible_bounds(
    work_area_x: i32,
    work_area_y: i32,
    work_area_width: u32,
    work_area_height: u32,
    position: &str,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    match position {
        "top" => (
            PhysicalPosition::new(work_area_x, work_area_y),
            PhysicalSize::new(work_area_width, WINDOW_HEIGHT),
        ),
        "bottom" => (
            PhysicalPosition::new(work_area_x, work_area_y + work_area_height as i32 - WINDOW_HEIGHT as i32),
            PhysicalSize::new(work_area_width, WINDOW_HEIGHT),
        ),
        "left" => (
            PhysicalPosition::new(work_area_x, work_area_y),
            PhysicalSize::new(WINDOW_HEIGHT, work_area_height),
        ),
        "right" => (
            PhysicalPosition::new(work_area_x + work_area_width as i32 - WINDOW_HEIGHT as i32, work_area_y),
            PhysicalSize::new(WINDOW_HEIGHT, work_area_height),
        ),
        _ => (
            // 默认 bottom
            PhysicalPosition::new(work_area_x, work_area_y + work_area_height as i32 - WINDOW_HEIGHT as i32),
            PhysicalSize::new(work_area_width, WINDOW_HEIGHT),
        ),
    }
}

/// 计算 Overlay 隐藏位置（屏幕外）
fn calculate_hidden_bounds(
    work_area_x: i32,
    work_area_y: i32,
    work_area_width: u32,
    work_area_height: u32,
    position: &str,
    visible_size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    match position {
        "top" => PhysicalPosition::new(work_area_x, work_area_y - visible_size.height as i32),
        "bottom" => PhysicalPosition::new(work_area_x, work_area_y + work_area_height as i32),
        "left" => PhysicalPosition::new(work_area_x - visible_size.width as i32, work_area_y),
        "right" => PhysicalPosition::new(work_area_x + work_area_width as i32, work_area_y),
        _ => PhysicalPosition::new(work_area_x, work_area_y + work_area_height as i32),
    }
}

/// 显示 Overlay 窗口（设置到可见位置）
pub fn show_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    let overlay = app
        .get_webview_window("overlay")
        .ok_or_else(|| AppError::not_found("Overlay 窗口"))?;

    let monitor = overlay
        .current_monitor()?
        .ok_or_else(|| AppError::config("无法获取显示器"))?;

    let work_area = monitor.work_area();
    let work_x = work_area.position.x;
    let work_y = work_area.position.y;
    let work_w = work_area.size.width;
    let work_h = work_area.size.height;

    let (pos, size) = calculate_visible_bounds(work_x, work_y, work_w, work_h, position);

    // 设置窗口大小和位置
    overlay.set_size(size)?;
    overlay.set_position(pos)?;
    overlay.show()?;
    overlay.set_focus()?;

    tracing::debug!("Overlay 窗口已显示于: {}, 位置: {:?}, 大小: {:?}", position, pos, size);
    Ok(())
}

/// 隐藏 Overlay 窗口（移动到屏幕外）
pub fn hide_overlay(app: &tauri::AppHandle) -> Result<()> {
    if let Some(overlay) = app.get_webview_window("overlay") {
        let monitor = overlay.current_monitor()?;
        if let Some(monitor) = monitor {
            let work_area = monitor.work_area();
            let work_x = work_area.position.x;
            let work_y = work_area.position.y;
            let work_w = work_area.size.width;
            let work_h = work_area.size.height;

            // 获取当前配置的位置（默认 bottom）
            let position = "bottom"; // TODO: 从配置读取
            let size = overlay.outer_size()?;
            let hidden_pos = calculate_hidden_bounds(
                work_x, work_y, work_w, work_h, position,
                PhysicalSize::new(size.width, size.height),
            );

            // 移动到屏幕外再隐藏
            overlay.set_position(hidden_pos)?;
        }
        overlay.hide()?;
        tracing::debug!("Overlay 窗口已隐藏");
    }
    Ok(())
}
