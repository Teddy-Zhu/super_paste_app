use crate::{FontId, LineLayout, Pixels, ShapedLine, SharedString, TextRun, TextSystem, px};
use collections::HashMap;
use std::{borrow::Cow, iter, ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

const INLINE_OBJECT_SOURCE: &str = "\u{fffc}";

/// Determines whether to truncate text from the start or end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TruncateFrom {
    /// Truncate text from the start.
    Start,
    /// Truncate text from the end.
    End,
    /// Truncate text from the middle, preserving the start and end.
    Middle,
}

/// Controls how soft-wrapped continuation lines are indented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndentAdjustment {
    /// No indent - continuation lines start at column 0.
    NoIndent,
    /// Match the original line's leading whitespace.
    #[default]
    SameIndent,
    /// Add N extra columns of indent (in space-character widths).
    ExtraColumns(u32),
}

/// The GPUI line wrapper, used to wrap lines of text to a given width.
pub struct LineWrapper {
    text_system: Arc<TextSystem>,
    pub(crate) font_id: FontId,
    pub(crate) font_size: Pixels,
    char_widths: CharWidthCache,
    text_buffer: String,
}

/// Cached character widths for one font and size.
pub(crate) struct CharWidthCache {
    ascii: [Option<Pixels>; 128],
    other: HashMap<char, Pixels>,
}

impl Default for CharWidthCache {
    fn default() -> Self {
        Self {
            ascii: [None; 128],
            other: HashMap::default(),
        }
    }
}

impl LineWrapper {
    /// The maximum indent that can be applied to a line.
    pub const MAX_INDENT: u32 = 256;

    pub(crate) fn new(
        font_id: FontId,
        font_size: Pixels,
        text_system: Arc<TextSystem>,
        char_widths: CharWidthCache,
    ) -> Self {
        Self {
            text_system,
            font_id,
            font_size,
            char_widths,
            text_buffer: String::new(),
        }
    }

    pub(crate) fn into_char_widths(self) -> CharWidthCache {
        self.char_widths
    }

