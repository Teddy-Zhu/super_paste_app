use std::sync::Arc;
use gpui_kit::{prelude::*, *};
use gpui_kit::base::TextViewStyle;
use gpui_kit::base::text::{MarkdownNode, StructuredBlock};
use gpui_kit::base::text::editor_host::{BlockNode, BlockState, FlowPosition, InlineNode, Paragraph, RenderContext, TextMark, table_cell};
use gpui_kit::component::tooltip::Tooltip;
use super_paste_core::preview::FilePreview;
use crate::ui::{configuration::ConfigurationGlobal, icon, palette::Palette, text::{normal_flow_text, preformatted_text}};

pub(super) const NODE: &str = "super-paste-files";
struct Files { files: Arc<[FilePreview]>, rows: Vec<Vec<BlockNode>> }
impl StructuredBlock for Files {
    fn visit_groups(&self, visitor: &mut dyn FnMut(&[BlockNode], &str)) {
        for row in &self.rows { visitor(row, "\n"); }
    }
}

pub(super) fn block(files: Arc<[FilePreview]>) -> BlockNode {
    let rows = files.iter().map(|file| {
        let mut blocks = vec![paragraph(&file.extension, FileText::Badge), paragraph(&file.name, FileText::Name)];
        if !file.directory.is_empty() { blocks.push(paragraph(&file.directory, FileText::Directory)); }
        blocks
    }).collect();
    BlockNode::Custom { node: MarkdownNode::structured(NODE, Files { files, rows }).accessibility_label("文件列表"), state: BlockState::default() }
}

#[derive(Clone, Copy)]
enum FileText { Badge, Name, Directory }

fn paragraph(text: &Arc<str>, kind: FileText) -> BlockNode {
    let source = SharedString::from(text.clone());
    let (source, policy) = match kind {
        // Keep badges on the native label path for actual ellipsis.
        FileText::Badge => (normal_flow_text(source), None),
        FileText::Name => (normal_flow_text(source), Some(TextSpacePolicy::Collapse)),
        FileText::Directory => (preformatted_text(source), Some(TextSpacePolicy::PreWrap)),
    };
    let (direction, alignment) = super::automatic_style(&source);
    let alignment = if matches!(kind, FileText::Badge) { TextAlign::Center } else { alignment };
    let length = source.len(); let mut inline = InlineNode::new(source);
    if let Some(policy) = policy.filter(|_| length > 0) {
        inline = inline.marks(vec![(0..length, TextMark { space_policy: Some(policy), ..TextMark::default() })]);
    }
    BlockNode::Paragraph(Paragraph::from_runs(vec![inline]).with_alignment(Some(alignment)).with_direction(Some(direction)))
}

pub(super) fn render(node: &MarkdownNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let Some(files) = node.data::<Files>() else { return div().into_any_element(); };
    let p = Palette::for_window(cx.global::<ConfigurationGlobal>().0.read(cx).projected.settings.color_mode, window);
    let context = |color| RenderContext::new(TextViewStyle::default().with_foreground(color).with_paragraph_gap(rems(0.0)));
    let primary = context(p.text_primary); let secondary = context(p.text_secondary); let symbol = context(p.type_teal);
    let mut list = div().w_full().min_w_0().flex().flex_col().gap(px(10.0)).text_size(px(14.0)).line_height(relative(1.65));
    for (index, (file, blocks)) in files.files.iter().zip(&files.rows).enumerate() {
        let path = SharedString::from(file.path.clone());
        let badge = div().w(px(40.0)).flex_shrink_0().flex().flex_col().items_center().text_color(p.type_teal)
            .child(icon("file-outline", 28.0))
            .child(div().w_full().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(9.0)).line_height(relative(1.3))
                .child(blocks[0].render(0, FlowPosition::at(blocks, 0, false), &symbol, window, cx)));
        let mut info = div().min_w_0();
        for (column, block) in blocks.iter().enumerate().skip(1) {
            info = info.child(div().min_w_0().overflow_wrap(OverflowWrap::Anywhere)
                .when(column == 1, |line| line.text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD))
                .when(column != 1, |line| line.text_size(px(11.0)).text_color(p.text_secondary))
                .child(block.render(column, FlowPosition::at(blocks, column, column + 1 == blocks.len()), if column == 1 { &primary } else { &secondary }, window, cx)));
        }
        let row = div().id(("file-row", index)).min_w_0().flex_shrink_0().py(px(8.0)).flex().items_center().gap(px(10.0))
            .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx)).child(badge).child(info);
        list = list.child(table_cell(row.into_any_element()));
    }
    list.into_any_element()
}
