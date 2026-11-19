//! Row-local tab widths over the paragraph's existing native advances.
//! The temporary prefix index belongs to this wrap pass, not a second cache.

use std::ops::Range;

use gpui::{InlineObjectAdvance, Pixels, ShapedLine, TabStopRun, px};
use smallvec::SmallVec;

struct Tab {
    index: usize,
    before: f64,
    after: f64,
    stop_run: usize,
}

pub(super) struct Widths<'a> {
    prefix: Vec<(usize, f64)>,
    tabs: SmallVec<[Tab; 4]>,
    resolved: &'a [TabStopRun],
}

pub(super) struct Measurement<'a> {
    prefix: &'a [(usize, f64)],
    tabs: &'a [Tab],
    resolved: &'a [TabStopRun],
    measured: Range<usize>,
    width: f64,
    prefix_index: usize,
    next_tab: usize,
}

impl<'a> Widths<'a> {
    pub fn new(line: &ShapedLine, atoms: &[InlineObjectAdvance], resolved: &'a [TabStopRun]) -> Option<Self> {
        let capacity = line.runs.iter().map(|run| run.glyphs.len()).sum::<usize>() + atoms.len();
        let mut advances = Vec::with_capacity(capacity + 1);
        advances.extend(line.source_advances_with_objects(atoms, 0));
        let mut tabs = SmallVec::new();
        let mut total = 0_f64;
        let mut stop_run = 0;
        // Reuse the native/object advance allocation for cumulative positions.
        for (index, advance) in &mut advances {
            if *index >= line.len() { return None; }
            let end = total + *advance;
            if !end.is_finite() { return None; }
            if line.text.as_bytes().get(*index) == Some(&b'\t') {
                while resolved.get(stop_run).is_some_and(|run| run.range.end <= *index) { stop_run += 1; }
                resolved.get(stop_run).filter(|run| run.range.contains(index))?;
                tabs.push(Tab { index: *index, before: total, after: end, stop_run });
            }
            *advance = total;
            total = end;
        }
        if tabs.len() != line.text.bytes().filter(|byte| *byte == b'\t').count() { return None; }
        advances.push((line.len(), total));
        Some(Self { prefix: advances, tabs, resolved })
    }

    pub fn positions(&self) -> &[(usize, f64)] { &self.prefix }

    pub fn measurement(&self) -> Measurement<'_> {
        Measurement { prefix: &self.prefix, tabs: &self.tabs, resolved: self.resolved,
            measured: 0..0, width: 0., prefix_index: 0, next_tab: 0 }
    }
}

impl Measurement<'_> {
    fn raw_position(&mut self, at: usize) -> f64 {
        // A partial native cluster is charged at its first source character,
        // just like LineFragment's candidate iterator. Wraps still require
        // the shared grapheme and native cluster boundary checks.
        if self.prefix[self.prefix_index].0 > at {
            self.prefix_index = self.prefix.partition_point(|entry| entry.0 < at);
        } else {
            while self.prefix[self.prefix_index].0 < at { self.prefix_index += 1; }
        }
        self.prefix[self.prefix_index].1
    }

    pub fn measure(&mut self, range: Range<usize>) -> Pixels {
        assert!(range.start <= range.end && range.end <= self.prefix.last().expect("a native line has an end").0);
        let (from, mut width, mut next_tab) = if self.measured.start == range.start && self.measured.end <= range.end {
            (self.measured.end, self.width, self.next_tab)
        } else { (range.start, 0., self.tabs.partition_point(|tab| tab.index < range.start)) };
        let mut consumed = self.raw_position(from);
        while let Some(tab) = self.tabs.get(next_tab).filter(|tab| tab.index < range.end) {
            width += tab.before - consumed;
            width = self.resolved[tab.stop_run].stops.next_position(width);
            consumed = tab.after;
            next_tab += 1;
        }
        width += self.raw_position(range.end) - consumed;
        self.measured = range;
        self.width = width;
        self.next_tab = next_tab;
        px(width as f32)
    }
}
