use crate::{Pixels, Point, Result, px};
use lyon::geom::{CubicBezierSegment, QuadraticBezierSegment, point};
use smallvec::SmallVec;
use std::ops::Range;

/// Transient intersections of a glyph outline with decoration bands.
///
/// Coordinates are baseline-relative logical pixels, with y increasing downwards.
/// Each band accumulates the projection of the outline inside the band, rather
/// than the whole glyph's bounding box. No outline or raster image is retained.
#[derive(Clone, Debug)]
pub struct GlyphInk {
    bands: SmallVec<[Band; 4]>,
    current: lyon::geom::Point<f64>,
    start: lyon::geom::Point<f64>,
    open: bool,
    invalid: bool,
}

#[derive(Clone, Debug)]
struct Band {
    top: f64,
    bottom: f64,
    left: f64,
    right: f64,
}

impl GlyphInk {
    /// Starts an intersection query without allocating for up to four bands.
    pub fn new(bands: &[Range<Pixels>]) -> Result<Self> {
        anyhow::ensure!(!bands.is_empty(), "expected at least one ink band");
        let bands = bands.iter().map(|band| {
            let top = f64::from(band.start.as_f32());
            let bottom = f64::from(band.end.as_f32());
            anyhow::ensure!(top.is_finite() && bottom.is_finite() && top <= bottom,
                "invalid glyph ink band");
            Ok(Band { top, bottom, left: f64::INFINITY, right: f64::NEG_INFINITY })
        }).collect::<Result<SmallVec<[_; 4]>>>()?;
        Ok(Self { bands, current: point(0.0, 0.0), start: point(0.0, 0.0), open: false, invalid: false })
    }

    /// Begins a contour. Font backends must convert their coordinates to pixels.
    pub fn move_to(&mut self, to: Point<f32>) {
        self.close();
        let to = self.convert(to);
        self.current = to;
        self.start = to;
        self.open = true;
    }

    /// Intersects a straight outline segment with the requested bands.
    pub fn line_to(&mut self, to: Point<f32>) {
        let to = self.convert(to);
        self.line_to_point(to);
    }

    fn line_to_point(&mut self, to: lyon::geom::Point<f64>) {
        if self.invalid { return; }
        if !self.open { self.invalid = true; return; }
        let from = self.current;
        for band in &mut self.bands {
            band.include(from);
            band.include(to);
            let dy = to.y - from.y;
            if dy != 0.0 {
                for y in [band.top, band.bottom] {
                    let t = (y - from.y) / dy;
                    if (0.0..=1.0).contains(&t) { band.extend(from.x + (to.x - from.x) * t); }
                }
            }
        }
        self.current = to;
    }

    /// Intersects a quadratic curve, including its interior x extremum.
    pub fn quad_to(&mut self, control: Point<f32>, to: Point<f32>) {
        let control = self.convert(control);
        let to = self.convert(to);
        let curve = QuadraticBezierSegment { from: self.current, ctrl: control, to };
        self.cubic(curve.to_cubic());
    }

    /// Intersects a cubic curve, including roots at both band edges and x extrema.
    pub fn curve_to(&mut self, first: Point<f32>, second: Point<f32>, to: Point<f32>) {
        let first = self.convert(first);
        let second = self.convert(second);
        let to = self.convert(to);
        self.cubic(CubicBezierSegment { from: self.current, ctrl1: first, ctrl2: second, to });
    }

    fn cubic(&mut self, curve: CubicBezierSegment<f64>) {
        if self.invalid { return; }
        if !self.open { self.invalid = true; return; }
        for band in &mut self.bands {
            // A control-point hull rejects the usual non-descending curves
            // before solving any polynomial. The curve itself supplies bounds.
            let points = [curve.from, curve.ctrl1, curve.ctrl2, curve.to];
            if points.iter().all(|p| p.y < band.top) || points.iter().all(|p| p.y > band.bottom) {
                continue;
            }
            band.include(curve.from);
            band.include(curve.to);
            for y in [band.top, band.bottom] {
                for t in curve.solve_t_for_y(y) { band.extend(curve.sample(t).x); }
            }
            curve.for_each_local_x_extremum_t(&mut |t| band.include(curve.sample(t)));
        }
        self.current = curve.to;
    }

    /// Closes the current contour, including its final fill edge.
    pub fn close(&mut self) {
        if self.open {
            self.line_to_point(self.start);
            self.open = false;
        }
    }

    /// Returns one optional x interval per input band.
    pub fn intersections(&mut self) -> Result<SmallVec<[Option<Range<Pixels>>; 4]>> {
        self.close();
        anyhow::ensure!(!self.invalid, "invalid glyph outline");
        Ok(self.bands.iter().map(|band| {
            (band.left < band.right).then(|| px(band.left as f32)..px(band.right as f32))
        }).collect())
    }

    fn convert(&mut self, value: Point<f32>) -> lyon::geom::Point<f64> {
        if !value.x.is_finite() || !value.y.is_finite() { self.invalid = true; }
        point(f64::from(value.x), f64::from(value.y))
    }
}

impl Band {
    fn include(&mut self, point: lyon::geom::Point<f64>) {
        if point.y >= self.top && point.y <= self.bottom { self.extend(point.x); }
    }

    fn extend(&mut self, x: f64) {
        self.left = self.left.min(x);
        self.right = self.right.max(x);
    }
}
