//! Repair actual soft-row cuts with the existing contextual font-run shaper.
//! Rewrap with its resulting advances; never clip one ligature into two rows.

use std::ops::Range;
use gpui::{IndentAdjustment, LineFragment, LineWrapperHandle, Pixels, TextStyle, Window};
use super::{Paragraph, ShapedSpan, atom_boundary};
use super::super::super::native_layout::NativeLayoutFailure;
use super::super::tabs;

// Bound repeated native work even for fonts whose substitutions keep moving
// wrap points. The initial shaping and ordinary no-repair path are unchanged.
const MAX_RESHAPES: usize = 8;

impl Paragraph<'_> {
    pub fn wrap(&self, mut span: ShapedSpan, style: &TextStyle, hard_line: &Range<usize>,
        width: Pixels, collapse: bool, strict: bool, wrapper: &mut LineWrapperHandle, window: &mut Window,
    ) -> Result<(Vec<Range<usize>>, ShapedSpan), NativeLayoutFailure> {
        let mut breaks = Vec::new();
        for attempt in 0..=MAX_RESHAPES {
            let rows = rows(&span, width, collapse, wrapper)?;
            if !strict || (rows.len() < 2 && breaks.is_empty()) {
                return Ok((rows.into_iter().map(|row| hard_line.start + row.start..hard_line.start + row.end).collect(), span));
            }
            let ShapedSpan::Text { line, atoms, .. } = &span else { unreachable!("contextual text span") };
            // Rows already come from the grapheme-aware wrapper. Match their
            // cuts against native source starts without constructing another
            // bidi/cursor index or rescanning every glyph for every row.
            let cuts = &rows[..rows.len().saturating_sub(1)];
            let mut native = vec![false; cuts.len()];
            for glyph in line.runs.iter().flat_map(|run| &run.glyphs) {
                if let Ok(index) = cuts.binary_search_by_key(&glyph.index, |row| row.end) { native[index] = true; }
            }
            let mut wanted = Vec::new();
            for (row, native) in cuts.iter().zip(native) {
                if atom_boundary(atoms, row.end) { continue; }
                let absolute = hard_line.start + row.end;
                let requested = breaks.binary_search(&absolute).is_ok();
                // A requested native partition must actually split the cluster.
                // Repeating the same request cannot repair an unsupported font.
                if requested && !native { return Err(NativeLayoutFailure::UnsafeClusterBoundary); }
                if requested || !native { wanted.push(absolute); }
            }
            // Rewrapping can retire a previous cut. Remove it as well: leaving
            // it inside a final row would suppress an otherwise valid ligature.
            if wanted == breaks {
                let rows = rows.into_iter().map(|row| hard_line.start + row.start..hard_line.start + row.end).collect();
                return Ok((rows, span));
            }
            if attempt == MAX_RESHAPES { return Err(NativeLayoutFailure::ShapingWorkLimit); }
            breaks = wanted;
            // Full source, font attributes and isolates remain in one native
            // paragraph. The font-run cache key includes these exact cuts.
            span = self.shape_with_breaks(style, hard_line, &breaks, Some(line.text.to_shared_string()), window)
                .ok_or(NativeLayoutFailure::UnsafeClusterBoundary)?;
        }
        unreachable!("bounded shaping loop returns its outcome")
    }
}

fn rows(span: &ShapedSpan, width: Pixels, collapse: bool, wrapper: &mut LineWrapperHandle)
    -> Result<Vec<Range<usize>>, NativeLayoutFailure> {
    let ShapedSpan::Text { line, atoms, tab_stops, nowrap_ranges, space_policies, .. } = span
        else { unreachable!("contextual text span") };
    let mut rows = Vec::new();
    let mut start = 0;
    let mut append = |end: usize| {
        let end = end.min(line.len());
        if end > start { rows.push(start..end); start = end; }
    };
    if tab_stops.is_empty() {
        let fragments = [LineFragment::shaped_text(&line.text, line, Pixels::ZERO)
            .with_no_wrap_ranges(nowrap_ranges).with_space_policy_ranges(space_policies).with_inline_objects(atoms)];
        for boundary in wrapper.wrap_line_with_space_policy(&fragments, width, IndentAdjustment::NoIndent, collapse) {
            append(boundary.ix);
        }
    } else {
        let widths = tabs::Widths::new(line, atoms, tab_stops).ok_or(NativeLayoutFailure::UnsafeClusterBoundary)?;
        let fragments = [LineFragment::shaped_text_with_cumulative_advances(&line.text, line, widths.positions())
            .with_no_wrap_ranges(nowrap_ranges).with_space_policy_ranges(space_policies).with_inline_objects(atoms)];
        let mut measurement = widths.measurement();
        let mut measure = |range| measurement.measure(range);
        for boundary in wrapper.wrap_line_with_measurement(&fragments, width, collapse, &mut measure) {
            append(boundary.ix);
        }
    }
    append(line.len());
    Ok(rows)
}
