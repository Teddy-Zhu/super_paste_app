use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

pub fn initialize(window: &tauri::Window) -> Result<(), String> {
    let window = window.gtk_window().map_err(|e| e.to_string())?;
    if window.is_realized() {
        return Err("浮层 GTK 窗口已经 realize，无法初始化 Layer Shell".into());
    }
    if !gtk_layer_shell::is_supported() {
        return Err("当前合成器不支持 GTK Layer Shell".into());
    }
    window.init_layer_shell();
    window.set_namespace("super-paste-overlay");
    window.set_layer(Layer::Overlay);
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    Ok(())
}

pub fn place(
    window: &tauri::Window,
    area: &super::windows::WorkArea,
    position: &str,
) -> Result<(), String> {
    let window = window.gtk_window().map_err(|e| e.to_string())?;
    let display = gdk::Display::default().ok_or("GDK 显示不可用")?;
    let monitor = (0..display.n_monitors())
        .filter_map(|index| display.monitor(index))
        .find(|monitor| {
            let rect = monitor.geometry();
            area.x >= rect.x() as f64
                && area.y >= rect.y() as f64
                && area.x < (rect.x() + rect.width()) as f64
                && area.y < (rect.y() + rect.height()) as f64
        })
        .ok_or("无法将 KWin 工作区匹配到 GDK 输出")?;
    let geometry = monitor.geometry();
    window.set_monitor(&monitor);
    for (edge, anchored) in [
        (Edge::Top, position != "bottom"),
        (Edge::Bottom, position != "top"),
        (Edge::Left, position != "right"),
        (Edge::Right, position != "left"),
    ] {
        window.set_anchor(edge, anchored);
    }
    for (edge, margin) in [
        (Edge::Top, area.y - geometry.y() as f64),
        (Edge::Left, area.x - geometry.x() as f64),
        (
            Edge::Bottom,
            (geometry.y() + geometry.height()) as f64 - area.y - area.height,
        ),
        (
            Edge::Right,
            (geometry.x() + geometry.width()) as f64 - area.x - area.width,
        ),
    ] {
        window.set_layer_shell_margin(edge, margin.round().max(0.) as i32);
    }
    Ok(())
}
