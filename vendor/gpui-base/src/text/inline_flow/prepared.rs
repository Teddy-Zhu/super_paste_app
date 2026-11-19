//! Rows borrow hard-line shaping instead of shaping each soft line again.
//! Complete hard paragraphs also retain paragraph bidi levels across rows.
//! Structured size/box contexts share one paragraph and split into its visual
//! fragments, including source atoms whose native font glyph is never painted.
//! Paint-only ligature cuts retain native glyphs with sparse edge masks;
//! soft-row and typography cuts still require a compatible native partition.
//! Geometry belongs to the existing per-flow, font-generation key.

use std::ops::Range;

use gpui::{FontId, Pixels, ShapedLine, ShapedLineCursor, TextDirection, TextRun, Window};
use smallvec::SmallVec;

use super::{ShapedSpan, contextual};

pub(super) fn font_ids(ids: impl Iterator<Item = FontId>) -> SmallVec<[FontId; 4]> {
    let mut ids: SmallVec<[FontId; 4]> = ids.collect();
    ids.sort_unstable_by_key(|id| id.0);
    ids.dedup();
    ids
}

struct Span<'a> {
    range: &'a Range<usize>,
    line: &'a ShapedLine,
    cursor: Option<ShapedLineCursor<'a>>,
    complete_paragraph: bool,
    consumed: usize,
    requested_fonts: &'a [FontId],
    actual_fonts: SmallVec<[FontId; 4]>,
    contexts: Option<&'a [(Range<usize>, f32)]>,
    needs_source_runs: bool,
    atoms: &'a [gpui::InlineObjectAdvance],
    tab_stops: &'a [gpui::TabStopRun],
}

pub(super) struct Rows<'a> {
    spans: Vec<Span<'a>>,
    next: usize,
    direction: TextDirection,
}

pub(super) struct Row {
    pub line: ShapedLine,
    pub reusable: bool,
}

/// One final flow fragment. Reordered native pieces retain a physical rank;
/// source order still owns boxes, selection and accessible reading order.
pub(super) struct Fragment {
    pub range: Range<usize>,
    pub scale: f32,
    pub line: Option<ShapedLine>,
    pub visual_rank: Option<usize>,
    /// Resolved source direction for an atomic object, not the paragraph base.
    pub atomic_rtl: bool,
}

impl<'a> Rows<'a> {
    pub fn new(spans: &'a [ShapedSpan], direction: TextDirection) -> Self {
        let mut rows = Self { spans: Vec::new(), next: 0, direction };
        for (index, span) in spans.iter().enumerate() {
            let ShapedSpan::Text { range, line, requested_fonts, contexts, needs_source_runs, atoms, tab_stops, .. } = span else {
                continue;
            };
            // Adjacent text/font/atom spans belong to the same hard paragraph;
            // hard separators leave a source-byte gap. A segment's own text
            // cannot recover the paragraph levels on either side of that gap.
            let complete_paragraph = !index.checked_sub(1).and_then(|index| spans.get(index))
                .is_some_and(|span| source_range(span).end == range.start)
                && !spans.get(index + 1).is_some_and(|span| source_range(span).start == range.end);
            if !complete_paragraph && (direction == TextDirection::Rtl
                || line.runs.iter().flat_map(|run| &run.glyphs).any(|glyph| glyph.is_rtl)
                || !line.runs.iter().flat_map(|run| &run.glyphs)
                    .map(|glyph| glyph.index).is_sorted()
                || (direction == TextDirection::Auto
                    && !line.text.is_ascii()
                    && line.text.contains(['\u{061c}', '\u{200f}'])))
            {
                continue;
            }
            rows.spans.push(Span {
                range,
                line,
                cursor: None,
                complete_paragraph,
                consumed: 0,
                requested_fonts,
                actual_fonts: font_ids(line.runs.iter()
                    .filter(|run| !run.glyphs.is_empty()).map(|run| run.font_id)),
                contexts: contexts.as_deref(),
                needs_source_runs: *needs_source_runs,
                atoms,
                tab_stops,
            });
        }
        rows
    }

