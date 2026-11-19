use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use gpui::{Bounds, Edges, Font, FontStyle, Hsla, Pixels, Window, point, px, quad, size};

use super::node::TextMark;

/// An authored length keeps its computing font/viewport basis in the host.
/// Layout resolves it once per inline scope, alongside existing font geometry.
#[derive(Clone)]
pub struct InlineBaselineOffset {
    resolve: Arc<dyn Fn(&Font, &Window) -> Pixels + Send + Sync>,
    viewport_dependent: bool,
}
impl InlineBaselineOffset {
    pub fn new(viewport_dependent: bool, resolve: impl Fn(&Font, &Window) -> Pixels + Send + Sync + 'static) -> Self {
        Self { resolve: Arc::new(resolve), viewport_dependent }
    }
    pub(super) fn pixels(&self, font: &Font, window: &Window) -> Pixels { (self.resolve)(font, window) }
    pub(super) fn viewport_dependent(&self) -> bool { self.viewport_dependent }
}
impl std::fmt::Debug for InlineBaselineOffset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct("InlineBaselineOffset").field("viewport_dependent", &self.viewport_dependent).finish_non_exhaustive() }
}
impl PartialEq for InlineBaselineOffset {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.resolve, &other.resolve) }
}

/// Alignment of a non-replaced inline box. Font-relative values resolve
/// against the parent; line-edge values position a complete aligned subtree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineVerticalAlign {
    #[default]
    Baseline,
    Middle,
    TextTop,
    TextBottom,
    Top,
    Bottom,
}

impl InlineVerticalAlign {
    fn is_edge(self) -> bool { matches!(self, Self::Top | Self::Bottom) }
    fn is_font_relative(self) -> bool { matches!(self, Self::Middle | Self::TextTop | Self::TextBottom) }
}

/// One structured inline box's computed typography and optional fill. Identity
/// distinguishes siblings from fragments; children share the ancestor chain.
#[derive(Debug)]
pub struct InlineBox {
    parent: Option<Arc<Self>>,
    color: Option<Hsla>,
    family: Option<gpui::SharedString>,
    weight: gpui::FontWeight,
    style: FontStyle,
    font_size: Pixels,
    baseline_shift: Pixels,
    baseline_offset: Option<InlineBaselineOffset>,
    viewport_dependent: bool,
    line_height: Pixels,
    root: bool,
    paints_background: bool,
    alignment: InlineVerticalAlign,
    isolation: Option<gpui::TextDirection>,
    isolation_parent: Option<Arc<Self>>,
}

impl PartialEq for InlineBox {
    fn eq(&self, other: &Self) -> bool { std::ptr::eq(self, other) }
}

impl InlineBox {
    /// `body_size` is the structured host's body em, the unit used by the
    /// mark's cumulative baseline displacement. No child resources are held.
    pub fn new(parent: Option<Arc<Self>>, color: Option<Hsla>, mark: &TextMark,
        font_size: Pixels, body_size: Pixels, line_height: Pixels) -> Arc<Self>
    {
        Self::aligned(parent, color, mark, font_size, body_size, line_height, InlineVerticalAlign::Baseline)
    }

    /// Font-relative alignment needs the computing parent's font scope;
    /// line-edge roots may start directly in an anonymous paragraph.
    pub fn aligned(parent: Option<Arc<Self>>, color: Option<Hsla>, mark: &TextMark,
        font_size: Pixels, body_size: Pixels, line_height: Pixels, alignment: InlineVerticalAlign) -> Arc<Self>
    {
        Self::aligned_with_isolation(parent, color, mark, font_size, body_size, line_height, alignment, None)
    }

    /// An isolate is part of the inline scope identity, including when it is
    /// empty. It does not alter the canonical text or allocate a text view.
    pub fn aligned_with_isolation(parent: Option<Arc<Self>>, color: Option<Hsla>, mark: &TextMark,
        font_size: Pixels, body_size: Pixels, line_height: Pixels, alignment: InlineVerticalAlign,
        isolation: Option<gpui::TextDirection>) -> Arc<Self>
    {
        Self::aligned_with_offset(parent, color, mark, font_size, body_size, line_height, alignment, isolation, None)
    }

