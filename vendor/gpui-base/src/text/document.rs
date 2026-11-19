use gpui::{
    App, IntoElement, ListState, ParentElement as _, SharedString, Styled as _, Window, div,
};

use std::{
    ops::{Range, RangeInclusive},
    sync::Arc,
};

use crate::text::{
    SelectionFormat,
    node::{BlockNode, NodeContext, PlainCopyRange, PlainTextCopy, SelectionStyles, SourceRangeSelection},
    selection_adapter::SelectionBlockElement,
};
use crate::TextSelectionCoverage;

/// The parsed document AST.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ParsedDocument {
    pub(crate) source: SharedString,
    pub(crate) blocks: Arc<Vec<BlockNode>>,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct NodeRenderOptions {
    pub(crate) ix: usize,
    pub(crate) in_list: bool,
    pub(crate) todo: bool,
    pub(crate) ordered: bool,
    pub(crate) list_start: Option<u32>,
    pub(crate) depth: usize,
    pub(crate) is_last: bool,
    /// Structured block boxes own their margins; do not add Markdown gaps.
    pub(crate) authored_flow: bool,
    /// Whether this block opens its flow -- the document or a blockquote --
    /// and so takes no gap above it. An HTML block container passes it on to
    /// its own first child.
    pub(crate) is_first: bool,
    /// The previous sibling block, whose own bottom gap counts toward the
    /// gap above a heading or a rule.
    pub(crate) prev: PrevBlock,
}

/// The kind of block before the one being rendered, as far as the gaps of
/// headings and rules care.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PrevBlock {
    #[default]
    Other,
    /// A heading of the given level.
    Heading(u8),
    Rule,
}

/// Where the block at `ix` stands in its flow: whether it is the first block
/// that shows anything, and the visible block before it. Link definitions
/// and other blocks that render nothing are skipped, so a document opening
/// with `[ref]: url` still starts flush.
pub(crate) fn flow_position<B: std::borrow::Borrow<BlockNode>>(
    blocks: &[B],
    ix: usize,
) -> (bool, PrevBlock) {
    blocks[..ix].iter().rev().find_map(|block| flow_tail(block.borrow()))
        .map_or((true, PrevBlock::Other), |prev| (false, prev))
}

/// A paint/grouping container does not hide the last visible block from the
/// following sibling's gap calculation. Borrow the tree; do not flatten it.
fn flow_tail(block: &BlockNode) -> Option<PrevBlock> {
    match block {
        BlockNode::Root { children, .. } => children.iter().rev().find_map(flow_tail),
        BlockNode::Heading { level, .. } => Some(PrevBlock::Heading(*level)),
        BlockNode::HorizontalRule { .. } => Some(PrevBlock::Rule),
        BlockNode::Definition { .. } | BlockNode::Break { .. } | BlockNode::Unknown => None,
        _ => Some(PrevBlock::Other),
    }
}

impl NodeRenderOptions {
    pub(crate) fn is_last(mut self, is_last: bool) -> Self {
        self.is_last = is_last;
        self
    }
}

impl ParsedDocument {
    pub(super) fn text(&self) -> String {
        let mut text = PlainTextCopy::default();
        let mut selection = None;
        for block in self.blocks.iter() { block.write_plain_copy(&mut selection, &mut text); }
        text.finish()
    }

    /// The selected text across all blocks, in `format`.
    ///
    /// In [`SelectionFormat::Source`] each block reconstructs its own Markdown
    /// source (inline markup, and block prefixes for headings and lists), and
    /// top-level blocks are joined with a blank line so the result re-renders
    /// with the same block structure.
    ///
    /// Paint and synchronous Copy project the current gesture onto retained
    /// native layouts. In a scrollable (virtualized) view some covered blocks
    /// have never been laid out and report nothing. The selection is continuous,
    /// so its interior blocks are emitted whole rather than dropped. `blocks`
    /// bounds that span; it comes from the selection endpoints, which hold on
    /// to their block index even after scrolling out of view. Without it, fall
    /// back to the blocks with projected selections.
    ///
    /// Blocks with no local selection geometry are still included when
    /// enclosed by the selected source range. Explicit whole-document
    /// coverage includes leading and trailing objects as well.
    pub(super) fn selected_text(
        &self,
        format: SelectionFormat,
        blocks: Option<RangeInclusive<usize>>,
    ) -> String {
        self.selected_text_with_coverage(format, blocks, TextSelectionCoverage::Bounded, false)
    }

