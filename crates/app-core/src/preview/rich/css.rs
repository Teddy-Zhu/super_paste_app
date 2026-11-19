//! The old preview's style whitelist, parsed once into the controlled document.
//! Cascade priority belongs to this attribute, never to the inherited style.
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser,
    ParserInput, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
    ToCss, Token, match_ignore_ascii_case, parse_important,
};
use super::{Alignment, CssColor, Decoration, DecorationPattern, DecorationThickness, FontStyle, LengthUnit, ListStyle, Style, StyleProperty as Property, StyleReset, VerticalAlignment, Weight, WhiteSpace};

mod number;
mod length;
mod variables;
mod colors;
pub use colors::ComputedColor;
pub use variables::DeferredStyles;

impl CssColor {
    fn parse(value: &str) -> Option<Self> {
        let mut input = ParserInput::new(value);
        let mut input = Parser::new(&mut input);
        if colors::is_computed(&mut input) {
            let value = colors::parse_computed(&mut input).ok()?;
            input.expect_exhausted().ok()?;
            return Some(Self::Computed(std::sync::Arc::new(value)));
        }
        standard_color_form(&mut input, 0).ok()?;
        input.expect_exhausted().ok()?;
        if value.eq_ignore_ascii_case("currentcolor") { Some(Self::Current) }
        else { csscolorparser::parse(value).ok().map(|color| Self::Rgba(color.clamp().to_rgba8())) }
    }
}

// The converter accepts bare hexadecimal values and non-CSS functions such
// as hsv()/hwba(). Restrict those extensions at both the outer color and every
// relative-color origin; channel conversion remains owned by the library.
fn standard_color_form<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<(), ParseError<'i, ()>> {
    match input.next()?.clone() {
        Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor")
            || name.eq_ignore_ascii_case("transparent")
            || cssparser::color::parse_named_color(&name).is_ok() => Ok(()),
        Token::Hash(value) | Token::IDHash(value)
            if cssparser::color::parse_hash_color(value.as_bytes()).is_ok() => Ok(()),
        Token::Function(name) if depth < 16 && [
            "rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch", "color",
        ].iter().any(|standard| name.eq_ignore_ascii_case(standard)) => input.parse_nested_block(|input| {
            if input.try_parse(|input| input.expect_ident_matching("from")).is_ok() {
                standard_color_form(input, depth + 1)?;
            }
            input.expect_no_error_token()?;
            Ok(())
        }),
        _ => Err(input.new_custom_error(())),
    }
}

impl Property {
    fn named(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "color" => Self::Color,
            "background-color" => Self::Background,
            "font-weight" => Self::Weight,
            "font-style" => Self::FontStyle,
            "text-decoration" => Self::Decoration,
            "text-align" => Self::Alignment,
            "vertical-align" => Self::Vertical,
            "white-space" => Self::Space,
            "list-style-type" => Self::List,
            _ => return None,
        })
    }
}

fn reset(property: Property, value: &str) -> Option<StyleReset> {
    if value.eq_ignore_ascii_case("inherit") { Some(StyleReset::Inherit) }
    else if value.eq_ignore_ascii_case("initial") { Some(StyleReset::Initial) }
    else if value.eq_ignore_ascii_case("unset") { Some(if property.inherited() { StyleReset::Inherit } else { StyleReset::Initial }) }
    else if value.eq_ignore_ascii_case("revert") { Some(StyleReset::Revert) }
    else if value.eq_ignore_ascii_case("revert-layer") { Some(StyleReset::RevertLayer) }
    else { None }
}

