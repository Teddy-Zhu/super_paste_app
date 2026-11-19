//! Validated, bounded CSS length calculations. A flat program avoids recursive
//! evaluation and retains no CSS text. Alignment and decoration share it.
use super::LengthUnit;
use super::math::{self, RoundingStrategy, Trigonometric};

#[derive(Clone, Debug, PartialEq)]
pub struct LengthPercentage { pub(super) steps: Box<[Step]> }

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Step {
    Number(f64), Length(f64, LengthUnit), Percentage(f64), Channel(u8),
    Add(u16, u16), Subtract(u16, u16), Multiply(u16, u16), Divide(u16, u16),
    Min(u16, u16), Max(u16, u16), Abs(u16), Sign(u16),
    Round(u16, u16, RoundingStrategy), Mod(u16, u16), Rem(u16, u16),
    Sqrt(u16), Exp(u16), Power(u16, u16), Hypot(u16, u16), Log(u16, u16),
    Trigonometric(Trigonometric, u16, bool), Atan2(u16, u16),
}

impl LengthPercentage {
    pub(super) const MAX_STEPS: usize = 256;
    pub(super) fn new(steps: Box<[Step]>) -> Self {
        let mut source = Self { steps };
        if source.steps.len() == 1 && matches!(source.steps[0], Step::Length(_, LengthUnit::Px)) { return source; }
        let constant = source.steps.iter().all(|step| match step {
            Step::Percentage(_) => false,
            Step::Length(_, unit) => unit.absolute_pixels(1.).is_some(),
            _ => true,
        });
        if constant {
            // Resolve absolute units before publishing the document. Contextual
            // programs keep their original font/viewport/percentage operands.
            let value = source.evaluate(|unit, value| unit.absolute_pixels(1.).expect("absolute unit") * value, 0.);
            source.steps = Box::new([Step::Length(value, LengthUnit::Px)]);
        }
        source
    }
    pub fn units(&self) -> impl Iterator<Item = LengthUnit> + '_ {
        self.steps.iter().filter_map(|step| match step { Step::Length(_, unit) => Some(*unit), _ => None })
    }

    /// Font/viewport lengths are supplied by the computing element. Percentage
    /// uses the consuming property's basis (line height or font size), including
    /// explicit inheritance of an unresolved percentage.
    /// Intermediate arithmetic stays f64; the renderer clamps only the result.
    pub fn evaluate(&self, length: impl FnMut(LengthUnit, f64) -> f64, percentage_basis: f64) -> f64 {
        evaluate_steps(&self.steps, length, percentage_basis, |_| unreachable!("lengths have no color channels"))
    }

    pub(super) fn weight(&self) -> u64 {
        (std::mem::size_of::<Self>() + std::mem::size_of_val(self.steps.as_ref()) + 2 * std::mem::size_of::<usize>()) as u64
    }
}

/// Shared bounded arithmetic evaluator; only the operand providers vary.
pub(super) fn evaluate_steps(
    steps: &[Step], mut length: impl FnMut(LengthUnit, f64) -> f64,
    percentage_basis: f64, channel: impl Fn(u8) -> f64,
) -> f64 {
        let atom = |step: &Step, length: &mut dyn FnMut(LengthUnit, f64) -> f64| match step {
            Step::Number(value) => Some(*value),
            Step::Length(value, unit) => Some(length(*unit, *value)),
            Step::Percentage(fraction) => Some(fraction * percentage_basis),
            Step::Channel(index) => Some(channel(*index)),
            _ => None,
        };
        if steps.len() == 1 { return atom(&steps[0], &mut length).expect("literal program"); }
        let mut values = [0_f64; LengthPercentage::MAX_STEPS];
        for (index, step) in steps.iter().enumerate() {
            let value = atom(step, &mut length).unwrap_or_else(|| {
                let get = |index: u16| values[usize::from(index)];
                match *step {
                    Step::Add(a, b) => get(a) + get(b), Step::Subtract(a, b) => get(a) - get(b),
                    Step::Multiply(a, b) => get(a) * get(b), Step::Divide(a, b) => get(a) / get(b),
                    Step::Min(a, b) => math::compare(get(a), get(b), false),
                    Step::Max(a, b) => math::compare(get(a), get(b), true),
                    Step::Abs(a) => get(a).abs(),
                    Step::Sign(a) => math::sign(get(a)),
                    Step::Round(a, b, strategy) => math::rounded(get(a), get(b).abs(), strategy),
                    Step::Mod(a, b) => math::remainder(get(a), get(b), true),
                    Step::Rem(a, b) => math::remainder(get(a), get(b), false),
                    Step::Sqrt(a) => get(a).sqrt(), Step::Exp(a) => get(a).exp(),
                    Step::Power(a, b) => math::power(get(a), get(b)),
                    Step::Hypot(a, b) => math::hypot(get(a), get(b)),
                    Step::Log(a, b) => math::logarithm(get(a), get(b)),
                    Step::Trigonometric(operation, a, angle) => operation.unary(get(a), angle),
                    Step::Atan2(y, x) => get(y).atan2(get(x)).to_degrees(),
                    _ => unreachable!("literal handled above"),
                }
            });
            values[index] = value;
        }
        values[steps.len() - 1]
}
