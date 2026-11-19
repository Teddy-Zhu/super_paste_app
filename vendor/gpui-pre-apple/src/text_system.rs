use anyhow::anyhow;
use collections::{HashMap, HashSet};
use core_foundation::{
    array::{CFArray, CFArrayRef},
    attributed_string::CFMutableAttributedString,
    base::{CFRange, TCFType},
    number::CFNumber,
    string::CFString,
};
use core_graphics::{
    base::{CGFloat, CGGlyph, kCGImageAlphaPremultipliedLast},
    color_space::CGColorSpace,
    context::{CGContext, CGTextDrawingMode},
    geometry::CGPoint,
};
use core_text::{
    font::CTFont,
    font_collection::CTFontCollectionRef,
    font_descriptor::{
        CTFontDescriptor, kCTFontDefaultOrientation, kCTFontSlantTrait, kCTFontSymbolicTrait, kCTFontWeightTrait,
        kCTFontWidthTrait,
    },
    line::CTLine,
    string_attributes::kCTFontAttributeName,
};
use font_kit::{
    font::Font as FontKitFont,
    handle::Handle,
    hinting::HintingOptions,
    metrics::Metrics,
    properties::{Style as FontkitStyle, Weight as FontkitWeight},
    source::SystemSource,
    sources::mem::MemSource,
};
use gpui::{
    Bounds, DevicePixels, Font, FontFallbacks, FontFeatures, FontId, FontMetrics, FontRun,
    FontStyle, FontWeight, GlyphId, GlyphInk, LineLayout, Pixels, Point, PlatformTextSystem, RenderGlyphParams,
    Result, SUBPIXEL_VARIANTS_X, ShapedGlyph, ShapedRun, SharedString, Size, TextRenderingMode,
    TextDirection, point, px, size, swap_rgba_pa_to_bgra,
};
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use pathfinder_geometry::{
    rect::RectF,
    vector::Vector2F,
};
use smallvec::SmallVec;
use std::{borrow::Cow, char, convert::TryFrom, hash::{Hash, Hasher}, sync::Arc};

#[cfg(target_os = "macos")]
use {
    core_foundation::base::CFType,
    gpui::{Hsla, Rgba},
    std::sync::OnceLock,
};

use crate::open_type::apply_features_and_fallbacks;

mod oblique;
mod ink;

#[allow(non_upper_case_globals)]
const kCGImageAlphaOnly: u32 = 7;

/// Apple text system using CoreText for font shaping.
pub struct AppleTextSystem(RwLock<AppleTextSystemState>);

#[derive(Clone, PartialEq, Eq, Hash)]
struct FontKey {
    font_family: SharedString,
    font_features: FontFeatures,
    font_fallbacks: Option<FontFallbacks>,
}

// A PostScript name identifies a face, not its Core Text instance. Keep the
// native size too: normalizing it can change an automatic optical-size axis.
// Requested style and weight remain part of source identity even when several
// requests select the same physical face; fallback fonts still need them.
#[derive(Clone, PartialEq, Eq)]
struct NativeFontKey(CTFont, oblique::Context);

impl Hash for NativeFontKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Match CTFont's CFEqual implementation, including native attributes
        // and transforms; pointer identity would miss equivalent run objects.
        unsafe { core_foundation::base::CFHash(self.0.as_CFTypeRef()) }.hash(state);
        self.1.hash(state);
    }
}

struct FontInfo {
    source: FontKitFont,
    drawing: Option<FontKitFont>,
    context: oblique::Context,
    synthetic_bold: bool,
}
impl FontInfo {
    fn drawing(&self) -> &FontKitFont { self.drawing.as_ref().unwrap_or(&self.source) }
    fn bold_offset(&self, size: f64) -> f64 { oblique::bold_offset(self.synthetic_bold, size) }
}

struct AppleTextSystemState {
    memory_source: MemSource,
    system_source: SystemSource,
    fonts: Vec<FontInfo>,
    font_selections: HashMap<Font, FontId>,
    font_ids_by_native_font: HashMap<NativeFontKey, FontId>,
    font_ids_by_font_key: HashMap<FontKey, SmallVec<[FontId; 4]>>,
    postscript_names_by_font_id: HashMap<FontId, String>,
}

impl AppleTextSystem {
    /// Create a new AppleTextSystem.
    pub fn new() -> Self {
        Self(RwLock::new(AppleTextSystemState {
            memory_source: MemSource::empty(),
            system_source: SystemSource::new(),
            fonts: Vec::new(),
            font_selections: HashMap::default(),
            font_ids_by_native_font: HashMap::default(),
            font_ids_by_font_key: HashMap::default(),
            postscript_names_by_font_id: HashMap::default(),
        }))
    }
}

