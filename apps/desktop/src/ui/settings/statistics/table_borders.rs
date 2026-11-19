use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, Window, fill, point, px,
};

/// A bottom-only collapsed table reuses the native grid and cell boxes. Keep
/// border geometry only for this frame, and emit one clipped quad per row.
pub(super) fn horizontal(content: AnyElement, columns: usize, color: Hsla) -> AnyElement {
    RowBorders { content, columns, color }.into_any_element()
}

struct RowBorders { content: AnyElement, columns: usize, color: Hsla }

impl IntoElement for RowBorders {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for RowBorders {
    type RequestLayoutState = (LayoutId, Pixels);
    type PrepaintState = Vec<Bounds<Pixels>>;

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout = self.content.request_layout(window, cx);
        let stroke = window.set_collapsed_table_row_border_width(layout, self.columns, px(1.0));
        (layout, (layout, stroke))
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        request: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let clip = window.content_mask().bounds;
        let half = request.1 / 2.0;
        let mut borders = Vec::new();
        let mut row: Option<Bounds<Pixels>> = None;
        window.visit_unrounded_layout_children(request.0, |index, bounds| {
            row = Some(match row {
                None => bounds,
                Some(previous) => Bounds::from_corners(
                    point(previous.left().min(bounds.left()), previous.top().min(bounds.top())),
                    point(previous.right().max(bounds.right()), previous.bottom().max(bounds.bottom())),
                ),
            });
            if (index + 1) % self.columns != 0 { return; }
            let row = row.take().expect("completed native table row");
            let left = row.left().max(clip.left());
            let right = row.right().min(clip.right());
            let top = (row.bottom() - half).max(clip.top());
            let bottom = (row.bottom() + half).min(clip.bottom());
            if right > left && bottom > top {
                borders.push(Bounds::from_corners(point(left, top), point(right, bottom)));
            }
        });
        self.content.prepaint(window, cx);
        borders
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        borders: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for &bounds in borders.iter() { window.paint_quad(fill(bounds, self.color)); }
        self.content.paint(window, cx);
    }
}
