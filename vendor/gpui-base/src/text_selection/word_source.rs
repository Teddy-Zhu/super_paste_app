use std::borrow::Cow;

use gpui::{Bounds, CaretAffinity, Pixels};

/// Immutable, source-ordered text of one inline flow. Replaced objects occupy
/// one U+FFFC segment. Implementations share model strings, not painted rows.
pub(crate) trait WordSelectionSource {
    fn segment_count(&self) -> usize;
    fn segment(&self, index: usize) -> Option<&str>;

    /// Exact edges of a replaced object registered in the same frame. Sources
    /// without that geometry return None rather than approximating a text caret.
    fn object_caret(&self, _segment: usize, _offset: usize) -> Option<Bounds<Pixels>> { None }
}

#[derive(Clone, Copy)]
pub(super) struct SourcePoint {
    pub segment: usize,
    pub offset: usize,
}

/// Resolve a word before projecting it onto laid-out fragments. Dictionary
/// context crosses style boundaries; objects and hard line breaks retain their
/// Unicode boundary semantics. Only a context crossing source strings allocates.
pub(super) fn word_range(
    source: &dyn WordSelectionSource,
    target: SourcePoint,
) -> Option<(SourcePoint, SourcePoint)> {
    let text = source.segment(target.segment)?;
    let character = text.get(target.offset..)?.chars().next()?;
    if matches!(character, '\r' | '\n') {
        return Some((target, SourcePoint {
            offset: target.offset + character.len_utf8(),
            ..target
        }));
    }

    // ASCII word characters cannot belong to an ignored-mark/whitespace group.
    // Include the neighboring separators rather than trimming Unicode context.
    let bounded = character.is_ascii_alphanumeric() || character == '_';
    let (start, end) = context_bounds(source, target, bounded)?;

    let (text, relative) = context(source, start, end, target)?;
    let range = gpui::word_ranges(text.as_ref())
        .find(|range| range.contains(&relative))?;
    Some((
        point_at(source, start, end, range.start, CaretAffinity::Downstream)?,
        point_at(source, start, end, range.end, CaretAffinity::Upstream)?,
    ))
}

fn context_bounds(
    source: &dyn WordSelectionSource,
    target: SourcePoint,
    bounded: bool,
) -> Option<(SourcePoint, SourcePoint)> {
    // Search within each borrowed source segment. Non-ASCII line context uses
    // byte searches for LF, without repeatedly decoding its scalar values or
    // calling through the source interface for every character.
    let mut start = target;
    loop {
        let text = source.segment(start.segment)?.get(..start.offset)?;
        let boundary = if bounded {
            text.char_indices().rfind(|(_, ch)| ch.is_whitespace()).map(|(at, _)| at)
        } else {
            text.rfind('\n').map(|at| at + 1)
        };
        if let Some(offset) = boundary {
            start.offset = offset;
            break;
        }
        start.offset = 0;
        if start.segment == 0 { break; }
        start.segment -= 1;
        start.offset = source.segment(start.segment)?.len();
    }
    let mut end = target;
    loop {
        let text = source.segment(end.segment)?;
        let remainder = text.get(end.offset..)?;
        let boundary = if bounded {
            remainder.char_indices().find(|(_, ch)| ch.is_whitespace())
                .map(|(at, ch)| at + ch.len_utf8())
        } else {
            remainder.find('\n')
        };
        if let Some(offset) = boundary {
            end.offset += offset;
            break;
        }
        end.offset = text.len();
        if end.segment + 1 >= source.segment_count() { break; }
        end.segment += 1;
        end.offset = 0;
    }
    Some((start, end))
}

fn chunk(
    source: &dyn WordSelectionSource,
    start: SourcePoint,
    end: SourcePoint,
    index: usize,
) -> Option<&str> {
    let text = source.segment(index)?;
    let from = if index == start.segment { start.offset } else { 0 };
    let to = if index == end.segment { end.offset } else { text.len() };
    text.get(from..to)
}

fn context(
    source: &dyn WordSelectionSource,
    start: SourcePoint,
    end: SourcePoint,
    target: SourcePoint,
) -> Option<(Cow<'_, str>, usize)> {
    if start.segment == end.segment {
        return Some((
            Cow::Borrowed(chunk(source, start, end, start.segment)?),
            target.offset - start.offset,
        ));
    }

    let mut len = 0;
    let mut relative = None;
    let mut single_chunk = None;
    let mut nonempty_chunks = 0;
    for index in start.segment..=end.segment {
        if index == target.segment {
            let from = if index == start.segment { start.offset } else { 0 };
            relative = Some(len + target.offset - from);
        }
        let chunk = chunk(source, start, end, index)?;
        if !chunk.is_empty() {
            single_chunk = Some(chunk);
            nonempty_chunks += 1;
        }
        len += chunk.len();
    }
    if nonempty_chunks == 1 {
        return Some((Cow::Borrowed(single_chunk?), relative?));
    }
    let mut text = String::with_capacity(len);
    for index in start.segment..=end.segment {
        text.push_str(chunk(source, start, end, index)?);
    }
    Some((Cow::Owned(text), relative?))
}

fn point_at(
    source: &dyn WordSelectionSource,
    start: SourcePoint,
    end: SourcePoint,
    mut offset: usize,
    affinity: CaretAffinity,
) -> Option<SourcePoint> {
    for index in start.segment..=end.segment {
        let len = chunk(source, start, end, index)?.len();
        if offset < len || (offset == len && len > 0 && affinity == CaretAffinity::Upstream) {
            let from = if index == start.segment { start.offset } else { 0 };
            return Some(SourcePoint { segment: index, offset: from + offset });
        }
        offset = offset.checked_sub(len)?;
    }
    (offset == 0).then_some(end)
}
