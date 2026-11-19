//! Directional scopes share the inline boxes' existing DOM ancestry. Empty
//! boxes are merged by absolute source position, since cached plain text can
//! precede all of its zero-byte anchors in the measured item list.

use std::{iter::Peekable, ops::Range};
use gpui::TextIsolation;
use super::super::MeasureItem;
use super::super::super::inline_box::InlineBox;

#[derive(Default)]
struct Collector<'a> {
    scopes: Vec<TextIsolation>,
    active: Vec<(&'a InlineBox, usize)>,
    chain: Vec<&'a InlineBox>,
}

impl<'a> Collector<'a> {
    fn visit(&mut self, range: Range<usize>, source: Option<&'a InlineBox>) {
        self.chain.clear();
        if let Some(source) = source { source.isolation_chain(&mut self.chain); }
        let shared = self.active.iter().zip(&self.chain)
            .take_while(|((active, _), next)| std::ptr::eq(*active, **next)).count();
        self.active.truncate(shared);
        for &source in &self.chain[shared..] {
            let index = self.scopes.len();
            self.scopes.push(TextIsolation {
                range: range.start..range.start,
                direction: source.isolation_direction().expect("directional ancestor"),
                parent: self.active.last().map(|(_, index)| *index),
            });
            self.active.push((source, index));
        }
        for (_, index) in &self.active { self.scopes[*index].range.end = range.end; }
    }

    fn span<I>(&mut self, range: Range<usize>, source: Option<&'a InlineBox>, anchors: &mut Peekable<I>) -> Option<()>
    where I: Iterator<Item = (usize, &'a InlineBox)> {
        let mut start = range.start;
        while let Some((at, anchor)) = anchors.peek().copied().filter(|(at, _)| *at < range.end) {
            if at < start { return None; }
            if start < at { self.visit(start..at, source); }
            self.visit(at..at, Some(anchor));
            start = at;
            anchors.next();
        }
        if start < range.end { self.visit(start..range.end, source); }
        Some(())
    }
}

pub(super) fn collect(items: &[MeasureItem]) -> Option<Vec<TextIsolation>> {
    let mut collector = Collector::default();
    let mut anchors = items.iter().filter_map(|item| match item {
        MeasureItem::Box { inline_box, at } => Some((*at, inline_box.as_ref())),
        _ => None,
    }).peekable();
    let mut offset = 0;
    for item in items {
        match item {
            MeasureItem::Box { .. } => continue,
            MeasureItem::Text { text, highlights, .. } => {
                let mut covered = 0;
                for (range, mark) in highlights {
                    if range.is_empty() { continue; }
                    if covered < range.start { collector.span(offset + covered..offset + range.start, None, &mut anchors)?; }
                    collector.span(offset + range.start..offset + range.end, mark.inline_box.as_deref(), &mut anchors)?;
                    covered = range.end;
                }
                if covered < text.len() { collector.span(offset + covered..offset + text.len(), None, &mut anchors)?; }
            },
            MeasureItem::Object { inline_box, .. } | MeasureItem::Image { inline_box, .. } => {
                collector.span(offset..offset + item.len(), inline_box.as_deref(), &mut anchors)?;
            },
        }
        offset += item.len();
    }
    for (at, source) in anchors {
        if at != offset { return None; }
        collector.visit(at..at, Some(source));
    }
    Some(collector.scopes)
}

/// Slice by source bounds without rescanning every scope for every hard line.
/// A binary search finds new openers; parent links recover enclosing scopes.
/// Paragraph separators restart CSS isolates without entering their ranges.
pub(super) fn clipped(scopes: &[TextIsolation], text: &str, range: &Range<usize>) -> Option<Vec<TextIsolation>> {
    let mut result = Vec::new();
    if scopes.is_empty() { return Some(result); }
    let mut ancestors = Vec::new();
    let mut active = Vec::new();
    let mut append = |segment: Range<usize>| -> Option<()> {
        let first = scopes.partition_point(|scope| scope.range.start < segment.start);
        ancestors.clear();
        // An opener at the boundary supplies the exact ancestor chain. A
        // preceding sibling ending here is already closed; retaining it would
        // inject a spurious empty isolate into the next paragraph. Ancestors
        // ending here are needed only for an actual boundary opener (including
        // an empty child), not merely because their ranges touch this row.
        let boundary_opener = scopes.get(first)
            .filter(|scope| scope.range.start == segment.start);
        let mut previous = match boundary_opener {
            Some(scope) => scope.parent,
            None => first.checked_sub(1),
        };
        while let Some(index) = previous {
            let scope = &scopes[index];
            if scope.range.end > segment.start
                || (boundary_opener.is_some() && scope.range.end == segment.start)
            { ancestors.push(index); }
            previous = scope.parent;
        }
        active.clear();
        let new_scopes = (first..scopes.len()).take_while(|index| scopes[*index].range.start <= segment.end);
        for index in ancestors.iter().rev().copied().chain(new_scopes) {
            let scope = &scopes[index];
            if scope.range.end < segment.start { continue; }
            while active.last().map(|(source, _)| *source) != scope.parent {
                active.pop()?;
            }
            let parent = active.last().map(|(_, mapped)| *mapped);
            active.push((index, result.len()));
            result.push(TextIsolation {
                range: scope.range.start.max(segment.start) - range.start
                    ..scope.range.end.min(segment.end) - range.start,
                direction: scope.direction, parent,
            });
        }
        Some(())
    };
    let mut start = range.start;
    for (offset, ch) in text[range.clone()].char_indices() {
        if !gpui::TextDirection::is_paragraph_separator(ch) { continue; }
        let at = range.start + offset;
        append(start..at)?;
        start = at + ch.len_utf8();
    }
    append(start..range.end)?;
    Some(result)
}
