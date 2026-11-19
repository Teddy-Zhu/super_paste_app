//! Outside markers use the content's actual first line. The content keeps its
//! native block layout: this wrapper contributes no flex row or margin barrier.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, ParentElement as _, Pixels, SharedString,
    StyledText, TextLayout, Window, point, size,
};

use crate::GlobalState;

pub(super) struct OutsideListItem {
    content: AnyElement,
    marker: AnyElement,
    marker_layout: TextLayout,
    text_geometry: Rc<Cell<MarkerGeometry>>,
    indent: Pixels,
    rtl: bool,
}

impl OutsideListItem {
    pub fn new(content: AnyElement, marker_box: Div, text: SharedString, indent: Pixels, rtl: bool) -> Self {
        let text = StyledText::new(text);
        let marker_layout = text.layout().clone();
        let text_geometry = Rc::new(Cell::new(MarkerGeometry::default()));
        let marker = marker_box.child(MarkerText {
            text,
            geometry: text_geometry.clone(),
        }).into_any_element();
        Self { content, marker, marker_layout, text_geometry, indent, rtl }
    }
}

impl IntoElement for OutsideListItem {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for OutsideListItem {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let marker_id = self.marker.request_layout(window, cx);
        self.marker.layout_as_root(
            size(AvailableSpace::Definite(self.indent), AvailableSpace::MaxContent), window, cx,
        );
        let content_id = self.content.request_layout(window, cx);
        if let Some(baseline) = self.marker_layout.first_baseline() {
            let text_id = self.text_geometry.get().layout_id.expect("marker text was requested");
            let marker_bounds = window.layout_bounds(marker_id);
            let text_top = window.layout_bounds(text_id).top();
            window.set_outside_list_marker(
                content_id, text_top - marker_bounds.top() + baseline, marker_bounds.size.height,
            );
        }
        (content_id, marker_id)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        marker_id: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        GlobalState::global(cx).push_first_line();
        self.content.prepaint(window, cx);
        let first = GlobalState::global(cx).pop_first_line();

        let mut top = bounds.top();
        if let Some(content_baseline) = first
            && let Some(marker_baseline) = self.marker_layout.first_baseline()
        {
            let mut geometry = self.text_geometry.get();
            let text_id = geometry.layout_id.expect("marker text was requested");
            let text_top = window.layout_bounds(text_id).top();
            let marker_top = window.layout_bounds(*marker_id).top();
            top = content_baseline - (text_top - marker_top + marker_baseline);
            // InlineFlow deliberately keeps subpixel baselines. Avoid losing
            // their alignment to the element-bound snapping of a new root.
            geometry.paint_top = Some(content_baseline - marker_baseline);
            self.text_geometry.set(geometry);
        }
        let left = if self.rtl { bounds.right() } else { bounds.left() - self.indent };
        self.marker.prepaint_at(point(left, top), window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.paint(window, cx);
        self.marker.paint(window, cx);
    }
}

/// Observe the native child's layout id without introducing a box or shaping.
struct MarkerText {
    text: StyledText,
    geometry: Rc<Cell<MarkerGeometry>>,
}

#[derive(Clone, Copy, Default)]
struct MarkerGeometry {
    layout_id: Option<LayoutId>,
    paint_top: Option<Pixels>,
}

impl IntoElement for MarkerText {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for MarkerText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let (layout, state) = self.text.request_layout(id, inspector, window, cx);
        self.geometry.set(MarkerGeometry { layout_id: Some(layout), paint_top: None });
        (layout, state)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        mut bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(top) = self.geometry.get().paint_top {
            bounds.origin.y = top;
        }
        self.text.prepaint(id, inspector, bounds, state, window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.paint(id, inspector, bounds, state, prepaint, window, cx);
    }
}
