//! Parse CSS color expressions here; color conversion, missing components, hue
//! fixup and alpha premultiplication belong to the color library.
use color::{AlphaColor, ColorSpaceTag, DynamicColor, Flags, HueDirection, Missing, Srgb};
use cssparser::{ParseError, Parser, Token};

mod relative;
use relative::Relative;

type Result<'i, T> = std::result::Result<T, ParseError<'i, ()>>;

/// A parsed computed color, shared with the controlled document.
/// Constants are folded without clipping; currentColor resolves at paint use.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedColor(Expression);

#[derive(Clone, Debug, PartialEq)]
enum Expression { Literal(DynamicColor), Current, Mix(Box<Mix>), Relative(Box<Relative>) }

#[derive(Clone, Debug, PartialEq)]
struct Mix {
    colors: [Expression; 2],
    space: ColorSpaceTag,
    hue: HueDirection,
    fraction: f32,
    alpha: f32,
}

impl ComputedColor {
    /// Unpremultiplied sRGB for the native renderer, with final gamut clipping.
    /// Input currentColor is already the consuming element's computed color.
    pub fn resolve(&self, current: [f32; 4]) -> [f32; 4] {
        self.0.resolve(DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new(current)))
            .convert(ColorSpaceTag::Srgb).clip().to_alpha_color::<Srgb>().components
    }

    pub(in crate::preview::rich) fn weight(&self) -> u64 {
        (std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>()) as u64 + self.0.allocated_weight()
    }
}

impl Expression {
    fn resolve(&self, current: DynamicColor) -> DynamicColor {
        match self {
            Self::Literal(value) => *value,
            Self::Current => current,
            Self::Mix(value) => value.resolve(current),
            Self::Relative(value) => value.resolve(current),
        }
    }

    fn allocated_weight(&self) -> u64 {
        match self {
            Self::Mix(value) => std::mem::size_of::<Mix>() as u64 + value.colors.iter().map(Self::allocated_weight).sum::<u64>(),
            Self::Relative(value) => value.allocated_weight(),
            _ => 0,
        }
    }
}

impl Mix {
    fn resolve(&self, current: DynamicColor) -> DynamicColor {
        let mut left = self.colors[0].resolve(current);
        let mut right = self.colors[1].resolve(current);
        // Color 0.3.3 fills an absent alpha but leaves its missing flag set,
        // so that endpoint then skips premultiplication. Alpha is analogous
        // in every space: complete it before delegating interpolation.
        let left_missing = left.flags.missing().contains(3);
        let right_missing = right.flags.missing().contains(3);
        if left_missing != right_missing {
            let (missing, present) = if left_missing { (&mut left, &right) } else { (&mut right, &left) };
            missing.components[3] = present.components[3];
            missing.flags = Flags::from_missing(missing.flags.missing() & !Missing::single(3));
        }
        left.interpolate(right, self.space, self.hue)
            .eval(self.fraction).multiply_alpha(self.alpha)
    }
}

/// Inspect a root without consuming it. Mixing, relative and channel-math
/// functions use typed expressions; ordinary colors keep their fast path.
pub(super) fn is_computed(input: &mut Parser<'_, '_>) -> bool {
    let state = input.state();
    let result = match input.next().cloned() {
        Ok(Token::Function(name)) if name.eq_ignore_ascii_case("color-mix") => true,
        Ok(Token::Function(name)) if relative::supported_function(&name) => {
            let mut computed = false;
            // Inspect only top-level channel tokens, never recursively walk
            // function arguments before the bounded parser owns the input.
            let _: Result<'_, ()> = input.parse_nested_block(|input| {
                computed = input.try_parse(|input| input.expect_ident_matching("from")).is_ok();
                if !computed {
                    while let Ok(token) = input.next() {
                        if matches!(token, Token::Function(_)) { computed = true; break; }
                    }
                }
                Err(input.new_custom_error(()))
            });
            computed
        },
        _ => false,
    };
    input.reset(&state);
    result
}

pub(super) fn parse_computed<'i>(input: &mut Parser<'i, '_>) -> Result<'i, ComputedColor> {
    expression(input, 0).map(ComputedColor)
}