impl Default for AppleTextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformTextSystem for AppleTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.0.write().add_fonts(fonts)
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let collection = core_text::font_collection::create_for_all_families();
        // NOTE: We intentionally avoid using `collection.get_descriptors()` here because
        // it has a memory leak bug in core-text v21.0.0. The upstream code uses
        // `wrap_under_get_rule` but `CTFontCollectionCreateMatchingFontDescriptors`
        // follows the Create Rule (caller owns the result), so it should use
        // `wrap_under_create_rule`. We call the function directly with correct memory management.
        unsafe extern "C" {
            fn CTFontCollectionCreateMatchingFontDescriptors(
                collection: CTFontCollectionRef,
            ) -> CFArrayRef;
        }
        let descriptors: Option<CFArray<CTFontDescriptor>> = unsafe {
            let array_ref =
                CTFontCollectionCreateMatchingFontDescriptors(collection.as_concrete_TypeRef());
            if array_ref.is_null() {
                None
            } else {
                Some(CFArray::wrap_under_create_rule(array_ref))
            }
        };
        let Some(descriptors) = descriptors else {
            return names;
        };
        for descriptor in descriptors.into_iter() {
            names.extend(lenient_font_attributes::family_name(&descriptor));
        }
        if let Ok(fonts_in_memory) = self.0.read().memory_source.all_families() {
            names.extend(fonts_in_memory);
        }
        names
    }

    fn font_id(&self, font: &Font) -> Result<FontId> {
        anyhow::ensure!(font.weight.0.is_finite() && (1.0..=1000.0).contains(&font.weight.0), "font weight must be finite and between 1 and 1000");
        let context = oblique::Context::new(font.style, font.weight.0);
        let lock = self.0.upgradable_read();
        if let Some(font_id) = lock.font_selections.get(font) {
            Ok(*font_id)
        } else {
            let mut lock = RwLockUpgradableReadGuard::upgrade(lock);
            let font_key = FontKey {
                font_family: font.family.clone(),
                font_features: font.features.clone(),
                font_fallbacks: font.fallbacks.clone(),
            };
            let candidates = if let Some(font_ids) = lock.font_ids_by_font_key.get(&font_key) {
                font_ids.as_slice()
            } else {
                let font_ids =
                    lock.load_family(&font.family, &font.features, font.fallbacks.as_ref())?;
                lock.font_ids_by_font_key.insert(font_key.clone(), font_ids);
                lock.font_ids_by_font_key[&font_key].as_ref()
            };

            let candidate_properties = candidates
                .iter()
                .map(|font_id| oblique::matching_properties(&lock.fonts[font_id.0].source, context))
                .collect::<SmallVec<[_; 4]>>();

            let ix = font_kit::matching::find_best_match(
                &candidate_properties,
                &font_kit::properties::Properties {
                    style: fontkit_style(font.style),
                    weight: fontkit_weight(font.weight),
                    stretch: Default::default(),
                },
            )?;

            let selected = candidates[ix];
            let native = oblique::with_axes(&lock.fonts[selected.0].source.native_font(), context);
            let font_id = lock.id_for_native_font(native, context);
            lock.font_selections.insert(font.clone(), font_id);
            Ok(font_id)
        }
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        let state = self.0.read();
        let info = &state.fonts[font_id.0];
        let mut metrics = font_kit_metrics_to_metrics(info.drawing().metrics());
        metrics.bounding_box.size.width += info.bold_offset(f64::from(metrics.units_per_em)) as f32;
        metrics
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let state = self.0.read();
        let info = &state.fonts[font_id.0];
        let mut bounds = bounds_from_rect(info.drawing().typographic_bounds(glyph_id.0)?);
        if bounds.size.width > 0. && bounds.size.height > 0. {
            bounds.size.width += info.bold_offset(f64::from(info.drawing().native_font().units_per_em())) as f32;
        }
        Ok(bounds)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.0.read().advance(font_id, glyph_id)
    }

    fn intersect_glyph_ink(
        &self, font_id: FontId, glyph_id: GlyphId, font_size: Pixels, ink: &mut GlyphInk,
    ) -> Result<()> {
        struct Sink<'a> { ink: &'a mut GlyphInk, scale: f32 }
        impl Sink<'_> {
            fn pixel(&self, value: Vector2F) -> Point<f32> {
                point(value.x() * self.scale, -value.y() * self.scale)
            }
        }
        impl font_kit::outline::OutlineSink for Sink<'_> {
            fn move_to(&mut self, to: Vector2F) {
                let to = self.pixel(to);
                self.ink.move_to(to);
            }
            fn line_to(&mut self, to: Vector2F) {
                let to = self.pixel(to);
                self.ink.line_to(to);
            }
            fn quadratic_curve_to(&mut self, ctrl: Vector2F, to: Vector2F) {
                let ctrl = self.pixel(ctrl);
                let to = self.pixel(to);
                self.ink.quad_to(ctrl, to);
            }
            fn cubic_curve_to(&mut self, ctrl: pathfinder_geometry::line_segment::LineSegment2F, to: Vector2F) {
                let first = self.pixel(ctrl.from());
                let second = self.pixel(ctrl.to());
                let to = self.pixel(to);
                self.ink.curve_to(first, second, to);
            }
            fn close(&mut self) { self.ink.close(); }
        }
        let state = self.0.read();
        let info = &state.fonts[font_id.0];
        let font = info.drawing();
        let native = font.native_font();
        let scale = font_size.as_f32() / native.units_per_em() as f32;
        anyhow::ensure!(scale.is_finite() && scale > 0.0, "invalid glyph outline scale");
        if info.synthetic_bold {
            return ink::intersect_bold(&native, glyph_id, font_size, info.bold_offset(f64::from(font_size.as_f32())), ink);
        }
        font.outline(glyph_id.0, HintingOptions::None, &mut Sink { ink, scale })?;
        Ok(())
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.0.read().glyph_for_char(font_id, ch)
    }

    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        self.0.read().raster_bounds(params)
    }

    fn rasterize_glyph(
        &self,
        glyph_id: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        self.0.read().rasterize_glyph(glyph_id, raster_bounds)
    }

    fn layout_line_with_direction(
        &self,
        text: &str,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
    ) -> LineLayout {
        self.0.write().layout_line(text, font_size, font_runs, direction)
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }

    #[cfg(target_os = "macos")]
    fn glyph_dilation_for_color(&self, color: Hsla) -> u8 {
        // When font smoothing is enabled, CoreGraphics thickens glyph strokes by an amount that
        // depends on the foreground color's luminance. We replicate the logic used by CoreGraphics
        // to select between the different levels of dilation.
        if !font_smoothing_allowed_by_user() {
            return 0;
        }
        let rgba: Rgba = color.into();
        let luminance = 0.2126 * rgba.r + 0.7152 * rgba.g + 0.0722 * rgba.b;
        let level = ((4.0 * luminance) + 0.5).floor() as i32;
        level.clamp(0, 4) as u8
    }
}

#[cfg(target_os = "macos")]
fn font_smoothing_allowed_by_user() -> bool {
    static ALLOWED: OnceLock<bool> = OnceLock::new();
    *ALLOWED.get_or_init(|| {
        use core_foundation_sys::preferences::{
            CFPreferencesCopyAppValue, kCFPreferencesCurrentApplication,
        };

        let key = CFString::new("AppleFontSmoothing");
        let value_ref = unsafe {
            CFPreferencesCopyAppValue(key.as_concrete_TypeRef(), kCFPreferencesCurrentApplication)
        };
        if value_ref.is_null() {
            return true;
        }
        let value = unsafe { CFType::wrap_under_create_rule(value_ref) };
        let Some(number) = value.downcast_into::<CFNumber>() else {
            return true;
        };
        // Only an explicit value of `0` means that font smoothing is disabled.
        number.to_i64() != Some(0)
    })
}

impl AppleTextSystemState {
    fn add_fonts(&mut self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        let fonts = fonts
            .into_iter()
            .map(|bytes| match bytes {
                Cow::Borrowed(embedded_font) => {
                    let data_provider = unsafe {
                        core_graphics::data_provider::CGDataProvider::from_slice(embedded_font)
                    };
                    let font = core_graphics::font::CGFont::from_data_provider(data_provider)
                        .map_err(|()| anyhow!("Could not load an embedded font."))?;
                    let font = font_kit::loaders::core_text::Font::from_core_graphics_font(font);
                    Ok(Handle::from_native(&font))
                }
                Cow::Owned(bytes) => Ok(Handle::from_memory(Arc::new(bytes), 0)),
            })
            .collect::<Result<Vec<_>>>()?;
        self.memory_source.add_fonts(fonts.into_iter())?;
        Ok(())
    }

