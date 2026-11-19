//! Absolute and relative colors share channel syntax and typed math.
//! Relative colors retain numeric bindings rather than CSS templates.
use color::{ColorSpaceTag, DynamicColor, Flags, Missing};
use cssparser::{Parser, Token};
use super::{Expression, Result};
use super::super::{length::channel_calculation, number};
use super::super::super::length::{Step, evaluate_steps};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Relative {
    origin: Expression,
    space: ColorSpaceTag,
    scale: [f64; 4],
    channels: [Channel; 4],
    hue: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
enum Channel { Missing, Reference(u8), Number(f64), Math { steps: Box<[Step]>, basis: f64 } }

impl Relative {
    pub fn resolve(&self, current: DynamicColor) -> DynamicColor {
        let origin = self.origin.resolve(current).convert(self.space);
        let mut values = origin.components.map(f64::from);
        for (value, scale) in values.iter_mut().zip(self.scale) { *value *= scale; }
        if let Some(index) = self.hue { values[index] = values[index].rem_euclid(360.); }
        let mut missing = Missing::default();
        let mut components = [0.; 4];
        for (index, channel) in self.channels.iter().enumerate() {
            let value = match channel {
                Channel::Missing => { missing.insert(index); 0. },
                Channel::Reference(source) => {
                    if origin.flags.missing().contains(usize::from(*source)) { missing.insert(index); }
                    values[usize::from(*source)]
                },
                Channel::Number(value) => *value,
                Channel::Math { steps, basis } => evaluate_steps(steps, |_, _| unreachable!("color channels have no lengths"), *basis,
                    |channel| if origin.flags.missing().contains(usize::from(channel)) { 0. } else { values[usize::from(channel)] }),
            };
            // A calculation on none yields a number, while a direct reference
            // carries missingness into further relative colors and mixing.
            let value = if value.is_nan() { 0. } else { value / self.scale[index] };
            components[index] = if index == 3 { value.clamp(0., 1.) as f32 } else { value as f32 };
        }
        DynamicColor { cs: self.space, flags: Flags::from_missing(missing), components }
    }

    pub fn allocated_weight(&self) -> u64 {
        std::mem::size_of::<Self>() as u64 + self.origin.allocated_weight() + self.channels.iter().map(|channel| {
            if let Channel::Math { steps, .. } = channel { std::mem::size_of_val(steps.as_ref()) as u64 } else { 0 }
        }).sum::<u64>()
    }
}

pub(super) fn supported_function(name: &str) -> bool {
    ["rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch", "color"]
        .iter().any(|candidate| name.eq_ignore_ascii_case(candidate))
}

struct Syntax {
    space: ColorSpaceTag, names: [&'static str; 4], scale: [f64; 4],
    percentages: [f64; 4], hue: Option<usize>,
}

fn syntax<'i>(input: &mut Parser<'i, '_>, name: &str) -> Result<'i, Syntax> {
    let name = name.to_ascii_lowercase();
    let (space, names, scale, percentages, hue) = match name.as_str() {
        "rgb" | "rgba" => (ColorSpaceTag::Srgb, ["r", "g", "b", "alpha"], [255., 255., 255., 1.], [255., 255., 255., 1.], None),
        "hsl" | "hsla" => (ColorSpaceTag::Hsl, ["h", "s", "l", "alpha"], [1.; 4], [0., 100., 100., 1.], Some(0)),
        "hwb" => (ColorSpaceTag::Hwb, ["h", "w", "b", "alpha"], [1.; 4], [0., 100., 100., 1.], Some(0)),
        "lab" => (ColorSpaceTag::Lab, ["l", "a", "b", "alpha"], [1.; 4], [100., 125., 125., 1.], None),
        "lch" => (ColorSpaceTag::Lch, ["l", "c", "h", "alpha"], [1.; 4], [100., 150., 0., 1.], Some(2)),
        "oklab" => (ColorSpaceTag::Oklab, ["l", "a", "b", "alpha"], [1.; 4], [1., 0.4, 0.4, 1.], None),
        "oklch" => (ColorSpaceTag::Oklch, ["l", "c", "h", "alpha"], [1.; 4], [1., 0.4, 0., 1.], Some(2)),
        "color" => {
            let name = input.expect_ident_cloned()?;
            let space = match name.to_ascii_lowercase().as_str() {
                "srgb" => ColorSpaceTag::Srgb, "srgb-linear" => ColorSpaceTag::LinearSrgb,
                "display-p3" => ColorSpaceTag::DisplayP3, "a98-rgb" => ColorSpaceTag::A98Rgb,
                "prophoto-rgb" => ColorSpaceTag::ProphotoRgb, "rec2020" => ColorSpaceTag::Rec2020,
                "xyz" | "xyz-d65" => ColorSpaceTag::XyzD65, "xyz-d50" => ColorSpaceTag::XyzD50,
                _ => return Err(input.new_custom_error(())),
            };
            let names = if matches!(space, ColorSpaceTag::XyzD50 | ColorSpaceTag::XyzD65) { ["x", "y", "z", "alpha"] }
                else { ["r", "g", "b", "alpha"] };
            (space, names, [1.; 4], [1.; 4], None)
        },
        _ => return Err(input.new_custom_error(())),
    };
    Ok(Syntax { space, names, scale, percentages, hue })
}

pub(super) fn parse<'i>(input: &mut Parser<'i, '_>, name: &str, depth: u8) -> Result<'i, Expression> {
    let origin = super::expression(input, depth + 1)?;
    let Syntax { space, names, scale, percentages, hue } = syntax(input, name)?;
    let mut channels = [Channel::Missing, Channel::Missing, Channel::Missing, Channel::Reference(3)];
    for index in 0..3 { channels[index] = channel(input, names, percentages[index], hue == Some(index))?; }
    if input.try_parse(|input| input.expect_delim('/')).is_ok() { channels[3] = channel(input, names, 1., false)?; }
    input.expect_exhausted()?;
    let value = Relative { origin, space, scale, channels, hue };
    if matches!(value.origin, Expression::Literal(_)) {
        // No dependence on currentColor remains; release the channel program.
        Ok(Expression::Literal(value.resolve(super::DynamicColor::from_alpha_color(super::AlphaColor::<super::Srgb>::new([0.; 4])))))
    } else { Ok(Expression::Relative(Box::new(value))) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { Number, Percentage, Angle, Missing }

fn channel<'i>(input: &mut Parser<'i, '_>, names: [&'static str; 4], basis: f64, hue: bool) -> Result<'i, Channel> {
    parsed_channel(input, names, basis, hue).map(|(value, _)| value)
}

fn parsed_channel<'i>(input: &mut Parser<'i, '_>, names: [&'static str; 4], basis: f64, hue: bool) -> Result<'i, (Channel, Kind)> {
    input.skip_whitespace();
    let state = input.state();
    let start = input.position();
    let mut kind = Kind::Number;
    let value = match input.next()?.clone() {
        Token::Ident(name) if name.eq_ignore_ascii_case("none") => { kind = Kind::Missing; Channel::Missing },
        Token::Ident(name) => {
            let index = names.iter().position(|candidate| name.eq_ignore_ascii_case(candidate)).ok_or_else(|| input.new_custom_error(()))?;
            Channel::Reference(index as u8)
        },
        Token::Number { .. } => Channel::Number(number::literal_prefix(input.slice_from(start)).ok_or_else(|| input.new_custom_error(()))?),
        Token::Percentage { .. } if !hue => { kind = Kind::Percentage; Channel::Number(number::literal_prefix(input.slice_from(start)).ok_or_else(|| input.new_custom_error(()))? / 100. * basis) },
        Token::Dimension { unit, .. } if hue => {
            kind = Kind::Angle;
            let value = number::Quantity::dimension(input.slice_from(start), &unit).filter(|value| value.is_angle()).ok_or_else(|| input.new_custom_error(()))?;
            Channel::Number(value.value)
        },
        Token::Function(_) => {
            input.reset(&state);
            let (steps, quantity) = channel_calculation(input, names, hue)?;
            kind = if quantity.is_percentage() { Kind::Percentage } else if quantity.is_angle() { Kind::Angle } else { Kind::Number };
            if steps.iter().any(|step| matches!(step, Step::Channel(_))) { Channel::Math { steps, basis } }
            else { Channel::Number(evaluate_steps(&steps, |_, _| unreachable!("color channels have no lengths"), basis, |_| unreachable!("constant program"))) }
        },
        _ => return Err(input.new_custom_error(())),
    };
    Ok((value, kind))
}

