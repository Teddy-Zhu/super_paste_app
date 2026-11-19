use std::{cell::RefCell, ops::RangeInclusive, rc::Rc};

use crate::{
    GlobalState, TextSelectionContentKey, TextSelectionCoverage, TextSelectionEndpoint,
    TextSelectionEvent, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun,
    TextSelectionSnapshot,
};
use gpui::{
    AnyElement, App, Bounds, Element, ElementId, EntityId, GlobalElementId, Hitbox,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, WeakEntity, Window, point,
};

use super::{
    TextViewState, document::ParsedDocument, inline::InlineSelectionProjection,
    inline_object::InlineObjectSelectionProjection, node::{BlockNode, CustomBlockSelectionProjection, SourceSelectionPosition},
};

#[derive(Clone, Debug)]
struct CachedBlockEndpoint {
    endpoint: TextSelectionEndpoint,
    block_ix: Option<usize>,
    boundary: Option<BoundaryProjection>,
    source_position: Option<SourceSelectionPosition>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BoundaryProjection {
    from_start: bool,
    resolved: bool,
}

/// A native range can preserve an already projected boundary while its other
/// endpoint waits for a virtual block to acquire geometry.
#[derive(Clone, Copy, Default)]
pub(super) struct SourceProjection {
    pub(super) active: bool,
    pub(super) changed: bool,
    pub(super) complete: bool,
}

#[derive(Default)]
struct VirtualBlockSelection {
    anchor: Option<CachedBlockEndpoint>,
    cursor: Option<CachedBlockEndpoint>,
    coverage: TextSelectionCoverage,
    is_selecting: bool,
    source_projected: bool,
    /// One derived ordering for the current logical pair, invalidated with the
    /// projection. Do not walk an entire source block on every paint frame.
    source_order: Option<std::cmp::Ordering>,
    /// Borrow the authoritative tree for offscreen endpoint ordering; never
    /// keep a retired document alive through a word/handle registration.
    source_blocks: std::sync::Weak<Vec<BlockNode>>,
}

impl VirtualBlockSelection {
    fn update(&mut self, snapshot: Option<TextSelectionSnapshot>, entity_id: EntityId) -> bool {
        let Some(snapshot) = snapshot else {
            let changed = self.anchor.is_some() || self.cursor.is_some();
            *self = Self::default();
            return changed;
        };

        // Origin and scroll changes only alter the window projection. They must
        // not turn a held logical range back into a hit test in reflowed text.
        let logical_changed = self.anchor.as_ref().map(|cached| cached.endpoint) != Some(snapshot.anchor())
            || self.cursor.as_ref().map(|cached| cached.endpoint) != Some(snapshot.cursor())
            || self.coverage != snapshot.coverage();
        let changed = logical_changed || self.is_selecting != snapshot.is_selecting();
        if logical_changed {
            self.source_projected = false;
            self.source_order = None;
        }
        self.coverage = snapshot.coverage();
        self.is_selecting = snapshot.is_selecting();

        let old_cursor = self.cursor.as_ref()
            .filter(|cached| cached.matches_endpoint(snapshot.anchor(), entity_id)).cloned();
        let old_anchor = self.anchor.as_ref()
            .filter(|cached| cached.matches_endpoint(snapshot.cursor(), entity_id)).cloned();
        Self::update_endpoint(&mut self.anchor, snapshot.anchor(), entity_id, old_cursor);
        Self::update_endpoint(&mut self.cursor, snapshot.cursor(), entity_id, old_anchor);
        if snapshot.coverage() == TextSelectionCoverage::Full {
            self.invalidate_projection();
        }
        changed
    }

    fn update_endpoint(
        cached: &mut Option<CachedBlockEndpoint>,
        endpoint: TextSelectionEndpoint,
        entity_id: EntityId,
        other: Option<CachedBlockEndpoint>,
    ) {
        if let Some(cached) = cached.as_mut()
            && cached.matches_endpoint(endpoint, entity_id)
        {
            cached.endpoint = endpoint;
            return;
        }
        let block_ix = (endpoint.entity_id() == Some(entity_id))
            .then(|| endpoint.content_key().and_then(|key| usize::try_from(key.value()).ok()))
            .flatten();
        let other = other.filter(|other| other.matches_endpoint(endpoint, entity_id));
        let boundary = other.as_ref().and_then(|other| other.boundary);
        let source_position = other.and_then(|other| other.source_position);
        *cached = Some(CachedBlockEndpoint { endpoint, block_ix, boundary, source_position });
    }

    fn invalidate_projection(&mut self) {
        self.source_projected = false;
        self.source_order = None;
        for endpoint in [&mut self.anchor, &mut self.cursor].into_iter().flatten() {
            endpoint.boundary = None;
        }
    }

    fn source_ordering(&mut self, owner: EntityId) -> Option<std::cmp::Ordering> {
        if let Some(order) = self.source_order { return Some(order); }
        let (a, b) = (self.anchor.as_ref()?, self.cursor.as_ref()?);
        if a.endpoint.entity_id() != Some(owner) || b.endpoint.entity_id() != Some(owner) { return None; }
        let (a_block, b_block) = (a.block_ix?, b.block_ix?);
        let document = self.source_blocks.upgrade()?;
        let order = if a_block != b_block {
            document.get(a_block)?;
            document.get(b_block)?;
            a_block.cmp(&b_block)
        } else {
            document.get(a_block)?.source_position_ordering(a.source_position.as_ref()?, b.source_position.as_ref()?)?
        };
        self.source_order = Some(order);
        Some(order)
    }

