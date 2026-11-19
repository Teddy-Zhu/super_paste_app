use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::{CFDictionary, CFMutableDictionary},
    number::CFNumber,
    string::CFString,
};
use core_text::{font::CTFont, font_descriptor::{kCTFontBoldTrait, kCTFontColorGlyphsTrait, kCTFontItalicTrait}};
use font_kit::font::Font as FontKitFont;
use gpui::{FontObliqueAngle, FontRun, FontStyle};
use objc2_core_foundation::CFRetained;
use smallvec::SmallVec;

#[cfg(test)]
mod tests;

const SLANT: i32 = i32::from_be_bytes(*b"slnt");
const ITALIC: i32 = i32::from_be_bytes(*b"ital");
const WEIGHT: i32 = i32::from_be_bytes(*b"wght");

/// Logical source properties survive fallback even when distinct requests
/// select the same physical face. Store finite CSS weights by exact bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Context { pub style: FontStyle, weight_bits: u32 }
impl Context {
    pub fn new(style: FontStyle, weight: f32) -> Self { Self { style, weight_bits: weight.to_bits() } }
    pub fn weight(self) -> f32 { f32::from_bits(self.weight_bits) }
    pub fn for_font(font: &FontKitFont) -> Self {
        use font_kit::properties::Style;
        let properties = font.properties();
        let style = match properties.style { Style::Normal => FontStyle::Normal, Style::Italic => FontStyle::Italic, Style::Oblique => FontStyle::Oblique };
        Self::new(style, properties.weight.0)
    }
}

// Logical-pixel offset relative to an em; shared by bounds, ink and raster.
const BOLD_OFFSET_EM: f64 = 1. / 36.;

