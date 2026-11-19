use std::ops::Range;

pub(crate) fn word_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
    crate::text_boundary::word_range_at(text, clip_offset(text, offset))
}

fn clip_offset(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    if offset == text.len() {
        return text.char_indices().next_back().map_or(0, |(ix, _)| ix);
    }

    while !text.is_char_boundary(offset) { offset -= 1; }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_range_at() {
        let text =
            "test text\nabcde 中文🎉 test\nhello[()]\ntest_connector ____\nRope\nrök\ngrande île";
        let tests = [
            (0, Some("test")),
            (4, Some(" ")),
            (10, Some("abcde")),
            (15, Some(" ")),
            (16, Some("中文")),
            (19, Some("中文")),
            (22, Some("🎉")),
            (27, Some("test")),
            (37, Some("[")),
            (38, Some("(")),
            (39, Some(")")),
            (40, Some("]")),
            (42, Some("test_connector")),
            (56, Some(" ")),
            (57, Some("____")),
            (62, Some("Rope")),
            (67, Some("rök")),
            (79, Some("île")),
        ];

        for (offset, expected) in tests {
            let actual = word_range_at(text, offset).map(|range| text[range].to_string());
            assert_eq!(actual.as_deref(), expected, "offset {offset}");
        }
    }
}
