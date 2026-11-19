//! Keep CSS calculations typed until native font/viewport metrics are known.
use std::sync::Arc;
use cssparser::{ParseError, Parser, Token};
use super::{number::{self, Quantity}, Property, Value, VerticalAlignment, reset};
use super::super::math::{RoundingStrategy, Trigonometric};
use super::super::{LengthPercentage, LengthUnit, length::Step};

type Result<'i, T> = std::result::Result<T, ParseError<'i, ()>>;
#[derive(Clone, Copy)]
struct Operand { index: u16, kind: Quantity }
#[derive(Default)]
struct Program { steps: Vec<Step>, limited: bool, channels: Option<[&'static str; 4]> }

pub(super) fn vertical<'i>(input: &mut Parser<'i, '_>, limited: &mut bool) -> Result<'i, Value> {
    if let Ok(name) = input.try_parse(|input| input.expect_ident_cloned()) {
        input.expect_exhausted()?;
        if let Some(reset) = reset(Property::Vertical, &name) { return Ok(Value::Reset(Property::Vertical, reset)); }
        return Value::from_normalized(Property::Vertical, &name.to_ascii_lowercase()).ok_or_else(|| input.new_custom_error(()));
    }
    let value = calculation(input, limited)?;
    input.expect_exhausted()?;
    Ok(Value::Vertical(VerticalAlignment::Offset(value)))
}

/// One numeric component, leaving the remaining shorthand tokens untouched.
pub(super) fn calculation<'i>(input: &mut Parser<'i, '_>, limited: &mut bool) -> Result<'i, Arc<LengthPercentage>> {
    let start = input.state();
    let literal = matches!(input.next(), Ok(Token::Number { .. }));
    input.reset(&start);
    let mut program = Program::default();
    let value = program.atom(input, 0);
    *limited |= program.limited;
    let value = value?;
    // A literal unitless zero is a length; calc(0) remains a number.
    let literal_zero = literal && program.steps.len() == 1 && matches!(program.steps[0], Step::Number(value) if value == 0.);
    if literal_zero { program.steps[0] = Step::Length(0., LengthUnit::Px); }
    else if !value.kind.is_length() { return Err(input.new_custom_error(())); }
    Ok(Arc::new(LengthPercentage::new(program.steps.into_boxed_slice())))
}

impl Program {
    fn push<'i>(&mut self, step: Step, kind: Quantity, input: &Parser<'i, '_>) -> Result<'i, Operand> {
        if self.steps.len() >= LengthPercentage::MAX_STEPS { self.limited = true; return Err(input.new_custom_error(())); }
        let index = self.steps.len() as u16;
        self.steps.push(step);
        Ok(Operand { index, kind: kind.with_value(1.) })
    }

    fn atom<'i>(&mut self, input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Operand> {
        input.skip_whitespace();
        let start = input.position();
        match input.next()?.clone() {
            Token::Number { .. } => {
                let value = number::literal_prefix(input.slice_from(start)).ok_or_else(|| input.new_custom_error(()))?;
                self.push(Step::Number(value), Quantity::number(1.), input)
            },
            Token::Dimension { unit, .. } => {
                if let Some(unit) = LengthUnit::parse(&unit.to_ascii_lowercase()) {
                    if self.channels.is_some() { return Err(input.new_custom_error(())); }
                    let value = number::literal_prefix(input.slice_from(start)).ok_or_else(|| input.new_custom_error(()))?;
                    self.push(Step::Length(value, unit), Quantity::length(1.), input)
                } else {
                    let quantity = Quantity::dimension(input.slice_from(start), &unit).ok_or_else(|| input.new_custom_error(()))?;
                    self.push(Step::Number(quantity.value), quantity, input)
                }
            },
            Token::Percentage { .. } => {
                let value = number::literal_prefix(input.slice_from(start)).ok_or_else(|| input.new_custom_error(()))?;
                self.push(Step::Percentage(value / 100.), if self.channels.is_some() { Quantity::percentage(1.) } else { Quantity::length(1.) }, input)
            },
            Token::Function(name) if depth < 16 => input.parse_nested_block(|input| self.function(input, &name, depth + 1)),
            Token::ParenthesisBlock if depth > 0 && depth < 16 => input.parse_nested_block(|input| {
                let value = self.sum(input, depth + 1)?; input.expect_exhausted()?; Ok(value)
            }),
            Token::Function(_) | Token::ParenthesisBlock if depth >= 16 => {
                self.limited = true; Err(input.new_custom_error(()))
            },
            Token::Ident(name) if depth > 0 => {
                if let Some(names) = self.channels {
                    if let Some(index) = names.iter().position(|channel| name.eq_ignore_ascii_case(channel)) {
                        return self.push(Step::Channel(index as u8), Quantity::number(1.), input);
                    }
                }
                let value = if name.eq_ignore_ascii_case("e") { std::f64::consts::E }
                    else if name.eq_ignore_ascii_case("pi") { std::f64::consts::PI }
                    else if name.eq_ignore_ascii_case("infinity") { f64::INFINITY }
                    else if name.eq_ignore_ascii_case("-infinity") { f64::NEG_INFINITY }
                    else if name.eq_ignore_ascii_case("nan") { f64::NAN }
                    else { return Err(input.new_custom_error(())); };
                self.push(Step::Number(value), Quantity::number(1.), input)
            },
            _ => Err(input.new_custom_error(())),
        }
    }

    fn sum<'i>(&mut self, input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Operand> {
        let mut value = self.product(input, depth)?;
        loop {
            let state = input.state();
            let before = number::whitespace(input);
            let operator = match input.next_including_whitespace_and_comments() {
                Ok(Token::Delim(operator @ ('+' | '-'))) => *operator,
                _ => { input.reset(&state); break; },
            };
            if !before || !number::whitespace(input) { return Err(input.new_custom_error(())); }
            let right = self.product(input, depth)?;
            if !value.kind.same_type(right.kind) { return Err(input.new_custom_error(())); }
            let step = if operator == '+' { Step::Add(value.index, right.index) } else { Step::Subtract(value.index, right.index) };
            value = self.push(step, value.kind, input)?;
        }
        Ok(value)
    }

