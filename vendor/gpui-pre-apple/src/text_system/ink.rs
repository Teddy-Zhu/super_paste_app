// Core Text path traversal adapted from font-kit's Core Text loader.
// Copyright © 2018 The Pathfinder Project Developers.
// SPDX-License-Identifier: Apache-2.0

use core_graphics::{geometry::{CGPoint, CG_AFFINE_TRANSFORM_IDENTITY}, path::CGPathElementType};
use core_text::font::CTFont;
use gpui::{GlyphId, GlyphInk, Pixels, Result, point};
use std::cell::RefCell;

/// Project both fills of synthetic bold into decoration bands. Extract the
/// native path once and reuse it; no contour or glyph cache is retained here.
pub(super) fn intersect_bold(font: &CTFont, glyph: GlyphId, size: Pixels, offset: f64, ink: &mut GlyphInk) -> Result<()> {
    anyhow::ensure!(glyph.0 < font.glyph_count() as u32, "invalid glyph id");
    let glyph = u16::try_from(glyph.0)?;
    let scale = f64::from(size.as_f32()) / font.pt_size();
    anyhow::ensure!(scale.is_finite() && scale > 0. && offset.is_finite() && offset >= 0., "invalid glyph outline scale");
    // Core Text also returns no path for valid empty glyphs such as spaces,
    // matching the existing font-kit outline behavior.
    let Ok(path) = font.create_path_for_glyph(glyph, &CG_AFFINE_TRANSFORM_IDENTITY) else { return Ok(()); };
    let ink = RefCell::new(ink);
    for shift in [0., offset] {
        let project = |p: CGPoint| point((p.x * scale + shift) as f32, (-p.y * scale) as f32);
        path.apply(&|element| {
            let points = element.points();
            let mut ink = ink.borrow_mut();
            match element.element_type {
                CGPathElementType::MoveToPoint => ink.move_to(project(points[0])),
                CGPathElementType::AddLineToPoint => ink.line_to(project(points[0])),
                CGPathElementType::AddQuadCurveToPoint => ink.quad_to(project(points[0]), project(points[1])),
                CGPathElementType::AddCurveToPoint => ink.curve_to(project(points[0]), project(points[1]), project(points[2])),
                CGPathElementType::CloseSubpath => ink.close(),
            }
        });
        ink.borrow_mut().close();
    }
    Ok(())
}