    /// A cross-participant prefix/suffix has just one partial endpoint block.
    /// Its other blocks are source-complete, independently of retained paint.
    /// Native partial endpoints, including a virtual endpoint awaiting layout,
    /// cannot use the legacy whole-block fallback for absent or zero-byte ranges.
    pub(super) fn selected_text_with_coverage(
        &self,
        format: SelectionFormat,
        blocks: Option<RangeInclusive<usize>>,
        coverage: TextSelectionCoverage,
        partial_endpoints: bool,
    ) -> String {
        if coverage == TextSelectionCoverage::Full {
            return if format == SelectionFormat::Plain {
                self.text()
            } else {
                self.source.to_string()
            };
        }
        if self.blocks.is_empty() {
            return String::new();
        }
        let (first, last) = match blocks {
            Some(blocks) if !blocks.is_empty() && *blocks.end() < self.blocks.len() => {
                (*blocks.start(), *blocks.end())
            },
            Some(_) => return String::new(),
            None if coverage != TextSelectionCoverage::Bounded => return String::new(),
            None => {
                let Some(first) = self.blocks.iter().position(BlockNode::has_selection) else {
                    return String::new();
                };
                let last = self.blocks.iter().rposition(BlockNode::has_selection).unwrap();
                (first, last)
            },
        };
        if coverage == TextSelectionCoverage::Bounded
            && (!partial_endpoints || last - first < 2)
            && !self.blocks[first..=last].iter().any(BlockNode::has_selection)
        {
            return String::new();
        }

        // Unresolved bounded gestures retain the virtual-block atomic policy.
        // Known native or open-ended endpoints are partial even at zero bytes.
        let whole_block = |ix: usize, observed: bool| match coverage {
            TextSelectionCoverage::Bounded => {
                (ix != first && ix != last) || (!partial_endpoints && !observed)
            },
            TextSelectionCoverage::FromStart => ix < last,
            TextSelectionCoverage::ToEnd => ix > first,
            TextSelectionCoverage::Full => unreachable!("full coverage was handled above"),
        };

        // A selection within one style copies as the text it renders; one
        // that is only partly painted cannot tell, and keeps its source.
        let format = match format {
            SelectionFormat::SourceAcrossStyles
                if first == last
                    && self.blocks[first].has_selection()
                    && self.blocks[first].selection_styles() != SelectionStyles::Mixed =>
            {
                SelectionFormat::Plain
            }
            SelectionFormat::SourceAcrossStyles => SelectionFormat::Source,
            format => format,
        };

        if format == SelectionFormat::Plain {
            let mut span = PlainCopyRange::default();
            for (ix, block) in self.blocks.iter().enumerate().take(last + 1).skip(first) {
                let whole = whole_block(ix, block.has_selection());
                block.scan_plain_copy(whole, &mut span);
            }
            if !span.rewind() { return String::new(); }
            let mut selection = Some(span);
            let mut text = PlainTextCopy::default();
            for block in self.blocks.iter().take(last + 1).skip(first) {
                block.write_plain_copy(&mut selection, &mut text);
            }
            return text.finish();
        }

        let mut out = String::new();
        for (ix, block) in self.blocks.iter().enumerate().take(last + 1).skip(first) {
            let observed = block.has_selection();
            let source = if whole_block(ix, observed) {
                self.whole_source(block)
            } else if observed {
                block.selected_text(format)
            } else {
                continue;
            };

            let trimmed = source.trim_end_matches('\n');
            if !trimmed.is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(trimmed);
            }
        }
        out
    }

    /// The whole source of a block the selection covers.
    ///
    /// Copied straight out of the original text, which the Markdown parser
    /// locates per block. That keeps whatever the author wrote — `_italic_`
    /// stays `_italic_`, a reference link keeps its `[ref]` form, a table keeps
    /// its column padding — and it needs no rule of its own per block type.
    /// Blocks the parser could not locate fall back to reconstruction.
    fn whole_source(&self, block: &BlockNode) -> String {
        if let Some(span) = block.span()
            && let Some(source) = self.source.get(span.start..span.end)
        {
            return source.to_string();
        }

        block.selected_text(SelectionFormat::Source)
    }

    pub(super) fn selected_source_range(&self) -> Option<Range<usize>> {
        let mut selected = SourceRangeSelection::Unselected;
        for block in self.blocks.iter() {
            selected.merge(block.selected_source_range());
        }
        selected.into_range()
    }

    /// Synchronously clear the selection stored in every inline state.
    ///
    /// This mirrors the [`selected_text`](Self::selected_text) traversal so the
    /// stored selection can be cleared without relying on a repaint. Offscreen
    /// (virtualized) views do not repaint, so their `InlineState.selection`
    /// would otherwise retain stale values from the last painted frame.
    pub(super) fn clear_selection(&self) {
        for block in self.blocks.iter() {
            block.clear_selection();
        }
    }

    /// Converts the node to markdown format.
    ///
    /// This is used to generate markdown for test.
    #[allow(dead_code)]
    pub(crate) fn to_markdown(&self) -> String {
        self.blocks
            .iter()
            .map(|child| child.to_markdown())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub(super) fn render_root(
        &self,
        list_state: Option<ListState>,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let Some(list_state) = list_state else {
            let blocks_len = self.blocks.len();
            return div().children(self.blocks.iter().enumerate().map(move |(ix, node)| {
                let is_last = ix + 1 == blocks_len;
                let (is_first, prev) = flow_position(&self.blocks, ix);
                SelectionBlockElement(node.render_block(
                    NodeRenderOptions {
                        ix,
                        authored_flow: node_cx.style.authored_flow(),
                        is_last,
                        is_first,
                        prev,
                        ..Default::default()
                    },
                    node_cx,
                    window,
                    cx,
                ).into_any_element(), ix)
            }));
        };

        let options = NodeRenderOptions {
            is_last: true,
            authored_flow: node_cx.style.authored_flow(),
            ..Default::default()
        };

        let blocks = &self.blocks;
        if list_state.item_count() != blocks.len() {
            list_state.reset(blocks.len());
        }

        div().size_full().child(
            gpui::list(list_state, {
                let node_cx = node_cx.clone();
                let blocks = blocks.clone();
                move |ix, window, cx| {
                    let is_last = ix + 1 == blocks.len();
                    let (is_first, prev) = flow_position(&blocks, ix);
                    SelectionBlockElement(blocks[ix]
                        .render_block(
                            NodeRenderOptions {
                                ix,
                                is_last,
                                is_first,
                                prev,
                                ..options
                            },
                            &node_cx,
                            window,
                            cx,
                        )
                        .into_any_element(), ix).into_any_element()
                }
            })
            .size_full(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{PrevBlock, flow_position};
    use crate::text::{format::markdown, node::NodeContext};

    fn positions(source: &str) -> Vec<(bool, PrevBlock)> {
        let document = markdown::parse(source, &mut NodeContext::default()).unwrap();
        (0..document.blocks.len())
            .map(|ix| flow_position(&document.blocks, ix))
            .collect()
    }

    #[test]
    fn flow_position_skips_blocks_that_render_nothing() {
        // The definition renders nothing, so the heading still opens the flow.
        let flow = positions("[ref]: https://example.com\n\n# Title\n\ntext");
        assert_eq!(flow[1], (true, PrevBlock::Other));
        assert_eq!(flow[2], (false, PrevBlock::Heading(1)));
    }

    #[test]
    fn flow_position_reports_the_previous_heading_or_rule() {
        let flow = positions("text\n\n## A\n\n---\n\n### B");
        assert_eq!(
            flow,
            [
                (true, PrevBlock::Other),
                (false, PrevBlock::Other),
                (false, PrevBlock::Heading(2)),
                (false, PrevBlock::Rule),
            ]
        );
    }
}
