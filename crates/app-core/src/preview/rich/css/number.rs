//! Constant CSS calculations are consumed directly from CSS tokens. No
//! expression tree or normalized string survives in the controlled document.
use cssparser::{ParseError, Parser, SourcePosition, Token};
use super::{FontStyle, Property, Value, Weight, reset};

mod stepped;
mod quantity;
mod trig;
pub(super) use quantity::Quantity;
use super::super::math::{self, compare, logarithm};

type Result<'i, T> = std::result::Result<T, ParseError<'i, ()>>;
const MAX_DEPTH: u8 = 16;

pub(super) fn font_style<'i>(input: &mut Parser<'i, '_>) -> Result<'i, Value> {
    let name = input.expect_ident_cloned()?;
    let value = if let Some(reset) = reset(Property::FontStyle, &name) {
        Value::Reset(Property::FontStyle, reset)
    } else if name.eq_ignore_ascii_case("normal") { Value::FontStyle(FontStyle::Normal) }
    else if name.eq_ignore_ascii_case("italic") { Value::FontStyle(FontStyle::Italic) }
    else if name.eq_ignore_ascii_case("oblique") {
        if input.is_exhausted() { return Ok(Value::FontStyle(FontStyle::Oblique)); }
        input.skip_whitespace();
        let start = input.position();
        let (angle, calculated) = match input.next()?.clone() {
            Token::Dimension { unit, .. } => (
                Quantity::dimension(input.slice_from(start), &unit)
                    .ok_or_else(|| input.new_custom_error(()))?, false),
            Token::Function(name) => (input.parse_nested_block(|input| function(input, &name, 1))?, true),
            _ => return Err(input.new_custom_error(())),
        };
        if !angle.is_angle() { return Err(input.new_custom_error(())); }
        let degrees = if calculated {
            // CSS calculation ranges apply at computed-value time; NaN is
            // censored to zero before the outer property's range is applied.
            (if angle.value.is_nan() { 0.0 } else { angle.value }).clamp(-90.0, 90.0)
        } else {
            if !(-90.0..=90.0).contains(&angle.value) { return Err(input.new_custom_error(())); }
            angle.value
        };
        Value::FontStyle(FontStyle::ObliqueAngle(degrees as f32))
    } else { return Err(input.new_custom_error(())); };
    input.expect_exhausted()?;
    Ok(value)
}

pub(super) fn weight<'i>(input: &mut Parser<'i, '_>) -> Result<'i, Value> {
    input.skip_whitespace();
    let start = input.position();
    let value = match input.next()?.clone() {
        Token::Ident(name) => {
            if let Some(reset) = reset(Property::Weight, &name) { Value::Reset(Property::Weight, reset) }
            else {
                let weight = if name.eq_ignore_ascii_case("normal") { Weight::Absolute(400.0) }
                    else if name.eq_ignore_ascii_case("bold") { Weight::Absolute(700.0) }
                    else if name.eq_ignore_ascii_case("bolder") { Weight::Bolder }
                    else if name.eq_ignore_ascii_case("lighter") { Weight::Lighter }
                    else { return Err(input.new_custom_error(())); };
                Value::Weight(weight)
            }
        },
        Token::Number { .. } => {
            let number = literal(input, start)?;
            if !(1.0..=1000.0).contains(&number) { return Err(input.new_custom_error(())); }
            Value::Weight(Weight::Absolute(number as f32))
        },
        Token::Function(name) => {
            let result = input.parse_nested_block(|input| function(input, &name, 1))?;
            if !result.is_number() { return Err(input.new_custom_error(())); }
            // Only the outer calculation censors NaN and applies the property's
            // range. Nested arithmetic retains infinities and signed zero.
            let number = if result.value.is_nan() { 0.0 } else { result.value };
            Value::Weight(Weight::Absolute(number.clamp(1.0, 1000.0) as f32))
        },
        _ => return Err(input.new_custom_error(())),
    };
    input.expect_exhausted()?;
    Ok(value)
}