    pub fn aligned_with_offset(parent: Option<Arc<Self>>, color: Option<Hsla>, mark: &TextMark,
        font_size: Pixels, body_size: Pixels, line_height: Pixels, alignment: InlineVerticalAlign,
        isolation: Option<gpui::TextDirection>, baseline_offset: Option<InlineBaselineOffset>) -> Arc<Self>
    {
        assert!(!alignment.is_font_relative() || parent.is_some(),
            "font-relative alignment requires parent typography");
        let color = color.filter(|color| color.a > 0.);
        let paints_background = color.is_some() || parent.as_ref().is_some_and(|parent| parent.paints_background);
        let isolation_parent = parent.as_ref().and_then(Self::enclosing_isolation);
        let viewport_dependent = baseline_offset.as_ref().is_some_and(InlineBaselineOffset::viewport_dependent)
            || parent.as_ref().is_some_and(|parent| parent.viewport_dependent);
        Arc::new(Self {
            parent, color, family: mark.font_family.clone(),
            weight: mark.font_weight.unwrap_or(if mark.bold { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL }),
            style: mark.resolved_font_style(),
            font_size, baseline_shift: body_size * mark.baseline_shift.unwrap_or(0.),
            baseline_offset, viewport_dependent,
            line_height, root: false, paints_background, alignment, isolation, isolation_parent,
        })
    }

    /// Replaced content contributes its own metrics. Its directional scope
    /// only repeats the parent's strut, without adding a shift or line group.
    pub fn isolated(parent: Arc<Self>, direction: gpui::TextDirection) -> Arc<Self> {
        Arc::new(Self {
            color: None, family: parent.family.clone(), weight: parent.weight, style: parent.style,
            font_size: parent.font_size, baseline_shift: Pixels::ZERO, line_height: parent.line_height,
            baseline_offset: None, viewport_dependent: parent.viewport_dependent,
            root: false, paints_background: parent.paints_background, alignment: InlineVerticalAlign::Baseline,
            isolation: Some(direction), isolation_parent: Self::enclosing_isolation(&parent), parent: Some(parent),
        })
    }

    fn enclosing_isolation(parent: &Arc<Self>) -> Option<Arc<Self>> {
        if parent.isolation.is_some() { Some(parent.clone()) } else { parent.isolation_parent.clone() }
    }

    pub(super) fn has_isolation(&self) -> bool { self.isolation.is_some() || self.isolation_parent.is_some() }

    pub(super) fn isolation_direction(&self) -> Option<gpui::TextDirection> { self.isolation }

    /// Reuse one temporary chain per paragraph; ordinary typography skips
    /// the ancestry walk completely. Direct ancestor links skip intermediate
    /// typography boxes; scope references live only during layout.
    pub(super) fn isolation_chain<'a>(&'a self, chain: &mut Vec<&'a Self>) {
        let mut current = Some(self);
        while let Some(source) = current {
            if source.isolation.is_some() { chain.push(source); }
            current = source.isolation_parent.as_deref();
        }
        chain.reverse();
    }

    /// A paragraph's own CSS font strut. All of its scopes share this root;
    /// its metrics do not depend on which fallback fonts appear on a soft row.
    pub fn root(mark: &TextMark, font_size: Pixels, line_height: Pixels) -> Arc<Self> {
        let mut source = Self::new(None, None, mark, font_size, font_size, line_height);
        let root = Arc::get_mut(&mut source).expect("new inline box is uniquely owned");
        root.root = true;
        root.baseline_shift = Pixels::ZERO;
        source
    }

    /// Reuse an inherited strut with identical computed typography. Inline
    /// ancestors may carry fills or baseline shifts; only a paragraph root
    /// can be reused, so none of those inline effects enter the new block.
    pub fn root_reusing(mut ancestor: Option<&Arc<Self>>, mark: &TextMark,
        font_size: Pixels, line_height: Pixels) -> Arc<Self>
    {
        let weight = mark.font_weight.unwrap_or(if mark.bold { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL });
        let style = mark.resolved_font_style();
        while let Some(source) = ancestor {
            if source.root && source.family == mark.font_family && source.weight == weight
                && source.style == style && source.font_size == font_size && source.line_height == line_height {
                return source.clone();
            }
            ancestor = source.parent.as_ref();
        }
        Self::root(mark, font_size, line_height)
    }

    pub(super) fn paints_background(&self) -> bool { self.paints_background }

    pub(super) fn baseline_shift(&self) -> Pixels { self.baseline_shift }
    pub(super) fn viewport_dependent(&self) -> bool { self.viewport_dependent }

    /// Baseline descendants of the same aligned box may still shape together.
    pub(super) fn alignment_scope(&self) -> Option<*const Self> {
        if self.alignment != InlineVerticalAlign::Baseline || self.baseline_offset.is_some() { Some(self as *const Self) }
        else { self.parent.as_ref().and_then(|parent| parent.alignment_scope()) }
    }
}

