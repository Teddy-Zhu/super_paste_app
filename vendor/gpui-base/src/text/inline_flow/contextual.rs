//! Whole-paragraph shaping for structured text with local size/box contexts.
//! Geometry remains in the ordinary flow and its existing retained layout.

use std::{borrow::Cow, ops::Range};
use unicode_segmentation::UnicodeSegmentation as _;

mod isolation;
mod wrap;

use gpui::{InlineObjectAdvance, Pixels, SharedString, Size, SizedTextRun, TextDirection, TextRun, TextStyle, Window};

use super::{ATOM_TEXT, MeasureItem, MeasuredInlineObject, ShapedSpan, prepared, shared_text_range};
use super::super::inline::{InlineHighlight, text_runs_for_window, text_size_ranges};

pub(super) struct Paragraph<'a> {
    text: Cow<'a, SharedString>,
    runs: Vec<(Range<usize>, TextRun)>,
    atoms: Vec<InlineObjectAdvance>,
    contexts: Vec<(Range<usize>, f32)>,
    /// Final child cuts must not introduce artificial font shaping breaks.
    /// Single-source paragraphs retain their existing context boundaries.
    native_breaks: Option<Vec<usize>>,
    nowrap: Vec<Range<usize>>,
    space_policies: Vec<(Range<usize>, gpui::TextSpacePolicy)>,
    font_size: Pixels,
    isolates: Vec<gpui::TextIsolation>,
}