enum Value {
    Reset(Property, StyleReset),
    Pending(Property, std::sync::Arc<str>),
    Color(CssColor), Background(CssColor), Weight(Weight), FontStyle(FontStyle),
    Decoration(Decoration), Alignment(Alignment), Vertical(VerticalAlignment),
    Space(WhiteSpace), List(ListStyle),
}
impl Value {
    fn from_normalized(property: Property, value: &str) -> Option<Self> {
        if let Some(reset) = reset(property, value) { return Some(Self::Reset(property, reset)); }
        Some(match property {
            Property::Color => Self::Color(CssColor::parse(value)?),
            Property::Background => Self::Background(CssColor::parse(value)?),
            // Numeric calculations consume the original token stream, where
            // comments do not satisfy required whitespace around + and -.
            Property::Weight => return None,
            Property::FontStyle => return None,
            Property::Decoration => return None,
            Property::Alignment => Self::Alignment(match value {
                "left" => Alignment::Left, "right" => Alignment::Right,
                "center" => Alignment::Center, "justify" => Alignment::Justify,
                "start" => Alignment::Start, "end" => Alignment::End,
                "match-parent" => Alignment::MatchParent, _ => return None,
            }),
            Property::Vertical => Self::Vertical(match value {
                "baseline" => VerticalAlignment::Baseline, "sub" => VerticalAlignment::Subscript,
                "super" => VerticalAlignment::Superscript, "top" => VerticalAlignment::Top,
                "middle" => VerticalAlignment::Middle, "bottom" => VerticalAlignment::Bottom,
                "text-top" => VerticalAlignment::TextTop, "text-bottom" => VerticalAlignment::TextBottom,
                _ => return None,
            }),
            Property::Space => Self::Space(white_space(value)?),
            Property::List => return None,
        })
    }
    fn apply(self, style: &mut Style) {
        match self {
            Self::Pending(property, source) => {
                Self::Reset(property, if property.inherited() { StyleReset::Inherit } else { StyleReset::Initial }).apply(style);
                let values = style.variable_styles.get_or_insert_with(Default::default);
                std::sync::Arc::make_mut(values).push(property, source);
            },
            Self::Reset(property, reset) => {
                style.resets.set(property, Some(reset));
                match property {
                    Property::Color => style.foreground = None,
                    Property::Background => style.background = None,
                    Property::Weight => style.weight = None,
                    Property::FontStyle => style.font_style = None,
                    Property::Decoration => style.decoration = None,
                    Property::Alignment => style.alignment = None,
                    Property::Vertical => style.vertical = None,
                    Property::Space => style.white_space = None,
                    Property::List => style.list_style = None,
                }
            },
            Self::Color(value) => style.foreground = Some(value),
            Self::Background(value) => style.background = Some(value),
            Self::Weight(value) => style.weight = Some(value),
            Self::FontStyle(value) => style.font_style = Some(value),
            Self::Decoration(value) => style.decoration = Some(value),
            Self::Alignment(value) => style.alignment = Some(value),
            Self::Vertical(value) => style.vertical = Some(value),
            Self::Space(value) => style.white_space = Some(value),
            Self::List(value) => style.list_style = Some(value),
        }
    }
}
/// Validate and retain the shorthand's typed components. Each
/// component occurs once, and the line keywords form one contiguous component.
/// Color remains typed until the decorating element computes currentcolor;
/// relative thickness remains tied to its decorating element.
fn decoration(input: &mut Parser<'_, '_>, text: &mut String, limited: &mut bool) -> Option<Decoration> {
    const UNDERLINE: u8 = 1; const OVERLINE: u8 = 2; const STRIKE: u8 = 4;
    const BLINK: u8 = 8; const NONE: u8 = 16;
    let mut lines = 0_u8; let mut line_closed = false;
    let mut pattern = None; let mut color = None; let mut thickness = None;
    while !input.is_exhausted() {
        let state = input.state();
        let start = input.position();
        let token = input.next().ok()?.clone();
        let line = match &token {
            Token::Ident(name) => match_ignore_ascii_case! { name.as_ref(),
                "underline" => UNDERLINE, "overline" => OVERLINE, "line-through" => STRIKE,
                "blink" => BLINK, "none" => NONE, _ => 0,
            },
            _ => 0,
        };
        if line != 0 {
            if line_closed || lines & line != 0 || lines & NONE != 0 || (line == NONE && lines != 0) { return None; }
            lines |= line; continue;
        }
        if lines != 0 { line_closed = true; }
        match token {
            Token::Ident(name) if ["solid", "double", "dotted", "dashed", "wavy"].iter().any(|value| name.eq_ignore_ascii_case(value)) => {
                if pattern.is_some() { return None; }
                pattern = Some(match_ignore_ascii_case! { name.as_ref(),
                    "solid" => DecorationPattern::Solid, "double" => DecorationPattern::Double,
                    "dotted" => DecorationPattern::Dotted, "dashed" => DecorationPattern::Dashed,
                    "wavy" => DecorationPattern::Wavy, _ => unreachable!(),
                });
            },
            Token::Ident(name) if name.eq_ignore_ascii_case("auto") || name.eq_ignore_ascii_case("from-font") => {
                if thickness.is_some() { return None; }
                thickness = Some(if name.eq_ignore_ascii_case("auto") { DecorationThickness::Auto } else { DecorationThickness::FromFont });
            },
            Token::Number { value, .. } if value == 0.0 => {
                if thickness.is_some() { return None; }
                thickness = Some(DecorationThickness::Length { value, unit: LengthUnit::Px });
            },
            Token::Dimension { value, unit, .. } if value.is_finite() => {
                if thickness.is_some() { return None; }
                thickness = Some(DecorationThickness::Length { value, unit: LengthUnit::parse(&unit.to_ascii_lowercase())? });
            },
            Token::Percentage { unit_value, .. } if unit_value.is_finite() => {
                if thickness.is_some() { return None; }
                thickness = Some(DecorationThickness::Percentage(unit_value));
            },
            Token::Ident(name) => {
                if color.is_some() { return None; }
                color = Some(CssColor::parse(&name)?);
            },
            Token::Hash(_) | Token::IDHash(_) => {
                if color.is_some() { return None; }
                color = Some(CssColor::parse(input.slice_from(start).trim())?);
            },
            Token::Function(_) => {
                // Math must consume original tokens: replacing comments with
                // spaces can turn an invalid + or - expression into valid CSS.
                input.reset(&state);
                if colors::is_computed(input) {
                    if color.is_some() { return None; }
                    color = Some(CssColor::Computed(std::sync::Arc::new(colors::parse_computed(input).ok()?)));
                    continue;
                }
                if let Ok(value) = input.try_parse(|input| length::calculation(input, limited)) {
                    if thickness.is_some() { return None; }
                    thickness = Some(DecorationThickness::Calculation(value));
                } else {
                    if color.is_some() { return None; }
                    let token = input.next().ok()?.clone();
                    text.clear();
                    token.to_css(text).ok()?;
                    input.parse_nested_block(|input| normalized_value(input, text, 0)).ok()?;
                    text.push(')');
                    text.make_ascii_lowercase();
                    color = Some(CssColor::parse(text)?);
                }
            },
            _ => return None,
        }
    }
    if lines == 0 && pattern.is_none() && color.is_none() && thickness.is_none() { return None; }
    // Omitted line components reset to none, including a color/style-only
    // shorthand. They do not preserve the preceding declaration's lines.
    Some(Decoration { underline: lines & UNDERLINE != 0, overline: lines & OVERLINE != 0,
        strike: lines & STRIKE != 0, blink: lines & BLINK != 0,
        pattern: pattern.unwrap_or_default(), color: color.unwrap_or_default(), thickness: thickness.unwrap_or_default() })
}

