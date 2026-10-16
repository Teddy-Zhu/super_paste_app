use tauri::{Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize};

use crate::error::AppError;
use crate::result::Result;

const OVERLAY_HEIGHT: u32 = 350;

/// 获取鼠标光标所在的显示器；取不到则回退主显示器。
///
/// 不使用 `overlay.current_monitor()`：Overlay 隐藏时会被移到屏幕外，
/// 此时 `current_monitor()` 可能返回 None 或错误的显示器，导致下次显示位置错乱。
/// 与原 Electron 项目一致——始终基于光标所在显示器定位。
fn monitor_under_cursor(app: &tauri::AppHandle) -> Option<Monitor> {
    if let Ok(pos) = app.cursor_position() {
        // 1) 优先使用内置 API
        if let Ok(Some(monitor)) = app.monitor_from_point(pos.x, pos.y) {
            return Some(monitor);
        }
        // 2) 回退：遍历所有显示器，判断光标点是否落在其矩形内
        if let Ok(monitors) = app.available_monitors() {
            let (cx, cy) = (pos.x as i32, pos.y as i32);
            for m in &monitors {
                let mp = m.position();
                let ms = m.size();
                let x1 = mp.x + ms.width as i32;
                let y1 = mp.y + ms.height as i32;
                if cx >= mp.x && cx < x1 && cy >= mp.y && cy < y1 {
                    return Some(m.clone());
                }
            }
            // 3) 仍未命中：取第一个显示器
            if let Some(first) = monitors.into_iter().next() {
                return Some(first);
            }
        }
    }
    // 4) 最终回退主显示器
    app.primary_monitor().ok().flatten()
}

/// 显示 Overlay 窗口（动态设置大小和位置，占满屏幕宽度/高度）。
pub fn show_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    let overlay = app
        .get_webview_window("overlay")
        .ok_or_else(|| AppError::not_found("Overlay 窗口"))?;

    // 关键：基于光标所在显示器定位，而非 overlay.current_monitor()
    let monitor = monitor_under_cursor(app)
        .ok_or_else(|| AppError::config("无法获取显示器"))?;

    let work_area = monitor.work_area();
    let wx = work_area.position.x;
    let wy = work_area.position.y;
    let ww = work_area.size.width as i32;
    let wh = work_area.size.height as i32;

    // 根据位置计算窗口大小和坐标
    let (size, pos) = match position {
        "top" => (
            PhysicalSize::new(ww as u32, OVERLAY_HEIGHT),
            PhysicalPosition::new(wx, wy),
        ),
        "bottom" => (
            PhysicalSize::new(ww as u32, OVERLAY_HEIGHT),
            PhysicalPosition::new(wx, wy + wh - OVERLAY_HEIGHT as i32),
        ),
        "left" => (
            PhysicalSize::new(OVERLAY_HEIGHT, wh as u32),
            PhysicalPosition::new(wx, wy),
        ),
        "right" => (
            PhysicalSize::new(OVERLAY_HEIGHT, wh as u32),
            PhysicalPosition::new(wx + ww - OVERLAY_HEIGHT as i32, wy),
        ),
        _ => (
            PhysicalSize::new(ww as u32, OVERLAY_HEIGHT),
            PhysicalPosition::new(wx, wy + wh - OVERLAY_HEIGHT as i32),
        ),
    };

    // 先定位到目标显示器再显示，避免先在旧位置闪现
    overlay.set_size(size)?;
    overlay.set_position(pos)?;
    overlay.show()?;
    overlay.set_focus()?;

    tracing::debug!("Overlay 已显示: {:?} @ {:?}", size, pos);
    Ok(())
}

/// 隐藏 Overlay 窗口。
///
/// 顺序：移走焦点 → `hide()` → 移到屏幕外。三重保险确保彻底隐藏，
/// 即使个别平台 `hide()` 不彻底，移到屏外也保证不再吞掉点击。
pub fn hide_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    if let Some(overlay) = app.get_webview_window("overlay") {
        // 1) 把焦点移到主窗口（若可见），近似原项目的 blur()
        if let Some(main) = app.get_webview_window("main") {
            if main.is_visible().unwrap_or(false) {
                let _ = main.set_focus();
            }
        }

        // 2) 真正隐藏窗口
        let _ = overlay.hide();

        // 3) 移到屏幕外（基于光标显示器计算屏外坐标）
        if let Some(monitor) = monitor_under_cursor(app) {
            let work_area = monitor.work_area();
            let wx = work_area.position.x;
            let wy = work_area.position.y;
            let ww = work_area.size.width as i32;
            let wh = work_area.size.height as i32;

            let hidden_pos = match position {
                "top" => PhysicalPosition::new(wx, wy - OVERLAY_HEIGHT as i32 - 100),
                "bottom" => PhysicalPosition::new(wx, wy + wh + 100),
                "left" => PhysicalPosition::new(wx - OVERLAY_HEIGHT as i32 - 100, wy),
                "right" => PhysicalPosition::new(wx + ww + 100, wy),
                _ => PhysicalPosition::new(wx, wy + wh + 100),
            };

            let _ = overlay.set_position(hidden_pos);
        }
    }
    Ok(())
}

/// 切换 Overlay 显示/隐藏。tray / 快捷键 / 命令共用此逻辑。
pub fn toggle_overlay(app: &tauri::AppHandle, position: &str) -> Result<()> {
    let overlay = app
        .get_webview_window("overlay")
        .ok_or_else(|| AppError::not_found("Overlay 窗口"))?;

    if overlay.is_visible().unwrap_or(false) {
        hide_overlay(app, position)
    } else {
        show_overlay(app, position)?;
        // 触发前端滑入动画
        let _ = overlay.emit("trigger-slide-in-animation", position);
        Ok(())
    }
}
