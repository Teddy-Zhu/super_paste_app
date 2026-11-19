//! Bounded scalar widths for source ranges in the wrapper's current document.

use gpui::{Pixels, TextDirection};
use std::{collections::{HashMap, VecDeque}, ops::Range};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    start: usize,
    end: usize,
    direction: TextDirection,
}

/// Only width-only reflows may reuse this cache. Every source edit and font,
/// direction, tab or inline-metric change clears it before any range lookup.
/// Range keys avoid retaining and hashing duplicate text fragments; distinct
/// source occurrences also remain distinct when their shaping context differs.
#[derive(Default)]
pub(super) struct WidthMeasurements {
    widths: HashMap<Key, Pixels>,
    order: VecDeque<Key>,
}

impl WidthMeasurements {
    pub(super) fn measure(
        &mut self, source: Range<usize>, direction: TextDirection, measure: impl FnOnce() -> Pixels,
    ) -> Pixels {
        const MAX_ENTRIES: usize = 32_768;
        assert_ne!(direction, TextDirection::Auto, "width measurements require a resolved paragraph direction");
        let key = Key { start: source.start, end: source.end, direction };
        if let Some(width) = self.widths.get(&key) { return *width; }
        let width = measure();
        if self.order.len() == MAX_ENTRIES {
            let expired = self.order.pop_front().expect("a full width cache has an eviction key");
            self.widths.remove(&expired);
        }
        self.order.push_back(key);
        self.widths.insert(key, width);
        width
    }

    pub(super) fn clear(&mut self) {
        self.widths.clear();
        self.order.clear();
    }
}