    /// Wrap a line of text to the given width with this wrapper's font and font size.
    pub fn wrap_line<'a>(
        &'a mut self,
        fragments: &'a [LineFragment],
        wrap_width: Pixels,
        indent_adjustment: IndentAdjustment,
    ) -> impl Iterator<Item = Boundary> + 'a {
        self.wrap_line_with_space_policy(fragments, wrap_width, indent_adjustment, false)
    }

    /// Optionally let legacy collapsing ASCII spaces hang at a soft line end. The
    /// source boundary remains after those spaces; the caller removes their
    /// advance from its shaped row. Atomic/no-wrap fragments retain their size.
    pub fn wrap_line_with_space_policy<'a>(
        &'a mut self,
        fragments: &'a [LineFragment],
        wrap_width: Pixels,
        indent_adjustment: IndentAdjustment,
        collapse_spaces: bool,
    ) -> impl Iterator<Item = Boundary> + 'a {
        self.wrap_line_impl(fragments, wrap_width, indent_adjustment, collapse_spaces, None)
    }

    /// Wrap position-dependent advances using the same Unicode and protected
    /// cluster boundaries as ordinary text. Ranges use source UTF-8 offsets;
    /// measurement starts at the row's own origin. There is no synthetic
    /// continuation indent. The caller may retain a streaming measurement.
    pub fn wrap_line_with_measurement<'a>(
        &'a mut self,
        fragments: &'a [LineFragment],
        wrap_width: Pixels,
        collapse_spaces: bool,
        measure: &'a mut dyn FnMut(Range<usize>) -> Pixels,
    ) -> impl Iterator<Item = Boundary> + 'a {
        self.wrap_line_impl(fragments, wrap_width, IndentAdjustment::NoIndent, collapse_spaces, Some(measure))
    }

    fn wrap_line_impl<'a>(
        &'a mut self,
        fragments: &'a [LineFragment],
        wrap_width: Pixels,
        indent_adjustment: IndentAdjustment,
        collapse_spaces: bool,
        mut measure: Option<&'a mut dyn FnMut(Range<usize>) -> Pixels>,
    ) -> impl Iterator<Item = Boundary> + 'a {
        let mut width = px(0.);
        let mut first_non_whitespace_ix = None;
        let mut base_indent = None;
        let mut last_candidate_ix = 0;
        let mut last_candidate_width = px(0.);
        let mut last_wrap_ix = 0;
        let mut index = 0;
        let Self {
            text_buffer: text,
            char_widths,
            text_system,
            font_id,
            font_size,
        } = self;
        text.clear();
        let line_text = match fragments {
            [LineFragment::Text { text } | LineFragment::ShapedText { text, .. }
                | LineFragment::NoWrapText { text, .. } | LineFragment::MeasuredText { text, .. }] => *text,
            [LineFragment::InlineObject { .. }] => INLINE_OBJECT_SOURCE,
            _ => {
                for fragment in fragments {
                    match fragment {
                        LineFragment::Text { text: fragment_text }
                            | LineFragment::ShapedText { text: fragment_text, .. }
                            | LineFragment::NoWrapText { text: fragment_text, .. }
                            | LineFragment::MeasuredText { text: fragment_text, .. } => text.push_str(fragment_text),
                        LineFragment::InlineObject { .. } => text.push_str(INLINE_OBJECT_SOURCE),
                        // Opaque elements and legacy measured ranges retain
                        // their existing unbroken placeholder context.
                        LineFragment::Element { len_utf8, .. } | LineFragment::NoWrap { len_utf8, .. } => {
                            text.extend(iter::repeat_n('a', *len_utf8));
                        }
                    }
                }
                text.as_str()
            }
        };
        let mut break_indices = Self::break_indices(line_text).peekable();
        let mut grapheme_indices = line_text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .peekable();
        let mut width_for_char = move |character| {
            width_for_char(char_widths, character, text_system, *font_id, *font_size)
        };
        let mut grapheme_index = 0;
        let mut width_before_grapheme = px(0.);
        let mut previous_space_break = false;
        let mut candidates = fragments
            .iter()
            .flat_map(move |fragment| fragment.wrap_boundary_candidates())
            .peekable();
        iter::from_fn(move || {
            while let Some(mut candidate) = candidates.next() {
                let mut trailing_hang_len = 0;
                if candidate.no_wrap() {
                    // Measure the complete protected group before deciding
                    // where to wrap. Looking only at its first few parts can
                    // emit an unnecessary row before the oversized group.
                    let can_break_before = candidate.can_break_before();
                    let mut first_non_whitespace = candidate.first_non_whitespace();
                    let mut len_utf8 = candidate.len_utf8();
                    let mut group_width = candidate.width(&mut width_for_char);
                    let mut trailing_hang = if candidate.hangs(collapse_spaces) { group_width } else { Pixels::ZERO };
                    if candidate.hangs(collapse_spaces) { trailing_hang_len = len_utf8; }
                    while candidates.peek().is_some_and(WrapBoundaryCandidate::no_wrap) {
                        let next = candidates.next().expect("peeked protected candidate");
                        if first_non_whitespace.is_none() {
                            first_non_whitespace = next.first_non_whitespace().map(|offset| len_utf8 + offset);
                        }
                        len_utf8 += next.len_utf8();
                        let advance = next.width(&mut width_for_char);
                        trailing_hang = if next.hangs(collapse_spaces) { trailing_hang + advance } else { Pixels::ZERO };
                        trailing_hang_len = if next.hangs(collapse_spaces) { trailing_hang_len + next.len_utf8() } else { 0 };
                        group_width += advance;
                    }
                    candidate = WrapBoundaryCandidate::Element {
                        width: group_width, len_utf8, no_wrap: true,
                        can_break_before, first_non_whitespace, trailing_hang,
                    };
                }
                let ix = index;
                index += candidate.len_utf8();
                while grapheme_indices
                    .next_if(|position| *position < ix)
                    .is_some()
                {}
                let is_grapheme_boundary = grapheme_indices.next_if_eq(&ix).is_some()
                    && candidate.can_break_before();
                if is_grapheme_boundary {
                    grapheme_index = ix;
                    width_before_grapheme = width;
                }
                while break_indices.next_if(|position| *position < ix).is_some() {}
                if (break_indices.next_if_eq(&ix).is_some() || previous_space_break)
                    && is_grapheme_boundary
                    && first_non_whitespace_ix.is_some()
                {
                    last_candidate_ix = ix;
                    last_candidate_width = width;
                }
                let hanging_space = candidate.hangs(collapse_spaces);
                let mut trailing_hang = match &candidate {
                    WrapBoundaryCandidate::Element { trailing_hang, .. } => *trailing_hang,
                    _ => Pixels::ZERO,
                };
                previous_space_break = matches!(&candidate,
                    WrapBoundaryCandidate::Char { character, space_policy: Some(TextSpacePolicy::BreakSpaces), no_wrap: false, .. }
                        if TextSpacePolicy::BreakSpaces.breaks_after(*character));
                let item_width = match candidate {
                    WrapBoundaryCandidate::Char { character: c, advance, .. } => {
                        if c == '\n' {
                            continue;
                        }

                        if c != ' ' && first_non_whitespace_ix.is_none() {
                            first_non_whitespace_ix = Some(ix);
                        }

                        advance.unwrap_or_else(|| width_for_char(c))
                    }
                    WrapBoundaryCandidate::Element {
                        width: element_width,
                        first_non_whitespace,
                        ..
                    } => {
                        if first_non_whitespace_ix.is_none() {
                            first_non_whitespace_ix = first_non_whitespace.map(|offset| ix + offset);
                        }

                        element_width
                    }
                };

                if let Some(measure) = measure.as_mut() {
                    width = measure(last_wrap_ix..index);
                    if trailing_hang_len > 0 {
                        trailing_hang = width - measure(last_wrap_ix..index - trailing_hang_len);
                    }
                } else { width += item_width; }
                if width - trailing_hang > wrap_width && grapheme_index > last_wrap_ix && !hanging_space {
                    let candidate_width = if last_candidate_ix > last_wrap_ix {
                        measure.as_mut().map_or(width - last_candidate_width, |measure| measure(last_candidate_ix..index))
                    } else { width };
                    let wrap_at_candidate = last_candidate_ix > last_wrap_ix && candidate_width <= wrap_width;

                    let carried_width = if wrap_at_candidate {
                        candidate_width
                    } else {
                        measure.as_mut().map_or(width - width_before_grapheme, |measure| measure(grapheme_index..index))
                    };

                    // Compute base indentation from the first non-whitespace character on the line
                    // and retain it for all subsequent wrap rows. If the line begins with leading
                    // whitespace that wraps before any non-whitespace character (or is all whitespace),
                    // base_indent remains None so continuation rows within the leading whitespace
                    // do not receive ExtraColumns indentation and degrade into cascading short rows.
                    if base_indent.is_none() {
                        if let Some(first_non_whitespace_ix) = first_non_whitespace_ix {
                            base_indent =
                                Some(Self::MAX_INDENT.min(
                                    first_non_whitespace_ix.saturating_sub(last_wrap_ix) as u32,
                                ));
                        }
                    }

                    let next_indent = match indent_adjustment {
                        IndentAdjustment::NoIndent => 0,
                        IndentAdjustment::SameIndent => base_indent.unwrap_or(0),
                        IndentAdjustment::ExtraColumns(extra) => {
                            if let Some(base_indent) = base_indent {
                                let candidate = base_indent.saturating_add(extra);
                                let candidate_indent_width = width_for_char(' ') * candidate as f32;
                                // Reserve headroom for any carried suffix from an earlier word boundary
                                // (and at least 2 columns for a full-width character) so the continuation
                                // line does not immediately exceed wrap width.
                                let min_headroom = carried_width.max(width_for_char(' ') * 2.0);
                                if candidate_indent_width + min_headroom > wrap_width {
                                    0
                                } else {
                                    Self::MAX_INDENT.min(candidate)
                                }
                            } else {
                                0
                            }
                        }
                    };

                    if wrap_at_candidate {
                        last_wrap_ix = last_candidate_ix;
                        if let Some(measure) = measure.as_mut() {
                            width = carried_width;
                            width_before_grapheme = measure(last_wrap_ix..grapheme_index);
                        } else {
                            width -= last_candidate_width;
                            width_before_grapheme -= last_candidate_width;
                        }
                    } else {
                        // Oversized graphemes and shaped clusters stay intact,
                        // even if they exceed the line width.
                        last_wrap_ix = grapheme_index;
                        if measure.is_some() { width = carried_width; }
                        else { width -= width_before_grapheme; }
                        width_before_grapheme = px(0.);
                    }
                    last_candidate_ix = 0;

                    let indent_width = if next_indent == 0 { Pixels::ZERO }
                        else { width_for_char(' ') * next_indent as f32 };
                    width += indent_width;
                    width_before_grapheme += indent_width;

                    return Some(Boundary::new(last_wrap_ix, next_indent));
                }
            }

            None
        })
    }

    /// Determines if a line should be truncated based on its width.
    ///
    /// Returns the truncation index in `line`.
    pub fn should_truncate_line(
        &mut self,
        line: &str,
        truncate_width: Pixels,
        truncation_affix: &str,
        truncate_from: TruncateFrom,
    ) -> Option<usize> {
        let mut width = px(0.);
        let suffix_width = truncation_affix
            .chars()
            .map(|c| self.width_for_char(c))
            .fold(px(0.0), |a, x| a + x);
        let mut truncate_ix = 0;

        match truncate_from {
            TruncateFrom::Start => {
                for (ix, c) in line.char_indices().rev() {
                    if width + suffix_width < truncate_width {
                        truncate_ix = ix;
                    }

                    let char_width = self.width_for_char(c);
                    width += char_width;

                    if width.floor() > truncate_width {
                        return Some(truncate_ix);
                    }
                }
            }
            TruncateFrom::End => {
                for (ix, c) in line.char_indices() {
                    if width + suffix_width < truncate_width {
                        truncate_ix = ix;
                    }

                    let char_width = self.width_for_char(c);
                    width += char_width;

                    if width.floor() > truncate_width {
                        return Some(truncate_ix);
                    }
                }
            }
            TruncateFrom::Middle => {}
        }

        None
    }

    fn should_truncate_line_middle(
        &mut self,
        line: &str,
        truncate_width: Pixels,
        truncation_affix: &str,
    ) -> Option<(usize, usize)> {
        let suffix_width = truncation_affix
            .chars()
            .map(|c| self.width_for_char(c))
            .fold(px(0.0), |a, x| a + x);

        let total_width: Pixels = line
            .chars()
            .map(|c| self.width_for_char(c))
            .fold(px(0.0), |a, x| a + x);

        if total_width <= truncate_width {
            return None;
        }

        let content_budget = truncate_width - suffix_width;
        if content_budget <= px(0.) {
            return Some((0, line.len()));
        }

        let front_budget = content_budget * (2.0 / 3.0);
        let back_budget = content_budget - front_budget;

        let mut front_width = px(0.);
        let mut front_end_ix = 0usize;
        for (ix, c) in line.char_indices() {
            let char_width = self.width_for_char(c);
            if front_width + char_width > front_budget {
                break;
            }
            front_width += char_width;
            front_end_ix = ix + c.len_utf8();
        }

        let mut back_width = px(0.);
        let mut back_start_ix = line.len();
        for (ix, c) in line.char_indices().rev() {
            let char_width = self.width_for_char(c);
            if back_width + char_width > back_budget {
                break;
            }
            back_width += char_width;
            back_start_ix = ix;
        }

        if front_end_ix >= back_start_ix {
            return Some((0, line.len()));
        }

        Some((front_end_ix, back_start_ix))
    }

    /// Truncate a line of text to the given width with this wrapper's font and font size.
    pub fn truncate_line<'a>(
        &mut self,
        line: SharedString,
        truncate_width: Pixels,
        truncation_affix: &str,
        runs: &'a [TextRun],
        truncate_from: TruncateFrom,
    ) -> (SharedString, Cow<'a, [TextRun]>) {
        if truncate_from == TruncateFrom::Middle {
            if let Some((front_end_ix, back_start_ix)) =
                self.should_truncate_line_middle(&line, truncate_width, truncation_affix)
            {
                let result = SharedString::from(format!(
                    "{}{truncation_affix}{}",
                    &line[..front_end_ix],
                    &line[back_start_ix..]
                ));
                let mut runs = runs.to_vec();
                update_runs_after_middle_truncation(
                    truncation_affix,
                    &mut runs,
                    front_end_ix,
                    back_start_ix,
                );
                return (result, Cow::Owned(runs));
            } else {
                return (line, Cow::Borrowed(runs));
            }
        }

        if let Some(truncate_ix) =
            self.should_truncate_line(&line, truncate_width, truncation_affix, truncate_from)
        {
            let result = match truncate_from {
                TruncateFrom::Start => SharedString::from(format!(
                    "{truncation_affix}{}",
                    &line[line.ceil_char_boundary(truncate_ix + 1)..]
                )),
                TruncateFrom::End => SharedString::from(format!(
                    "{}{truncation_affix}",
                    line[..truncate_ix]
                        .trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                )),
                TruncateFrom::Middle => unreachable!("Middle truncation is handled above"),
            };
            let mut runs = runs.to_vec();
            update_runs_after_truncation(&result, truncation_affix, &mut runs, truncate_from);
            (result, Cow::Owned(runs))
        } else {
            (line, Cow::Borrowed(runs))
        }
    }

    /// Truncate text to fit within a given number of wrapped lines.
    ///
    /// Unlike `truncate_line` which treats the text as a flat width budget
    /// (`width * max_lines`), this method accounts for word-boundary wrapping:
    /// it walks through characters once, tracking wrap boundaries and the
    /// truncation point simultaneously. When text overflows on the last
    /// allowed line, it truncates there and appends the affix.
    ///
    /// For `max_lines == 1`, this delegates to `truncate_line`.
    pub fn truncate_wrapped_line<'a>(
        &mut self,
        text: SharedString,
        wrap_width: Pixels,
        max_lines: usize,
        truncation_affix: &str,
        runs: &'a [TextRun],
        truncate_from: TruncateFrom,
    ) -> (SharedString, Cow<'a, [TextRun]>) {
        if max_lines <= 1 || truncate_from == TruncateFrom::Start {
            return self.truncate_line(
                text,
                wrap_width * max_lines,
                truncation_affix,
                runs,
                truncate_from,
            );
        }
        if truncate_from == TruncateFrom::Middle {
            return self.truncate_line(text, wrap_width, truncation_affix, runs, truncate_from);
        }

        let affix_width: Pixels = truncation_affix
            .chars()
            .map(|c| self.width_for_char(c))
            .sum();

        let mut width = px(0.);
        let mut line = 0usize;
        let mut first_non_whitespace_ix = None;
        let mut last_candidate_ix = 0usize;
        let mut last_candidate_width = px(0.);
        let mut last_wrap_ix = 0usize;
        let mut indent: Option<u32> = None;
        let mut truncate_ix = 0usize;

        let mut break_indices = Self::break_indices(&text).peekable();
        for (ix, c) in text.char_indices() {
            // Consume boundaries on newlines too, so skipping an empty line does not
            // leave the iterator stuck before subsequent text.
            let is_break = break_indices.next_if_eq(&ix).is_some();
            if c == '\n' {
                if line >= max_lines - 1 && !text[ix + 1..].trim().is_empty() {
                    // Newline on the last allowed line with real content
                    // below. Truncate here.
                    let truncated = text[..truncate_ix]
                        .trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
                    let result = SharedString::from(format!("{truncated}{truncation_affix}"));
                    let mut runs = runs.to_vec();
                    update_runs_after_truncation(
                        &result,
                        truncation_affix,
                        &mut runs,
                        TruncateFrom::End,
                    );
                    return (result, Cow::Owned(runs));
                }

                // Newline before the last line: it consumes a line.
                line += 1;
                width = px(0.);
                first_non_whitespace_ix = None;
                last_candidate_ix = 0;
                last_candidate_width = px(0.);
                last_wrap_ix = ix + 1;
                indent = None;
                truncate_ix = ix + 1;
                continue;
            }

            let char_width = self.width_for_char(c);

            if is_break && first_non_whitespace_ix.is_some() {
                last_candidate_ix = ix;
                last_candidate_width = width;
            }

            if c != ' ' && first_non_whitespace_ix.is_none() {
                first_non_whitespace_ix = Some(ix);
            }

            width += char_width;

            if line < max_lines - 1 {
                // Before the last line: replicate wrap_line's boundary logic.
                if width > wrap_width && ix > last_wrap_ix {
                    if let (None, Some(first_nw)) = (indent, first_non_whitespace_ix) {
                        indent = Some(Self::MAX_INDENT.min((first_nw - last_wrap_ix) as u32));
                    }

                    if last_candidate_ix > last_wrap_ix {
                        last_wrap_ix = last_candidate_ix;
                        width -= last_candidate_width;
                        last_candidate_ix = 0;
                    } else {
                        last_wrap_ix = ix;
                        width = char_width;
                    }

                    if let Some(ind) = indent {
                        width += self.width_for_char(' ') * ind as f32;
                    }

                    line += 1;
                    truncate_ix = last_wrap_ix;
                }
            } else {
                // On the last line: track the furthest point where the affix
                // still fits, and stop as soon as the line overflows.
                if width + affix_width <= wrap_width {
                    truncate_ix = ix + c.len_utf8();
                }

                if width > wrap_width {
                    let truncated = text[..truncate_ix]
                        .trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
                    let result = SharedString::from(format!("{truncated}{truncation_affix}"));
                    let mut runs = runs.to_vec();
                    update_runs_after_truncation(
                        &result,
                        truncation_affix,
                        &mut runs,
                        TruncateFrom::End,
                    );
                    return (result, Cow::Owned(runs));
                }
            }
        }

        // Text fits within max_lines without truncation.
        drop(break_indices);
        (text, Cow::Borrowed(runs))
    }

    /// The byte offsets in `text` at which a line break is allowed.
    pub(crate) fn break_indices(text: &str) -> impl Iterator<Item = usize> {
        super::line_break_indices(text)
    }

    #[inline(always)]
    fn width_for_char(&mut self, c: char) -> Pixels {
        width_for_char(
            &mut self.char_widths,
            c,
            &self.text_system,
            self.font_id,
            self.font_size,
        )
    }
}

