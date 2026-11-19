use crate::input::InputModeKind;
use gpui::{App, CaretAffinity, Context, Pixels, TextHit, Window, px};
use sum_tree::Bias;

use crate::input::{
    InputBaseState, MoveDown, MoveEnd, MoveHome, MoveLeft, MovePageDown, MovePageUp, MoveRight,
    MoveToEnd, MoveToNextWord, MoveToPreviousWord, MoveToStart, MoveUp, RopeExt as _,
    cursor::CursorSelection,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveDirection {
    Up,
    Down,
}

// Keep the usual borrowed row allocation-free; only offscreen rows own text.
#[allow(clippy::large_enum_variant)]
enum NavigationLine<'a> {
    Painted(&'a crate::input::display_map::InputLine),
    Temporary(crate::input::display_map::InputLine),
}

impl std::ops::Deref for NavigationLine<'_> {
    type Target = crate::input::display_map::InputLine;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Painted(line) => line,
            Self::Temporary(line) => line,
        }
    }
}

struct NavigationRow<'a> {
    line: NavigationLine<'a>,
    display_start: usize,
    is_last: bool,
}

impl<M: InputModeKind> InputBaseState<M> {
    /// Repair a missing UI/font recipe after releasing the entity borrow, in
    /// this event's effect cycle. Token presenters may read the input while
    /// measuring, so drawing from inside its action handler is not safe.
    /// Resume the focus handle's bubble chain rather than calling the handler:
    /// parent propagation survives and capture listeners do not run twice.
    pub(super) fn defer_navigation_for_layout(
        &self,
        action: &dyn gpui::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // Empty source has a single stop, independent of font/token metrics.
        if self.text.len() == 0 || self.navigation_recipe().is_some() {
            return false;
        }
        let input = cx.weak_entity();
        let focus = self.focus_handle.clone();
        let revision = self.document_revision;
        let active = self.active_selection().id;
        let selections: smallvec::SmallVec<[CursorSelection; 1]> =
            self.selections.iter().copied().collect();
        let action = action.boxed_clone();
        cx.notify();
        window.defer(cx, move |window, cx| {
            let Some(input) = input.upgrade() else { return; };
            let valid = |state: &Self, window: &Window| {
                !state.disabled && focus.is_focused(window)
                    && state.document_revision == revision
                    && state.active_selection().id == active
                    && state.selections.iter().eq(selections.iter())
            };
            let needs_layout = {
                let state = input.read(cx);
                if !valid(state, window) {
                    return;
                }
                state.navigation_recipe().is_none()
            };

            // A cold/masked/token recipe needs real UI measurement. Draw once
            // through GPUI's normal pipeline; do not keep a second font/token
            // cache or a queue that can replay after later platform input.
            if needs_layout {
                window.refresh();
                window.draw(cx).clear(cx);
            }
            {
                let state = input.read(cx);
                if !valid(state, window) {
                    return;
                }
                if state.navigation_recipe().is_none() {
                    // An absent/hidden input cannot be repaired by repeatedly
                    // drawing. Do not turn that into source-order navigation.
                    tracing::warn!("native input navigation has no measured layout after redraw");
                    return;
                }
            }
            focus.resume_action_after_capture(action.as_ref(), window, cx);
        });
        true
    }

    /// Compute the column anchor for the given `offset`. Wrap/fold-aware.
    pub(super) fn preferred_column_for(&self, offset: usize) -> Option<(Pixels, usize)> {
        self.preferred_column_for_with_affinity(offset, false)
    }

    /// Like [`Self::preferred_column_for`], but resolves an offset on a soft wrap
    /// boundary to the row the caret is drawn on.
    pub(super) fn preferred_column_for_with_affinity(
        &self,
        offset: usize,
        line_end_affinity: bool,
    ) -> Option<(Pixels, usize)> {
        self.preferred_column_for_caret(TextHit { index: offset, affinity: self.caret_affinity_at(offset) }, line_end_affinity)
    }

