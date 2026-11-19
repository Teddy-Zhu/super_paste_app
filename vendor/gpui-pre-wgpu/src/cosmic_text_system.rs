use anyhow::{Context as _, Ok, Result};
use collections::HashMap;
use cosmic_text::{
    Attrs, AttrsList, CacheKeyFlags, Ellipsize, Family, Font as CosmicTextFont,
    FontFeatures as CosmicFontFeatures, FontSystem, ShapeBuffer, ShapeLine, Stretch, Style, Weight,
};
use cosmic_text::skrifa::{
    FontRef, MetadataProvider as _, instance::LocationRef,
    raw::{
        ReadError as FontReadError, TableProvider as _,
        tables::{
            gdef::{CaretValue, DeviceOrVariationIndex},
            variations::{DeltaSetIndex, FloatItemDeltaTarget as _},
        },
        types::{FWord, GlyphId as FontGlyphId},
    },
};
use gpui::{
    Bounds, DevicePixels, FallbackFontClass, Font, FontFallbacks, FontFeatures, FontId,
    FontMetrics, FontRun, GlyphId, IsZero as _, LineLayout, MissingGlyph, MissingGlyphSink, Pixels,
    PlatformTextSystem, RenderGlyphParams, SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, ShapedGlyph,
    ShapedRun, SharedString, Size, TextDirection, TextRenderingMode, point, px, size,
};

use itertools::Itertools;
use parking_lot::RwLock;
use smallvec::SmallVec;
use std::{borrow::Cow, ops::Range, sync::Arc};
use swash::{
    scale::{Render, ScaleContext, Source, StrikeWith},
    zeno::{Format, Transform, Vector},
};
use unicode_segmentation::UnicodeSegmentation;

pub struct CosmicTextSystem(RwLock<CosmicTextSystemState>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FontKey {
    family: SharedString,
    features: FontFeatures,
    fallbacks: Option<FontFallbacks>,
}

impl FontKey {
    fn new(family: SharedString, features: FontFeatures, fallbacks: Option<FontFallbacks>) -> Self {
        Self {
            family,
            features,
            fallbacks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LoadedFontKey {
    database_id: cosmic_text::fontdb::ID,
    // Output glyphs have already consumed the source font's features.
    font: Option<FontKey>,
    weight: Weight,
    style: Style,
    oblique_angle: Option<cosmic_text::ObliqueAngle>,
    synthetic_italic: bool,
}

struct CosmicTextSystemState {
    font_system: FontSystem,
    scratch: ShapeBuffer,
    swash_scale_context: ScaleContext,
    pending_glyph_images: HashMap<RenderGlyphParams, swash::scale::image::Image>,
    /// Contains all already loaded fonts, including all faces. Indexed by `FontId`.
    loaded_fonts: Vec<LoadedFont>,
    loaded_font_ids_by_key: HashMap<LoadedFontKey, FontId>,
    /// Caches the `FontId`s associated with a specific family to avoid iterating the font database
    /// for every font face in a family.
    font_ids_by_family_cache: HashMap<FontKey, SmallVec<[FontId; 4]>>,
    system_font_fallback: String,
    missing_glyph_sink: Option<Arc<dyn MissingGlyphSink>>,
}

struct LoadedFont {
    font: Arc<CosmicTextFont>,
    weight: Weight,
    style: Style,
    oblique_angle: Option<cosmic_text::ObliqueAngle>,
    synthetic_skew: f32,
    weight_range: Option<(f32, f32)>,
    /// Swash and HarfRust use the same normalized location. Convert once per
    /// retained font instance, not once per glyph or metrics query.
    normalized_coords: SmallVec<[swash::NormalizedCoord; 8]>,
    features: CosmicFontFeatures,
    is_known_emoji_font: bool,
    /// resolved at load time so `layout_line` shares one chain across faces.
    /// `Arc` keeps clone cheap on the per-run hot path.
    user_fallback_chain: Arc<[(FontId, SharedString)]>,
}

impl LoadedFont {
    fn new(font: Arc<CosmicTextFont>, key: &LoadedFontKey, face_index: u32,
        features: CosmicFontFeatures, is_known_emoji_font: bool,
        user_fallback_chain: Arc<[(FontId, SharedString)]>) -> Result<Self> {
        let reference = FontRef::from_index(font.data(), face_index)?;
        let weight_range = reference.axes().get_by_tag(cosmic_text::skrifa::Tag::new(b"wght"))
            .map(|axis| (axis.min_value(), axis.max_value()));
        if let Some((minimum, maximum)) = weight_range {
            anyhow::ensure!(minimum.is_finite() && maximum.is_finite() && minimum <= maximum,
                "invalid variable font weight range");
        }
        let normalized_coords = font.shaper().coords().iter().map(|coordinate| coordinate.to_bits()).collect();
        let synthetic_skew = if key.synthetic_italic {
            key.oblique_angle.unwrap_or_else(|| cosmic_text::ObliqueAngle::new(14.).expect("default oblique angle")).shear()
        } else { 0. };
        Ok(Self { font, weight: key.weight, style: key.style, oblique_angle: key.oblique_angle, synthetic_skew, weight_range, normalized_coords, features,
            is_known_emoji_font, user_fallback_chain })
    }

    fn variable_weight(&self, requested: f32) -> Option<Weight> {
        let (minimum, maximum) = self.weight_range?;
        Some(Weight(requested.clamp(minimum, maximum).round().clamp(0., u16::MAX as f32) as u16))
    }

    fn synthetic_transform(&self) -> Option<Transform> {
        (self.synthetic_skew != 0.).then_some(Transform { yx: self.synthetic_skew, ..Transform::IDENTITY })
    }
}

struct FontMatchProperties {
    primary_family_name: SharedString,
    stretch: Stretch,
    style: Style,
    oblique_angle: Option<cosmic_text::ObliqueAngle>,
    weight: Weight,
    features: CosmicFontFeatures,
    fallback_chain: Arc<[(FontId, SharedString)]>,
}

impl FontMatchProperties {
    fn attributes<'a>(&'a self, font_id: FontId, family_name: &'a str, size: Option<Pixels>) -> Attrs<'a> {
        let attrs = Attrs::new()
            .metadata(font_id.0)
            .family(Family::Name(family_name))
            .stretch(self.stretch)
            .style(self.style)
            .oblique_angle(self.oblique_angle)
            .weight(self.weight)
            .font_features(self.features.clone());
        match size {
            Some(size) => attrs.metrics(cosmic_text::Metrics::new(size.as_f32(), size.as_f32())),
            None => attrs,
        }
    }
}

impl CosmicTextSystem {
    /// Returns the selected face's weight and style, which may differ from the request.
    pub fn font_weight_and_style(
        &self,
        font_id: FontId,
    ) -> Result<(gpui::FontWeight, gpui::FontStyle)> {
        let state = self.0.read();
        let font = state
            .loaded_fonts
            .get(font_id.0)
            .context("invalid font ID")?;
        let style = match font.font.effective_style() {
            cosmic_text::Style::Normal => gpui::FontStyle::Normal,
            cosmic_text::Style::Italic => gpui::FontStyle::Italic,
            cosmic_text::Style::Oblique => gpui::FontStyle::Oblique,
        };
        let weight = font.weight;
        Ok((gpui::FontWeight(weight.0 as f32), style))
    }

    /// Builds reports for unresolved source indices after an outer text system
    /// has applied its own fallback.
    pub fn missing_glyphs(
        &self,
        text: &str,
        font_runs: &[FontRun],
        missing_text_indices: impl IntoIterator<Item = usize>,
    ) -> Vec<MissingGlyph> {
        self.0
            .read()
            .missing_glyphs(text, font_runs, missing_text_indices)
    }

    pub fn new(system_font_fallback: &str) -> Self {
        #[allow(unused_mut)]
        let mut font_system = FontSystem::new();
        #[cfg(all(feature = "font-kit", any(target_os = "linux", target_os = "freebsd")))]
        configure_generic_families(&mut font_system);

        Self(RwLock::new(CosmicTextSystemState {
            font_system,
            scratch: ShapeBuffer::default(),
            swash_scale_context: ScaleContext::new(),
            pending_glyph_images: HashMap::default(),
            loaded_fonts: Vec::new(),
            loaded_font_ids_by_key: HashMap::default(),
            font_ids_by_family_cache: HashMap::default(),
            system_font_fallback: system_font_fallback.to_string(),
            missing_glyph_sink: None,
        }))
    }

    pub fn new_without_system_fonts(system_font_fallback: &str) -> Self {
        let font_system = FontSystem::new_with_locale_and_db(
            "en-US".to_string(),
            cosmic_text::fontdb::Database::new(),
        );

        Self(RwLock::new(CosmicTextSystemState {
            font_system,
            scratch: ShapeBuffer::default(),
            swash_scale_context: ScaleContext::new(),
            pending_glyph_images: HashMap::default(),
            loaded_fonts: Vec::new(),
            loaded_font_ids_by_key: HashMap::default(),
            font_ids_by_family_cache: HashMap::default(),
            system_font_fallback: system_font_fallback.to_string(),
            missing_glyph_sink: None,
        }))
    }
}

/// Cosmic-text supplies hard-coded generic families after loading the system
/// database. Replace those names once at startup using fontconfig's selection,
/// retaining the already parsed faces and the existing font/layout caches.
#[cfg(all(feature = "font-kit", any(target_os = "linux", target_os = "freebsd")))]
fn configure_generic_families(font_system: &mut FontSystem) {
    use cosmic_text::fontdb::Source;
    use font_kit::{handle::Handle, source::SystemSource};

    let source = SystemSource::new();
    for (generic, requested) in [
        (Family::Serif, "serif"),
        (Family::SansSerif, "sans-serif"),
        (Family::Monospace, "monospace"),
    ] {
        // Only ask for the family handles. select_best_match would load each
        // face again to read properties which our database already contains.
        let selected = match source.select_family_by_name(requested) {
            std::result::Result::Ok(selected) => selected,
            Err(error) => {
                log::warn!("Could not resolve system {generic:?} font: {error}");
                continue;
            }
        };
        let family = selected.fonts().iter().find_map(|handle| match handle {
            Handle::Path { path, font_index } => font_system.db().faces().find(|face| {
                face.index == *font_index && match &face.source {
                    Source::File(file) | Source::SharedFile(file, _) => file == path,
                    _ => false,
                }
            }).and_then(|face| face.families.first()).map(|family| family.0.clone()),
            _ => None,
        });
        let Some(family) = family else {
            log::warn!("System {generic:?} font is absent from the loaded font database");
            continue;
        };
        match generic {
            Family::Serif => font_system.db_mut().set_serif_family(family),
            Family::SansSerif => font_system.db_mut().set_sans_serif_family(family),
            Family::Monospace => font_system.db_mut().set_monospace_family(family),
            _ => unreachable!(),
        }
    }
}

impl PlatformTextSystem for CosmicTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.0.write().add_fonts(fonts)
    }

    fn set_missing_glyph_sink(&self, sink: Option<Arc<dyn MissingGlyphSink>>) {
        self.0.write().missing_glyph_sink = sink;
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut result = self
            .0
            .read()
            .font_system
            .db()
            .faces()
            .filter_map(|face| face.families.first().map(|family| family.0.clone()))
            .collect_vec();
        result.sort_unstable();
        result.dedup();
        result
    }

    fn font_id(&self, font: &Font) -> Result<FontId> {
        anyhow::ensure!(font.weight.0.is_finite(), "non-finite font weight");
        let mut state = self.0.write();
        let key = FontKey::new(
            font.family.clone(),
            font.features.clone(),
            font.fallbacks.clone(),
        );
        let candidates = if let Some(font_ids) = state.font_ids_by_family_cache.get(&key) {
            font_ids.as_slice()
        } else {
            let font_ids =
                state.load_family(&font.family, &font.features, font.fallbacks.as_ref())?;
            state.font_ids_by_family_cache.insert(key.clone(), font_ids);
            state.font_ids_by_family_cache[&key].as_ref()
        };

        let ix = find_best_match(font, candidates, &state)?;

        let selected = candidates[ix];
        let loaded = state.loaded_font(selected);
        let weight = loaded.variable_weight(font.weight.0).unwrap_or(loaded.weight);
        let (style, oblique_angle) = cosmic_style(font.style);
        if weight == loaded.weight && style == loaded.style && oblique_angle == loaded.oblique_angle { return Ok(selected); }
        let face = state.font_system.db().face(loaded.font.id()).context("selected font face is missing")?;
        let index = face.index;
        let database_id = loaded.font.id();
        let fallback = loaded.user_fallback_chain.clone();
        let features = loaded.features.clone();
        let emoji = loaded.is_known_emoji_font;
        let instance = state.font_system.get_font_with_variations(database_id, weight, style, oblique_angle)
            .context("could not load styled font instance")?;
        let synthetic_italic = style != Style::Normal && !instance.is_italic_or_oblique();
        let key = LoadedFontKey { database_id, font: Some(key), weight, style, oblique_angle, synthetic_italic };
        if let Some(&id) = state.loaded_font_ids_by_key.get(&key) {
            state.loaded_fonts[id.0].user_fallback_chain = fallback;
            return Ok(id);
        }
        let loaded = LoadedFont::new(instance, &key, index, features, emoji, fallback)?;
        let id = FontId(state.loaded_fonts.len());
        state.loaded_fonts.push(loaded);
        state.loaded_font_ids_by_key.insert(key, id);
        Ok(id)
    }

    fn prewarm_fonts(&self, font_ids: &[FontId]) {
        self.0.write().prewarm_fonts(font_ids);
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        let state = self.0.read();
        let loaded = state.loaded_font(font_id);
        let metrics = loaded.font.as_swash().metrics(&loaded.normalized_coords);

        FontMetrics {
            units_per_em: metrics.units_per_em as u32,
            ascent: metrics.ascent,
            descent: -metrics.descent,
            line_gap: metrics.leading,
            underline_position: metrics.underline_offset,
            underline_thickness: metrics.stroke_size,
            cap_height: metrics.cap_height,
            x_height: metrics.x_height,
            bounding_box: Bounds {
                origin: point(0.0, 0.0),
                size: size(metrics.max_width, metrics.ascent + metrics.descent),
            },
        }
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let lock = self.0.read();
        let loaded = lock.loaded_font(font_id);
        let glyph_metrics = loaded.font.as_swash().glyph_metrics(&loaded.normalized_coords);
        let glyph_id = glyph_id.0 as u16;
        Ok(Bounds {
            origin: point(0.0, 0.0),
            size: size(
                glyph_metrics.advance_width(glyph_id),
                glyph_metrics.advance_height(glyph_id),
            ),
        })
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.0.read().advance(font_id, glyph_id)
    }

    fn intersect_glyph_ink(
        &self, font_id: FontId, glyph_id: GlyphId, font_size: Pixels, ink: &mut gpui::GlyphInk,
    ) -> Result<()> {
        use cosmic_text::skrifa::{
            instance::Size as FontSize,
            outline::{DrawSettings, OutlinePen},
        };
        struct Pen<'a>(&'a mut gpui::GlyphInk, f32);
        impl OutlinePen for Pen<'_> {
            fn move_to(&mut self, x: f32, y: f32) { self.0.move_to(point(x + self.1 * y, -y)); }
            fn line_to(&mut self, x: f32, y: f32) { self.0.line_to(point(x + self.1 * y, -y)); }
            fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
                self.0.quad_to(point(cx + self.1 * cy, -cy), point(x + self.1 * y, -y));
            }
            fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
                self.0.curve_to(point(ax + self.1 * ay, -ay), point(bx + self.1 * by, -by), point(x + self.1 * y, -y));
            }
            fn close(&mut self) { self.0.close(); }
        }
        anyhow::ensure!(font_size.as_f32().is_finite() && font_size > Pixels::ZERO,
            "invalid glyph outline size");
        let state = self.0.read();
        let loaded = state.loaded_font(font_id);
        let face = state.font_system.db().face(loaded.font.id())
            .context("glyph outline font is missing from the font database")?;
        // Borrow the exact loaded face and its normalized variation location.
        // Drawing into the strip accumulator never builds a second path array.
        let font = FontRef::from_index(loaded.font.data(), face.index)?;
        let outlines = font.outline_glyphs();
        if let Some(outline) = outlines.get(FontGlyphId::new(glyph_id.0)) {
            outline.draw(DrawSettings::unhinted(
                FontSize::new(font_size.as_f32()),
                LocationRef::new(loaded.font.shaper().coords()),
            ), &mut Pen(ink, loaded.synthetic_skew))?;
        }
        Ok(())
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.0.read().glyph_for_char(font_id, ch)
    }

    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        self.0.write().raster_bounds(params)
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        self.0.write().rasterize_glyph(params, raster_bounds)
    }

    fn layout_line_with_direction(
        &self,
        text: &str,
        font_size: Pixels,
        runs: &[FontRun],
        direction: TextDirection,
    ) -> LineLayout {
        self.0.write().layout_line(text, font_size, runs, direction)
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        TextRenderingMode::Subpixel
    }
}

