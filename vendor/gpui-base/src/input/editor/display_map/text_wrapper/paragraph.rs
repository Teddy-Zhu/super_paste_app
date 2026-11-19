use std::ops::Range;
use gpui::{Pixels, ShapedLine, TextDirection, px};
use crate::input::tab_stops::next_tab_shift;

/// Temporary prefix advances indexed by the wrapper's existing grapheme ends.
/// Tab entries reference that same table; no shaped rows or text is retained.
pub(super) struct ParagraphWidths {
    advances: Vec<Pixels>,
    /// Tab starts as indices into the existing grapheme endpoint table.
    tabs: Vec<usize>,
}

impl ParagraphWidths {
    /// Require every possible grapheme cut to be a genuine shaping-cluster
    /// boundary. Intracluster breaks need contextual reshaping, not a portion
    /// of a ligature's advance. Do not mix that unresolved case into this model.
    pub(super) fn new(line: &ShapedLine, ends: &[usize], direction: TextDirection) -> Option<Self> {
        if ends.last().copied().unwrap_or(0) != line.len() { return None; }
        // Source-order measurement needs neither row reordering nor a second
        // bidi analysis. The cursor builds a source index only when necessary.
        let mut cursor = line.cursor();
        let mut previous = px(0.);
        let mut tabs = Vec::new();
        for (index, &end) in ends.iter().enumerate() {
            if !cursor.is_cluster_boundary(end) { return None; }
            cursor.skip_until(end);
            let advance = cursor.source_advance();
            if !f32::from(advance).is_finite() || advance < previous { return None; }
            previous = advance;
            if line.text.as_bytes()[end - 1] == b'\t' {
                tabs.push(index);
            }
        }
        if !tabs.is_empty()
            && (!exact_tab_advances(line) || !ordered_tab_segments(line, ends, &tabs, direction))
        {
            return None;
        }
        // Unsupported ligature cuts often occur near the start of a large
        // paragraph. Allocate no paragraph-sized prefix array before preflight.
        cursor.rewind();
        let mut advances = Vec::with_capacity(ends.len() + 1);
        advances.push(px(0.));
        for &end in ends {
            cursor.skip_until(end);
            advances.push(cursor.source_advance());
        }
        Some(Self { advances, tabs })
    }

    pub(super) fn width(&self, range: &Range<usize>, ends: &[usize], tab_stop: Pixels) -> Option<Pixels> {
        let index = |offset| {
            if offset == 0 { Some(0) } else { ends.binary_search(&offset).ok().map(|index| index + 1) }
        };
        // Leading-space indentation may end inside a grapheme containing a
        // combining mark; such a probe has no entry in this cluster model.
        let start = index(range.start)?;
        let end = index(range.end)?;
        let origin = self.advances[start];
        let width = self.advances[end] - origin;
        if tab_stop <= px(0.) { return Some(width); }
        let stop = f64::from(f32::from(tab_stop));
        let mut shift = 0_f64;
        let first_tab = self.tabs.partition_point(|tab| *tab < start);
        for &tab in self.tabs[first_tab..].iter().take_while(|tab| **tab < end) {
            shift = next_tab_shift(
                f64::from(f32::from(self.advances[tab] - origin)),
                f64::from(f32::from(self.advances[tab + 1] - origin)),
                shift,
                stop,
            );
        }
        Some(width + px(shift as f32))
    }
}

/// Tab rounding amplifies even one ULP into a whole additional tab stop.
/// Require nonnegative advances on an exactly representable common binary
/// grid, so source traversal, visual traversal and every soft-row subtraction
/// agree without a tolerance or a second per-grapheme precision table.
fn exact_tab_advances(line: &ShapedLine) -> bool {
    let mut unit: Option<i32> = None;
    let mut total = 0_f64;
    for glyph in line.runs.iter().flat_map(|run| &run.glyphs) {
        let value = f32::from(glyph.advance);
        if !value.is_finite() || value < 0. { return false; }
        if value == 0. { continue; }
        let bits = value.to_bits();
        let exponent = ((bits >> 23) & 0xff) as i32;
        let mantissa = (bits & 0x7f_ffff) | if exponent == 0 { 0 } else { 0x80_0000 };
        let low_bit = (if exponent == 0 { -149 } else { exponent - 150 })
            + mantissa.trailing_zeros() as i32;
        unit = Some(unit.map_or(low_bit, |unit| unit.min(low_bit)));
        total += f64::from(value);
    }
    total as f32 == f32::from(line.width)
        && unit.is_none_or(|unit| total * 2_f64.powi(-unit) <= f64::from(1_u32 << 24))
}

/// Tabs must separate the same source segments from the paragraph's start
/// edge. Runs within a segment may reorder freely. Check actual native cells
/// rather than assuming that every isolate/embedding has this property.
fn ordered_tab_segments(
    line: &ShapedLine,
    ends: &[usize],
    tabs: &[usize],
    direction: TextDirection,
) -> bool {
    let rtl = match direction {
        TextDirection::Ltr => false,
        TextDirection::Rtl => true,
        TextDirection::Auto => return false,
    };
    let source = |tab: usize| if tab == 0 { 0 } else { ends[tab - 1] };
    let mut previous = None;
    let mut seen = 0;
    for (index, _) in line.visual_cluster_advances() {
        let segment = tabs.partition_point(|tab| source(*tab) < index);
        let is_tab = tabs.get(segment).is_some_and(|tab| source(*tab) == index);
        // Even positions are text segments; odd positions are their tabs.
        let phase = segment * 2 + usize::from(is_tab);
        if previous.is_some_and(|previous| if rtl { phase > previous } else { phase < previous }) {
            return false;
        }
        previous = Some(phase);
        seen += usize::from(is_tab);
    }
    // A missing control glyph cannot silently become a zero-width tab here.
    seen == tabs.len()
}