#[inline(always)]
fn width_for_char(
    char_widths: &mut CharWidthCache,
    c: char,
    text_system: &TextSystem,
    font_id: FontId,
    font_size: Pixels,
) -> Pixels {
    if (c as u32) < 128 {
        if let Some(cached_width) = char_widths.ascii[c as usize] {
            cached_width
        } else {
            let width = text_system.layout_width(font_id, font_size, c);
            char_widths.ascii[c as usize] = Some(width);
            width
        }
    } else if let Some(cached_width) = char_widths.other.get(&c) {
        *cached_width
    } else {
        let width = text_system.layout_width(font_id, font_size, c);
        char_widths.other.insert(c, width);
        width
    }
}

fn update_runs_after_truncation(
    result: &str,
    ellipsis: &str,
    runs: &mut Vec<TextRun>,
    truncate_from: TruncateFrom,
) {
    let mut truncate_at = result.len() - ellipsis.len();
    match truncate_from {
        TruncateFrom::Start => {
            for (run_index, run) in runs.iter_mut().enumerate().rev() {
                if run.len <= truncate_at {
                    truncate_at -= run.len;
                } else {
                    run.len = truncate_at + ellipsis.len();
                    runs.splice(..run_index, std::iter::empty());
                    break;
                }
            }
        }
        TruncateFrom::End => {
            for (run_index, run) in runs.iter_mut().enumerate() {
                if run.len <= truncate_at {
                    truncate_at -= run.len;
                } else {
                    run.len = truncate_at + ellipsis.len();
                    runs.truncate(run_index + 1);
                    break;
                }
            }
        }
        TruncateFrom::Middle => {
            unreachable!("Middle truncation calls this function with TruncateFrom::End directly")
        }
    }
}

fn update_runs_after_middle_truncation(
    ellipsis: &str,
    runs: &mut Vec<TextRun>,
    front_end_ix: usize,
    back_start_ix: usize,
) {
    let original_runs = std::mem::take(runs);
    let mut result_runs: Vec<TextRun> = Vec::with_capacity(original_runs.len());

    // Front segment [0, front_end_ix) + ellipsis: walk forward until the run
    // that straddles or ends at front_end_ix, then extend that run's length
    // to include the ellipsis.
    let mut front_remaining = front_end_ix;
    let mut front_done = false;
    for run in &original_runs {
        if front_done {
            break;
        }
        if run.len <= front_remaining {
            result_runs.push(run.clone());
            front_remaining -= run.len;
        } else {
            let mut partial = run.clone();
            partial.len = front_remaining + ellipsis.len();
            result_runs.push(partial);
            front_done = true;
        }
    }
    if !front_done {
        // front_end_ix landed exactly on a run boundary; append ellipsis to
        // the last front run (or, if the front is empty, to the first back run).
        if let Some(last) = result_runs.last_mut() {
            last.len += ellipsis.len();
        } else if let Some(first) = original_runs.first() {
            let mut affix_run = first.clone();
            affix_run.len = ellipsis.len();
            result_runs.push(affix_run);
        }
    }

    // Back segment [back_start_ix, original.len()): skip runs entirely in the
    // removed middle, keep the rest.
    let mut byte_pos = 0usize;
    for run in &original_runs {
        let run_end = byte_pos + run.len;
        if run_end > back_start_ix {
            if byte_pos < back_start_ix {
                // Run straddles back_start_ix; keep only the tail.
                let mut partial = run.clone();
                partial.len = run_end - back_start_ix;
                result_runs.push(partial);
            } else {
                result_runs.push(run.clone());
            }
        }
        byte_pos = run_end;
    }

    *runs = result_runs;
}

/// An actual replaced object in a shaped paragraph's UTF-8 source. The native
/// shaper sees U+FFFC for context; wrapping uses the host's measured advance.
/// Object painting remains the host's responsibility.
#[derive(Clone, Copy, Debug)]
pub struct InlineObjectAdvance {
    /// Source offset of exactly one U+FFFC.
    pub index: usize,
    /// Measured layout width, independent of the font's U+FFFC glyph.
    pub width: Pixels,
}

impl InlineObjectAdvance {
    /// The object's complete source interval.
    pub fn source_range(&self) -> std::ops::Range<usize> {
        self.index..self.index + INLINE_OBJECT_SOURCE.len()
    }
}

/// White-space semantics for source-preserving structured text. Runs without
/// an explicit policy retain the wrapper's existing behavior. Source collapse
/// happens in the host; these rules govern line edges and wrap opportunities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSpacePolicy {
    /// CSS normal, nowrap and pre-line: remove collapsible edge spaces.
    Collapse,
    /// CSS pre: retain edge advances; no-wrap is a separate constraint.
    Preserve,
    /// CSS pre-wrap: preserve spaces and let trailing spaces hang.
    PreWrap,
    /// CSS break-spaces: retain advances and break after each separator.
    BreakSpaces,
}

impl TextSpacePolicy {
    /// Whether a source character loses its advance at the trailing line edge.
    pub fn collapses_at_line_end(self, character: char) -> bool {
        self == Self::Collapse && matches!(character, ' ' | '\t' | '\r' | '\u{1680}')
    }

    /// Whether a trailing character may be excluded when fitting a row.
    /// Conditional pre-wrap hanging is resolved by the host after fitting.
    pub fn hangs_at_line_end(self, character: char) -> bool {
        match self {
            Self::Collapse => self.collapses_at_line_end(character) || other_space_separator(character),
            Self::PreWrap => matches!(character, ' ' | '\t' | '\r') || other_space_separator(character),
            Self::Preserve | Self::BreakSpaces => false,
        }
    }

    /// A break-spaces opportunity still respects graphemes, native clusters
    /// and explicit no-wrap constraints.
    pub fn breaks_after(self, character: char) -> bool {
        self == Self::BreakSpaces && (matches!(character, ' ' | '\t' | '\r') || other_space_separator(character))
    }
}

fn other_space_separator(character: char) -> bool {
    // Unicode Zs, excluding U+0020 SPACE and U+00A0 NO-BREAK SPACE.
    matches!(character, '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// Source-order advances shared by shaped wrapping candidates.
/// A caller measuring position-dependent widths can lend its existing prefix
/// table instead of retaining a second full cluster index in the wrapper.
#[derive(Clone)]
pub struct LogicalAdvances<'a>(LogicalAdvanceSource<'a>);

#[derive(Clone)]
enum LogicalAdvanceSource<'a> {
    Widths(Arc<[(usize, Pixels)]>),
    Cumulative(&'a [(usize, f64)]),
}

impl LogicalAdvances<'_> {
    fn partition_point(&self, index: usize) -> usize {
        match &self.0 {
            LogicalAdvanceSource::Widths(entries) => entries.partition_point(|entry| entry.0 < index),
            LogicalAdvanceSource::Cumulative(entries) => entries[..entries.len() - 1].partition_point(|entry| entry.0 < index),
        }
    }

    fn get(&self, index: usize) -> Option<(usize, Pixels)> {
        match &self.0 {
            LogicalAdvanceSource::Widths(entries) => entries.get(index).copied(),
            LogicalAdvanceSource::Cumulative(entries) => {
                let &(source, before) = entries.get(index)?;
                let &(_, after) = entries.get(index + 1)?;
                Some((source, px((after - before) as f32)))
            }
        }
    }
}

