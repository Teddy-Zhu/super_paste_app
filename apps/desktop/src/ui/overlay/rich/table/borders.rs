use gpui::{
    AnyElement, App, Bounds, Element, ElementId, FillOptions, FillRule, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, Path, PathBuilder, PathStyle, Pixels,
    Window, fill, point, px,
};
use gpui_kit::base::text::MarkdownNode;

use super::NativeTable;

/// The reference page gives every HTML cell the same solid border. Keep its
/// half-width in native layout, then paint the union once, between all cell
/// backgrounds and their content. Missing cells do not acquire phantom edges.
pub(super) fn table(content: AnyElement, owner: MarkdownNode, color: Hsla) -> AnyElement {
    CollapsedTable { content, owner, color, data_backgrounds: None }.into_any_element()
}

pub(super) fn data_table(content: AnyElement, owner: MarkdownNode, color: Hsla, header: Hsla, alternate: Hsla) -> AnyElement {
    CollapsedTable { content, owner, color, data_backgrounds: Some((header, alternate)) }.into_any_element()
}

struct CollapsedTable { content: AnyElement, owner: MarkdownNode, color: Hsla, data_backgrounds: Option<(Hsla, Hsla)> }
struct TablePaint { backgrounds: Vec<(Bounds<Pixels>, Hsla)>, borders: Option<Path<Pixels>> }

impl IntoElement for CollapsedTable {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for CollapsedTable {
    type RequestLayoutState = (LayoutId, Pixels);
    type PrepaintState = TablePaint;

    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.content.request_layout(window, cx);
        let width = window.set_collapsed_table_border_width(layout_id, px(1.0));
        (layout_id, (layout_id, width))
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
        let table = self.owner.data::<NativeTable>().expect("native table paint owner must match its layout");
        let clip = window.content_mask().bounds;
        let half = request.1 / 2.0;
        let mut backgrounds = Vec::new();
        if self.data_backgrounds.is_none() {
            collect_row_gap_backgrounds(table, request.0, clip, &mut backgrounds, window);
        }
        let mut path = PathBuilder::fill().with_style(PathStyle::Fill(
            FillOptions::default().with_fill_rule(FillRule::NonZero),
        ));
        let mut has_borders = false;
        let mut children = 0;
        // Inspect the already computed boxes in source order. This neither
        // copies LayoutOutput nor stores a second row/column geometry model.
        window.visit_unrounded_layout_children(request.0, |index, bounds| {
            children += 1;
            let cell = table.cells.get(index).expect("native table cells must match layout children");
            let background = match self.data_backgrounds {
                Some((header, _)) if cell.column == 0 => Some(header),
                Some((_, alternate)) if cell.row % 2 == 1 => Some(alternate),
                Some(_) => None,
                None => cell.background,
            };
            if let (Some(color), Some(bounds)) = (background, intersection(bounds, clip)) {
                backgrounds.push((bounds, color));
            }
            let left = bounds.left() - half;
            let right = bounds.right() + half;
            let top = bounds.top() - half;
            let bottom = bounds.bottom() + half;
            for strip in [
                Bounds::from_corners(point(left, top), point(right, bounds.top() + half)),
                Bounds::from_corners(point(left, bounds.bottom() - half), point(right, bottom)),
                Bounds::from_corners(point(left, bounds.top() + half), point(bounds.left() + half, bounds.bottom() - half)),
                Bounds::from_corners(point(bounds.right() - half, bounds.top() + half), point(right, bounds.bottom() - half)),
            ] {
                // Cull and clip individual strips, including enormous spans
                // whose border lies completely outside the visible viewport.
                if let Some(strip) = intersection(strip, clip) {
                    rectangle(&mut path, strip);
                    has_borders = true;
                }
            }
        });
        assert_eq!(children, table.cells.len(), "native table cells must match layout children");
        // Non-zero fill unions coincident edges and intersections. A table
        // fade therefore does not darken seams by applying opacity twice.
        // These finite, clipped rectangles are trusted paint geometry; wide
        // tessellation indices remove the old 65,535-vertex input limit.
        let borders = has_borders.then(|| path.build().expect("native border rectangles must tessellate"));
        self.content.prepaint(window, cx);
        TablePaint { backgrounds, borders }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        paint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let table = self.owner.data::<NativeTable>().expect("native table paint owner must match its layout");
        if let Some(background) = table.background { window.paint_quad(fill(bounds, background)); }
        for &(bounds, background) in &paint.backgrounds { window.paint_quad(fill(bounds, background)); }
        if let Some(borders) = paint.borders.take() { window.paint_path(borders, self.color); }
        self.content.paint(window, cx);
    }
}

/// Cell fills already contain their originating row/section layers. Paint
/// those layers only in missing cells, otherwise translucent fills double.
/// One temporary coverage row is enough for spans; native tracks supply all
/// coordinates and the existing paint vector holds only visible rectangles.
fn collect_row_gap_backgrounds(
    table: &NativeTable, layout_id: LayoutId, clip: Bounds<Pixels>,
    backgrounds: &mut Vec<(Bounds<Pixels>, Hsla)>, window: &mut Window,
) {
    if table.row_backgrounds.is_empty() { return; }
    let columns = usize::from(table.columns);
    let bounds = window.unrounded_grid_area_bounds(layout_id, 0..usize::from(table.rows), 0..columns)
        .expect("native table must have resolved grid tracks");
    if intersection(bounds, clip).is_none() { return; }
    let mut covered_until = Vec::new();
    let mut next_cell = 0;
    for &(row, color) in &table.row_backgrounds {
        let rows = usize::from(row)..usize::from(row) + 1;
        let bounds = window.unrounded_grid_area_bounds(layout_id, rows.clone(), 0..columns)
            .expect("native table row must have resolved grid tracks");
        if intersection(bounds, clip).is_none() { continue; }
        if covered_until.is_empty() { covered_until.resize(columns, 0_u16); }
        // Hidden preceding rows need no geometry or fill. Their spanning
        // cells still participate when the next visible styled row is read.
        while let Some(cell) = table.cells.get(next_cell).filter(|cell| cell.row <= row) {
            let start = usize::from(cell.column);
            covered_until[start..start + usize::from(cell.columns)].fill(cell.row + cell.rows);
            next_cell += 1;
        }
        let mut column = 0;
        while column < columns {
            while column < columns && covered_until[column] > row { column += 1; }
            let start = column;
            while column < columns && covered_until[column] <= row { column += 1; }
            if start == column { continue; }
            let physical = if table.rtl { columns - column..columns - start } else { start..column };
            let bounds = window.unrounded_grid_area_bounds(layout_id, rows.clone(), physical)
                .expect("native table gap must have resolved grid tracks");
            if let Some(bounds) = intersection(bounds, clip) { backgrounds.push((bounds, color)); }
        }
    }
}

fn intersection(bounds: Bounds<Pixels>, clip: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    let left = bounds.left().max(clip.left());
    let top = bounds.top().max(clip.top());
    let right = bounds.right().min(clip.right());
    let bottom = bounds.bottom().min(clip.bottom());
    (right > left && bottom > top).then(|| Bounds::from_corners(point(left, top), point(right, bottom)))
}

fn rectangle(path: &mut PathBuilder, bounds: Bounds<Pixels>) {
    path.move_to(point(bounds.left(), bounds.top()));
    path.line_to(point(bounds.right(), bounds.top()));
    path.line_to(point(bounds.right(), bounds.bottom()));
    path.line_to(point(bounds.left(), bounds.bottom()));
    path.close();
}
