use cssparser::Parser;
use super::{Quantity, Result, consistent, number, sum};
pub(super) use crate::preview::rich::math::Trigonometric as Operation;

impl Operation {
    pub(super) fn evaluate<'i>(self, input: &mut Parser<'i, '_>, depth: u8) -> Result<'i, Quantity> {
        match self {
            Self::Sin | Self::Cos | Self::Tan => {
                let value = sum(input, depth)?;
                if !value.is_angle() && !value.is_number() { return Err(input.new_custom_error(())); }
                Ok(Quantity::number(self.unary(value.value, value.is_angle())))
            },
            Self::Asin | Self::Acos | Self::Atan => Ok(Quantity::angle(self.unary(number(input, depth)?, false))),
            Self::Atan2 => {
                let y = sum(input, depth)?; input.expect_comma()?;
                let x = sum(input, depth)?; consistent(y, x, input)?;
                Ok(Quantity::angle(y.value.atan2(x.value).to_degrees()))
            },
        }
    }
}
