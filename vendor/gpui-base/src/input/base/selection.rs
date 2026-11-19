use crate::input::InputModeKind;
use std::ops::Range;

use gpui::{CaretAffinity, Context, TextHit, Window};
use ropey::Rope;
use sum_tree::Bias;

use super::{InputBaseState, RopeExt as _};

/// Unique identifier for a cursor/selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub(super) struct CursorId(usize);

impl CursorId {
    pub(super) fn new(id: usize) -> Self {
        Self(id)
    }
}

impl<M: InputModeKind> InputBaseState<M> {
    /// A native trailing stop belongs to the preceding grapheme, including at EOF.
    pub(super) fn word_range_at_caret(&self, hit: TextHit) -> Option<Range<usize>> {
        // A masked run has no visible word boundaries. Selecting all avoids
        // disclosing where words in the underlying value begin or end.
        if self.masked {
            return Some(0..self.text.len());
        }
        let offset = if hit.affinity == CaretAffinity::Upstream {
            self.previous_boundary(hit.index)
        } else {
            hit.index
        };
        let range = TextSelector::word_range(&self.text, offset)?;
        Some(
            self.grapheme_boundary(range.start, Bias::Left)
                ..self.grapheme_boundary(range.end, Bias::Right),
        )
    }

    pub(super) fn select_word_at_caret(
        &mut self,
        hit: TextHit,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.word_range_at_caret(hit) else {
            return;
        };

        self.undo_manager.break_transaction_coalescing();
        self.selections.remove_all_but_active();
        self.set_selection(range.start, range.end);
        let active = self.active_selection_mut();
        active.reversed = false;
        active.column_anchor = None;
        active.set_caret(CaretAffinity::Upstream, true);
        self.selected_word_range = Some(*self.active_selection());
        cx.notify()
    }

    /// Select the word beginning at a UTF-8 source offset.
    pub(super) fn select_word(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select_word_at_caret(
            TextHit { index: offset, affinity: CaretAffinity::Downstream },
            window,
            cx,
        );
    }

    /// Select the line at the given offset on triple-click.
    ///
    /// The offset is the UTF-8 offset.
    pub(super) fn select_line(&mut self, offset: usize, _: &mut Window, cx: &mut Context<Self>) {
        let range = TextSelector::line_range(&self.text, offset);
        // Like LF, the whole CRLF terminator belongs outside a line selection.
        let end = self.grapheme_boundary(range.end, Bias::Left);
        self.undo_manager.break_transaction_coalescing();
        self.selections.remove_all_but_active();
        self.set_selection(range.start, end);
        let active = self.active_selection_mut();
        active.reversed = false;
        active.column_anchor = None;
        active.set_caret(CaretAffinity::Upstream, true);
        self.selected_word_range = None;
        cx.notify()
    }
}

pub(super) struct TextSelector;
impl TextSelector {
    /// Select a line in the given text at the specified offset.
    ///
    /// The offset is the UTF-8 offset.
    ///
    /// Returns the start and end offsets of the selected line.
    pub(crate) fn line_range(text: &Rope, offset: usize) -> Range<usize> {
        let offset = text.clip_offset(offset, Bias::Left);
        let row = text.offset_to_point(offset).row;
        let start = text.line_start_offset(row);
        let end = text.line_end_offset(row);

        start..end
    }

    /// Select a word in the given text at the specified offset.
    ///
    /// The offset is the UTF-8 offset.
    ///
    /// Returns the start and end offsets of the selected word.
    pub(crate) fn word_range(text: &Rope, offset: usize) -> Option<Range<usize>> {
        let offset = text.clip_offset(offset, Bias::Left);
        let character = text.char_at(offset)?;
        if matches!(character, '\n' | '\r') {
            return Some(offset..offset + character.len_utf8());
        }
        let context_range = if character.is_ascii_alphanumeric() || character == '_' {
            // An ASCII word cannot cross whitespace. Include the separator on
            // each side so ignored Unicode marks retain their boundary context.
            // This avoids flattening a long Rope line for a short ASCII word.
            let mut start = offset;
            for ch in text.chars_at(offset).reversed() {
                start -= ch.len_utf8();
                if ch.is_whitespace() { break; }
            }
            let mut end = offset + character.len_utf8();
            for ch in text.chars_at(end) {
                end += ch.len_utf8();
                if ch.is_whitespace() { break; }
            }
            start..end
        } else {
            // Dictionary runs and ignored marks require complete line context.
            Self::line_range(text, offset)
        };
        let start = context_range.start;
        let context: std::borrow::Cow<'_, str> = text.slice(context_range).into();
        let relative = offset - start;
        gpui::word_ranges(context.as_ref())
            .find(|range| range.contains(&relative))
            .map(|range| start + range.start..start + range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    #[test]
    fn test_word_range() {
        use indoc::indoc;

        let rope = Rope::from(indoc! {
            r#"
            test text:
            abcde 中文🎉 test
            hello[()]
            test_connector ____
            Rope
            rök
            grande île
            "#
        });

        let tests = vec![
            (0, 0, Some("test")),
            (0, 4, Some(" ")),
            (1, 0, Some("abcde")),
            (1, 4, Some("abcde")),
            (1, 5, Some(" ")),
            (1, 6, Some("中文")),
            (1, 9, Some("中文")),
            (1, 13, Some("🎉")),
            (1, 20, Some("test")),
            (2, 5, Some("[")),
            (2, 6, Some("(")),
            (2, 7, Some(")")),
            (2, 8, Some("]")),
            (3, 5, Some("test_connector")),
            (3, 14, Some(" ")),
            (3, 16, Some("____")),
            (4, 0, Some("Rope")),
            (5, 0, Some("rök")),
            (6, 8, Some("île")),
        ];

        for (line, column, expected) in tests {
            let line_start_offset = rope.line_start_offset(line);
            let offset = line_start_offset + column;
            let range = TextSelector::word_range(&rope, offset);

            let actual = range.map(|r| rope.slice(r).to_string());
            let expect = expected.map(|s| s.to_string());
            assert_eq!(actual, expect, "line {}, column {}", line, column);
        }
    }

    #[test]
    fn test_line_range() {
        let rope = Rope::from("first line\nsecond line\nthird");
        let tests = vec![
            (0, 0, "first line"),
            (0, 5, "first line"),
            (1, 3, "second line"),
            (2, 1, "third"),
        ];

        for (line, column, expected) in tests {
            let line_start_offset = rope.line_start_offset(line);
            let offset = line_start_offset + column;
            let range = TextSelector::line_range(&rope, offset);

            let actual = rope.slice(range).to_string();
            assert_eq!(actual, expected, "line {}, column {}", line, column);
        }
    }
}
