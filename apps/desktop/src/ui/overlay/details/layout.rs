use gpui_kit::{Bounds, Pixels, Size, TitlebarOptions, WindowBounds, WindowDecorations, WindowKind, WindowOptions, point, px, size};
use super_paste_core::preview::{PreviewBody, PreviewModel, text_whitespace};
use super_paste_format::APPLICATION_ID;
use crate::ui::geometry::WorkArea;

pub(super) fn options(work: WorkArea, model: &PreviewModel, dimensions: Option<(u32, u32)>) -> WindowOptions {
    let available = size((work.bounds.size.width - px(64.)).max(px(1.)), (work.bounds.size.height - px(64.)).max(px(1.)));
    let limit = size(available.width.min(px(1280.)), available.height.min(px(1000.)));
    let lines = model.display_text.lines().take(40).count().max(1) as f32;
    let characters = model.display_text.chars().take(4000).count();
    let long = characters > 160;
    let wrapped_lines = lines.max((characters as f32 / 88.).ceil());
    let (width, height) = match &model.body {
        PreviewBody::Image => {
            let (width, height) = dimensions.unwrap_or((800, 600));
            let fitted = fit_image(width, height, size((limit.width - px(48.)).max(px(1.)), (limit.height - px(180.)).max(px(1.))));
            (f32::from(fitted.width) + 48., f32::from(fitted.height) + if model.text.trim_matches(text_whitespace).is_empty() { 120. } else { 220. })
        },
        PreviewBody::Table(table) => ((table.columns as f32 * 180. + 80.).clamp(640., 1280.), (table.rows.len().min(24) as f32 * 36. + 160.).clamp(320., 960.)),
        PreviewBody::Rich(document) => {
            let complex = document.nodes.len() > 3 || document.weight() > 8192 || long || lines > 3.;
            (if complex { 960. } else { 640. }, if complex { 720. } else { 340. })
        },
        PreviewBody::Code => (1040., (lines * 24. + 180.).clamp(360., 900.)),
        PreviewBody::Files => (760., (model.files_total.min(10) as f32 * 64. + 160.).clamp(320., 880.)),
        PreviewBody::Color(_) => (520., 520.),
        PreviewBody::Contact => (560., 340.),
        PreviewBody::Link(_) => (760., 420.),
        PreviewBody::Empty => (440., 280.),
        PreviewBody::Text => (if long { 820. } else { 560. }, (wrapped_lines * 25. + 160.).clamp(280., 820.)),
    };
    let extent = size(px(width.max(440.)).min(limit.width), px(height.max(280.)).min(limit.height));
    let origin = work.bounds.origin + point((work.bounds.size.width - extent.width) / 2., (work.bounds.size.height - extent.height) / 2.);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, extent))),
        titlebar: Some(TitlebarOptions { title: Some(format!("{} · 详情", model.kind.label()).into()), ..Default::default() }),
        window_decorations: Some(WindowDecorations::Server),
        kind: WindowKind::Floating,
        display_id: Some(work.display_id), app_id: Some(APPLICATION_ID.into()),
        window_min_size: Some(size(px(360.).min(extent.width), px(240.).min(extent.height))),
        is_resizable: true, is_minimizable: false, focus: true, show: true,
        ..Default::default()
    }
}

pub(super) fn fit_image(width: u32, height: u32, available: Size<Pixels>) -> Size<Pixels> {
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (f32::from(available.width).max(1.) / width).min(f32::from(available.height).max(1.) / height).min(1.);
    size(px(width * scale), px(height * scale))
}