/// A fragment of a line that can be wrapped.
pub enum LineFragment<'a> {
    /// A text fragment consisting of characters.
    Text {
        /// The text content of the fragment.
        text: &'a str,
    },
    /// Text measured with its actual shaped font runs. Keeping the layout
    /// shares glyph positions; it does not allocate per-character advances.
    ShapedText {
        /// The visible suffix of the shaped source text.
        text: &'a str,
        /// Shared native glyph positions of the original source text.
        layout: Arc<LineLayout>,
        /// Source-order cluster advances when native paint order differs or
        /// a custom measurer lends its existing prefix. Continuations share
        /// the same data; ordinary ordered layouts require no index.
        logical_advances: Option<LogicalAdvances<'a>>,
        /// Sorted disjoint nonempty unbreakable ranges in the shaped source.
        /// Keep source characters and native clusters intact across their edges.
        no_wrap_ranges: &'a [std::ops::Range<usize>],
        /// Optional uniform policy and sorted disjoint overrides, using the
        /// original layout's UTF-8 coordinates (also for suffix fragments).
        space_policy: Option<TextSpacePolicy>,
        space_policy_ranges: &'a [(std::ops::Range<usize>, TextSpacePolicy)],
        /// Sorted object advances in original UTF-8 coordinates. Native
        /// glyphs remain unchanged; hosts distribute text and object pieces.
        inline_objects: &'a [InlineObjectAdvance],
        /// UTF-8 offset of this suffix in that original text.
        offset: usize,
        /// Horizontal box padding charged at the start of this suffix.
        padding: Pixels,
    },
    /// An unbreakable styled range, including ranges wider than the line.
    /// Hosts can coalesce adjacent ranges across style/image boundaries.
    NoWrap {
        /// The measured width of the complete unbreakable range.
        width: Pixels,
        /// Its length in the host's UTF-8 coordinate space.
        len_utf8: usize,
    },
    /// An unbreakable measured part retaining its actual source. Adjacent
    /// protected parts form one group without concatenating another string.
    NoWrapText {
        /// Nonempty source text from one hard line.
        text: &'a str,
        /// The measured width, including any inline box padding.
        width: Pixels,
    },
    /// One already-measured text unit retaining Unicode context at its edges.
    /// Unlike a protected range it does not join adjacent protected units.
    MeasuredText {
        /// Nonempty source text from one hard line.
        text: &'a str,
        /// Its measured width, including inline box padding.
        width: Pixels,
    },
    /// A non-text element with a fixed width.
    Element {
        /// The width of the element in pixels.
        width: Pixels,
        /// The UTF-8 encoded length of the element.
        len_utf8: usize,
    },
    /// An atomic replaced object occupying one U+FFFC in the UTF-8 source.
    /// Its real Unicode line-break context is retained beside surrounding text.
    InlineObject {
        /// The measured width of the object in pixels.
        width: Pixels,
        /// Whether this object belongs to the adjacent protected source group.
        no_wrap: bool,
    },
}

impl<'a> LineFragment<'a> {
    /// Creates a new text fragment from the given text.
    pub fn text(text: &'a str) -> Self {
        LineFragment::Text { text }
    }

    /// Creates a new non-text element with the given width and UTF-8 encoded length.
    pub fn element(width: Pixels, len_utf8: usize) -> Self {
        LineFragment::Element { width, len_utf8 }
    }

    /// Creates a replaced object with its measured width. The source occupies
    /// exactly one U+FFFC; no glyph or font measurement is needed for it.
    pub fn inline_object(width: Pixels) -> Self {
        Self::InlineObject { width, no_wrap: false }
    }

    /// Creates an object inside an unbreakable range. Adjacent protected text
    /// and objects share one measured group and retain real Unicode context.
    pub fn no_wrap_inline_object(width: Pixels) -> Self {
        Self::InlineObject { width, no_wrap: true }
    }

    /// Borrows one nonempty measured part of an unbreakable hard-line range.
    pub fn no_wrap_text(text: &'a str, width: Pixels) -> Self {
        assert!(!text.is_empty(), "protected text must have source bytes");
        Self::NoWrapText { text, width }
    }

    /// Borrows an already-measured source unit without replacing its text in
    /// the Unicode context buffer or charging its glyph width a second time.
    pub fn measured_text(text: &'a str, width: Pixels) -> Self {
        assert!(!text.is_empty(), "measured text must have source bytes");
        Self::MeasuredText { text, width }
    }

    /// `line` must have been shaped from `text`. Padding is charged at the
    /// beginning of the fragment; a host wrapping one row at a time can keep
    /// the same padding when slicing a continuation.
    pub fn shaped_text(text: &'a str, line: &ShapedLine, padding: Pixels) -> Self {
        let byte_ordered = line.layout.runs.iter().flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index)).is_sorted();
        let logical_advances = if byte_ordered { None } else {
            // Index the shaper's actual cluster advances in source order.
            // Ink offsets and visual neighbors do not determine text width.
            let mut advances: Vec<_> = line.layout.runs.iter().flat_map(|run| &run.glyphs)
                .map(|glyph| (glyph.index, glyph.advance)).collect();
            advances.sort_unstable_by_key(|glyph| glyph.0);
            advances.dedup_by(|next, previous| {
                if next.0 == previous.0 { previous.1 += next.1; true } else { false }
            });
            Some(LogicalAdvances(LogicalAdvanceSource::Widths(Arc::from(advances))))
        };
        Self::ShapedText { text, layout: line.layout.clone(), logical_advances, no_wrap_ranges: &[],
            space_policy: None, space_policy_ranges: &[], inline_objects: &[], offset: 0, padding }
    }

    /// Borrow the complete source/cluster prefix already owned by a custom
    /// measurer. Entries must be native cluster or registered object starts,
    /// followed by a source-length sentinel. Suffixes retain that same table.
    pub fn shaped_text_with_cumulative_advances(
        text: &'a str, line: &ShapedLine, positions: &'a [(usize, f64)],
    ) -> Self {
        assert!(text.len() == line.len() && positions.last().is_some_and(|entry| entry.0 == text.len()),
            "cumulative advances must cover the complete shaped source");
        assert!(positions.iter().all(|(index, position)| text.is_char_boundary(*index) && position.is_finite())
            && positions.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "cumulative advances must have unique sorted source boundaries and finite positions");
        Self::ShapedText {
            text, layout: line.layout.clone(), logical_advances: Some(LogicalAdvances(LogicalAdvanceSource::Cumulative(positions))),
            no_wrap_ranges: &[], space_policy: None, space_policy_ranges: &[], inline_objects: &[],
            offset: 0, padding: Pixels::ZERO,
        }
    }

    /// Apply sorted disjoint nonempty ranges to shaped text without separating
    /// its Unicode input, native glyphs or logical advance index. The ranges
    /// use the original layout's UTF-8 coordinates, including for suffixes.
    pub fn with_no_wrap_ranges(mut self, ranges: &'a [std::ops::Range<usize>]) -> Self {
        let Self::ShapedText { no_wrap_ranges, .. } = &mut self else {
            panic!("unbreakable source ranges require shaped text");
        };
        *no_wrap_ranges = ranges;
        self
    }

    /// Apply one white-space policy without allocating a range table.
    pub fn with_space_policy(mut self, policy: Option<TextSpacePolicy>) -> Self {
        let Self::ShapedText { space_policy, .. } = &mut self else {
            panic!("white-space policies require shaped text");
        };
        *space_policy = policy;
        self
    }

    /// Apply source ranges without splitting native font, joining or bidi
    /// context. Ranges must be nonempty, sorted and disjoint.
    pub fn with_space_policy_ranges(mut self, ranges: &'a [(std::ops::Range<usize>, TextSpacePolicy)]) -> Self {
        let Self::ShapedText { space_policy_ranges, .. } = &mut self else {
            panic!("white-space policies require shaped text");
        };
        *space_policy_ranges = ranges;
        self
    }

    /// Replace only measured object advances, keeping the complete source for
    /// Unicode segmentation and native joining/bidi context. Configure the
    /// complete shaped input once; suffixes retain the same borrowed table.
    pub fn with_inline_objects(mut self, objects: &'a [InlineObjectAdvance]) -> Self {
        let Self::ShapedText { text, layout, offset, inline_objects, .. } = &mut self else {
            panic!("object advances require shaped text");
        };
        assert!(*offset == 0 && text.len() == layout.len, "object advances require complete shaped source");
        let mut end = 0;
        for object in objects {
            assert!(object.index >= end && text.get(object.source_range()) == Some(INLINE_OBJECT_SOURCE),
                "object advances must identify sorted, distinct U+FFFC source atoms");
            assert!(object.width.as_f32().is_finite() && object.width >= Pixels::ZERO,
                "object advances require finite nonnegative widths");
            end = object.source_range().end;
        }
        *inline_objects = objects;
        self
    }

    fn wrap_boundary_candidates(&self) -> impl Iterator<Item = WrapBoundaryCandidate> {
        let text = match self {
            LineFragment::Text { text } | LineFragment::ShapedText { text, .. }
                | LineFragment::NoWrapText { text, .. } | LineFragment::MeasuredText { text, .. } => text,
            LineFragment::Element { .. } | LineFragment::NoWrap { .. } => "\0",
            LineFragment::InlineObject { .. } => INLINE_OBJECT_SOURCE,
        };
        let element = match self {
            Self::Element { width, len_utf8 } => Some((*width, *len_utf8, false, Some(0))),
            Self::NoWrap { width, len_utf8 } => Some((*width, *len_utf8, true, Some(0))),
            Self::NoWrapText { text, width } | Self::MeasuredText { text, width } => Some((*width, text.len(), matches!(self, Self::NoWrapText { .. }),
                text.char_indices().find_map(|(index, character)|
                    (character != ' ' && character != '\n').then_some(index)))),
            Self::InlineObject { width, no_wrap } => Some((*width, INLINE_OBJECT_SOURCE.len(), *no_wrap, Some(0))),
            _ => None,
        };
        let (layout, mut index, padding, logical, no_wrap_ranges, space_policy, space_policy_ranges, inline_objects) = match self {
            Self::ShapedText { layout, offset, padding, logical_advances, no_wrap_ranges, space_policy, space_policy_ranges, inline_objects, .. } =>
                (Some(layout), *offset, *padding, logical_advances.as_ref(), *no_wrap_ranges, *space_policy, *space_policy_ranges, *inline_objects),
            _ => (None, 0, Pixels::ZERO, None, &[][..], None, &[][..], &[][..]),
        };
        let mut no_wrap_index = no_wrap_ranges.partition_point(|range| range.end <= index);
        let mut space_policy_index = space_policy_ranges.partition_point(|(range, _)| range.end <= index);
        let mut object_index = inline_objects.partition_point(|object| object.index < index);
        let mut logical_index = logical.map_or(0, |advances| advances.partition_point(index));
        let mut glyphs = layout.into_iter().flat_map(|layout| &layout.runs).flat_map(|run| &run.glyphs).peekable();
        if logical.is_none() { while glyphs.peek().is_some_and(|glyph| glyph.index < index) { glyphs.next(); } }
        let mut first = true;
        // Measured parts emit one candidate; Unicode context still comes from
        // their real source. Ordinary text keeps its direct character iterator.
        let candidate_text = if element.is_some() { "\0" } else { text };
        candidate_text.chars().map(move |character| {
            if let Some((width, len_utf8, no_wrap, first_non_whitespace)) = element {
                WrapBoundaryCandidate::Element {
                    width, len_utf8, no_wrap, first_non_whitespace,
                    can_break_before: true, trailing_hang: Pixels::ZERO,
                }
            } else {
                // A grapheme boundary may still be inside a ligature. These
                // fragments reuse their shaped glyphs when sliced, so a break
                // must not strand the remaining bytes without their glyph.
                while no_wrap_ranges.get(no_wrap_index).is_some_and(|range| range.end <= index) {
                    no_wrap_index += 1;
                }
                let protected = no_wrap_ranges.get(no_wrap_index).filter(|range| range.start <= index);
                while space_policy_ranges.get(space_policy_index).is_some_and(|(range, _)| range.end <= index) {
                    space_policy_index += 1;
                }
                let policy = space_policy_ranges.get(space_policy_index)
                    .filter(|(range, _)| range.start <= index).map(|(_, policy)| *policy).or(space_policy);
                let inside_no_wrap = protected.is_some_and(|range| range.start < index
                    || no_wrap_index.checked_sub(1).is_some_and(|previous| no_wrap_ranges[previous].end == index));
                let object = inline_objects.get(object_index).filter(|object| object.index == index);
                let can_break_before = !inside_no_wrap && if layout.is_none() || object.is_some() {
                    true
                } else if let Some(advances) = logical {
                    advances.get(logical_index).is_some_and(|(byte, _)| byte == index)
                } else {
                    glyphs.peek().is_some_and(|glyph| glyph.index == index)
                };
                let advance = layout.map(|_| {
                    if let Some(advances) = logical {
                        index += character.len_utf8();
                        let mut width = Pixels::ZERO;
                        while let Some((_, advance)) = advances.get(logical_index).filter(|(byte, _)| *byte < index) {
                            width += advance; logical_index += 1;
                        }
                        return object.map_or(width, |object| object.width) + if first { padding } else { Pixels::ZERO };
                    }
                    index += character.len_utf8();
                    let mut width = Pixels::ZERO;
                    while glyphs.peek().is_some_and(|glyph| glyph.index < index) {
                        width += glyphs.next().unwrap().advance;
                    }
                    object.map_or(width, |object| object.width) + if first { padding } else { Pixels::ZERO }
                });
                if object.is_some() { object_index += 1; }
                first = false;
                WrapBoundaryCandidate::Char { character, advance, can_break_before, no_wrap: protected.is_some(), space_policy: policy }
            }
        })
    }
}

