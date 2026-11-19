use std::ops::Range;

/// UTF-8 line-break opportunities with the bundled complex-script dictionaries.
/// Keep the existing GPUI line-rule options; enabling dictionaries avoids
/// treating an entire Thai, Lao, Khmer or Myanmar run as an unbroken word.
/// The borrowed segmenter uses compiled data, not a new process-level cache.
pub fn line_break_indices(text: &str) -> impl Iterator<Item = usize> + '_ {
    icu_segmenter::LineSegmenter::new_dictionary(Default::default()).segment_str(text)
}

/// UTF-8 byte ranges using Unicode word rules and the bundled ICU dictionaries.
/// The borrowed segmenter reads compiled data; it does not download a model,
/// transcode the input or retain a boundary table between editing operations.
pub fn word_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    icu_segmenter::WordSegmenter::new_dictionary(Default::default())
        .segment_str(text)
        .scan(0, |start, end| {
            let range = *start..end;
            *start = end;
            Some(range)
        })
        .filter(|range| !range.is_empty())
}
