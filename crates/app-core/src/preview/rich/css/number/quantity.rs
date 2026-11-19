//! Temporary calculation types shared by constant and retained expressions.
//! Canonical scalar units are deg, s, Hz and dppx; lengths may need layout.
//! https://drafts.csswg.org/css-values-4/#calc-type-checking
use crate::preview::rich::LengthUnit;

#[derive(Clone, Copy)]
pub(in crate::preview::rich::css) struct Quantity {
    pub value: f64,
    powers: [i32; 6],
}

impl Quantity {
    pub fn number(value: f64) -> Self { Self { value, powers: [0; 6] } }
    pub fn percentage(value: f64) -> Self { Self { value, powers: [0, 0, 0, 0, 0, 1] } }
    pub fn is_percentage(self) -> bool { self.powers == [0, 0, 0, 0, 0, 1] }
    pub fn angle(value: f64) -> Self { Self { value, powers: [0, 1, 0, 0, 0, 0] } }
    pub fn length(value: f64) -> Self { Self { value, powers: [1, 0, 0, 0, 0, 0] } }
    pub fn is_number(self) -> bool { self.powers == [0; 6] }
    pub fn is_angle(self) -> bool { self.powers == [0, 1, 0, 0, 0, 0] }
    pub fn is_length(self) -> bool { self.powers == [1, 0, 0, 0, 0, 0] }
    pub fn same_type(self, other: Self) -> bool { self.powers == other.powers }
    pub fn with_value(self, value: f64) -> Self { Self { value, ..self } }

    pub fn product(self, other: Self, divide: bool) -> Option<Self> {
        let mut powers = self.powers;
        for (power, right) in powers.iter_mut().zip(other.powers) {
            *power = if divide { power.checked_sub(right)? } else { power.checked_add(right)? };
        }
        Some(Self { powers, value: if divide { self.value / other.value } else { self.value * other.value } })
    }

    pub fn dimension(source: &str, unit: &str) -> Option<Self> {
        let length = [("px", LengthUnit::Px), ("in", LengthUnit::In), ("cm", LengthUnit::Cm),
            ("mm", LengthUnit::Mm), ("q", LengthUnit::Q), ("pt", LengthUnit::Pt), ("pc", LengthUnit::Pc)]
            .into_iter().find_map(|(name, kind)| unit.eq_ignore_ascii_case(name).then_some(kind));
        let (kind, scale) = if let Some(length) = length { (0, length.absolute_pixels(1.0)?) } else { [
            ("rad", 1, 180.0 / std::f64::consts::PI), ("deg", 1, 1.0),
            ("grad", 1, 0.9), ("turn", 1, 360.0),
            ("s", 2, 1.0), ("ms", 2, 0.001),
            ("hz", 3, 1.0), ("khz", 3, 1000.0),
            ("dppx", 4, 1.0), ("x", 4, 1.0), ("dpi", 4, 1.0 / 96.0), ("dpcm", 4, 2.54 / 96.0),
        ].into_iter().find_map(|(name, kind, scale)| unit.eq_ignore_ascii_case(name).then_some((kind, scale)))? };
        let value = super::literal_prefix(source)? * scale;
        let mut powers = [0; 6];
        powers[kind] = 1;
        Some(Self { value, powers })
    }
}