fn mix<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Expression> {
    if depth >= 16 { return Err(input.new_custom_error(())); }
    let mut space = ColorSpaceTag::Oklab;
    let mut hue = HueDirection::Shorter;
    if input.try_parse(|input| input.expect_ident_matching("in")).is_ok() {
        let name = input.expect_ident_cloned()?;
        space = match name.to_ascii_lowercase().as_str() {
            "srgb" => ColorSpaceTag::Srgb, "srgb-linear" => ColorSpaceTag::LinearSrgb,
            "display-p3" => ColorSpaceTag::DisplayP3, "a98-rgb" => ColorSpaceTag::A98Rgb,
            "prophoto-rgb" => ColorSpaceTag::ProphotoRgb, "rec2020" => ColorSpaceTag::Rec2020,
            "lab" => ColorSpaceTag::Lab, "oklab" => ColorSpaceTag::Oklab,
            "xyz" | "xyz-d65" => ColorSpaceTag::XyzD65, "xyz-d50" => ColorSpaceTag::XyzD50,
            "hsl" => ColorSpaceTag::Hsl, "hwb" => ColorSpaceTag::Hwb,
            "lch" => ColorSpaceTag::Lch, "oklch" => ColorSpaceTag::Oklch,
            _ => return Err(input.new_custom_error(())),
        };
        if let Ok(name) = input.try_parse(|input| input.expect_ident_cloned()) {
            if !matches!(space, ColorSpaceTag::Hsl | ColorSpaceTag::Hwb | ColorSpaceTag::Lch | ColorSpaceTag::Oklch) {
                return Err(input.new_custom_error(()));
            }
            hue = match name.to_ascii_lowercase().as_str() {
                "shorter" => HueDirection::Shorter, "longer" => HueDirection::Longer,
                "increasing" => HueDirection::Increasing, "decreasing" => HueDirection::Decreasing,
                _ => return Err(input.new_custom_error(())),
            };
            input.expect_ident_matching("hue")?;
        }
        input.expect_comma()?;
    }
    let (first, p1) = operand(input, depth + 1)?;
    input.expect_comma()?;
    let (second, p2) = operand(input, depth + 1)?;
    input.expect_exhausted()?;
    let (p1, p2) = match (p1, p2) {
        (None, None) => (0.5, 0.5), (Some(a), None) => (a, 1. - a),
        (None, Some(b)) => (1. - b, b), (Some(a), Some(b)) => (a, b),
    };
    let sum = p1 + p2;
    if sum <= 0. { return Err(input.new_custom_error(())); }
    let value = Mix { colors: [first, second], space, hue, fraction: (p2 / sum) as f32, alpha: sum.min(1.) as f32 };
    if matches!(value.colors, [Expression::Literal(_), Expression::Literal(_)]) {
        // Preserve floats and missing-component flags until an outer mix or
        // the actual renderer consumes this value.
        Ok(Expression::Literal(value.resolve(DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([0.; 4])))))
    } else { Ok(Expression::Mix(Box::new(value))) }
}

fn operand<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, (Expression, Option<f64>)> {
    let before = input.try_parse(super::number::percentage).ok();
    let value = expression(input, depth)?;
    let weight = if before.is_some() { before } else { input.try_parse(super::number::percentage).ok() };
    Ok((value, weight))
}

