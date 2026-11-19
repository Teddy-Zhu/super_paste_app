use gpui_kit::{App, Bounds, DisplayId, Pixels, Point, Size, WindowBounds, WindowDecorations, WindowOptions, point, px, size};
use super_paste_core::{Error, Result};
use super_paste_format::{APPLICATION_ID, OverlayPosition};
use super_paste_platform::geometry::WorkAreaSnapshot;

#[derive(Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub bounds: Bounds<Pixels>,
    pub display_bounds: Bounds<Pixels>,
    pub display_id: DisplayId,
    // The legacy X11 host constrains panel thickness in native work-area pixels.
    panel_pixel_scale: f32,
}

pub fn work_area(snapshot: Option<&WorkAreaSnapshot>, cx: &App) -> Result<WorkArea> {
    if let Some(snapshot) = snapshot {
        let area = &snapshot.cursor_area;
        area.validate()?;
        if let Some(root) = &snapshot.x11_root {
            let display = cx.displays().into_iter().find(|display| display.id() == DisplayId::new(u64::from(root.index)))
                .ok_or_else(|| Error::Platform("光标所在的 X11 屏幕已不可用".into()))?;
            let screen = display.bounds(); validate(screen)?;
            let scale = f32::from(root.width) / f32::from(screen.size.width);
            let scale_y = f32::from(root.height) / f32::from(screen.size.height);
            if !scale.is_finite() || scale <= 0.0 || (scale - scale_y).abs() > scale * 0.001 {
                return Err(Error::Platform("X11 屏幕尺寸已变化，请重新打开窗口".into()));
            }
            let bounds = Bounds::new(screen.origin + point(px(area.x as f32 / scale), px(area.y as f32 / scale)),
                size(px(area.width as f32 / scale), px(area.height as f32 / scale)));
            validate(bounds)?;
            return Ok(WorkArea { bounds, display_bounds: screen, display_id: display.id(), panel_pixel_scale: scale });
        }
        let bounds = Bounds::new(point(px(area.x as f32), px(area.y as f32)), size(px(area.width as f32), px(area.height as f32)));
        let center = bounds.center();
        let display = cx.displays().into_iter().find(|display| display.bounds().contains(&center))
            .ok_or_else(|| Error::Platform("无法匹配光标所在的显示器".into()))?;
        return Ok(WorkArea { bounds, display_bounds: display.bounds(), display_id: display.id(), panel_pixel_scale: 1.0 });
    }
    let display = cx.display_under_cursor().map_err(|error| Error::Platform(format!("读取光标所在显示器失败：{error}").into()))?
        .or_else(|| cx.primary_display()).ok_or_else(|| Error::Platform("没有可用的显示器".into()))?;
    let bounds = display.visible_bounds();
    validate(bounds)?;
    Ok(WorkArea { bounds, display_bounds: display.bounds(), display_id: display.id(), panel_pixel_scale: 1.0 })
}

pub fn settings_options(work: WorkArea) -> Result<WindowOptions> {
    validate(work.bounds)?;
    let width = f32::from(work.bounds.size.width);
    let height = f32::from(work.bounds.size.height);
    let extent = size(px((width * 0.78).round().clamp(960.0, 1400.0).min(width)), px((height * 0.82).round().clamp(640.0, 1000.0).min(height)));
    let origin = Point {
        x: work.bounds.origin.x + (work.bounds.size.width - extent.width) / 2.0,
        y: work.bounds.origin.y + (work.bounds.size.height - extent.height) / 2.0,
    };
    Ok(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, extent))),
        titlebar: None,
        app_owns_titlebar_drag: true,
        show: true,
        focus: true,
        display_id: Some(work.display_id),
        app_id: Some(APPLICATION_ID.into()),
        window_min_size: Some(size(px(720.0_f32.min(width)), px(520.0_f32.min(height)))),
        window_decorations: Some(WindowDecorations::Client),
        is_resizable: true,
        is_minimizable: true,
        ..Default::default()
    })
}

pub fn overlay_bounds(work: WorkArea, position: OverlayPosition) -> Result<Bounds<Pixels>> {
    validate(work.bounds)?;
    let width = f32::from(work.bounds.size.width);
    let height = f32::from(work.bounds.size.height);
    let scale = work.panel_pixel_scale;
    let thickness = (height * scale * 0.36).round().clamp(260.0, 480.0) / scale;
    let mut bounds = work.bounds;
    match position {
        OverlayPosition::Top => bounds.size.height = px(thickness.min(height)),
        OverlayPosition::Bottom => {
            bounds.size.height = px(thickness.min(height));
            bounds.origin.y += work.bounds.size.height - bounds.size.height;
        }
        OverlayPosition::Left => bounds.size.width = px(thickness.min(width)),
        OverlayPosition::Right => {
            bounds.size.width = px(thickness.min(width));
            bounds.origin.x += work.bounds.size.width - bounds.size.width;
        }
    }
    Ok(bounds)
}

pub fn overlay_options(work: WorkArea, position: OverlayPosition, wayland: bool) -> Result<WindowOptions> {
    let bounds = overlay_bounds(work, position)?;
    let mut kind = gpui_kit::WindowKind::PopUp;
    #[cfg(target_os = "linux")]
    if wayland {
        use gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
        let display = work.display_bounds;
        let left = bounds.origin.x - display.origin.x;
        let top = bounds.origin.y - display.origin.y;
        let right = display.origin.x + display.size.width - bounds.origin.x - bounds.size.width;
        let bottom = display.origin.y + display.size.height - bounds.origin.y - bounds.size.height;
        let (anchor, margin) = match position {
            OverlayPosition::Top => (Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, (top, px(0.0), px(0.0), left)),
            OverlayPosition::Bottom => (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, (px(0.0), px(0.0), bottom, left)),
            OverlayPosition::Left => (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (top, px(0.0), px(0.0), left)),
            OverlayPosition::Right => (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (top, right, px(0.0), px(0.0))),
        };
        // A work area already excludes panels. -1 prevents the compositor from
        // subtracting its exclusive zones a second time.
        let margin = match position {
            OverlayPosition::Top | OverlayPosition::Bottom => (margin.0, right, margin.2, margin.3),
            OverlayPosition::Left | OverlayPosition::Right => (margin.0, margin.1, bottom, margin.3),
        };
        kind = gpui_kit::WindowKind::LayerShell(LayerShellOptions { namespace: APPLICATION_ID.into(), layer: Layer::Overlay,
            anchor, margin: Some(margin), exclusive_zone: Some(px(-1.0)), keyboard_interactivity: KeyboardInteractivity::None, ..Default::default() });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = wayland;
    Ok(WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), titlebar: None,
        focus: false, show: false, kind, is_movable: false, app_owns_titlebar_drag: true,
        is_resizable: false, is_minimizable: false, display_id: Some(work.display_id),
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        // Popup/layer surfaces have no native frame. Kit client decorations would
        // add shadow insets, changing both the screen-edge position and content size.
        app_id: Some(APPLICATION_ID.into()), window_decorations: Some(WindowDecorations::Server), ..Default::default() })
}

fn validate(bounds: Bounds<Pixels>) -> Result<()> {
    if [bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height].into_iter()
        .any(|value| !f32::from(value).is_finite()) || bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0)
    {
        return Err(Error::Platform("显示器工作区无效".into()));
    }
    Ok(())
}