impl CosmicTextSystemState {
    fn loaded_font(&self, font_id: FontId) -> &LoadedFont {
        &self.loaded_fonts[font_id.0]
    }

    fn font_match_properties(&self, font_id: FontId) -> Option<FontMatchProperties> {
        let loaded_font = self.loaded_font(font_id);
        let Some(face) = self.font_system.db().face(loaded_font.font.id()) else {
            log::warn!("font face not found in database for font_id {:?}", font_id);
            return None;
        };
        let Some(first_family) = face.families.first() else {
            log::warn!("font face has no family names for font_id {:?}", font_id);
            return None;
        };

        Some(FontMatchProperties {
            primary_family_name: first_family.0.clone().into(),
            stretch: face.stretch,
            style: loaded_font.style,
            oblique_angle: loaded_font.oblique_angle,
            weight: loaded_font.weight,
            features: loaded_font.features.clone(),
            fallback_chain: Arc::clone(&loaded_font.user_fallback_chain),
        })
    }

    fn prewarm_fonts(&mut self, font_ids: &[FontId]) {
        for &font_id in font_ids {
            let Some(properties) = self.font_match_properties(font_id) else {
                continue;
            };
            let primary_attributes =
                properties.attributes(font_id, &properties.primary_family_name, None);
            self.font_system.get_font_matches(&primary_attributes);

            for (fallback_id, fallback_name) in &*properties.fallback_chain {
                let fallback_attributes = properties.attributes(*fallback_id, fallback_name, None);
                self.font_system.get_font_matches(&fallback_attributes);
            }
        }
    }

    #[profiling::function]
    fn add_fonts(&mut self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.font_ids_by_family_cache.clear();
        let db = self.font_system.db_mut();
        for bytes in fonts {
            db.load_font_source(cosmic_text::fontdb::Source::Binary(Arc::new(bytes)));
        }
        Ok(())
    }

