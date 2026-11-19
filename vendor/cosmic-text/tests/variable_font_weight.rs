use std::path::PathBuf;

use cosmic_text::{fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight};

/// Variable fonts must be matched at all weights within their `wght` axis
/// range, not just the default weight they register at in fontdb, otherwise
/// they will fall back to a system font despite being able to provide the
/// requested weight.
#[test]
fn variable_font_all_weights_match() {
    let repo_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let fonts_path = PathBuf::from(&repo_dir).join("fonts");

    let mut font_system = FontSystem::new();
    font_system
        .db_mut()
        .load_font_data(std::fs::read(fonts_path.join("InterVariable.ttf")).unwrap());

    for w in [100, 200, 300, 400, 500, 600, 700, 800, 900] {
        let metrics = Metrics::new(16.0, 20.0);
        let mut buffer = Buffer::new(&mut font_system, metrics);

        let glyph_font_ids: Vec<fontdb::ID>;
        {
            let mut buffer = buffer.borrow_with(&mut font_system);
            let attrs = Attrs::new()
                .family(Family::Name("Inter Variable"))
                .weight(Weight(w));
            buffer.set_size(Some(300.0), Some(100.0));
            buffer.set_text("Hello world", &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(true);

            glyph_font_ids = buffer
                .layout_runs()
                .flat_map(|run| run.glyphs.iter().map(|g| g.font_id))
                .collect();
        }

        assert!(!glyph_font_ids.is_empty(), "Weight {w}: no glyphs produced");

        for id in &glyph_font_ids {
            let face = font_system.db().face(*id).unwrap();
            let family = &face.families[0].0;
            assert!(
                family.contains("Inter"),
                "Weight {w}: expected Inter, got \"{family}\""
            );
        }
    }
}

/// Reuse the licensed complete Inter fixture, remapping only its weight axis
/// in memory. This exercises native variation propagation without another font
/// download or asserting that the fixture's outlines are a designed italic.
fn style_axis_fixture(tag: &[u8; 4]) -> Vec<u8> {
    let mut bytes = include_bytes!("../fonts/InterVariable.ttf").to_vec();
    let u16_at = |data: &[u8], at| u16::from_be_bytes(data[at..at + 2].try_into().unwrap()) as usize;
    let u32_at = |data: &[u8], at| u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let record = (0..u16_at(&bytes, 4)).map(|i| 12 + i * 16)
        .find(|&at| &bytes[at..at + 4] == b"fvar").unwrap();
    let fvar = u32_at(&bytes, record + 8);
    let axes = fvar + u16_at(&bytes, fvar + 4);
    let size = u16_at(&bytes, fvar + 10);
    let axis = (0..u16_at(&bytes, fvar + 8)).map(|i| axes + i * size)
        .find(|&at| &bytes[at..at + 4] == b"wght").unwrap();
    bytes[axis..axis + 4].copy_from_slice(tag);
    let values: [i32; 3] = if tag == b"ital" { [0, 0, 65536] } else { [-20 * 65536, 0, 0] };
    for (index, value) in values.into_iter().enumerate() {
        bytes[axis + 4 + index * 4..axis + 8 + index * 4].copy_from_slice(&value.to_be_bytes());
    }
    bytes
}

#[test]
fn native_style_axes_survive_matching_shaping_and_raster_cache() {
    use cosmic_text::{CacheKeyFlags, Style, SwashCache};
    use std::{collections::{BTreeSet, HashSet}, sync::Arc};
    for (tag, style) in [(b"ital", Style::Italic), (b"slnt", Style::Oblique)] {
        let mut db = fontdb::Database::new();
        db.load_font_data(style_axis_fixture(tag));
        db.set_sans_serif_family("Inter Variable");
        let id = db.faces().next().unwrap().id;
        let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
        let regular = fonts.get_font(id, Weight::NORMAL).unwrap();
        let native = fonts.get_font_with_style(id, Weight::NORMAL, style).unwrap();
        assert!(native.supports_style(style));
        assert_eq!(native.effective_style(), style);
        assert!(native.is_italic_or_oblique());
        assert_ne!(regular.shaper().coords(), native.shaper().coords());
        assert!(!Arc::ptr_eq(&regular, &native));
        assert!(Arc::ptr_eq(&native, &fonts.get_font_with_style(id, Weight::NORMAL, style).unwrap()));
        assert!(Arc::ptr_eq(&regular, &fonts.get_font_with_style(id, Weight::NORMAL, Style::Normal).unwrap()));

        for family in ["Inter Variable", "Unavailable Style Fixture Family"] {
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(32., 40.));
            {
                let mut editor = buffer.borrow_with(&mut fonts);
                editor.set_size(Some(600.), Some(80.));
                editor.set_text("Minimum office 123", &Attrs::new().family(Family::Name(family)).style(style), Shaping::Advanced, None);
                editor.shape_until_scroll(true);
            }
            let glyphs: Vec<_> = buffer.layout_runs().flat_map(|run| run.glyphs.iter().cloned()).collect();
            assert!(!glyphs.is_empty());
            let mut raster = SwashCache::new();
            let mut changed = false;
            for glyph in glyphs {
                assert_eq!(glyph.font_id, id, "{tag:?}, {family}");
                assert_eq!(glyph.font_style, style);
                assert!(!glyph.cache_key_flags.contains(CacheKeyFlags::FAKE_ITALIC));
                let key = glyph.physical((0., 0.), 1.).cache_key;
                assert_eq!(key.font_style, Some(style));
                let mut regular_key = key;
                regular_key.font_style = None;
                assert_eq!(HashSet::from([key, regular_key]).len(), 2);
                assert_eq!(BTreeSet::from([key, regular_key]).len(), 2);
                let a = raster.get_image_uncached(&mut fonts, key).unwrap();
                let b = raster.get_image_uncached(&mut fonts, regular_key).unwrap();
                changed |= a.data != b.data;
                assert_eq!(raster.get_image(&mut fonts, key).as_ref().unwrap().data, a.data);
                assert_eq!(raster.get_image(&mut fonts, regular_key).as_ref().unwrap().data, b.data);
            }
            assert!(changed, "native axis location must reach rasterization: {tag:?}, {family}");
        }
    }
    // Static fonts keep the existing Arc and use the ordinary synthetic path.
    let mut db = fontdb::Database::new();
    db.load_font_data(include_bytes!("../fonts/Inter-Regular.ttf").to_vec());
    let id = db.faces().next().unwrap().id;
    let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
    let regular = fonts.get_font(id, Weight::NORMAL).unwrap();
    let italic = fonts.get_font_with_style(id, Weight::NORMAL, Style::Italic).unwrap();
    assert!(Arc::ptr_eq(&regular, &italic));
    assert!(!italic.is_italic_or_oblique());
}