fn white_space(value: &str) -> Option<WhiteSpace> {
    match value {
        "normal" => return Some(WhiteSpace::Normal),
        "pre" => return Some(WhiteSpace::Pre),
        "pre-wrap" => return Some(WhiteSpace::PreWrap),
        "pre-line" => return Some(WhiteSpace::PreLine),
        _ => (),
    }
    let mut collapse = None;
    let mut nowrap = None;
    for keyword in value.split_ascii_whitespace() {
        let behavior = match keyword {
            "wrap" | "nowrap" => {
                if nowrap.replace(keyword == "nowrap").is_some() { return None; }
                continue;
            },
            "collapse" => WhiteSpace::Normal,
            "preserve" => WhiteSpace::PreWrap,
            "preserve-breaks" => WhiteSpace::PreLine,
            "break-spaces" => WhiteSpace::BreakSpaces,
            _ => return None,
        };
        if collapse.replace(behavior).is_some() { return None; }
    }
    if collapse.is_none() && nowrap.is_none() { return None; }
    // Each omitted shorthand component resets to its initial value. Keep
    // collapsing and wrapping independent, including preserved hard breaks.
    Some(match (collapse.unwrap_or(WhiteSpace::Normal), nowrap.unwrap_or(false)) {
        (WhiteSpace::Normal, true) => WhiteSpace::NoWrap,
        (WhiteSpace::PreWrap, true) => WhiteSpace::Pre,
        (WhiteSpace::PreLine, true) => WhiteSpace::PreLineNoWrap,
        (WhiteSpace::BreakSpaces, true) => WhiteSpace::BreakSpacesNoWrap,
        (behavior, _) => behavior,
    })
}

fn typed_value<'i>(property: Property, input: &mut Parser<'i, '_>, text: &mut String, limited: &mut bool) -> Result<Value, ParseError<'i, ()>> {
    if matches!(property, Property::Color | Property::Background)
        && colors::is_computed(input) {
        let value = CssColor::Computed(std::sync::Arc::new(colors::parse_computed(input)?));
        input.expect_exhausted()?;
        return Ok(if property == Property::Color { Value::Color(value) } else { Value::Background(value) });
    }
    if property == Property::Weight { return number::weight(input); }
    if property == Property::FontStyle { return number::font_style(input); }
    if property == Property::Vertical { return length::vertical(input, limited); }
    if property == Property::Decoration {
        if let Ok(name) = input.try_parse(|input| -> Result<_, ParseError<'_, ()>> {
            let name = input.expect_ident_cloned()?;
            if reset(Property::Decoration, &name).is_some() { Ok(name) }
            else { Err(input.new_custom_error(())) }
        }) {
            input.expect_exhausted()?;
            return Ok(Value::Reset(Property::Decoration, reset(Property::Decoration, &name).expect("validated reset")));
        }
        let value = decoration(input, text, limited).ok_or_else(|| input.new_custom_error(()))?;
        input.expect_exhausted()?;
        return Ok(Value::Decoration(value));
    }
    if property == Property::List {
        let value = if let Ok(name) = input.try_parse(|input| input.expect_ident_cloned()) {
            if let Some(reset) = reset(Property::List, &name) { Value::Reset(Property::List, reset) }
            else { Value::List(ListStyle::named(&name).ok_or_else(|| input.new_custom_error(()))?) }
        } else { Value::List(ListStyle::non_ident(input)?) };
        input.expect_exhausted()?;
        return Ok(value);
    }
    normalized_value(input, text, 0)?;
    text.make_ascii_lowercase();
    Value::from_normalized(property, text.trim()).ok_or_else(|| input.new_custom_error(()))
}

struct Declaration { property: Property, value: Value, important: bool }
struct Declarations { text: String, limited: bool }
impl<'i> DeclarationParser<'i> for Declarations {
    type Declaration = Declaration;
    type Error = ();
    fn parse_value<'t>(
        &mut self, name: CowRcStr<'i>, input: &mut Parser<'i, 't>, _: &ParserState,
    ) -> Result<Declaration, ParseError<'i, ()>> {
        let property = Property::named(&name).ok_or_else(|| input.new_custom_error(()))?;
        let text = &mut self.text;
        let limited = &mut self.limited;
        text.clear();
        let value = input.parse_until_before(Delimiter::Bang, |input| {
            match input.try_parse(|input| typed_value(property, input, text, limited)) {
                Ok(value) => Ok(value),
                Err(error) => variables::pending(input, limited).map(|source| Value::Pending(property, source)).ok_or(error),
            }
        })?;
        let important = input.try_parse(parse_important).is_ok();
        input.expect_exhausted()?;
        Ok(Declaration { property, value, important })
    }
}
impl<'i> AtRuleParser<'i> for Declarations { type Prelude = (); type AtRule = Declaration; type Error = (); }
impl<'i> QualifiedRuleParser<'i> for Declarations { type Prelude = (); type QualifiedRule = Declaration; type Error = (); }
impl<'i> RuleBodyItemParser<'i, Declaration, ()> for Declarations {
    fn parse_declarations(&self) -> bool { true }
    fn parse_qualified(&self) -> bool { false }
}

/// Normalize only one declaration value. The tokenizer owns CSS escapes,
/// comments and block boundaries; semicolons inside blocks cannot split styles.
/// Bound nested functions independently of the document's DOM depth limit.
fn normalized_value<'i>(input: &mut Parser<'i, '_>, text: &mut String, depth: usize) -> Result<(), ParseError<'i, ()>> {
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if token.is_parse_error() { return Err(input.new_custom_error(())); }
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => text.push(' '),
            Token::Function(_) => {
                if depth >= 16 { return Err(input.new_custom_error(())); }
                token.to_css(text).expect("writing CSS into a String cannot fail");
                input.parse_nested_block(|input| normalized_value(input, text, depth + 1))?;
                text.push(')');
            },
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => return Err(input.new_custom_error(())),
            _ => token.to_css(text).expect("writing CSS into a String cannot fail"),
        }
    }
    Ok(())
}