fn expression<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Expression> {
    if depth >= 16 { return Err(input.new_custom_error(())); }
    input.skip_whitespace();
    let value = match input.next()?.clone() {
        Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor") => return Ok(Expression::Current),
        Token::Ident(name) if name.eq_ignore_ascii_case("transparent") || cssparser::color::parse_named_color(&name).is_ok() => {
            color::parse_color(&name).map_err(|_| input.new_custom_error(()))?
        },
        Token::Hash(value) | Token::IDHash(value) if cssparser::color::parse_hash_color(value.as_bytes()).is_ok() => {
            color::parse_color(&format!("#{value}")).map_err(|_| input.new_custom_error(()))?
        },
        Token::Function(name) if name.eq_ignore_ascii_case("color-mix") => return input.parse_nested_block(|input| mix(input, depth)),
        Token::Function(name) if relative::supported_function(&name) => {
            return input.parse_nested_block(|input| {
                if input.try_parse(|input| input.expect_ident_matching("from")).is_ok() {
                    relative::parse(input, &name, depth)
                } else { relative::absolute(input, &name).map(Expression::Literal) }
            });
        },
        _ => return Err(input.new_custom_error(())),
    };
    Ok(Expression::Literal(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::rich::{CssColor, css::style};

    fn parsed(source: &str) -> std::sync::Arc<ComputedColor> {
        let Some(CssColor::Computed(value)) = style(&format!("color:{source}")).unwrap().foreground else {
            panic!("computed color was not preserved: {source}");
        };
        value
    }

    fn close(actual: [f32; 4], expected: [f32; 4]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
        }
    }

    #[test]
    fn mixing_space_premultiplication_and_percentage_alpha() {
        close(parsed("color-mix(in srgb, red, blue)").resolve([0.; 4]), [0.5, 0., 0.5, 1.]);
        close(parsed("color-mix(in srgb-linear, red, blue)").resolve([0.; 4]), [0.735357, 0., 0.735357, 1.]);
        close(parsed("color-mix(in srgb, rgb(255 0 0 / .2) 25%, rgb(0 0 255 / .8) 75%)").resolve([0.; 4]), [1. / 13., 0., 12. / 13., 0.65]);
        close(parsed("color-mix(in srgb, red 20%, blue 20%)").resolve([0.; 4]), [0.5, 0., 0.5, 0.4]);
        close(parsed("color-mix(in srgb, red 100%, blue 100%)").resolve([0.; 4]), [0.5, 0., 0.5, 1.]);
    }

    #[test]
    fn currentcolor_stays_live_inside_nested_mix_and_decoration() {
        let source = "color-mix(in srgb, color-mix(in srgb, currentColor, transparent), white)";
        let value = parsed(source);
        close(value.resolve([1., 0., 0., 1.]), [1., 2. / 3., 2. / 3., 0.75]);
        close(value.resolve([0., 0., 1., 1.]), [2. / 3., 2. / 3., 1., 0.75]);
        let decoration = style(&format!("text-decoration:underline {source} 2px")).unwrap().decoration.unwrap();
        let CssColor::Computed(decoration) = decoration.color else { panic!("missing decoration mix") };
        assert_eq!(value, decoration);
    }

    #[test]
    fn missing_components_hue_direction_and_unclipped_nested_colors() {
        let expected = color::parse_color("oklab(.6 .2 .1)").unwrap().convert(ColorSpaceTag::Srgb).clip().to_alpha_color::<Srgb>().components;
        close(parsed("color-mix(in oklab, oklab(.6 none .1), oklab(.6 .2 none))").resolve([0.; 4]), expected);
        close(parsed("color-mix(in hsl shorter hue, hsl(350 100% 50%), hsl(10 100% 50%))").resolve([0.; 4]), [1., 0., 0., 1.]);
        close(parsed("color-mix(in hsl longer hue, hsl(350 100% 50%), hsl(10 100% 50%))").resolve([0.; 4]), [0., 1., 1., 1.]);
        close(parsed("color-mix(in srgb, color-mix(in srgb, color(srgb 2 -1 .5), black), white)").resolve([0.; 4]), [1., 0.25, 0.625, 1.]);
    }

    #[test]
    fn grammar_calculations_and_invalid_declarations() {
        close(parsed("COLOR-MIX(IN SRGB, calc(10% + 15%) RED, BLUE)").resolve([0.; 4]), [0.25, 0., 0.75, 1.]);
        close(parsed("color-mix(in srgb, red calc(200%), blue)").resolve([0.; 4]), [1., 0., 0., 1.]);
        for invalid in [
            "color-mix(in srgb, red 0%, blue 0%)", "color-mix(in srgb, red -1%, blue)",
            "color-mix(in srgb, red 101%, blue)", "color-mix(in srgb longer hue, red, blue)",
            "color-mix(in srgb, red calc(10%/**/+15%), blue)", "color-mix(in srgb, red calc(10% +/**/15%), blue)",
            "color-mix(in srgb, red calc(10), blue)", "color-mix(in srgb, red, blue, green)",
            "color-mix(in srgb, hsv(0, 100%, 100%), blue)", "color-mix(in srgb, ff0000, blue)",
        ] {
            assert_eq!(style(&format!("color:red;color:{invalid}")).unwrap().foreground, Some(CssColor::Rgba([255, 0, 0, 255])), "{invalid}");
        }
    }

    #[test]
    fn relative_currentcolor_and_mixing_share_computed_channels() {
        let value = parsed("rgb(from currentColor calc(r / 2) g b / calc(alpha * 2))");
        close(value.resolve([1., 0.2, 0.4, 0.3]), [0.5, 0.2, 0.4, 0.6]);
        close(value.resolve([0.2, 0.4, 0.6, 0.9]), [0.1, 0.4, 0.6, 1.]);
        close(parsed("color-mix(in srgb, rgb(from currentcolor calc(r / 2) g b), white)").resolve([1., 0., 0., 1.]), [0.75, 0.5, 0.5, 1.]);
        close(parsed("rgb(from color-mix(in srgb, red 25%, blue) b g r)").resolve([0.; 4]), [0.75, 0., 0.25, 1.]);
    }

    #[test]
    fn relative_percentage_scales_and_color_space_conversions() {
        close(parsed("color(from color(srgb 0 0 0 / 60%) srgb alpha .6 .6 / .9)").resolve([0.; 4]), [0.6, 0.6, 0.6, 0.9]);
        close(parsed("rgb(from rgb(0 0 0 / 60%) alpha 153 153 / .9)").resolve([0.; 4]), [0.6 / 255., 0.6, 0.6, 0.9]);
        close(parsed("rgb(from red calc(30%) 0 0 / 25%)").resolve([0.; 4]), [0.3, 0., 0., 0.25]);
        close(parsed("hsl(from red calc(h + 120) s l)").resolve([0.; 4]), [0., 1., 0., 1.]);
        for (source, expected) in [
            ("lab(from lab(50 25 -25) l a b)", "lab(50 25 -25)"),
            ("lch(from lch(60 20 350) l c calc(h + 30))", "lch(60 20 20)"),
            ("oklab(from red 50% 25% -25%)", "oklab(.5 .1 -.1)"),
            ("oklch(from red 50% 25% 30deg)", "oklch(.5 .1 30)"),
            ("hwb(from red h 20% 30%)", "hwb(0 20% 30%)"),
            ("color(from red display-p3 r g b)", "red"),
            ("color(from red xyz-d50 x y z)", "red"),
        ] {
            let expected = color::parse_color(expected).unwrap().convert(ColorSpaceTag::Srgb).clip().to_alpha_color::<Srgb>().components;
            close(parsed(source).resolve([0.; 4]), expected);
        }
    }

    #[test]
    fn relative_missing_components_and_unclipped_origins() {
        let expected = color::parse_color("oklab(.6 .2 .1)").unwrap().convert(ColorSpaceTag::Srgb).clip().to_alpha_color::<Srgb>().components;
        close(parsed("color-mix(in oklab, oklab(from oklab(.6 none .1) l a b), oklab(.6 .2 none))").resolve([0.; 4]), expected);
        let expected = color::parse_color("oklab(.6 .1 .1)").unwrap().convert(ColorSpaceTag::Srgb).clip().to_alpha_color::<Srgb>().components;
        close(parsed("oklab(from oklab(.6 none .1) l calc(a + .1) b)").resolve([0.; 4]), expected);
        close(parsed("color-mix(in srgb, rgb(from color(srgb 2 -1 .5) r g b), white)").resolve([0.; 4]), [1., 0., 0.75, 1.]);
    }

    #[test]
    fn relative_math_keeps_grammar_boundaries_and_resource_limits() {
        for invalid in [
            "rgb(from red calc(r/**/+10) g b)", "rgb(from red calc(r +/**/10) g b)",
            "rgb(from red calc(r + 10%) g b)", "rgb(from red calc(1px) g b)",
            "rgb(from red h g b)", "rgb(from red r, g, b)", "rgb(from red r g b / alpha alpha)",
            "hsl(from red 10% s l)", "color(from red lab l a b)", "rgb(from hsv(0,1,1) r g b)",
        ] {
            assert_eq!(style(&format!("color:red;color:{invalid}")).unwrap().foreground, Some(CssColor::Rgba([255, 0, 0, 255])), "{invalid}");
        }
        let enormous = format!("rgb(from currentcolor calc(r{}) g b)", " + 1".repeat(300));
        assert_eq!(style(&format!("color:red;color:{enormous}")).unwrap().foreground, Some(CssColor::Rgba([255, 0, 0, 255])));
    }

    #[test]
    fn absolute_math_composes_with_mix_relative_and_style_properties() {
        close(parsed("rgb(calc(255 / 2) min(40%, 20%) 0 / clamp(0, .6, 1))").resolve([0.; 4]), [0.5, 0.2, 0., 0.6]);
        close(parsed("rgba(calc(50%), 0%, 100%, calc(25%))").resolve([0.; 4]), [0.5, 0., 1., 0.25]);
        close(parsed("color-mix(in srgb, rgb(calc(255 / 2) 0 0), currentColor)").resolve([0., 0., 1., 1.]), [0.25, 0., 0.5, 1.]);
        close(parsed("rgb(from rgb(calc(255 / 2) 0 0) calc(r * 2) g b)").resolve([0.; 4]), [1., 0., 0., 1.]);
        let foreground = parsed("rgb(calc(255 / 2) 0 0)");
        let styles = style("background-color:rgb(calc(255 / 2) 0 0);text-decoration:underline rgb(calc(255 / 2) 0 0) 2px").unwrap();
        assert_eq!(styles.background, Some(CssColor::Computed(foreground.clone())));
        assert_eq!(styles.decoration.unwrap().color, CssColor::Computed(foreground));
        assert!(matches!(parsed("rgb(calc(255 / 2) 0 0)").0, Expression::Literal(_)));
    }

    #[test]
    fn absolute_math_preserves_space_ranges_and_missing_components() {
        for (source, expected) in [
            ("hsl(calc(1turn / 3) 100% 50%)", "hsl(120 100% 50%)"),
            ("hsla(calc(120), calc(100%), 50%, .5)", "hsl(120 100% 50% / .5)"),
            ("hwb(calc(120) 20 30)", "hwb(120 20% 30%)"),
            ("lab(calc(120) calc(10%) -25)", "lab(100 12.5 -25)"),
            ("lch(calc(50%) calc(-10) calc(1turn + 20deg))", "lch(50 0 20)"),
            ("oklab(calc(50%) calc(25%) -.1)", "oklab(.5 .1 -.1)"),
            ("oklch(calc(50%) calc(25%) 30)", "oklch(.5 .1 30)"),
            ("color(display-p3 calc(50%) .2 .3)", "color(display-p3 .5 .2 .3)"),
            ("color(xyz-d50 calc(.2) .3 .4)", "color(xyz-d50 .2 .3 .4)"),
        ] {
            let expected = color::parse_color(expected).unwrap().convert(ColorSpaceTag::Srgb).clip().to_alpha_color::<Srgb>().components;
            let actual = parsed(source).resolve([0.; 4]);
            assert!(actual.into_iter().zip(expected).all(|(a, b)| (a - b).abs() < 0.0001), "{source}: {actual:?} != {expected:?}");
        }
        close(parsed("color-mix(in srgb, color(srgb calc(2) -1 .5), white)").resolve([0.; 4]), [1., 0., 0.75, 1.]);
        close(parsed("color-mix(in srgb, rgb(calc(510) -255 127.5), white)").resolve([0.; 4]), [1., 0.5, 0.75, 1.]);
        let value = parsed("color-mix(in oklab, oklab(calc(.6) none .1 / none), oklab(.6 .2 none / .5))");
        let expected = color::parse_color("oklab(.6 .2 .1 / .5)").unwrap().convert(ColorSpaceTag::Srgb).clip().to_alpha_color::<Srgb>().components;
        close(value.resolve([0.; 4]), expected);
        close(parsed("color-mix(in srgb, rgb(calc(255) 0 0 / .5), rgb(0 0 255 / none))").resolve([0.; 4]), [0.5, 0., 0.5, 0.5]);
        close(parsed("color-mix(in srgb, rgb(from currentcolor r g b / none), rgb(0 0 255 / .5))").resolve([1., 0., 0., 0.2]), [0.5, 0., 0.5, 0.5]);
    }

    #[test]
    fn absolute_math_rejects_invalid_grammar_without_losing_prior_styles() {
        for invalid in [
            "rgb(calc(10)/**/+20 0 0)", "rgb(calc(10/**/+20) 0 0)", "rgb(calc(10 +/**/20) 0 0)",
            "rgb(calc(10px) 0 0)", "rgb(calc(10deg) 0 0)", "rgb(calc(r) 0 0)",
            "rgb(calc(10), 20%, 30)", "rgb(calc(10), 20, none)", "rgb(calc(10), 20, 30 / .5)",
            "rgb(calc(10) 20 30, .5)", "rgb(calc(10), 20, 30, none)",
            "hsl(calc(10), 20, 30%)", "hsl(calc(10%), 20%, 30%)", "hwb(calc(10), 20%, 30%)",
            "lab(calc(50), 0, 0)", "color(srgb calc(1), 0, 0)", "rgb(calc(10) 20 30 40)",
        ] {
            assert_eq!(style(&format!("color:red;color:{invalid}")).unwrap().foreground, Some(CssColor::Rgba([255, 0, 0, 255])), "{invalid}");
        }
        let enormous = format!("rgb(calc(1{}) 0 0)", " + 1".repeat(300));
        assert_eq!(style(&format!("color:red;color:{enormous}")).unwrap().foreground, Some(CssColor::Rgba([255, 0, 0, 255])));
    }
}
