//! A visual row whose offsets remain in source UTF-8 bytes.
use gpui::{App, CaretAffinity, Pixels, Point, ShapedLine, SharedString, TextAlign, TextDirection, TextHit, Window, point, px};
use std::ops::Range;

pub(crate) struct InlineFragment {
    pub(crate) range: Range<usize>,
    pub(crate) x: Pixels,
    pub(crate) width: Pixels,
    pub(crate) text: Option<ShapedLine>,
}

pub(crate) struct InputLine {
    pub(crate) len: usize,
    pub(crate) width: Pixels,
    pub(crate) text: SharedString,
    /// Resolved direction of the enclosing hard paragraph, not this soft row.
    pub(crate) direction: TextDirection,
    content: Content,
}
// Keep the ordinary shaped row inline: boxing it would add an allocation to
// every existing plain-text row. Only token rows allocate fragment storage.
#[allow(clippy::large_enum_variant)]
enum Content {
    Text(ShapedLine),
    Inline(Vec<InlineFragment>),
}
impl From<ShapedLine> for InputLine {
    fn from(line: ShapedLine) -> Self {
        Self::text_with_direction(line, TextDirection::Auto)
    }
}
impl InputLine {
    pub(crate) fn text_with_direction(line: ShapedLine, direction: TextDirection) -> Self {
        let direction = if direction == TextDirection::Auto {
            line.detected_paragraph_direction().unwrap_or(TextDirection::Ltr)
        } else { direction };
        Self {
            len: line.len,
            width: line.width,
            text: line.text.to_shared_string(),
            direction,
            content: Content::Text(line),
        }
    }
    pub(crate) fn inline(text: SharedString, fragments: Vec<InlineFragment>, direction: TextDirection) -> Self {
        let width = fragments.last().map_or(px(0.), |f| f.x + f.width);
        Self {
            len: text.len(),
            direction: direction.resolve(&text),
            text,
            width,
            content: Content::Inline(fragments),
        }
    }
    pub(crate) fn x_for_index(&self, ix: usize) -> Pixels {
        self.x_for_index_with_affinity(ix, CaretAffinity::Downstream)
    }

    pub(crate) fn x_for_index_with_affinity(&self, ix: usize, affinity: CaretAffinity) -> Pixels {
        match &self.content {
            Content::Text(line) => line.x_for_index_with_affinity(ix, affinity),
            Content::Inline(fragments) => {
                for f in fragments {
                    if ix < f.range.end || (affinity == CaretAffinity::Upstream && ix == f.range.end && ix > f.range.start) {
                        return f.x
                            + f.text.as_ref().map_or(
                                if ix <= f.range.start { px(0.) } else { f.width },
                                |line| line.x_for_index_with_affinity(ix.saturating_sub(f.range.start), affinity),
                            );
                    }
                }
                self.width
            }
        }
    }
    pub(crate) fn closest_index_for_x(&self, x: Pixels) -> usize {
        self.closest_caret_for_x(x).index
    }

    pub(crate) fn closest_caret_for_x(&self, x: Pixels) -> TextHit {
        match &self.content {
            Content::Text(line) => line.closest_caret_for_x(x),
            Content::Inline(fragments) => {
                for (index, f) in fragments.iter().enumerate() {
                    if x <= f.x + f.width || index + 1 == fragments.len() {
                        return match &f.text {
                            Some(line) => {
                                let mut hit = line.closest_caret_for_x(x - f.x);
                                hit.index += f.range.start;
                                hit
                            }
                            None if x - f.x < f.width / 2. => TextHit { index: f.range.start, affinity: CaretAffinity::Downstream },
                            None => TextHit { index: f.range.end, affinity: CaretAffinity::Upstream },
                        };
                    }
                }
                TextHit { index: self.len, affinity: CaretAffinity::Upstream }
            }
        }
    }
    pub(crate) fn index_for_x(&self, x: Pixels) -> Option<usize> {
        match &self.content {
            Content::Text(line) => line.index_for_x(x),
            Content::Inline(_) => {
                (x >= px(0.) && x <= self.width).then(|| self.closest_index_for_x(x))
            }
        }
    }

