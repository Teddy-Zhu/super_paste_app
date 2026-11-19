//! Logical soft rows for a paragraph whose glyphs are already in visual order.
//! Preserve paragraph shaping across soft wraps (CSS Text 3, line-breaking
//! details) and apply UAX #9 L1/L2 to each source row. Splitting an existing
//! ligature still needs a contextual shaping path; never reshape each row as
//! an independent word and accidentally discard its original joining forms.

use super::{LineLayout, LineWrapper, ShapedRun, WrapBoundary};
use crate::{OverflowWrap, Pixels, TextDirection};
use smallvec::SmallVec;
use std::{ops::Range, sync::Arc};
use unicode_bidi::{BidiInfo, Level, ParagraphBidiInfo};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation as _};

struct Cluster {
    source: usize,
    grapheme: usize,
    first: WrapBoundary,
    pen: Pixels,
    width: Pixels,
}

#[derive(Clone)]
struct Group {
    source: usize,
    clusters: Range<usize>,
    width: Pixels,
}

struct CursorRun {
    source: Range<usize>,
    rtl: bool,
}

/// A borrowed view of a source row in visual order. Ranges use the original
/// paragraph's UTF-8 offsets, including controls with no native glyph.
/// No source text, glyphs or ordering index is copied into this view.
pub struct VisualSourceRuns<'a> {
    runs: &'a [CursorRun],
    source_order: &'a [usize],
    uniform: Option<Range<usize>>,
}

impl VisualSourceRuns<'_> {
    pub(crate) fn ltr(source: Range<usize>) -> Self {
        Self { runs: &[], source_order: &[], uniform: Some(source) }
    }

    /// Source ranges and their direction, from the left edge of the row.
    pub fn iter(&self) -> impl Iterator<Item = (Range<usize>, bool)> + '_ {
        self.uniform.iter().map(|source| (source.clone(), false))
            .chain(self.runs.iter().map(|run| (run.source.clone(), run.rtl)))
    }

    /// The first visual-run boundary strictly inside a source range.
    /// This lets callers check native cluster ownership before slicing it.
    pub fn first_boundary(&self, source: Range<usize>) -> Option<usize> {
        let next = self.source_order.partition_point(|index| self.runs[*index].source.start <= source.start);
        self.source_order.get(next).map(|index| self.runs[*index].source.start)
            .filter(|boundary| *boundary < source.end)
    }
}

/// Paragraph analysis shared by successive source slices of an already shaped
/// line. Only the current row's L1/L2 scratch is retained in addition to the
/// paragraph levels; no glyphs, source strings or painted layouts are copied.
pub(crate) struct CursorBidi<'a> {
    text: &'a str,
    isolation: Option<&'a crate::TextIsolationContext>,
    paragraph: BidiInfo<'a>,
    row: ParagraphBidiInfo<'a>,
    graphemes: Vec<usize>,
    runs: Vec<CursorRun>,
    source_order: Vec<usize>,
    prepared_source: Option<Range<usize>>,
}

impl<'a> CursorBidi<'a> {
    pub(crate) fn new(text: &'a str, direction: TextDirection, layout: &'a LineLayout) -> Option<Self> {
        needs_bidi(layout, text, direction).then(|| {
            Self::analyze(text, direction, Vec::new(), layout.isolation.as_ref())
        })
    }

    fn analyze(text: &'a str, direction: TextDirection, graphemes: Vec<usize>, isolation: Option<&'a crate::TextIsolationContext>) -> Self {
        let base = match direction {
            TextDirection::Auto => None,
            TextDirection::Ltr => Some(Level::ltr()),
            TextDirection::Rtl => Some(Level::rtl()),
        };
        Self {
            text,
            isolation,
            paragraph: BidiInfo::new(isolation.map_or(text, |context| context.source.text.as_ref()), base),
            row: ParagraphBidiInfo {
                text: "", original_classes: Vec::new(), levels: Vec::new(),
                paragraph_level: Level::ltr(), is_pure_ltr: false,
            },
            graphemes,
            runs: Vec::new(),
            source_order: Vec::new(),
            prepared_source: None,
        }
    }

    pub(crate) fn grapheme(&self, source: usize) -> usize {
        let index = self.graphemes.partition_point(|start| *start <= source).saturating_sub(1);
        self.graphemes[index]
    }

    pub(crate) fn prepare_graphemes(&mut self) {
        if self.graphemes.is_empty() {
            self.graphemes.extend(self.text.grapheme_indices(true).map(|(index, _)| index));
        }
    }

    pub(crate) fn is_grapheme_boundary(&self, source: usize) -> bool {
        let text = self.text;
        if !text.is_char_boundary(source) { return false; }
        if source == 0 || source == text.len() { return true; }
        if !self.graphemes.is_empty() {
            return self.graphemes.binary_search(&source).is_ok();
        }
        // A boundary query before the first cut must not build a full source
        // index. The complete paragraph supplies any preceding Unicode context.
        GraphemeCursor::new(source, text.len(), true).is_boundary(text, 0)
            .expect("complete paragraph must supply grapheme context")
    }

