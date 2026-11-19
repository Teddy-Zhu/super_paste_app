use super::*;
use crate::AppleTextSystem;
use anyhow::Result;
use gpui::{DevicePixels, GlyphInk, PlatformTextSystem, RenderGlyphParams, font, point, px};

fn variation(font: &CTFont, tag: i32) -> Option<f64> {
    unsafe {
        let native = &*font.as_concrete_TypeRef().cast::<objc2_core_text::CTFont>();
        let values = native.variation()?;
        let values = CFDictionary::<CFNumber, CFType>::wrap_under_get_rule(CFRetained::as_ptr(&values).as_ptr().cast());
        values.find(&CFNumber::from(tag))?.downcast::<CFNumber>()?.to_f64()
    }
}

#[test]
fn requested_fractional_weight_reaches_native_axes_and_fallback_context() -> Result<()> {
    let fonts = AppleTextSystem::new();
    fonts.add_fonts(vec![include_bytes!("../../../../cosmic-text/fonts/InterVariable.ttf").as_slice().into()])?;
    let family = fonts.0.read().memory_source.all_families()?.pop().unwrap();
    let base = font(family);
    let base_id = fonts.font_id(&base)?;
    let original = fonts.0.read().fonts[base_id.0].source.native_font();
    let optical_size = variation(&original, i32::from_be_bytes(*b"opsz"));
    // A nondefault unrelated axis must survive replacement of wght, rather
    // than relying on descriptor merging to merge the nested dictionaries.
    unsafe {
        let optical = CFDictionary::from_CFType_pairs(&[(CFNumber::from(i32::from_be_bytes(*b"opsz")), CFNumber::from(20.))]);
        let attrs = CFDictionary::from_CFType_pairs(&[(key(objc2_core_text::kCTFontVariationAttribute), optical.as_CFType())]);
        let native = &*original.as_concrete_TypeRef().cast::<objc2_core_text::CTFont>();
        let descriptor = native.font_descriptor().copy_with_attributes(&*attrs.as_concrete_TypeRef().cast::<objc2_core_foundation::CFDictionary>());
        let copy = native.copy_with_attributes(0., std::ptr::null(), Some(&descriptor));
        let font = CTFont::wrap_under_create_rule(CFRetained::into_raw(copy).as_ptr().cast());
        assert_eq!(variation(&with_axes(&font, Context::new(FontStyle::Normal, 650.5)), i32::from_be_bytes(*b"opsz")), Some(20.));
    }
    let matrix = |font: &CTFont| {
        let m = font.get_matrix();
        (m.a, m.b, m.c, m.d, m.tx, m.ty)
    };
    let mut rasters = Vec::new();
    for weight in [350.5f32, 650.5, 1000.] {
        let mut requested = base.clone();
        requested.weight = gpui::FontWeight(weight);
        let id = fonts.font_id(&requested)?;
        assert_eq!(fonts.font_id(&requested)?, id);
        {
            let state = fonts.0.read();
            let info = &state.fonts[id.0];
            let native = info.source.native_font();
            assert_eq!(info.context.weight(), weight);
            assert!(!info.synthetic_bold, "an actual variable weight must not be drawn twice");
            assert!((variation(&native, WEIGHT).unwrap() - f64::from(weight.min(900.))).abs() < 0.001);
            assert_eq!(variation(&native, i32::from_be_bytes(*b"opsz")), optical_size);
            assert_eq!(native.pt_size(), original.pt_size());
            assert_eq!(matrix(&native), matrix(&original));
        }
        let text = "office 中文";
        let layout = fonts.layout_line(text, px(32.), &[FontRun { font_id: id, len: text.len(), font_size: None }]);
        let state = fonts.0.read();
        assert!(!layout.runs.is_empty());
        for run in &layout.runs { assert_eq!(state.fonts[run.font_id.0].context.weight(), weight); }
        drop(state);
        let params = RenderGlyphParams {
            font_id: id, glyph_id: fonts.glyph_for_char(id, 'A').unwrap(), font_size: px(32.),
            subpixel_variant: point(1u8, 1u8), scale_factor: 1.5,
            is_emoji: false, subpixel_rendering: false, dilation: 0,
        };
        let bounds = fonts.glyph_raster_bounds(&params)?;
        let (_, bytes) = fonts.rasterize_glyph(&params, bounds)?;
        rasters.push(bytes.iter().map(|&value| u64::from(value)).sum::<u64>());
    }
    assert!(rasters[0] < rasters[1] && rasters[1] < rasters[2], "native outlines must reflect the requested axis");
    Ok(())
}

