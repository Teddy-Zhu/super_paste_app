use gpui::Rgba;

use super::Interpolate;

/// A paint color interpolated in premultiplied sRGB, including transparent endpoints.
/// This leaves the existing HSL interpolation policy available to callers.
#[derive(Clone, Copy, PartialEq)]
pub struct PremultipliedColor(pub Rgba);

impl Interpolate for PremultipliedColor {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        let from = self.0;
        let to = target.0;
        let alpha = from.a + (to.a - from.a) * progress;
        let channel = |from: f32, to: f32| {
            if alpha > 0.0 { (from + (to - from) * progress) / alpha } else { 0.0 }
        };
        Self(Rgba {
            r: channel(from.r * from.a, to.r * to.a),
            g: channel(from.g * from.a, to.g * to.a),
            b: channel(from.b * from.a, to.b * to.a),
            a: alpha,
        })
    }
}
