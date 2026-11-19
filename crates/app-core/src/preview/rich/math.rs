//! Shared CSS arithmetic for constant declarations and retained lengths.
//! Keep intermediate infinities, NaN and signed zero until the property's
//! outer used-value boundary; native float helpers alone differ at those edges.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RoundingStrategy { Nearest, Up, Down, ToZero }
impl RoundingStrategy {
    pub fn named(name: &str) -> Option<Self> {
        [("nearest", Self::Nearest), ("up", Self::Up), ("down", Self::Down), ("to-zero", Self::ToZero)]
            .into_iter().find_map(|(expected, strategy)| name.eq_ignore_ascii_case(expected).then_some(strategy))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Trigonometric { Sin, Cos, Tan, Asin, Acos, Atan, Atan2 }
impl Trigonometric {
    pub fn named(name: &str) -> Option<Self> {
        [("sin", Self::Sin), ("cos", Self::Cos), ("tan", Self::Tan),
            ("asin", Self::Asin), ("acos", Self::Acos), ("atan", Self::Atan), ("atan2", Self::Atan2)]
            .into_iter().find_map(|(expected, operation)| name.eq_ignore_ascii_case(expected).then_some(operation))
    }
    pub fn unary(self, value: f64, angle: bool) -> f64 {
        match self {
            Self::Sin | Self::Cos | Self::Tan => {
                let degrees = if angle { value % 360. }
                    else if value.abs() <= std::f64::consts::TAU { value.to_degrees() }
                    else { f64::NAN };
                let radians = if angle { degrees.to_radians() } else { value };
                self.cardinal(degrees).unwrap_or_else(|| match self {
                    Self::Sin => radians.sin(), Self::Cos => radians.cos(), Self::Tan => radians.tan(),
                    _ => unreachable!("forward trigonometric operation"),
                })
            },
            Self::Asin => value.asin().to_degrees(), Self::Acos => value.acos().to_degrees(),
            Self::Atan => value.atan().to_degrees(),
            Self::Atan2 => unreachable!("atan2 takes two operands"),
        }
    }
    fn cardinal(self, degrees: f64) -> Option<f64> {
        let phase = degrees % 360.0;
        match self {
            Self::Sin if phase == 0.0 || phase.abs() == 180.0 => Some(0.0_f64.copysign(phase)),
            Self::Sin if phase == 90.0 || phase == -270.0 => Some(1.0),
            Self::Sin if phase == -90.0 || phase == 270.0 => Some(-1.0),
            Self::Cos if phase == 0.0 => Some(1.0),
            Self::Cos if phase.abs() == 180.0 => Some(-1.0),
            Self::Cos if phase.abs() == 90.0 || phase.abs() == 270.0 => Some(0.0),
            Self::Tan if phase == 0.0 || phase.abs() == 180.0 => Some(0.0_f64.copysign(phase)),
            Self::Tan if phase == 90.0 || phase == -270.0 => Some(f64::INFINITY),
            Self::Tan if phase == -90.0 || phase == 270.0 => Some(f64::NEG_INFINITY),
            _ => None,
        }
    }
}

pub(super) fn rounded(value: f64, step: f64, strategy: RoundingStrategy) -> f64 {
    if value.is_nan() || step.is_nan() || step == 0.0 || (value.is_infinite() && step.is_infinite()) {
        return f64::NAN;
    }
    if value.is_infinite() || value == 0.0 { return value; }
    if step.is_infinite() {
        return match strategy {
            RoundingStrategy::Up if value > 0.0 => f64::INFINITY,
            RoundingStrategy::Down if value < 0.0 => f64::NEG_INFINITY,
            _ => 0.0_f64.copysign(value),
        };
    }
    // A remainder retains information even when value / step overflows or
    // underflows. Do not reconstruct it by multiplying an enormous quotient.
    let remainder = value % step;
    if remainder == 0.0 { return value; }
    let toward_zero = (value - remainder).copysign(value);
    let (lower, upper, lower_distance, upper_distance) = if value < 0.0 {
        (toward_zero - step, toward_zero, step - remainder.abs(), remainder.abs())
    } else {
        (toward_zero, toward_zero + step, remainder, step - remainder)
    };
    match strategy {
        RoundingStrategy::Nearest => if lower_distance < upper_distance { lower } else { upper },
        RoundingStrategy::Up => upper,
        RoundingStrategy::Down => lower,
        RoundingStrategy::ToZero => toward_zero,
    }
}

pub(super) fn remainder(value: f64, step: f64, modulus: bool) -> f64 {
    if value.is_nan() || step.is_nan() || step == 0.0 || value.is_infinite() { return f64::NAN; }
    let opposite_signs = value.is_sign_negative() != step.is_sign_negative();
    if step.is_infinite() {
        // The sign test includes oppositely-signed zero for mod, while rem
        // accepts either infinite step and preserves the original value.
        return if modulus && opposite_signs { f64::NAN } else { value };
    }
    let remainder = value % step;
    if !modulus { remainder }
    else if remainder == 0.0 { 0.0_f64.copysign(step) }
    else if opposite_signs { remainder + step }
    else { remainder }
}

pub(super) fn logarithm(value: f64, base: f64) -> f64 {
    // CSS Values 4 defines the domain and special argument results before
    // ordinary change-of-base arithmetic, including a positive zero for 1.
    if value.is_nan() || base.is_nan() || base <= 0.0 || base == 1.0 || value < 0.0 { f64::NAN }
    else if value == 0.0 { f64::NEG_INFINITY }
    else if value == 1.0 { 0.0 }
    else if value.is_infinite() { f64::INFINITY }
    else { value.ln() / base.ln() }
}

pub(super) fn compare(left: f64, right: f64, maximum: bool) -> f64 {
    if left.is_nan() || right.is_nan() { return f64::NAN; }
    if left == 0.0 && right == 0.0 {
        let negative = if maximum { left.is_sign_negative() && right.is_sign_negative() }
            else { left.is_sign_negative() || right.is_sign_negative() };
        return if negative { -0.0 } else { 0.0 };
    }
    if maximum { left.max(right) } else { left.min(right) }
}

pub(super) fn sign(value: f64) -> f64 {
    if value == 0. || value.is_nan() { value } else { value.signum() }
}
pub(super) fn power(base: f64, exponent: f64) -> f64 {
    if base.is_nan() || exponent.is_nan() || (base.abs() == 1. && exponent.is_infinite()) { f64::NAN }
    else { base.powf(exponent) }
}
pub(super) fn hypot(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() { f64::NAN } else { left.hypot(right) }
}
