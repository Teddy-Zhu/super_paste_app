//! Font-only regression: no window, GPU device, event loop or UI is created.
use std::borrow::Cow;

use gpui::{FontRun, FontStyle, PlatformTextSystem, RenderGlyphParams, TextDirection, point, px};
use gpui_wgpu::CosmicTextSystem;

#[test]
fn authored_angles_reach_gpui_shaping_raster_and_allocation_guard() {
    let system = CosmicTextSystem::new_without_system_fonts("Inter");
    system.add_fonts(vec![Cow::Borrowed(include_bytes!("../../cosmic-text/fonts/Inter-Regular.ttf"))]).unwrap();
    let mut masks = Vec::new();
    let mut widths = Vec::new();
    let mut font_ids = Vec::new();
    for degrees in [0., 14., 20., -10.] {
        let mut font = gpui::font("Inter");
        font.style = FontStyle::with_oblique_angle(degrees).unwrap();
        let font_id = system.font_id(&font).unwrap();
        let line = system.layout_line_with_direction("Minimum office", px(40.),
            &[FontRun { font_size: None, len: "Minimum office".len(), font_id }], TextDirection::Ltr);
        assert!(!line.runs.is_empty());
        assert!(line.runs.iter().all(|run| run.font_id == font_id), "angle lost during shaping: {degrees}");
        widths.push(line.width);
        font_ids.push(font_id);
        let params = RenderGlyphParams {
            font_id, glyph_id: system.glyph_for_char(font_id, 'M').unwrap(), font_size: px(40.),
            subpixel_variant: point(0, 0), scale_factor: 1., is_emoji: false,
            subpixel_rendering: false, dilation: 0,
        };
        let bounds = system.glyph_raster_bounds(&params).unwrap();
        let (size, bytes) = system.rasterize_glyph(&params, bounds).unwrap();
        assert!(!bytes.is_empty());
        masks.push((size, bytes));
    }
    assert!(widths.iter().all(|width| *width == widths[0]), "synthetic shear must preserve advances");
    for i in 1..font_ids.len() {
        for j in 0..i {
            assert_ne!(font_ids[i], font_ids[j]);
            assert_ne!(masks[i], masks[j], "different angles must not share raster output");
        }
    }
    for degrees in [89.999, 90., -90.] {
        let mut font = gpui::font("Inter");
        font.style = FontStyle::with_oblique_angle(degrees).unwrap();
        let font_id = system.font_id(&font).unwrap();
        let params = RenderGlyphParams {
            font_id, glyph_id: system.glyph_for_char(font_id, 'M').unwrap(), font_size: px(40.),
            subpixel_variant: point(0, 0), scale_factor: 1., is_emoji: false,
            subpixel_rendering: false, dilation: 0,
        };
        let error = system.glyph_raster_bounds(&params).unwrap_err();
        assert!(error.to_string().contains("raster limits"), "{degrees}: {error}");
    }
}


#[test]
fn reverse_angles_choose_upright_synthesis_before_forward_italic_faces() {
    let system = CosmicTextSystem::new_without_system_fonts("Inter Variable");
    system.add_fonts(vec![
        Cow::Borrowed(include_bytes!("../../cosmic-text/fonts/InterVariable.ttf")),
        Cow::Borrowed(include_bytes!("../../cosmic-text/fonts/InterVariable-Italic.ttf")),
    ]).unwrap();
    for (degrees, expected) in [(14., FontStyle::Italic), (20., FontStyle::Italic), (90., FontStyle::Italic), (-20., FontStyle::Normal), (-90., FontStyle::Normal)] {
        let mut font = gpui::font("Inter Variable");
        font.style = FontStyle::with_oblique_angle(degrees).unwrap();
        let font_id = system.font_id(&font).unwrap();
        assert_eq!(system.font_weight_and_style(font_id).unwrap().1, expected, "{degrees}");
        let line = system.layout_line_with_direction("Minimum office", px(32.),
            &[FontRun { font_size: None, len: "Minimum office".len(), font_id }], TextDirection::Ltr);
        assert!(!line.runs.is_empty());
        assert!(line.runs.iter().all(|run| run.font_id == font_id),
            "native matching and fallback must agree: {degrees}");
    }
}