    pub(crate) fn adjacent_visual_caret(&self, mut from: TextHit, right: bool, mut valid: impl FnMut(usize) -> bool) -> Option<TextHit> {
        if from.index == 0 { from.affinity = CaretAffinity::Downstream; }
        else if from.index == self.len { from.affinity = CaretAffinity::Upstream; }
        match &self.content {
            Content::Text(line) => line.adjacent_visual_caret(Some(from), right, valid),
            Content::Inline(fragments) => {
                let mut previous = None;
                let mut entered = false;
                for fragment in fragments {
                    let owns = match from.affinity {
                        CaretAffinity::Upstream => fragment.range.start < from.index && from.index <= fragment.range.end,
                        CaretAffinity::Downstream => fragment.range.start <= from.index && from.index < fragment.range.end,
                    };
                    if owns {
                        if let Some(hit) = Self::fragment_navigation_caret(fragment, Some(from), right, &mut valid) { return Some(hit); }
                        if !right { return previous; }
                        entered = true;
                    } else if entered || !right {
                        let target = Self::fragment_navigation_caret(fragment, None, right, &mut valid);
                        if right && target.is_some() { return target; }
                        if !right && target.is_some() { previous = target; }
                    }
                }
                None
            }
        }
    }

    /// Used only after ordinary native traversal fails, so a missing source
    /// prefix does not add a scan to every successful arrow-key movement.
    pub(crate) fn resolve_visual_caret(&self, mut hit: TextHit) -> Option<TextHit> {
        if hit.index > self.len { return None; }
        match &self.content {
            Content::Text(line) => line.resolve_visual_caret(hit),
            Content::Inline(fragments) => {
                if hit.index == 0 { hit.affinity = CaretAffinity::Downstream; }
                else if hit.index == self.len { hit.affinity = CaretAffinity::Upstream; }
                let fragment = fragments.iter().find(|fragment| match hit.affinity {
                    CaretAffinity::Upstream => fragment.range.start < hit.index && hit.index <= fragment.range.end,
                    CaretAffinity::Downstream => fragment.range.start <= hit.index && hit.index < fragment.range.end,
                });
                let Some(fragment) = fragment else {
                    return (self.len == 0).then_some(hit);
                };
                if let Some(line) = &fragment.text {
                    let mut resolved = line.resolve_visual_caret(TextHit {
                        index: hit.index - fragment.range.start, affinity: hit.affinity,
                    })?;
                    resolved.index += fragment.range.start;
                    Some(resolved)
                } else {
                    Some(hit)
                }
            }
        }
    }

    fn fragment_navigation_caret(fragment: &InlineFragment, from: Option<TextHit>, right: bool, valid: &mut impl FnMut(usize) -> bool) -> Option<TextHit> {
        if let Some(line) = &fragment.text {
            let from = from.map(|hit| TextHit { index: hit.index - fragment.range.start, affinity: hit.affinity });
            let mut target = line.adjacent_visual_caret(from, right, |index| valid(fragment.range.start + index));
            if target.is_none() {
                if let Some(from) = from {
                    if let Some(resolved) = line.resolve_visual_caret(from).filter(|hit| *hit != from) {
                        target = line.adjacent_visual_caret(Some(resolved), right, |index| valid(fragment.range.start + index));
                    }
                }
            }
            return target.map(|mut hit| {
                hit.index += fragment.range.start;
                hit
            });
        }
        let index = if right { fragment.range.end } else { fragment.range.start };
        (from.is_none_or(|hit| hit.index != index) && valid(index)).then_some(TextHit {
            index, affinity: if right { CaretAffinity::Upstream } else { CaretAffinity::Downstream },
        })
    }