    fn load_family(
        &mut self,
        name: &str,
        features: &FontFeatures,
        fallbacks: Option<&FontFallbacks>,
    ) -> Result<SmallVec<[FontId; 4]>> {
        let name = gpui::font_name_with_fallbacks(name, ".AppleSystemUIFont");

        let mut font_ids = SmallVec::new();
        let mut native_fonts_seen = HashSet::default();
        let family = self
            .memory_source
            .select_family_by_name(name)
            .or_else(|_| self.system_source.select_family_by_name(name))?;
        for font in family.fonts() {
            let mut font = font.load()?;

            apply_features_and_fallbacks(&mut font, features, fallbacks)?;
            // This block contains a precautionary fix to guard against loading fonts
            // that might cause panics due to `.unwrap()`s up the chain.
            {
                // We use the 'm' character for text measurements in various spots
                // (e.g., the editor). However, at time of writing some of those usages
                // will panic if the font has no 'm' glyph.
                //
                // Therefore, we check up front that the font has the necessary glyph.
                let has_m_glyph = font.glyph_for_char('m').is_some();

                // HACK: The 'Segoe Fluent Icons' font does not have an 'm' glyph,
                // but we need to be able to load it for rendering Windows icons in
                // the Storybook (on macOS).
                let is_segoe_fluent_icons = font.full_name() == "Segoe Fluent Icons";

                if !has_m_glyph && !is_segoe_fluent_icons {
                    // I spent far too long trying to track down why a font missing the 'm'
                    // character wasn't loading. This log statement will hopefully save
                    // someone else from suffering the same fate.
                    log::warn!(
                        "font '{}' has no 'm' character and was not loaded",
                        font.full_name()
                    );
                    continue;
                }
            }

            // We've seen a number of panics in production caused by calling font.properties()
            // which unwraps a downcast to CFNumber. This is an attempt to avoid the panic,
            // and to try and identify the incalcitrant font.
            let traits = font.native_font().all_traits();
            if unsafe {
                !(traits
                    .get(kCTFontSymbolicTrait)
                    .downcast::<CFNumber>()
                    .is_some()
                    && traits
                        .get(kCTFontWidthTrait)
                        .downcast::<CFNumber>()
                        .is_some()
                    && traits
                        .get(kCTFontWeightTrait)
                        .downcast::<CFNumber>()
                        .is_some()
                    && traits
                        .get(kCTFontSlantTrait)
                        .downcast::<CFNumber>()
                        .is_some())
            } {
                log::error!(
                    "Failed to read traits for font {:?} (PostScript name {:?})",
                    font.full_name(),
                    font.postscript_name(),
                );
                continue;
            }

            let Some(postscript_name) = font.postscript_name() else {
                log::warn!(
                    "font {:?} in family {:?} has no PostScript name; skipping",
                    font.full_name(),
                    name,
                );
                continue;
            };
            // Dedup is scoped to this single `load_family` call (issue #55472).
            // The same family can be reloaded later under a different `FontKey`
            // (different features/fallbacks); a global check against
            // `font_ids_by_native_font` would skip every already-registered
            // font and leave the second call's `font_ids` empty.
            let context = oblique::Context::for_font(&font);
            let native_font = NativeFontKey(font.native_font(), context);
            if !native_fonts_seen.insert(native_font.clone()) {
                log::warn!(
                    "skipping duplicate font {:?} with PostScript name {:?} \
                     in family {:?}",
                    font.full_name(),
                    postscript_name,
                    name,
                );
                continue;
            }
            let font_id = FontId(self.fonts.len());
            font_ids.push(font_id);
            self.font_ids_by_native_font
                .insert(native_font, font_id);
            self.postscript_names_by_font_id
                .insert(font_id, postscript_name);
            self.fonts.push(FontInfo { source: font, drawing: None, context, synthetic_bold: false });
        }
        Ok(font_ids)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        Ok(size_from_vector2f(
            self.fonts[font_id.0].source.advance(glyph_id.0)?,
        ))
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.fonts[font_id.0].source.glyph_for_char(ch).map(GlyphId)
    }

    fn id_for_native_font(&mut self, requested_font: CTFont, context: oblique::Context) -> FontId {
        let key = NativeFontKey(requested_font, context);
        if let Some(font_id) = self.font_ids_by_native_font.get(&key) {
            *font_id
        } else {
            let font_id = FontId(self.fonts.len());
            let postscript_name = key.0.postscript_name();
            // The run already chose its variations, matrix and native traits.
            // Rebuilding from CGFont loses those instance attributes.
            let font = unsafe { FontKitFont::from_native_font(&key.0) };
            let synthetic_bold = oblique::synthetic_bold(&font, context);
            let drawing = oblique::drawing_font(&key.0, context.style)
                .map(|font| unsafe { FontKitFont::from_native_font(&font) });
            self.font_ids_by_native_font.insert(key, font_id);
            self.postscript_names_by_font_id
                .insert(font_id, postscript_name);
            self.fonts.push(FontInfo { source: font, drawing, context, synthetic_bold });
            font_id
        }
    }

    fn is_emoji(&self, font_id: FontId) -> bool {
        self.postscript_names_by_font_id
            .get(&font_id)
            .is_some_and(|postscript_name| {
                postscript_name == "AppleColorEmoji" || postscript_name == ".AppleColorEmojiUI"
            })
    }

    fn prepare_fallback_fonts(
        &self, string: &mut CFMutableAttributedString, line: &CTLine, text: &str,
        sources: &oblique::SourceRuns<'_>,
    ) -> bool {
        let mut converter = StringIndexConverter::new(text);
        let mut changed = false;
        for run in line.glyph_runs().into_iter() {
            if run.glyph_count() == 0 { continue; }
            let Some(index) = run.string_indices().first().and_then(|index| usize::try_from(*index).ok()) else { continue; };
            converter.advance_to_utf16_ix(index);
            let Some(source) = sources.at(converter.utf8_ix) else { continue; };
            let context = self.fonts[source.font_id.0].context;
            let attributes = run.attributes().unwrap();
            let font = unsafe { attributes.get(kCTFontAttributeName).downcast::<CTFont>().unwrap() };
            let cached = self.font_ids_by_native_font.get(&NativeFontKey(font.clone(), context));
            let source_name = &self.postscript_names_by_font_id[&source.font_id];
            let fallback = cached.map_or_else(|| font.postscript_name() != *source_name,
                |id| self.postscript_names_by_font_id[id] != *source_name);
            // Known primary instances and actual styled fallback faces already
            // carry the requested axes. A synthesized fallback must still try
            // a real sibling: a previous character may lack that face's glyph.
            if cached.is_some_and(|id| !fallback || {
                let info = &self.fonts[id.0];
                !info.synthetic_bold && info.drawing.is_none()
            }) { continue; }
            let selected = if fallback { oblique::with_fallback_face(&font, context) }
                else { oblique::with_axes(&font, context) };
            if selected != font {
                let native_run = unsafe { &*run.as_concrete_TypeRef().cast::<objc2_core_text::CTRun>() };
                let range = unsafe { native_run.string_range() };
                unsafe {
                    string.set_attribute(CFRange::init(range.location, range.length), kCTFontAttributeName, &selected);
                }
                changed = true;
            }
        }
        changed
    }