enum WrapBoundaryCandidate {
    Char {
        no_wrap: bool,
        space_policy: Option<TextSpacePolicy>,
        character: char,
        advance: Option<Pixels>,
        can_break_before: bool,
    },
    Element {
        width: Pixels,
        len_utf8: usize,
        no_wrap: bool,
        can_break_before: bool,
        first_non_whitespace: Option<usize>,
        trailing_hang: Pixels,
    },
}

impl WrapBoundaryCandidate {
    fn hangs(&self, legacy_collapse: bool) -> bool {
        match self {
            Self::Char { character, space_policy: Some(policy), .. } => policy.hangs_at_line_end(*character),
            Self::Char { character: ' ', no_wrap: false, space_policy: None, .. } => legacy_collapse,
            _ => false,
        }
    }

    fn can_break_before(&self) -> bool {
        match self {
            Self::Char { can_break_before, .. } => *can_break_before,
            Self::Element { can_break_before, .. } => *can_break_before,
        }
    }

    fn no_wrap(&self) -> bool {
        match self {
            Self::Char { no_wrap, .. } | Self::Element { no_wrap, .. } => *no_wrap,
        }
    }

    fn first_non_whitespace(&self) -> Option<usize> {
        match self {
            Self::Char { character, .. } => (*character != ' ' && *character != '\n').then_some(0),
            Self::Element { first_non_whitespace, .. } => *first_non_whitespace,
        }
    }

    fn width(&self, width_for_char: &mut impl FnMut(char) -> Pixels) -> Pixels {
        match self {
            Self::Char { character: '\n', .. } => Pixels::ZERO,
            Self::Char { character, advance, .. } => advance.unwrap_or_else(|| width_for_char(*character)),
            Self::Element { width, .. } => *width,
        }
    }

    pub fn len_utf8(&self) -> usize {
        match self {
            WrapBoundaryCandidate::Char { character, .. } => character.len_utf8(),
            WrapBoundaryCandidate::Element { len_utf8: len, .. } => *len,
        }
    }
}

/// A boundary between two lines of text.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Boundary {
    /// The index of the last character in a line
    pub ix: usize,
    /// The indent of the next line.
    pub next_indent: u32,
}

impl Boundary {
    fn new(ix: usize, next_indent: u32) -> Self {
        Self { ix, next_indent }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Font, FontFeatures, FontStyle, FontWeight, TestAppContext, TestDispatcher, font};
    #[cfg(target_os = "macos")]
    use crate::{TextRun, WindowTextSystem, WrapBoundary};

    fn build_wrapper() -> LineWrapper {
        let dispatcher = TestDispatcher::new(0);
        let cx = TestAppContext::build(dispatcher, None);
        let id = cx.text_system().resolve_font(&font(".ZedMono"));
        LineWrapper::new(
            id,
            px(16.),
            cx.text_system().clone(),
            CharWidthCache::default(),
        )
    }