impl<'a> Paragraph<'a> {
    pub fn new(
        items: &'a [MeasureItem], images: &[Option<Size<Pixels>>],
        objects: &[Option<MeasuredInlineObject>], style: &TextStyle, window: &Window,
    ) -> Option<Self> {
        let mut single = None;
        let mut parts = 0;
        let mut has_isolation = false;
        for item in items {
            match item {
                MeasureItem::Box { inline_box, .. } => {
                    has_isolation |= inline_box.has_isolation();
                    continue;
                },
                MeasureItem::Text { text, highlights, .. } => {
                    let mut covered = 0;
                    for (range, mark) in highlights {
                        has_isolation |= mark.inline_box.as_ref().is_some_and(|source| source.has_isolation());
                        if range.start < covered || range.start > range.end || range.end > text.len()
                            || !text.is_char_boundary(range.start) || !text.is_char_boundary(range.end)
                            || mark.font_size_scale.is_some()
                            || mark.code_style.is_some() { return None; }
                        covered = range.end;
                    }
                    if text.is_empty() { continue; }
                    single = Some((text, highlights));
                },
                MeasureItem::Object { inline_box, .. } | MeasureItem::Image { inline_box, .. } => {
                    has_isolation |= inline_box.as_ref().is_some_and(|source| source.has_isolation());
                },
            }
            parts += 1;
        }
        let isolates = if has_isolation { isolation::collect(items)? } else { Vec::new() };
        // A single object has no internal ordering; neutral object-only LTR
        // flows already have the right order. Keep their direct measured path
        // instead of shaping unused font glyphs for ordinary image previews.
        if isolates.is_empty() && single.is_none() && (parts < 2 || style.text_direction != TextDirection::Rtl) {
            return None;
        }
        let mut atoms = Vec::new();
        let mut cuts = Vec::new();
        let (text, highlights) = if parts == 1 && let Some((text, highlights)) = single {
            (Cow::Borrowed(text), Cow::Borrowed(highlights.as_slice()))
        } else {
            // One temporary source for the flow, never one source per soft
            // row. Native lines retain the resulting shared hard-line text.
            let mut text = String::with_capacity(items.iter().map(MeasureItem::len).sum());
            let mut highlights = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let start = text.len();
                match item {
                    MeasureItem::Box { .. } => continue,
                    MeasureItem::Text { text: source, highlights: marks, .. } => {
                        text.push_str(source);
                        highlights.extend(marks.iter().map(|(range, mark)|
                            (start + range.start..start + range.end, mark.clone())));
                    },
                    MeasureItem::Object { no_wrap, inline_box, .. }
                    | MeasureItem::Image { no_wrap, inline_box, .. } => {
                        let width = match item {
                            MeasureItem::Object { .. } => objects.get(index)?.as_ref()?.metrics.size.width,
                            _ => images.get(index)?.as_ref()?.width,
                        };
                        if !width.as_f32().is_finite() || width < Pixels::ZERO { return None; }
                        text.push_str(ATOM_TEXT);
                        atoms.push(InlineObjectAdvance { index: start, width });
                        highlights.push((start..text.len(), InlineHighlight {
                            no_wrap: *no_wrap, inline_box: inline_box.clone(), ..Default::default()
                        }));
                    },
                }
                if text.len() > start { cuts.push(text.len()); }
            }
            (Cow::Owned(SharedString::from(text)), Cow::Owned(highlights))
        };
        if text.is_empty() { return None; }
        if isolates.iter().any(|scope| !text.is_char_boundary(scope.range.start)
            || !text.is_char_boundary(scope.range.end)) { return None; }
        let font_size = style.font_size.to_pixels(window.rem_size());
        let sizes = text_size_ranges(text.len(), &highlights, font_size, window.rem_size());
        // Resolve sparse highlights once. The existing run builder supplies
        // inherited body attributes for gaps; no expanded mark copy is needed.
        let mut offset = 0;
        let runs = text_runs_for_window(text.len(), style, &highlights, window).into_iter()
            .filter(|run| run.len > 0).map(|run| {
                let start = offset;
                offset += run.len;
                (start..offset, run)
            }).collect::<Vec<_>>();
        let context_capacity = sizes.len() + cuts.len();
        let mut child_cuts = cuts.into_iter().peekable();
        let mut paint_cuts = runs.windows(2).filter_map(|pair|
            (!same_paint(&pair[0].1, &pair[1].1)).then_some(pair[1].0.start)).peekable();
        let has_cuts = child_cuts.peek().is_some() || paint_cuts.peek().is_some();
        // Paint/child ownership must not introduce artificial shaping breaks.
        // Keep only the original size/box contexts as native font boundaries.
        let native_breaks = has_cuts.then(|| sizes.iter().map(|(range, _)| range.start).collect::<Vec<_>>());
        let contexts = if !has_cuts { sizes } else {
            // Both streams are ordered. Merge them while partitioning contexts
            // instead of sorting or storing a second boundary index.
            let mut boundaries = std::iter::from_fn(|| match (child_cuts.peek(), paint_cuts.peek()) {
                (Some(child), Some(paint)) if child == paint => {
                    paint_cuts.next(); child_cuts.next()
                },
                (Some(child), Some(paint)) if child < paint => child_cuts.next(),
                (Some(_), Some(_)) | (None, Some(_)) => paint_cuts.next(),
                (Some(_), None) => child_cuts.next(),
                (None, None) => None,
            }).peekable();
            let mut contexts = Vec::with_capacity(context_capacity);
            for (range, scale) in sizes {
                while boundaries.peek().is_some_and(|at| *at <= range.start) { boundaries.next(); }
                let mut start = range.start;
                while let Some(&end) = boundaries.peek().filter(|at| **at < range.end) {
                    contexts.push((start..end, scale));
                    start = end;
                    boundaries.next();
                }
                contexts.push((start..range.end, scale));
            }
            contexts
        };
        let mut nowrap: Vec<Range<usize>> = Vec::new();
        for (range, mark) in highlights.iter() {
            if !mark.no_wrap || range.is_empty() { continue; }
            if let Some(previous) = nowrap.last_mut().filter(|previous| previous.end == range.start) {
                previous.end = range.end;
            } else { nowrap.push(range.clone()); }
        }
        let mut space_policies: Vec<(Range<usize>, gpui::TextSpacePolicy)> = Vec::new();
        for (range, mark) in highlights.iter() {
            let Some(policy) = mark.space_policy.filter(|_| !range.is_empty()) else { continue; };
            if let Some((previous, _)) = space_policies.last_mut()
                .filter(|(previous, previous_policy)| previous.end == range.start && *previous_policy == policy) {
                previous.end = range.end;
            } else { space_policies.push((range.clone(), policy)); }
        }
        Some(Self { text, runs, atoms, contexts, native_breaks, nowrap, space_policies, font_size, isolates })
    }

    pub fn shape(&self, style: &TextStyle, hard_line: &Range<usize>, window: &mut Window) -> Option<ShapedSpan> {
        self.shape_with_breaks(style, hard_line, &[], None, window)
    }

    fn shape_with_breaks(&self, style: &TextStyle, hard_line: &Range<usize>, breaks: &[usize], source: Option<SharedString>, window: &mut Window) -> Option<ShapedSpan> {
        if hard_line.is_empty() { return None; }
        let first = self.contexts.partition_point(|(range, _)| range.end <= hard_line.start);
        let contexts = self.contexts[first..].iter().take_while(|(range, _)| range.start < hard_line.end)
            .map(|(range, scale)| (range.start.max(hard_line.start)..range.end.min(hard_line.end), *scale))
            .collect::<Vec<_>>();
        let first = self.nowrap.partition_point(|range| range.end <= hard_line.start);
        let nowrap = self.nowrap[first..].iter().take_while(|range| range.start < hard_line.end)
            .map(|range| range.start.max(hard_line.start) - hard_line.start
                ..range.end.min(hard_line.end) - hard_line.start).collect::<Vec<_>>();
        let first = self.space_policies.partition_point(|(range, _)| range.end <= hard_line.start);
        let space_policies = self.space_policies[first..].iter().take_while(|(range, _)| range.start < hard_line.end)
            .map(|(range, policy)| (range.start.max(hard_line.start) - hard_line.start
                ..range.end.min(hard_line.end) - hard_line.start, *policy)).collect();
        let first = self.atoms.partition_point(|atom| atom.index < hard_line.start);
        let atoms = self.atoms[first..].iter().take_while(|atom| atom.index < hard_line.end)
            .map(|atom| InlineObjectAdvance { index: atom.index - hard_line.start, width: atom.width })
            .collect::<Vec<_>>();
        let text = self.text.as_ref();
        let font_size = self.font_size;
        let mut runs = Vec::new();
        for (range, scale) in &contexts {
            let break_before = range.start == hard_line.start
                || self.native_breaks.as_ref().is_none_or(|breaks| breaks.binary_search(&range.start).is_ok())
                || atom_boundary(&self.atoms, range.start);
            let first = self.runs.partition_point(|(source, _)| source.end <= range.start);
            runs.extend(self.runs[first..].iter().take_while(|(source, _)| source.start < range.end)
                .enumerate().map(|(index, (source, run))| {
                    let mut run = run.clone();
                    run.len = source.end.min(range.end) - source.start.max(range.start);
                    SizedTextRun { run, font_size: font_size * *scale, break_before: index == 0 && break_before }
                }));
        }
        if !breaks.is_empty() {
            let mut partitioned = Vec::with_capacity(runs.len() + breaks.len());
            let mut offset = hard_line.start;
            for sized in runs {
                let end = offset + sized.run.len;
                let mut start = offset;
                let first = breaks.partition_point(|cut| *cut <= start);
                for cut in breaks[first..].iter().copied().take_while(|cut| *cut < end).chain(std::iter::once(end)) {
                    let mut part = sized.clone();
                    part.run.len = cut - start;
                    part.break_before = (start == offset && sized.break_before) || breaks.binary_search(&start).is_ok();
                    partitioned.push(part);
                    start = cut;
                }
                offset = end;
            }
            runs = partitioned;
        }
        if runs.len() > 1 {
            // Paint can span a native ligature, but a source grapheme cannot
            // acquire two partial ink owners. Keep the toolkit's native font
            // boundary at such style cuts, without allocating a boundary map.
            let mut graphemes = text[hard_line.clone()].grapheme_indices(true)
                .map(|(offset, _)| offset).peekable();
            let mut offset = 0;
            for run in &mut runs {
                while graphemes.peek().is_some_and(|boundary| *boundary < offset) { graphemes.next(); }
                if offset > 0 && graphemes.peek() != Some(&offset) { run.break_before = true; }
                offset += run.run.len;
            }
        }
        let isolates = isolation::clipped(&self.isolates, text, hard_line)?;
        let line = window.text_system().shape_line_with_sized_runs_and_isolates(
            source.unwrap_or_else(|| shared_text_range(text, hard_line.clone())), font_size, &runs, None, style.text_direction, &isolates,
        );
        let tab_stops = resolved_tab_stops(&line.text, &runs, style.tab_size, window)?;

        // A paint-only boundary can share a retained cluster at a native
        // insertion. Font/box changes still require a real shaping boundary;
        // omitted source-only controls can keep their zero-width anchors.
        let mut covered = 0;
        let mut first = line.len();
        let mut omitted_controls = false;
        for cluster in line.clusters() {
            let start = hard_line.start + cluster.source.start;
            let first_context = contexts.partition_point(|(range, _)| range.end <= start);
            let mut previous_caret = line.cluster_caret_x(&cluster, cluster.source.start)?;
            for (context, _) in contexts[first_context..].iter()
                .take_while(|(range, _)| range.end < hard_line.start + cluster.source.end)
            {
                let cut = context.end - hard_line.start;
                if source_only(&line.text[cut..cluster.source.end], cut, &atoms) { continue; }
                if self.native_breaks.as_ref().is_none_or(|breaks| breaks.binary_search(&context.end).is_ok()) {
                    return None;
                }
                let caret = line.cluster_caret_x(&cluster, cut)?;
                if (cluster.is_rtl && caret > previous_caret) || (!cluster.is_rtl && caret < previous_caret) {
                    return None;
                }
                previous_caret = caret;
            }
            first = first.min(cluster.source.start);
            covered += cluster.source.len();
            // A control at the cluster's native source start has a glyph
            // owner already. Only its source tail may need an independent
            // zero-width anchor; ordinary single-character clusters are cheap.
            omitted_controls = omitted_controls
                || line.text[cluster.source.clone()].chars().skip(1).any(format_control);
        }
        if covered != line.len() - first || !source_only(&line.text[..first], 0, &atoms) { return None; }
        if first == line.len() && line.width() != Pixels::ZERO { return None; }

        Some(ShapedSpan::Text {
            range: hard_line.clone(),
            line: Box::new(line),
            padding: gpui::Pixels::ZERO,
            nowrap_ranges: nowrap,
            space_policies,
            requested_fonts: prepared::font_ids(runs.iter().filter(|run| run.run.len > 0)
                .map(|run| window.text_system().resolve_font(&run.run.font))),
            contexts: Some(contexts),
            needs_source_runs: omitted_controls || first != 0 || !atoms.is_empty(),
            atoms,
            tab_stops,
        })
    }
}