    fn raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let info = &self.fonts[params.font_id.0];
        let font = info.drawing().native_font();
        let font_size = f64::from(params.font_size.as_f32());
        let scale_factor = f64::from(params.scale_factor);
        anyhow::ensure!(font_size.is_finite() && font_size > 0. && scale_factor.is_finite() && scale_factor > 0., "invalid glyph raster scale");
        let scale = font_size / font.pt_size() * scale_factor;
        anyhow::ensure!(scale.is_finite() && scale > 0., "invalid native glyph scale");
        let glyph = CGGlyph::try_from(params.glyph_id.0)?;
        let rect = font.get_bounding_rects_for_glyphs(kCTFontDefaultOrientation, &[glyph]);
        // Keep native floating bounds until after range checks. Font-kit's
        // integer rectangle can overflow first for a nearly vertical slant.
        // Reflect Y and include one antialiasing pixel on each edge.
        let left = (rect.origin.x * scale).floor() - 1.;
        let top = (-(rect.origin.y + rect.size.height) * scale).floor() - 1.;
        let bold = if rect.size.width > 0. && rect.size.height > 0. { info.bold_offset(font_size) * scale_factor } else { 0. };
        let right = ((rect.origin.x + rect.size.width) * scale + bold).ceil() + 1.;
        let bottom = (-rect.origin.y * scale).ceil() + 1.;
        anyhow::ensure!([left, top, right, bottom].into_iter().all(|value| value.is_finite()
            && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX)), "glyph raster origin out of range");
        let width = right - left;
        let height = bottom - top;
        anyhow::ensure!(width > 0. && height > 0. && width <= MAX_RASTER_SIDE as f64 && height <= MAX_RASTER_SIDE as f64,
            "glyph raster dimensions exceed limit");
        let raster_size = size(DevicePixels(width as i32), DevicePixels(height as i32));
        raster_buffer_layout(params, raster_size)?;
        Ok(Bounds { origin: point(DevicePixels(left as i32), DevicePixels(top as i32)), size: raster_size })
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        } else {
            let (bitmap_size, stride, buffer_len) = raster_buffer_layout(params, glyph_bounds.size)?;

            let mut bytes;
            let cx;
            if params.is_emoji {
                bytes = vec![0; buffer_len];
                cx = CGContext::create_bitmap_context(
                    Some(bytes.as_mut_ptr() as *mut _),
                    bitmap_size.width.0 as usize,
                    bitmap_size.height.0 as usize,
                    8,
                    stride,
                    &CGColorSpace::create_device_rgb(),
                    kCGImageAlphaPremultipliedLast,
                );
            } else {
                bytes = vec![0; buffer_len];
                cx = CGContext::create_bitmap_context(
                    Some(bytes.as_mut_ptr() as *mut _),
                    bitmap_size.width.0 as usize,
                    bitmap_size.height.0 as usize,
                    8,
                    stride,
                    &CGColorSpace::create_device_gray(),
                    kCGImageAlphaOnly,
                );
            }

            // Move the origin to bottom left and account for scaling, this
            // makes drawing text consistent with the font-kit's raster_bounds.
            cx.translate(
                -(glyph_bounds.origin.x.0 as CGFloat),
                glyph_bounds.origin.y.0 as CGFloat + glyph_bounds.size.height.0 as CGFloat,
            );
            cx.scale(
                params.scale_factor as CGFloat,
                params.scale_factor as CGFloat,
            );

            let subpixel_shift = params
                .subpixel_variant
                .map(|v| v as f32 / SUBPIXEL_VARIANTS_X as f32);
            cx.set_text_drawing_mode(CGTextDrawingMode::CGTextFill);
            cx.set_allows_antialiasing(true);
            cx.set_should_antialias(true);
            cx.set_allows_font_subpixel_positioning(true);
            cx.set_should_subpixel_position_fonts(true);
            cx.set_allows_font_subpixel_quantization(false);
            cx.set_should_subpixel_quantize_fonts(false);

            if cfg!(target_os = "macos") && params.dilation > 0 {
                let luminance = params.dilation as f64 * 0.25;
                cx.set_should_smooth_fonts(true);
                cx.set_gray_fill_color(luminance, 1.0);
            } else {
                cx.set_gray_fill_color(0.0, 1.0);
            }
            let info = &self.fonts[params.font_id.0];
            let font = info.drawing().native_font().clone_with_font_size(f32::from(params.font_size) as CGFloat);
            let glyphs = [CGGlyph::try_from(params.glyph_id.0)?];
            let positions = [CGPoint::new(
                (subpixel_shift.x / params.scale_factor) as CGFloat,
                (subpixel_shift.y / params.scale_factor) as CGFloat,
            )];
            if info.synthetic_bold && !params.is_emoji {
                font.draw_glyphs(&glyphs, &positions, cx.clone());
                // Translate the context so the offset stays in user space,
                // including when the native font has a slant or scale matrix.
                cx.translate(info.bold_offset(f64::from(params.font_size.as_f32())), 0.);
            }
            font.draw_glyphs(&glyphs, &positions, cx);

            if params.is_emoji {
                // Convert from RGBA with premultiplied alpha to BGRA with straight alpha.
                for pixel in bytes.chunks_exact_mut(4) {
                    swap_rgba_pa_to_bgra(pixel);
                }
            }

            Ok((bitmap_size, bytes))
        }
    }

    fn layout_line(
        &mut self,
        text: &str,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
    ) -> LineLayout {
        // Insert the complete source once, then apply UTF-16 font ranges.
        // Decoding each font run separately can consume its initial U+FEFF as
        // an encoding signature and invalidate every subsequent source index.
        let mut string = CFMutableAttributedString::new();
        {
            let source = if text.starts_with('\u{feff}') {
                let utf16 = text.encode_utf16().collect::<Vec<_>>();
                // Explicit characters preserve a literal initial BOM. Core
                // Foundation copies this temporary buffer under Create Rule.
                unsafe {
                    CFString::wrap_under_create_rule(
                        core_foundation_sys::string::CFStringCreateWithCharacters(
                            core_foundation_sys::base::kCFAllocatorDefault,
                            utf16.as_ptr(),
                            utf16.len() as _,
                        ),
                    )
                }
            } else {
                CFString::new(text)
            };
            string.replace_str(&source, CFRange::init(0, 0));
        }
        let mut max_ascent = 0.0f32;
        let mut max_descent = 0.0f32;

        {
            let mut text = text;
            let mut utf16_offset = 0;
            // CoreText merges adjacent runs that end up in equal fonts, including runs of
            // different fonts that fall back to the same one, so every other nonempty
            // font run is set a hair larger to keep ligatures from forming across them. The first
            // run keeps its exact size: SF tracks by its exact point size, and a nudged size
            // draws wider or narrower than AppKit sets the same text.
            let mut break_ligature = false;
            for run in font_runs {
                let text_run;
                (text_run, text) = text.split_at(run.len);

                let length = text_run.encode_utf16().count() as _;
                let cf_range = CFRange::init(utf16_offset, length);
                utf16_offset += length;
                let font = &self.fonts[run.font_id.0].source;
                let font_size = run.font_size.unwrap_or(font_size);

                let native_font = font.native_font();
                let font_scale = f64::from(font_size.as_f32()) / native_font.pt_size();
                max_ascent = max_ascent.max((native_font.ascent() * font_scale) as f32);
                max_descent = max_descent.max((native_font.descent() * font_scale) as f32);

                let font_size = if break_ligature {
                    px(f32::from(font_size).next_up())
                } else {
                    font_size
                };
                unsafe {
                    string.set_attribute(
                        cf_range,
                        kCTFontAttributeName,
                        &native_font.clone_with_font_size(font_size.into()),
                    );
                }
                if length > 0 {
                    break_ligature = !break_ligature;
                }
            }
        }
        if direction != TextDirection::Auto && string.char_len() > 0 {
            use objc2_core_text::{
                CTParagraphStyle, CTParagraphStyleSetting, CTParagraphStyleSpecifier,
                CTWritingDirection,
            };
            let writing_direction = if direction == TextDirection::Rtl {
                CTWritingDirection::RightToLeft
            } else {
                CTWritingDirection::LeftToRight
            };
            let setting = CTParagraphStyleSetting {
                spec: CTParagraphStyleSpecifier::BaseWritingDirection,
                valueSize: std::mem::size_of_val(&writing_direction),
                value: std::ptr::NonNull::from(&writing_direction).cast(),
            };
            // Core Text copies this stack setting and the attributed string
            // retains its immutable style. The raw bridge only connects the
            // two Core Foundation binding versions already used by this crate.
            unsafe {
                let style = CTParagraphStyle::new(&setting, 1);
                core_foundation_sys::attributed_string::CFAttributedStringSetAttribute(
                    string.as_concrete_TypeRef(),
                    CFRange::init(0, string.char_len()),
                    std::ptr::from_ref(objc2_core_text::kCTParagraphStyleAttributeName).cast(),
                    objc2_core_foundation::CFRetained::as_ptr(&style).as_ptr().cast(),
                );
            }
        }
        // Retrieve the glyphs from the shaped line, converting UTF16 offsets to UTF8 offsets.
        let sources = oblique::SourceRuns::new(font_runs);
        let mut line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
        if self.prepare_fallback_fonts(&mut string, &line, text, &sources) {
            // Faces and axes can alter advances. Reshape the whole paragraph
            // once after assigning each selected fallback its native instance.
            line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
        }
        let glyph_runs = line.glyph_runs();
        let mut runs = <Vec<ShapedRun>>::with_capacity(glyph_runs.len() as usize);
        let mut ix_converter = StringIndexConverter::new(text);
        for run in glyph_runs.into_iter() {
            if run.glyph_count() == 0 { continue; }
            let attributes = run.attributes().unwrap();
            let font = unsafe {
                attributes
                    .get(kCTFontAttributeName)
                    .downcast::<CTFont>()
                    .unwrap()
            };
            let source = run.string_indices().first().and_then(|index| usize::try_from(*index).ok()).and_then(|index| {
                ix_converter.advance_to_utf16_ix(index);
                sources.at(ix_converter.utf8_ix)
            });
            let mut run_size = source.and_then(|run| run.font_size);
            let context = source.map_or_else(|| {
                let native = unsafe { FontKitFont::from_native_font(&font) };
                oblique::Context::for_font(&native)
            }, |run| self.fonts[run.font_id.0].context);
            let requested_size = run_size.unwrap_or(font_size);
            let native_size = font.pt_size();
            // A fallback can carry a native scale. Preserve it in the drawing
            // request, except for our one-ULP ligature-breaking size above.
            let effective_size = if native_size == requested_size.as_f32().next_up() as f64 {
                requested_size
            } else {
                px(native_size as f32)
            };
            if effective_size != requested_size { run_size = Some(effective_size); }
            // Include actual fallback metrics without letting the deliberate
            // ligature nudge grow a line by a pixel after baseline rounding.
            let metric_scale = f64::from(effective_size.as_f32()) / native_size;
            max_ascent = max_ascent.max((font.ascent() * metric_scale) as f32);
            max_descent = max_descent.max((font.descent() * metric_scale) as f32);
            let font_id = self.id_for_native_font(font, context);

            let glyphs = match runs.last_mut() {
                Some(run) if run.font_id == font_id && run.font_size == run_size => &mut run.glyphs,
                _ => {
                    runs.push(ShapedRun { font_size: run_size,
                        font_id,
                        glyphs: Vec::with_capacity(run.glyph_count().try_into().unwrap_or(0)),
                    });
                    &mut runs.last_mut().unwrap().glyphs
                }
            };
            // Both wrappers refer to the same retained, immutable Core Text run.
            let native_run = unsafe { &*run.as_concrete_TypeRef().cast::<objc2_core_text::CTRun>() };
            let advances = unsafe { native_run.advances_ptr() };
            let is_rtl = unsafe { native_run.status() }.contains(objc2_core_text::CTRunStatus::RightToLeft);
            for (glyph_ix, ((&glyph_id, position), &glyph_utf16_ix)) in run
                .glyphs()
                .iter()
                .zip(run.positions().iter())
                .zip(run.string_indices().iter())
                .enumerate()
            {
                let glyph_utf16_ix = usize::try_from(glyph_utf16_ix).unwrap();
                ix_converter.advance_to_utf16_ix(glyph_utf16_ix);
                let advance = if advances.is_null() {
                    let mut value = Default::default();
                    unsafe {
                        native_run.advances(
                            objc2_core_foundation::CFRange { location: glyph_ix as isize, length: 1 },
                            std::ptr::NonNull::from(&mut value),
                        );
                    }
                    value.width
                } else {
                    // A non-null Core Text pointer has glyph_count entries and
                    // stays valid while `run` owns the immutable native object.
                    unsafe { (*advances.add(glyph_ix)).width }
                };
                glyphs.push(ShapedGlyph {
                    id: GlyphId(glyph_id as u32),
                    position: point(position.x as f32, position.y as f32).map(px),
                    index: ix_converter.utf8_ix,
                    is_emoji: self.is_emoji(font_id),
                    advance: advance.into(),
                    is_rtl,
                });
            }
        }
        let typographic_bounds = line.get_typographic_bounds();
        let mut layout = LineLayout {
            runs,
            font_size,
            width: typographic_bounds.width.into(),
            ascent: max_ascent.into(),
            descent: max_descent.into(),
            len: text.len(),
            interior_carets: Vec::new(),
            isolation: None,
            ink_clip: None,
        };
        let mut carets = layout.interior_caret_candidates(text);
        if !carets.is_empty() {
            // Both wrappers borrow the same immutable Core Text line; it stays
            // alive for all queries. Sorted source offsets need one UTF-16 pass.
            let native_line = unsafe { &*line.as_concrete_TypeRef().cast::<objc2_core_text::CTLine>() };
            let mut converter = StringIndexConverter::new(text);
            for caret in &mut carets {
                converter.advance_to_utf8_ix(caret.index);
                let x = unsafe {
                    native_line.offset_for_string_index(converter.utf16_ix as isize, std::ptr::null_mut())
                } as f32;
                if x.is_finite() {
                    caret.x = px(x);
                } else {
                    log::warn!("Core Text returned a non-finite ligature caret; using its proportional cell position");
                }
            }
        }
        layout.interior_carets = carets;
        layout
    }
}