    fn block_range(&self, entity_id: EntityId, last: usize) -> Option<RangeInclusive<usize>> {
        let anchor = self.anchor.as_ref()?;
        let cursor = self.cursor.as_ref()?;
        match self.coverage {
            TextSelectionCoverage::Full => Some(0..=last),
            TextSelectionCoverage::FromStart => Some(0..=anchor.block_ix.or(cursor.block_ix)?),
            TextSelectionCoverage::ToEnd => Some(anchor.block_ix.or(cursor.block_ix)?..=last),
            TextSelectionCoverage::Bounded => {
                if anchor.endpoint.entity_id() != Some(entity_id)
                    || cursor.endpoint.entity_id() != Some(entity_id)
                {
                    return None;
                }
                let (anchor, cursor) = (anchor.block_ix?, cursor.block_ix?);
                Some(anchor.min(cursor)..=anchor.max(cursor))
            }
        }
    }
}

/// TextView's renderer-specific bridge to base-owned window selection.
#[derive(Clone)]
pub(super) struct TextViewSelectionAdapter {
    selection: TextSelectionHandle,
    virtual_blocks: Rc<RefCell<VirtualBlockSelection>>,
    /// What the view's text painted this frame, shared by the adapter's
    /// clones. Inlines report it as they paint, through a read of the view's
    /// state: reporting it is not a change of the view.
    frame: Rc<RefCell<PaintedText>>,
    layout_revision: Option<usize>,
}

/// The text a [`TextViewSelectionAdapter`]'s view painted this frame.
#[derive(Default)]
struct PaintedText {
    text_bounds: Vec<Bounds<Pixels>>,
    text_runs: Vec<TextSelectionRun>,
    /// Each with the table cell it painted in, if any.
    copy_projections: Vec<(CopyProjection, Option<Bounds<Pixels>>, Option<usize>)>,
    /// Actual top-level layout boxes and their current visible intersections,
    /// including blocks without painted leaves. Cleared with the retained frame.
    source_blocks: Vec<(usize, Bounds<Pixels>, Bounds<Pixels>)>,
    /// Scoped by the top-level source block currently painting, not visual y.
    current_block: Option<SourceBlockSelection>,
    /// Coordinate origin of these retained layouts, including their own scroll.
    content_origin: Option<Point<Pixels>>,
    /// Skip object lookup on the common, text-only pointer path.
    has_inline_objects: bool,
    /// Current-frame caret boxes; real native endpoints override fragment cuts.
    selection_edges: PaintedSelectionEdges,
}

#[derive(Default)]
struct PaintedSelectionEdges {
    start: Option<Bounds<Pixels>>,
    end: Option<Bounds<Pixels>>,
    native_end: bool,
}

#[derive(Clone, Copy)]
struct SourceBlockSelection {
    ix: usize,
    coverage: Option<bool>,
    open: bool,
}

/// A weak view of the existing per-frame copy projections. Word source
/// snapshots must not retain the participant that owns their registration.
#[derive(Clone)]
pub(super) struct WordSelectionGeometry {
    frame: std::rc::Weak<RefCell<PaintedText>>,
    selection: std::rc::Weak<RefCell<VirtualBlockSelection>>,
}

impl WordSelectionGeometry {
    fn endpoint_ordering(
        &self,
        a: TextSelectionEndpoint,
        b: TextSelectionEndpoint,
    ) -> Option<std::cmp::Ordering> {
        let frame = self.frame.upgrade()?;
        let painted = frame.borrow();
        if let Some(selection) = self.selection.upgrade() {
            let selection = selection.borrow();
            if let Some(order) = selection.source_order
                && let (Some(anchor), Some(cursor)) = (&selection.anchor, &selection.cursor)
            {
                if (a, b) == (anchor.endpoint, cursor.endpoint) { return Some(order); }
                if (a, b) == (cursor.endpoint, anchor.endpoint) { return Some(order.reverse()); }
            }
            if let Some(document) = selection.source_blocks.upgrade() {
                if let (Some(a), Some(b)) = (a.content_key(), b.content_key())
                    && a != b
                {
                    document.get(usize::try_from(a.value()).ok()?)?;
                    document.get(usize::try_from(b.value()).ok()?)?;
                    return Some(a.value().cmp(&b.value()));
                }
                let cached = |point| {
                    [&selection.anchor, &selection.cursor].into_iter().flatten()
                        .find(|cached| cached.endpoint == point)
                        .and_then(|cached| cached.source_position.as_ref())
                };
                let resolve = |point: TextSelectionEndpoint| {
                    let retained = cached(point).cloned();
                    let point = painted.with_native_source_key(point)?;
                    let block = usize::try_from(point.content_key()?.value()).ok()?;
                    let source = retained.or_else(|| {
                        if point.source_boundary().is_some() && selection.coverage != TextSelectionCoverage::Full {
                            return None;
                        }
                        painted.source_endpoint(point, &document)
                    })?;
                    Some((block, source))
                };
                if let (Some((a_block, a_source)), Some((b_block, b_source))) = (resolve(a), resolve(b)) {
                    // A moving native endpoint may acquire its key only after
                    // the window lease ends. Resolve an actual painted source
                    // now without calling the application key resolver here.
                    return if a_block != b_block { Some(a_block.cmp(&b_block)) }
                    else { document.get(a_block)?.source_position_ordering(&a_source, &b_source) };
                }
                // A known logical endpoint must not fall back to a hit test
                // against an old coordinate when the other end is unresolved.
                if cached(a).is_some() || cached(b).is_some()
                    || a.source_boundary().is_some() || b.source_boundary().is_some()
                { return None; }
            }
        }
        painted.endpoint_ordering(a, b)
    }

    fn atomic_unit_at(
        &self,
        position: Point<Pixels>,
        click_count: usize,
    ) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
        let frame = self.frame.upgrade()?;
        let painted = frame.borrow();
        if !painted.has_inline_objects {
            return None;
        }
        let carets = painted.copy_projections.iter().find_map(|(projection, _, _)| {
            match projection {
                CopyProjection::InlineObject(object) => object.unit_at(position, click_count),
                _ => None,
            }
        });
        carets
    }

    pub(super) fn object_caret(
        &self,
        source: &dyn crate::text_selection::WordSelectionSource,
        segment: usize,
        offset: usize,
    ) -> Option<Bounds<Pixels>> {
        let frame = self.frame.upgrade()?;
        let painted = frame.borrow();
        if !painted.has_inline_objects {
            return None;
        }
        let caret = painted.copy_projections.iter().find_map(|(projection, _, _)| {
            match projection {
                CopyProjection::InlineObject(object) => object.word_caret(source, segment, offset),
                _ => None,
            }
        });
        caret
    }
}

pub(super) enum CopyProjection {
    Text(InlineSelectionProjection),
    InlineObject(InlineObjectSelectionProjection),
    CustomBlock(CustomBlockSelectionProjection),
}

impl CopyProjection {
    fn bounds(&self) -> Bounds<Pixels> {
        match self {
            Self::Text(projection) => projection.bounds(),
            Self::InlineObject(projection) => projection.bounds(),
            Self::CustomBlock(projection) => projection.bounds(),
        }
    }

    fn is_visible_in(&self, visible: Bounds<Pixels>) -> bool {
        let bounds = self.bounds();
        let clipped = bounds.intersect(&visible);
        clipped.size.height > Pixels::ZERO
            && (clipped.size.width > Pixels::ZERO
                || matches!(self, Self::Text(_)) && bounds.size.width == Pixels::ZERO
                    && bounds.left() >= visible.left() && bounds.left() <= visible.right())
    }

    fn select_whole(&self, selected: bool) {
        match self {
            Self::Text(projection) if selected => projection.select_whole(),
            Self::Text(_) => {}, // All shared source states were reset first.
            Self::InlineObject(projection) => projection.select_whole(selected),
            Self::CustomBlock(projection) if selected => projection.select_whole(true),
            // Empty observations are endpoint markers for the source copier.
            // An excluded interior fragment must not create a false boundary.
            Self::CustomBlock(projection) => projection.deselect(),
        }
    }

    /// Apply source byte boundaries after all shared text states were reset.
    /// Custom blocks can occur only in the interior: endpoint resolution never
    /// invents insertion points for them.
    fn select_slice(&self, start: Option<usize>, end: Option<usize>) {
        match self {
            Self::Text(text) => match (start, end) {
                (Some(start), Some(end)) => text.select_range(start..end),
                (Some(start), None) => text.select_to_offset(start, false),
                (None, Some(end)) => text.select_to_offset(end, true),
                (None, None) => text.select_whole(),
            },
            Self::InlineObject(object) => {
                object.select_whole(start.unwrap_or(0) < end.unwrap_or("\u{fffc}".len()));
            },
            Self::CustomBlock(block) => block.select_whole(true),
        }
    }

