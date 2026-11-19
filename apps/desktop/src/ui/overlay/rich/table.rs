use std::sync::OnceLock;
use gpui::{Rgba, TableCellAlignment};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::base::text::{MarkdownExtensions, MarkdownNode, StructuredBlock};
use gpui_kit::base::text::editor_host::{BlockNode, BlockState, FlowPosition, RenderContext, auto_table_block, native_table_cell};
use super_paste_core::preview::rich::{Element, ElementKind as Kind, Node, VerticalAlignment};
use super::{Adapter, Inherited, LINE_HEIGHT, TABLE_FONT_SIZE};
use gpui_kit::base::text::editor_host::{InlineNode, Paragraph};

mod borders;

const TABLE_NODE: &str = "super-paste-rich-table";
const MAX_COLUMNS: usize = 2_000;
// Grid line coordinates are signed 16-bit values. Reserve the final positive
// line for the end edge; RTF can contain many empty rows without text nodes.
const MAX_ROWS: usize = i16::MAX as usize - 1;
const NUMBER_WIDTH: f32 = 28.0;
const DATA_MIN_WIDTH: f32 = 100.0;

pub(super) fn from_preview(preview: &super_paste_core::preview::TablePreview) -> Option<BlockNode> {
    let columns = preview.columns.checked_add(1)?;
    if columns >= i16::MAX as usize || preview.rows.len() > MAX_ROWS { return None; }
    let mut table = NativeTable { cells: Vec::new(), rows: preview.rows.len() as u16, columns: columns as u16, data: true, rtl: false, background: None, row_backgrounds: Vec::new(), images: super::images::Sources::default() };
    for (row, values) in preview.rows.iter().enumerate() {
        let number = SharedString::from((row + 1).to_string());
        for (column, value) in std::iter::once(number).chain(values.iter().cloned().map(SharedString::from)).enumerate() {
            let title = (column != 0).then(|| value.clone());
            let (direction, alignment) = super::automatic_style(&value);
            let alignment = if column == 0 { TextAlign::Center } else { alignment };
            let blocks = vec![BlockNode::Paragraph(Paragraph::from_runs(vec![InlineNode::new(value)]).with_alignment(Some(alignment)).with_direction(Some(direction)))];
            table.cells.push(Cell { row: row as u16, column: column as u16, rows: 1, columns: 1, blocks, title, background: None, vertical: VerticalAlignment::Top });
        }
    }
    Some(BlockNode::Custom { node: MarkdownNode::structured(TABLE_NODE, table).accessibility_label("表格"), state: BlockState::default() })
}

#[derive(Clone)]
struct NativeTable { cells: Vec<Cell>, rows: u16, columns: u16, data: bool, rtl: bool, background: Option<Hsla>, row_backgrounds: Vec<(u16, Hsla)>, images: super::images::Sources }
impl StructuredBlock for NativeTable {
    fn reuse_presentation_sources(&mut self, previous: &Self) -> bool {
        if self.rows != previous.rows || self.columns != previous.columns || self.cells.len() != previous.cells.len() { return false; }
        self.cells.iter_mut().zip(&previous.cells).all(|(next, previous)| {
            next.row == previous.row && next.column == previous.column && next.rows == previous.rows && next.columns == previous.columns
                && next.blocks.len() == previous.blocks.len()
                && next.blocks.iter_mut().zip(&previous.blocks).all(|(next, previous)| next.reuse_presentation_sources(previous))
        })
    }
    fn visit_groups(&self, visitor: &mut dyn FnMut(&[BlockNode], &str)) {
        for (index, cell) in self.cells.iter().enumerate() {
            let separator = if self.cells.get(index + 1).is_some_and(|next| next.row == cell.row) { "\t" } else { "\n" };
            visitor(&cell.blocks, separator);
        }
    }
}
#[derive(Clone)]
struct Cell { row: u16, column: u16, rows: u16, columns: u16, blocks: Vec<BlockNode>, title: Option<SharedString>, background: Option<Hsla>, vertical: VerticalAlignment }
// Keep source references, not an owned style snapshot for every source row.
// Resolve only the section and row that adaptation has actually reached.
struct Group<'a> { rank: u8, nodes: &'a [Node], parent: Option<&'a Element> }