const MAX_RASTER_SIDE: i64 = 8192;

fn raster_buffer_layout(params: &RenderGlyphParams, size: Size<DevicePixels>) -> Result<(Size<DevicePixels>, usize, usize)> {
    anyhow::ensure!(params.font_size.as_f32().is_finite() && params.font_size.as_f32() > 0.
        && params.scale_factor.is_finite() && params.scale_factor > 0., "invalid glyph raster scale");
    anyhow::ensure!(size.width.0 > 0 && size.height.0 > 0, "glyph bounds are empty");
    let width = i64::from(size.width.0) + i64::from(params.subpixel_variant.x > 0);
    let height = i64::from(size.height.0) + i64::from(params.subpixel_variant.y > 0);
    anyhow::ensure!(width <= MAX_RASTER_SIDE && height <= MAX_RASTER_SIDE, "glyph raster dimensions exceed limit");
    let channels = if params.is_emoji { 4 } else { 1 };
    let stride = (width as usize).checked_mul(channels).ok_or_else(|| anyhow!("glyph raster stride overflow"))?;
    let length = stride.checked_mul(height as usize).ok_or_else(|| anyhow!("glyph raster allocation overflow"))?;
    anyhow::ensure!(length <= 16 * 1024 * 1024, "glyph raster buffer exceeds limit");
    Ok((gpui::size(DevicePixels(width as i32), DevicePixels(height as i32)), stride, length))
}

