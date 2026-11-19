// SPDX-License-Identifier: MIT OR Apache-2.0

use harfrust::Shaper;
use linebender_resource_handle::{Blob, FontData};
use skrifa::raw::{ReadError, TableProvider as _};
use skrifa::{metrics::Metrics, prelude::*};
// re-export skrifa
pub use skrifa;
// re-export peniko::Font;
#[cfg(feature = "peniko")]
pub use linebender_resource_handle::FontData as PenikoFont;

use core::fmt;

use alloc::sync::Arc;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use fontdb::Style;
use crate::ObliqueAngle;
use self_cell::self_cell;

pub mod fallback;
pub use fallback::{Fallback, PlatformFallback};

pub use self::system::*;
mod system;

struct OwnedFaceData {
    data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    shaper_data: harfrust::ShaperData,
    shaper_instance: harfrust::ShaperInstance,
    metrics: Metrics,
}

self_cell!(
    struct OwnedFace {
        owner: OwnedFaceData,

        #[covariant]
        dependent: Shaper,
    }
);

struct FontMonospaceFallback {
    monospace_em_width: Option<f32>,
    scripts: Vec<[u8; 4]>,
    unicode_codepoints: Vec<u32>,
}

#[derive(Clone, Copy)]
struct StyleAxis { min: f32, default: f32, max: f32 }

impl StyleAxis {
    fn contains(self, value: f32) -> bool { value >= self.min && value <= self.max }
    fn clamp(self, value: f32) -> f32 { value.clamp(self.min, self.max) }
}

#[derive(Clone, Copy)]
struct StyleAxes { italic: Option<StyleAxis>, slant: Option<StyleAxis> }

/// Ordered CSS matching priority for an authored oblique request.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ObliqueMatch {
    phase: u8,
    distance: u32,
}

impl StyleAxes {
    fn new(font: &FontRef<'_>) -> Option<Self> {
        let axis = |tag| -> Option<Option<StyleAxis>> {
            let Some(axis) = font.axes().get_by_tag(tag) else { return Some(None); };
            let (min, default, max) = (axis.min_value(), axis.default_value(), axis.max_value());
            (min.is_finite() && default.is_finite() && max.is_finite() && min <= default && default <= max)
                .then_some(Some(StyleAxis { min, default, max }))
        };
        Some(Self { italic: axis(Tag::new(b"ital"))?, slant: axis(Tag::new(b"slnt"))? })
    }

    fn values(self, style: Style, angle: Option<ObliqueAngle>) -> (f32, f32) {
        let italic = if style == Style::Italic { 1. } else { 0. };
        // A real italic axis owns its designed slant. Otherwise italic may use
        // the same oblique fallback already supported by the shaping API.
        let slant = match style {
            Style::Normal => 0.,
            Style::Italic if self.italic.is_some_and(|axis| axis.contains(1.)) =>
                self.slant.map_or(0., |axis| axis.default),
            Style::Italic => -14.,
            Style::Oblique => -angle.map_or(14., ObliqueAngle::degrees),
        };
        (self.italic.map_or(0., |axis| axis.clamp(italic)),
            self.slant.map_or(0., |axis| axis.clamp(slant)))
    }

    fn supports(self, requested: Style, native: Style, angle: Option<ObliqueAngle>) -> bool {
        match requested {
            Style::Normal =>
                (native != Style::Italic || self.italic.is_some())
                && (native != Style::Oblique || self.slant.is_some())
                && self.italic.is_none_or(|axis| axis.contains(0.))
                && self.slant.is_none_or(|axis| axis.contains(0.)),
            Style::Italic => native == Style::Italic
                || self.italic.is_some_and(|axis| axis.contains(1.))
                || self.slant.is_some_and(|axis| axis.contains(-14.)),
            Style::Oblique => native == Style::Oblique
                || self.slant.is_some_and(|axis| axis.contains(-angle.map_or(14., ObliqueAngle::degrees))),
        }
    }

    fn effective_style(self, style: Style, native: Style, angle: Option<ObliqueAngle>) -> Style {
        let (italic, slant) = self.values(style, angle);
        if italic != 0. || (native == Style::Italic && self.italic.is_none()) { Style::Italic }
        else if slant != 0. || (native == Style::Oblique && self.slant.is_none()) { Style::Oblique }
        else { Style::Normal }
    }