    fn reset(&self) {
        if let Self::Text(projection) = self {
            projection.reset();
        }
    }

    fn project(&self, start: Point<Pixels>, end: Point<Pixels>) {
        match self {
            Self::Text(projection) => projection.project(start, end),
            Self::InlineObject(projection) => projection.project(start, end),
            Self::CustomBlock(projection) => projection.project(start, end),
        }
    }

    /// Selects nothing: the selection started in a table cell this is not in.
    fn deselect(&self) {
        match self {
            Self::Text(_) => {}, // Do not erase a preceding shared fragment.
            Self::InlineObject(projection) => projection.deselect(),
            Self::CustomBlock(projection) => projection.deselect(),
        }
    }
}

impl PaintedText {
    fn with_native_source_key(&self, endpoint: TextSelectionEndpoint) -> Option<TextSelectionEndpoint> {
        if endpoint.content_key().is_some() { return Some(endpoint); }
        if endpoint.source_boundary().is_some() { return None; }
        let position = endpoint.content_point() + self.content_origin?;
        // A visible block also owns the gaps between its native rows. Resolve
        // that ownership before the deferred callback, without assigning an
        // offscreen point to whichever fragment happens to be painted.
        let block = self.native_block_at(position).or_else(|| self.visible_block_at(position))?;
        Some(endpoint.with_content_key(TextSelectionContentKey::new(u64::try_from(block).ok()?)))
    }

    fn source_caret(&self, block: usize, position: &SourceSelectionPosition) -> Option<Bounds<Pixels>> {
        self.copy_projections.iter()
            .filter(|(_, _, owner)| *owner == Some(block))
            .find_map(|(projection, _, _)| match (projection, position) {
                (CopyProjection::Text(text), SourceSelectionPosition::Text(position)) => text.source_caret(position),
                (CopyProjection::InlineObject(object), SourceSelectionPosition::Atomic(position)) => object.source_caret(position),
                // Plugin interiors do not supply native text caret metrics.
                _ => None,
            })
    }

    fn source_endpoint(&self, endpoint: TextSelectionEndpoint, document: &[BlockNode]) -> Option<SourceSelectionPosition> {
        let block = usize::try_from(endpoint.content_key()?.value()).ok()?;
        if let Some(edge) = endpoint.source_boundary() {
            return document.get(block)?.source_boundary(edge == crate::SelectionEdge::Start);
        }
        let position = endpoint.content_point() + self.content_origin?;
        // Real insertion geometry wins, including content overflowing its
        // block. Only an unresolved outside point uses the block's source edge.
        let projected = self.source_projection_at(position, Some(block), None).and_then(|(_, projection, _)| match projection {
            CopyProjection::Text(text) => {
                if let Some(position) = text.source_position(position) {
                    return Some(SourceSelectionPosition::Text(position));
                }
                let source = SourceSelectionPosition::Text(text.whole_source_boundary(position)?);
                document.get(block).is_some_and(|block| block.is_single_source_position(&source))
                    .then_some(source)
            },
            CopyProjection::InlineObject(object) => object.source_position(position).map(SourceSelectionPosition::Atomic),
            CopyProjection::CustomBlock(custom) => {
                let source = SourceSelectionPosition::Block(custom.source_position(position)?);
                let bounds = custom.bounds();
                // Padding can resolve outside the box only when this is the
                // root's sole source. Otherwise an unpainted source may own it.
                (position.y >= bounds.top() && position.y <= bounds.bottom()
                    || document.get(block).is_some_and(|block| block.is_single_source_position(&source)))
                    .then_some(source)
            },
        });
        projected.or_else(|| {
            let (_, bounds, visible) = self.source_blocks.iter().find(|(ix, _, _)| *ix == block)?;
            if bounds.size.height <= Pixels::ZERO { return None; }
            let from_start = if position.y < bounds.top() { true }
                else if position.y >= bounds.bottom() { false }
                else {
                    // A pointer in visible padding belongs to the closest
                    // visible native row of this same source block. Clamp only
                    // the hit query; keep the endpoint's document coordinate
                    // and capture the resulting canonical source position.
                    if !visible.contains(&position) { return None; }
                    let (_, projection, _) = self.source_projection_at(position, Some(block), Some(*visible))?;
                    let row = projection.bounds();
                    let hit = point(position.x.max(row.left()).min(row.right()), position.y.max(row.top()).min(row.bottom()));
                    return match projection {
                        CopyProjection::Text(text) => text.source_position(hit).map(SourceSelectionPosition::Text),
                        CopyProjection::InlineObject(object) => object.source_position(hit).map(SourceSelectionPosition::Atomic),
                        CopyProjection::CustomBlock(_) => None,
                    };
                };
            document.get(block)?.source_boundary(from_start)
        })
    }

    fn visible_block_at(&self, position: Point<Pixels>) -> Option<usize> {
        self.source_blocks.iter().find_map(|(ix, _, visible)| {
            (visible.size.width > Pixels::ZERO && visible.size.height > Pixels::ZERO
                && visible.contains(&position)).then_some(*ix)
        })
    }

    fn native_block_at(&self, position: Point<Pixels>) -> Option<usize> {
        let (_, projection, block) = self.source_projection_at(position, None, None)?;
        let bounds = projection.bounds();
        // Overflowing native content keeps its own source ownership.
        (position.x >= bounds.left() && position.x <= bounds.right()
            && position.y >= bounds.top() && position.y <= bounds.bottom()).then_some(block)
    }

    /// The projections are already in source order, independently of bidi
    /// visual placement. Resolve only within a known endpoint block when given.
    fn source_projection_at(
        &self,
        position: Point<Pixels>,
        block: Option<usize>,
        visible: Option<Bounds<Pixels>>,
    ) -> Option<(usize, &CopyProjection, usize)> {
        self.copy_projections.iter().enumerate()
            .filter(|(_, (projection, _, ix))| {
                let bounds = projection.bounds();
                ix.is_some() && (block.is_none() || *ix == block)
                    && bounds.size.height > Pixels::ZERO
                    && (matches!(projection, CopyProjection::Text(_)) || bounds.size.width > Pixels::ZERO)
                    && visible.is_none_or(|visible| projection.is_visible_in(visible))
            })
            .min_by(|(_, (a, _, _)), (_, (b, _, _))| {
                crate::text_selection::distance_to_bounds(position, a.bounds())
                    .total_cmp(&crate::text_selection::distance_to_bounds(position, b.bounds()))
            })
            .and_then(|(ix, (projection, _, block))| Some((ix, projection, (*block)?)))
    }

    fn source_position(&self, endpoint: TextSelectionEndpoint) -> Option<(usize, usize, usize)> {
        let position = endpoint.content_point() + self.content_origin?;
        let block = match endpoint.content_key() {
            Some(key) => Some(usize::try_from(key.value()).ok()?),
            None => None,
        };
        let (ix, projection, block) = self.source_projection_at(position, block, None)?;
        let offset = match projection {
            CopyProjection::Text(text) => text.endpoint_offset(position)?,
            CopyProjection::InlineObject(object) => object.endpoint_offset(position),
            // A custom block has no native insertion points. Do not invent
            // one by hitting a nearby text run or estimating half its height.
            CopyProjection::CustomBlock(_) => return None,
        };
        Some((block, ix, offset))
    }