fn background_over(backdrop: Option<Rgba>, foreground: Option<Hsla>) -> Option<Rgba> {
    let Some(front) = foreground.map(Hsla::to_rgb).filter(|color| color.a > 0.0) else { return backdrop; };
    let Some(back) = backdrop.filter(|_| front.a < 1.0) else { return Some(front); };
    // Rgba::blend assumes an opaque destination. These layers remain
    // translucent over the separately painted table fill.
    let retained = back.a * (1.0 - front.a);
    let alpha = front.a + retained;
    Some(Rgba { r: (front.r * front.a + back.r * retained) / alpha,
        g: (front.g * front.a + back.g * retained) / alpha,
        b: (front.b * front.a + back.b * retained) / alpha, a: alpha })
}

/// One renderer registry per process. Native cell documents recursively use
/// the same registry; no HTML, parser task, or extra TextView entity per cell.
pub(super) fn extensions() -> &'static MarkdownExtensions {
    static EXTENSIONS: OnceLock<MarkdownExtensions> = OnceLock::new();
    EXTENSIONS.get_or_init(|| MarkdownExtensions::default().block_renderer_with_context(TABLE_NODE, render).block_renderer(super::files::NODE, super::files::render))
}

fn render(node: &MarkdownNode, inherited: &RenderContext, window: &mut Window, cx: &mut App) -> AnyElement {
    let Some(table) = node.data::<NativeTable>() else { return div().into_any_element(); };
    if table.data { return render_data(node, table, window, cx); }
    let border = crate::ui::palette::Palette::for_window(cx.global::<crate::ui::configuration::ConfigurationGlobal>().0.read(cx).projected.settings.color_mode, window).control_border;
    let images = table.images.clone();
    let context = inherited.clone().with_style(super::style(window, TABLE_FONT_SIZE))
        .with_body_font_size(px(TABLE_FONT_SIZE)).with_markdown_extensions(extensions().clone())
        .with_image_source(move |uri| images.resolve(uri));
    // Structured data lives in the document's stable Arc allocation. Scope
    // cell IDs to that owner, including tables nested inside another cell.
    let mut grid = div().id((TABLE_NODE, std::ptr::from_ref(table) as usize)).w_full().min_w_0().grid().grid_cols_auto(table.columns.max(1)).grid_rows_auto(table.rows)
        .text_size(px(TABLE_FONT_SIZE)).line_height(relative(LINE_HEIGHT));
    for (index, cell) in table.cells.iter().enumerate() {
        // Mirror the occupied span in the table's grid, not the source cells.
        // Row/cell direction only affects content; it cannot reverse columns.
        let column = if table.rtl { table.columns - cell.column - cell.columns } else { cell.column };
        let mut element = div().id(("rich-cell", index)).min_w_0().px(px(8.0)).py(px(5.0))
            .flex().flex_col()
            .col_span(cell.columns).col_start(column as i16 + 1)
            .row_span(cell.rows).row_start(cell.row as i16 + 1);
        let alignment = match cell.vertical {
            VerticalAlignment::Top => TableCellAlignment::Top,
            VerticalAlignment::Middle => TableCellAlignment::Middle,
            VerticalAlignment::Bottom => TableCellAlignment::Bottom,
            // CSS 2 table cells treat other vertical-align values as baseline.
            _ => TableCellAlignment::Baseline,
        };
        // One block formatting context inside the alignment flex item keeps
        // paragraph margins inside the cell and collapses adjacent margins.
        // A baseline-aligned rowspan may overflow its allocated border box.
        // Preserve the natural content height and the reference page's visible
        // overflow; alignment must not shrink or clip that content to fit.
        let mut content = div().w_full().min_w_0().flex_shrink_0();
        for (index, block) in cell.blocks.iter().enumerate() {
            content = content.child(block.render(index, FlowPosition::at(&cell.blocks, index, index + 1 == cell.blocks.len()), &context, window, cx));
        }
        element = element.child(content);
        grid = grid.child(native_table_cell(element.into_any_element(), alignment));
    }
    borders::table(auto_table_block(grid.into_any_element()), node.clone(), border)
}