    fn prepare(&mut self, source: Range<usize>) {
        self.prepared_source = None;
        self.runs.clear();
        let source = self.isolation.map_or(source.clone(), |context| context.presentation_range(source));
        let first = self.paragraph.paragraphs.partition_point(|paragraph| paragraph.range.end <= source.start);
        for paragraph in self.paragraph.paragraphs[first..].iter()
            .take_while(|paragraph| paragraph.range.start < source.end) {
            let start = source.start.max(paragraph.range.start);
            let end = source.end.min(paragraph.range.end);
            self.row.text = &self.paragraph.text[start..end];
            self.row.paragraph_level = paragraph.level;
            self.row.original_classes.clear();
            self.row.original_classes.extend_from_slice(&self.paragraph.original_classes[start..end]);
            self.row.levels.clear();
            self.row.levels.extend_from_slice(&self.paragraph.levels[start..end]);
            let (levels, runs) = self.row.visual_runs(0..end - start);
            for run in runs {
                let source = start + run.start..start + run.end;
                let source = self.isolation.map_or(source.clone(), |context| context.original_range(source));
                if source.is_empty() { continue; }
                self.runs.push(CursorRun {
                    rtl: levels[run.start].is_rtl(),
                    source,
                });
            }
        }
    }

    pub(crate) fn prepare_cursor(&mut self, source: Range<usize>) {
        if self.prepared_source.as_ref() == Some(&source) { return; }
        self.prepare(source.clone());
        // Ordinary wrapping only paints visual runs and needs no source index.
        self.source_order.clear();
        self.source_order.extend(0..self.runs.len());
        self.source_order.sort_unstable_by_key(|index| self.runs[*index].source.start);
        self.prepared_source = Some(source);
    }

    pub(crate) fn visual_source_runs(&self) -> VisualSourceRuns<'_> {
        VisualSourceRuns { runs: &self.runs, source_order: &self.source_order, uniform: None }
    }

    pub(crate) fn order(&self, source: usize) -> (usize, bool) {
        let index = self.source_order.partition_point(|index| self.runs[*index].source.start <= source).saturating_sub(1);
        let visual = self.source_order[index];
        let run = &self.runs[visual];
        debug_assert!(run.source.contains(&source));
        (visual, run.rtl)
    }
}