    pub fn take_contextual(&mut self, range: Range<usize>, strict: bool, window: &Window) -> Option<Vec<Fragment>> {
        while self.spans.get(self.next).is_some_and(|span| span.range.end <= range.start) {
            self.next += 1;
        }
        let span = self.spans.get_mut(self.next)?;
        let contexts = span.contexts?;
        if range.start < span.range.start || range.end > span.range.end { return None; }
        let start = range.start - span.range.start;
        let end = range.end - span.range.start;
        assert!(start >= span.consumed, "inline rows must follow source order");
        let line = if start == 0 && end == span.line.len() {
            span.line.clone()
        } else {
            let cursor = span.cursor.get_or_insert_with(|| span.line.cursor_with_direction(self.direction));
            if start > span.consumed { cursor.skip_until(start); }
            // Contextual wrapping uses this paragraph's shaped clusters; a
            // soft boundary cannot strand half of a retained native ligature.
            if !(cursor.is_cluster_boundary(start) || contextual::atom_boundary(span.atoms, start))
                || !(cursor.is_cluster_boundary(end) || contextual::atom_boundary(span.atoms, end)) {
                return None;
            }
            cursor.take_until(end)
        };
        span.consumed = end;

        let line = if !span.tab_stops.is_empty() {
            if !span.complete_paragraph { return None; }
            line.with_resolved_tab_stops(span.tab_stops, self.direction.resolve(&span.line.text), span.atoms, start)?
        } else { line };

        // A single owner can retain the native row, including its internal
        // bidi order. Splitting it into visual runs would copy glyph slices
        // without adding a host box or a required source-only anchor.
        if contexts.len() == 1 && !span.needs_source_runs {
            let scale = contexts[0].1;
            return Some(vec![Fragment {
                range,
                scale,
                line: Some(fit_fragment(line, span.line.font_size * scale, window)),
                visual_rank: None,
                atomic_rtl: false,
            }]);
        }

        // Omitted controls and objects must retain their own source embedding
        // levels. Resolve the complete row before replacing the native font
        // object cells with their independently measured host frames.
        let mut shared_clusters = line.clusters().filter(|cluster| {
            let first = contexts.partition_point(|(context, _)| context.end <= range.start + cluster.source.start);
            contexts[first..].iter()
                .take_while(|(context, _)| context.end < range.start + cluster.source.end)
                .any(|(context, _)| {
                    let cut = context.end - range.start;
                    !contextual::source_only(&line.text[cut..cluster.source.end], start + cut, span.atoms)
                })
        }).collect::<Vec<_>>();
        shared_clusters.sort_unstable_by_key(|cluster| cluster.source.start);
        let visual_source = if (span.needs_source_runs || !shared_clusters.is_empty()) && start < end {
            let cursor = span.cursor.get_or_insert_with(|| span.line.cursor_with_direction(self.direction));
            let visual = cursor.visual_source_runs(start..end);
            let safe = line.clusters().all(|cluster| {
                let source = start + cluster.source.start..start + cluster.source.end;
                visual.first_boundary(source).is_none_or(|boundary|
                    contextual::source_only(&line.text[boundary - start..cluster.source.end], boundary, span.atoms))
            });
            // A backend can keep a real ligature across source embedding
            // levels. Preserve that native cluster rather than strand part of
            // its visible source in a fragment with no glyphs.
            if !safe && (strict || !shared_clusters.is_empty()) { return None; }
            safe.then_some(visual)
        } else { None };

        struct Piece { range: Range<usize>, context: usize, visual_rank: usize, rtl: bool }
        let mut pieces: Vec<Piece> = Vec::new();
        let mut append = |source: Range<usize>, rtl: bool, merge: bool| {
            let start = range.start + source.start;
            let end = range.start + source.end;
            let first = contexts.partition_point(|(source, _)| source.end <= start);
            let count = if contexts[first].0.end >= end { 1 }
                else { contexts[first..].partition_point(|(source, _)| source.start < end) };
            for offset in 0..count {
                let context = first + if rtl { count - offset - 1 } else { offset };
                let bounds = &contexts[context].0;
                let source = bounds.start.max(start) - range.start..bounds.end.min(end) - range.start;
                if let Some(previous) = pieces.last_mut().filter(|previous|
                    merge && previous.context == context
                        && (previous.range.end == source.start || previous.range.start == source.end)) {
                    previous.range.start = previous.range.start.min(source.start);
                    previous.range.end = previous.range.end.max(source.end);
                } else {
                    pieces.push(Piece { range: source, context, visual_rank: pieces.len(), rtl });
                }
            }
        };
        if let Some(visual) = visual_source {
            for (source, rtl) in visual.iter() {
                // Do not merge distinct embedding runs: even two adjacent
                // ranges in one font context can have opposite caret edges.
                append(source.start - start..source.end - start, rtl, false);
            }
        } else {
            // Only a leading control can have an omitted prefix after preflight.
            // Ordinary rows need no additional glyph scan or source index.
            let first_source = if line.text.chars().next().is_some_and(contextual::format_control) {
                let mut first = line.len();
                for glyph in line.runs.iter().flat_map(|run| &run.glyphs) {
                    first = first.min(glyph.index);
                    if first == 0 { break; }
                }
                first
            } else { 0 };
            if first_source == line.len() {
                append(0..line.len(), false, true);
            } else {
                for cluster in line.clusters() {
                    let start = if cluster.source.start == first_source { 0 } else { cluster.source.start };
                    // The context containing the native source start owns the
                    // glyph; prefix/tail contexts were checked as controls only.
                    // This path preserves native paint order if source-run cuts
                    // would cross visible source inside a retained ligature.
                    append(start..cluster.source.end, cluster.is_rtl, true);
                }
            }
        }
        if !shared_clusters.is_empty() {
            // Only a boundary cluster gets an ink mask. Separate it from the
            // surrounding text so unrelated glyph overhangs remain intact.
            // Each source run has one direction; reverse its logical pieces
            // before assigning physical ranks, including zero-width anchors.
            let mut split = Vec::new();
            for piece in pieces {
                let mut cuts = smallvec::SmallVec::<[usize; 8]>::new();
                cuts.push(piece.range.start);
                let first = shared_clusters.partition_point(|cluster| cluster.source.end <= piece.range.start);
                for cluster in shared_clusters[first..].iter()
                    .take_while(|cluster| cluster.source.start < piece.range.end)
                {
                    for cut in [cluster.source.start, cluster.source.end] {
                        if cut > piece.range.start && cut < piece.range.end { cuts.push(cut); }
                    }
                }
                cuts.push(piece.range.end);
                cuts.dedup();
                let count = cuts.len() - 1;
                for offset in 0..count {
                    let index = if piece.rtl { count - offset - 1 } else { offset };
                    split.push(Piece { range: cuts[index]..cuts[index + 1], context: piece.context,
                        visual_rank: split.len(), rtl: piece.rtl });
                }
            }
            pieces = split;
        }
        pieces.sort_unstable_by_key(|piece| piece.range.start);
        if pieces.len() == 1 {
            let piece = pieces.pop().expect("one native piece");
            assert_eq!(piece.range, 0..line.len(), "native fragment must cover row source");
            let scale = contexts[piece.context].1;
            return Some(vec![Fragment {
                range,
                scale,
                line: (!contextual::is_atom(span.atoms, start..end))
                    .then(|| fit_fragment(line, span.line.font_size * scale, window)),
                visual_rank: None,
                atomic_rtl: piece.rtl,
            }]);
        }
        let reordered = pieces.iter().enumerate().any(|(index, piece)| index != piece.visual_rank);
        let mut cursor = line.cursor();
        let mut consumed = 0;
        let mut fragments = Vec::with_capacity(pieces.len());
        for piece in pieces {
            assert_eq!(piece.range.start, consumed, "native fragments must partition row source");
            let scale = contexts[piece.context].1;
            let size = span.line.font_size * scale;
            let shared = shared_clusters.get(shared_clusters.partition_point(|cluster| cluster.source.end <= piece.range.start))
                .filter(|cluster| cluster.source.start <= piece.range.start && piece.range.end <= cluster.source.end);
            let part = if let Some(cluster) = shared {
                if piece.range.start == cluster.source.start {
                    // Advance the ordinary source cursor once; subsequent
                    // paint owners retain only this boundary cluster's glyphs.
                    cursor.skip_until(cluster.source.end);
                }
                line.slice_cluster_for_paint(cluster, piece.range.clone())?
            } else {
                cursor.take_until(piece.range.end)
            };
            consumed = piece.range.end;
            fragments.push(Fragment {
                range: range.start + piece.range.start..range.start + piece.range.end,
                scale,
                line: (!contextual::is_atom(span.atoms, start + piece.range.start..start + piece.range.end))
                    .then(|| fit_fragment(part, size, window)),
                visual_rank: reordered.then_some(piece.visual_rank),
                atomic_rtl: piece.rtl,
            });
        }
        assert_eq!(consumed, line.len(), "native fragments must cover row source");
        Some(fragments)
    }