#[test]
fn missing_bold_face_unifies_ink_bounds_and_pixels_without_changing_shaping() -> Result<()> {
    let fonts = AppleTextSystem::new();
    fonts.add_fonts(vec![include_bytes!("../../../tests/fonts/lilex/Lilex-Regular.ttf").as_slice().into()])?;
    for style in [FontStyle::Normal, FontStyle::with_oblique_angle(30.).unwrap()] {
        let mut regular = font("Lilex");
        regular.style = style;
        let regular_id = fonts.font_id(&regular)?;
        let mut bold = regular;
        bold.weight = gpui::FontWeight(700.);
        let bold_id = fonts.font_id(&bold)?;
        let glyph = fonts.glyph_for_char(regular_id, 'A').unwrap();
        {
            let state = fonts.0.read();
            assert!(state.fonts[bold_id.0].synthetic_bold);
            assert_eq!(state.fonts[regular_id.0].source.native_font(), state.fonts[bold_id.0].source.native_font());
        }
        let layout = |id| fonts.layout_line("Affi", px(32.), &[FontRun { font_id: id, len: 4, font_size: None }]);
        let regular_layout = layout(regular_id);
        let bold_layout = layout(bold_id);
        let geometry = |line: &gpui::LineLayout| line.runs.iter().flat_map(|run| &run.glyphs)
            .map(|glyph| (glyph.id, glyph.index, glyph.position, glyph.advance)).collect::<Vec<_>>();
        assert_eq!(bold_layout.width, regular_layout.width);
        assert_eq!(geometry(&bold_layout), geometry(&regular_layout));
        let mut ends = Vec::new();
        let mut alpha = Vec::new();
        for id in [regular_id, bold_id] {
            let mut ink = GlyphInk::new(&[px(-100.)..px(100.)])?;
            fonts.intersect_glyph_ink(id, glyph, px(32.), &mut ink)?;
            ends.push(ink.intersections()?[0].clone().unwrap());
            let params = RenderGlyphParams {
                font_id: id, glyph_id: glyph, font_size: px(32.),
                subpixel_variant: point(1u8, 1u8), scale_factor: 1.5,
                is_emoji: false, subpixel_rendering: false, dilation: 0,
            };
            let bounds = fonts.glyph_raster_bounds(&params)?;
            let (size, bytes) = fonts.rasterize_glyph(&params, bounds)?;
            assert_eq!(bytes.len(), size.width.0 as usize * size.height.0 as usize);
            alpha.push(bytes.iter().map(|&value| u64::from(value)).sum::<u64>());
        }
        assert!((ends[1].start.as_f32() - ends[0].start.as_f32()).abs() < 0.001);
        assert!((ends[1].end.as_f32() - ends[0].end.as_f32() - 32. / 36.).abs() < 0.001);
        assert!(alpha[1] > alpha[0], "synthetic bold must change actual pixel coverage");
    }
    let native = core_text::font::new_from_name("Helvetica", 16.).unwrap();
    let context = Context::new(FontStyle::Italic, 700.);
    let selected = with_fallback_face(&native, context);
    assert_eq!(selected.symbolic_traits() & (kCTFontItalicTrait | kCTFontBoldTrait), kCTFontItalicTrait | kCTFontBoldTrait);
    let id = fonts.0.write().id_for_native_font(selected, context);
    let state = fonts.0.read();
    assert!(!state.fonts[id.0].synthetic_bold);
    assert!(state.fonts[id.0].drawing.is_none(), "real bold italic must not receive another transform");
    Ok(())
}