/// Font geometry belongs to the existing font-generation / typography / flow
/// key. Background prepaint borrows the same numbers instead of resolving
/// fonts for each repaint or retaining a separate metric cache.
#[derive(Default, Debug)]
pub(super) struct InlineBoxMetrics {
    // Address keys identify sources retained by entries; never dereference them.
    index: HashMap<usize, usize>,
    entries: Vec<BoxMetric>,
    backgrounds: Vec<usize>,
    root: Option<usize>,
    anchors: Vec<(usize, usize)>,
}

#[derive(Debug)]
struct BoxMetric {
    source: Arc<InlineBox>,
    parent: Option<usize>,
    font_id: gpui::FontId,
    ascent: Pixels,
    descent: Pixels,
    x_height: Option<Pixels>,
    above: Pixels,
    below: Pixels,
    background: Option<usize>,
    shift: Pixels,
    // Nearest line-edge ancestor, including this box if it starts a group.
    group: Option<usize>,
}

impl InlineBoxMetrics {
    pub(super) fn register(&mut self, source: &Arc<InlineBox>, base: &Font, window: &Window) -> usize {
        let key = Arc::as_ptr(source).addr();
        if let Some(index) = self.index.get(&key) { return *index; }
        let parent = source.parent.as_ref().map(|parent| self.register(parent, base, window));
        let mut font = base.clone();
        if let Some(family) = &source.family { font.family = family.clone(); }
        font.weight = source.weight;
        font.style = source.style;
        let system = window.text_system();
        let font_id = system.resolve_font(&font);
        let ascent = system.ascent(font_id, source.font_size);
        // Font metrics keep descenders signed, unlike shaped-line metrics.
        let descent = -system.descent(font_id, source.font_size);
        let leading = (window.pixel_snap(source.line_height) - ascent - descent) / 2.;
        let inherited_shift = parent.map_or(Pixels::ZERO, |index| {
            let parent = &self.entries[index];
            parent.shift - parent.source.baseline_shift
        });
        let own_above = ascent + leading;
        let own_below = descent + leading;
        let adjustment = if !source.alignment.is_font_relative() { Pixels::ZERO } else {
            let parent = &mut self.entries[parent.expect("font-relative alignment requires parent typography")];
            match source.alignment {
                InlineVerticalAlign::Baseline | InlineVerticalAlign::Top | InlineVerticalAlign::Bottom => Pixels::ZERO,
                InlineVerticalAlign::Middle => {
                    let (font_id, font_size) = (parent.font_id, parent.source.font_size);
                    let x_height = *parent.x_height.get_or_insert_with(|| system.x_height(font_id, font_size));
                    (own_below - own_above) / 2. + x_height / 2.
                },
                InlineVerticalAlign::TextTop => parent.ascent - own_above,
                InlineVerticalAlign::TextBottom => own_below - parent.descent,
            }
        };
        // A line-edge subtree uses a local, initially zero baseline. Its
        // final translation is resolved only after every row item is known.
        let shift = if source.alignment.is_edge() { Pixels::ZERO }
            else { source.baseline_shift + inherited_shift + adjustment
                + source.baseline_offset.as_ref().map_or(Pixels::ZERO, |offset| offset.pixels(base, window)) };
        let index = self.entries.len();
        let group = if source.alignment.is_edge() { Some(index) }
            else { parent.and_then(|parent| self.entries[parent].group) };
        let background = source.color.map(|_| {
            let slot = self.backgrounds.len();
            self.backgrounds.push(index);
            slot
        });
        if source.root {
            assert!(self.root.is_none(), "one structured strut per paragraph");
            self.root = Some(index);
        }
        self.entries.push(BoxMetric { source: source.clone(), parent, font_id, ascent, descent, x_height: None,
            above: own_above,
            below: own_below, background, shift, group });
        self.index.insert(key, index);
        index
    }