    fn product<'i>(&mut self, input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Operand> {
        let mut value = self.atom(input, depth)?;
        loop {
            let operator = input.try_parse(|input| -> Result<'i, char> {
                match input.next()? { Token::Delim(operator @ ('*' | '/')) => Ok(*operator), _ => Err(input.new_custom_error(())) }
            });
            let Ok(operator) = operator else { break; };
            let right = self.atom(input, depth)?;
            let kind = value.kind.product(right.kind, operator == '/').ok_or_else(|| input.new_custom_error(()))?;
            let step = if operator == '*' { Step::Multiply(value.index, right.index) } else { Step::Divide(value.index, right.index) };
            value = self.push(step, kind, input)?;
        }
        Ok(value)
    }

    fn function<'i>(&mut self, input: &mut Parser<'i, '_>, name: &str, depth: u8) -> Result<'i, Operand> {
        let value = if name.eq_ignore_ascii_case("calc") { self.sum(input, depth)? }
        else if name.eq_ignore_ascii_case("min") || name.eq_ignore_ascii_case("max") || name.eq_ignore_ascii_case("hypot") {
            let maximum = name.eq_ignore_ascii_case("max");
            let hypot = name.eq_ignore_ascii_case("hypot");
            let mut value = self.sum(input, depth)?;
            if hypot { value = self.push(Step::Abs(value.index), value.kind, input)?; }
            while !input.is_exhausted() {
                input.expect_comma()?;
                let right = self.sum(input, depth)?;
                self.consistent(value, right, input)?;
                let step = if hypot { Step::Hypot(value.index, right.index) }
                    else if maximum { Step::Max(value.index, right.index) } else { Step::Min(value.index, right.index) };
                value = self.push(step, value.kind, input)?;
            }
            value
        } else if name.eq_ignore_ascii_case("clamp") {
            let minimum = self.bound(input, depth)?; input.expect_comma()?;
            let mut value = self.sum(input, depth)?; input.expect_comma()?;
            let maximum = self.bound(input, depth)?;
            for bound in [minimum, maximum].into_iter().flatten() { self.consistent(value, bound, input)?; }
            if let Some(maximum) = maximum { value = self.push(Step::Min(value.index, maximum.index), value.kind, input)?; }
            if let Some(minimum) = minimum { value = self.push(Step::Max(minimum.index, value.index), value.kind, input)?; }
            value
        } else if name.eq_ignore_ascii_case("round") {
            let strategy = input.try_parse(|input| -> Result<'i, RoundingStrategy> {
                let name = input.expect_ident()?;
                RoundingStrategy::named(name).ok_or_else(|| input.new_custom_error(()))
            });
            let strategy = if let Ok(strategy) = strategy { input.expect_comma()?; strategy } else { RoundingStrategy::Nearest };
            let value = self.sum(input, depth)?;
            let step = if input.is_exhausted() {
                if !value.kind.is_number() { return Err(input.new_custom_error(())); }
                self.push(Step::Number(1.), Quantity::number(1.), input)?
            } else { input.expect_comma()?; self.sum(input, depth)? };
            self.consistent(value, step, input)?;
            self.push(Step::Round(value.index, step.index, strategy), value.kind, input)?
        } else if name.eq_ignore_ascii_case("mod") || name.eq_ignore_ascii_case("rem") {
            let value = self.sum(input, depth)?; input.expect_comma()?;
            let step = self.sum(input, depth)?;
            self.consistent(value, step, input)?;
            let operation = if name.eq_ignore_ascii_case("mod") { Step::Mod(value.index, step.index) } else { Step::Rem(value.index, step.index) };
            self.push(operation, value.kind, input)?
        } else if name.eq_ignore_ascii_case("abs") || name.eq_ignore_ascii_case("sign") {
            let value = self.sum(input, depth)?;
            if name.eq_ignore_ascii_case("abs") { self.push(Step::Abs(value.index), value.kind, input)? }
            else { self.push(Step::Sign(value.index), Quantity::number(1.), input)? }
        } else if name.eq_ignore_ascii_case("sqrt") || name.eq_ignore_ascii_case("exp") {
            let value = self.number(input, depth)?;
            let step = if name.eq_ignore_ascii_case("sqrt") { Step::Sqrt(value.index) } else { Step::Exp(value.index) };
            self.push(step, Quantity::number(1.), input)?
        } else if name.eq_ignore_ascii_case("pow") || name.eq_ignore_ascii_case("log") {
            let value = self.number(input, depth)?;
            let right = if name.eq_ignore_ascii_case("log") && input.is_exhausted() {
                self.push(Step::Number(std::f64::consts::E), Quantity::number(1.), input)?
            } else { input.expect_comma()?; self.number(input, depth)? };
            let step = if name.eq_ignore_ascii_case("pow") { Step::Power(value.index, right.index) } else { Step::Log(value.index, right.index) };
            self.push(step, Quantity::number(1.), input)?
        } else if let Some(operation) = Trigonometric::named(name) {
            match operation {
                Trigonometric::Sin | Trigonometric::Cos | Trigonometric::Tan => {
                    let value = self.sum(input, depth)?;
                    if !value.kind.is_number() && !value.kind.is_angle() { return Err(input.new_custom_error(())); }
                    self.push(Step::Trigonometric(operation, value.index, value.kind.is_angle()), Quantity::number(1.), input)?
                },
                Trigonometric::Asin | Trigonometric::Acos | Trigonometric::Atan => {
                    let value = self.number(input, depth)?;
                    self.push(Step::Trigonometric(operation, value.index, false), Quantity::angle(1.), input)?
                },
                Trigonometric::Atan2 => {
                    let y = self.sum(input, depth)?; input.expect_comma()?;
                    let x = self.sum(input, depth)?;
                    self.consistent(y, x, input)?;
                    self.push(Step::Atan2(y.index, x.index), Quantity::angle(1.), input)?
                },
            }
        } else { return Err(input.new_custom_error(())); };
        input.expect_exhausted()?;
        Ok(value)
    }

    fn consistent<'i>(&self, left: Operand, right: Operand, input: &Parser<'i, '_>) -> Result<'i, ()> {
        if left.kind.same_type(right.kind) { Ok(()) } else { Err(input.new_custom_error(())) }
    }

    fn number<'i>(&mut self, input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Operand> {
        let value = self.sum(input, depth)?;
        if value.kind.is_number() { Ok(value) } else { Err(input.new_custom_error(())) }
    }

    fn bound<'i>(&mut self, input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Option<Operand>> {
        if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() { Ok(None) }
        else { self.sum(input, depth).map(Some) }
    }
}

/// Color channel math shares the length parser's typed, bounded program.
/// It has no font/viewport length operands and retains no source CSS.
pub(super) fn channel_calculation<'i>(
    input: &mut Parser<'i, '_>, names: [&'static str; 4], hue: bool,
) -> Result<'i, (Box<[Step]>, Quantity)> {
    let mut program = Program { channels: Some(names), ..Default::default() };
    let value = program.atom(input, 0)?;
    if !(value.kind.is_number() || if hue { value.kind.is_angle() } else { value.kind.is_percentage() }) {
        return Err(input.new_custom_error(()));
    }
    Ok((program.steps.into_boxed_slice(), value.kind))
}