    fn endpoint_ordering(
        &self,
        a: TextSelectionEndpoint,
        b: TextSelectionEndpoint,
    ) -> Option<std::cmp::Ordering> {
        // No comparison is valid while the retained frame is invalidated.
        self.content_origin?;
        if let (Some(a), Some(b)) = (a.content_key(), b.content_key())
            && a != b
        {
            // Only this adapter interprets its keys as top-level source blocks.
            // Different known blocks need no hit test, even when offscreen.
            return Some(a.value().cmp(&b.value()));
        }
        Some(self.source_position(a)?.cmp(&self.source_position(b)?))
    }

    fn block_at(&self, content_point: Point<Pixels>) -> Option<usize> {
        let position = content_point + self.content_origin?;
        self.native_block_at(position).or_else(|| self.visible_block_at(position))
            .or_else(|| self.source_projection_at(position, None, None).map(|(_, _, block)| block))
    }

    fn project_bounded(&self, snapshot: TextSelectionSnapshot, owner: EntityId) -> bool {
        if snapshot.coverage() != TextSelectionCoverage::Bounded
            || snapshot.anchor().entity_id() != Some(owner)
            || snapshot.cursor().entity_id() != Some(owner)
        {
            return false;
        }
        // Resolve both native endpoints before mutating any source range.
        // Missing virtual geometry and custom atomic endpoints retain their
        // existing selection policy rather than half-applying a new range.
        let (Some(mut start), Some(mut end)) = (
            self.source_position(snapshot.anchor()),
            self.source_position(snapshot.cursor()),
        ) else { return false; };
        if end < start {
            std::mem::swap(&mut start, &mut end);
        }
        let (first, last) = ((start.0, start.1), (end.0, end.1));
        for (projection, _, _) in &self.copy_projections {
            projection.reset();
        }
        for (ix, (projection, _, block)) in self.copy_projections.iter().enumerate() {
            let Some(block) = block else { continue; };
            let key = (*block, ix);
            if key < first || key > last {
                projection.select_whole(false);
            } else {
                projection.select_slice(
                    (key == first).then_some(start.2),
                    (key == last).then_some(end.2),
                );
            }
        }
        true
    }

    fn project_complete_blocks(&self, covered: impl Fn(usize) -> Option<bool>) {
        // Boundary blocks already own their exact source ranges. Reset other
        // shared text owners before merging any of their represented fragments.
        for (projection, _, block) in &self.copy_projections {
            if block.is_some_and(|block| covered(block).is_some()) {
                projection.reset();
            }
        }
        for (projection, _, block) in &self.copy_projections {
            if let Some(selected) = block.and_then(&covered) {
                projection.select_whole(selected);
            }
        }
    }
}

impl CachedBlockEndpoint {
    fn matches_endpoint(&self, endpoint: TextSelectionEndpoint, owner: EntityId) -> bool {
        self.endpoint == endpoint || endpoint.source_boundary().is_some()
            && self.endpoint.source_boundary() == endpoint.source_boundary()
            && self.endpoint.entity_id() == Some(owner) && endpoint.entity_id() == Some(owner)
            && self.block_ix.is_some_and(|block| endpoint.content_key()
                .is_some_and(|key| u64::try_from(block).ok() == Some(key.value())))
    }

    fn project_boundary(
        &mut self,
        from_start: bool,
        document: &ParsedDocument,
    ) -> Option<(bool, bool)> {
        let block = self.block_ix?;
        let source = document.blocks.get(block)?;
        let mut changed = false;
        let changed_role = self.boundary.is_none_or(|boundary| boundary.from_start != from_start);
        if changed_role {
            // A new role cannot borrow its previous prefix/suffix. Clear once,
            // or let the validated source projection replace it directly.
            self.boundary = Some(BoundaryProjection { from_start, resolved: false });
            changed = true;
        }
        let boundary = self.boundary.as_mut()?;
        let projected = !boundary.resolved &&
            self.source_position.as_ref().is_some_and(|position| {
                let (start, end) = if from_start { (None, Some(position)) } else { (Some(position), None) };
                source.project_source_selection(start, end)
            });
        if projected {
            boundary.resolved = true;
            changed = true;
        } else if changed_role {
            // Keep unresolved endpoints pending instead of substituting a
            // different painted fragment from the same root.
            source.clear_selection();
        }
        Some((boundary.resolved, changed))
    }
}

/// Adds source ownership to the existing child layout without another box.
pub(super) struct SelectionBlockElement(pub(super) AnyElement, pub(super) usize);

impl IntoElement for SelectionBlockElement {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for SelectionBlockElement {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }
    fn request_layout(
        &mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        window: &mut Window, cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.0.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        _: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App,
    ) {
        self.0.prepaint(window, cx);
    }
    fn paint(
        &mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App,
    ) {
        let adapter = GlobalState::global(cx).text_view_state().and_then(|view| {
            let state = view.read(cx);
            // Nonselectable cards do not participate in selection recording.
            state.is_selectable().then(|| state.selection_adapter.clone())
        });
        let scope = adapter.as_ref().map(|adapter| {
            let scope = adapter.selection_for_block(self.1, cx);
            let mut frame = adapter.frame.borrow_mut();
            let block_index = frame.source_blocks.len();
            let first_projection = frame.copy_projections.len();
            frame.source_blocks.push((self.1, bounds, bounds.intersect(&window.content_mask().bounds)));
            (frame.current_block.replace(scope), block_index, first_projection)
        });
        self.0.paint(window, cx);
        if let (Some(adapter), Some((previous, block_index, first_projection))) = (adapter, scope) {
            let mut frame = adapter.frame.borrow_mut();
            let visible = frame.source_blocks[block_index].2;
            let has_native = frame.copy_projections[first_projection..].iter().any(|(projection, _, block)| {
                *block == Some(self.1)
                    && matches!(projection, CopyProjection::Text(_) | CopyProjection::InlineObject(_))
                    && projection.is_visible_in(visible)
            });
            if has_native {
                // A drag can start and finish in gaps without any sampled
                // pointer position crossing a glyph's rectangle.
                frame.text_bounds.push(visible);
            } else {
                // A box alone must not make unpainted/opaque content a native
                // hit target. Keep its actual bounds for outer source edges.
                frame.source_blocks[block_index].2 = Bounds::default();
            }
            frame.current_block = previous;
        }
    }
}