    pub(super) fn register_anchor(&mut self, at: usize, source: &Arc<InlineBox>, base: &Font, window: &Window) {
        let index = self.register(source, base, window);
        self.anchors.push((at, index));
    }

    pub(super) fn seal(&mut self) { self.anchors.sort_unstable_by_key(|(at, _)| *at); }

    fn index(&self, source: &Arc<InlineBox>) -> usize {
        *self.index.get(&Arc::as_ptr(source).addr()).expect("inline box measured with flow")
    }

    pub(super) fn shift(&self, source: &Arc<InlineBox>) -> Pixels {
        self.entries[self.index(source)].shift
    }

    pub(super) fn adjustment(&self, source: &Arc<InlineBox>) -> Pixels {
        self.shift(source) - source.baseline_shift
    }

    pub(super) fn root_strut(&self) -> Option<(Pixels, Pixels)> {
        self.root.map(|index| (self.entries[index].above, self.entries[index].below))
    }

    pub(super) fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub(super) fn has_backgrounds(&self) -> bool { !self.backgrounds.is_empty() }
}

/// Immutable offsets owned by the retained flow. Aligned rows share the
/// primary-font projection; fallback-font and line-edge translations use
/// sparse overrides. Ordinary rows retain neither. The flow owns invalidation.
#[derive(Debug)]
pub(super) struct InlineBoxGeometry {
    metrics: Arc<InlineBoxMetrics>,
    shifts: Box<[(usize, Pixels)]>,
}

impl PartialEq for InlineBoxGeometry {
    fn eq(&self, other: &Self) -> bool { std::ptr::eq(self, other) }
}

impl InlineBoxGeometry {
    pub(super) fn shift(&self, source: &Arc<InlineBox>) -> Pixels {
        let index = self.metrics.index(source);
        self.shifts.binary_search_by_key(&index, |(index, _)| *index)
            .map_or(self.metrics.entries[index].shift, |position| self.shifts[position].1)
    }

    pub(super) fn adjustment(&self, source: &Arc<InlineBox>) -> Pixels {
        self.shift(source) - source.baseline_shift
    }
}

#[derive(Default)]
struct RowBox {
    stamp: usize,
    has_glyphs: bool,
    ascent: Pixels,
    descent: Pixels,
    shift: Pixels,
    above: Pixels,
    below: Pixels,
    subtree: Option<(Pixels, Pixels)>,
}

/// Only boxes active on a row participate. A box with no glyphs of its own
/// contributes its first-font strut even if it has smaller descendants.
/// Actual fallback fonts contribute their own half-leading at the leaf.
pub(super) struct InlineBoxRow {
    stamp: usize,
    boxes: Vec<RowBox>,
    active: Vec<usize>,
    primary: Option<Arc<InlineBoxGeometry>>,
    aligned: bool,
}

impl InlineBoxRow {
    pub(super) fn new(metrics: &InlineBoxMetrics) -> Self {
        Self { stamp: 0, boxes: (0..metrics.entries.len()).map(|_| RowBox::default()).collect(), active: Vec::new(), primary: None, aligned: false }
    }