#[test]
fn authored_angles_preserve_native_endpoints_and_synthetic_glyphs() {
    use cosmic_text::{AttrsOwned, CacheKeyFlags, ObliqueAngle, Style, SwashCache};
    use std::sync::Arc;
    let angle = |degrees| Some(ObliqueAngle::new(degrees).unwrap());
    let mut db = fontdb::Database::new();
    db.load_font_data(style_axis_fixture(b"slnt"));
    db.set_sans_serif_family("Inter Variable");
    let id = db.faces().next().unwrap().id;
    let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
    let native_10 = fonts.get_font_with_variations(id, Weight::NORMAL, Style::Oblique, angle(10.)).unwrap();
    let native_20 = fonts.get_font_with_variations(id, Weight::NORMAL, Style::Oblique, angle(20.)).unwrap();
    let native_30 = fonts.get_font_with_variations(id, Weight::NORMAL, Style::Oblique, angle(30.)).unwrap();
    assert_ne!(native_10.shaper().coords(), native_20.shaper().coords());
    assert!(Arc::ptr_eq(&native_20, &native_30), "clamped endpoint instances must share their shaper");
    let native_90 = fonts.get_font_with_variations(id, Weight::NORMAL, Style::Oblique, angle(90.)).unwrap();
    assert!(Arc::ptr_eq(&native_20, &native_90), "90 degrees must reach the native axis endpoint");
    let regular = fonts.get_font(id, Weight::NORMAL).unwrap();
    let negative = fonts.get_font_with_variations(id, Weight::NORMAL, Style::Oblique, angle(-10.)).unwrap();
    assert!(Arc::ptr_eq(&regular, &negative), "unavailable reverse slant reuses the upright instance for synthesis");
    let negative_90 = fonts.get_font_with_variations(id, Weight::NORMAL, Style::Oblique, angle(-90.)).unwrap();
    assert!(Arc::ptr_eq(&regular, &negative_90));

    // Test both a variable native face and a static face. Same-face synthetic
    // angles must not alias masks; native angles must not acquire fake skew.
    for variable in [true, false] {
        let mut db = fontdb::Database::new();
        let family = if variable {
            db.load_font_data(style_axis_fixture(b"slnt"));
            "Inter Variable"
        } else {
            db.load_font_data(include_bytes!("../fonts/Inter-Regular.ttf").to_vec());
            "Inter"
        };
        db.set_sans_serif_family(family);
        let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
        let mut raster = SwashCache::new();
        let mut masks = Vec::new();
        for degrees in [10., 20., -10., 0., 90., -90.] {
            let attrs = Attrs::new().family(Family::Name(family)).style(Style::Oblique).oblique_angle(angle(degrees));
            let owned = AttrsOwned::new(&attrs);
            assert_eq!(owned.as_attrs().oblique_angle, attrs.oblique_angle);
            assert!(!attrs.compatible(&attrs.clone().oblique_angle(angle(if degrees == 0. { 1. } else { 0. }))));
            let mut buffer = Buffer::new(&mut fonts, Metrics::new(40., 48.));
            {
                let mut editor = buffer.borrow_with(&mut fonts);
                editor.set_size(Some(300.), Some(80.));
                editor.set_text("Minimum", &attrs, Shaping::Advanced, None);
                editor.shape_until_scroll(true);
            }
            let glyph = buffer.layout_runs().next().unwrap().glyphs.first().unwrap().clone();
            assert_eq!(glyph.oblique_angle, angle(degrees));
            let synthetic = degrees != 0. && (!variable || degrees < 0.);
            assert_eq!(glyph.cache_key_flags.contains(CacheKeyFlags::FAKE_ITALIC), synthetic);
            let key = glyph.physical((0., 0.), 1.).cache_key;
            assert_eq!(key.oblique_angle, angle(degrees));
            let image = raster.get_image(&mut fonts, key).as_ref();
            if synthetic && degrees.abs() == 90. {
                assert!(image.is_none(), "singular synthesis must not allocate a raster");
                assert!(raster.get_outline_commands(&mut fonts, key).is_none());
                continue;
            }
            let image = image.unwrap();
            masks.push((image.data.clone(), image.placement.left, image.placement.top));
        }
        assert_ne!(masks[0], masks[1], "10 and 20 degree images must differ, native={variable}");
        assert_ne!(masks[0], masks[2], "reverse slant must differ, native={variable}");
        assert_ne!(masks[1], masks[3], "zero must remain upright, native={variable}");
        if variable { assert_eq!(masks[1], masks[4], "90 degrees must reuse the native endpoint raster"); }
    }
    assert_eq!(ObliqueAngle::new(-0.), ObliqueAngle::new(0.));
    for invalid in [f32::NAN, f32::INFINITY, -90.01, 90.01] {
        assert!(ObliqueAngle::new(invalid).is_none());
    }
}