impl TextViewSelectionAdapter {
    pub(super) fn new(view: WeakEntity<TextViewState>, cx: &mut App) -> Self {
        let selection = TextSelectionHandle::new("", cx);
        let selection_id = selection.entity_id();
        let virtual_blocks = Rc::new(RefCell::new(VirtualBlockSelection::default()));
        let frame = Rc::new(RefCell::new(PaintedText::default()));
        let word_geometry = WordSelectionGeometry {
            frame: Rc::downgrade(&frame),
            selection: Rc::downgrade(&virtual_blocks),
        };
        let boundary_blocks = Rc::downgrade(&virtual_blocks);
        selection.resolve_source_boundaries_with(move || {
            let blocks = boundary_blocks.upgrade()?;
            let blocks = blocks.borrow();
            let document = blocks.source_blocks.upgrade()?;
            let first = document.iter().position(|block| block.source_boundary(true).is_some())?;
            let last = document.iter().rposition(|block| block.source_boundary(false).is_some())?;
            Some((
                TextSelectionContentKey::new(u64::try_from(first).ok()?),
                TextSelectionContentKey::new(u64::try_from(last).ok()?),
            ))
        }, cx);
        let ordering_geometry = word_geometry.clone();
        selection.resolve_endpoint_ordering_with(
            move |a, b| ordering_geometry.endpoint_ordering(a, b),
            cx,
        );
        selection.resolve_atomic_unit_with(
            move |position, click_count| word_geometry.atomic_unit_at(position, click_count),
            cx,
        );

        let view_for_events = view.clone();
        let blocks_for_events = virtual_blocks.clone();
        selection
            .subscribe(
                move |event, cx| match event {
                    TextSelectionEvent::SelectionChanged(snapshot) => {
                        let snapshot = *snapshot;
                        let _ = view_for_events.update(cx, |state, cx| {
                            let changed = blocks_for_events
                                .borrow_mut()
                                .update(snapshot, selection_id);
                            if snapshot.is_none() || changed {
                                state.preserve_inline_selection = false;
                            }
                            state.is_selecting =
                                snapshot.is_some_and(|snapshot| snapshot.is_selecting());
                            if state.is_selecting {
                                state.select_all = false;
                                state.end_multi_click_selection(cx);
                            }
                            if snapshot.is_some_and(|snapshot| snapshot.coverage() == TextSelectionCoverage::Full
                                && snapshot.anchor().source_boundary().is_some())
                            {
                                state.selection_adapter.refresh_source_selection(&state.parsed_content.document, cx);
                            }
                            cx.notify();
                        });
                    }
                    TextSelectionEvent::AutoScroll(delta) => {
                        let delta = *delta;
                        let _ = view_for_events.update(cx, |state, cx| {
                            if state.scrollable {
                                state.set_auto_scroll(delta, cx);
                            } else if delta.is_none() {
                                state.stop_auto_scroll();
                            }
                        });
                    }
                    TextSelectionEvent::Cleared => {}
                    TextSelectionEvent::TouchSelectionChanged => {
                        let _ = view_for_events.update(cx, |_, cx| cx.notify());
                    }
                },
                cx,
            )
            .detach();

        let view_for_clear = view.clone();
        let blocks_for_clear = virtual_blocks.clone();
        selection.clear_with(
            move |cx| {
                blocks_for_clear.replace(VirtualBlockSelection::default());
                let _ = view_for_clear.update(cx, |state, cx| {
                    state.reset_selection();
                    cx.notify();
                });
            },
            cx,
        );

        let view_for_copy = view.clone();
        let blocks_for_copy = virtual_blocks.clone();
        let frame_for_copy = frame.clone();
        selection.copy_with(
            move |cx| {
                let Some(view) = view_for_copy.upgrade() else {
                    return String::new();
                };
                let state = view.read(cx);
                if !state.has_current_copy_document() {
                    return String::new();
                }
                // Read the handle itself: event subscribers may not have run
                // yet when input and Copy arrive in the same App update.
                let snapshot = state.selection_adapter.selection.snapshot(cx);
                let view_selection = state.has_view_selection()
                    && state.selection_adapter.selection.has_local_selection(cx);
                let mut current = VirtualBlockSelection::default();
                current.update(snapshot, selection_id);
                let anchor_cell = state.selection_adapter.selection.table_cell_selection(cx);
                if anchor_cell.is_none() && current.coverage == TextSelectionCoverage::Full {
                    return state.all_text_for_copy();
                }
                let previous = blocks_for_copy.borrow();
                let range_changed = current.anchor.as_ref().map(|end| end.endpoint) != previous.anchor.as_ref().map(|end| end.endpoint)
                    || current.cursor.as_ref().map(|end| end.endpoint) != previous.cursor.as_ref().map(|end| end.endpoint)
                    || current.coverage != previous.coverage;
                drop(previous);
                // Gesture snapshots are current before paint. Refresh the rich
                // document's byte ranges from its retained native layouts
                // before its existing Plain/Source serializer reads them.
                let native_range = !view_selection
                    && anchor_cell.is_none()
                    && state.selection_adapter.refresh_source_selection(&state.parsed_content.document, cx).active;
                if !view_selection {
                    let open = anchor_cell.is_none()
                        && matches!(
                            current.coverage,
                            TextSelectionCoverage::FromStart | TextSelectionCoverage::ToEnd
                        );
                    if !native_range && !open
                        && (!state.preserve_inline_selection || range_changed)
                        && let Some((start, end)) = state.selection_points(cx)
                    {
                        // Only represented source states are refreshed.
                        // Offscreen AST selections retain their logical ends.
                        let frame = frame_for_copy.borrow();
                        for (projection, _, _) in &frame.copy_projections {
                            projection.reset();
                        }
                        for (projection, cell, _) in &frame.copy_projections {
                            match crate::global_state::confine_to_cell(
                                anchor_cell.map(|bounds| (start, bounds)),
                                *cell,
                                start,
                                end,
                            ) {
                                Some((start, end)) => projection.project(start, end),
                                None => projection.deselect(),
                            }
                        }
                    }
                }
                let last = state.parsed_content.document.blocks.len().saturating_sub(1);
                let blocks = if anchor_cell.is_some() {
                    let Some(anchor) = current
                        .anchor
                        .as_ref()
                        .filter(|anchor| anchor.endpoint.entity_id() == Some(selection_id))
                    else {
                        return String::new();
                    };
                    let Some(ix) = anchor.block_ix else {
                        return String::new();
                    };
                    Some(ix..=ix)
                } else {
                    current.block_range(selection_id, last)
                };
                let coverage = if anchor_cell.is_some() {
                    TextSelectionCoverage::Bounded
                } else {
                    current.coverage
                };
                if view_selection {
                    state.selected_text_with_coverage(blocks, coverage, native_range)
                } else {
                    state.selected_window_text_with_coverage(blocks, coverage, native_range)
                }
            },
            cx,
        );

        let view_for_content_key = view.clone();
        let frame_for_content_key = frame.clone();
        selection.resolve_content_key_with(
            move |point, cx| {
                let view = view_for_content_key.upgrade()?;
                view.read(cx).block_ix_at(point.y)
                    .or_else(|| frame_for_content_key.borrow().block_at(point))
                    .map(|block| TextSelectionContentKey::new(block as u64))
            },
            cx,
        );

        selection.focus_with(
            move |window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                let focus_handle = view.read(cx).focus_handle.clone();
                view.update(cx, |state, _| state.is_selecting = true);
                focus_handle.focus(window, cx);
            },
            cx,
        );