    pub(super) fn begin(&mut self) { self.stamp += 1; self.active.clear(); }

    pub(super) fn activate(&mut self, source: &Arc<InlineBox>, metrics: &InlineBoxMetrics) {
        self.activate_index(metrics.index(source), metrics);
    }

    fn activate_index(&mut self, source: usize, metrics: &InlineBoxMetrics) {
        let mut index = Some(source);
        while let Some(current) = index {
            let state = &mut self.boxes[current];
            if state.stamp == self.stamp { break; }
            *state = RowBox { stamp: self.stamp, ..Default::default() };
            self.active.push(current);
            index = metrics.entries[current].parent;
        }
    }

    pub(super) fn anchors(&mut self, range: &std::ops::Range<usize>, soft_start: bool, metrics: &InlineBoxMetrics) {
        let first = metrics.anchors.partition_point(|(at, _)| *at < range.start || soft_start && *at == range.start);
        for (_, index) in metrics.anchors[first..].iter().take_while(|(at, _)| *at <= range.end) {
            self.activate_index(*index, metrics);
        }
    }

    pub(super) fn text(&mut self, highlights: &[(std::ops::Range<usize>, super::inline::InlineHighlight)],
        line: &gpui::ShapedLine, font_size: Pixels, metrics: &InlineBoxMetrics, window: &Window)
    {
        for (_, mark) in highlights {
            if let Some(source) = &mark.inline_box { self.activate(source, metrics); }
        }
        if line.runs.len() == 1 {
            let run = &line.runs[0];
            if !run.glyphs.is_empty() {
                for (range, mark) in highlights {
                    if !range.is_empty() && let Some(source) = &mark.inline_box {
                        self.font(source, run.font_id, font_size, metrics, window);
                    }
                }
            }
            return;
        }
        // Structured source marks are flattened, disjoint source intervals.
        // Native clusters retain source ranges through ligatures and bidi;
        // use those intervals instead of guessing ranges from visual order.
        let scopes = highlights.iter().filter(|(_, mark)| mark.inline_box.is_some()).collect::<Vec<_>>();
        let mut clusters = line.clusters().peekable();
        while let Some(cluster) = clusters.next() {
            let end = clusters.peek().map(|next| next.first_glyph);
            let first = scopes.partition_point(|(range, _)| range.end <= cluster.source.start);
            for (range, mark) in scopes[first..].iter().take_while(|(range, _)| range.start < cluster.source.end) {
                if range.is_empty() { continue; }
                let last = end.map_or(line.runs.len(), |end| end.run_ix + usize::from(end.glyph_ix > 0));
                for run in &line.runs[cluster.first_glyph.run_ix..last] {
                    if !run.glyphs.is_empty() {
                        self.font(mark.inline_box.as_ref().expect("scoped interval"), run.font_id, font_size, metrics, window);
                    }
                }
            }
        }
    }

    fn font(&mut self, source: &Arc<InlineBox>, font_id: gpui::FontId,
        font_size: Pixels, metrics: &InlineBoxMetrics, window: &Window)
    {
        let metric = &metrics.entries[metrics.index(source)];
        let state = &mut self.boxes[metrics.index(source)];
        let system = window.text_system();
        let (ascent, descent) = if font_id == metric.font_id && font_size == source.font_size {
            (metric.ascent, metric.descent)
        } else {
            (system.ascent(font_id, font_size), -system.descent(font_id, font_size))
        };
        let leading = (window.pixel_snap(source.line_height) - ascent - descent) / 2.;
        let above = ascent + leading;
        let below = descent + leading;
        if state.has_glyphs {
            state.above = state.above.max(above); state.below = state.below.max(below);
            state.ascent = state.ascent.max(ascent); state.descent = state.descent.max(descent);
        } else {
            state.has_glyphs = true; state.above = above; state.below = below;
            state.ascent = ascent; state.descent = descent;
        }
    }