    fn oblique_match(self, requested: ObliqueAngle, native: Style, static_angle: f32) -> ObliqueMatch {
        let requested = requested.degrees();
        let direction = if requested < 0. { -1. } else { 1. };
        let desired = requested.abs();
        let immutable_italic = native == Style::Italic && self.italic.is_none_or(|axis| !axis.contains(0.));
        // Preserve GPUI's shared italic/oblique matching scale. CSS Fonts 4
        // permits it and maps italic strength 1 to an 11-degree oblique.
        let actual = if immutable_italic { 11. } else if let Some(axis) = self.slant {
            -axis.clamp(-requested)
        } else if native != Style::Normal && !(native == Style::Italic && !immutable_italic) {
            static_angle
        } else { 0. } * direction;
        let (phase, distance) = if actual == desired {
            (0, 0.)
        } else if actual > 0. {
            if (desired >= 11. && actual > desired) || (desired < 11. && actual < desired) {
                (0, (actual - desired).abs())
            } else { (1, (actual - desired).abs()) }
        } else if actual == 0. {
            // Synthesis from an upright face precedes reverse-direction
            // fallbacks after matching the requested side of the scale.
            (2, 0.)
        } else { (4, actual.abs()) };
        ObliqueMatch { phase, distance: distance.to_bits() }
    }
}

/// A font
pub struct Font {
    #[cfg(feature = "swash")]
    swash: (u32, swash::CacheKey),
    harfrust: OwnedFace,
    data: FontData,
    id: fontdb::ID,
    monospace_fallback: Option<FontMonospaceFallback>,
    pub(crate) italic_or_oblique: bool,
    native_style: Style,
    effective_style: Style,
    style_axes: StyleAxes,
    static_angle: f32,
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Font {
    /// Whether this instance already has a native italic or oblique shape.
    pub fn is_italic_or_oblique(&self) -> bool { self.italic_or_oblique }

    /// The native style after applying this instance's variation axes.
    pub fn effective_style(&self) -> Style { self.effective_style }

    /// Whether the face or its variation axes cover the requested style.
    pub fn supports_style(&self, style: Style) -> bool { self.supports_style_with_angle(style, None) }

    pub fn supports_style_with_angle(&self, style: Style, angle: Option<ObliqueAngle>) -> bool {
        self.style_axes.supports(style, self.native_style, angle)
    }

    /// Match native slant ranges before synthesis and italic fallbacks.
    pub fn oblique_match(&self, angle: ObliqueAngle) -> ObliqueMatch {
        self.style_axes.oblique_match(angle, self.native_style, self.static_angle)
    }

    // Equal axis locations share the existing instance rather than retaining
    // another shaper merely for a synthetic style handled by the renderer.
    pub(crate) fn instance_style(&self, style: Style, angle: Option<ObliqueAngle>) -> (Style, Option<ObliqueAngle>) {
        let values = self.style_axes.values(style, angle);
        for equivalent in [self.native_style, Style::Normal, Style::Italic, Style::Oblique] {
            if values == self.style_axes.values(equivalent, None) { return (equivalent, None); }
        }
        // Clamp once in the instance identity as well as in its coordinates.
        // Requests outside a native axis range share the same endpoint shaper.
        let angle = if style == Style::Oblique && self.style_axes.slant.is_some() {
            ObliqueAngle::new(-values.1).or(angle)
        } else { angle };
        (style, angle)
    }

    pub const fn id(&self) -> fontdb::ID {
        self.id
    }

    pub fn monospace_em_width(&self) -> Option<f32> {
        self.monospace_fallback
            .as_ref()
            .and_then(|x| x.monospace_em_width)
    }

    pub fn scripts(&self) -> &[[u8; 4]] {
        self.monospace_fallback.as_ref().map_or(&[], |x| &x.scripts)
    }

    pub fn unicode_codepoints(&self) -> &[u32] {
        self.monospace_fallback
            .as_ref()
            .map_or(&[], |x| &x.unicode_codepoints)
    }

    pub fn data(&self) -> &[u8] {
        self.data.data.data()
    }

    pub fn shaper(&self) -> &harfrust::Shaper<'_> {
        self.harfrust.borrow_dependent()
    }

    pub(crate) fn shaper_instance(&self) -> &harfrust::ShaperInstance {
        &self.harfrust.borrow_owner().shaper_instance
    }

    pub fn metrics(&self) -> &Metrics {
        &self.harfrust.borrow_owner().metrics
    }

    #[cfg(feature = "peniko")]
    pub fn as_peniko(&self) -> PenikoFont {
        self.data.clone()
    }

    #[cfg(feature = "swash")]
    pub fn as_swash(&self) -> swash::FontRef<'_> {
        let swash = &self.swash;
        swash::FontRef {
            data: self.data(),
            offset: swash.0,
            key: swash.1,
        }
    }
}

impl Font {
    pub fn new(db: &fontdb::Database, id: fontdb::ID, weight: fontdb::Weight) -> Option<Self> {
        Self::new_with_style(db, id, weight, db.face(id)?.style, None)
    }