/// A conservative set of invisible formatting characters. Use it only for
/// source bytes a native backend actually omitted; never infer missing text
/// from a zero advance or discard printable content, marks or soft hyphens.
pub(super) fn format_control(character: char) -> bool {
    matches!(character, '\u{061c}' | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

/// Resolve only actual preserved tabs. Equal geometry coalesces across text
/// gaps without retaining a second per-character style or font table.
fn resolved_tab_stops(text: &str, runs: &[SizedTextRun], spaces: Option<u32>, window: &Window) -> Option<Vec<gpui::TabStopRun>> {
    let Some(spaces) = spaces.filter(|_| text.contains('\t')) else { return Some(Vec::new()); };
    if spaces == 0 {
        let stops = gpui::TabStops::new(Pixels::ZERO, Pixels::ZERO)?;
        return Some(vec![gpui::TabStopRun { range: 0..text.len(), stops }]);
    }
    let mut resolved: Vec<gpui::TabStopRun> = Vec::new();
    let mut run_index = 0;
    let mut run_end = runs.first()?.run.len;
    let mut previous: Option<(&gpui::Font, Pixels, gpui::TabStops)> = None;
    for (index, _) in text.match_indices('\t') {
        while index >= run_end {
            run_index += 1;
            run_end += runs.get(run_index)?.run.len;
        }
        let owner = &runs[run_index];
        // Use the requested font before fallback, with the exact size used
        // for native shaping. Font/size changes resolve at layout time, not
        // while adapting the shared document into persistent view state.
        let stops = match previous {
            Some((font, size, stops)) if font == &owner.run.font && size == owner.font_size => stops,
            _ => {
                let system = window.text_system();
                let font = system.resolve_font(&owner.run.font);
                let stops = gpui::TabStops::new(
                    system.layout_width(font, owner.font_size, ' ') * spaces as f32,
                    system.layout_width(font, owner.font_size, '0') * 0.5,
                )?;
                previous = Some((&owner.run.font, owner.font_size, stops));
                stops
            },
        };
        if let Some(previous) = resolved.last_mut().filter(|run| run.stops == stops) {
            previous.range.end = index + 1;
        } else { resolved.push(gpui::TabStopRun { range: index..index + 1, stops }); }
    }
    Some(resolved)
}

/// Native font runs own font changes. Paint changes additionally need a safe
/// source cut: one retained ligature cannot receive two independent colors or
/// decorating-box ancestries. nowrap alone introduces no such ownership cut.
fn same_paint(left: &TextRun, right: &TextRun) -> bool {
    left.color == right.color && left.background_color == right.background_color
        && left.underline == right.underline && left.overline == right.overline
        && left.strikethrough == right.strikethrough
        && left.decoration_layers == right.decoration_layers
}

/// Only explicitly typed source atoms may stand in for a native glyph a
/// backend omitted. A literal U+FFFC in text remains ordinary printable input.
pub(super) fn source_only(text: &str, start: usize, atoms: &[InlineObjectAdvance]) -> bool {
    text.char_indices().all(|(offset, character)| format_control(character)
        || (character == '\u{fffc}' && atoms.binary_search_by_key(&(start + offset), |atom| atom.index).is_ok()))
}

pub(super) fn is_atom(atoms: &[InlineObjectAdvance], range: Range<usize>) -> bool {
    atoms.binary_search_by_key(&range.start, |atom| atom.index).ok()
        .is_some_and(|index| atoms[index].source_range() == range)
}

pub(super) fn atom_boundary(atoms: &[InlineObjectAdvance], at: usize) -> bool {
    let index = atoms.partition_point(|atom| atom.index < at);
    atoms.get(index).is_some_and(|atom| atom.index == at)
        || index.checked_sub(1).is_some_and(|index| atoms[index].source_range().end == at)
}