    /// Resolve font-relative baselines first. Edge groups accumulate bounds
    /// without enlarging the normal baseline strut; their children may extend
    /// above or below the group's own font box.
    pub(super) fn resolve(&mut self, metrics: &InlineBoxMetrics, above: &mut Pixels, below: &mut Pixels) {
        self.aligned = self.active.iter().any(|index| metrics.entries[*index].source.alignment != InlineVerticalAlign::Baseline);
        if self.aligned { self.active.sort_unstable(); }
        for offset in 0..self.active.len() {
            let index = self.active[offset];
            let metric = &metrics.entries[index];
            let state = &self.boxes[index];
            let own_above = if state.has_glyphs { state.above } else { metric.above };
            let own_below = if state.has_glyphs { state.below } else { metric.below };
            let shift = if metric.source.alignment.is_edge() { Pixels::ZERO }
                else if !self.aligned { metric.shift } else {
                    let inherited = metric.parent.map_or(Pixels::ZERO, |parent| {
                        self.boxes[parent].shift - metrics.entries[parent].source.baseline_shift
                    });
                    let adjustment = if !metric.source.alignment.is_font_relative() { Pixels::ZERO } else {
                        let parent = metric.parent.expect("font-relative alignment has parent typography");
                        let parent_state = &self.boxes[parent];
                        let parent_metric = &metrics.entries[parent];
                        match metric.source.alignment {
                            InlineVerticalAlign::Middle => (own_below - own_above) / 2.
                                + parent_metric.x_height.expect("middle alignment measured parent x-height") / 2.,
                            InlineVerticalAlign::TextTop => (if parent_state.has_glyphs { parent_state.ascent } else { parent_metric.ascent }) - own_above,
                            InlineVerticalAlign::TextBottom => own_below - if parent_state.has_glyphs { parent_state.descent } else { parent_metric.descent },
                            _ => Pixels::ZERO,
                        }
                    };
                    metric.source.baseline_shift + inherited + adjustment
                };
            self.boxes[index].shift = shift;
            self.include(metric.group, own_above + shift, own_below - shift, above, below);
        }
    }

    fn include(&mut self, group: Option<usize>, own_above: Pixels, own_below: Pixels,
        above: &mut Pixels, below: &mut Pixels)
    {
        if let Some(group) = group {
            let bounds = &mut self.boxes[group].subtree;
            *bounds = Some(bounds.map_or((own_above, own_below), |(above, below)|
                (above.max(own_above), below.max(own_below))));
        } else {
            *above = (*above).max(own_above);
            *below = (*below).max(own_below);
        }
    }

    pub(super) fn shift(&self, source: &Arc<InlineBox>, metrics: &InlineBoxMetrics) -> Pixels {
        self.boxes[metrics.index(source)].shift
    }

    /// Atomic boxes enlarge their nearest aligned subtree, rather than the
    /// paragraph baseline. Independently edge-aligned images bypass this call.
    pub(super) fn atomic(&mut self, source: &Arc<InlineBox>, baseline: Pixels, height: Pixels,
        metrics: &InlineBoxMetrics, above: &mut Pixels, below: &mut Pixels)
    {
        self.include(metrics.entries[metrics.index(source)].group, baseline, height - baseline, above, below);
    }