    #[profiling::function]
    fn load_family(
        &mut self,
        name: &str,
        features: &FontFeatures,
        fallbacks: Option<&FontFallbacks>,
    ) -> Result<SmallVec<[FontId; 4]>> {
        let loaded_font_key = FontKey::new(
            SharedString::from(name.to_owned()),
            features.clone(),
            fallbacks.cloned(),
        );

        // recurse with `fallbacks = None` so a fallback family cannot pull in
        // another chain. missing fallback families are dropped so a typo in
        // settings still lets the primary family load.
        let user_fallback_chain: Arc<[(FontId, SharedString)]> = match fallbacks {
            Some(fallbacks) if !fallbacks.fallback_list().is_empty() => {
                let mut chain: Vec<(FontId, SharedString)> = Vec::new();
                for fallback_name in fallbacks.fallback_list() {
                    let fb_key = FontKey::new(
                        SharedString::from(fallback_name.clone()),
                        features.clone(),
                        None,
                    );
                    let fb_ids = if let Some(cached) = self.font_ids_by_family_cache.get(&fb_key) {
                        cached.clone()
                    } else {
                        let loaded = self.load_family(fallback_name, features, None)?;
                        self.font_ids_by_family_cache
                            .insert(fb_key.clone(), loaded.clone());
                        loaded
                    };
                    let Some(&fb_id) = fb_ids.first() else {
                        continue;
                    };
                    let db_id = self.loaded_fonts[fb_id.0].font.id();
                    if let Some(face) = self.font_system.db().face(db_id)
                        && let Some(family) = face.families.first()
                    {
                        chain.push((fb_id, SharedString::from(family.0.clone())));
                    }
                }
                Arc::from(chain)
            }
            _ => Arc::from(Vec::new()),
        };

        let name = gpui::font_name_with_fallbacks(name, &self.system_font_fallback);

        // Resolve CSS generic families through the font database's system
        // configuration. Treating them as literal names silently falls back
        // to the UI font and gives root/monospace metrics the wrong basis.
        let generic = match name {
            "serif" => Some(Family::Serif),
            "sans-serif" => Some(Family::SansSerif),
            "monospace" => Some(Family::Monospace),
            _ => None,
        };
        let generic_name = generic.map(|family| self.font_system.db().family_name(&family).to_owned());
        let name = generic_name.as_deref().unwrap_or(name);

        let families = self
            .font_system
            .db()
            .faces()
            .filter(|face| face.families.iter().any(|family| *name == family.0))
            .map(|face| (face.id, face.post_script_name.clone(), face.weight, face.style, face.index))
            .collect::<SmallVec<[_; 4]>>();

        let cosmic_features = cosmic_font_features(features)?;

        let mut loaded_font_ids = SmallVec::new();
        for (database_id, postscript_name, weight, style, face_index) in families {
            let key = LoadedFontKey {
                database_id,
                font: Some(loaded_font_key.clone()),
                weight,
                style,
                oblique_angle: None,
                synthetic_italic: false,
            };
            if let Some(&font_id) = self.loaded_font_ids_by_key.get(&key) {
                self.loaded_fonts[font_id.0].user_fallback_chain = Arc::clone(&user_fallback_chain);
                loaded_font_ids.push(font_id);
                continue;
            }

            let font = self
                .font_system
                .get_font(database_id, weight)
                .context("Could not load font")?;

            // HACK: To let the storybook run and render Windows caption icons. We should actually do better font fallback.
            let allowed_bad_font_names = [
                "SegoeFluentIcons", // NOTE: Segoe fluent icons postscript name is inconsistent
                "Segoe Fluent Icons",
            ];

            // This only decides whether a face can be a primary font. The
            // editor measures layout with the width of 'm' and would panic
            // without it.
            //
            // Don't remove the face from the font database. Arabic, Devanagari
            // and emoji fonts often have no Latin glyphs, and they are exactly
            // what cosmic-text falls back to when the primary font lacks a
            // glyph.
            if font.as_swash().charmap().map('m') == 0
                && !allowed_bad_font_names.contains(&postscript_name.as_str())
            {
                continue;
            };

            let font_id = FontId(self.loaded_fonts.len());
            loaded_font_ids.push(font_id);
            self.loaded_fonts.push(LoadedFont::new(font, &key, face_index,
                cosmic_features.clone(), check_is_known_emoji_font(&postscript_name),
                Arc::clone(&user_fallback_chain))?);
            self.loaded_font_ids_by_key.insert(key, font_id);
        }

        Ok(loaded_font_ids)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        let loaded = self.loaded_font(font_id);
        let glyph_metrics = loaded.font.as_swash().glyph_metrics(&loaded.normalized_coords);
        Ok(Size {
            width: glyph_metrics.advance_width(glyph_id.0 as u16),
            height: glyph_metrics.advance_height(glyph_id.0 as u16),
        })
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        let glyph_id = self.loaded_font(font_id).font.as_swash().charmap().map(ch);
        if glyph_id == 0 {
            None
        } else {
            Some(GlyphId(glyph_id.into()))
        }
    }

    fn raster_bounds(&mut self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let image = self.render_glyph_image(params)?;
        let bounds = Bounds {
            origin: point(image.placement.left.into(), (-image.placement.top).into()),
            size: size(image.placement.width.into(), image.placement.height.into()),
        };
        if !bounds.is_zero() {
            self.pending_glyph_images.insert(params.clone(), image);
        }
        Ok(bounds)
    }

    #[profiling::function]
    fn rasterize_glyph(
        &mut self,
        params: &RenderGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        }