/// Absolute channels use the same units and math, but have no origin bindings.
/// They are folded immediately, retaining neither a source nor a program.
pub(super) fn absolute<'i>(input: &mut Parser<'i, '_>, name: &str) -> Result<'i, DynamicColor> {
    let syntax = syntax(input, name)?;
    let mut components = [0., 0., 0., 1.];
    let mut kinds = [Kind::Number; 4];
    let mut missing = Missing::default();
    let mut legacy = false;
    let legacy_allowed = ["rgb", "rgba", "hsl", "hsla"].iter().any(|candidate| name.eq_ignore_ascii_case(candidate));
    for index in 0..3 {
        if index == 2 && legacy { input.expect_comma()?; }
        let (channel, kind) = parsed_channel(input, [""; 4], syntax.percentages[index], syntax.hue == Some(index))?;
        kinds[index] = kind;
        match channel {
            Channel::Missing => missing.insert(index),
            Channel::Number(value) => components[index] = without_nan(value) / syntax.scale[index],
            _ => unreachable!("absolute channels have no origin bindings"),
        }
        if index == 0 {
            legacy = input.try_parse(|input| input.expect_comma()).is_ok();
            if legacy && !legacy_allowed { return Err(input.new_custom_error(())); }
        }
    }
    let has_alpha = if legacy { input.try_parse(|input| input.expect_comma()).is_ok() }
        else { input.try_parse(|input| input.expect_delim('/')).is_ok() };
    if has_alpha {
        let (channel, kind) = parsed_channel(input, [""; 4], 1., false)?;
        kinds[3] = kind;
        match channel {
            Channel::Missing => { missing.insert(3); components[3] = 0.; },
            Channel::Number(value) => components[3] = without_nan(value).clamp(0., 1.),
            _ => unreachable!("absolute alpha has no origin bindings"),
        }
    }
    input.expect_exhausted()?;
    if legacy && (kinds.contains(&Kind::Missing) || match syntax.space {
        ColorSpaceTag::Srgb => kinds[0] != kinds[1] || kinds[1] != kinds[2],
        ColorSpaceTag::Hsl => kinds[1] != Kind::Percentage || kinds[2] != Kind::Percentage,
        _ => true,
    }) { return Err(input.new_custom_error(())); }

    // CSS's parse-time limits differ from display gamut clipping. In
    // particular color() keeps extended channels for subsequent operations.
    match syntax.space {
        ColorSpaceTag::Srgb if !name.eq_ignore_ascii_case("color") => {
            for value in &mut components[..3] { *value = value.clamp(0., 1.); }
        },
        ColorSpaceTag::Hsl => components[1] = components[1].max(0.),
        ColorSpaceTag::Hwb => { components[1] = components[1].max(0.); components[2] = components[2].max(0.); },
        ColorSpaceTag::Lab | ColorSpaceTag::Lch => components[0] = components[0].clamp(0., 100.),
        ColorSpaceTag::Oklab | ColorSpaceTag::Oklch => components[0] = components[0].clamp(0., 1.),
        _ => {},
    }
    if matches!(syntax.space, ColorSpaceTag::Lch | ColorSpaceTag::Oklch) { components[1] = components[1].max(0.); }
    if let Some(index) = syntax.hue { components[index] = without_nan(components[index].rem_euclid(360.)); }
    Ok(DynamicColor { cs: syntax.space, flags: Flags::from_missing(missing), components: components.map(|value| value as f32) })
}

fn without_nan(value: f64) -> f64 { if value.is_nan() { 0. } else { value } }