    pub fn take(&mut self, range: Range<usize>, runs: &[TextRun], scoped: bool, window: &Window) -> Option<Row> {
        while self.spans.get(self.next).is_some_and(|span| span.range.end <= range.start) {
            self.next += 1;
        }
        let span = self.spans.get_mut(self.next)?;
        // Contextual preparation owns this cursor, including on rejection:
        // fragment validation can fail after it has consumed the entire row.
        // Legacy fallback must reshape the source text rather than retry
        // a cursor that has already advanced beyond the requested fragment.
        if span.contexts.is_some() { return None; }
        if range.start < span.range.start || range.end > span.range.end {
            return None;
        }
        let start = range.start - span.range.start;
        let end = range.end - span.range.start;
        assert!(start >= span.consumed, "inline rows must follow source order");
        let whole_span = start == 0 && end == span.line.len();
        let (line, safe_cut) = if whole_span {
            (span.line.clone(), true)
        } else {
            // No source index or bidi analysis is needed for an uncut span.
            let cursor = span.cursor.get_or_insert_with(|| {
                if span.complete_paragraph { span.line.cursor_with_direction(self.direction) }
                else { span.line.cursor() }
            });
            if start > span.consumed { cursor.skip_until(start); }
            let safe_cut = cursor.is_cluster_boundary(start) && cursor.is_cluster_boundary(end);
            (cursor.take_until(end), safe_cut)
        };
        span.consumed = end;
        if !safe_cut { return Some(Row { line, reusable: false }); }
        // Keep native extents when both font sets still match. This also
        // avoids fetching per-font metrics on ordinary scoped soft rows.
        let requested_match = span.requested_fonts == font_ids(runs.iter().filter(|run| run.len > 0)
            .map(|run| window.text_system().resolve_font(&run.font))).as_slice();
        let actual_fonts = (!whole_span).then(|| font_ids(line.runs.iter()
            .filter(|run| !run.glyphs.is_empty()).map(|run| run.font_id)));
        if requested_match
            && actual_fonts.as_ref().is_none_or(|fonts| fonts.as_slice() == span.actual_fonts.as_slice()) {
            return Some(Row { line, reusable: true });
        }
        if scoped {
            // CSS boxes supply their own font struts and half-leading. Keep
            // the paragraph's glyphs even when this row omits a styled or
            // fallback font; carrying its aggregate extent would enlarge the
            // local glyph box. Baseline-relative glyphs need no y translation.
            if whole_span { return Some(Row { line, reusable: true }); }
            let fonts = actual_fonts.expect("a sliced row has an actual font set");
            if !fonts.is_empty() {
                let text = window.text_system();
                let (mut ascent, mut descent) = (Pixels::ZERO, Pixels::ZERO);
                for font in fonts {
                    ascent = ascent.max(text.ascent(font, line.font_size));
                    descent = descent.max(-text.descent(font, line.font_size));
                }
                let line = line.with_vertical_metrics(ascent, descent);
                return Some(Row { line, reusable: true });
            }
        }
        // Legacy Markdown/editor flows retain their native platform metrics.
        // Only identical font sets can reuse those aggregate extents safely.
        Some(Row { line, reusable: false })
    }
}