    pub(super) fn finish(&mut self, metrics: &Arc<InlineBoxMetrics>, above: &mut Pixels,
        below: &mut Pixels, mut top_height: Pixels, mut bottom_height: Pixels) -> Option<Arc<InlineBoxGeometry>>
    {
        if !self.aligned {
            *above = (*above).max(bottom_height - *below);
            *below = (*below).max(top_height - *above);
            return None;
        }
        for index in &self.active {
            let metric = &metrics.entries[*index];
            if metric.source.alignment.is_edge() {
                let (group_above, group_below) = self.boxes[*index].subtree.expect("edge root contributes its own box");
                let height = group_above + group_below;
                if metric.source.alignment == InlineVerticalAlign::Top { top_height = top_height.max(height); }
                else { bottom_height = bottom_height.max(height); }
            }
        }
        // Preserve the existing deterministic baseline policy for atomic
        // edge boxes. Both constraints apply once to all groups, so source
        // order cannot change the minimum line height.
        *above = (*above).max(bottom_height - *below);
        *below = (*below).max(top_height - *above);
        let mut shifts = Vec::new();
        for index in &self.active {
            let metric = &metrics.entries[*index];
            if let Some(group) = metric.group {
                let (group_above, group_below) = self.boxes[group].subtree.expect("active edge group has bounds");
                let translation = match metrics.entries[group].source.alignment {
                    InlineVerticalAlign::Top => *above - group_above,
                    InlineVerticalAlign::Bottom => group_below - *below,
                    _ => unreachable!("only edge boxes start aligned groups"),
                };
                self.boxes[*index].shift += translation;
            }
            let shift = self.boxes[*index].shift;
            if shift != metric.shift { shifts.push((*index, shift)); }
        }
        if shifts.is_empty() {
            Some(self.primary.get_or_insert_with(|| Arc::new(InlineBoxGeometry {
                metrics: metrics.clone(), shifts: Box::default(),
            })).clone())
        } else {
            Some(Arc::new(InlineBoxGeometry { metrics: metrics.clone(), shifts: shifts.into_boxed_slice() }))
        }
    }
}

/// Only the current prepaint's visible rectangles. Slots and source order
/// come from the measured flow; no second identity/metric index is built.
pub(super) struct InlineBackgrounds {
    metrics: Arc<InlineBoxMetrics>,
    layers: Vec<Vec<Bounds<Pixels>>>,
}

impl InlineBackgrounds {
    pub(super) fn new(metrics: Arc<InlineBoxMetrics>) -> Self {
        let layers = (0..metrics.backgrounds.len()).map(|_| Vec::new()).collect();
        Self { metrics, layers }
    }

    pub(super) fn include(&mut self, source: &Arc<InlineBox>, left: Pixels,
        right: Pixels, baseline: Pixels, geometry: Option<&InlineBoxGeometry>, window: &Window)
    {
        if right <= left || !source.paints_background { return; }
        let clip = window.content_mask().bounds;
        let mut index = Some(self.metrics.index(source));
        while let Some(current) = index {
            let metric = &self.metrics.entries[current];
            if let Some(slot) = metric.background {
                let shift = geometry.map_or(metric.shift, |geometry| geometry.shift(&metric.source));
                let top = baseline - shift - metric.ascent;
                let height = metric.ascent + metric.descent;
                let rect = Bounds::new(point(left, top), size(right - left, height));
                if height > Pixels::ZERO && rect.left() < clip.right() && rect.right() > clip.left()
                    && rect.top() < clip.bottom() && rect.bottom() > clip.top()
                {
                    self.layers[slot].push(rect);
                }
            }
            index = metric.parent;
        }
    }

    pub(super) fn paint(&mut self, window: &mut Window) {
        for (slot, rects) in self.layers.iter_mut().enumerate() {
            let color = self.metrics.entries[self.metrics.backgrounds[slot]].source.color.expect("background slot has a fill");
            rects.sort_unstable_by(|a, b| a.top().partial_cmp(&b.top()).unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.left().partial_cmp(&b.left()).unwrap_or(std::cmp::Ordering::Equal)));
            let mut merged: Option<Bounds<Pixels>> = None;
            for rect in rects.iter().copied() {
                if let Some(previous) = merged.as_mut()
                    && previous.top() == rect.top() && previous.size.height == rect.size.height
                    && rect.left() <= previous.right()
                {
                    previous.size.width = previous.right().max(rect.right()) - previous.left();
                    continue;
                }
                if let Some(rect) = merged.replace(rect) { paint_box(rect, color, window); }
            }
            if let Some(rect) = merged { paint_box(rect, color, window); }
        }
    }
}

fn paint_box(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    window.paint_quad(quad(bounds, px(0.), color, Edges::default(), gpui::transparent_black(), gpui::BorderStyle::Solid));
}

#[derive(Clone)]
pub(super) struct BackgroundContext {
    pub(super) frame: Rc<RefCell<InlineBackgrounds>>,
    pub(super) baseline: Pixels,
}
