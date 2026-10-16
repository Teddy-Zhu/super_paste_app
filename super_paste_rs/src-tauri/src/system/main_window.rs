use tauri::{Manager, PhysicalSize};

use crate::error::AppError;
use crate::result::Result;

/// 主窗口配置（与原项目 MAIN_WINDOW_CONFIG 一致）
const WIDTH_RATIO: f64 = 0.7;
const HEIGHT_RATIO: f64 = 0.75;
const MIN_WIDTH: u32 = 800;
const MAX_WIDTH: u32 = 1200;
const MIN_HEIGHT: u32 = 600;
const MAX_HEIGHT: u32 = 900;

/// 计算主窗口最佳尺寸（屏幕 70%×75%，限制在 800-1200 × 600-900）
fn calculate_main_window_size(screen_width: u32, screen_height: u32) -> (u32, u32) {
    let width = ((screen_width as f64 * WIDTH_RATIO) as u32).clamp(MIN_WIDTH, MAX_WIDTH);
    let height = ((screen_height as f64 * HEIGHT_RATIO) as u32).clamp(MIN_HEIGHT, MAX_HEIGHT);
    (width, height)
}

/// 显示主窗口（动态计算大小并居中）
pub fn show_main_window(app: &tauri::AppHandle) -> Result<()> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::not_found("主窗口"))?;

    let monitor = window
        .current_monitor()?
        .ok_or_else(|| AppError::config("无法获取显示器"))?;

    let work_area = monitor.work_area();
    let (width, height) = calculate_main_window_size(
        work_area.size.width,
        work_area.size.height,
    );

    window.set_size(PhysicalSize::new(width, height))?;
    window.center()?;
    window.show()?;
    window.set_focus()?;

    tracing::debug!("主窗口已显示: {}x{}", width, height);
    Ok(())
}