fn render_data(node: &MarkdownNode, table: &NativeTable, window: &mut Window, cx: &mut App) -> AnyElement {
    let p = crate::ui::palette::Palette::for_window(cx.global::<crate::ui::configuration::ConfigurationGlobal>().0.read(cx).projected.settings.color_mode, window);
    let context = RenderContext::new(gpui_kit::base::TextViewStyle::default().with_foreground(p.text_primary));
    let numbers = RenderContext::new(gpui_kit::base::TextViewStyle::default().with_foreground(p.text_secondary));
    let mut body = div().id((TABLE_NODE, std::ptr::from_ref(table) as usize)).w_full().min_w_0().grid().grid_rows_auto(table.rows)
        .grid_cols_auto_with_leading(table.columns, px(NUMBER_WIDTH))
        .text_size(px(TABLE_FONT_SIZE)).line_height(relative(LINE_HEIGHT));
    for (index, cell) in table.cells.iter().enumerate() {
        let mut element = div().id(("data-cell", index)).flex().flex_col().py(px(5.0))
            .col_start(cell.column as i16 + 1).row_start(cell.row as i16 + 1);
        // Numeric row labels have no wrapping opportunities. Preserve that
        // intrinsic width even when a long row number exceeds the preference.
        element = if cell.column == 0 { element.min_w_0().px(px(3.0)).text_size(px(10.0)).font_weight(FontWeight::BOLD).whitespace_nowrap() }
            else { element.min_w(px(DATA_MIN_WIDTH)).px(px(8.0)) };
        element = element.when_some(cell.title.clone(), |cell, title| cell.tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx)));
        let mut content = div().w_full().min_w_0().flex_shrink_0();
        for (index, block) in cell.blocks.iter().enumerate() {
            content = content.child(block.render(index, FlowPosition::at(&cell.blocks, index, index + 1 == cell.blocks.len()), if cell.column == 0 { &numbers } else { &context }, window, cx));
        }
        body = body.child(native_table_cell(element.child(content).into_any_element(), TableCellAlignment::Top));
    }
    borders::data_table(auto_table_block(body.into_any_element()), node.clone(), p.control_border, p.bg_secondary, p.bg_tertiary)
}