    pub(crate) fn visual_edge_caret(&self, right: bool, mut valid: impl FnMut(usize) -> bool) -> Option<TextHit> {
        match &self.content {
            Content::Text(line) => line.visual_edge_caret(right, valid),
            Content::Inline(fragments) => {
                let mut edge = None;
                for fragment in fragments {
                    let target = if let Some(line) = &fragment.text {
                        line.visual_edge_caret(right, |index| valid(fragment.range.start + index)).map(|mut hit| {
                            hit.index += fragment.range.start;
                            hit
                        })
                    } else {
                        let index = if right { fragment.range.end } else { fragment.range.start };
                        valid(index).then_some(TextHit { index, affinity: if right { CaretAffinity::Upstream } else { CaretAffinity::Downstream } })
                    };
                    if !right && target.is_some() { return target; }
                    if right && target.is_some() { edge = target; }
                }
                edge.or_else(|| (self.len == 0 && valid(0)).then_some(TextHit { index: 0, affinity: CaretAffinity::Downstream }))
            }
        }
    }

    /// Emit physical cells for one source range, retaining disjoint bidi runs.
    /// A selected hard newline owns a space cell at the logical trailing edge.
    pub(crate) fn for_each_selection_span(
        &self,
        range: Range<usize>,
        newline_width: Option<Pixels>,
        mut visit: impl FnMut(Range<Pixels>),
    ) {
        let trailing = match &self.content {
            Content::Text(line) => Self::text_selection_spans(line, &range, 0, px(0.), &mut visit),
            Content::Inline(fragments) => {
                let mut trailing = (self.width, false);
                for fragment in fragments {
                    let selected = !range.is_empty() && range.start < fragment.range.end && range.end > fragment.range.start;
                    if !selected && !(newline_width.is_some() && fragment.range.end == self.len) {
                        continue;
                    }
                    trailing = match &fragment.text {
                        Some(line) => Self::text_selection_spans(line, &range, fragment.range.start, fragment.x, &mut visit),
                        None => {
                            if selected && fragment.width > px(0.) {
                                visit(fragment.x..fragment.x + fragment.width);
                            }
                            (fragment.x + fragment.width, false)
                        }
                    };
                }
                trailing
            }
        };
        if let Some(width) = newline_width.filter(|width| *width > px(0.)) {
            let (x, rtl) = trailing;
            visit(if rtl { x - width..x } else { x..x + width });
        }
    }

    fn text_selection_spans(
        line: &ShapedLine,
        range: &Range<usize>,
        source_offset: usize,
        x: Pixels,
        visit: &mut impl FnMut(Range<Pixels>),
    ) -> (Pixels, bool) {
        let mut trailing = (x + line.width, false);
        for cluster in line.clusters() {
            if cluster.source.end == line.len {
                trailing = (x + if cluster.is_rtl { cluster.cell.start } else { cluster.cell.end }, cluster.is_rtl);
            }
            if !range.is_empty() && source_offset + cluster.source.start < range.end
                && source_offset + cluster.source.end > range.start
                && cluster.cell.start < cluster.cell.end
            {
                for (source, cell) in line.cluster_cells(&cluster) {
                    if source_offset + source.start < range.end && source_offset + source.end > range.start {
                        visit(x + cell.start..x + cell.end);
                    }
                }
            }
        }
        trailing
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint_pass(pos, height, align, width, false, window, cx);
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_background(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint_pass(pos, height, align, width, true, window, cx);
    }
    #[allow(clippy::too_many_arguments)]
    fn paint_pass(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        background: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let paint = |line: &ShapedLine, pos, align, width, window: &mut Window, cx: &mut App| {
            if background {
                let _ = line.paint_background(pos, height, align, width, window, cx);
            } else {
                let _ = line.paint(pos, height, align, width, window, cx);
            }
        };
        match &self.content {
            Content::Text(line) => paint(line, pos, align, width, window, cx),
            Content::Inline(fragments) => {
                let remaining = (width.unwrap_or(self.width) - self.width).max(px(0.));
                let offset = match align {
                    TextAlign::Right => remaining,
                    TextAlign::Center => remaining / 2.,
                    _ => px(0.),
                };
                for f in fragments {
                    if let Some(line) = &f.text {
                        paint(
                            line,
                            pos + point(offset + f.x, px(0.)),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            }
        }
    }
}
