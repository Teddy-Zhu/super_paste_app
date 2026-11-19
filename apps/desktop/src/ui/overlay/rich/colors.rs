use gpui_kit::{Hsla, Rgba, rgb};

/// Copied page colors may depend on a missing backdrop or an unsupported
/// gradient. Preserve readable colors; repair invisible text on the backdrop
/// that the native preview actually paints. The source document stays intact.
pub(super) fn readable(foreground: Hsla, background: Rgba) -> Hsla {
    if contrast(background.blend(foreground.into()), background) >= 4.5 { return foreground; }
    let ink = rgb(0x334155);
    if contrast(ink, background) >= 4.5 { return ink.into(); }
    let black = rgb(0x000000);
    let white = rgb(0xffffff);
    if contrast(black, background) >= contrast(white, background) { black.into() } else { white.into() }
}

fn luminance(color: Rgba) -> f32 {
    let linear = |channel: f32| if channel <= 0.04045 { channel / 12.92 }
        else { ((channel + 0.055) / 1.055).powf(2.4) };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::overlay::rich::{CssColor, Inherited};

    #[test]
    fn copied_colors_remain_readable_on_the_actual_composited_backdrop() {
        for (foreground, background) in [
            (rgb(0xffffff).into(), rgb(0xffffff)),
            (rgb(0x000000).into(), rgb(0x000000)),
            (gpui_kit::rgba(0x33415510).into(), rgb(0xffffff)),
            (rgb(0xffffff).into(), rgb(0xffffff).blend(gpui_kit::rgba(0x00000050))),
        ] {
            let corrected = readable(foreground, background);
            assert!(contrast(background.blend(corrected.into()), background) >= 4.5);
        }
        let white: Hsla = rgb(0xffffff).into();
        assert_eq!(readable(white, rgb(0x102030)), white, "white text on a painted dark background must stay white");
        let blue: Hsla = rgb(0x003399).into();
        assert_eq!(readable(blue, rgb(0xffffff)), blue, "readable authored colors must survive");
    }

    #[test]
    fn paint_correction_does_not_change_currentcolor_backgrounds() {
        let mut inherited = Inherited::default();
        inherited.authored_foreground = rgb(0xffffff).into();
        inherited.background = Some(CssColor::Current);
        inherited.mark.foreground = Some(readable(inherited.authored_foreground, inherited.painted_background));
        assert_ne!(inherited.mark.foreground, Some(inherited.authored_foreground));
        assert_eq!(inherited.background_color(), Some(inherited.authored_foreground));
    }
}