fn function<'i>(input: &mut Parser<'i, '_>, name: &str, depth: u8) -> Result<'i, Quantity> {
    let value = if name.eq_ignore_ascii_case("calc") {
        sum(input, depth)?
    } else if name.eq_ignore_ascii_case("min") || name.eq_ignore_ascii_case("max") {
        let maximum = name.eq_ignore_ascii_case("max");
        let mut value = sum(input, depth)?;
        while !input.is_exhausted() {
            input.expect_comma()?;
            let next = sum(input, depth)?;
            consistent(value, next, input)?;
            value.value = compare(value.value, next.value, maximum);
        }
        value
    } else if name.eq_ignore_ascii_case("clamp") {
        let minimum = bound(input, depth)?;
        input.expect_comma()?;
        let value = sum(input, depth)?;
        input.expect_comma()?;
        let maximum = bound(input, depth)?;
        for bound in [minimum, maximum].into_iter().flatten() { consistent(value, bound, input)?; }
        // A reversed interval gives the lower bound priority.
        value.with_value(compare(minimum.map_or(f64::NEG_INFINITY, |bound| bound.value),
            compare(value.value, maximum.map_or(f64::INFINITY, |bound| bound.value), false), true))
    } else if name.eq_ignore_ascii_case("round") {
        stepped::round(input, depth)?
    } else if name.eq_ignore_ascii_case("mod") || name.eq_ignore_ascii_case("rem") {
        let value = sum(input, depth)?;
        input.expect_comma()?;
        let step = sum(input, depth)?;
        consistent(value, step, input)?;
        value.with_value(stepped::remainder(value.value, step.value, name.eq_ignore_ascii_case("mod")))
    } else if name.eq_ignore_ascii_case("abs") {
        let value = sum(input, depth)?;
        value.with_value(value.value.abs())
    } else if name.eq_ignore_ascii_case("sign") {
        let value = sum(input, depth)?.value;
        // Rust's signum maps both zeroes to +/-1; CSS retains the input zero.
        Quantity::number(math::sign(value))
    } else if name.eq_ignore_ascii_case("sqrt") {
        Quantity::number(number(input, depth)?.sqrt())
    } else if name.eq_ignore_ascii_case("exp") {
        Quantity::number(number(input, depth)?.exp())
    } else if name.eq_ignore_ascii_case("pow") {
        let base = number(input, depth)?;
        input.expect_comma()?;
        let exponent = number(input, depth)?;
        // CSS propagates NaN even for NaN^0 and 1^NaN. Infinite exponents
        // with a base of +/-1 are also NaN, unlike the native pow operation.
        Quantity::number(math::power(base, exponent))
    } else if name.eq_ignore_ascii_case("hypot") {
        let mut value = sum(input, depth)?;
        value.value = value.value.abs();
        while !input.is_exhausted() {
            input.expect_comma()?;
            let component = sum(input, depth)?;
            consistent(value, component, input)?;
            // Native hypot avoids intermediate square overflow/underflow,
            // but its infinity-over-NaN precedence differs from CSS.
            value.value = math::hypot(value.value, component.value);
        }
        value
    } else if name.eq_ignore_ascii_case("log") {
        let value = number(input, depth)?;
        let base = if input.is_exhausted() { std::f64::consts::E } else {
            input.expect_comma()?;
            number(input, depth)?
        };
        Quantity::number(logarithm(value, base))
    } else if let Some(operation) = trig::Operation::named(name) {
        operation.evaluate(input, depth)?
    } else { return Err(input.new_custom_error(())); };
    input.expect_exhausted()?;
    Ok(value)
}

fn number<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, f64> {
    let value = sum(input, depth)?;
    if value.is_number() { Ok(value.value) } else { Err(input.new_custom_error(())) }
}

fn consistent<'i>(left: Quantity, right: Quantity, input: &Parser<'i, '_>) -> Result<'i, ()> {
    if left.same_type(right) { Ok(()) } else { Err(input.new_custom_error(())) }
}

fn bound<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Option<Quantity>> {
    if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() { Ok(None) }
    else { sum(input, depth).map(Some) }
}

fn sum<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Quantity> {
    let mut value = product(input, depth)?;
    loop {
        let state = input.state();
        let before = whitespace(input);
        let operator = match input.next_including_whitespace_and_comments() {
            Ok(Token::Delim(operator @ ('+' | '-'))) => *operator,
            _ => { input.reset(&state); break; },
        };
        if !before || !whitespace(input) { return Err(input.new_custom_error(())); }
        let right = product(input, depth)?;
        consistent(value, right, input)?;
        value.value = if operator == '+' { value.value + right.value } else { value.value - right.value };
    }
    Ok(value)
}

