//! CSS stepped calculations preserve operand types without retaining an AST.
use cssparser::Parser;
use super::{Quantity, Result, consistent, sum};

use crate::preview::rich::math::{RoundingStrategy as Strategy, rounded};
pub(super) use crate::preview::rich::math::remainder;

pub(super) fn round<'i>(input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Quantity> {
    let strategy = input.try_parse(|input| -> Result<'i, Strategy> {
        let name = input.expect_ident()?;
        Strategy::named(name).ok_or_else(|| input.new_custom_error(()))
    });
    let strategy = if let Ok(strategy) = strategy {
        input.expect_comma()?;
        strategy
    } else { Strategy::Nearest };
    let value = sum(input, depth)?;
    let step = if input.is_exhausted() {
        // Dimension calculations always need an explicit, compatible step.
        if !value.is_number() { return Err(input.new_custom_error(())); }
        Quantity::number(1.0)
    } else {
        input.expect_comma()?;
        sum(input, depth)?
    };
    consistent(value, step, input)?;
    Ok(value.with_value(rounded(value.value, step.value.abs(), strategy)))
}