        Self {
            selection,
            virtual_blocks,
            frame,
            layout_revision: None,
        }
    }

    pub(super) fn update_layout_revision(&mut self, revision: usize, is_selecting: bool) -> bool {
        let changed = self
            .layout_revision
            .is_some_and(|previous| previous != revision);
        if !changed || !is_selecting {
            self.layout_revision = Some(revision);
        }
        changed && !is_selecting
    }

    /// Invalidate window ownership as well as this view's retained geometry.
    /// The caller is responsible for resetting its own document selection.
    pub(super) fn retire_content(&self, cx: &mut App) {
        self.selection.retire_content(cx);
        self.virtual_blocks.replace(VirtualBlockSelection::default());
        self.begin_frame();
        self.frame.borrow_mut().content_origin = None;
    }

    /// A compatible parse keeps the gesture, but neither a replaced source
    /// owner nor its previous layout can resolve endpoints in the new tree.
    pub(super) fn document_changed(&self, document: &ParsedDocument) {
        self.begin_frame();
        let mut blocks = self.virtual_blocks.borrow_mut();
        blocks.source_blocks = std::sync::Arc::downgrade(&document.blocks);
        blocks.invalidate_projection();
        let blocks = &mut *blocks;
        for endpoint in [&mut blocks.anchor, &mut blocks.cursor].into_iter().flatten() {
            if blocks.coverage == TextSelectionCoverage::Full
                && endpoint.endpoint.entity_id() == Some(self.selection.entity_id())
                && let Some(edge) = endpoint.endpoint.source_boundary()
            {
                endpoint.block_ix = if edge == crate::SelectionEdge::Start {
                    document.blocks.iter().position(|block| block.source_boundary(true).is_some())
                } else {
                    document.blocks.iter().rposition(|block| block.source_boundary(false).is_some())
                };
                endpoint.source_position = endpoint.block_ix.and_then(|ix| document.blocks.get(ix))
                    .and_then(|block| block.source_boundary(edge == crate::SelectionEdge::Start));
                continue;
            }
            if let Some(position) = &endpoint.source_position
                && !endpoint.block_ix.and_then(|ix| document.blocks.get(ix))
                    .is_some_and(|block| block.contains_source_position(position))
            {
                endpoint.source_position = None;
            }
        }
    }

    pub(super) fn word_selection_geometry(&self) -> WordSelectionGeometry {
        WordSelectionGeometry {
            frame: Rc::downgrade(&self.frame),
            selection: Rc::downgrade(&self.virtual_blocks),
        }
    }

    /// Registers `projection` as it paints, with the table cell it is in.
    pub(super) fn register_copy_projection(&self, projection: CopyProjection, cx: &App) {
        let cell = GlobalState::global(cx).table_cell();
        let mut frame = self.frame.borrow_mut();
        frame.has_inline_objects |= matches!(&projection, CopyProjection::InlineObject(_));
        let block = frame.current_block.map(|block| block.ix);
        frame.copy_projections.push((projection, cell, block));
    }

    pub(super) fn begin_frame(&self) {
        let mut frame = self.frame.borrow_mut();
        frame.text_bounds.clear();
        frame.text_runs.clear();
        frame.copy_projections.clear();
        frame.source_blocks.clear();
        frame.current_block = None;
        frame.has_inline_objects = false;
        frame.selection_edges = PaintedSelectionEdges::default();
    }

    /// Records one inline's painted selection ends. Inlines paint in document
    /// order, so the first start and the last end are the view's.
    pub(super) fn register_selection_edges(&self, start: Bounds<Pixels>, end: Bounds<Pixels>) {
        let mut frame = self.frame.borrow_mut();
        let edges = &mut frame.selection_edges;
        edges.start.get_or_insert(start);
        if !edges.native_end { edges.end = Some(end); }
    }

    pub(super) fn register_native_selection_edges(&self, start: Option<Bounds<Pixels>>, end: Option<Bounds<Pixels>>) {
        let mut frame = self.frame.borrow_mut();
        let edges = &mut frame.selection_edges;
        if let Some(start) = start { edges.start = Some(start); }
        if let Some(end) = end { edges.end = Some(end); edges.native_end = true; }
    }

    /// For ordinary source ranges, only their actual endpoint sources may
    /// report handles. An unpainted source is explicitly absent, not a request
    /// to substitute the first/last currently visible selected fragment.
    fn source_selection_edges(&self, frame: &PaintedText, cx: &App) -> Option<(Option<Bounds<Pixels>>, Option<Bounds<Pixels>>)> {
        if self.selection.table_cell_selection(cx).is_some() { return None; }
        let snapshot = self.selection.snapshot(cx)?;
        let owner = self.selection.entity_id();
        let mut blocks = self.virtual_blocks.borrow_mut();
        let caret = |endpoint: Option<&CachedBlockEndpoint>| {
            let endpoint = endpoint.filter(|endpoint| endpoint.endpoint.entity_id() == Some(owner))?;
            frame.source_caret(endpoint.block_ix?, endpoint.source_position.as_ref()?)
        };
        match snapshot.coverage() {
            TextSelectionCoverage::Bounded => {
                let mut endpoints = || {
                    let order = blocks.source_ordering(owner)?;
                    let (a, b) = (blocks.anchor.as_ref()?, blocks.cursor.as_ref()?);
                    Some(match order {
                        std::cmp::Ordering::Less => (caret(Some(a)), caret(Some(b))),
                        std::cmp::Ordering::Greater => (caret(Some(b)), caret(Some(a))),
                        std::cmp::Ordering::Equal => (None, None),
                    })
                };
                Some(endpoints().unwrap_or((None, None)))
            },
            TextSelectionCoverage::ToEnd | TextSelectionCoverage::FromStart => {
                let local = [&blocks.anchor, &blocks.cursor].into_iter().flatten()
                    .find(|endpoint| endpoint.endpoint.entity_id() == Some(owner));
                Some(if snapshot.coverage() == TextSelectionCoverage::ToEnd {
                    (caret(local), frame.selection_edges.end)
                } else {
                    (frame.selection_edges.start, caret(local))
                })
            },
            TextSelectionCoverage::Full => {
                // Whole-participant coverage in a cross-view range has no own
                // logical endpoints. Explicit touch Select All does, and must
                // hide an unpainted real edge rather than use a visible cut.
                let (a, b) = (blocks.anchor.as_ref()?, blocks.cursor.as_ref()?);
                (a.endpoint.source_boundary().is_some() && b.endpoint.source_boundary().is_some()
                    && a.endpoint.entity_id() == Some(owner) && b.endpoint.entity_id() == Some(owner))
                    .then(|| (caret(Some(a)), caret(Some(b))))
            },
        }
    }

    pub(super) fn register_text_run(&self, run: TextSelectionRun) {
        self.frame.borrow_mut().text_runs.push(run);
    }

    pub(super) fn register_inline(&self, bounds: Vec<Bounds<Pixels>>) {
        self.frame.borrow_mut().text_bounds.extend(bounds);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn register(
        &self,
        hitbox: Hitbox,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        self_scroll: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let boundary_keys = (|| {
            let blocks = self.virtual_blocks.borrow();
            if blocks.coverage != TextSelectionCoverage::Full { return None; }
            let (a, b) = (blocks.anchor.as_ref()?, blocks.cursor.as_ref()?);
            if a.endpoint.source_boundary().is_none() || b.endpoint.source_boundary().is_none()
                || a.endpoint.entity_id() != Some(self.selection.entity_id())
                || b.endpoint.entity_id() != Some(self.selection.entity_id())
            { return None; }
            let start = TextSelectionContentKey::new(u64::try_from(a.block_ix?).ok()?);
            let end = TextSelectionContentKey::new(u64::try_from(b.block_ix?).ok()?);
            (a.endpoint.content_key() != Some(start) || b.endpoint.content_key() != Some(end))
                .then_some((start, end))
        })();
        if let Some((start, end)) = boundary_keys {
            self.selection.refresh_source_boundaries(start, end, cx);
        }
        let mut frame = self.frame.borrow_mut();
        frame.content_origin = Some(bounds.origin + scroll_offset);
        self.selection.set_hit_test_runs(&frame.text_runs, cx);
        let registration = TextSelectionRegistration::new(hitbox, bounds)
            .with_scroll_offset(scroll_offset)
            .in_paint_order()
            .with_text_bounds(frame.text_bounds.clone())
            .with_self_scroll(self_scroll)
            .with_rendered_element(&self.selection, window, cx);
        let registration = match self.source_selection_edges(&frame, cx) {
            Some((start, end)) => registration.with_partial_selection_edges(start, end),
            None => match frame.selection_edges.start.zip(frame.selection_edges.end) {
                Some((start, end)) => registration.with_selection_edges(start, end),
                None => registration,
            },
        };
        drop(frame);
        self.selection.register(registration, window, cx);
    }

    /// Lays out the touch handles this view owns; see
    /// [`TextSelectionHandle::prepaint_touch_handles`].
    pub(super) fn prepaint_touch_handles(
        &self,
        window: &mut Window,
        cx: &App,
    ) -> crate::TouchHandleLayout {
        self.selection.prepaint_touch_handles(window, cx)
    }

    /// Paints the touch handles this view owns, after its content for the
    /// frame; see [`TextSelectionHandle::paint_touch_handles`].
    pub(super) fn paint_touch_handles(
        &self,
        layout: &crate::TouchHandleLayout,
        color: gpui::Hsla,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.selection
            .paint_touch_handles(layout, color, window, cx);
    }

    pub(super) fn record_table_cell(&self, bounds: Bounds<Pixels>, cx: &App) {
        self.selection.record_table_cell(bounds, cx);
    }

    pub(super) fn painted_selection_points(
        &self,
        cx: &App,
    ) -> Option<(Point<Pixels>, Point<Pixels>)> {
        let (anchor, cursor) = self.selection_points(cx)?;
        crate::global_state::confine_to_cell(
            self.selection
                .table_cell_selection(cx)
                .map(|bounds| (anchor, bounds)),
            GlobalState::global(cx).table_cell(),
            anchor,
            cursor,
        )
    }

    pub(super) fn selection_points(&self, cx: &App) -> Option<(Point<Pixels>, Point<Pixels>)> {
        let points = self.selection.snapshot(cx)?.window_points()?;
        Some((points.anchor(), points.cursor()))
    }

    pub(super) fn covers_all(&self, cx: &App) -> bool {
        self.block_coverage(cx) == Some(true)
    }

    pub(super) fn excludes_current_block(&self, cx: &App) -> bool {
        self.block_coverage(cx) == Some(false)
    }

    fn block_coverage(&self, cx: &App) -> Option<bool> {
        if let Some(block) = self.frame.borrow().current_block {
            return block.coverage;
        }
        if self.selection.table_cell_selection(cx).is_some() {
            return None;
        }
        let snapshot = self.selection.snapshot(cx)?;
        if snapshot.coverage() == TextSelectionCoverage::Full {
            return Some(true);
        }
        None
    }

    fn selection_for_block(&self, ix: usize, cx: &App) -> SourceBlockSelection {
        let snapshot = self.selection.table_cell_selection(cx).is_none()
            .then(|| self.selection.snapshot(cx)).flatten();
        let open = snapshot.is_some_and(|snapshot| matches!(
            snapshot.coverage(), TextSelectionCoverage::FromStart | TextSelectionCoverage::ToEnd,
        ));
        let coverage = snapshot.and_then(|snapshot| {
            if snapshot.coverage() == TextSelectionCoverage::Full {
                return Some(true);
            }
            if snapshot.coverage() == TextSelectionCoverage::Bounded {
                let owner = Some(self.selection.entity_id());
                if snapshot.anchor().entity_id() != owner || snapshot.cursor().entity_id() != owner {
                    return None;
                }
                let (a, b) = (snapshot.anchor().content_key()?, snapshot.cursor().content_key()?);
                let (first, last) = (a.value().min(b.value()), a.value().max(b.value()));
                let ix = u64::try_from(ix).ok()?;
                return if ix < first || ix > last {
                    Some(false)
                } else {
                    (ix != first && ix != last).then_some(true)
                };
            }
            if !open { return None; }
            let endpoint = [snapshot.anchor(), snapshot.cursor()].into_iter()
                .find(|endpoint| endpoint.entity_id() == Some(self.selection.entity_id()))?;
            let end = usize::try_from(endpoint.content_key()?.value()).ok()?;
            (ix != end).then_some((ix < end) == (snapshot.coverage() == TextSelectionCoverage::FromStart))
        });
        SourceBlockSelection { ix, coverage, open }
    }

    pub(super) fn has_open_selection(&self, cx: &App) -> bool {
        if let Some(block) = self.frame.borrow().current_block {
            return block.open;
        }
        self.open_snapshot(cx).is_some()
    }

    fn open_snapshot(&self, cx: &App) -> Option<TextSelectionSnapshot> {
        if self.selection.table_cell_selection(cx).is_some() {
            return None;
        }
        self.selection.snapshot(cx).filter(|snapshot| {
            matches!(snapshot.coverage(), TextSelectionCoverage::FromStart | TextSelectionCoverage::ToEnd)
        })
    }

    /// Resolve through the retained native frame before replacing its geometry.
    /// Held source ranges survive subsequent reflow without another hit test.
    pub(super) fn refresh_source_selection(&self, document: &ParsedDocument, cx: &App) -> SourceProjection {
        if self.selection.table_cell_selection(cx).is_some() {
            self.virtual_blocks.borrow_mut().invalidate_projection();
            return SourceProjection::default();
        }
        let snapshot = self.selection.snapshot(cx);
        let mut blocks = self.virtual_blocks.borrow_mut();
        let owner = self.selection.entity_id();
        blocks.update(snapshot, owner);
        blocks.source_blocks = std::sync::Arc::downgrade(&document.blocks);
        let Some(snapshot) = snapshot else { return SourceProjection::default(); };
        let frame = self.frame.borrow();
        // Capture each endpoint while its own native fragment is available.
        // A later drag can then keep the stationary endpoint offscreen.
        {
            let cached = &mut *blocks;
            for endpoint in [&mut cached.anchor, &mut cached.cursor].into_iter().flatten() {
                if endpoint.endpoint.entity_id() == Some(owner) && endpoint.source_position.is_none() {
                    // Once Full becomes a bounded gesture, its held source
                    // offset is fixed. A replaced/dead source cannot silently
                    // turn that marker into the new document end after append.
                    if endpoint.endpoint.source_boundary().is_none() || snapshot.coverage() == TextSelectionCoverage::Full {
                        endpoint.source_position = frame.source_endpoint(endpoint.endpoint, &document.blocks);
                    }
                }
            }
        }
        if blocks.source_projected {
            return SourceProjection { active: true, complete: true, changed: false };
        }
        let projected = match snapshot.coverage() {
            TextSelectionCoverage::FromStart | TextSelectionCoverage::ToEnd => {
                let endpoint = if snapshot.anchor().entity_id() == Some(owner) {
                    blocks.anchor.as_mut()
                } else if snapshot.cursor().entity_id() == Some(owner) {
                    blocks.cursor.as_mut()
                } else { None };
                endpoint.and_then(|endpoint| {
                    let block = endpoint.block_ix?;
                    let from_start = snapshot.coverage() == TextSelectionCoverage::FromStart;
                    let (complete, changed) = endpoint.project_boundary(from_start, document)?;
                    frame.project_complete_blocks(|ix| {
                        (ix != block).then_some((ix < block) == from_start)
                    });
                    Some(SourceProjection { active: true, changed, complete })
                })
            },
            TextSelectionCoverage::Bounded => {
                let same_block = (|| {
                    let (a, b) = (blocks.anchor.as_ref()?, blocks.cursor.as_ref()?);
                    if a.endpoint.entity_id() != Some(owner) || b.endpoint.entity_id() != Some(owner) {
                        return None;
                    }
                    let block = a.block_ix?;
                    if b.block_ix != Some(block) { return None; }
                    let source = document.blocks.get(block)?;
                    let (start, end) = (a.source_position.as_ref()?, b.source_position.as_ref()?);
                    let same_text = match (start, end) {
                        (SourceSelectionPosition::Text(start), SourceSelectionPosition::Text(end)) => {
                            start.select_to(end, || source.clear_source_selection())
                        },
                        _ => false,
                    };
                    if !same_text && !source.project_source_selection(Some(start), Some(end))
                    { return None; }
                    frame.project_complete_blocks(|ix| (ix != block).then_some(false));
                    Some(SourceProjection { active: true, complete: true, changed: true })
                })();
                let split = (|| {
                    if same_block.is_some() { return None; }
                    let (a, b) = (blocks.anchor.as_ref()?, blocks.cursor.as_ref()?);
                    if a.endpoint.entity_id() != Some(owner) || b.endpoint.entity_id() != Some(owner) {
                        return None;
                    }
                    let (a_block, b_block) = (a.block_ix?, b.block_ix?);
                    if a_block == b_block || document.blocks.get(a_block).is_none()
                        || document.blocks.get(b_block).is_none()
                    { return None; }
                    let (first, last) = (a_block.min(b_block), a_block.max(b_block));
                    let (a_ready, a_changed) = blocks.anchor.as_mut()?
                        .project_boundary(a_block == last, document)?;
                    let (b_ready, b_changed) = blocks.cursor.as_mut()?
                        .project_boundary(b_block == last, document)?;
                    frame.project_complete_blocks(|ix| {
                        (ix != first && ix != last).then_some(ix > first && ix < last)
                    });
                    Some(SourceProjection {
                        active: true, changed: a_changed || b_changed,
                        complete: a_ready && b_ready,
                    })
                })();
                let pending = (|| {
                    if same_block.is_some() || split.is_some() { return None; }
                    let (a, b) = (blocks.anchor.as_ref()?, blocks.cursor.as_ref()?);
                    if a.endpoint.entity_id() != Some(owner) || b.endpoint.entity_id() != Some(owner)
                        || a.block_ix.is_none() || a.block_ix != b.block_ix
                    { return None; }
                    let block = a.block_ix?;
                    let source = document.blocks.get(block)?;
                    // An unresolved source must wait for its own geometry,
                    // even when neither endpoint has been resolved yet.
                    // Never substitute a different visible text/plugin source.
                    let changed = source.has_selection();
                    if changed { source.clear_selection(); }
                    frame.project_complete_blocks(|ix| (ix != block).then_some(false));
                    Some(SourceProjection { active: true, complete: false, changed })
                })();
                if same_block.is_some() {
                    blocks.invalidate_projection();
                    same_block
                } else if split.is_some() { split }
                else if pending.is_some() { pending }
                else if frame.project_bounded(snapshot, owner) {
                    // A paired range in one block is not a reusable complete
                    // prefix or suffix of either endpoint.
                    blocks.invalidate_projection();
                    Some(SourceProjection { active: true, complete: true, changed: true })
                } else { None }
            },
            TextSelectionCoverage::Full => None,
        };
        if let Some(projected) = projected {
            blocks.source_projected = projected.complete;
            projected
        } else {
            blocks.invalidate_projection();
            SourceProjection::default()
        }
    }

    pub(super) fn clear_selection(&self, cx: &mut App) {
        self.selection.clear_selection(cx);
        self.virtual_blocks.replace(VirtualBlockSelection::default());
    }

    pub(super) fn set_local_selection(&self, active: bool, cx: &mut App) {
        if active { self.virtual_blocks.borrow_mut().invalidate_projection(); }
        self.selection.set_local_selection(active, cx);
    }

    /// See [`TextSelectionHandle::hold_multi_click`].
    pub(super) fn hold_multi_click(
        &self,
        start: Bounds<Pixels>,
        end: Bounds<Pixels>,
        click_count: usize,
        cx: &mut App,
    ) {
        self.selection.hold_multi_click(start, end, click_count, cx);
    }

    pub(super) fn is_part_of_window_selection(&self, cx: &App) -> bool {
        let id = self.selection.entity_id();
        self.selection.snapshot(cx).is_some_and(|snapshot| {
            snapshot.anchor().entity_id() == Some(id) || snapshot.cursor().entity_id() == Some(id)
        })
    }

    pub(super) fn has_selection_snapshot(&self, cx: &App) -> bool {
        self.selection.snapshot(cx).is_some()
    }

    #[cfg(test)]
    pub(super) fn text_bounds(&self) -> Vec<Bounds<Pixels>> {
        self.frame.borrow().text_bounds.clone()
    }

    /// Just inside the character at `index` of the text run painted this
    /// frame with `text`, in the middle of its row.
    #[cfg(test)]
    pub(super) fn text_position(&self, text: &str, index: usize) -> Point<Pixels> {
        let frame = self.frame.borrow();
        let layout = frame
            .text_runs
            .iter()
            .find(|run| run.text().as_ref() == text)
            .unwrap()
            .layout();
        layout.position_for_index(index).unwrap()
            + gpui::point(gpui::px(1.), layout.line_height() / 2.)
    }

    #[cfg(test)]
    pub(super) fn first_text_position(&self, index: usize) -> Point<Pixels> {
        let frame = self.frame.borrow();
        let layout = frame.text_runs.first().unwrap().layout();
        layout.position_for_index(index).unwrap()
            + gpui::point(Pixels::ZERO, layout.line_height() / 2.)
    }
}