#[derive(Debug, Clone)]
struct StringIndexConverter<'a> {
    text: &'a str,
    /// Index in UTF-8 bytes
    utf8_ix: usize,
    /// Index in UTF-16 code units
    utf16_ix: usize,
    reversed_lookup: Option<Vec<usize>>,
}

impl<'a> StringIndexConverter<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            utf8_ix: 0,
            utf16_ix: 0,
            reversed_lookup: None,
        }
    }

    fn advance_to_utf16_ix(&mut self, utf16_target: usize) {
        if self.reversed_lookup.is_none() && utf16_target < self.utf16_ix {
            let mut indices = Vec::with_capacity(self.text.encode_utf16().count() + 1);
            for (index, character) in self.text.char_indices() {
                indices.extend(std::iter::repeat_n(index, character.len_utf16()));
            }
            indices.push(self.text.len());
            self.reversed_lookup = Some(indices);
        }
        if let Some(indices) = &self.reversed_lookup {
            self.utf16_ix = utf16_target.min(indices.len() - 1);
            self.utf8_ix = indices[self.utf16_ix];
            return;
        }
        for (ix, c) in self.text[self.utf8_ix..].char_indices() {
            if self.utf16_ix >= utf16_target {
                self.utf8_ix += ix;
                return;
            }
            self.utf16_ix += c.len_utf16();
        }
        self.utf8_ix = self.text.len();
    }

    fn advance_to_utf8_ix(&mut self, utf8_target: usize) {
        for (ix, c) in self.text[self.utf8_ix..].char_indices() {
            if self.utf8_ix + ix >= utf8_target {
                self.utf8_ix += ix;
                return;
            }
            self.utf16_ix += c.len_utf16();
        }
        self.utf8_ix = self.text.len();
    }
}

fn font_kit_metrics_to_metrics(metrics: Metrics) -> FontMetrics {
    FontMetrics {
        units_per_em: metrics.units_per_em,
        ascent: metrics.ascent,
        descent: metrics.descent,
        line_gap: metrics.line_gap,
        underline_position: metrics.underline_position,
        underline_thickness: metrics.underline_thickness,
        cap_height: metrics.cap_height,
        x_height: metrics.x_height,
        bounding_box: bounds_from_rect(metrics.bounding_box),
    }
}

fn bounds_from_rect(rect: RectF) -> Bounds<f32> {
    Bounds {
        origin: point(rect.origin_x(), rect.origin_y()),
        size: size(rect.width(), rect.height()),
    }
}

// impl From<Vector2I> for Size<DevicePixels> {
//     fn from(value: Vector2I) -> Self {
//         size(value.x().into(), value.y().into())
//     }
// }

// impl From<RectI> for Bounds<i32> {
//     fn from(rect: RectI) -> Self {
//         Bounds {
//             origin: point(rect.origin_x(), rect.origin_y()),
//             size: size(rect.width(), rect.height()),
//         }
//     }
// }

// impl From<Point<u32>> for Vector2I {
//     fn from(size: Point<u32>) -> Self {
//         Vector2I::new(size.x as i32, size.y as i32)
//     }
// }

fn size_from_vector2f(vec: Vector2F) -> Size<f32> {
    size(vec.x(), vec.y())
}

fn fontkit_weight(value: FontWeight) -> FontkitWeight {
    FontkitWeight(value.0)
}

fn fontkit_style(style: FontStyle) -> FontkitStyle {
    match style {
        FontStyle::Normal => FontkitStyle::Normal,
        FontStyle::Italic => FontkitStyle::Italic,
        FontStyle::ObliqueAngle(angle) if angle.degrees() == 0. => FontkitStyle::Normal,
        FontStyle::Oblique | FontStyle::ObliqueAngle(_) => FontkitStyle::Oblique,
    }
}

// Some fonts may have no attributes despite `core_text` requiring them (and panicking).
// This is the same version as `core_text` has without `expect` calls.
mod lenient_font_attributes {
    use core_foundation::{
        base::{CFRetain, CFType, TCFType},
        string::{CFString, CFStringRef},
    };
    use core_text::font_descriptor::{
        CTFontDescriptor, CTFontDescriptorCopyAttribute, kCTFontFamilyNameAttribute,
    };

