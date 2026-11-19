//! Presentation-only bidi controls. Public indices always address the caller's
//! source; the native shaper and UAX #9 share one isolated presentation source.

use std::{ops::Range, sync::Arc};
use crate::{FontRun, LineLayout, SharedString, TextDirection};

/// A directional isolate in the original UTF-8 source. Ranges must be
/// properly nested or disjoint, and supplied in opening order. Explicit parent
/// indices preserve empty scopes at an ancestor's start or end. Each range
/// belongs to one bidi paragraph; callers split scopes at paragraph breaks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextIsolation {
    /// Original UTF-8 byte range.
    pub range: Range<usize>,
    /// LRI, RLI, or first-strong isolation for `Auto`.
    pub direction: TextDirection,
    /// Index of the immediately enclosing isolate in the same input slice.
    /// It precedes this entry; `None` establishes a sibling at paragraph level.
    pub parent: Option<usize>,
}

/// Shared native isolation context. Slices retain the enclosing paragraph's
/// controls and merely change their original source window.
#[derive(Clone, Debug)]
pub struct TextIsolationContext {
    pub(crate) source: Arc<IsolationSource>,
    pub(crate) range: Range<usize>,
    pub(crate) direction: TextDirection,
}

impl TextIsolationContext {
    pub(crate) fn detected_direction(&self) -> Option<TextDirection> { self.source.detected_direction }

    pub(crate) fn subrange(&self, range: Range<usize>) -> Self {
        assert!(range.start <= range.end && range.end <= self.range.len());
        Self { source: self.source.clone(), range: self.range.start + range.start..self.range.start + range.end, direction: self.direction }
    }

    pub(crate) fn presentation_range(&self, range: Range<usize>) -> Range<usize> {
        self.source.presentation_boundary(self.range.start + range.start)
            ..self.source.presentation_boundary(self.range.start + range.end)
    }

    pub(crate) fn original_range(&self, range: Range<usize>) -> Range<usize> {
        self.source.original_index(range.start) - self.range.start
            ..self.source.original_index(range.end) - self.range.start
    }
}

#[derive(Debug)]
struct Boundary {
    original: usize,
    before: usize,
    split: usize,
    after: usize,
}

#[derive(Debug)]
pub(crate) struct IsolationSource {
    pub(crate) text: SharedString,
    boundaries: Box<[Boundary]>,
    original_len: usize,
    detected_direction: Option<TextDirection>,
}

impl IsolationSource {
    pub(crate) fn new(text: &str, isolates: &[TextIsolation]) -> Self {
        let extra = isolates.len().checked_mul(6).expect("isolate presentation source length overflow");
        let capacity = text.len().checked_add(extra).expect("isolate presentation source length overflow");
        let mut presentation = String::with_capacity(capacity);
        let mut boundaries: Vec<Boundary> = Vec::with_capacity(isolates.len() * 2);
        let mut stack: Vec<(usize, usize)> = Vec::new();
        // Classify source separators once, rather than rescanning a long
        // paragraph for every nested scope. Ordinary paragraphs allocate none.
        let separators: Vec<_> = text.char_indices()
            .filter_map(|(index, ch)| (unicode_bidi::bidi_class(ch) == unicode_bidi::BidiClass::B).then_some(index))
            .collect();
        let mut original = 0;
        let mut control = |at: usize, value: char, opening: bool| {
            if boundaries.last().is_none_or(|boundary| boundary.original != at) {
                presentation.push_str(&text[original..at]);
                original = at;
                let position = presentation.len();
                boundaries.push(Boundary { original: at, before: position, split: position, after: position });
            }
            presentation.push(value);
            let boundary = boundaries.last_mut().expect("isolation boundary was inserted");
            boundary.after = presentation.len();
            if !opening { boundary.split = boundary.after; }
        };
        let mut previous_start = 0;
        for (index, isolate) in isolates.iter().enumerate() {
            let range = &isolate.range;
            assert!(range.start <= range.end && range.end <= text.len()
                && text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
                "isolate ranges must be valid UTF-8 source ranges");
            assert!(range.start >= previous_start, "isolate ranges must be in opening order");
            let separator = separators.partition_point(|index| *index < range.start);
            assert!(separators.get(separator).is_none_or(|index| *index >= range.end),
                "isolate ranges must not cross paragraph separators");
            // Parent identity distinguishes a zero-length child at its
            // ancestor's end from an empty following sibling at the same byte.
            assert!(isolate.parent.is_none_or(|parent| parent < index), "isolate parents must precede children");
            while stack.last().map(|(parent, _)| *parent) != isolate.parent {
                let (_, end) = stack.pop().expect("isolate parent must be an active ancestor");
                assert!(end <= range.start, "isolate siblings must not overlap");
                control(end, '\u{2069}', false);
            }
            assert!(stack.last().is_none_or(|(_, end)| range.end <= *end), "isolate ranges must not cross");
            control(range.start, match isolate.direction {
                TextDirection::Auto => '\u{2068}',
                TextDirection::Ltr => '\u{2066}',
                TextDirection::Rtl => '\u{2067}',
            }, true);
            stack.push((index, range.end));
            previous_start = range.start;
        }
        while let Some((_, end)) = stack.pop() { control(end, '\u{2069}', false); }
        presentation.push_str(&text[original..]);
        let detected_direction = TextDirection::detect_fragments(std::iter::once(presentation.as_str()));
        Self { text: presentation.into(), boundaries: boundaries.into_boxed_slice(), original_len: text.len(), detected_direction }
    }