    fn assert_emergency_wrap_preserves_graphemes(wrapper: &mut LineWrapper) {
        use unicode_segmentation::UnicodeSegmentation;

        let mut failures = Vec::new();
        for cluster in ["e\u{301}", "か\u{3099}", "👍🏽", "🇯🇵", "👩\u{200d}💻"] {
            let cluster_width: Pixels = cluster
                .chars()
                .map(|character| wrapper.width_for_char(character))
                .sum();
            let first_width = cluster
                .chars()
                .next()
                .map(|character| wrapper.width_for_char(character))
                .expect("nonempty fixture");
            for text in [cluster.to_owned(), format!("x{cluster}y")] {
                let grapheme_boundaries: Vec<_> = text
                    .grapheme_indices(true)
                    .map(|(index, _)| index)
                    .chain(std::iter::once(text.len()))
                    .collect();
                assert!(
                    LineWrapper::break_indices(&text)
                        .all(|index| grapheme_boundaries.contains(&index)),
                    "UAX #14 boundary inside {text:?}"
                );
                let fragments = [LineFragment::text(&text)];
                let full_width: Pixels = text
                    .chars()
                    .map(|character| wrapper.width_for_char(character))
                    .sum();
                assert!(
                    wrapper
                        .wrap_line(&fragments, full_width + px(1.), IndentAdjustment::NoIndent)
                        .next()
                        .is_none()
                );
                for wrap_width in [
                    first_width - px(0.1),
                    first_width,
                    first_width + px(0.1),
                    cluster_width - px(0.1),
                    cluster_width,
                    cluster_width + px(0.1),
                ] {
                    let boundaries: Vec<_> = wrapper
                        .wrap_line(&fragments, wrap_width, IndentAdjustment::NoIndent)
                        .map(|boundary| boundary.ix)
                        .collect();
                    let split_fragments: Vec<_> = text
                        .split_inclusive(|_: char| true)
                        .map(LineFragment::text)
                        .collect();
                    let split_boundaries: Vec<_> = wrapper
                        .wrap_line(&split_fragments, wrap_width, IndentAdjustment::NoIndent)
                        .map(|boundary| boundary.ix)
                        .collect();
                    assert_eq!(
                        boundaries, split_boundaries,
                        "fragmentation changed {text:?}"
                    );
                    let invalid: Vec<_> = boundaries
                        .iter()
                        .copied()
                        .filter(|index| !grapheme_boundaries.contains(index))
                        .collect();
                    if !invalid.is_empty() {
                        failures.push(format!("text={text:?}, width={wrap_width:?}, cluster_width={cluster_width:?}, breaks={boundaries:?}, inside_grapheme={invalid:?}"));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn test_emergency_wrap_preserves_graphemes() {
        assert_emergency_wrap_preserves_graphemes(&mut build_wrapper());
    }

    #[test]
    fn test_emergency_wrap_exact_boundaries() {
        let mut wrapper = build_wrapper();
        wrapper.char_widths.ascii.fill(Some(px(1.)));
        wrapper.char_widths.other.insert('\u{301}', px(1.));

        for (text, width, adjustment, expected) in [
            (
                "xe\u{301}y",
                1.,
                IndentAdjustment::NoIndent,
                vec![Boundary::new(1, 0), Boundary::new(4, 0)],
            ),
            (
                "  xe\u{301}yze\u{301}q",
                5.,
                IndentAdjustment::SameIndent,
                vec![Boundary::new(6, 2), Boundary::new(8, 2)],
            ),
            (
                "  xe\u{301}yze\u{301}q",
                5.,
                IndentAdjustment::ExtraColumns(1),
                vec![
                    Boundary::new(6, 3),
                    Boundary::new(8, 3),
                    Boundary::new(11, 3),
                ],
            ),
        ] {
            let fragments = [LineFragment::text(text)];
            assert_eq!(
                wrapper
                    .wrap_line(&fragments, px(width), adjustment)
                    .collect::<Vec<_>>(),
                expected,
                "{text:?}, {adjustment:?}",
            );
        }
    }

    #[test]
    fn test_emergency_wrap_elements_and_graphemes() {
        let mut wrapper = build_wrapper();
        wrapper.char_widths.ascii.fill(Some(px(1.)));
        wrapper.char_widths.other.insert('\u{301}', px(1.));

        for element_length in [1, 3, 32] {
            let fragments = [
                LineFragment::text("xe\u{301}"),
                LineFragment::element(px(4.), element_length),
                LineFragment::text("e\u{301}yz"),
            ];
            assert_eq!(
                wrapper
                    .wrap_line(&fragments, px(2.), IndentAdjustment::NoIndent)
                    .collect::<Vec<_>>(),
                vec![
                    Boundary::new(1, 0),
                    Boundary::new(4, 0),
                    Boundary::new(4 + element_length, 0),
                    Boundary::new(7 + element_length, 0),
                ],
                "element_length={element_length}",
            );
        }
    }

    fn generate_test_runs(input_run_len: &[usize]) -> Vec<TextRun> {
        input_run_len
            .iter()
            .map(|run_len| TextRun {
                len: *run_len,
                font: Font {
                    family: "Dummy".into(),
                    features: FontFeatures::default(),
                    fallbacks: None,
                    weight: FontWeight::default(),
                    style: FontStyle::Normal,
                },
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn test_wrap_line() {
        let mut wrapper = build_wrapper();

        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("aa bbb cccc ddddd eeee")],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(12, 0),
                Boundary::new(18, 0)
            ],
        );
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("aaa aaaaaaaaaaaaaaaaaa")],
                    px(72.0),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(4, 0),
                Boundary::new(11, 0),
                Boundary::new(18, 0)
            ],
        );
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("     aaaaaaa")],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 5),
                Boundary::new(9, 5),
                Boundary::new(11, 5),
            ]
        );
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("                            ")],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(14, 0),
                Boundary::new(21, 0)
            ]
        );
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("          aaaaaaaaaaaaaa")],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(14, 3),
                Boundary::new(18, 3),
                Boundary::new(22, 3),
            ]
        );

        // Test wrapping multiple text fragments
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::text("aa bbb "),
                        LineFragment::text("cccc ddddd eeee")
                    ],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(12, 0),
                Boundary::new(18, 0)
            ],
        );

        // Test wrapping with a mix of text and element fragments
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::text("aa "),
                        LineFragment::element(px(20.), 1),
                        LineFragment::text(" bbb "),
                        LineFragment::element(px(30.), 1),
                        LineFragment::text(" cccc")
                    ],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(5, 0),
                Boundary::new(9, 0),
                Boundary::new(11, 0)
            ],
        );

        // Test with element at the beginning and text afterward
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::element(px(50.), 1),
                        LineFragment::text(" aaaa bbbb cccc dddd")
                    ],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(2, 0),
                Boundary::new(7, 0),
                Boundary::new(12, 0),
                Boundary::new(17, 0)
            ],
        );

        // Test with a large element that forces wrapping by itself
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::text("short text "),
                        LineFragment::element(px(100.), 1),
                        LineFragment::text(" more text")
                    ],
                    px(72.),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(6, 0),
                Boundary::new(11, 0),
                Boundary::new(12, 0),
                Boundary::new(18, 0)
            ],
        );

        // Test with non-breaking glue characters
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("a\u{202F}b\u{00A0}c\u{2011}d e")],
                    px(72.0),
                    IndentAdjustment::default()
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(12, 0),], // special chars above take up 3, 2 and 3 bytes, so boundary ends up at 12
        );
    }

    #[test]
    fn test_wrap_line_break_opportunities() {
        let mut wrapper = build_wrapper();
        for (text, fitting_prefix, expected_index) in [
            ("aaa required!", "aaa required", 4),
            ("aaa foo/bar", "aaa foo/", 8),
            ("aaa (hello)", "aaa (hello", 4),
            ("aaa αβγδε", "aaa αβγδ", 4),
            ("aaa a\u{a0}b", "aaa a\u{a0}", 4),
            ("aaa 你好！", "aaa 你好", 7),
        ] {
            let wrap_width = fitting_prefix
                .chars()
                .map(|character| wrapper.width_for_char(character))
                .sum();
            assert_eq!(
                wrapper
                    .wrap_line(
                        &[LineFragment::text(text)],
                        wrap_width,
                        IndentAdjustment::default()
                    )
                    .collect::<Vec<_>>(),
                [Boundary::new(expected_index, 0)],
                "{text:?}",
            );
        }
    }

    #[test]
    fn test_truncate_line_end() {
        let mut wrapper = build_wrapper();

        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &'static str,
            ellipsis: &str,
        ) {
            let dummy_run_lens = vec![text.len()];
            let dummy_runs = generate_test_runs(&dummy_run_lens);
            let (result, dummy_runs) = wrapper.truncate_line(
                text.into(),
                px(220.),
                ellipsis,
                &dummy_runs,
                TruncateFrom::End,
            );
            assert_eq!(result, expected);
            assert_eq!(dummy_runs.first().unwrap().len, result.len());
        }

        perform_test(
            &mut wrapper,
            "aa bbb cccc ddddd eeee ffff gggg",
            "aa bbb cccc ddddd eeee",
            "",
        );
        perform_test(
            &mut wrapper,
            "aa bbb cccc ddddd eeee ffff gggg",
            "aa bbb cccc ddddd eee…",
            "…",
        );
        perform_test(
            &mut wrapper,
            "aa bbb cccc ddddd eeee ffff gggg",
            "aa bbb cccc dddd......",
            "......",
        );
        perform_test(
            &mut wrapper,
            "aa bbb cccc 🦀🦀🦀🦀🦀 eeee ffff gggg",
            "aa bbb cccc 🦀🦀🦀🦀…",
            "…",
        );
    }

    #[test]
    fn test_truncate_line_start() {
        let mut wrapper = build_wrapper();

        #[track_caller]
        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &'static str,
            ellipsis: &str,
        ) {
            let dummy_run_lens = vec![text.len()];
            let dummy_runs = generate_test_runs(&dummy_run_lens);
            let (result, dummy_runs) = wrapper.truncate_line(
                text.into(),
                px(220.),
                ellipsis,
                &dummy_runs,
                TruncateFrom::Start,
            );
            assert_eq!(result, expected);
            assert_eq!(dummy_runs.first().unwrap().len, result.len());
        }

        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc ddddd eeee fff gg",
            "cccc ddddd eeee fff gg",
            "",
        );
        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc ddddd eeee fff gg",
            "…ccc ddddd eeee fff gg",
            "…",
        );
        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc ddddd eeee fff gg",
            "......dddd eeee fff gg",
            "......",
        );
        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc 🦀🦀🦀🦀🦀 eeee fff gg",
            "…🦀🦀🦀🦀 eeee fff gg",
            "…",
        );
    }

    #[test]
    fn test_truncate_multiple_runs_end() {
        let mut wrapper = build_wrapper();

        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &str,
            run_lens: &[usize],
            result_run_len: &[usize],
            line_width: Pixels,
        ) {
            let dummy_runs = generate_test_runs(run_lens);
            let (result, dummy_runs) =
                wrapper.truncate_line(text.into(), line_width, "…", &dummy_runs, TruncateFrom::End);
            assert_eq!(result, expected);
            for (run, result_len) in dummy_runs.iter().zip(result_run_len) {
                assert_eq!(run.len, *result_len);
            }
        }
        // Case 0: Normal
        // Text: abcdefghijkl
        // Runs: Run0 { len: 12, ... }
        //
        // Truncate res: abcd… (truncate_at = 4)
        // Run res: Run0 { string: abcd…, len: 7, ... }
        perform_test(&mut wrapper, "abcdefghijkl", "abcd…", &[12], &[7], px(50.));
        // Case 1: Drop some runs
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdef… (truncate_at = 6)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: ef…, len:
        // 5, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "abcdef…",
            &[4, 4, 4],
            &[4, 5],
            px(70.),
        );
        // Case 2: Truncate at start of some run
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdefgh… (truncate_at = 8)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: efgh, len:
        // 4, ... }, Run2 { string: …, len: 3, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "abcdefgh…",
            &[4, 4, 4],
            &[4, 4, 3],
            px(90.),
        );
    }

    #[test]
    fn test_truncate_multiple_runs_start() {
        let mut wrapper = build_wrapper();

        #[track_caller]
        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &str,
            run_lens: &[usize],
            result_run_len: &[usize],
            line_width: Pixels,
        ) {
            let dummy_runs = generate_test_runs(run_lens);
            let (result, dummy_runs) = wrapper.truncate_line(
                text.into(),
                line_width,
                "…",
                &dummy_runs,
                TruncateFrom::Start,
            );
            assert_eq!(result, expected);
            for (run, result_len) in dummy_runs.iter().zip(result_run_len) {
                assert_eq!(run.len, *result_len);
            }
        }
        // Case 0: Normal
        // Text: abcdefghijkl
        // Runs: Run0 { len: 12, ... }
        //
        // Truncate res: …ijkl (truncate_at = 9)
        // Run res: Run0 { string: …ijkl, len: 7, ... }
        perform_test(&mut wrapper, "abcdefghijkl", "…ijkl", &[12], &[7], px(50.));
        // Case 1: Drop some runs
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: …ghijkl (truncate_at = 7)
        // Runs res: Run0 { string: …gh, len: 5, ... }, Run1 { string: ijkl, len:
        // 4, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "…ghijkl",
            &[4, 4, 4],
            &[5, 4],
            px(70.),
        );
        // Case 2: Truncate at start of some run
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdefgh… (truncate_at = 3)
        // Runs res: Run0 { string: …, len: 3, ... }, Run1 { string: efgh, len:
        // 4, ... }, Run2 { string: ijkl, len: 4, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "…efghijkl",
            &[4, 4, 4],
            &[3, 4, 4],
            px(90.),
        );
    }

    #[test]
    fn test_update_run_after_truncation_end() {
        fn perform_test(result: &str, run_lens: &[usize], result_run_lens: &[usize]) {
            let mut dummy_runs = generate_test_runs(run_lens);
            update_runs_after_truncation(result, "…", &mut dummy_runs, TruncateFrom::End);
            for (run, result_len) in dummy_runs.iter().zip(result_run_lens) {
                assert_eq!(run.len, *result_len);
            }
        }
        // Case 0: Normal
        // Text: abcdefghijkl
        // Runs: Run0 { len: 12, ... }
        //
        // Truncate res: abcd… (truncate_at = 4)
        // Run res: Run0 { string: abcd…, len: 7, ... }
        perform_test("abcd…", &[12], &[7]);
        // Case 1: Drop some runs
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdef… (truncate_at = 6)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: ef…, len:
        // 5, ... }
        perform_test("abcdef…", &[4, 4, 4], &[4, 5]);
        // Case 2: Truncate at start of some run
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdefgh… (truncate_at = 8)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: efgh, len:
        // 4, ... }, Run2 { string: …, len: 3, ... }
        perform_test("abcdefgh…", &[4, 4, 4], &[4, 4, 3]);
    }

    // For compatibility with the test macro
    #[cfg(target_os = "macos")]
    use crate as gpui;

    // These seem to vary wildly based on the text system.
    #[cfg(target_os = "macos")]
    #[crate::test]
    fn test_wrap_shaped_line(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let text_system = WindowTextSystem::new(cx.text_system().clone());

            let normal = TextRun {
                len: 0,
                font: font("Helvetica"),
                color: Default::default(),
                underline: Default::default(),
                ..Default::default()
            };
            let bold = TextRun {
                len: 0,
                font: font("Helvetica").bold(),
                ..Default::default()
            };

            let text = "aa bbb cccc ddddd eeee".into();
            let lines = text_system
                .shape_text(
                    text,
                    px(16.),
                    &[
                        normal.with_len(4),
                        bold.with_len(5),
                        normal.with_len(6),
                        bold.with_len(1),
                        normal.with_len(7),
                    ],
                    Some(px(72.)),
                    None,
                )
                .unwrap();

            assert_eq!(
                lines[0].layout.wrap_boundaries(),
                &[
                    WrapBoundary {
                        run_ix: 0,
                        glyph_ix: 7
                    },
                    WrapBoundary {
                        run_ix: 0,
                        glyph_ix: 12
                    },
                    WrapBoundary {
                        run_ix: 0,
                        glyph_ix: 18
                    }
                ],
            );
        });
    }

    #[test]
    fn test_multiline_truncation_fits_within_wrapped_lines() {
        let mut wrapper = build_wrapper();

        // With .ZedMono at 16px, each char is 9.6px wide.
        // wrap_width = 72px fits ~7 chars per line.
        //
        // "aa bbbbbb cccccc dddddd eeee ffff" with wrap_width=72px wraps as:
        //   Line 1: "aa "       (28.8px, wraps because "bbbbbb" won't fit)
        //   Line 2: "bbbbbb "   (67.2px)
        //   Line 3: "cccccc "   (67.2px)
        //   ...
        //
        // truncate_wrapped_line should wrap first to find line 2 starts at
        // "bbbbbb...", then truncate only that line to fit with ellipsis.
        let text: &str = "aa bbbbbb cccccc dddddd eeee ffff";
        let wrap_width = px(72.);
        let max_lines: usize = 2;

        let runs = generate_test_runs(&[text.len()]);
        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "\u{2026}",
            &runs,
            TruncateFrom::End,
        );

        // The truncated text, when wrapped, must fit within max_lines lines.
        let wrap_count = wrapper
            .wrap_line(
                &[LineFragment::text(&truncated)],
                wrap_width,
                IndentAdjustment::default(),
            )
            .count();

        assert!(
            wrap_count < max_lines,
            "Truncated text '{}' wraps into {} visual lines, expected at most {}",
            truncated,
            wrap_count + 1,
            max_lines
        );

        // The truncated text should end with the ellipsis.
        assert!(
            truncated.ends_with('\u{2026}'),
            "Truncated text '{}' should end with ellipsis",
            truncated
        );
    }

    #[test]
    fn test_multiline_truncation_no_truncation_needed() {
        let mut wrapper = build_wrapper();

        // Text that fits in 2 lines shouldn't be truncated.
        // Line 1: "aa bbb " (67.2px), Line 2: "cccccc" (57.6px)
        let text: &str = "aa bbb cccccc";
        let wrap_width = px(72.);
        let max_lines: usize = 2;

        let runs = generate_test_runs(&[text.len()]);
        let (result, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "\u{2026}",
            &runs,
            TruncateFrom::End,
        );

        assert_eq!(
            result.as_ref(),
            text,
            "Text that fits should not be modified"
        );
    }

    #[test]
    fn test_multiline_truncation_three_lines() {
        let mut wrapper = build_wrapper();

        let text: &str = "aa bbb cccc ddddd eeee ffff gggg hhhh iiii jjjj";
        let wrap_width = px(72.);
        let max_lines: usize = 3;

        let runs = generate_test_runs(&[text.len()]);
        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "\u{2026}",
            &runs,
            TruncateFrom::End,
        );

        let wrap_count = wrapper
            .wrap_line(
                &[LineFragment::text(&truncated)],
                wrap_width,
                IndentAdjustment::default(),
            )
            .count();

        assert!(
            wrap_count < max_lines,
            "Truncated text '{}' wraps into {} visual lines, expected at most {}",
            truncated,
            wrap_count + 1,
            max_lines
        );

        assert!(
            truncated.ends_with('\u{2026}'),
            "Truncated text '{}' should end with ellipsis",
            truncated
        );
    }

    #[test]
    fn test_multiline_truncation_preserves_breaks_after_empty_lines() {
        let mut wrapper = build_wrapper();
        for prefix in ["", "\n", "\n\n", "hello\n\n"] {
            let text = format!("{prefix}aaa required! more words follow");
            let runs = generate_test_runs(&[text.len()]);
            let max_lines = prefix.bytes().filter(|byte| *byte == b'\n').count() + 2;
            let (truncated, _) = wrapper.truncate_wrapped_line(
                text.into(),
                px(72.),
                max_lines,
                "…",
                &runs,
                TruncateFrom::End,
            );
            assert_eq!(
                truncated.as_ref(),
                format!("{prefix}aaa {}…", &"required!"[..6]),
                "prefix: {prefix:?}",
            );
        }
    }

    #[test]
    fn test_multiline_truncation_with_newlines() {
        let mut wrapper = build_wrapper();

        // "hello\nworld foo bar baz" with line_clamp(2):
        // shape_text splits on \n, giving physical lines "hello" and
        // "world foo bar baz". The newline consumes line 1, so the
        // second physical line should be truncated on line 2.
        let text: &str = "hello\nworld foo bar baz";
        let wrap_width = px(72.);
        let max_lines: usize = 2;

        let runs = generate_test_runs(&[text.len()]);
        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "\u{2026}",
            &runs,
            TruncateFrom::End,
        );

        // The newline should be preserved.
        let parts: Vec<&str> = truncated.splitn(2, '\n').collect();
        assert_eq!(
            parts.len(),
            2,
            "Newline should be preserved: '{}'",
            truncated
        );
        assert_eq!(parts[0], "hello");

        // The second line should fit within wrap_width and end with ellipsis.
        let second_line_width: Pixels = parts[1].chars().map(|c| wrapper.width_for_char(c)).sum();
        assert!(
            second_line_width <= wrap_width,
            "Second line '{}' ({}px) exceeds wrap_width ({}px)",
            parts[1],
            second_line_width,
            wrap_width
        );
        assert!(
            truncated.ends_with('\u{2026}'),
            "Should end with ellipsis: '{}'",
            truncated
        );
    }

    #[test]
    fn test_multiline_truncation_newline_on_last_line() {
        let mut wrapper = build_wrapper();

        // "hello\nworld\nmore" with line_clamp(2):
        // Line 1: "hello", Line 2: "world" — but there's a third line,
        // so line 2 should be truncated with ellipsis.
        let text: &str = "hello\nworld\nmore";
        let wrap_width = px(72.);
        let max_lines: usize = 2;

        let runs = generate_test_runs(&[text.len()]);
        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "\u{2026}",
            &runs,
            TruncateFrom::End,
        );

        let parts: Vec<&str> = truncated.splitn(2, '\n').collect();
        assert_eq!(parts[0], "hello");
        assert!(
            truncated.ends_with('\u{2026}'),
            "Should end with ellipsis since there's more content: '{}'",
            truncated
        );
    }

    #[test]
    fn test_truncate_line_middle() {
        let mut wrapper = build_wrapper();

        // No truncation when text fits within a very wide budget.
        let short_text = "hello world";
        let runs = generate_test_runs(&[short_text.len()]);
        let (result, result_runs) = wrapper.truncate_line(
            short_text.into(),
            px(10000.),
            "…",
            &runs,
            TruncateFrom::Middle,
        );
        assert_eq!(result.as_ref(), short_text);
        assert_eq!(result_runs.len(), 1);
        assert_eq!(result_runs[0].len, short_text.len());

        // Basic middle truncation: long string with px(100.) budget.
        let long_text = "abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz";
        let runs = generate_test_runs(&[long_text.len()]);
        let (result, _result_runs) =
            wrapper.truncate_line(long_text.into(), px(100.), "…", &runs, TruncateFrom::Middle);
        assert!(
            result.contains('…'),
            "Middle-truncated result should contain '…', got: '{}'",
            result
        );
        assert!(
            result.chars().count() < long_text.chars().count(),
            "Middle-truncated result should be shorter than original"
        );
        assert_eq!(
            result.chars().next(),
            long_text.chars().next(),
            "Result should start with the same first character as original"
        );
        assert_eq!(
            result.chars().last(),
            long_text.chars().last(),
            "Result should end with the same last character as original"
        );

        // Degenerate case: budget so narrow that middle truncation cannot find a valid split.
        // Still show the truncation affix instead of returning the original overflowing text.
        let text = "abcdef";
        let runs = generate_test_runs(&[text.len()]);
        let (result, result_runs) =
            wrapper.truncate_line(text.into(), px(1.), "…", &runs, TruncateFrom::Middle);
        assert_eq!(result.as_ref(), "…");
        assert_eq!(result_runs.len(), 1);
        assert_eq!(result_runs[0].len, "…".len());

        // Run adjustment correctness: multiple runs across the string.
        // Verify that the returned runs' lengths sum to result.len().
        let multi_run_text = "abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz";
        let run_lens = [20, 20, multi_run_text.len() - 40];
        let runs = generate_test_runs(&run_lens);
        let (result, result_runs) = wrapper.truncate_line(
            multi_run_text.into(),
            px(100.),
            "…",
            &runs,
            TruncateFrom::Middle,
        );
        let total_run_len: usize = result_runs.iter().map(|r| r.len).sum();
        assert_eq!(
            total_run_len,
            result.len(),
            "Sum of run lengths ({}) should equal result byte length ({})",
            total_run_len,
            result.len()
        );
    }

    #[test]
    fn test_multiline_truncation_trailing_newline() {
        let mut wrapper = build_wrapper();

        // "hello\nworld\n" with line_clamp(2):
        // The trailing newline has no content after it, so no ellipsis.
        let text: &str = "hello\nworld\n";
        let wrap_width = px(72.);
        let max_lines: usize = 2;

        let runs = generate_test_runs(&[text.len()]);
        let (result, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "\u{2026}",
            &runs,
            TruncateFrom::End,
        );

        assert!(
            !result.ends_with('\u{2026}'),
            "Trailing newline with no content should not add ellipsis: '{}'",
            result
        );
    }

    #[test]
    fn test_multiline_truncation_newline_fits_exactly() {
        let mut wrapper = build_wrapper();

        // "hello\nworld" with line_clamp(2):
        // Exactly 2 lines, no truncation needed.
        let text: &str = "hello\nworld";
        let wrap_width = px(72.);
        let max_lines: usize = 2;

        let runs = generate_test_runs(&[text.len()]);
        let (result, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "\u{2026}",
            &runs,
            TruncateFrom::End,
        );

        assert_eq!(
            result.as_ref(),
            text,
            "Text that fits exactly should not be modified: '{}'",
            result
        );
    }

    #[test]
    fn test_extra_columns_overflow_guard() {
        let mut wrapper = build_wrapper();
        let space_width = wrapper.width_for_char(' ');

        // 6 spaces indent, wrap width 10 columns.
        let text = "      ab cd ef gh";
        let wrap_width = space_width * 10.0;

        // When base_indent + extra overflows wrap width (6 + 8 + 2 > 10),
        // indent must fall back to 0 instead of degrading to one character per row.
        //
        // Expected wrapped lines (10 columns):
        //   |      ab  |  (row 0: 6 spaces + "ab ", len 9)
        //   |cd ef gh  |  (row 1: 0 indent + "cd ef gh", len 8)
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text(text)],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(8),
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(9, 0)]
        );

        // When base_indent + extra fits within wrap width (6 + 2 + 2 <= 10),
        // the extra indent is applied and not clamped.
        // Spaces count toward the width, so the separators occupy their own rows
        // rather than overflowing the preceding row or splitting the following word.
        //
        // Expected wrapped lines (10 columns):
        //   |      ab  |  (row 0: 6 spaces + "ab ", len 9)
        //   |        cd|  (row 1: 8 spaces + "cd", len 10)
        //   |          |  (row 2: 8 spaces + " ", len 9)
        //   |        ef|  (row 3: 8 spaces + "ef", len 10)
        //   |          |  (row 4: 8 spaces + " ", len 9)
        //   |        gh|  (row 5: 8 spaces + "gh", len 10)
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text(text)],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(2),
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(9, 8),
                Boundary::new(11, 8),
                Boundary::new(12, 8),
                Boundary::new(14, 8),
                Boundary::new(15, 8),
            ]
        );

        // When text contains full-width (two-column) glyphs, reserving headroom for a
        // two-column character (candidate + 2 > wrap_width) ensures that candidate
        // indents leaving only 1 column cannot accept the indent and overflow.
        //
        // 1 space indent, wrap width 10 columns.
        // " 🦀🦀🦀🦀🦀🦀🦀🦀" with extra=8 (extra_two with tab_size=4):
        // candidate = 1 + 8 = 9.
        // With +2 headroom (9 + 2 > 10), indent falls back to 0.
        //
        // Expected wrapped lines (10 columns):
        //   | 🦀🦀🦀🦀 |  (row 0: 1 space + 4 two-column glyphs, 9 cols)
        //   |🦀🦀🦀🦀  |  (row 1: 0 indent + 4 two-column glyphs, 8 cols)
        let full_width_text = " 🦀🦀🦀🦀🦀🦀🦀🦀";
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text(full_width_text)],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(8),
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(17, 0)]
        );

        // When the carried suffix fits within wrap width alongside extra indent
        // (1 indent + 9 carried suffix = 10 <= 10), the extra indent is applied.
        //
        // Expected wrapped lines (10 columns):
        //   |a         |  (row 0: "a ", len 2)
        //   | abcdefghi|  (row 1: 1 indent + "abcdefghi", len 10)
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("a abcdefghi")],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(1),
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(2, 1)]
        );

        // When extra indent exceeds wrap width by even 1 column
        // (2 indent + 9 carried suffix = 11 > 10), indent falls back to 0.
        //
        // Expected wrapped lines (10 columns):
        //   |a         |  (row 0: "a ", len 2)
        //   |abcdefghi |  (row 1: 0 indent + "abcdefghi", len 9)
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("a abcdefghi")],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(2),
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(2, 0)]
        );

        // When text wraps at an earlier word boundary, the carried suffix
        // must be accounted for so indent + carried_suffix <= wrap_width.
        //
        // "a abcdefghij" with wrap_width 10 columns, ExtraColumns(8):
        // candidate = 0 + 8 = 8.
        // carried suffix = "abcdefghi" (9 columns).
        // 8 indent + 9 carried suffix = 17 > 10, so indent falls back to 0.
        //
        // Expected wrapped lines (10 columns):
        //   |a         |  (row 0: "a ", len 2)
        //   |abcdefghij|  (row 1: 0 indent + "abcdefghij", len 10)
        let carried_text = "a abcdefghij";
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text(carried_text)],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(8),
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(2, 0)]
        );

        // When a wider inline element is encountered after the first wrap,
        // the overflow guard must evaluate whether the element fits with the extra indent.
        // If text continues after the element row and wraps again, subsequent continuation
        // lines resume the extra indent if their carried content fits.
        //
        // "abcdefghijk " followed by an 8-column element and "z 12":
        // Row 0: "abcdefghij" (len 10)
        // Row 1: "k " (2 cols) with 8 indent (len 10)
        // Row 2: element (8 cols) cannot fit with 8 indent (8 + 8 = 16 > 10),
        //        so indent falls back to 0. Element (8 cols) + "z " (2 cols) = len 10.
        // Row 3: "12" (2 cols) fits with 8 indent (8 + 2 = 10 <= 10).
        //
        // Expected wrapped lines (10 columns):
        //   |abcdefghij|  (row 0: 10 cols)
        //   |        k |  (row 1: 8 indent + "k ", len 10)
        //   |[ELEMENT]z|  (row 2: 0 indent + [ELEMENT (8)] + "z ", len 10)
        //   |        12|  (row 3: 8 indent + "12", len 10)
        let element_fragments = [
            LineFragment::text("abcdefghijk "),
            LineFragment::element(space_width * 8.0, 1),
            LineFragment::text("z 12"),
        ];
        assert_eq!(
            wrapper
                .wrap_line(
                    &element_fragments,
                    wrap_width,
                    IndentAdjustment::ExtraColumns(8),
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(10, 8),
                Boundary::new(12, 0),
                Boundary::new(15, 8),
            ]
        );

        // A line of only whitespace wrapping before any non-whitespace character
        // must not have extra columns added to subsequent rows.
        let spaces = "                    "; // 20 spaces
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text(spaces)],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(8),
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(10, 0)]
        );

        // When leading whitespace wraps before the first non-whitespace character,
        // base_indent should reflect the leading whitespace on the row where non-whitespace begins.
        // 14 spaces followed by "ab cd ef gh", wrap width 10:
        // Row 0: 10 spaces (len 10) -> wraps at ix 10 with indent 0
        // Row 1: 4 spaces + "ab cd" (len 9) -> wraps at ix 20
        //        base_indent is 14 - 10 = 4.
        //        With ExtraColumns(2), candidate = 4 + 2 = 6. 6 + 2 (headroom) = 8 <= 10.
        //        Row 2 and subsequent rows get indent 6.
        let multiline_indent_text = "              ab cd ef gh";
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text(multiline_indent_text)],
                    wrap_width,
                    IndentAdjustment::ExtraColumns(2),
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(10, 0),
                Boundary::new(20, 6),
                Boundary::new(23, 6),
            ]
        );

        // When a word boundary precedes an oversized carried suffix that itself
        // exceeds wrap_width (e.g. text followed by an inline element), wrapping
        // at the earlier candidate would force the continuation row to immediately
        // overflow even with 0 indent. The wrapper must reject the candidate and
        // break before the overflowing item instead.
        //
        // "aaaaaaaaaaaaaaaaaaa a" (21 chars: 19 'a's, space, 'a') followed by 10-column element:
        // Row 0: "aaaaaaaaaa" (10 chars, len 10) -> wraps at ix 10
        // Row 1: "aaaaaaaa" (8 chars) with 2 indent (len 10) -> wraps at ix 18
        // Row 2: "a a" (3 chars) with 2 indent (len 5)
        //        candidate boundary at ix 20 (space)
        //        Then element (10 cols): width becomes 5 + 10 = 15 > wrap_width (10).
        //        Candidate would carry 'a' (1 col) + element (10 cols) = 11 cols > 10.
        //        Because carried suffix (11) > wrap_width (10), candidate is rejected.
        //        Wrapper breaks at ix 21 (before element) with indent 0.
        // Row 3: element (10 cols, len 10)
        let oversized_suffix_fragments = [
            LineFragment::text("aaaaaaaaaaaaaaaaaaa a"),
            LineFragment::element(space_width * 10.0, 1),
        ];
        assert_eq!(
            wrapper
                .wrap_line(
                    &oversized_suffix_fragments,
                    wrap_width,
                    IndentAdjustment::ExtraColumns(2),
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(10, 2),
                Boundary::new(18, 2),
                Boundary::new(21, 0),
            ]
        );

        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("  aaaaaaaa")],
                    space_width * 4.0,
                    IndentAdjustment::ExtraColumns(u32::MAX),
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(4, 0), Boundary::new(8, 0)]
        );
    }
}