pub(super) fn angle(style: FontStyle) -> Option<FontObliqueAngle> {
    match style {
        FontStyle::Italic | FontStyle::Oblique => Some(FontObliqueAngle::try_from(14.).expect("valid default oblique angle")),
        FontStyle::ObliqueAngle(angle) => Some(angle),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Axis { minimum: f64, maximum: f64, default: f64 }

fn key(value: &objc2_core_foundation::CFString) -> CFString {
    // Both binding versions describe the same immutable CF object. Retain it
    // through the older wrapper used by font-kit, without transferring ownership.
    unsafe { CFString::wrap_under_get_rule(std::ptr::from_ref(value).cast()) }
}

fn number(axis: &CFDictionary<CFString, CFType>, name: &objc2_core_foundation::CFString) -> Option<f64> {
    axis.find(&key(name))?.downcast::<CFNumber>()?.to_f64()
}

fn axes(font: &CTFont) -> (Option<Axis>, Option<Axis>, Option<Axis>) {
    let mut slant = None;
    let mut italic = None;
    let mut weight = None;
    unsafe {
        let native = &*font.as_concrete_TypeRef().cast::<objc2_core_text::CTFont>();
        let Some(values) = native.variation_axes() else { return (None, None, None); };
        let values = CFArray::<CFDictionary<CFString, CFType>>::wrap_under_get_rule(
            CFRetained::as_ptr(&values).as_ptr().cast(),
        );
        for value in values.iter() {
            let Some(identifier) = number(&value, objc2_core_text::kCTFontVariationAxisIdentifierKey) else { continue; };
            let target = if identifier == f64::from(SLANT) { &mut slant }
                else if identifier == f64::from(ITALIC) { &mut italic }
                else if identifier == f64::from(WEIGHT) { &mut weight } else { continue; };
            let (Some(minimum), Some(maximum), Some(default)) = (
                number(&value, objc2_core_text::kCTFontVariationAxisMinimumValueKey),
                number(&value, objc2_core_text::kCTFontVariationAxisMaximumValueKey),
                number(&value, objc2_core_text::kCTFontVariationAxisDefaultValueKey),
            ) else { continue; };
            if minimum.is_finite() && maximum.is_finite() && default.is_finite()
                && minimum < maximum && (minimum..=maximum).contains(&default) {
                *target = Some(Axis { minimum, maximum, default });
            }
        }
    }
    (slant, italic, weight)
}

pub(super) fn matching_weight(font: &CTFont, requested: f32, actual: f32) -> f32 {
    axes(font).2.map_or(actual, |axis| f64::from(requested).clamp(axis.minimum, axis.maximum) as f32)
}

pub(super) fn matching_properties(font: &FontKitFont, context: Context) -> font_kit::properties::Properties {
    use font_kit::properties::Style;
    let mut properties = font.properties();
    let (slant, italic, weight) = axes(&font.native_font());
    let contains = |axis: Option<Axis>, value| axis.is_some_and(|axis| (axis.minimum..=axis.maximum).contains(&value));
    if let Some(axis) = weight {
        properties.weight.0 = f64::from(context.weight()).clamp(axis.minimum, axis.maximum) as f32;
    }
    match context.style {
        FontStyle::Normal if (slant.is_some() || italic.is_some())
            && slant.is_none_or(|axis| (axis.minimum..=axis.maximum).contains(&0.))
            && italic.is_none_or(|axis| (axis.minimum..=axis.maximum).contains(&0.)) => properties.style = Style::Normal,
        FontStyle::Italic if contains(italic, 1.) => properties.style = Style::Italic,
        FontStyle::Oblique | FontStyle::ObliqueAngle(_) if slant.is_some() => properties.style = Style::Oblique,
        FontStyle::Oblique | FontStyle::ObliqueAngle(_) if contains(italic, 1.) => properties.style = Style::Italic,
        _ => {}
    }
    properties
}

pub(super) fn synthetic_bold(font: &FontKitFont, context: Context) -> bool {
    if context.weight() < 600. { return false; }
    let native = font.native_font();
    if native.symbolic_traits() & (kCTFontBoldTrait | kCTFontColorGlyphsTrait) != 0 { return false; }
    matching_weight(&native, context.weight(), font.properties().weight.0) < 600.
}

pub(super) fn bold_offset(synthetic: bool, size: f64) -> f64 {
    if synthetic { size * BOLD_OFFSET_EM } else { 0. }
}

/// Choose native axes before shaping. Copy the existing variation dictionary
/// so unrelated axes survive replacement of the descriptor's variation value.
pub(super) fn with_axes(font: &CTFont, context: Context) -> CTFont {
    let (slant, italic, weight) = axes(font);
    let mut desired = SmallVec::<[(i32, f64, f64); 3]>::new();
    let mut set = |tag, axis: Axis, value: f64| desired.push((tag, value.clamp(axis.minimum, axis.maximum), axis.default));
    if let Some(axis) = weight { set(WEIGHT, axis, f64::from(context.weight())); }
    if let Some(angle) = angle(context.style) {
        if let Some(axis) = italic.filter(|_| context.style == FontStyle::Italic) {
            set(ITALIC, axis, 1.);
            if let Some(axis) = slant { set(SLANT, axis, 0.); }
        } else if let Some(axis) = slant {
            set(SLANT, axis, -f64::from(angle.degrees()));
            if let Some(axis) = italic { set(ITALIC, axis, 0.); }
        } else if let Some(axis) = italic {
            set(ITALIC, axis, if angle.degrees() == 0. { 0. } else { 1. });
        }
    } else {
        if let Some(axis) = italic { set(ITALIC, axis, 0.); }
        if let Some(axis) = slant { set(SLANT, axis, 0.); }
    }
    if desired.is_empty() { return font.clone(); }
    unsafe {
        let native = &*font.as_concrete_TypeRef().cast::<objc2_core_text::CTFont>();
        let current = native.variation().map(|value| CFDictionary::<CFNumber, CFType>::wrap_under_get_rule(
            CFRetained::as_ptr(&value).as_ptr().cast(),
        ));
        if desired.iter().all(|(identifier, target, default)| {
            let number = CFNumber::from(*identifier);
            let value = current.as_ref().and_then(|values| values.find(&number));
            let value = value.map(|value| value.downcast::<CFNumber>().and_then(|value| value.to_f64()))
                .unwrap_or(Some(*default));
            value == Some(*target)
        }) { return font.clone(); }
        let mut variations = current.as_ref().map_or_else(CFMutableDictionary::<CFNumber, CFType>::new, |current| {
            CFMutableDictionary::wrap_under_create_rule(core_foundation_sys::dictionary::CFDictionaryCreateMutableCopy(
                core_foundation_sys::base::kCFAllocatorDefault, 0, current.as_concrete_TypeRef(),
            ))
        });
        for (identifier, value, _) in desired {
            variations.set(CFNumber::from(identifier), CFNumber::from(value).as_CFType());
        }
        let attributes = CFDictionary::from_CFType_pairs(&[(key(objc2_core_text::kCTFontVariationAttribute), variations.as_CFType())]);
        let descriptor = native.font_descriptor().copy_with_attributes(
            &*attributes.as_concrete_TypeRef().cast::<objc2_core_foundation::CFDictionary>(),
        );
        let copy = native.copy_with_attributes(0., std::ptr::null(), Some(&descriptor));
        CTFont::wrap_under_create_rule(CFRetained::into_raw(copy).as_ptr().cast())
    }
}

/// Synthetic slant is a drawing instance only. Core Text keeps shaping the
/// original face, so glyph advances, caret positions and ligatures are intact.
/// All drawing metrics, outlines and pixels use this same native transform.
fn needs_slant(font: &CTFont, style: FontStyle) -> bool {
    if angle(style).is_none_or(|angle| angle.degrees() == 0.) { return false; }
    let matrix = font.get_matrix();
    let native = unsafe { &*font.as_concrete_TypeRef().cast::<objc2_core_text::CTFont>() };
    let (slant, italic, _) = axes(font);
    (font.symbolic_traits() & kCTFontItalicTrait) == 0 && unsafe { native.slant_angle() } == 0.
        && matrix.b == 0. && matrix.c == 0. && slant.is_none() && italic.is_none()
}

/// A synthetic source face does not advertise its requested style to Core
/// Text's fallback matching. Prefer a real sibling of that fallback family
/// before applying our own synthesis; Core Text retains the native attributes.
pub(super) fn with_fallback_face(font: &CTFont, context: Context) -> CTFont {
    let mut selected = with_axes(font, context);
    // Style precedes weight in family matching. If bold italic is unavailable,
    // keep a real italic and synthesize weight, rather than losing its style.
    if needs_slant(&selected, context.style) {
        if let Some(face) = selected.clone_with_symbolic_traits(kCTFontItalicTrait, kCTFontItalicTrait) {
            selected = with_axes(&face, context);
        }
    }
    if context.weight() >= 600. {
        let native = unsafe { FontKitFont::from_native_font(&selected) };
        if synthetic_bold(&native, context) {
            let italic = selected.symbolic_traits() & kCTFontItalicTrait;
            if let Some(face) = selected.clone_with_symbolic_traits(kCTFontBoldTrait | italic, kCTFontBoldTrait | kCTFontItalicTrait) {
                selected = with_axes(&face, context);
            }
        }
    }
    selected
}

pub(super) fn drawing_font(font: &CTFont, style: FontStyle) -> Option<CTFont> {
    if !needs_slant(font, style) { return None; }
    let angle = angle(style)?;
    let matrix = font.get_matrix();
    let native = unsafe { &*font.as_concrete_TypeRef().cast::<objc2_core_text::CTFont>() };
    let shear = f64::from(angle.degrees()).to_radians().tan();
    // Apply the requested slant in the existing font's drawing space. In
    // particular, a nonuniform native scale must not change that angle.
    let matrix = objc2_core_foundation::CGAffineTransform {
        a: matrix.a + shear * matrix.b, b: matrix.b,
        c: matrix.c + shear * matrix.d, d: matrix.d,
        tx: matrix.tx + shear * matrix.ty, ty: matrix.ty,
    };
    unsafe {
        let copy = native.copy_with_attributes(0., &matrix, None);
        Some(CTFont::wrap_under_create_rule(CFRetained::into_raw(copy).as_ptr().cast()))
    }
}

/// Visual runs may arrive in either direction. Borrow source font/size data
/// with one small boundary index instead of rescanning for every glyph run.
pub(super) struct SourceRuns<'a> { runs: &'a [FontRun], ends: SmallVec<[usize; 4]> }
impl<'a> SourceRuns<'a> {
    pub fn new(runs: &'a [FontRun]) -> Self {
        let mut ends = SmallVec::new();
        if runs.len() > 1 {
            let mut end = 0;
            for run in runs { end += run.len; ends.push(end); }
        }
        Self { runs, ends }
    }
    pub fn at(&self, index: usize) -> Option<&FontRun> {
        if self.ends.is_empty() { return self.runs.first().filter(|run| index < run.len); }
        self.runs.get(self.ends.partition_point(|end| *end <= index))
    }
}