    pub(super) fn preferred_column_for_caret(
        &self,
        hit: TextHit,
        line_end_affinity: bool,
    ) -> Option<(Pixels, usize)> {
        let last_layout = self.painted_navigation_layout()?;
        let point = self.text.offset_to_point(hit.index);
        let line = last_layout.line(point.row)?;
        let local_index = self.display_index(hit.index) - self.display_index(self.text.line_start_offset(point.row));
        let pos = line.position_for_caret(TextHit { index: local_index, affinity: if self.masked { CaretAffinity::Downstream } else { hit.affinity } }, last_layout, line_end_affinity)?;
        Some((pos.x, point.column))
    }

    /// Share painted geometry when possible. Cold alignment retains the target
    /// row and at most one other native row, never a hard-line layout cache.
    fn navigation_row_geometry<'a>(
        &'a self,
        buffer_row: usize,
        local_row: usize,
        window: &mut Window,
        cx: &App,
    ) -> Option<(NavigationRow<'a>, Pixels)> {
        let row = self.navigation_row(buffer_row, local_row, window, cx)?;
        if let Some(layout) = self.painted_navigation_layout() {
            if let Some(line) = layout.line(buffer_row) {
                let indent = if local_row > 0 { line.wrap_indent } else { px(0.) };
                return Some((row, layout.alignment_offset(line.longest_width) + indent));
            }
        }

        let layout = self.navigation_recipe()?;
        let item = self.display_map.line(buffer_row)?;
        let all_rows = layout.text_align != gpui::TextAlign::Left;
        let indent_len = if local_row > 0 && item.indent > 0 {
            let source_len: usize = self.text.slice_line(buffer_row).chars()
                .take(item.indent as usize).map(char::len_utf8).sum();
            let start = self.text.line_start_offset(buffer_row);
            self.display_index(start + source_len) - self.display_index(start)
        } else {
            0
        };
        let mut indent = px(0.);
        let mut longest = row.line.width;
        let rows = if all_rows {
            0..item.wrapped_lines.len()
        } else {
            0..usize::from(indent_len > 0)
        };
        for other_row in rows {
            if other_row == local_row {
                continue;
            }
            let other = self.navigation_row(buffer_row, other_row, window, cx)?;
            longest = longest.max(other.line.width);
            if other_row == 0 && indent_len > 0 {
                indent = other.line.x_for_index(indent_len);
            }
        }
        Some((row, layout.alignment_offset(longest) + indent))
    }

    /// Resolve a fresh column using the same origin as native row hit-testing.
    fn navigation_column_for_caret(
        &self,
        hit: TextHit,
        line_end_affinity: bool,
        window: &mut Window,
        cx: &App,
    ) -> Option<(Pixels, usize)> {
        if let Some(column) = self.preferred_column_for_caret(hit, line_end_affinity) {
            return Some(column);
        }
        let layout = self.navigation_recipe()?;
        if self.text.len() == 0 {
            let affinity = if self.masked { CaretAffinity::Downstream } else { hit.affinity };
            return Some((super::element::TextElement::<M>::placeholder_column(
                self, layout, affinity, window,
            ), 0));
        }
        let point = self.text.offset_to_point(hit.index);
        let local_row = self.display_map.offset_to_wrap_display_point_with_affinity(
            hit.index, line_end_affinity,
        ).local_row;
        let (row, origin) = self.navigation_row_geometry(point.row, local_row, window, cx)?;
        let index = self.display_index(hit.index).checked_sub(row.display_start)?;
        if index > row.line.len {
            return None;
        }
        let affinity = if self.masked { CaretAffinity::Downstream } else { hit.affinity };
        Some((row.line.x_for_index_with_affinity(index, affinity) + origin, point.column))
    }

    /// Soft-row ownership travels with each head, independently of bidi affinity.
    pub(super) fn line_end_affinity_for(&self, sel: &CursorSelection) -> bool {
        sel.line_end_affinity()
    }

    /// The line-end affinity for a cursor known only by its offset. Cursors never share an
    /// offset, so a matching selection owns the visual side at this position.
    pub(super) fn line_end_affinity_at(&self, offset: usize) -> bool {
        self.selections.iter().find(|sel| sel.cursor_offset() == offset)
            .is_some_and(|sel| sel.line_end_affinity())
    }

    pub(super) fn caret_affinity_at(&self, offset: usize) -> CaretAffinity {
        if self.masked {
            return CaretAffinity::Downstream;
        }
        let active = self.active_selection();
        if active.cursor_offset() == offset {
            return active.caret().affinity;
        }
        self.selections.iter().find(|sel| sel.cursor_offset() == offset)
            .map_or(CaretAffinity::Downstream, |sel| sel.caret().affinity)
    }

    /// A source-order move arrives on the side of the grapheme it crossed.
    /// Bidi affinity and soft-row ownership are independent: the former is
    /// retained even when the destination row is outside the painted viewport.
    pub(super) fn directional_caret(&self, offset: usize, forward: bool) -> (TextHit, bool) {
        let hit = TextHit {
            index: offset,
            affinity: if forward {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            },
        };
        let line_end = if forward && self.soft_wrap && !self.is_single_line() {
            let point = self.text.offset_to_point(offset);
            self.display_map.line(point.row).is_some_and(|line| {
                let row = line.wrapped_lines.partition_point(|range| range.end < point.column);
                row + 1 < line.wrapped_lines.len()
                    && line.wrapped_lines[row].end == point.column
            })
        } else {
            false
        };
        (hit, line_end)
    }

    pub(super) fn horizontal_target(
        &self,
        sel: &CursorSelection,
        right: bool,
        extend: bool,
        window: &mut Window,
        cx: &App,
    ) -> (TextHit, bool) {
        if !extend && !sel.is_empty() {
            if let Some(target) = self.visual_selection_edge(sel, right, window, cx) { return target; }
        } else if let Some(target) = self.visual_horizontal_target(sel, right, window, cx) {
            return target;
        }
        let offset = if !extend && !sel.is_empty() {
            self.grapheme_boundary(
                if right { sel.end } else { sel.start },
                if right { Bias::Right } else { Bias::Left },
            )
        } else if right {
            self.next_boundary(sel.cursor_offset())
        } else {
            self.previous_boundary(sel.cursor_offset())
        };
        if offset == sel.cursor_offset() && (extend || sel.is_empty()) {
            return (sel.caret(), self.line_end_affinity_for(sel));
        }
        self.directional_caret(offset, right)
    }

    fn valid_visual_stop(&self, display: usize) -> bool {
        let offset = self.resolve_index(display);
        self.display_index(offset) == display
            && self.cursor_boundary(offset, Bias::Left) == offset
            && self.grapheme_boundary(offset, Bias::Left) == offset
    }

    /// Resolve every head before mutating selections or borrowing the UI
    /// context for their shared lifecycle. The iterator order is preserved.
    pub(super) fn horizontal_targets(
        &self,
        right: bool,
        extend: bool,
        window: &mut Window,
        cx: &App,
    ) -> smallvec::SmallVec<[(TextHit, Option<(Pixels, usize)>, bool); 1]> {
        self.selections.iter()
            .map(|sel| {
                let (hit, line_end) = self.horizontal_target(sel, right, extend, window, cx);
                let column = self.navigation_column_for_caret(hit, line_end, window, cx);
                (hit, column, line_end)
            })
            .collect()
    }

    /// Its font recipe can outlive the rows. Borrow native rows only through
    /// `painted_navigation_layout`, which also checks their geometry inputs.
    fn navigation_recipe(&self) -> Option<&super::layout::LastLayout> {
        self.last_layout.as_ref().filter(|layout| {
            // The last painted font is still the navigation recipe after an
            // edit. Token metrics, unlike ordinary text, need a UI measurement
            // before the current source ranges can be trusted for wrapping.
            let Some(generation) = layout.font_generation.as_ref() else {
                return false;
            };
            generation.is_current()
                && layout.masked == self.masked
                && layout.tokens_visible == self.tokens_visible()
                && (!self.tokens_visible()
                    || self.token_layout_cache.as_ref().is_some_and(|cache| {
                        cache.revision == self.document_revision
                            && cache.key.as_ref().is_some_and(|key| {
                                key.direction == layout.text_style.text_direction
                                    && key.font_generation == generation.generation()
                            })
                    }))
        })
    }

    fn painted_navigation_layout(&self) -> Option<&super::layout::LastLayout> {
        self.navigation_recipe().filter(|layout| {
            layout.document_revision == self.document_revision
                && layout.font_generation.as_ref().is_some_and(|generation| {
                    self.display_map.matches_layout(
                        &layout.text_style.font(),
                        layout.font_size,
                        layout.wrap_width,
                        layout.wrapping_indent,
                        layout.tab_size,
                        layout.text_style.text_direction,
                        generation.generation(),
                    )
                })
        })
    }

    fn navigation_row<'a>(
        &'a self,
        buffer_row: usize,
        local_row: usize,
        window: &mut Window,
        cx: &App,
    ) -> Option<NavigationRow<'a>> {
        let layout = self.navigation_recipe()?;
        let item = self.display_map.line(buffer_row)?;
        let range = item.wrapped_lines.get(local_row)?;
        let display_start = self.display_index(self.text.line_start_offset(buffer_row) + range.start);
        let painted = self.painted_navigation_layout()
            .and_then(|layout| layout.line(buffer_row))
            .and_then(|line| line.wrapped_lines.get(local_row));
        let line = match painted {
            Some(line) => NavigationLine::Painted(line),
            None => NavigationLine::Temporary(super::element::TextElement::<M>::shape_navigation_row(
                self, buffer_row, local_row, layout, window, cx,
            )?),
        };
        Some(NavigationRow { line, display_start, is_last: local_row + 1 == item.wrapped_lines.len() })
    }

    fn resolve_navigation_caret(&self, row: &NavigationRow<'_>, mut hit: TextHit) -> (TextHit, bool) {
        let soft = !row.is_last && hit.index == row.line.len;
        hit.index = self.resolve_index(row.display_start + hit.index);
        (hit, soft)
    }

    fn visual_selection_edge(
        &self,
        sel: &CursorSelection,
        right: bool,
        window: &mut Window,
        cx: &App,
    ) -> Option<(TextHit, bool)> {
        self.navigation_recipe()?;
        let start = self.directional_caret(
            self.clamp_offset_to_visible_backward(self.grapheme_boundary(sel.start, Bias::Left)), false,
        );
        let end = self.directional_caret(
            self.clamp_offset_to_visible_forward(self.grapheme_boundary(sel.end, Bias::Right)), true,
        );
        let a = self.display_map.offset_to_wrap_display_point_with_affinity(start.0.index, start.1);
        let b = self.display_map.offset_to_wrap_display_point_with_affinity(end.0.index, end.1);
        // Distinct rows need no shaping or viewport geometry to compare.
        let end_is_right = if a.row != b.row {
            b.row > a.row
        } else {
            let buffer_row = self.text.offset_to_point(start.0.index).row;
            let row = self.navigation_row(buffer_row, a.local_row, window, cx)?;
            let a_index = self.display_index(start.0.index).checked_sub(row.display_start)?;
            let b_index = self.display_index(end.0.index).checked_sub(row.display_start)?;
            row.line.x_for_index_with_affinity(b_index, end.0.affinity)
                >= row.line.x_for_index_with_affinity(a_index, start.0.affinity)
        };
        Some(if right == end_is_right { end } else { start })
    }

    fn visual_horizontal_target(
        &self,
        sel: &CursorSelection,
        right: bool,
        window: &mut Window,
        cx: &App,
    ) -> Option<(TextHit, bool)> {
        if self.text.len() == 0 { return None; }
        let offset = sel.cursor_offset();
        let current = self.display_map.offset_to_wrap_display_point_with_affinity(
            offset, self.line_end_affinity_for(sel),
        );
        let point = self.text.offset_to_point(offset);
        let row = self.navigation_row(point.row, current.local_row, window, cx)?;
        let mut hit = TextHit {
            index: self.display_index(offset).checked_sub(row.display_start)?,
            affinity: sel.caret().affinity,
        };
        if hit.index > row.line.len { return None; }
        if let Some(hit) = row.line.adjacent_visual_caret(
            hit, right, |index| self.valid_visual_stop(row.display_start + index),
        ) {
            return Some(self.resolve_navigation_caret(&row, hit));
        }
        // A native shaper can omit leading control characters. Drawing maps
        // their source offsets to the first logical cluster's leading caret;
        // retry from that same owner instead of navigating in source order.
        // Keep the selection's original source anchor and resolve only this
        // temporary navigation hit. Successful ordinary moves skip this scan.
        let resolved = row.line.resolve_visual_caret(hit)?;
        if resolved != hit {
            hit = resolved;
            if let Some(hit) = row.line.adjacent_visual_caret(
                hit, right, |index| self.valid_visual_stop(row.display_start + index),
            ) {
                return Some(self.resolve_navigation_caret(&row, hit));
            }
        }
        // Missing native control glyphs do not prove that a row edge was reached.
        row.line.visual_edge_caret(right, |index| self.valid_visual_stop(row.display_start + index))
            .filter(|edge| edge.index == hit.index
                && (edge.affinity == hit.affinity
                    || row.line.x_for_index_with_affinity(edge.index, edge.affinity)
                        == row.line.x_for_index_with_affinity(hit.index, hit.affinity)))?;

        let rtl = row.line.direction == gpui::TextDirection::Rtl;
        let forward = right != rtl;
        drop(row);
        let display_row = self.display_map.wrap_row_to_display_row(current.row)
            .unwrap_or_else(|| self.display_map.nearest_visible_display_row(current.row));
        let target_row = if forward {
            display_row.checked_add(1).filter(|row| *row < self.display_map.display_row_count())
        } else {
            display_row.checked_sub(1)
        };
        let Some(target_row) = target_row else {
            return Some((sel.caret(), self.line_end_affinity_for(sel)));
        };
        let mut target = current;
        target.row = self.display_map.display_row_to_wrap_row(target_row)?;
        target.column = 0;
        let target_offset = self.display_map.wrap_display_point_to_offset(target);
        let target = self.display_map.offset_to_wrap_display_point(target_offset);
        let point = self.text.offset_to_point(target_offset);
        let row = self.navigation_row(point.row, target.local_row, window, cx)?;
        let rtl = row.line.direction == gpui::TextDirection::Rtl;
        let mut hit = row.line.visual_edge_caret(
            if forward { rtl } else { !rtl },
            |index| self.valid_visual_stop(row.display_start + index),
        )?;
        // Do not add a second key stop at the two sides of one soft wrap.
        if self.resolve_index(row.display_start + hit.index) == offset {
            hit = row.line.adjacent_visual_caret(
                hit, forward != rtl, |index| self.valid_visual_stop(row.display_start + index),
            ).unwrap_or(hit);
        }
        Some(self.resolve_navigation_caret(&row, hit))
    }

    pub(super) fn word_target(&self, sel: &CursorSelection, forward: bool) -> (TextHit, bool) {
        let offset = if forward {
            self.next_end_of_word_at(sel.cursor_offset())
        } else {
            self.previous_start_of_word_at(sel.cursor_offset())
        };
        if offset == sel.cursor_offset() {
            (sel.caret(), self.line_end_affinity_for(sel))
        } else {
            self.directional_caret(offset, forward)
        }
    }

    /// Called after moving the cursor. Updates the active selection's
    /// `column_anchor` if we know where the cursor now is.
    pub(super) fn update_preferred_column(&mut self) {
        let anchor =
            self.preferred_column_for_with_affinity(self.cursor(), self.line_end_affinity_at(self.cursor()));
        self.active_selection_mut().column_anchor = anchor;
    }

    /// Move the cursor to the given offset.
    ///
    /// The offset is the UTF-8 offset.
    ///
    /// Ensure the offset use self.next_boundary or self.previous_boundary to get the correct offset.
    pub(crate) fn move_to(
        &mut self,
        offset: usize,
        direction: Option<MoveDirection>,
        cx: &mut Context<Self>,
    ) {
        self.move_to_with_affinity(offset, direction, false, cx);
    }

    /// Like [`Self::move_to`], but also carries the caret's line-end affinity.
    ///
    /// A soft wrap boundary is one offset shared by the end of one visual line and the start of
    /// the next, so the offset alone cannot say where to draw the caret. Callers that resolved
    /// the offset from a visual position -- a click, a drag, a vertical move -- already know
    /// which of the two rows the user meant, and pass it here. Taking it in the same call as the
    /// move is what keeps the two from drifting apart.
    pub(crate) fn move_to_with_affinity(
        &mut self,
        offset: usize,
        direction: Option<MoveDirection>,
        line_end_affinity: bool,
        cx: &mut Context<Self>,
    ) {
        self.move_to_caret(TextHit { index: offset, affinity: CaretAffinity::Downstream }, direction, line_end_affinity, cx);
    }

    pub(super) fn move_to_caret(
        &mut self,
        hit: TextHit,
        direction: Option<MoveDirection>,
        line_end_affinity: bool,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        self.selections.remove_all_but_active();
        let offset = self.cursor_boundary(hit.index, if hit.affinity == CaretAffinity::Upstream { Bias::Right } else { Bias::Left });
        self.set_cursor_to(offset);
        self.active_selection_mut().set_caret(
            if offset == hit.index { hit.affinity } else { CaretAffinity::Downstream },
            line_end_affinity && offset == hit.index,
        );
        self.scroll_to(offset, direction, cx);
        self.pause_blink_cursor(cx);
        self.update_preferred_column();
        M::hide_context_menu(self, cx);
        M::clear_inline_completion(self, cx);
        cx.notify()
    }

    /// Move through visible wrap rows using native glyph geometry, including
    /// rows outside the last viewport. A missing measurement leaves the caret
    /// in place instead of interpreting a byte column as a visual position.
    fn vertical_target(
        &self,
        source: TextHit,
        column_anchor: Option<(Pixels, usize)>,
        line_end_affinity: bool,
        move_lines: isize,
        window: &mut Window,
        cx: &App,
    ) -> (TextHit, bool) {
        let unchanged = (source, line_end_affinity);
        if self.navigation_recipe().is_none() {
            return unchanged;
        }
        let mut point = self.display_map.offset_to_wrap_display_point_with_affinity(
            source.index, line_end_affinity,
        );
        let current_row = self.display_map.wrap_row_to_display_row(point.row)
            .unwrap_or_else(|| self.display_map.nearest_visible_display_row(point.row));
        let max_row = self.display_map.display_row_count().saturating_sub(1);
        let target_row = current_row.saturating_add_signed(move_lines).min(max_row);
        if target_row == current_row {
            return unchanged;
        }
        let Some((preferred_x, _)) = column_anchor else {
            return unchanged;
        };
        let Some(wrap_row) = self.display_map.display_row_to_wrap_row(target_row) else {
            return unchanged;
        };
        point.row = wrap_row;
        point.column = 0;
        let offset = self.display_map.wrap_display_point_to_offset(point);
        let buffer_row = self.text.offset_to_point(offset).row;
        let local_row = self.display_map.offset_to_wrap_display_point(offset).local_row;
        let Some((row, origin)) = self.navigation_row_geometry(buffer_row, local_row, window, cx) else {
            return unchanged;
        };
        let native = row.line.closest_caret_for_x(preferred_x - origin);
        let (mut hit, _) = self.resolve_navigation_caret(&row, native);
        // Native cluster stops can sit inside a source grapheme (notably the
        // CR/LF boundary). Normalization must stay on the target visual row.
        let start = self.grapheme_boundary(self.resolve_index(row.display_start), Bias::Right);
        let end = self.grapheme_boundary(
            self.resolve_index(row.display_start + row.line.len), Bias::Left,
        );
        if start > end {
            return unchanged;
        }
        hit.index = self.grapheme_boundary(
            hit.index,
            if hit.affinity == CaretAffinity::Upstream { Bias::Right } else { Bias::Left },
        ).clamp(start, end);
        let soft = !row.is_last && self.display_index(hit.index) == row.display_start + row.line.len;
        (hit, soft)
    }

    /// Extend to the source edge only when there is no further visible row.
    /// Failed/pending native measurements must not select the rest of the file.
    fn vertical_selection_target(
        &self,
        source: TextHit,
        column_anchor: Option<(Pixels, usize)>,
        line_end_affinity: bool,
        move_lines: isize,
        window: &mut Window,
        cx: &App,
    ) -> (TextHit, bool) {
        if self.navigation_recipe().is_some() {
            let point = self.display_map.offset_to_wrap_display_point_with_affinity(
                source.index, line_end_affinity,
            );
            let row = self.display_map.wrap_row_to_display_row(point.row)
                .unwrap_or_else(|| self.display_map.nearest_visible_display_row(point.row));
            let last = self.display_map.display_row_count().saturating_sub(1);
            if move_lines < 0 && row == 0 {
                return self.directional_caret(0, false);
            }
            if move_lines > 0 && row == last {
                return self.directional_caret(self.text.len(), true);
            }
        }
        self.vertical_target(source, column_anchor, line_end_affinity, move_lines, window, cx)
    }

    /// Resolve all native measurements before mutating selections or entering
    /// their shared lifecycle. One cursor stays in SmallVec's inline storage.
    pub(super) fn vertical_targets(
        &self,
        move_lines: isize,
        collapse: bool,
        extend: bool,
        window: &mut Window,
        cx: &App,
    ) -> smallvec::SmallVec<[(TextHit, Option<(Pixels, usize)>, bool); 1]> {
        self.selections.iter().map(|sel| {
            let (source, anchor, soft) = if sel.is_empty() || !collapse {
                (sel.caret(), sel.column_anchor, sel.line_end_affinity())
            } else if move_lines < 0 {
                (TextHit { index: sel.start, affinity: CaretAffinity::Downstream }, None, false)
            } else {
                (TextHit { index: sel.end, affinity: CaretAffinity::Upstream }, None, true)
            };
            let anchor = anchor.or_else(|| self.navigation_column_for_caret(source, soft, window, cx));
            let (hit, soft) = if extend {
                self.vertical_selection_target(source, anchor, soft, move_lines, window, cx)
            } else {
                self.vertical_target(source, anchor, soft, move_lines, window, cx)
            };
            (hit, anchor, soft)
        }).collect()
    }

    /// Move every cursor through `f`, which maps each selection to a
    /// `(new_offset, column_anchor, line_end_affinity)`, collapsing each to a
    /// cursor. Overlapping cursors are merged, then the standard post-move
    /// sequence runs. Each cursor retains its own bidi and wrap-row affinity.
    pub(super) fn move_all_cursors(
        &mut self,
        f: impl Fn(&Self, &CursorSelection) -> (usize, Option<(Pixels, usize)>, bool),
        direction: Option<MoveDirection>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_all_cursors_to_carets(|state, sel| {
            let (offset, anchor, affinity) = f(state, sel);
            (TextHit { index: offset, affinity: CaretAffinity::Downstream }, anchor, affinity)
        }, direction, _window, cx);
    }

    pub(super) fn move_all_cursors_to_carets(
        &mut self,
        mut f: impl FnMut(&Self, &CursorSelection) -> (TextHit, Option<(Pixels, usize)>, bool),
        direction: Option<MoveDirection>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        let new_selections: Vec<CursorSelection> = self
            .selections
            .iter()
            .map(|sel| {
                let (hit, anchor, line_end_affinity) = f(self, sel);
                let mut new_sel = *sel;
                new_sel.place_at(self.cursor_boundary(hit.index, if hit.affinity == CaretAffinity::Upstream { Bias::Right } else { Bias::Left }), anchor);
                let at_hit = new_sel.cursor_offset() == hit.index;
                if !at_hit {
                    new_sel.column_anchor = None;
                }
                new_sel.set_caret(
                    if at_hit { hit.affinity } else { CaretAffinity::Downstream },
                    line_end_affinity && at_hit,
                );
                new_sel
            })
            .collect();
        self.selections.replace_all(new_selections);
        self.selections.merge_overlapping();

        self.scroll_to(self.cursor(), direction, cx);
        self.pause_blink_cursor(cx);
        M::hide_context_menu(self, cx);
        M::clear_inline_completion(self, cx);
        cx.notify();
    }

    /// Move every cursor vertically by `move_lines`.
    ///
    /// When `collapse` is set, a non-empty selection first collapses to its
    /// start (up) or end (down) before moving, otherwise the cursor
    /// offset is used.
    fn move_vertical(
        &mut self,
        move_lines: isize,
        collapse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_single_line() {
            return;
        }
        self.pause_blink_cursor(cx);

        let direction = if move_lines < 0 {
            MoveDirection::Up
        } else {
            MoveDirection::Down
        };

        let targets = self.vertical_targets(move_lines, collapse, false, window, cx);
        let mut targets = targets.into_iter();
        self.move_all_cursors_to_carets(
            |_, _| targets.next().expect("one vertical target per original cursor"),
            Some(direction),
            window,
            cx,
        );
    }

    pub(super) fn left(&mut self, action: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_navigation_for_layout(action, window, cx) { return; }
        self.move_horizontal(false, window, cx);
    }

    pub(super) fn right(&mut self, action: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_navigation_for_layout(action, window, cx) { return; }
        self.move_horizontal(true, window, cx);
    }

    fn move_horizontal(&mut self, right: bool, window: &mut Window, cx: &mut Context<Self>) {
        let active = *self.active_selection();
        let targets = self.horizontal_targets(right, false, window, cx);
        let active_index = self.selections.iter().position(|sel| sel.id == active.id)
            .expect("active selection is present");
        let target = targets[active_index];
        // Source 0/len need not be a physical edge in bidi text. Only an
        // unchanged native insertion lets a lone empty cursor propagate.
        if self.selections.is_single() && active.is_empty()
            && (target.0, target.2) == (active.caret(), self.line_end_affinity_for(&active)) {
            cx.propagate();
            return;
        }
        let mut targets = targets.into_iter();
        self.move_all_cursors_to_carets(
            |_, _| targets.next().expect("one target per selection"),
            None,
            window,
            cx,
        );
    }

    pub(super) fn up(&mut self, action: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        if M::handle_context_menu_action(self, Box::new(action.clone()), window, cx) {
            return;
        }

        if self.is_single_line() { return; }
        if self.defer_navigation_for_layout(action, window, cx) { return; }
        self.move_vertical(-1, true, window, cx);
    }

    pub(super) fn down(&mut self, action: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        if M::handle_context_menu_action(self, Box::new(action.clone()), window, cx) {
            return;
        }

        if self.is_single_line() { return; }
        if self.defer_navigation_for_layout(action, window, cx) { return; }
        self.move_vertical(1, true, window, cx);
    }

    pub(super) fn page_up(&mut self, action: &MovePageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_single_line() {
            return;
        }
        if self.defer_navigation_for_layout(action, window, cx) { return; }

        let Some(last_layout) = &self.last_layout else {
            return;
        };

        let display_lines = (self.input_bounds.size.height / last_layout.line_height) as isize;
        self.move_vertical(-display_lines, false, window, cx);
    }

    pub(super) fn page_down(
        &mut self,
        action: &MovePageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_single_line() {
            return;
        }
        if self.defer_navigation_for_layout(action, window, cx) { return; }

        let Some(last_layout) = &self.last_layout else {
            return;
        };

        let display_lines = (self.input_bounds.size.height / last_layout.line_height) as isize;
        self.move_vertical(display_lines, false, window, cx);
    }

    pub(super) fn home(&mut self, _: &MoveHome, window: &mut Window, cx: &mut Context<Self>) {
        self.move_all_cursors(
            |s, sel| {
                let offset = s.start_of_line_at(sel.cursor_offset(), s.line_end_affinity_for(sel));
                (offset, s.preferred_column_for(offset), false)
            },
            Some(MoveDirection::Up),
            window,
            cx,
        );
    }

    pub(super) fn end(&mut self, _: &MoveEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.move_all_cursors_to_carets(
            |s, sel| {
                let offset = s.end_of_line_at(sel.cursor_offset(), s.line_end_affinity_for(sel));
                // The caret belongs at the end of the visual row it is on.
                let hit = s.directional_caret(offset, true).0;
                (hit, s.preferred_column_for_caret(hit, true), true)
            },
            Some(MoveDirection::Down),
            window,
            cx,
        );
    }

    pub(super) fn move_to_start(
        &mut self,
        _: &MoveToStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(0, None, cx);
    }

    pub(super) fn move_to_end(
        &mut self,
        _: &MoveToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (hit, line_end) = self.directional_caret(self.text.len(), true);
        self.move_to_caret(hit, None, line_end, cx);
    }

    pub(super) fn move_to_previous_word(
        &mut self,
        _: &MoveToPreviousWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_all_cursors_to_carets(
            |s, sel| {
                let (hit, line_end) = s.word_target(sel, false);
                (hit, s.preferred_column_for_caret(hit, line_end), line_end)
            },
            None,
            window,
            cx,
        );
    }

    pub(super) fn move_to_next_word(
        &mut self,
        _: &MoveToNextWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_all_cursors_to_carets(
            |s, sel| {
                let (hit, line_end) = s.word_target(sel, true);
                (hit, s.preferred_column_for_caret(hit, line_end), line_end)
            },
            None,
            window,
            cx,
        );
    }
}