    // A soft-row boundary follows the last closer at that original byte;
    // any subsequent openers belong to the next source window. Empty nested
    // scopes may close after an opener; the shared full-paragraph context
    // still carries its ancestry across both contiguous source windows.
    fn presentation_boundary(&self, original: usize) -> usize {
        let next = self.boundaries.partition_point(|boundary| boundary.original <= original);
        match next.checked_sub(1).map(|index| &self.boundaries[index]) {
            Some(boundary) if boundary.original == original => boundary.split,
            Some(boundary) => original + boundary.after - boundary.original,
            None => original,
        }
    }

    fn original_index(&self, presentation: usize) -> usize {
        let next = self.boundaries.partition_point(|boundary| boundary.before <= presentation);
        match next.checked_sub(1).map(|index| &self.boundaries[index]) {
            Some(boundary) if presentation < boundary.after => boundary.original,
            Some(boundary) => boundary.original + presentation - boundary.after,
            None => presentation,
        }
    }

    pub(crate) fn font_runs(&self, runs: &[FontRun]) -> Vec<FontRun> {
        let mut original = 0;
        let mut presentation = 0;
        runs.iter().map(|run| {
            original += run.len;
            let end = self.presentation_boundary(original);
            let mapped = FontRun { len: end - presentation, font_id: run.font_id, font_size: run.font_size };
            presentation = end;
            mapped
        }).collect()
    }

    pub(crate) fn restore_layout(self: &Arc<Self>, layout: &mut LineLayout, direction: TextDirection) {
        // Native clusters can start on a control but still own real text.
        // Retain precisely those clusters with a nonempty original interval.
        let mut boundaries = layout.runs.iter().flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index)).collect::<Vec<_>>();
        let presentation_len = layout.len;
        boundaries.push(presentation_len);
        boundaries.sort_unstable();
        boundaries.dedup();
        for run in &mut layout.runs {
            run.glyphs.retain_mut(|glyph| {
                let end = boundaries.partition_point(|index| *index <= glyph.index);
                let end = boundaries.get(end).copied().unwrap_or(presentation_len);
                let original = self.original_index(glyph.index);
                if original == self.original_index(end) { return false; }
                glyph.index = original;
                true
            });
        }
        layout.runs.retain(|run| !run.glyphs.is_empty());
        layout.interior_carets.retain_mut(|caret| {
            caret.index = self.original_index(caret.index);
            caret.cluster_start = self.original_index(caret.cluster_start);
            caret.index > caret.cluster_start && caret.index < self.original_len
        });
        layout.interior_carets.sort_unstable_by_key(|caret| caret.index);
        layout.interior_carets.dedup_by_key(|caret| caret.index);
        layout.len = self.original_len;
        layout.isolation = Some(TextIsolationContext { source: self.clone(), range: 0..self.original_len, direction });
    }
}