pub(super) fn style(source: &str) -> crate::Result<Style> {
    let mut input = ParserInput::new(source);
    let mut input = Parser::new(&mut input);
    let mut parser = Declarations { text: String::new(), limited: false };
    let mut style = Style::default();
    let mut important = 0_u16;
    for declaration in RuleBodyParser::new(&mut input, &mut parser) {
        // CSS invalid-declaration recovery preserves earlier valid values.
        let Ok(declaration) = declaration else { continue; };
        let bit = 1_u16 << declaration.property as u8;
        if important & bit != 0 && !declaration.important { continue; }
        if declaration.important { important |= bit; }
        if let Some(values) = &mut style.variable_styles {
            std::sync::Arc::make_mut(values).remove(declaration.property);
            if values.is_empty() { style.variable_styles = None; }
        }
        style.resets.set(declaration.property, None);
        declaration.value.apply(&mut style);
    }
    if parser.limited { return Err(crate::Error::ResourceLimit("富文本样式计算过于复杂".into())); }
    Ok(style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(source: &str) -> Style { super::style(source).expect("bounded CSS") }

    fn variables(source: &str) -> Style {
        style(source).resolve_variables(|name| match name {
            "--Ink" => Some("#123456"), "--n" => Some("400"),
            "--height" => Some("10"), "--empty" => Some(""), _ => None,
        }).into_owned()
    }

    #[test]
    fn font_styles_preserve_authored_angles_and_css_cascade() {
        for (source, expected) in [
            ("normal", FontStyle::Normal), ("italic", FontStyle::Italic),
            ("OBLIQUE", FontStyle::Oblique), (r"ob\6c ique/**/", FontStyle::Oblique),
        ] {
            assert_eq!(style(&format!("font-style:{source}")).font_style, Some(expected), "{source}");
        }
        for (source, degrees) in [
            ("oblique 0deg", 0.), ("oblique -90deg", -90.), ("oblique 90deg", 90.),
            ("oblique .125turn", 45.), ("oblique 50grad", 45.),
            (r"oblique -2.5d\65 g", -2.5), ("oblique 0rad", 0.),
            ("oblique calc(20deg - 1deg)", 19.), ("oblique min(10deg,20deg)", 10.),
            ("oblique atan2(1,1)", 45.), ("oblique calc(sin(90deg)*20deg)", 20.),
            ("oblique calc(120deg)", 90.), ("oblique calc(-120deg)", -90.),
            ("oblique calc(infinity*1deg)", 90.), ("oblique calc(0deg/0)", 0.),
        ] {
            assert_eq!(style(&format!("font-style:{source}")).font_style,
                Some(FontStyle::ObliqueAngle(degrees)), "{source}");
        }
        for source in ["oblique 0", "oblique 91deg", "oblique -91deg", "oblique 10%", "oblique 2px",
            "oblique calc(10)", "oblique calc(1deg+2deg)", "oblique calc(1deg/**/+/**/2deg)",
            "oblique min(1deg,2px)", "oblique 10deg 20deg", "italic 10deg", "normal oblique"] {
            assert_eq!(style(&format!("font-style:italic;font-style:{source}")).font_style,
                Some(FontStyle::Italic), "{source}");
        }
        assert_eq!(style("font-style:oblique 20deg!important;font-style:normal").font_style,
            Some(FontStyle::ObliqueAngle(20.)));
        assert_eq!(style("font-style:oblique 20deg;font-style:normal").font_style, Some(FontStyle::Normal));
        let reset = style("font-style:oblique 20deg;font-style:unset");
        assert_eq!(reset.font_style, None);
        assert_eq!(reset.resets.get(Property::FontStyle), Some(StyleReset::Inherit));
        assert_eq!(variables("font-style:var(--missing,oblique -12deg)").font_style, Some(FontStyle::ObliqueAngle(-12.)));
        assert_eq!(variables("font-style:oblique calc(var(--n)*.05deg)").font_style, Some(FontStyle::ObliqueAngle(20.)));
    }

    #[test]
    fn variable_names_are_case_sensitive_escaped_and_fallbacks_are_tokenized() {
        assert_eq!(variables(r"color:VAR(--\49 nk,blue)").foreground, Some(CssColor::Rgba([0x12, 0x34, 0x56, 255])));
        assert_eq!(variables("color:var(--ink, var(--missing,rgb(1,2,3)))").foreground, Some(CssColor::Rgba([1, 2, 3, 255])));
        assert_eq!(variables("list-style-type:var(--missing,symbols(cyclic \"A\" \"var(--Ink)\"))").list_style,
            style("list-style-type:symbols(cyclic \"A\" \"var(--Ink)\")").list_style);
    }

    #[test]
    fn variable_cascade_distinguishes_parse_errors_from_computed_invalidity() {
        assert_eq!(variables("color:red;color:var(foo,blue)").foreground, style("color:red").foreground);
        let missing = variables("color:red;color:var(--missing)!important;color:blue");
        assert_eq!(missing.foreground, None);
        assert_eq!(missing.resets.get(Property::Color), Some(StyleReset::Inherit));
        let replaced = variables("color:var(--missing)!important;color:green!important");
        assert_eq!(replaced.foreground, style("color:green").foreground);
        assert!(replaced.variable_styles.is_none());
        assert_eq!(variables("color:red;color:var(--empty,blue)").foreground, None);
        assert_eq!(variables("color:red;color:var(--missing,)").resets.get(Property::Color), Some(StyleReset::Inherit));
        assert_eq!(variables("color:var(--Ink,unknown(!;))").foreground, variables("color:var(--Ink)").foreground);
        assert_eq!(variables("color:red;color:var(--missing,!bad)").foreground, style("color:red").foreground);
        let decoration = variables("text-decoration:overline red;text-decoration:underline calc(1px) var(--thickness)");
        assert_eq!(decoration.decoration, None);
        assert_eq!(decoration.resets.get(Property::Decoration), Some(StyleReset::Initial));
    }

    #[test]
    fn substitution_preserves_numeric_token_boundaries_and_operator_whitespace() {
        assert_eq!(variables("font-weight:calc(var(--n) + 100)").weight, style("font-weight:500").weight);
        assert_eq!(variables("font-weight:calc(var(--n)+ 100)").weight, None);
        let joined = variables("vertical-align:var(--height)px");
        assert_eq!(joined.vertical, None);
        assert_eq!(joined.resets.get(Property::Vertical), Some(StyleReset::Initial));
        assert_eq!(variables("vertical-align:calc(var(--height) * 1px)").vertical, style("vertical-align:10px").vertical);
    }

    #[test]
    fn deferred_style_recursion_is_bounded_before_native_adaptation() {
        let source = format!("color:{}red{}", "var(--missing,".repeat(20), ")".repeat(20));
        assert!(super::style(&source).is_err());
        assert!(style("color:blue").variable_styles.is_none());
    }

    #[test]
    fn decoration_calculations_keep_components_and_contextual_length_bases() {
        let source = style(r"text-decoration: UNDERLINE OVERLINE dashed calc(1PX + 10% + .2em) RGB(10/**/ 20 30 / .5)");
        let decoration = source.decoration.expect("valid shorthand");
        assert!(decoration.underline && decoration.overline && !decoration.strike);
        assert_eq!(decoration.pattern, DecorationPattern::Dashed);
        assert_eq!(decoration.color, CssColor::Rgba([10, 20, 30, 128]));
        let DecorationThickness::Calculation(value) = decoration.thickness else { panic!("retained arithmetic"); };
        let evaluate = |font_size| value.evaluate(|unit, value| value * match unit {
            LengthUnit::Em => 20., LengthUnit::Px => 1., _ => panic!("unexpected unit"),
        }, font_size);
        assert_eq!(evaluate(20.), 7.);
        // Explicit inheritance keeps the computing em basis, while the
        // percentage follows the receiving font, as with a literal percentage.
        assert_eq!(evaluate(40.), 9.);
        let source = style(r"text-decoration: line-through clamp(1px, 2vw, 3px) currentColor");
        let decoration = source.decoration.expect("viewport expression");
        assert!(decoration.strike);
        assert_eq!(decoration.color, CssColor::Current);
        let DecorationThickness::Calculation(value) = decoration.thickness else { panic!("viewport arithmetic"); };
        assert_eq!(value.evaluate(|unit, value| value * if unit == LengthUnit::Vw { 0.1 } else { 1. }, 0.), 1.);
        assert_eq!(value.evaluate(|unit, value| value * if unit == LengthUnit::Vw { 4. } else { 1. }, 0.), 3.);
        let source = style("text-decoration: calc(hypot(3pt, 3px)) underline");
        let DecorationThickness::Calculation(value) = source.decoration.unwrap().thickness else { panic!("absolute arithmetic"); };
        assert_eq!(value.evaluate(|unit, value| { assert_eq!(unit, LengthUnit::Px); value }, 0.), 5.);
    }

    #[test]
    fn decoration_calculations_preserve_invalid_declarations_and_resource_limits() {
        let previous = style("text-decoration: overline red").decoration;
        for invalid in ["underline calc(1px/**/+ 2px)", "underline calc(1px +/**/2px)",
            "underline calc(1px + 2s)", "underline calc(2)", "underline calc(0)",
            "underline calc(1px) 2px", "underline rgb(1 2 3) rgb(4 5 6)",
            ] {
            assert_eq!(style(&format!("text-decoration:overline red; text-decoration:{invalid}")).decoration, previous, "{invalid}");
        }
        let source = style(r"text-decoration: calc(1px + 1px) UNDERLINE !important; text-decoration:none");
        assert!(source.decoration.unwrap().underline);
        let source = style("text-decoration: underline calc(1px); text-decoration: inherit !important");
        assert_eq!(source.resets.get(Property::Decoration), Some(StyleReset::Inherit));
        assert!(source.decoration.is_none());
        assert_eq!(style("text-decoration:underline 2PX").decoration.unwrap().thickness,
            DecorationThickness::Length { value: 2., unit: LengthUnit::Px });
        let source = style("text-decoration:underline calc(NaN * 1px)");
        let DecorationThickness::Calculation(value) = source.decoration.unwrap().thickness else { panic!("NaN remains mathematical until used-value resolution"); };
        assert!(value.evaluate(|_, value| value, 0.).is_nan());
        let expression = std::iter::repeat_n("1px", 140).collect::<Vec<_>>().join(" + ");
        assert!(matches!(super::style(&format!("text-decoration:underline calc({expression})")), Err(crate::Error::ResourceLimit(_))));
    }

    fn list_counter(value: &str) -> ListStyle {
        style(&format!("list-style-type:{value}")).list_style.expect("valid list style")
    }

    #[test]
    fn list_style_tokens_preserve_case_escapes_cascade_and_resets() {
        assert_eq!(list_counter(r"LOWER-\67 REEK").marker(25, false).as_ref(), "αα. ");
        assert_eq!(list_counter(r#""Keep\43 ASE""#).marker(3, false).as_ref(), "KeepCASE");
        assert_eq!(list_counter("unregistered-counter").marker(3, false).as_ref(), "3. ");
        assert_eq!(style("list-style-type: upper-alpha; list-style-type:default").list_style, Some(ListStyle::UpperAlpha));
        assert_eq!(style("list-style-type:disc;list-style-type:symbols(numeric 'A')").list_style, Some(ListStyle::Disc));
        assert_eq!(style("list-style-type:disc;list-style-type:'A' extra").list_style, Some(ListStyle::Disc));
        let reset = style("list-style-type: unset !important; list-style-type:'A'");
        assert_eq!(reset.resets.get(Property::List), Some(StyleReset::Inherit));
        assert!(reset.list_style.is_none());
        assert_eq!(list_counter("none").marker(i64::MIN, true).as_ref(), "");
    }

    #[test]
    fn list_predefined_counters_preserve_script_ranges_suffixes_and_padding() {
        for (name, ordinal, expected) in [
            ("bengali", -12, "-১২. "), ("arabic-indic", 0, "٠. "),
            ("hiragana", 49, "ああ、"), ("katakana-iroha", 48, "イイ、"),
            ("hebrew", 15, "טו. "), ("armenian", 1, "Ա. "), ("georgian", 1, "ა. "),
            ("upper-roman", 3999, "MMMCMXCIX. "), ("upper-roman", 4000, "4000. "),
            ("decimal-leading-zero", 1, "01. "), ("decimal-leading-zero", -1, "-1. "),
            ("cjk-earthly-branch", 13, "一三、"), ("cjk-earthly-branch", -1, "-1、"),
            ("cjk-decimal", -1, "-1、"),
        ] { assert_eq!(list_counter(name).marker(ordinal, false).as_ref(), expected, "{name}:{ordinal}"); }
        assert_eq!(list_counter("decimal").marker(i64::MIN, false).as_ref(), "-9223372036854775808. ");
        assert_eq!(list_counter("disclosure-closed").marker(1, true).as_ref(), "◂ ");
    }

    #[test]
    fn list_complex_counters_cover_zero_groups_negatives_and_extended_range() {
        for (name, ordinal, expected) in [
            ("simp-chinese-informal", 0, "零、"), ("simp-chinese-informal", 10, "十、"),
            ("simp-chinese-informal", 1120, "一千一百二十、"),
            ("simp-chinese-informal", 10001, "一万零一、"),
            ("simp-chinese-formal", -10, "负壹拾、"),
            ("simp-chinese-informal", 1_000_000_000_000, "一万亿、"),
            ("trad-chinese-informal", 1_000_000_000_000, "一兆、"),
            ("japanese-formal", 10, "壱拾、"), ("japanese-informal", 101, "百一、"),
            ("korean-hangul-formal", 10, "일십, "), ("korean-hanja-informal", 10000, "萬, "),
            ("ethiopic-numeric", 1, "፩/ "), ("ethiopic-numeric", 100, "፻/ "),
            ("ethiopic-numeric", 101, "፻፩/ "), ("ethiopic-numeric", 10000, "፼/ "),
            ("ethiopic-numeric", 1000000, "፻፼/ "), ("ethiopic-numeric", 0, "0/ "),
        ] { assert_eq!(list_counter(name).marker(ordinal, false).as_ref(), expected, "{name}:{ordinal}"); }
        assert_eq!(list_counter("simp-chinese-informal").marker(i64::MAX, false).as_ref(), "九二二三三七二〇三六八五四七七五八〇七、");
    }

    #[test]
    fn list_symbols_use_native_css_systems_and_bound_untrusted_expansion() {
        for (value, ordinal, expected) in [
            ("symbols(cyclic 'A' 'B')", 0, "B "), ("symbols(cyclic 'A' 'B')", -1, "A "),
            ("symbols(cyclic 'A' 'B')", i64::MIN, "B "),
            ("symbols(fixed 'A' 'B')", 3, "3 "),
            ("symbols(symbolic 'A' 'B')", 3, "AA "),
            ("symbols(alphabetic 'A' 'B')", 3, "AA "),
            ("symbols(numeric 'A' 'B')", -3, "-BB "),
            ("symbols('')", i64::MAX, " "), ("symbols('X')", i64::MAX, "9223372036854775807 "),
        ] { assert_eq!(list_counter(value).marker(ordinal, false).as_ref(), expected, "{value}:{ordinal}"); }
        let unicode = list_counter("symbols('😀')");
        assert_eq!(unicode.marker(150, false).as_ref(), format!("{} ", "😀".repeat(150)));
        assert_eq!(unicode.marker(151, false).as_ref(), "151 ");
    }

    #[test]
    fn list_owned_symbols_are_budgeted_and_literal_markers_stay_shared() {
        let literal = "A".repeat(4096);
        let style = style(&format!("list-style-type:'{literal}'"));
        let list = style.list_style.as_ref().unwrap();
        let super::super::MarkerText::Shared(first) = list.marker(1, false) else { panic!("shared literal"); };
        let super::super::MarkerText::Shared(second) = list.marker(100, false) else { panic!("shared literal"); };
        assert!(std::sync::Arc::ptr_eq(first, second));
        let document = super::super::Document::new(vec![super::super::element(super::super::ElementKind::ListItem, style, vec![])]).unwrap();
        let empty = super::super::Document::new(vec![super::super::element(super::super::ElementKind::ListItem, Style::default(), vec![])]).unwrap();
        assert!(document.weight() >= empty.weight() + literal.len() as u64 + 16);
        assert_eq!(document.text.as_ref(), "");
        assert!(list_counter("symbols('ABC' 'def')").allocated_weight() > 6);
    }

    #[test]
    fn white_space_combinations_preserve_independent_content_semantics() {
        for (source, spaces, breaks, wrap) in [
            ("normal", false, false, true), ("nowrap", false, false, false),
            ("pre", true, true, false), ("pre-wrap", true, true, true),
            ("pre-line", false, true, true), ("break-spaces", true, true, true),
            ("collapse", false, false, true), ("wrap", false, false, true),
            ("collapse nowrap", false, false, false), ("nowrap collapse", false, false, false),
            ("preserve", true, true, true), ("wrap preserve", true, true, true),
            ("preserve nowrap", true, true, false),
            ("preserve-breaks", false, true, true), ("preserve-breaks nowrap", false, true, false),
            ("nowrap preserve-breaks", false, true, false),
            ("break-spaces nowrap", true, true, false), ("wrap break-spaces", true, true, true),
            ("PRESERVE/**/NOWRAP", true, true, false), (r"pre\73 erve-breaks nowrap", false, true, false),
        ] {
            let value = style(&format!("white-space:{source}")).white_space.expect(source);
            assert_eq!((value.preserves_spaces(), value.preserves_breaks(), value.allows_wrap()),
                (spaces, breaks, wrap), "{source}");
        }
    }

    #[test]
    fn white_space_invalid_shorthands_do_not_replace_valid_declarations() {
        for source in ["preserve preserve", "collapse preserve", "wrap nowrap", "nowrap nowrap",
            "pre wrap", "normal nowrap", "preserve-breaks break-spaces", "preserve 1", "\"preserve\"", ""] {
            assert_eq!(style(&format!("white-space:preserve-breaks nowrap;white-space:{source}")).white_space,
                Some(WhiteSpace::PreLineNoWrap), "{source}");
        }
        assert_eq!(style("white-space:preserve nowrap;white-space:wrap").white_space, Some(WhiteSpace::Normal));
        assert_eq!(style("white-space:preserve-breaks nowrap!important;white-space:pre").white_space,
            Some(WhiteSpace::PreLineNoWrap));
        let reset = style("white-space:preserve-breaks nowrap;white-space:inherit");
        assert_eq!(reset.white_space, None);
        assert_eq!(reset.resets.get(Property::Space), Some(StyleReset::Inherit));
    }

    fn offset(source: &str) -> std::sync::Arc<super::super::LengthPercentage> {
        let parsed = style(&format!("vertical-align:{source}"));
        let Some(VerticalAlignment::Offset(value)) = parsed.vertical else { panic!("expected numeric vertical alignment: {source}"); };
        value
    }

    #[test]
    fn vertical_offsets_preserve_signed_units_percentages_and_typed_calculations() {
        for (source, expected) in [
            ("0", 0.), ("-0px", -0.), ("-3pt", -4.), ("25%", 5.), ("-.5em", -7.),
            (r"1e\6d", 14.), (r"1\70 x", 1.),
            ("calc(50% - .5em + 2px)", 5.),
            ("min(5px, 1em)", 5.), ("max(5px, 1em)", 14.),
            ("clamp(20px, 10px, 2px)", 20.), ("clamp(none, 20%, 3px)", 3.),
            ("calc((2em * 3px) / 2px)", 42.), ("calc(1e100em / 1e100)", 14.),
            ("calc(abs(-2em) * sign(-1px))", -28.),
        ] {
            let value = offset(source).evaluate(|unit, value| if unit == LengthUnit::Em { value * 14. }
                else { unit.absolute_pixels(value).expect("absolute test unit") }, 20.);
            assert!((value - expected).abs() < 1e-9, "{source}: {value} != {expected}");
        }
        assert!(offset("calc(0 / 0 * 1px)").evaluate(|_, value| value, 20.).is_nan());
        assert_eq!(offset("calc(-infinity * 1px)").evaluate(|_, value| value, 20.), f64::NEG_INFINITY);
    }

    #[test]
    fn vertical_offsets_share_typed_math_and_preserve_contextual_inputs() {
        let evaluate = |source: &str| offset(source).evaluate(|unit, value| value * match unit {
            LengthUnit::Em => 10., LengthUnit::Vh => 6.,
            _ => unit.absolute_pixels(1.).expect("absolute test unit"),
        }, 20.);
        for (source, expected) in [
            ("round(-1.5em, 1em)", -10.), ("round(up, 1.1em, 1em)", 20.),
            ("round(down, -1.1em, 1em)", -20.), ("round(to-zero, -1.9em, 1em)", -10.),
            ("calc(round(2.5) * 1em)", 30.), ("mod(-2.5em, 2em)", 15.), ("rem(-2.5em, 2em)", -5.),
            ("hypot(3em, 40px)", 50.), ("hypot(-2em)", 20.), ("hypot(30%, 8px)", 10.),
            ("calc(sqrt(9) * pow(2, 3) * 1em)", 240.),
            ("calc(log(exp(2)) * 1vh)", 12.), ("calc(log(8, 2) * 1em)", 30.),
            ("calc(sin(.25turn) * 1em)", 10.), ("calc(cos(180deg) * 1em)", -10.),
            ("calc(tan(pi / 4) * 1em)", 10.), ("calc(asin(1) / 90deg * 1em)", 10.),
            ("calc(acos(-1) / .5turn * 1em)", 10.), ("calc(atan(1) / 50grad * 1em)", 10.),
            ("calc(atan2(1em, 10px) / 45deg * 1em)", 10.),
            ("calc(1em * 1s / 250ms)", 40.), ("calc(1em * 2khz / 1000hz)", 20.),
            ("calc(1em * 192dpi / 1dppx)", 20.),
        ] {
            let value = evaluate(source);
            assert!((value - expected).abs() < 1e-9, "{source}: {value} != {expected}");
        }
        // Absolute calculations no longer ask the renderer for their original
        // operands; relative and percentage inputs must still respond to layout.
        assert_eq!(offset("hypot(3pt, 3px)").evaluate(|unit, value| { assert_eq!(unit, LengthUnit::Px); value }, 0.), 5.);
        assert_eq!(offset("hypot(1em, 0px)").evaluate(|_, value| value * 21., 0.), 21.);
        for invalid in ["sin(2px)", "sqrt(2px)", "calc(pow(2px, 2) * 1px)", "round(1em)",
            "mod(1em, 2)", "hypot(1em, 2s)", "calc(atan2(1em, 2s) / 1deg * 1px)", "calc(log(2s) * 1px)"] {
            assert_eq!(style(&format!("vertical-align:top; vertical-align:{invalid}")).vertical, Some(VerticalAlignment::Top), "{invalid}");
        }
    }

    #[test]
    fn shared_math_keeps_css_nonfinite_and_signed_zero_rules() {
        let evaluate = |source: &str| offset(source).evaluate(|_, value| value, 20.);
        for source in ["round(1px, 0px)", "mod(1px, 0px)", "rem(1px, 0px)",
            "calc(pow(1, infinity) * 1px)", "hypot(infinity * 1px, NaN * 1px)"] {
            assert!(evaluate(source).is_nan(), "{source}");
        }
        for source in ["calc(sign(-0) * 1px)", "rem(-4px, 2px)", "round(-.4px, 1px)"] {
            let value = evaluate(source);
            assert_eq!(value, 0., "{source}"); assert!(value.is_sign_negative(), "{source}");
        }
        for (source, expected) in [
            ("calc(100 * log(exp(5)))", 500.), ("calc(100 * sin(.25turn))", 100.),
            ("calc(100 * atan2(1s, 1000ms) / 45deg)", 100.),
            ("round(nearest, 450, 100)", 500.), ("calc(pow(1, infinity) * 500)", 1.),
        ] {
            assert_eq!(style(&format!("font-weight:{source}")).weight, Some(Weight::Absolute(expected)), "{source}");
        }
    }

    #[test]
    fn vertical_offsets_keep_computing_lengths_and_receiver_line_height_separate() {
        let expression = offset("calc(1em + 50% + 1vh)");
        let units = expression.units().collect::<Vec<_>>();
        assert_eq!(units, [LengthUnit::Em, LengthUnit::Vh]);
        let computing_basis = |unit, value| value * match unit { LengthUnit::Em => 14., LengthUnit::Vh => 6., _ => panic!("unexpected unit") };
        assert_eq!(expression.evaluate(computing_basis, 20.), 30.);
        assert_eq!(expression.evaluate(computing_basis, 40.), 40.);
        let resized = expression.evaluate(|unit, value| value * if unit == LengthUnit::Vh { 8. } else { 14. }, 40.);
        assert_eq!(resized, 42.);
        assert_eq!(style("vertical-align:2px !important; vertical-align:top").vertical, Some(VerticalAlignment::Offset(offset("2px"))));
        for (keyword, expected) in [("inherit", StyleReset::Inherit), ("unset", StyleReset::Initial)] {
            let inherited = style(&format!("vertical-align:2em; vertical-align:{keyword}"));
            assert_eq!(inherited.vertical, None);
            assert_eq!(inherited.resets.get(Property::Vertical), Some(expected));
        }
        // Font-weight shares the original numeric spelling reader, including
        // escaped suffixes and dimensional cancellation.
        assert_eq!(style(r"font-weight:calc(500 * 1e2\70 x / 100px)").weight, Some(Weight::Absolute(500.)));
    }

    #[test]
    fn vertical_offsets_reject_invalid_types_spacing_and_excessive_work() {
        let nested = format!("{}1px{}", "calc(".repeat(17), ")".repeat(17));
        let long = format!("calc({})", vec!["1px"; 130].join(" + "));
        for invalid in ["2", "calc(0)", "calc(1em + 0)", "calc(1em * 2em)", "1px 2px", "(1px)",
            "calc(1em+ 2px)", "calc(1em/**/+/**/2px)", "clamp(1px, 2, 3px)"] {
            assert_eq!(style(&format!("vertical-align:top; vertical-align:{invalid}")).vertical,
                Some(VerticalAlignment::Top), "{invalid}");
        }
        for limited in [nested, long] {
            let declaration = format!("vertical-align:top; vertical-align:{limited}");
            assert!(matches!(super::style(&declaration), Err(crate::Error::ResourceLimit(_))));
            let html = format!("<p><span style=\"{declaration}\">content</span></p>");
            assert!(matches!(super::super::html(&html), Err(crate::Error::ResourceLimit(_))));
        }
    }

    #[test]
    fn match_parent_alignment_parses_cascade_and_parent_direction() {
        for (source, parent, rtl, expected) in [
            ("text-align:match-parent", Alignment::Start, false, Alignment::Left),
            ("text-align:match-parent", Alignment::Start, true, Alignment::Right),
            ("text-align:match-parent", Alignment::End, false, Alignment::Right),
            ("text-align:match-parent", Alignment::End, true, Alignment::Left),
            (r"text-align:m\61 tch-parent", Alignment::Start, true, Alignment::Right),
            ("text-align:right; text-align:MATCH-PARENT !important; text-align:left", Alignment::End, true, Alignment::Left),
            ("text-align:right; text-align:match-parent extra", Alignment::End, true, Alignment::Right),
        ] {
            let specified = style(source).alignment.expect("valid alignment survives cascade");
            assert_eq!(specified.resolve_match_parent(Some((parent, rtl))), expected, "{source}");
        }
    }

    #[test]
    fn match_parent_alignment_keeps_computed_edges_and_normal_inheritance() {
        let matched = style("text-align:match-parent").alignment.expect("match-parent");
        assert_eq!(matched.resolve_match_parent(None), Alignment::Start);
        for alignment in [Alignment::Left, Alignment::Right, Alignment::Center, Alignment::Justify] {
            for rtl in [false, true] {
                assert_eq!(matched.resolve_match_parent(Some((alignment, rtl))), alignment);
            }
        }
        let parent = matched.resolve_match_parent(Some((Alignment::Start, true)));
        assert_eq!(matched.resolve_match_parent(Some((parent, false))), Alignment::Right);
        for value in [Alignment::Start, Alignment::End] {
            assert_eq!(value.resolve_match_parent(Some((parent, true))), value);
        }
        for keyword in ["inherit", "unset"] {
            let inherited = style(&format!("text-align:match-parent; text-align:{keyword}"));
            assert_eq!(inherited.alignment, None);
            assert_eq!(inherited.resets.get(Property::Alignment), Some(StyleReset::Inherit));
        }
    }
}
