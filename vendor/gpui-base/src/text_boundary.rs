use std::ops::Range;

pub(crate) fn word_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = clip_offset_left(text, offset);
    let character = text[offset..].chars().next()?;
    if matches!(character, '\n' | '\r') {
        return Some(offset..offset + character.len_utf8());
    }
    let line = if character.is_ascii_alphanumeric() || character == '_' {
        // Preserve Unicode boundary context without scanning the entire prefix
        // of a long line when the hit belongs to a short ASCII word.
        let start = text[..offset].char_indices()
            .rfind(|(_, ch)| ch.is_whitespace())
            .map_or(0, |(at, _)| at);
        let end = text[offset..].char_indices()
            .find(|(_, ch)| ch.is_whitespace())
            .map_or(text.len(), |(at, ch)| offset + at + ch.len_utf8());
        start..end
    } else {
        line_range_at(text, offset)
    };
    let context = &text[line.clone()];
    let relative = offset - line.start;
    gpui::word_ranges(context)
        .find(|range| range.contains(&relative))
        .map(|range| line.start + range.start..line.start + range.end)
}

pub(crate) fn line_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = clip_offset_left(text, offset);
    let start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |newline| offset + newline);
    start..end
}

fn clip_offset_left(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