    fn new_with_style(db: &fontdb::Database, id: fontdb::ID, weight: fontdb::Weight, style: Style, angle: Option<ObliqueAngle>) -> Option<Self> {
        let info = db.face(id)?;

        let data = match &info.source {
            fontdb::Source::Binary(data) => Arc::clone(data),
            #[cfg(feature = "std")]
            fontdb::Source::File(path) => {
                log::warn!("Unsupported fontdb Source::File('{}')", path.display());
                return None;
            }
            #[cfg(feature = "std")]
            fontdb::Source::SharedFile(_path, data) => Arc::clone(data),
        };

        // It's a bit unfortunate but we need to parse the data into a `FontRef`
        // twice--once to construct the HarfRust `ShaperInstance` and
        // `ShaperData`, and once to create the persistent `FontRef` tied to the
        // lifetime of the face data.
        let font_ref = FontRef::from_index((*data).as_ref(), info.index).ok()?;
        let style_axes = StyleAxes::new(&font_ref)?;
        let static_angle = font_ref.post().ok().map(|post| -post.italic_angle().to_f64() as f32)
            .filter(|angle| angle.is_finite() && *angle != 0. && angle.abs() <= 90.).unwrap_or(14.);
        let (italic, slant) = style_axes.values(style, angle);
        let location = font_ref
            .axes()
            .location([(Tag::new(b"wght"), weight.0 as f32), (Tag::new(b"ital"), italic), (Tag::new(b"slnt"), slant)]);
        let metrics = font_ref.metrics(Size::unscaled(), &location);

        let monospace_fallback = if cfg!(feature = "monospace_fallback") {
            (|| {
                let glyph_metrics = font_ref.glyph_metrics(Size::unscaled(), &location);
                let charmap = font_ref.charmap();
                let monospace_em_width = info
                    .monospaced
                    .then(|| {
                        let hor_advance = glyph_metrics.advance_width(charmap.map(' ')?)?;
                        let upem = metrics.units_per_em;
                        Some(hor_advance / f32::from(upem))
                    })
                    .flatten();

                if info.monospaced && monospace_em_width.is_none() {
                    None?;
                }

                let scripts = font_ref
                    .gpos()
                    .ok()?
                    .script_list()
                    .ok()?
                    .script_records()
                    .iter()
                    .chain(
                        font_ref
                            .gsub()
                            .ok()?
                            .script_list()
                            .ok()?
                            .script_records()
                            .iter(),
                    )
                    .map(|script| script.script_tag().into_bytes())
                    .collect();

                let mut unicode_codepoints = Vec::new();

                for (code_point, _) in charmap.mappings() {
                    unicode_codepoints.push(code_point);
                }

                unicode_codepoints.shrink_to_fit();

                Some(FontMonospaceFallback {
                    monospace_em_width,
                    scripts,
                    unicode_codepoints,
                })
            })()
        } else {
            None
        };

        let (shaper_instance, shaper_data) = {
            (
                harfrust::ShaperInstance::from_coords(&font_ref, location.coords().iter().copied()),
                harfrust::ShaperData::new(&font_ref),
            )
        };

        Some(Self {
            id: info.id,
            monospace_fallback,
            #[cfg(feature = "swash")]
            swash: {
                let swash = swash::FontRef::from_index((*data).as_ref(), info.index as usize)?;
                (swash.offset, swash.key)
            },
            harfrust: OwnedFace::try_new(
                OwnedFaceData {
                    data: Arc::clone(&data),
                    shaper_data,
                    shaper_instance,
                    metrics,
                },
                |OwnedFaceData {
                     data,
                     shaper_data,
                     shaper_instance,
                     ..
                 }| {
                    let font_ref = FontRef::from_index((**data).as_ref(), info.index)?;
                    let shaper = shaper_data
                        .shaper(&font_ref)
                        .instance(Some(shaper_instance))
                        .build();
                    Ok::<_, ReadError>(shaper)
                },
            )
            .ok()?,
            data: FontData::new(Blob::new(data), info.index),
            italic_or_oblique: style_axes.effective_style(style, info.style, angle) != Style::Normal,
            native_style: info.style,
            effective_style: style_axes.effective_style(style, info.style, angle),
            style_axes,
            static_angle,
        })
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_fonts_load_time() {
        use crate::FontSystem;
        use sys_locale::get_locale;

        #[cfg(not(target_arch = "wasm32"))]
        let now = std::time::Instant::now();

        let mut db = fontdb::Database::new();
        let locale = get_locale().expect("Local available");
        db.load_system_fonts();
        FontSystem::new_with_locale_and_db(locale, db);

        #[cfg(not(target_arch = "wasm32"))]
        println!("Fonts load time {}ms.", now.elapsed().as_millis());
    }
}