    pub fn family_name(descriptor: &CTFontDescriptor) -> Option<String> {
        unsafe { get_string_attribute(descriptor, kCTFontFamilyNameAttribute) }
    }

    fn get_string_attribute(
        descriptor: &CTFontDescriptor,
        attribute: CFStringRef,
    ) -> Option<String> {
        unsafe {
            let value = CTFontDescriptorCopyAttribute(descriptor.as_concrete_TypeRef(), attribute);
            if value.is_null() {
                return None;
            }

            let value = CFType::wrap_under_create_rule(value);
            assert!(value.instance_of::<CFString>());
            let s = wrap_under_get_rule(value.as_CFTypeRef() as CFStringRef);
            Some(s.to_string())
        }
    }

    unsafe fn wrap_under_get_rule(reference: CFStringRef) -> CFString {
        unsafe {
            assert!(!reference.is_null(), "Attempted to create a NULL object.");
            let reference = CFRetain(reference as *const ::std::os::raw::c_void) as CFStringRef;
            TCFType::wrap_under_create_rule(reference)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::AppleTextSystem;
    use gpui::{
        FontFallbacks, FontFeatures, FontId, FontRun, GlyphId, PlatformTextSystem, font, px,
    };
    use std::sync::Arc;

    #[test]
    fn test_native_font_cache_preserves_run_instances() {
        use core_foundation::base::TCFType;
        use core_text::font::CTFont;

        let original = core_text::font::new_from_name("Helvetica", 16.).unwrap();
        let larger = original.clone_with_font_size(32.);
        let transformed = unsafe {
            let native = &*original.as_concrete_TypeRef().cast::<objc2_core_text::CTFont>();
            let matrix = objc2_core_foundation::CGAffineTransform {
                a: 1., b: 0., c: 0.25, d: 1., tx: 0., ty: 0.,
            };
            let font = native.copy_with_attributes(0., &matrix, None);
            CTFont::wrap_under_create_rule(
                objc2_core_foundation::CFRetained::into_raw(font).as_ptr().cast(),
            )
        };
        let fonts = AppleTextSystem::new();
        let mut state = fonts.0.write();
        let mut ids = Vec::new();
        for requested in [original, larger, transformed] {
            assert_eq!(requested.postscript_name(), "Helvetica");
            let id = state.id_for_native_font(requested.clone(), super::oblique::Context::new(gpui::FontStyle::Normal, 400.));
            assert!(!ids.contains(&id), "distinct native instances must not share a FontId");
            ids.push(id);
            let retained = state.fonts[id.0].source.native_font();
            assert_eq!(retained, requested);
            assert_eq!(retained.pt_size(), requested.pt_size());
            assert_eq!(retained.get_matrix().c, requested.get_matrix().c);
            let equivalent = requested.clone_with_font_size(requested.pt_size());
            assert_eq!(state.id_for_native_font(equivalent, super::oblique::Context::new(gpui::FontStyle::Normal, 400.)), id);
        }
        assert_eq!(state.fonts.len(), 3);
        assert_eq!(state.font_ids_by_native_font.len(), 3);
    }

    #[test]
    fn test_reload_family_with_features_and_fallbacks() {
        let fonts = AppleTextSystem::new();
        let mut font = font("Helvetica");
        let original_id = fonts.font_id(&font).unwrap();
        font.features = FontFeatures::disable_ligatures();
        font.fallbacks = Some(FontFallbacks(Arc::new(vec!["Times".into()])));
        let configured_id = fonts.font_id(&font).unwrap();
        assert_ne!(original_id, configured_id);
        assert_eq!(fonts.font_id(&font).unwrap(), configured_id);
        assert!(fonts.glyph_for_char(configured_id, 'm').is_some());
        let layout = fonts.layout_line(
            "office",
            px(16.),
            &[FontRun { font_size: None,
                font_id: configured_id,
                len: 6,
            }],
        );
        assert_eq!(layout.len, 6);
        assert!(!layout.runs.is_empty());
    }

    #[test]
    fn test_layout_line_sets_a_lone_run_at_its_exact_size() {
        use core_foundation::attributed_string::CFMutableAttributedString;
        use core_foundation::base::{CFRange, TCFType};
        use core_foundation::string::CFString;
        use core_text::line::CTLine;
        use core_text::string_attributes::kCTFontAttributeName;

        let fonts = AppleTextSystem::new();
        let font_id = fonts.font_id(&font(".SystemUIFont")).unwrap();
        let text = "Hamburgefonstiv 0123 Wafer";
        let bom = "\u{feff}";
        let with_bom = format!("{bom}{text}");
        for size in [11., 17., 22.] {
            let exact = fonts.0.read().fonts[font_id.0].source
                .native_font()
                .clone_with_font_size(size as f64);
            let mut string = CFMutableAttributedString::new();
            string.replace_str(&CFString::new(text), CFRange::init(0, 0));
            let range = CFRange::init(0, string.char_len());
            unsafe { string.set_attribute(range, kCTFontAttributeName, &exact) };
            let line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
            let width = line.get_typographic_bounds().width as f32;

            let layout = fonts.layout_line(
                text,
                px(size),
                &[FontRun { font_size: None,
                    font_id,
                    len: text.len(),
                }],
            );
            assert_eq!(f32::from(layout.width), width, "{size} px");
            for run in &layout.runs {
                let retained = fonts.0.read().fonts[run.font_id.0].source.native_font();
                assert_eq!(retained.pt_size(), size as f64);
                assert_eq!(run.font_size.unwrap_or(layout.font_size), px(size));
            }

            // replace_str drops a leading BOM, so its run must not count as the previous run.
            let layout = fonts.layout_line(
                &with_bom,
                px(size),
                &[
                    FontRun { font_size: None,
                        font_id,
                        len: bom.len(),
                    },
                    FontRun { font_size: None,
                        font_id,
                        len: text.len(),
                    },
                ],
            );
            assert_eq!(f32::from(layout.width), width, "{size} px after a BOM run");
        }
    }

    #[test]
    fn test_layout_line_breaks_ligatures_between_runs() {
        let fonts = AppleTextSystem::new();
        fonts
            .add_fonts(vec![
                include_bytes!("../tests/fonts/lilex/Lilex-Regular.ttf")
                    .as_slice()
                    .into(),
            ])
            .unwrap();
        let glyphs = |text: &str, runs: &[(FontId, usize)]| -> Vec<GlyphId> {
            let runs = runs
                .iter()
                .map(|&(font_id, len)| FontRun { font_size: None, font_id, len })
                .collect::<Vec<_>>();
            fonts
                .layout_line(text, px(16.), &runs)
                .runs
                .iter()
                .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.id))
                .collect()
        };

        let lilex = fonts.font_id(&font("Lilex")).unwrap();
        let gt = glyphs(">", &[(lilex, 1)])[0];
        assert_ne!(glyphs(">>>", &[(lilex, 3)]), vec![gt; 3], ">>> as one run");
        assert_eq!(
            glyphs(">>>", &[(lilex, 1), (lilex, 1), (lilex, 1)]),
            vec![gt; 3]
        );
        let mut split = glyphs(">>", &[(lilex, 2)]);
        split.push(gt);
        assert_eq!(glyphs(">>>", &[(lilex, 2), (lilex, 1)]), split);
        let mut split = vec![gt];
        split.extend(glyphs(">>", &[(lilex, 2)]));
        assert_eq!(glyphs(">>>", &[(lilex, 1), (lilex, 2)]), split);

        // Distinct font ids that resolve to the same font.
        let zed_mono = fonts.font_id(&font(".ZedMono")).unwrap();
        let mut no_fallbacks = font("Lilex");
        no_fallbacks.fallbacks = Some(FontFallbacks(Arc::new(Vec::new())));
        let no_fallbacks = fonts.font_id(&no_fallbacks).unwrap();
        for other in [zed_mono, no_fallbacks] {
            assert_ne!(lilex, other);
            assert_eq!(
                glyphs(">>>", &[(lilex, 1), (other, 1), (lilex, 1)]),
                vec![gt; 3]
            );
        }

        // Empty runs between two runs of one font.
        let times = fonts.font_id(&font("Times")).unwrap();
        let helvetica = fonts.font_id(&font("Helvetica")).unwrap();
        let system = fonts.font_id(&font(".SystemUIFont")).unwrap();
        let mut split = glyphs("f", &[(times, 1)]);
        split.extend(glyphs("i", &[(times, 1)]));
        assert_ne!(glyphs("fi", &[(times, 2)]), split, "fi as one run");
        assert_eq!(
            glyphs("fi", &[(times, 1), (helvetica, 0), (system, 0), (times, 1)]),
            split
        );

        // Runs of different fonts that both fall back to Geeza Pro for Arabic.
        assert_eq!(glyphs("لا", &[(helvetica, 4)]).len(), 1, "لا as one run");
        assert_eq!(glyphs("لا", &[(helvetica, 2), (times, 2)]).len(), 2);
    }