fn needs_bidi(layout: &LineLayout, text: &str, direction: TextDirection) -> bool {
    layout.isolation.is_some() || direction == TextDirection::Rtl
        || layout.runs.iter().flat_map(|run| &run.glyphs).any(|glyph| glyph.is_rtl)
        || (!text.is_ascii() && text.contains([
            '\u{061c}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}',
            '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ]))
}

pub(super) fn wrap(
    original: &Arc<LineLayout>,
    text: &str,
    width: Pixels,
    max_lines: Option<usize>,
    direction: TextDirection,
    overflow_wrap: Option<OverflowWrap>,
) -> Option<(Arc<LineLayout>, SmallVec<[WrapBoundary; 1]>)> {
    if original.width <= width {
        return None;
    }
    // An RTL paragraph can contain only LTR glyphs. L1 still moves each row's
    // trailing whitespace; native shapers may omit its invisible first strong
    // ALM/RLM, so glyph parity alone cannot detect that automatic base level.
    if !needs_bidi(original, text, direction) {
        return None;
    }

    let graphemes: Vec<_> = text.grapheme_indices(true).map(|(index, _)| index).collect();
    let mut clusters = Vec::<Cluster>::new();
    let mut pen = Pixels::ZERO;
    for (run_ix, run) in original.runs.iter().enumerate() {
        for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
            if let Some(cluster) = clusters.last_mut().filter(|cluster| cluster.source == glyph.index) {
                cluster.width += glyph.advance;
            } else {
                let grapheme = graphemes
                    .partition_point(|start| *start <= glyph.index)
                    .saturating_sub(1);
                clusters.push(Cluster {
                    source: glyph.index, grapheme: graphemes[grapheme],
                    first: WrapBoundary { run_ix, glyph_ix }, pen, width: glyph.advance,
                });
            }
            pen += glyph.advance;
        }
    }
    if clusters.len() < 2 {
        return None;
    }

    // Sort source groups, retaining native glyph order *inside* each grapheme.
    // Indic reordering and fallback marks can carry different source indices
    // within one grapheme. A ligature remains atomic even across graphemes.
    clusters.sort_unstable_by_key(|cluster| (cluster.grapheme, cluster.first));
    let mut groups = Vec::<Group>::new();
    for (index, cluster) in clusters.iter().enumerate() {
        if let Some(group) = groups.last_mut().filter(|group| group.source == cluster.grapheme) {
            group.clusters.end = index + 1;
            group.width += cluster.width;
        } else {
            groups.push(Group { source: cluster.grapheme, clusters: index..index + 1, width: cluster.width });
        }
    }
    // A grapheme edge inside a shaping cluster is not a legal cut. Merge
    // connected groups in place, then restore their original glyph order.
    let mut retained = 0;
    for index in 0..groups.len() {
        let group = groups[index].clone();
        let joint = clusters[group.clusters.clone()].iter().any(|cluster| cluster.source == group.source);
        if !joint && retained > 0 {
            let previous = &mut groups[retained - 1];
            previous.clusters.end = group.clusters.end;
            previous.width += group.width;
        } else {
            groups[retained] = group;
            retained += 1;
        }
    }
    groups.truncate(retained);
    for group in &groups {
        clusters[group.clusters.clone()].sort_unstable_by_key(|cluster| cluster.first);
    }
    let rows = source_rows(&groups, text, width, max_lines, overflow_wrap);
    if rows.len() < 2 {
        return Some((original.clone(), SmallVec::new()));
    }

    let mut bidi = CursorBidi::analyze(text, direction, graphemes, original.isolation.as_ref());
    let mut result = LineLayout {
        font_size: original.font_size, ascent: original.ascent, descent: original.descent,
        len: original.len, isolation: original.isolation.clone(), ..Default::default()
    };
    let mut boundaries = SmallVec::new();
    for row in rows {
        let source_start = if row.start == 0 { 0 } else { groups[row.start].source };
        let source_end = groups.get(row.end).map_or(text.len(), |group| group.source);
        let had_glyphs = !result.runs.is_empty();
        let mut row_start = None;
        bidi.prepare(source_start..source_end);
        for run in &bidi.runs {
            let first = groups.partition_point(|group| group.source < run.source.start).max(row.start);
            let last = groups.partition_point(|group| group.source < run.source.end).min(row.end);
            let selected = &groups[first..last];
            if run.rtl {
                for group in selected.iter().rev() { append_group(&mut result, original, &clusters, group, run.rtl, &mut row_start); }
            } else {
                for group in selected { append_group(&mut result, original, &clusters, group, run.rtl, &mut row_start); }
            }
        }
        if had_glyphs {
            boundaries.extend(row_start);
        }
    }
    result.interior_carets.sort_unstable_by_key(|caret| caret.index);
    Some((Arc::new(result), boundaries))
}

fn source_rows(
    groups: &[Group],
    text: &str,
    width: Pixels,
    max_lines: Option<usize>,
    overflow_wrap: Option<OverflowWrap>,
) -> Vec<Range<usize>> {
    let mut breaks = LineWrapper::break_indices(text).peekable();
    let legal: Vec<_> = groups.iter().map(|group| {
        while breaks.peek().is_some_and(|index| *index < group.source) { breaks.next(); }
        breaks.peek().is_some_and(|index| *index == group.source)
    }).collect();
    let mut rows = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut used = Pixels::ZERO;
    let mut candidate = None;
    while index < groups.len() {
        if index > start && legal[index] { candidate = Some(index); }
        if used + groups[index].width > width && index > start
            && (overflow_wrap != Some(OverflowWrap::Normal) || candidate.is_some()) {
            if max_lines.is_some_and(|max| rows.len() >= max.saturating_sub(1)) { break; }
            let end = candidate.take().unwrap_or(index);
            rows.push(start..end);
            start = end;
            index = end;
            used = Pixels::ZERO;
        } else {
            used += groups[index].width;
            index += 1;
        }
    }
    rows.push(start..groups.len());
    rows
}

fn append_group(
    target: &mut LineLayout,
    original: &LineLayout,
    clusters: &[Cluster],
    group: &Group,
    rtl: bool,
    row_start: &mut Option<WrapBoundary>,
) {
    // Move the whole grapheme/shaping group by one delta. Fallback marks and
    // overhanging glyphs may interleave with another group's native glyphs;
    // shifting each constituent cluster by its new pen would distort them.
    let delta = target.width - clusters[group.clusters.start].pen;
    for cluster in &clusters[group.clusters.clone()] {
        let caret_start = original.interior_carets.partition_point(|caret| caret.index <= cluster.source);
        for caret in original.interior_carets[caret_start..].iter().take_while(|caret| caret.cluster_start == cluster.source) {
            if caret.x >= cluster.pen && caret.x <= cluster.pen + cluster.width {
                let mut caret = *caret;
                caret.x += delta;
                target.interior_carets.push(caret);
            }
        }
        'cluster: for (offset, run) in original.runs[cluster.first.run_ix..].iter().enumerate() {
            let first = if offset == 0 { cluster.first.glyph_ix } else { 0 };
            for glyph in &run.glyphs[first..] {
                if glyph.index != cluster.source { break 'cluster; }
                if target.runs.last().is_none_or(|last| last.font_id != run.font_id || last.font_size != run.font_size) {
                    target.runs.push(ShapedRun { font_size: run.font_size, font_id: run.font_id, glyphs: Vec::new() });
                }
                let mut glyph = glyph.clone();
                glyph.position.x += delta;
                glyph.is_rtl = rtl;
                target.width += glyph.advance;
                let run_ix = target.runs.len() - 1;
                let target_run = &mut target.runs[run_ix];
                if row_start.is_none() {
                    *row_start = Some(WrapBoundary { run_ix, glyph_ix: target_run.glyphs.len() });
                }
                target_run.glyphs.push(glyph);
            }
        }
    }
}