fn product<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Quantity> {
    let mut value = atom(input, depth)?;
    loop {
        let operator = input.try_parse(|input| -> Result<'i, char> {
            match input.next()? {
                Token::Delim(operator @ ('*' | '/')) => Ok(*operator),
                _ => Err(input.new_custom_error(())),
            }
        });
        let Ok(operator) = operator else { break; };
        let right = atom(input, depth)?;
        value = value.product(right, operator == '/').ok_or_else(|| input.new_custom_error(()))?;
    }
    Ok(value)
}

fn atom<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Quantity> {
    input.skip_whitespace();
    let start = input.position();
    match input.next()?.clone() {
        Token::Number { .. } => literal(input, start).map(Quantity::number),
        Token::Percentage { .. } => literal_prefix(input.slice_from(start)).map(|value| Quantity::percentage(value / 100.)).ok_or_else(|| input.new_custom_error(())),
        Token::Dimension { unit, .. } => Quantity::dimension(input.slice_from(start), &unit).ok_or_else(|| input.new_custom_error(())),
        Token::Ident(name) => {
            if name.eq_ignore_ascii_case("e") { Ok(Quantity::number(std::f64::consts::E)) }
            else if name.eq_ignore_ascii_case("pi") { Ok(Quantity::number(std::f64::consts::PI)) }
            else if name.eq_ignore_ascii_case("infinity") { Ok(Quantity::number(f64::INFINITY)) }
            else if name.eq_ignore_ascii_case("-infinity") { Ok(Quantity::number(f64::NEG_INFINITY)) }
            else if name.eq_ignore_ascii_case("nan") { Ok(Quantity::number(f64::NAN)) }
            else { Err(input.new_custom_error(())) }
        },
        Token::Function(name) if depth < MAX_DEPTH => input.parse_nested_block(|input| function(input, &name, depth + 1)),
        Token::ParenthesisBlock if depth < MAX_DEPTH => input.parse_nested_block(|input| sum(input, depth + 1)),
        _ => Err(input.new_custom_error(())),
    }
}

fn literal<'i>(input: &Parser<'i, '_>, start: SourcePosition) -> Result<'i, f64> {
    // Token::Number contains f32. Borrow the token's original spelling so
    // intermediate arithmetic does not overflow or round at that boundary.
    let value = input.slice_from(start).parse::<f64>().map_err(|_| input.new_custom_error(()))?;
    Ok(value.clamp(-f64::MAX, f64::MAX))
}

pub(super) fn whitespace(input: &mut Parser<'_, '_>) -> bool {
    let mut found = false;
    loop {
        let state = input.state();
        match input.next_including_whitespace_and_comments() {
            Ok(Token::WhiteSpace(_)) => found = true,
            Ok(Token::Comment(_)) => {},
            _ => { input.reset(&state); break; },
        }
    }
    found
}

/// Numeric spelling before a tokenizer-validated dimension/percentage suffix.
/// The suffix may contain escapes, so its decoded byte length is not usable.
pub(super) fn literal_prefix(source: &str) -> Option<f64> {
    let bytes = source.as_bytes();
    let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    while bytes.get(end).is_some_and(u8::is_ascii_digit) { end += 1; }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) { end += 1; }
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) { exponent += 1; }
        if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            end = exponent + 1;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) { end += 1; }
        }
    }
    Some(source[..end].parse::<f64>().ok()?.clamp(-f64::MAX, f64::MAX))
}

/// Percentages keep their own dimension while evaluating a color weight.
pub(super) fn percentage<'i>(input: &mut Parser<'i, '_>) -> Result<'i, f64> {
    input.skip_whitespace();
    let start = input.position();
    match input.next()?.clone() {
        Token::Percentage { .. } => {
            let value = literal_prefix(input.slice_from(start)).ok_or_else(|| input.new_custom_error(()))? / 100.;
            if !(0.0..=1.0).contains(&value) { return Err(input.new_custom_error(())); }
            Ok(value)
        },
        Token::Function(name) => {
            let value = input.parse_nested_block(|input| function(input, &name, 1))?;
            if !value.is_percentage() { return Err(input.new_custom_error(())); }
            Ok(if value.value.is_nan() { 0.0 } else { value.value.clamp(0.0, 1.0) })
        },
        _ => Err(input.new_custom_error(())),
    }
}