#[test]
fn authored_angle_keeps_shaping_outline_and_raster_consistent() -> Result<()> {
    let fonts = AppleTextSystem::new();
    fonts.add_fonts(vec![include_bytes!("../../../tests/fonts/lilex/Lilex-Regular.ttf").as_slice().into()])?;
    let base = font("Lilex");
    let base_id = fonts.font_id(&base)?;
    let glyph = fonts.glyph_for_char(base_id, 'A').unwrap();
    let mut params = RenderGlyphParams {
        font_id: base_id, glyph_id: glyph, font_size: px(32.),
        subpixel_variant: point(1u8, 1u8), scale_factor: 1.5,
        is_emoji: false, subpixel_rendering: false, dilation: 0,
    };
    let centroid = |bytes: &[u8], size: gpui::Size<DevicePixels>, x: DevicePixels| {
        assert_eq!(bytes.len(), size.width.0 as usize * size.height.0 as usize);
        let mut total = 0u64;
        let mut weighted = 0f64;
        for (index, &alpha) in bytes.iter().enumerate() {
            total += u64::from(alpha);
            weighted += f64::from(alpha) * (f64::from(x.0) + (index % size.width.0 as usize) as f64);
        }
        assert!(total > 0, "native raster must contain the glyph");
        weighted / total as f64
    };
    let base_bounds = fonts.glyph_raster_bounds(&params)?;
    let (bitmap_size, bitmap) = fonts.rasterize_glyph(&params, base_bounds)?;
    let base_center = centroid(&bitmap, bitmap_size, base_bounds.origin.x);
    let metrics = fonts.font_metrics(base_id);
    let cap = metrics.cap_height / metrics.units_per_em as f32 * 32.;
    let band = px(-cap * 0.85)..px(-cap * 0.7);
    let mut ink = GlyphInk::new(&[band.clone()])?;
    fonts.intersect_glyph_ink(base_id, glyph, px(32.), &mut ink)?;
    let base_ink = ink.intersections()?[0].clone().unwrap();
    let regular = fonts.layout_line("Afi", px(32.), &[FontRun { font_id: base_id, len: 3, font_size: None }]);
    let geometry = |layout: &gpui::LineLayout| layout.runs.iter().flat_map(|run| run.glyphs.iter())
        .map(|glyph| (glyph.id, glyph.index, glyph.position, glyph.advance)).collect::<Vec<_>>();
    let expected_geometry = geometry(&regular);
    let cases = [-30f32, -5., 14., 30.].map(|degrees| (degrees, FontStyle::with_oblique_angle(degrees).unwrap()));
    let mut seen = Vec::new();
    for (degrees, style) in cases.into_iter().chain([(14., FontStyle::Italic)]) {
        let mut requested = base.clone();
        requested.style = style;
        let id = fonts.font_id(&requested)?;
        assert!(!seen.contains(&id), "requested style remains part of native instance identity");
        seen.push(id);
        assert_eq!(fonts.font_id(&requested)?, id);
        {
            let state = fonts.0.read();
            let info = &state.fonts[id.0];
            assert_eq!(info.source.native_font(), state.fonts[base_id.0].source.native_font());
            assert_eq!(info.context.style, requested.style);
            let drawing = info.drawing.as_ref().expect("upright fixture requires synthetic slant").native_font();
            assert!((drawing.get_matrix().c - f64::from(degrees).to_radians().tan()).abs() < 1e-8);
        }
        let layout = fonts.layout_line("Afi", px(32.), &[FontRun { font_id: id, len: 3, font_size: None }]);
        assert_eq!(layout.width, regular.width);
        assert_eq!(geometry(&layout), expected_geometry, "drawing shear must preserve shaping");
        let fallback_text = "A لا 中文";
        let fallback = fonts.layout_line(fallback_text, px(32.), &[FontRun {
            font_id: id, len: fallback_text.len(), font_size: None,
        }]);
        let state = fonts.0.read();
        let base_name = state.fonts[base_id.0].source.native_font().postscript_name();
        assert!(fallback.runs.iter().any(|run| state.fonts[run.font_id.0].source.native_font().postscript_name() != base_name));
        for run in &fallback.runs {
            assert_eq!(state.fonts[run.font_id.0].context.style, requested.style);
        }
        drop(state);
        let mut ink = GlyphInk::new(&[band.clone()])?;
        fonts.intersect_glyph_ink(id, glyph, px(32.), &mut ink)?;
        let ink = ink.intersections()?[0].clone().unwrap();
        if degrees > 0. { assert!(ink.start > base_ink.start); }
        else { assert!(ink.end < base_ink.end); }
        params.font_id = id;
        let bounds = fonts.glyph_raster_bounds(&params)?;
        let (size, bitmap) = fonts.rasterize_glyph(&params, bounds)?;
        let center = centroid(&bitmap, size, bounds.origin.x);
        if degrees > 0. { assert!(center > base_center + 0.5); }
        else { assert!(center < base_center - 0.5); }
    }
    let mut excessive = base;
    excessive.style = FontStyle::with_oblique_angle(89.99).unwrap();
    params.font_id = fonts.font_id(&excessive)?;
    assert!(fonts.glyph_raster_bounds(&params).is_err(), "reject excessive rasters before allocation");
    params.font_size = px(f32::MAX);
    assert!(fonts.glyph_raster_bounds(&params).is_err(), "reject native bounds before integer conversion");
    Ok(())
}

#[test]
fn real_italic_keeps_requested_angle_context_without_double_slant() {
    let fonts = AppleTextSystem::new();
    let native = core_text::font::new_from_name("Helvetica-Oblique", 16.).unwrap();
    let mut ids = Vec::new();
    for degrees in [-30f32, 14., 30.] {
        let requested = FontStyle::with_oblique_angle(degrees).unwrap();
        let id = {
            let mut state = fonts.0.write();
            let id = state.id_for_native_font(native.clone(), Context::new(requested, 400.));
            assert!(!ids.contains(&id));
            assert_eq!(state.id_for_native_font(native.clone(), Context::new(requested, 400.)), id);
            let info = &state.fonts[id.0];
            assert_eq!(info.source.native_font(), native);
            assert!(info.drawing.is_none());
            assert_eq!(info.context.style, requested);
            id
        };
        ids.push(id);
        let text = "office 中文";
        let layout = fonts.layout_line(text, px(24.), &[FontRun { font_id: id, len: text.len(), font_size: None }]);
        assert!(!layout.runs.is_empty());
        let state = fonts.0.read();
        for run in layout.runs {
            assert_eq!(state.fonts[run.font_id.0].context.style, requested);
        }
    }
}