impl Adapter {
    pub(super) fn table(&mut self, nodes: &[Node], state: &Inherited) -> Vec<BlockNode> {
        let mut output = Vec::new(); let mut groups = Vec::new();
        let mut direct: Option<std::ops::Range<usize>> = None;
        let mut header_seen = false; let mut footer_seen = false;
        for (index, node) in nodes.iter().enumerate() {
            let Node::Element(element) = node else { continue; };
            match element.kind {
                Kind::Caption => {
                    let mut caption = self.styled(element, state);
                    let background = caption.background_color().filter(|color| color.a > 0.0);
                    caption.mark.highlight = None;
                    caption.mark.baseline_shift = None;
                    caption.apply_block_strut(state);
                    if !self.layout_node() { break; }
                    let children = self.blocks(&element.children, &caption, false);
                    output.push(super::block_box(children, background, super::block_flow(&element.kind, &caption, false)));
                },
                Kind::TableRow => match direct.as_mut() {
                    Some(range) => range.end = index + 1,
                    None => direct = Some(index..index + 1),
                },
                Kind::TableHead | Kind::TableBody | Kind::TableFoot => {
                    if let Some(range) = direct.take() { groups.push(Group { rank: 1, nodes: &nodes[range], parent: None }); }
                    // Only the first header/footer receives the specialized
                    // placement. Later sections keep their body/source order,
                    // even when the first specialized section is empty.
                    let rank = match element.kind {
                        Kind::TableHead if !header_seen => { header_seen = true; 0 },
                        Kind::TableFoot if !footer_seen => { footer_seen = true; 2 },
                        _ => 1,
                    };
                    groups.push(Group { rank, nodes: &element.children, parent: Some(element) });
                },
                _ => self.unsupported = true,
            }
        }
        if let Some(range) = direct { groups.push(Group { rank: 1, nodes: &nodes[range], parent: None }); }
        // Browser table layout places head/foot sections around the body,
        // independently of their source order. Spans stop at group boundaries.
        groups.sort_by_key(|group| group.rank);
        let mut table = NativeTable { cells: Vec::new(), rows: 0, columns: 0, data: false, rtl: state.rtl,
            background: state.background_color().filter(|color| color.a > 0.0), row_backgrounds: Vec::new(), images: self.images.clone() }; let mut row = 0usize;
        'groups: for group in groups {
            let rows = group.nodes.iter().filter_map(|node| match node {
                Node::Element(element) if matches!(element.kind, Kind::TableRow) => Some(element),
                _ => None,
            });
            let group_rows = rows.clone().count();
            if group_rows == 0 { continue; }
            if self.nodes_left == 0 || self.text_left == 0 { self.truncated = true; break; }
            let section = match group.parent { Some(element) => self.styled(element, state), None => state.clone() };
            let section_background = background_over(None, group.parent.and_then(|_| section.background_color()));
            let mut occupied_until = Vec::<usize>::new();
            for (group_index, source_row) in rows.enumerate() {
                if row >= MAX_ROWS { self.truncated = true; break 'groups; }
                if !self.layout_node() { break 'groups; }
                let inherited = self.styled(source_row, &section);
                let row_background = background_over(section_background, inherited.background_color());
                if let Some(color) = row_background { table.row_backgrounds.push((row as u16, color.into())); }
                let mut column = 0usize;
                for node in source_row.children.iter() {
                    let Node::Element(element) = node else { continue; };
                    let Kind::Cell { columns, rows, .. } = element.kind else { self.unsupported = true; continue; };
                    let columns = usize::from(columns.max(1));
                    let rows = usize::from(rows.max(1)).min(group_rows - group_index).min(MAX_ROWS - row);
                    loop {
                        while occupied_until.get(column).is_some_and(|end| *end > row) { column += 1; }
                        if column + columns > MAX_COLUMNS { self.truncated = true; break 'groups; }
                        if let Some(offset) = (column..column + columns).position(|index| occupied_until.get(index).is_some_and(|end| *end > row)) { column += offset + 1; }
                        else { break; }
                    }
                    if !self.layout_node() { break 'groups; }
                    let mut cell = self.styled(element, &inherited); let vertical = cell.vertical.clone();
                    cell.native_body_size = TABLE_FONT_SIZE;
                    cell.mark.inline_box = None;
                    cell.mark.baseline_shift = None;
                    cell.apply_block_strut(state);
                    // CSS paints the originating section and row through the
                    // complete cell, including row spans, then its own fill.
                    // Solid colors compose once here; no full-width row band
                    // may overwrite the originating row in a spanning cell.
                    let background = background_over(row_background, cell.background_color()).map(Into::into);
                    cell.mark.highlight = None;
                    let blocks = self.blocks(&element.children, &cell, false);
                    occupied_until.resize(occupied_until.len().max(column + columns), 0);
                    for end in &mut occupied_until[column..column + columns] { *end = row + rows; }
                    table.columns = table.columns.max((column + columns) as u16);
                    table.cells.push(Cell { row: row as u16, column: column as u16, rows: rows as u16, columns: columns as u16, blocks, title: None, background, vertical });
                    column += columns;
                }
                row += 1;
            }
        }
        // Keep empty rows in the native grid as well; their existence and
        // positions affect cross-row distribution even when their height is 0.
        let rows = table.cells.last().map_or(row, |cell| row.max(usize::from(cell.row) + 1));
        table.rows = rows as u16;
        table.row_backgrounds.retain(|(row, _)| usize::from(*row) < rows);
        if self.truncated {
            // A partially adapted last row can already own cells. Keep that
            // row, but do not leave spans pointing into rows omitted by the
            // display budget: grid would otherwise create phantom tracks.
            for cell in &mut table.cells {
                cell.rows = cell.rows.min((rows - usize::from(cell.row)) as u16);
            }
        }
        // Complete tables paint these layers through their cell fills. Drop
        // redundant row metadata so the common path needs no coverage scan.
        let occupied = table.cells.iter().map(|cell| u64::from(cell.rows) * u64::from(cell.columns)).sum::<u64>();
        if occupied == u64::from(table.rows) * u64::from(table.columns) {
            table.row_backgrounds = Vec::new();
        }
        if !table.cells.is_empty() {
            let node = if self.restyle_variables { MarkdownNode::restylable_structured(TABLE_NODE, table) }
                else { MarkdownNode::structured(TABLE_NODE, table) };
            output.push(BlockNode::Custom { node: node.accessibility_label("表格"), state: BlockState::default() });
        }
        output
    }
}

