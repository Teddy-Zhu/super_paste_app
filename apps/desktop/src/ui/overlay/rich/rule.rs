use gpui_kit::{Hsla, Rgba};
use gpui_kit::base::text::editor_host::RuleStyle;

/// The reference UA paints HR as a one-pixel inset border, not the theme's
/// Markdown hairline. Resolve its two tones once during document adaptation.
/// Algorithm reference: Chromium box_border_painter.cc and Color::Light/Dark.
pub(super) fn appearance(color: Hsla) -> RuleStyle {
    let color = Rgba::from(color);
    let linear = |channel: f32| if channel <= 0.04045 { channel / 12.92 }
        else { ((channel + 0.055) / 1.055).powf(2.4) };
    let luminance = 0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b);
    let (dark, light) = if luminance <= 0.014443844 {
        let dark = tone(color, true);
        (dark, tone(dark, true))
    } else {
        (tone(color, false), if luminance > 0.83077 { color } else { tone(color, true) })
    };
    RuleStyle::Inset { dark: dark.into(), light: light.into() }
}

fn tone(color: Rgba, lighter: bool) -> Rgba {
    let peak = color.r.max(color.g).max(color.b);
    if peak == 0.0 && lighter { return Rgba { r: 84.0 / 255.0, g: 84.0 / 255.0, b: 84.0 / 255.0, a: color.a }; }
    let factor = if peak == 0.0 { 0.0 } else if lighter { (peak + 0.33).min(1.0) / peak }
        else { (peak - 0.33).max(0.0) / peak };
    let quantize = |channel: f32| (channel * factor * 255.0).round().clamp(0.0, 255.0) / 255.0;
    Rgba { r: quantize(color.r), g: quantize(color.g), b: quantize(color.b), a: color.a }
}