    #[test]
    fn test_layout_line_bom_char() {
        let fonts = AppleTextSystem::new();
        let font_id = fonts.font_id(&font("Helvetica")).unwrap();
        let line = "\u{feff}";
        let mut style = FontRun { font_size: None,
            font_id,
            len: line.len(),
        };

        let layout = fonts.layout_line(line, px(16.), &[style]);
        assert_eq!(layout.len, line.len());
        assert!(layout.runs.is_empty());

        let line = "a\u{feff}b";
        style.len = line.len();
        let layout = fonts.layout_line(line, px(16.), &[style]);
        assert_eq!(layout.len, line.len());
        assert_eq!(layout.runs.len(), 1);
        assert_eq!(layout.runs[0].glyphs.len(), 2);
        assert_eq!(layout.runs[0].glyphs[0].id, GlyphId(68u32)); // a
        // There's no glyph for \u{feff}
        assert_eq!(layout.runs[0].glyphs[1].id, GlyphId(69u32)); // b

        let line = "\u{feff}ab";
        let font_runs = &[
            FontRun { font_size: None,
                len: "\u{feff}".len(),
                font_id,
            },
            FontRun { font_size: None,
                len: "ab".len(),
                font_id,
            },
        ];
        let layout = fonts.layout_line(line, px(16.), font_runs);
        assert_eq!(layout.len, line.len());
        assert_eq!(layout.runs.len(), 1);
        assert_eq!(layout.runs[0].glyphs.len(), 2);
        // There's no glyph for \u{feff}
        assert_eq!(layout.runs[0].glyphs[0].id, GlyphId(68u32)); // a
        assert_eq!(layout.runs[0].glyphs[1].id, GlyphId(69u32)); // b
    }

    #[test]
    fn test_layout_line_zwnj_insertion() {
        let fonts = AppleTextSystem::new();
        let font_id = fonts.font_id(&font("Helvetica")).unwrap();

        let text = "hello world";
        let font_runs = &[
            FontRun { font_size: None, font_id, len: 5 }, // "hello"
            FontRun { font_size: None, font_id, len: 6 }, // " world"
        ];

        let layout = fonts.layout_line(text, px(16.), font_runs);
        assert_eq!(layout.len, text.len());

        for run in &layout.runs {
            for glyph in &run.glyphs {
                assert!(
                    glyph.index < text.len(),
                    "Glyph index {} is out of bounds for text length {}",
                    glyph.index,
                    text.len()
                );
            }
        }

        // Test with different font runs - should not insert ZWNJ
        let font_id2 = fonts.font_id(&font("Times")).unwrap_or(font_id);
        let font_runs_different = &[
            FontRun { font_size: None, font_id, len: 5 }, // "hello"
            // " world"
            FontRun { font_size: None,
                font_id: font_id2,
                len: 6,
            },
        ];

        let layout2 = fonts.layout_line(text, px(16.), font_runs_different);
        assert_eq!(layout2.len, text.len());

        for run in &layout2.runs {
            for glyph in &run.glyphs {
                assert!(
                    glyph.index < text.len(),
                    "Glyph index {} is out of bounds for text length {}",
                    glyph.index,
                    text.len()
                );
            }
        }
    }

    #[test]
    fn test_layout_line_zwnj_edge_cases() {
        let fonts = AppleTextSystem::new();
        let font_id = fonts.font_id(&font("Helvetica")).unwrap();

        let text = "hello";
        let font_runs = &[FontRun { font_size: None, font_id, len: 5 }];
        let layout = fonts.layout_line(text, px(16.), font_runs);
        assert_eq!(layout.len, text.len());

        let text = "abc";
        let font_runs = &[
            FontRun { font_size: None, font_id, len: 1 }, // "a"
            FontRun { font_size: None, font_id, len: 1 }, // "b"
            FontRun { font_size: None, font_id, len: 1 }, // "c"
        ];
        let layout = fonts.layout_line(text, px(16.), font_runs);
        assert_eq!(layout.len, text.len());

        for run in &layout.runs {
            for glyph in &run.glyphs {
                assert!(
                    glyph.index < text.len(),
                    "Glyph index {} is out of bounds for text length {}",
                    glyph.index,
                    text.len()
                );
            }
        }

        // Test with empty text
        let text = "";
        let font_runs = &[];
        let layout = fonts.layout_line(text, px(16.), font_runs);
        assert_eq!(layout.len, 0);
        assert!(layout.runs.is_empty());
    }
}