        let mut image = match self.pending_glyph_images.remove(params) {
            Some(image) => image,
            None => self.render_glyph_image(params)?,
        };
        let bitmap_size = glyph_bounds.size;
        match image.content {
            swash::scale::image::Content::Color | swash::scale::image::Content::SubpixelMask => {
                // Convert from RGBA to BGRA.
                for pixel in image.data.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                Ok((bitmap_size, image.data))
            }
            swash::scale::image::Content::Mask => {
                if params.subpixel_rendering {
                    // We must always return RGBA data when subpixel rendering is requested.
                    let expanded = image.data.iter().flat_map(|&a| [a, a, a, a]).collect();
                    Ok((bitmap_size, expanded))
                } else {
                    Ok((bitmap_size, image.data))
                }
            }
        }
    }

    fn render_glyph_image(
        &mut self,
        params: &RenderGlyphParams,
    ) -> Result<swash::scale::image::Image> {
        let loaded_font = &self.loaded_fonts[params.font_id.0];
        anyhow::ensure!(!(loaded_font.synthetic_skew != 0.
            && loaded_font.oblique_angle.is_some_and(|angle| angle.degrees().abs() == 90.)),
            "authored oblique glyph exceeds raster limits: singular synthetic angle");
        let font_ref = loaded_font.font.as_swash();
        let pixel_size = f32::from(params.font_size);

        let subpixel_offset = Vector::new(
            params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / params.scale_factor,
            params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / params.scale_factor,
        );

        let mut scaler = self
            .swash_scale_context
            .builder(font_ref)
            .size(pixel_size * params.scale_factor)
            .hint(true)
            .normalized_coords(loaded_font.normalized_coords.iter().copied())
            .build();

        let sources: &[Source] = if params.is_emoji {
            &[
                Source::ColorOutline(0),
                Source::ColorBitmap(StrikeWith::BestFit),
                Source::Outline,
            ]
        } else if loaded_font.synthetic_skew != 0. {
            // Embedded bitmap strikes do not accept Swash's outline transform.
            &[Source::Outline, Source::Bitmap(StrikeWith::ExactSize)]
        } else {
            &[Source::Bitmap(StrikeWith::ExactSize), Source::Outline]
        };

        let mut renderer = Render::new(sources);
        renderer.transform(loaded_font.synthetic_transform());
        if params.subpixel_rendering {
            // There seems to be a bug in Swash where the B and R values are swapped.
            renderer
                .format(Format::subpixel_bgra())
                .offset(subpixel_offset);
        } else {
            renderer.format(Format::Alpha).offset(subpixel_offset);
        }

        let glyph_id: u16 = params.glyph_id.0.try_into()?;
        if loaded_font.oblique_angle.is_some() && loaded_font.synthetic_skew.abs() > 1. {
            // Large authored shears can otherwise allocate a huge mask before
            // the atlas rejects it. Inspect actual transformed outline bounds;
            // ordinary styles and angles below 45 degrees avoid this work.
            let outline = if params.is_emoji {
                scaler.scale_color_outline(glyph_id).or_else(|| {
                    (!scaler.has_color_bitmaps()).then(|| scaler.scale_outline(glyph_id)).flatten()
                })
            } else { scaler.scale_outline(glyph_id) };
            if let Some(mut outline) = outline {
                outline.transform(&loaded_font.synthetic_transform().expect("nonzero synthetic shear"));
                let bounds = outline.bounds();
                let width = f64::from(bounds.max.x - bounds.min.x).ceil() + 4.;
                let height = f64::from(bounds.max.y - bounds.min.y).ceil() + 4.;
                anyhow::ensure!(width.is_finite() && height.is_finite()
                    && width <= 8192. && height <= 8192. && width * height * 4. <= 16. * 1024. * 1024.,
                    "authored oblique glyph exceeds raster limits");
            }
        }
        renderer
            .render(&mut scaler, glyph_id)
            .with_context(|| format!("unable to render glyph via swash for {params:?}"))
    }

    /// This is used when cosmic_text has chosen a fallback font instead of using the requested
    /// font, typically to handle some unicode characters. When this happens, `loaded_fonts` may not
    /// yet have an entry for this fallback font, and so one is added.
    ///
    /// Features have already been applied to these output glyph IDs. A
    /// featureless output identity shares the existing instance map and keeps
    /// the shaper's actual weight and synthetic style, without scanning every
    /// loaded face per glyph.
    fn font_id_for_cosmic_id(&mut self, id: cosmic_text::fontdb::ID, weight: Weight, style: Style, oblique_angle: Option<cosmic_text::ObliqueAngle>, synthetic_italic: bool) -> Result<FontId> {
        let face = self.font_system.db().face(id)
            .context("fallback font face not found in cosmic-text database")?;
        let key = LoadedFontKey { database_id: id, font: None, weight, style, oblique_angle, synthetic_italic };
        if let Some(&font_id) = self.loaded_font_ids_by_key.get(&key) { return Ok(font_id); }
        let index = face.index;
        let emoji = check_is_known_emoji_font(&face.post_script_name);
        let font = self.font_system.get_font_with_variations(id, weight, style, oblique_angle)
            .context("failed to get fallback font from cosmic-text font system")?;
        let loaded = LoadedFont::new(font, &key, index, CosmicFontFeatures::new(), emoji, Arc::from(Vec::new()))?;
        let font_id = FontId(self.loaded_fonts.len());
        self.loaded_fonts.push(loaded);
        self.loaded_font_ids_by_key.insert(key, font_id);
        Ok(font_id)
    }

    #[profiling::function]
    fn layout_line(
        &mut self,
        text: &str,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
    ) -> LineLayout {
        if contains_paragraph_separator(text) {
            self.layout_line_with_separators(text, font_size, font_runs, direction)
        } else {
            self.layout_line_no_separators(text, font_size, font_runs, direction)
        }
    }

    fn layout_line_with_separators(
        &mut self,
        text: &str,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
    ) -> LineLayout {
        let mut layout = LineLayout {
            font_size,
            len: text.len(),
            ..Default::default()
        };
        let mut paragraph_start = 0;

        for (separator_start, separator) in text
            .char_indices()
            .filter(|(_, character)| is_paragraph_separator(*character))
        {
            let separator_end = separator_start + separator.len_utf8();
            self.shape_segment(
                text,
                paragraph_start..separator_start,
                font_size,
                font_runs,
                direction,
                &mut layout,
            );
            self.shape_segment(
                text,
                separator_start..separator_end,
                font_size,
                font_runs,
                direction,
                &mut layout,
            );
            paragraph_start = separator_end;
        }

        self.shape_segment(
            text,
            paragraph_start..text.len(),
            font_size,
            font_runs,
            direction,
            &mut layout,
        );

        layout
    }

    fn shape_segment(
        &mut self,
        text: &str,
        range: Range<usize>,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
        layout: &mut LineLayout,
    ) {
        if range.is_empty() {
            return;
        }

        let segment_font_runs = clip_font_runs(font_runs, range.clone());
        let segment =
            self.layout_line_no_separators(&text[range.clone()], font_size, &segment_font_runs, direction);

        layout.interior_carets.extend(segment.interior_carets.into_iter().map(|mut caret| {
            caret.index += range.start;
            caret.cluster_start += range.start;
            caret.x += layout.width;
            caret
        }));
        let mut segment_runs = segment.runs;
        for run in &mut segment_runs {
            for glyph in &mut run.glyphs {
                glyph.index += range.start;
                glyph.position.x += layout.width;
            }
        }

        for mut run in segment_runs {
            if let Some(same_run) = layout
                .runs
                .last_mut()
                .filter(|last| last.font_id == run.font_id && last.font_size == run.font_size)
            {
                same_run.glyphs.append(&mut run.glyphs);
            } else {
                layout.runs.push(run);
            }
        }

        layout.width += segment.width;
        layout.ascent = layout.ascent.max(segment.ascent);
        layout.descent = layout.descent.max(segment.descent);
    }

    fn layout_line_no_separators(
        &mut self,
        text: &str,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
    ) -> LineLayout {
        let mut attrs_list = AttrsList::new(&Attrs::new());
        let mut offs = 0;
        for (run_index, run) in font_runs.iter().enumerate() {
            let run_end = offs + run.len;

            let Some(properties) = self.font_match_properties(run.font_id) else {
                offs = run_end;
                continue;
            };

            // Preserve native font-run cuts without perturbing size or
            // overloading glyph metadata. Cosmic borrows adjacent source
            // context while shaping these explicitly bounded native spans.
            let shaping_span = run_index + 1;
            let primary_attrs = properties.attributes(run.font_id, &properties.primary_family_name, run.font_size)
                .shaping_span(shaping_span);
            let fallback_attrs: SmallVec<[Attrs<'_>; 4]> = properties
                .fallback_chain
                .iter()
                .map(|(font_id, family_name)| properties.attributes(*font_id, family_name, run.font_size)
                    .shaping_span(shaping_span))
                .collect();

            let spans = if properties.fallback_chain.is_empty() {
                let mut spans = SmallVec::<[RunSpan; 4]>::new();
                spans.push(RunSpan {
                    start: offs,
                    end: run_end,
                    slot: None,
                    font_id: run.font_id,
                });
                spans
            } else {
                let loaded_fonts = &self.loaded_fonts;
                let covers = |id: FontId, ch: char| charmap_covers(loaded_fonts, id, ch);
                compute_run_spans(
                    text,
                    offs,
                    run.len,
                    run.font_id,
                    &properties.fallback_chain,
                    &covers,
                )
            };

            for span in spans {
                let attrs = match span.slot {
                    None => &primary_attrs,
                    Some(ix) => &fallback_attrs[ix],
                };
                attrs_list.add_span(span.start..span.end, attrs);
            }
            offs = run_end;
        }

        let base_level = match direction {
            TextDirection::Auto => None,
            TextDirection::Ltr => Some(unicode_bidi::Level::ltr()),
            TextDirection::Rtl => Some(unicode_bidi::Level::rtl()),
        };
        let line = ShapeLine::new_with_direction(
            &mut self.font_system,
            text,
            &attrs_list,
            cosmic_text::Shaping::Advanced,
            4,
            base_level,
        );
        let mut layout_lines = Vec::with_capacity(1);
        line.layout_to_buffer(
            &mut self.scratch,
            f32::from(font_size),
            None, // We do our own wrapping
            cosmic_text::Wrap::None,
            Ellipsize::None,
            None,
            &mut layout_lines,
            None,
            cosmic_text::Hinting::Disabled,
        );

        let Some(layout) = layout_lines.first_mut() else {
            return LineLayout {
                font_size,
                width: Pixels::ZERO,
                ascent: Pixels::ZERO,
                descent: Pixels::ZERO,
                runs: Vec::new(),
                len: text.len(),
                interior_carets: Vec::new(),
                isolation: None,
                ink_clip: None,
            };
        };

        // GPUI paints and wraps runs in visual order. Cosmic can return an RTL
        // paragraph from right to left; retain stable ordering for tied marks.
        if layout.glyphs.windows(2).any(|pair| pair[0].x > pair[1].x) {
            layout.glyphs.sort_by(|left, right| left.x.total_cmp(&right.x));
        }
        let missing_glyphs = self.missing_glyph_sink.as_ref().map(|_| {
            self.missing_glyphs(
                text,
                font_runs,
                layout
                    .glyphs
                    .iter()
                    .filter(|glyph| glyph.glyph_id == 0)
                    .map(|glyph| glyph.start),
            )
        });

        let mut runs: Vec<ShapedRun> = Vec::new();
        for glyph in &layout.glyphs {
            let mut font_id = FontId(glyph.metadata);
            let mut loaded_font = self.loaded_font(font_id);
            let synthetic_italic = glyph.cache_key_flags.contains(CacheKeyFlags::FAKE_ITALIC);
            if loaded_font.font.id() != glyph.font_id || loaded_font.weight != glyph.font_weight || loaded_font.style != glyph.font_style || loaded_font.oblique_angle != glyph.oblique_angle
                || (loaded_font.synthetic_skew != 0.) != synthetic_italic {
                match self.font_id_for_cosmic_id(glyph.font_id, glyph.font_weight, glyph.font_style, glyph.oblique_angle, synthetic_italic) {
                    std::result::Result::Ok(resolved_id) => {
                        font_id = resolved_id;
                        loaded_font = self.loaded_font(font_id);
                    }
                    Err(error) => {
                        log::warn!(
                            "failed to resolve cosmic font id {:?}: {error:#}",
                            glyph.font_id
                        );
                        continue;
                    }
                }
            }
            let is_emoji = loaded_font.is_known_emoji_font;

            // HACK: Prevent crash caused by variation selectors.
            if glyph.glyph_id == 3 && is_emoji {
                continue;
            }

            let shaped_glyph = ShapedGlyph {
                id: GlyphId(glyph.glyph_id as u32),
                position: point(px(glyph.x + glyph.font_size * glyph.x_offset), px(glyph.y - glyph.font_size * glyph.y_offset)),
                index: glyph.start,
                is_emoji,
                advance: px(glyph.w),
                is_rtl: glyph.level.is_rtl(),
            };

            if let Some(last_run) = runs
                .last_mut()
                .filter(|last_run| last_run.font_id == font_id
                    && last_run.font_size.unwrap_or(font_size) == px(glyph.font_size))
            {
                last_run.glyphs.push(shaped_glyph);
            } else {
                runs.push(ShapedRun { font_size: (px(glyph.font_size) != font_size).then_some(px(glyph.font_size)),
                    font_id,
                    glyphs: vec![shaped_glyph],
                });
            }
        }

        if let Some((sink, missing_glyphs)) = self.missing_glyph_sink.as_ref().zip(missing_glyphs) {
            sink.report(missing_glyphs);
        }

        let mut shaped = LineLayout {
            font_size,
            width: layout.w.into(),
            ascent: layout.max_ascent.into(),
            descent: layout.max_descent.into(),
            runs,
            len: text.len(),
            interior_carets: Vec::new(),
            isolation: None,
            ink_clip: None,
        };
        shaped.interior_carets = shaped.interior_caret_candidates(text);
        self.apply_native_ligature_carets(&mut shaped);
        shaped
    }

    fn apply_native_ligature_carets(&self, layout: &mut LineLayout) {
        if layout.interior_carets.is_empty() { return; }
        let carets = &mut layout.interior_carets;
        for run in &layout.runs {
            let font_size = run.font_size.unwrap_or(layout.font_size).as_f32();
            if !run.glyphs.iter().any(|glyph| glyph.advance > Pixels::ZERO
                && carets.binary_search_by_key(&glyph.index, |caret| caret.cluster_start).is_ok())
            { continue; }
            let loaded = self.loaded_font(run.font_id);
            let result = (|| -> Result<()> {
                let face = self.font_system.db().face(loaded.font.id())
                    .context("loaded ligature font is missing from the font database")?;
                // Borrow the existing face bytes, including the selected TTC
                // index. No second font cache or copy of the font is retained.
                let font = FontRef::from_index(loaded.font.data(), face.index)?;
                let gdef = match font.gdef() {
                    std::result::Result::Ok(table) => table,
                    Err(FontReadError::TableIsMissing(_)) => return Ok(()),
                    Err(error) => return Err(error.into()),
                };
                let Some(ligatures) = gdef.lig_caret_list().transpose()? else { return Ok(()); };
                let coverage = ligatures.coverage()?;
                let variations = gdef.item_var_store().transpose()?;
                let scale = font_size / loaded.font.shaper().units_per_em() as f32;
                anyhow::ensure!(scale.is_finite() && scale > 0.0, "invalid ligature font scale");
                let mut positions = SmallVec::<[Pixels; 4]>::new();
                let mut point_targets = SmallVec::<[(u16, usize); 4]>::new();
                let mut outlines = None;
                for glyph in &run.glyphs {
                    if glyph.advance <= Pixels::ZERO { continue; }
                    let start = carets.partition_point(|caret| caret.cluster_start < glyph.index);
                    let end = start + carets[start..].partition_point(|caret| caret.cluster_start == glyph.index);
                    if start == end { continue; }
                    let Some(index) = coverage.get(FontGlyphId::new(glyph.id.0)) else { continue; };
                    let ligature = ligatures.lig_glyphs().get(index as usize)?;
                    // A glyph may cover components that are not independent
                    // Unicode graphemes. Only a complete mapping replaces the
                    // shaper's proportional fallback; never assign by UTF-16
                    // units or expose a stop inside a combining sequence.
                    if usize::from(ligature.caret_count()) != end - start { continue; }
                    positions.clear();
                    point_targets.clear();
                    for value in ligature.caret_values().iter() {
                        let coordinate = match value? {
                            CaretValue::Format1(value) => f32::from(value.coordinate()),
                            CaretValue::Format2(value) => {
                                point_targets.push((value.caret_value_point_index(), positions.len()));
                                0.0
                            }
                            CaretValue::Format3(value) => match value.device()? {
                                // Layout uses unhinted advances, so integer
                                // raster device corrections do not move carets.
                                DeviceOrVariationIndex::Device(_) => f32::from(value.coordinate()),
                                DeviceOrVariationIndex::VariationIndex(index) => {
                                    let delta = variations.as_ref()
                                        .context("ligature variation index has no item variation store")?
                                        .compute_float_delta(DeltaSetIndex {
                                            outer: index.delta_set_outer_index(),
                                            inner: index.delta_set_inner_index(),
                                        }, loaded.font.shaper().coords())?;
                                    FWord::new(value.coordinate()).apply_float_delta(delta)
                                }
                            },
                        };
                        let x = glyph.position.x + gpui::px(coordinate * scale);
                        anyhow::ensure!(x.as_f32().is_finite(), "non-finite font ligature caret");
                        positions.push(x);
                    }
                    if !point_targets.is_empty() {
                        point_targets.sort_unstable_by_key(|target| target.0);
                        let outline = outlines.get_or_insert_with(|| font.outline_glyphs())
                            .get(FontGlyphId::new(glyph.id.0))
                            .context("ligature point caret has no glyph outline")?;
                        let mut resolved = 0;
                        // Expand the varied/composite outline once for all
                        // requested points. Only their coordinates are kept;
                        // no path conversion or second full point array.
                        let count = outline.visit_glyf_points(
                            LocationRef::new(loaded.font.shaper().coords()),
                            None,
                            |index, x, y| {
                                while let Some(&(wanted, slot)) = point_targets.get(resolved) {
                                    if usize::from(wanted) != index { break; }
                                    positions[slot] = glyph.position.x + gpui::px((x + loaded.synthetic_skew * y) * scale);
                                    resolved += 1;
                                }
                            },
                        )?.context("ligature point caret requires a TrueType outline")?;
                        anyhow::ensure!(resolved == point_targets.len(),
                            "ligature caret point is outside the glyph's {count} contour points");
                        anyhow::ensure!(positions.iter().all(|x| x.as_f32().is_finite()),
                            "non-finite contour ligature caret");
                    }
                    positions.sort_unstable_by(|a, b| a.as_f32().total_cmp(&b.as_f32()));
                    if glyph.is_rtl { positions.reverse(); }
                    for (caret, x) in carets[start..end].iter_mut().zip(positions.iter().copied()) {
                        caret.x = x;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                log::warn!("failed to read native ligature carets for font {:?}: {error:#}; retaining proportional positions for unresolved clusters", run.font_id);
            }
        }
    }

    fn missing_glyphs(
        &self,
        text: &str,
        font_runs: &[FontRun],
        missing_text_indices: impl IntoIterator<Item = usize>,
    ) -> Vec<MissingGlyph> {
        let mut missing_text_indices = missing_text_indices.into_iter().peekable();
        if missing_text_indices.peek().is_none() {
            return Vec::new();
        }
        let mut missing_text_indices = missing_text_indices.collect::<Vec<_>>();
        missing_text_indices.sort_unstable();
        missing_text_indices.dedup();

        let mut font_run_index = 0;
        let mut font_run_end = font_runs.first().map_or(0, |font_run| font_run.len);
        let mut missing_glyphs = Vec::new();
        let mut missing_index = 0;
        for (grapheme_start, grapheme) in text.grapheme_indices(true) {
            let grapheme_end = grapheme_start + grapheme.len();
            while missing_text_indices
                .get(missing_index)
                .is_some_and(|text_index| *text_index < grapheme_start)
            {
                missing_index += 1;
            }
            let Some(&text_index) = missing_text_indices.get(missing_index) else {
                break;
            };
            if text_index >= grapheme_end {
                continue;
            }

            while font_run_end <= text_index && font_run_index + 1 < font_runs.len() {
                font_run_index += 1;
                let Some(font_run) = font_runs.get(font_run_index) else {
                    break;
                };
                font_run_end += font_run.len;
            }
            let font_class = self.fallback_font_class(
                font_runs
                    .get(font_run_index)
                    .or_else(|| font_runs.last())
                    .map(|font_run| font_run.font_id),
            );
            missing_glyphs.push(MissingGlyph::new(grapheme.into(), font_class));
            while missing_text_indices
                .get(missing_index)
                .is_some_and(|text_index| *text_index < grapheme_end)
            {
                missing_index += 1;
            }
        }
        missing_glyphs
    }

    fn fallback_font_class(&self, font_id: Option<FontId>) -> FallbackFontClass {
        let Some(font_id) = font_id else {
            return FallbackFontClass::Proportional;
        };
        let loaded_font = self.loaded_font(font_id);
        let is_monospace = self
            .font_system
            .db()
            .face(loaded_font.font.id())
            .is_some_and(|face| face.monospaced);
        if is_monospace {
            FallbackFontClass::Monospace
        } else {
            FallbackFontClass::Proportional
        }
    }
}

#[inline(always)]
fn is_paragraph_separator(character: char) -> bool {
    unicode_bidi::bidi_class(character) == unicode_bidi::BidiClass::B
}

fn contains_paragraph_separator(text: &str) -> bool {
    if text
        .bytes()
        .any(|byte| matches!(byte, b'\n' | b'\r' | 0x1c | 0x1d | 0x1e))
    {
        return true;
    }

    !text.is_ascii() && text.chars().any(is_paragraph_separator)
}

fn clip_font_runs(font_runs: &[FontRun], range: Range<usize>) -> SmallVec<[FontRun; 4]> {
    let mut clipped = SmallVec::new();
    let mut offs = 0;
    for run in font_runs {
        let run_start = offs;
        offs += run.len;
        if offs <= range.start {
            continue;
        }
        if run_start >= range.end {
            break;
        }
        let start = run_start.max(range.start);
        let end = offs.min(range.end);
        if start < end {
            clipped.push(FontRun { font_size: run.font_size,
                len: end - start,
                font_id: run.font_id,
            });
        }
    }
    clipped
}

// GPUI validates finite angles at construction/deserialization; Cosmic owns
// an independent scalar type to keep its core free of GPUI dependencies.
fn cosmic_style(style: gpui::FontStyle) -> (Style, Option<cosmic_text::ObliqueAngle>) {
    match style {
        gpui::FontStyle::Normal => (Style::Normal, None),
        gpui::FontStyle::Italic => (Style::Italic, None),
        gpui::FontStyle::Oblique => (Style::Oblique, None),
        gpui::FontStyle::ObliqueAngle(angle) if angle.degrees() == 0. => (Style::Normal, None),
        gpui::FontStyle::ObliqueAngle(angle) => (Style::Oblique,
            Some(cosmic_text::ObliqueAngle::new(angle.degrees()).expect("validated GPUI oblique angle"))),
    }
}

// Match widths before native angle ranges and actual weights. Both font-kit
// configurations use the same angle decision, including reverse slants.
fn find_oblique_match(font: &Font, candidates: &[FontId], state: &CosmicTextSystemState,
    angle: cosmic_text::ObliqueAngle) -> Result<usize> {
    let mut best = None;
    for (index, id) in candidates.iter().enumerate() {
        let loaded = state.loaded_font(*id);
        let face = state.font_system.db().face(loaded.font.id()).context("font face not found")?;
        let width = face.stretch.to_number();
        let (width_phase, width_distance) = if width <= 5 { (0u8, 5 - width) } else { (1, width - 5) };
        let target = font.weight.0;
        let weight = loaded.variable_weight(target).unwrap_or(face.weight).0 as f32;
        let weight_phase = if target >= 400. && target <= 500. {
            if weight >= target && weight <= 500. { 0u8 } else if weight < target { 1 } else { 2 }
        } else if (target < 400. && weight <= target) || (target > 500. && weight >= target) { 0 } else { 1 };
        let rank = (width_phase, width_distance, loaded.font.oblique_match(angle),
            weight_phase, (weight - target).abs().to_bits());
        if best.as_ref().is_none_or(|(previous, _)| rank < *previous) { best = Some((rank, index)); }
    }
    best.map(|(_, index)| index).context("requested font family contains no matching face")
}

#[cfg(feature = "font-kit")]
fn find_best_match(
    font: &Font,
    candidates: &[FontId],
    state: &CosmicTextSystemState,
) -> Result<usize> {
    if let (Style::Oblique, angle) = cosmic_style(font.style) {
        return find_oblique_match(font, candidates, state,
            angle.unwrap_or_else(|| cosmic_text::ObliqueAngle::new(14.).expect("default oblique angle")));
    }
    let candidate_properties = candidates
        .iter()
        .map(|font_id| {
            let loaded = state.loaded_font(*font_id);
            let database_id = loaded.font.id();
            let face_info = state
                .font_system
                .db()
                .face(database_id)
                .context("font face not found in database")?;
            let mut properties = face_info_into_properties(face_info);
            let (requested_style, requested_angle) = cosmic_style(font.style);
            if loaded.font.supports_style_with_angle(requested_style, requested_angle) {
                properties.style = font_into_properties(font).style;
            }
            if let Some(weight) = loaded.variable_weight(font.weight.0) {
                properties.weight = font_kit::properties::Weight(weight.0 as f32);
            }
            Ok(properties)
        })
        .collect::<Result<SmallVec<[_; 4]>>>()?;

    let ix =
        font_kit::matching::find_best_match(&candidate_properties, &font_into_properties(font))
            .context("requested font family contains no font matching the other parameters")?;

    Ok(ix)
}

#[cfg(not(feature = "font-kit"))]
fn find_best_match(
    font: &Font,
    candidates: &[FontId],
    state: &CosmicTextSystemState,
) -> Result<usize> {
    if let (Style::Oblique, angle) = cosmic_style(font.style) {
        return find_oblique_match(font, candidates, state,
            angle.unwrap_or_else(|| cosmic_text::ObliqueAngle::new(14.).expect("default oblique angle")));
    }
    if candidates.is_empty() {
        anyhow::bail!("requested font family contains no font matching the other parameters");
    }
    if candidates.len() == 1 {
        return Ok(0);
    }

    let target_weight = font.weight.0;
    let (target_style, target_angle) = cosmic_style(font.style);
    let target_italic = matches!(
        font.style,
        gpui::FontStyle::Italic | gpui::FontStyle::Oblique | gpui::FontStyle::ObliqueAngle(_)
    );

    let mut best_index = 0;
    let mut best_score = u32::MAX;

    for (index, font_id) in candidates.iter().enumerate() {
        let database_id = state.loaded_font(*font_id).font.id();
        let face_info = state
            .font_system
            .db()
            .face(database_id)
            .context("font face not found in database")?;

        let is_italic = matches!(
            face_info.style,
            cosmic_text::Style::Italic | cosmic_text::Style::Oblique
        );
        let supports_style = state.loaded_font(*font_id).font.supports_style_with_angle(target_style, target_angle);
        let style_penalty: u32 = if supports_style || is_italic == target_italic { 0 } else { 1000 };
        let weight = state.loaded_font(*font_id).variable_weight(target_weight).unwrap_or(face_info.weight);
        let weight_diff = (weight.0 as i32 - target_weight as i32).unsigned_abs();
        let score = style_penalty + weight_diff;

        if score < best_score {
            best_score = score;
            best_index = index;
        }
    }

    Ok(best_index)
}

/// one contiguous slice of a `FontRun` that maps to a single slot. `slot` is
/// `None` for the primary font and `Some(ix)` for `fallback_chain[ix]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RunSpan {
    start: usize,
    end: usize,
    slot: Option<usize>,
    font_id: FontId,
}

/// walks `text[run_offset..run_offset + run_len]` and groups codepoints into
/// spans. inheriting codepoints stay in the current span so shaping clusters
/// like emoji zwj sequences and combining marks are not torn apart.
fn compute_run_spans(
    text: &str,
    run_offset: usize,
    run_len: usize,
    primary: FontId,
    fallback_chain: &[(FontId, SharedString)],
    covers: &impl Fn(FontId, char) -> bool,
) -> SmallVec<[RunSpan; 4]> {
    let mut spans = SmallVec::new();
    let run_end = run_offset + run_len;
    if run_end <= run_offset {
        return spans;
    }
    if fallback_chain.is_empty() {
        spans.push(RunSpan {
            start: run_offset,
            end: run_end,
            slot: None,
            font_id: primary,
        });
        return spans;
    }
    let run_text = &text[run_offset..run_end];
    let mut span_start = run_offset;
    let mut span_slot: Option<usize> = None;
    let mut span_font_id = primary;
    for (grapheme_idx, grapheme) in run_text.grapheme_indices(true) {
        let abs = run_offset + grapheme_idx;
        let ch = grapheme.chars().next().unwrap_or('\0');
        let next_slot = pick_covering_slot(ch, span_slot, primary, fallback_chain, covers);
        if next_slot == span_slot {
            continue;
        }
        if abs > span_start {
            spans.push(RunSpan {
                start: span_start,
                end: abs,
                slot: span_slot,
                font_id: span_font_id,
            });
        }
        span_start = abs;
        span_slot = next_slot;
        span_font_id = slot_font_id(next_slot, primary, fallback_chain);
    }
    if span_start < run_end {
        spans.push(RunSpan {
            start: span_start,
            end: run_end,
            slot: span_slot,
            font_id: span_font_id,
        });
    }
    spans
}

fn slot_font_id(
    slot: Option<usize>,
    primary: FontId,
    fallback_chain: &[(FontId, SharedString)],
) -> FontId {
    match slot {
        None => primary,
        Some(ix) => fallback_chain[ix].0,
    }
}

fn pick_covering_slot(
    ch: char,
    current: Option<usize>,
    primary: FontId,
    fallback_chain: &[(FontId, SharedString)],
    covers: &impl Fn(FontId, char) -> bool,
) -> Option<usize> {
    if (ch as u32) <= 0x7F {
        return None;
    }
    if covers(primary, ch) {
        return None;
    }
    let current_id = slot_font_id(current, primary, fallback_chain);
    if covers(current_id, ch) {
        return current;
    }

    fallback_chain
        .iter()
        .position(|(fb_id, _)| covers(*fb_id, ch))
}

fn charmap_covers(loaded_fonts: &[LoadedFont], id: FontId, ch: char) -> bool {
    loaded_fonts
        .get(id.0)
        .is_some_and(|loaded| loaded.font.as_swash().charmap().map(ch) != 0)
}

fn cosmic_font_features(features: &FontFeatures) -> Result<CosmicFontFeatures> {
    let mut result = CosmicFontFeatures::new();
    for feature in features.0.iter() {
        let name_bytes: [u8; 4] = feature
            .0
            .as_bytes()
            .try_into()
            .context("Incorrect feature flag format")?;

        let tag = cosmic_text::FeatureTag::new(&name_bytes);

        result.set(tag, feature.1);
    }
    Ok(result)
}

#[cfg(feature = "font-kit")]
fn font_into_properties(font: &gpui::Font) -> font_kit::properties::Properties {
    font_kit::properties::Properties {
        style: match font.style {
            gpui::FontStyle::Normal => font_kit::properties::Style::Normal,
            gpui::FontStyle::Italic => font_kit::properties::Style::Italic,
            gpui::FontStyle::Oblique | gpui::FontStyle::ObliqueAngle(_) => font_kit::properties::Style::Oblique,
        },
        weight: font_kit::properties::Weight(font.weight.0),
        stretch: Default::default(),
    }
}

#[cfg(feature = "font-kit")]
fn face_info_into_properties(
    face_info: &cosmic_text::fontdb::FaceInfo,
) -> font_kit::properties::Properties {
    font_kit::properties::Properties {
        style: match face_info.style {
            cosmic_text::Style::Normal => font_kit::properties::Style::Normal,
            cosmic_text::Style::Italic => font_kit::properties::Style::Italic,
            cosmic_text::Style::Oblique => font_kit::properties::Style::Oblique,
        },
        weight: font_kit::properties::Weight(face_info.weight.0.into()),
        stretch: match face_info.stretch {
            cosmic_text::Stretch::Condensed => font_kit::properties::Stretch::CONDENSED,
            cosmic_text::Stretch::Expanded => font_kit::properties::Stretch::EXPANDED,
            cosmic_text::Stretch::ExtraCondensed => font_kit::properties::Stretch::EXTRA_CONDENSED,
            cosmic_text::Stretch::ExtraExpanded => font_kit::properties::Stretch::EXTRA_EXPANDED,
            cosmic_text::Stretch::Normal => font_kit::properties::Stretch::NORMAL,
            cosmic_text::Stretch::SemiCondensed => font_kit::properties::Stretch::SEMI_CONDENSED,
            cosmic_text::Stretch::SemiExpanded => font_kit::properties::Stretch::SEMI_EXPANDED,
            cosmic_text::Stretch::UltraCondensed => font_kit::properties::Stretch::ULTRA_CONDENSED,
            cosmic_text::Stretch::UltraExpanded => font_kit::properties::Stretch::ULTRA_EXPANDED,
        },
    }
}

fn check_is_known_emoji_font(postscript_name: &str) -> bool {
    // TODO: Include other common emoji fonts
    postscript_name == "NotoColorEmoji"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "test-support")]
    use std::{cell::RefCell, rc::Rc};
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn all_font_names_tracks_available_families() -> Result<()> {
        let text_system = gpui::TextSystem::new(Arc::new(
            CosmicTextSystem::new_without_system_fonts("IBM Plex Sans"),
        ));
        assert!(text_system.all_font_names().is_empty());

        text_system.add_fonts(vec![Cow::Borrowed(include_bytes!(
            "../../../assets/fonts/lilex/Lilex-Regular.ttf"
        ))])?;
        assert_eq!(text_system.all_font_names(), ["Lilex"]);

        text_system.add_fonts(vec![
            Cow::Borrowed(IBM_PLEX),
            Cow::Borrowed(include_bytes!("../../../assets/fonts/lilex/Lilex-Bold.ttf")),
        ])?;
        assert_eq!(text_system.all_font_names(), ["IBM Plex Sans", "Lilex"]);
        Ok(())
    }

    fn fid(i: usize) -> FontId {
        FontId(i)
    }

    fn chain(ids: &[usize]) -> SmallVec<[(FontId, SharedString); 4]> {
        ids.iter()
            .map(|&i| (fid(i), SharedString::from(format!("fb{i}"))))
            .collect()
    }

    fn span(start: usize, end: usize, slot: Option<usize>, font_id: FontId) -> RunSpan {
        RunSpan {
            start,
            end,
            slot,
            font_id,
        }
    }

    const IBM_PLEX: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
    const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");

    /// Every code point of `Bidi_Class=B`, each of which starts a new bidi
    /// paragraph and so can split one line into mixed-direction paragraphs.
    const SEPARATORS: &[char] = &[
        '\u{000a}', '\u{000d}', '\u{001c}', '\u{001d}', '\u{001e}', '\u{0085}', '\u{2029}',
    ];

    fn text_system() -> Result<CosmicTextSystem> {
        let text_system = CosmicTextSystem::new_without_system_fonts("IBM Plex Sans");
        text_system.add_fonts(vec![Cow::Borrowed(IBM_PLEX)])?;
        Ok(text_system)
    }

    #[test]
    fn cached_font_resolution_does_not_wait_for_cold_lookups() -> Result<()> {
        for family in [".ZedSans", "Missing Font"] {
            let platform_text_system = Arc::new(text_system()?);
            let text_system = gpui::TextSystem::new(platform_text_system.clone());
            let cached = gpui::font(family);
            let expected = platform_text_system.font_id(&gpui::font(".ZedSans"))?;
            assert_eq!(text_system.resolve_font(&cached), expected);
            let (sender, receiver) = mpsc::channel();

            thread::scope(|scope| {
                let backend = platform_text_system.0.read();
                let deadline = Instant::now() + Duration::from_secs(5);
                let cold = scope.spawn(|| text_system.resolve_font(&cached.clone().bold()));
                while !platform_text_system.0.is_locked_exclusive() && Instant::now() < deadline {
                    thread::yield_now();
                }
                let cold_waiting = platform_text_system.0.is_locked_exclusive();
                let warm = scope.spawn(|| sender.send(text_system.resolve_font(&cached)));
                let result =
                    receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()));
                drop(backend);
                let cold_result = cold.join();
                let warm_result = warm.join();

                assert!(cold_waiting);
                assert_eq!(result.expect("cached lookup blocked"), expected);
                assert_eq!(cold_result.expect("cold lookup panicked"), expected);
                warm_result
                    .expect("warm lookup panicked")
                    .expect("receiver dropped");
            });
        }
        Ok(())
    }

    #[test]
    fn adding_fonts_retries_cached_font_failures() -> Result<()> {
        let platform_text_system = Arc::new(CosmicTextSystem::new_without_system_fonts("Lilex"));
        let text_system = gpui::TextSystem::new(platform_text_system.clone());
        text_system.add_fonts(vec![Cow::Borrowed(IBM_PLEX)])?;
        let original_fallback = platform_text_system.font_id(&gpui::font(".ZedSans"))?;
        let requests = [
            gpui::Font::default(),
            gpui::font("Lilex"),
            gpui::font("Missing Font"),
        ];
        for requested in &requests {
            assert_eq!(text_system.resolve_font(requested), original_fallback);
        }
        assert_eq!(
            text_system.get_font_for_id(original_fallback),
            Some(gpui::font(".ZedSans"))
        );

        text_system.add_fonts(vec![Cow::Borrowed(LILEX)])?;
        for (requested, resolved) in requests.iter().zip([".SystemUIFont", "Lilex", ".ZedMono"]) {
            let installed = platform_text_system.font_id(&gpui::font(resolved))?;
            assert_ne!(installed, original_fallback);
            assert_eq!(text_system.resolve_font(requested), installed);
        }
        Ok(())
    }

    #[test]
    fn font_properties_describe_the_selected_face() -> Result<()> {
        let text_system = text_system()?;
        let regular = gpui::font("IBM Plex Sans");
        let regular_id = text_system.font_id(&regular)?;
        for (weight, style) in [
            (gpui::FontWeight::MEDIUM, gpui::FontStyle::Normal),
            (gpui::FontWeight::BOLD, gpui::FontStyle::Italic),
            (gpui::FontWeight::NORMAL, gpui::FontStyle::Oblique),
        ] {
            let requested = Font {
                weight,
                style,
                ..regular.clone()
            };
            let font_id = text_system.font_id(&requested)?;
            let state = text_system.0.read();
            assert_eq!(state.loaded_font(font_id).font.id(), state.loaded_font(regular_id).font.id());
            drop(state);
            assert_eq!(
                text_system.font_weight_and_style(font_id)?,
                (gpui::FontWeight::NORMAL, gpui::FontStyle::Normal)
            );
        }
        Ok(())
    }

    fn layout_text(text_system: &CosmicTextSystem, text: &str) -> Result<LineLayout> {
        let font_id = text_system.font_id(&gpui::font("IBM Plex Sans"))?;
        let runs = [FontRun { font_size: None,
            len: text.len(),
            font_id,
        }];
        Ok(text_system.layout_line(text, gpui::px(14.0), &runs))
    }

    /// Mirrors the original crash: mixed-direction text reaching the shaper
    /// through `shape_text`, which only splits lines on `\n`.
    #[test]
    fn shape_text_with_mixed_direction_paragraphs() -> Result<()> {
        let platform_text_system = Arc::new(text_system()?);
        let text_system = Arc::new(gpui::TextSystem::new(platform_text_system));
        let window_text_system = gpui::WindowTextSystem::new(text_system);

        let text: SharedString = "first line\n\u{05d0}\u{001c}A".into();
        let runs = [gpui::TextRun {
            len: text.len(),
            font: gpui::font("IBM Plex Sans"),
            ..Default::default()
        }];

        let lines = window_text_system.shape_text(text, gpui::px(14.0), &runs, None, None)?;

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].len(), "\u{05d0}\u{001c}A".len());
        assert!(lines[1].width() > Pixels::ZERO);
        Ok(())
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn reports_graphemes_that_exhaust_font_fallback() -> Result<()> {
        let platform_text_system = Arc::new(text_system()?);
        let dispatcher = gpui::TestDispatcher::new(0);
        let cx =
            gpui::TestAppContext::build_with_text_system(dispatcher, None, platform_text_system);
        let observed = Rc::new(RefCell::new(Vec::new()));
        let _subscription = cx.update(|cx| {
            let observed = observed.clone();
            cx.on_missing_glyphs(move |missing_glyphs, _| {
                observed.borrow_mut().extend_from_slice(missing_glyphs);
            })
        });
        let text: SharedString = "界".into();

        cx.update(|cx| {
            let text_system = gpui::WindowTextSystem::new(cx.text_system().clone());
            let runs = [gpui::TextRun {
                len: text.len(),
                font: gpui::font("IBM Plex Sans"),
                ..Default::default()
            }];
            text_system.shape_line(text, gpui::px(14.0), &runs, None);
        });
        cx.run_until_parked();

        let observed = observed.borrow();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].grapheme(), "界");
        assert_eq!(
            observed[0].font_class(),
            gpui::FallbackFontClass::Proportional
        );
        Ok(())
    }

    #[test]
    fn combines_missing_glyphs_from_one_grapheme() -> Result<()> {
        let text_system = text_system()?;
        let font_id = text_system.font_id(&gpui::font("IBM Plex Sans"))?;
        let text = "x\u{0301}";
        let runs = [FontRun { font_size: None,
            len: text.len(),
            font_id,
        }];

        let missing_glyphs = text_system
            .0
            .read()
            .missing_glyphs(text, &runs, [0, "x".len()]);

        assert_eq!(missing_glyphs.len(), 1);
        assert_eq!(missing_glyphs[0].grapheme(), text);
        Ok(())
    }

    #[test]
    fn adding_fonts_invalidates_cached_line_layouts() -> Result<()> {
        let platform_text_system = Arc::new(text_system()?);
        let text_system = Arc::new(gpui::TextSystem::new(platform_text_system.clone()));
        let window_text_system = gpui::WindowTextSystem::new(text_system.clone());
        let text: SharedString = "cached text".into();
        let runs = [gpui::TextRun {
            len: text.len(),
            font: gpui::font("IBM Plex Sans"),
            ..Default::default()
        }];

        let first_layout = window_text_system.shape_line(text.clone(), gpui::px(14.0), &runs, None);
        let cached_layout =
            window_text_system.shape_line(text.clone(), gpui::px(14.0), &runs, None);
        assert!(std::ptr::eq::<LineLayout>(
            &**first_layout,
            &**cached_layout
        ));
        let loaded_font_count = platform_text_system.0.read().loaded_fonts.len();

        text_system.add_fonts(vec![Cow::Borrowed(LILEX)])?;

        let refreshed_layout = window_text_system.shape_line(text, gpui::px(14.0), &runs, None);
        assert!(!std::ptr::eq::<LineLayout>(
            &**first_layout,
            &**refreshed_layout
        ));
        assert_eq!(
            platform_text_system.0.read().loaded_fonts.len(),
            loaded_font_count
        );
        Ok(())
    }

    #[test]
    fn layout_line_with_mixed_direction_paragraphs() -> Result<()> {
        let text_system = text_system()?;

        for separator in SEPARATORS {
            for text in [
                format!("\u{05d0}{separator}A"),
                format!("A{separator}\u{05d0}"),
            ] {
                let layout = layout_text(&text_system, &text)?;

                assert_eq!(layout.len, text.len(), "{text:?}");
                assert!(layout.width > Pixels::ZERO, "{text:?}");
                assert!(
                    layout.runs.iter().any(|run| !run.glyphs.is_empty()),
                    "{text:?}"
                );
            }
        }

        Ok(())
    }

    #[test]
    fn layout_line_with_separators_at_line_edges() -> Result<()> {
        let text_system = text_system()?;

        for text in [
            "\u{001c}",
            "\u{001c}\u{001c}",
            "\u{001c}\u{05d0}",
            "\u{05d0}\u{001c}",
            "\u{05d0}\u{001c}\u{001c}A",
            "\u{001c}\u{05d0}\u{001c}A\u{001c}",
        ] {
            let layout = layout_text(&text_system, text)?;
            assert_eq!(layout.len, text.len(), "{text:?}");
        }

        Ok(())
    }

    /// Glyph indices must stay absolute and positions ordered across segment
    /// boundaries, otherwise cursor placement and hit testing desync. Uses
    /// single-direction text so visual order matches logical order.
    #[test]
    fn layout_line_keeps_indices_and_positions_ordered_across_paragraphs() -> Result<()> {
        let text_system = text_system()?;
        let text = "ab\u{001c}cd\u{2029}ef";
        let layout = layout_text(&text_system, text)?;

        let glyphs: Vec<_> = layout.runs.iter().flat_map(|run| &run.glyphs).collect();
        assert!(!glyphs.is_empty());

        for glyph in &glyphs {
            assert!(glyph.index < text.len(), "{:?}", glyph.index);
            assert!(text.is_char_boundary(glyph.index), "{:?}", glyph.index);
        }
        for pair in glyphs.windows(2) {
            assert!(pair[0].index < pair[1].index);
            assert!(pair[0].position.x <= pair[1].position.x);
        }

        // Every segment contributes width, so the whole line is wider than its
        // leading paragraph alone.
        assert!(layout.width > layout_text(&text_system, "ab")?.width);
        Ok(())
    }

    /// A font run boundary that does not line up with a paragraph boundary must
    /// still be clipped to the right segments.
    #[test]
    fn layout_line_with_font_run_straddling_a_separator() -> Result<()> {
        let text_system = text_system()?;
        let font_id = text_system.font_id(&gpui::font("IBM Plex Sans"))?;
        let text = "ab\u{001c}\u{05d0}\u{05d1}";

        // The run boundary falls inside the trailing RTL paragraph.
        let runs = [
            FontRun { font_size: None,
                len: "ab\u{001c}\u{05d0}".len(),
                font_id,
            },
            FontRun { font_size: None,
                len: "\u{05d1}".len(),
                font_id,
            },
        ];
        let layout = text_system.layout_line(text, gpui::px(14.0), &runs);

        assert_eq!(layout.len, text.len());
        assert!(layout.width > Pixels::ZERO);
        Ok(())
    }

    /// Lines with no separator take the fast path and must be shaped exactly as
    /// they were before paragraph splitting existed.
    #[test]
    fn layout_line_without_separators_takes_fast_path() -> Result<()> {
        let text_system = text_system()?;

        for text in [
            "hello world",
            "\u{05d0}\u{05d1}\u{05d2}",
            "mixed \u{05d0}\u{05d1}",
        ] {
            assert!(!contains_paragraph_separator(text), "{text:?}");
            let layout = layout_text(&text_system, text)?;
            assert_eq!(layout.len, text.len(), "{text:?}");
            assert!(layout.width > Pixels::ZERO, "{text:?}");
        }

        Ok(())
    }

    /// cosmic-text sums word widths to get a line's width but accumulates glyph
    /// advances to position glyphs, so a trailing zero-advance glyph (here a
    /// zero-width space) can land a few ulps past the width. When that glyph is a
    /// wrap boundary, the row before it extends past the line's width, and hit
    /// testing in that sliver used to panic (ZED-BW8, ZED-75K, ZED-81Z).
    #[test]
    fn index_for_position_past_line_width() -> Result<()> {
        let text_system = Arc::new(gpui::TextSystem::new(Arc::new(text_system()?)));
        let window_text_system = gpui::WindowTextSystem::new(text_system);
        let text: SharedString = "Warning: this will delete files\u{200b}".into();
        let runs = [gpui::TextRun {
            len: text.len(),
            font: gpui::font("IBM Plex Sans"),
            ..Default::default()
        }];
        let lines =
            window_text_system.shape_text(text, gpui::px(14.), &runs, Some(gpui::px(4.)), None)?;
        let line = &lines[0];
        let width = line.unwrapped_layout.width;
        let boundary_glyph = |row: usize| {
            let boundary = line.wrap_boundaries()[row];
            &line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix]
        };
        let row = (1..line.wrap_boundaries().len())
            .find(|row| boundary_glyph(*row).position.x > width)
            .expect("trailing zero-width space should be a wrap boundary past the line width");
        let row_start_x = f32::from(boundary_glyph(row - 1).position.x);
        let row_end = boundary_glyph(row);

        let mut x = f32::from(width) - row_start_x;
        while x + row_start_x < f32::from(width) {
            x = x.next_up();
        }
        assert!(gpui::px(x + row_start_x) < row_end.position.x);

        let line_height = gpui::px(20.);
        let position = gpui::point(gpui::px(x), line_height * row as f32 + gpui::px(1.));
        assert_eq!(
            line.index_for_position(position, line_height),
            Err(row_end.index)
        );
        Ok(())
    }

    #[test]
    fn paragraph_separator_detection() {
        for separator in SEPARATORS {
            assert!(is_paragraph_separator(*separator), "{separator:?}");
            assert!(contains_paragraph_separator(&format!("a{separator}b")));
        }

        for text in [
            "",
            "plain ascii",
            "\u{05d0}",
            "tab\there",
            "emoji \u{1f600}",
        ] {
            assert!(!contains_paragraph_separator(text), "{text:?}");
        }
    }

    #[test]
    fn font_runs_are_clipped_to_segment() {
        let runs = [
            FontRun { font_size: None,
                len: 3,
                font_id: fid(1),
            },
            FontRun { font_size: None,
                len: 4,
                font_id: fid(2),
            },
        ];

        assert_eq!(clip_font_runs(&runs, 0..7).as_slice(), &runs);
        assert_eq!(
            clip_font_runs(&runs, 2..5).as_slice(),
            &[
                FontRun { font_size: None,
                    len: 1,
                    font_id: fid(1)
                },
                FontRun { font_size: None,
                    len: 2,
                    font_id: fid(2)
                },
            ]
        );
        assert_eq!(
            clip_font_runs(&runs, 3..7).as_slice(),
            &[FontRun { font_size: None,
                len: 4,
                font_id: fid(2)
            }]
        );
        assert!(clip_font_runs(&runs, 5..5).is_empty());
    }

    #[test]
    fn primary_wins_over_current_fallback_when_primary_covers() {
        let primary = fid(0);
        let fb = chain(&[1, 2]);
        let covers = |id: FontId, _: char| id == fid(0) || id == fid(1);
        assert_eq!(
            pick_covering_slot('a', Some(0), primary, &fb, &covers),
            None
        );
    }

    #[test]
    fn primary_preferred_over_fallback_when_both_cover() {
        let primary = fid(0);
        let fb = chain(&[1]);
        let covers = |_: FontId, _: char| true;
        assert_eq!(pick_covering_slot('a', None, primary, &fb, &covers), None);
    }

    #[test]
    fn falls_through_chain_in_order() {
        let primary = fid(0);
        let fb = chain(&[1, 2, 3]);
        // only fallback 2 at index 1 covers.
        let covers = |id: FontId, _: char| id == fid(2);
        assert_eq!(
            pick_covering_slot('字', None, primary, &fb, &covers),
            Some(1)
        );
    }

    #[test]
    fn no_coverage_returns_primary() {
        let primary = fid(0);
        let fb = chain(&[1, 2]);
        let covers = |_: FontId, _: char| false;
        // nothing covers. return `None` so the `cosmic-text` built in script
        // fallback can take over during shaping.
        assert_eq!(
            pick_covering_slot('\u{1F600}', Some(1), primary, &fb, &covers),
            None
        );
    }

    #[test]
    fn empty_chain_always_returns_primary() {
        let primary = fid(0);
        let fb: SmallVec<[(FontId, SharedString); 4]> = SmallVec::new();
        let covers = |_: FontId, _: char| false;
        assert_eq!(pick_covering_slot('a', None, primary, &fb, &covers), None);
    }

    #[test]
    fn slot_font_id_resolution() {
        let primary = fid(7);
        let fb = chain(&[10, 20]);
        assert_eq!(slot_font_id(None, primary, &fb), fid(7));
        assert_eq!(slot_font_id(Some(0), primary, &fb), fid(10));
        assert_eq!(slot_font_id(Some(1), primary, &fb), fid(20));
    }

    #[test]
    fn run_spans_with_no_chain_emit_one_primary_span() {
        let primary = fid(0);
        let fb: SmallVec<[(FontId, SharedString); 4]> = SmallVec::new();
        let covers = |_: FontId, _: char| false;
        let text = "hello";
        let spans = compute_run_spans(text, 0, text.len(), primary, &fb, &covers);
        assert_eq!(spans.as_slice(), &[span(0, text.len(), None, primary)]);
    }

    #[test]
    fn run_spans_use_byte_offsets_for_multibyte_chars() {
        let primary = fid(0);
        let fb = chain(&[1]);
        // primary covers ascii. fallback covers cjk.
        let covers = |id: FontId, ch: char| {
            if id == primary {
                ch.is_ascii()
            } else {
                !ch.is_ascii()
            }
        };
        let text = "a字b";
        let spans = compute_run_spans(text, 0, text.len(), primary, &fb, &covers);
        // '字' is 3 bytes so split is at 1 then 4.
        assert_eq!(
            spans.as_slice(),
            &[
                span(0, 1, None, primary),
                span(1, 4, Some(0), fid(1)),
                span(4, 5, None, primary),
            ]
        );
    }

    #[test]
    fn run_spans_respect_run_offset() {
        let primary = fid(0);
        let fb = chain(&[1]);
        let covers = |id: FontId, ch: char| {
            if id == primary {
                ch.is_ascii()
            } else {
                !ch.is_ascii()
            }
        };
        // outer text has a prefix that is not part of this run.
        let text = "xx字y";
        let run_offset = 2;
        let run_len = text.len() - run_offset;
        let spans = compute_run_spans(text, run_offset, run_len, primary, &fb, &covers);
        assert_eq!(
            spans.as_slice(),
            &[span(2, 5, Some(0), fid(1)), span(5, 6, None, primary)]
        );
    }

    #[test]
    fn run_spans_keep_combining_marks_with_base_in_fallback() {
        let primary = fid(0);
        let fb = chain(&[1]);
        // primary covers ascii only. fallback covers the base char.
        // combining mark must stay in the fallback span even when fallback
        // does not advertise coverage of it.
        let covers = |id: FontId, ch: char| {
            if id == primary {
                ch.is_ascii()
            } else {
                ch == '\u{0905}'
            }
        };
        // \u{0905} devanagari short a + \u{0902} candrabindu mark.
        let text = "\u{0905}\u{0902}";
        let spans = compute_run_spans(text, 0, text.len(), primary, &fb, &covers);
        assert_eq!(spans.as_slice(), &[span(0, text.len(), Some(0), fid(1))]);
    }

    #[test]
    fn run_spans_keep_zwj_inside_emoji_cluster() {
        let primary = fid(0);
        let fb = chain(&[1]);
        // only fallback covers the emoji codepoints. zwj must not split.
        let covers = |id: FontId, ch: char| id == fid(1) && ch != '\u{200D}';
        // family zwj sequence woman zwj girl.
        let text = "\u{1F469}\u{200D}\u{1F467}";
        let spans = compute_run_spans(text, 0, text.len(), primary, &fb, &covers);
        assert_eq!(spans.as_slice(), &[span(0, text.len(), Some(0), fid(1))]);
    }

    #[test]
    fn run_spans_collapse_adjacent_same_slot() {
        let primary = fid(0);
        let fb = chain(&[1]);
        let covers = |id: FontId, ch: char| {
            if id == primary {
                ch.is_ascii()
            } else {
                !ch.is_ascii()
            }
        };
        let text = "字字字";
        let spans = compute_run_spans(text, 0, text.len(), primary, &fb, &covers);
        assert_eq!(spans.as_slice(), &[span(0, text.len(), Some(0), fid(1))]);
    }

    #[test]
    fn run_spans_empty_run_returns_no_spans() {
        let primary = fid(0);
        let fb = chain(&[1]);
        let covers = |_: FontId, _: char| true;
        let spans = compute_run_spans("anything", 3, 0, primary, &fb, &covers);
        assert!(spans.is_empty());
    }
}