fn fit_fragment(line: ShapedLine, size: Pixels, window: &Window) -> ShapedLine {
    // Each piece owns its actual font extents. Carrying the full paragraph's
    // largest size into a small/sup/sub box changes its baseline, half-leading,
    // selection and decoration geometry. Single-piece rows reuse their layout.
    let line = line.with_default_font_size(size);
    let text = window.text_system();
    let (mut ascent, mut descent) = (Pixels::ZERO, Pixels::ZERO);
    let mut painted = false;
    for run in line.runs.iter().filter(|run| !run.glyphs.is_empty()) {
        painted = true;
        let size = run.font_size.unwrap_or(line.font_size);
        ascent = ascent.max(text.ascent(run.font_id, size));
        descent = descent.max(-text.descent(run.font_id, size));
    }
    if painted { line.with_vertical_metrics(ascent, descent) } else { line }
}

/// A source-only control box still has its authored font strut and selection
/// height. Resolve its requested fonts; another piece's glyph metrics do not
/// describe it, and a native backend need not return even an empty font run.
pub(super) fn fit_unpainted_fragment(line: ShapedLine, runs: &[TextRun], window: &Window) -> ShapedLine {
    if line.runs.iter().any(|run| !run.glyphs.is_empty()) { return line; }
    let text = window.text_system();
    let (mut ascent, mut descent) = (Pixels::ZERO, Pixels::ZERO);
    for run in runs.iter().filter(|run| run.len > 0) {
        let font = text.resolve_font(&run.font);
        ascent = ascent.max(text.ascent(font, line.font_size));
        descent = descent.max(-text.descent(font, line.font_size));
    }
    line.with_vertical_metrics(ascent, descent)
}

fn source_range(span: &ShapedSpan) -> &Range<usize> {
    match span {
        ShapedSpan::Text { range, .. } | ShapedSpan::Fixed { range, .. } => range,
    }
}
