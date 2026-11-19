//! Paint-only cluster partitions retain native glyphs and insertion positions.

use super::{LineLayout, ShapedCluster, ShapedLine, ShapedRun};
use crate::Pixels;
use smallvec::SmallVec;
use std::{ops::Range, sync::Arc};

impl ShapedLine {
    /// A native insertion in this cluster. Unlike nearest-index lookup, this
    /// never invents an insertion inside an indivisible source grapheme.
    pub fn cluster_caret_x(&self, cluster: &ShapedCluster, index: usize) -> Option<Pixels> {
        let x = if index == cluster.source.start {
            if cluster.is_rtl { cluster.cell.end } else { cluster.cell.start }
        } else if index == cluster.source.end {
            if cluster.is_rtl { cluster.cell.start } else { cluster.cell.end }
        } else {
            if !cluster.source.contains(&index) { return None; }
            let at = self.interior_carets.partition_point(|caret| caret.index < index);
            let caret = self.interior_carets.get(at)?;
            if caret.index != index || caret.cluster_start != cluster.source.start { return None; }
            caret.x
        };
        (x.as_f32().is_finite() && x >= cluster.cell.start && x <= cluster.cell.end).then_some(x)
    }

    /// Retain a source part of one cluster for independent paint ownership.
    /// This is not a soft-line-break operation: glyphs keep their original
    /// shaping and are clipped only at the native insertions inside the cluster.
    pub fn slice_cluster_for_paint(&self, cluster: &ShapedCluster, range: Range<usize>) -> Option<Self> {
        if range.is_empty() || range.start < cluster.source.start || range.end > cluster.source.end
            || range.end > self.len() || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end) { return None; }
        let start = self.cluster_caret_x(cluster, range.start)?;
        let end = self.cluster_caret_x(cluster, range.end)?;
        if (cluster.is_rtl && end > start) || (!cluster.is_rtl && end < start) { return None; }
        let left = start.min(end);
        let right = start.max(end);
        let width = right - left;
        let first_run = self.runs.get(cluster.first_glyph.run_ix)?;
        if first_run.glyphs.get(cluster.first_glyph.glyph_ix)?.index != cluster.source.start { return None; }
        let mut runs = Vec::new();
        let mut first = true;
        'glyphs: for (offset, run) in self.runs[cluster.first_glyph.run_ix..].iter().enumerate() {
            let begin = if offset == 0 { cluster.first_glyph.glyph_ix } else { 0 };
            let mut glyphs = Vec::new();
            for glyph in &run.glyphs[begin..] {
                if glyph.index != cluster.source.start {
                    if !glyphs.is_empty() {
                        runs.push(ShapedRun { font_id: run.font_id, font_size: run.font_size, glyphs });
                    }
                    break 'glyphs;
                }
                let mut glyph = glyph.clone();
                glyph.position.x -= left;
                glyph.index = 0;
                glyph.advance = if first { width } else { Pixels::ZERO };
                first = false;
                glyphs.push(glyph);
            }
            if !glyphs.is_empty() {
                runs.push(ShapedRun { font_id: run.font_id, font_size: run.font_size, glyphs });
            }
        }
        if first { return None; }
        let first_caret = self.interior_carets.partition_point(|caret| caret.index <= range.start);
        let interior_carets = self.interior_carets[first_caret..].iter()
            .take_while(|caret| caret.index < range.end).map(|caret| super::ShapedCaret {
                index: caret.index - range.start, cluster_start: 0, x: caret.x - left,
            }).collect();
        let mut decoration_runs = SmallVec::new();
        let mut offset = 0;
        for run in &self.decoration_runs {
            let end = offset + run.len as usize;
            if end > range.start && offset < range.end {
                let mut part = run.clone();
                part.len = (end.min(range.end) - offset.max(range.start)) as u32;
                decoration_runs.push(part);
            }
            offset = end;
            if offset >= range.end { break; }
        }
        Some(Self {
            layout: Arc::new(LineLayout {
                font_size: self.font_size, width, ascent: self.ascent, descent: self.descent,
                runs, len: range.len(), interior_carets,
                isolation: self.isolation.as_ref().map(|context| context.subrange(range.clone())),
                ink_clip: Some(if cluster.is_rtl {
                    (range.end < cluster.source.end, range.start > cluster.source.start)
                } else {
                    (range.start > cluster.source.start, range.end < cluster.source.end)
                }),
            }),
            text: if range == (0..self.len()) { self.text.clone() }
                else { self.text.slice(range) },
            decoration_runs,
        })
    }
}
