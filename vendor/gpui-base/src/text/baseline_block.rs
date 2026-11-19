use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window,
};

use crate::GlobalState;

/// A semantic block exports its native box baseline, rather than whichever
/// text descendant happens to prepaint first. It adds no box or layout cache.
pub(super) enum TableSizing { Fixed, Auto }
pub(super) struct BaselineBlock(pub(super) AnyElement, pub(super) Option<TableSizing>);

impl IntoElement for BaselineBlock {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for BaselineBlock {
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
        let layout_id = self.0.request_layout(window, cx);
        if let Some(sizing) = &self.1 {
            window.set_table_grid(layout_id);
            if matches!(sizing, TableSizing::Auto) { window.set_table_auto_columns(layout_id); }
        }
        (layout_id, layout_id)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout_id: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let observing = GlobalState::global(cx).first_line_pending();
        if observing {
            if let Some(baseline) = window.layout_first_baseline(*layout_id) {
                GlobalState::global(cx).report_first_line(bounds.top() + baseline);
            }
            // A baseline-less native box does not export an arbitrary text
            // descendant as its baseline. Nested markers still get own scopes.
            GlobalState::global(cx).suppress_first_line();
        }
        self.0.prepaint(window, cx);
        if observing { GlobalState::global(cx).pop_suppressed_first_line(); }
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
        self.0.paint(window, cx);
    }
}
