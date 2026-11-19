//! Source-preserving edge spacing and justification of an already shaped row.
//! Glyph clusters and Unicode graphemes both constrain expansion boundaries;
//! shaping cache entries are never modified or copied into a second cache.

use std::{collections::BTreeMap, ops::Range};

use gpui::{ClusterSpacing, Pixels, ShapedLine, TextSpacePolicy};
use unicode_linebreak::{BreakClass, break_property};
use unicode_script::{Script, UnicodeScript as _};
use unicode_segmentation::UnicodeSegmentation as _;

use super::{LineFragmentKind, LineFragmentLayout};
use super::super::inline::InlineHighlight;

#[derive(Clone, Copy)]
enum Kind { Space, Ideograph, Content, Barrier }

impl Kind {
    fn of(text: &str) -> Self {
        if text.contains(['\t', '\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}']) {
            return Self::Barrier;
        }
        if text.chars().all(char::is_whitespace) { return Self::Space; }
        let first = text.chars().next().expect("nonempty native cluster");
        match break_property(first as u32) {
            BreakClass::Ideographic | BreakClass::ConditionalJapaneseStarter => Self::Ideograph,
            // NonStarter also contains Thai/Khmer characters. Their text uses
            // word expansion; do not mistake every line-break class for CJK.
            BreakClass::NonStarter if matches!(first.script(), Script::Han | Script::Hiragana | Script::Katakana | Script::Bopomofo | Script::Common) => Self::Ideograph,
            BreakClass::CombiningMark | BreakClass::WordJoiner | BreakClass::ZeroWidthJoiner => Self::Barrier,
            _ => Self::Content,
        }
    }

    fn space(self) -> bool { matches!(self, Self::Space) }
    fn ideograph(self) -> bool { matches!(self, Self::Ideograph) }
    fn barrier(self) -> bool { matches!(self, Self::Barrier) }
}

#[derive(Clone, Copy)]
struct Target { fragment: usize, index: Option<usize> }

#[derive(Clone, Copy)]
enum Edge { Before, After }

struct Cluster {
    target: Target,
    kind: Kind,
    advance: Pixels,
    expandable: bool,
    leading_collapsible: bool,
    trailing: Trailing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Trailing { None, Collapse, Hang, ConditionalHang }

/// A row's fit/alignment width can exclude preserved hanging spaces while
/// its native glyph, selection and decoration advances retain those spaces.
pub(super) struct RowSpacing { pub width: Pixels, pub hanging: Pixels }

fn space_edges(text: &str, range: Range<usize>, highlights: &[(Range<usize>, InlineHighlight)]) -> (bool, Trailing) {
    let policy = highlights.first().filter(|(mark, _)| mark.start <= range.start)
        .and_then(|(_, mark)| mark.space_policy);
    let mut covered = range.start;
    let mut no_wrap = false;
    for (mark_range, mark) in highlights.iter().take_while(|(mark, _)| mark.start < range.end) {
        // One native cluster cannot carry two different spacing adjustments.
        // Do not remove any part of a cluster crossing a policy boundary.
        if mark.space_policy != policy || (mark_range.start > covered && policy.is_some()) {
            return (false, Trailing::None);
        }
        no_wrap |= mark.no_wrap;
        covered = covered.max(mark_range.end);
    }
    if covered < range.end && policy.is_some() { return (false, Trailing::None); }
    let Some(policy) = policy else {
        return (false, if !no_wrap && text.bytes().all(|byte| byte == b' ') { Trailing::Collapse } else { Trailing::None });
    };
    let leading = policy == TextSpacePolicy::Collapse && text.chars().all(|c| matches!(c, ' ' | '\t' | '\r'));
    let trailing = if text.chars().all(|c| policy.collapses_at_line_end(c)) {
        Trailing::Collapse
    } else if text.chars().all(|c| policy.hangs_at_line_end(c)) {
        if policy == TextSpacePolicy::PreWrap { Trailing::ConditionalHang } else { Trailing::Hang }
    } else { Trailing::None };
    (leading, trailing)
}

pub(super) struct Opportunities { clusters: smallvec::SmallVec<[Cluster; 4]>, expansion: bool }

impl Opportunities {
    pub(super) fn new(expansion: bool) -> Self { Self { clusters: smallvec::SmallVec::new(), expansion } }

    pub(super) fn text(&mut self, fragment: usize, line: &ShapedLine, highlights: &[(Range<usize>, InlineHighlight)]) {
        let mut boundaries = line.text.grapheme_indices(true).map(|(index, _)| index)
            .chain(std::iter::once(line.text.len())).peekable();
        // Native clusters below advance in logical source order.
        let mut boundary_at = |at| {
            while boundaries.next_if(|index| *index < at).is_some() {}
            boundaries.peek() == Some(&at)
        };
        // A native run is in visual order. Classify opportunities in logical
        // source order, grouping glyphs that share the shaper's source cluster.
        // Ordinary aligned rows only need their edge clusters. Retain one
        // source barrier for the interior instead of allocating and sorting
        // every glyph merely to trim the row's spaces. Justification still
        // collects the complete set of expansion opportunities.
        let leading_end = line.text.len() - line.text.trim_start_matches(char::is_whitespace).len();
        let trailing_start = line.text.trim_end_matches(char::is_whitespace).len();
        if !self.expansion && leading_end == 0 && trailing_start == line.text.len() {
            if !line.text.is_empty() {
                self.clusters.push(Cluster {
                    target: Target { fragment, index: None }, kind: Kind::Content, advance: Pixels::ZERO,
                    expandable: false, leading_collapsible: false, trailing: Trailing::None,
                });
            }
            return;
        }
        let mut interior = None;
        let mut glyphs = line.runs.iter().flat_map(|run| &run.glyphs)
            .filter(|glyph| glyph.index < line.text.len())
            .filter_map(|glyph| {
                if !self.expansion && glyph.index >= leading_end && glyph.index < trailing_start {
                    interior = Some(interior.map_or(glyph.index, |previous: usize| previous.min(glyph.index)));
                    None
                } else { Some((glyph.index, glyph.advance)) }
            }).collect::<smallvec::SmallVec<[(usize, Pixels); 4]>>();
        if let Some(index) = interior { glyphs.push((index, Pixels::ZERO)); }
        glyphs.sort_unstable_by_key(|(index, _)| *index);
        let mut glyphs = glyphs.into_iter().peekable();
        let mut found = false;
        let mut highlight_index = 0;
        while let Some((index, mut advance)) = glyphs.next() {
            while glyphs.peek().is_some_and(|(next, _)| *next == index) {
                advance += glyphs.next().expect("peeked glyph").1;
            }
            let end = glyphs.peek().map_or(line.text.len(), |(index, _)| *index);
            if interior == Some(index) {
                self.clusters.push(Cluster {
                    target: Target { fragment, index: None }, kind: Kind::Content, advance: Pixels::ZERO,
                    expandable: false, leading_collapsible: false, trailing: Trailing::None,
                });
                found = true;
                continue;
            }
            let text = &line.text[index..end];
            let complete = boundary_at(index) && boundary_at(end);
            while highlights.get(highlight_index).is_some_and(|(range, _)| range.end <= index) {
                highlight_index += 1;
            }
            let (leading_collapsible, trailing) = if complete && text.chars().all(char::is_whitespace) {
                space_edges(text, index..end, &highlights[highlight_index..])
            } else { (false, Trailing::None) };
            self.clusters.push(Cluster {
                target: Target { fragment, index: Some(index) },
                kind: Kind::of(text),
                advance,
                expandable: complete && advance > Pixels::ZERO,
                leading_collapsible,
                trailing,
            });
            found = true;
        }
        if !found && !line.text.is_empty() {
            self.clusters.push(Cluster {
                target: Target { fragment, index: None }, kind: Kind::Barrier,
                advance: Pixels::ZERO, expandable: false, leading_collapsible: false, trailing: Trailing::None,
            });
        }
    }

    pub(super) fn object(&mut self, fragment: usize) {
        self.clusters.push(Cluster {
            target: Target { fragment, index: None }, kind: Kind::Content,
            advance: Pixels::ZERO, expandable: true, leading_collapsible: false, trailing: Trailing::None,
        });
    }

    fn opportunity(&self, index: usize, first: usize, last: usize) -> Option<(Target, Edge)> {
        let cluster = &self.clusters[index];
        if index >= last || !cluster.expandable { return None; }
        if cluster.kind.space() {
            return (index > first).then_some((cluster.target, Edge::After));
        }
        let next = &self.clusters[index + 1];
        if next.kind.space() || cluster.kind.barrier() || next.kind.barrier() { return None; }
        // A CJK boundary belongs to the ideograph, including at an atom or
        // another inline box. This keeps its background and selection cell
        // together instead of leaving an unowned gap after the previous box.
        if cluster.kind.ideograph() {
            Some((cluster.target, Edge::After))
        } else if next.kind.ideograph() && next.expandable {
            Some((next.target, Edge::Before))
        } else {
            None
        }
    }

    pub(super) fn apply(self, fragments: &mut [LineFragmentLayout], mut width: Pixels, available: Pixels, soft_wrap: bool, expand: bool) -> RowSpacing {
        let mut plans = BTreeMap::<usize, Vec<ClusterSpacing>>::new();
        let mut adjust = |target: Target, edge: Edge, delta: Pixels| {
            if delta == Pixels::ZERO { return; }
            let index = target.index.expect("spacing belongs to a native text cluster");
            plans.entry(target.fragment).or_default().push(ClusterSpacing {
                index,
                before: if matches!(edge, Edge::Before) { delta } else { Pixels::ZERO },
                after: if matches!(edge, Edge::After) { delta } else { Pixels::ZERO },
            });
        };
        let leading = self.clusters.iter().take_while(|cluster| cluster.leading_collapsible).count();
        let trailing = self.clusters[leading..].iter().rev().take_while(|cluster| cluster.trailing == Trailing::Collapse).count();
        for cluster in self.clusters[..leading].iter().chain(self.clusters[self.clusters.len() - trailing..].iter()) {
            let reduction = cluster.advance.max(Pixels::ZERO);
            adjust(cluster.target, Edge::After, -reduction);
            width -= reduction;
        }
        let content = &self.clusters[leading..self.clusters.len() - trailing];
        let mut unconditional = Pixels::ZERO;
        let mut conditional = Pixels::ZERO;
        let mut hanging_count = 0;
        for cluster in content.iter().rev() {
            match cluster.trailing {
                // Collapse only the suffix removed above. A normal space
                // before preserved trailing spaces remains in the sequence
                // and hangs with it (CSS Text phase II, step 4).
                Trailing::Collapse | Trailing::Hang => unconditional += cluster.advance.max(Pixels::ZERO),
                Trailing::ConditionalHang if soft_wrap => unconditional += cluster.advance.max(Pixels::ZERO),
                Trailing::ConditionalHang => conditional += cluster.advance.max(Pixels::ZERO),
                _ => break,
            }
            hanging_count += 1;
        }
        // CSS conditional hanging is resolved before justification and only
        // excludes the overflowing portion on a forced (including final) row.
        let hanging = unconditional + conditional.min((width - unconditional - available).max(Pixels::ZERO));
        let first = content.iter().position(|cluster| !cluster.kind.space()).map(|index| leading + index);
        let last = content[..content.len() - hanging_count].iter().rposition(|cluster| !cluster.kind.space())
            .map(|index| leading + index);
        let count = first.zip(last).filter(|_| expand).map_or(0, |(first, last)|
            (first..last).filter_map(|index| self.opportunity(index, first, last)).count());
        let remaining = (available - (width - hanging)).max(Pixels::ZERO);
        if count != 0 && remaining > Pixels::ZERO {
            let (first, last) = first.zip(last).expect("expansion has content");
            let each = remaining / count as f32;
            let mut rest = remaining;
            let mut emitted = 0;
            for index in first..last {
                let Some((target, edge)) = self.opportunity(index, first, last) else { continue; };
                emitted += 1;
                let amount = if emitted == count { rest } else { each.min(rest) };
                rest -= amount;
                adjust(target, edge, amount);
            }
            width += remaining;
        }
        for (index, mut plan) in plans {
            plan.sort_unstable_by_key(|gap| gap.index);
            // One ideograph may own its preceding and following opportunity.
            // The native layout accepts one entry per source cluster.
            plan.dedup_by(|following, first| {
                if following.index != first.index { return false; }
                first.before += following.before;
                first.after += following.after;
                true
            });
            let delta = plan.iter().fold(Pixels::ZERO, |sum, gap| sum + gap.before + gap.after);
            let fragment = &mut fragments[index];
            fragment.size.width += delta;
            let LineFragmentKind::Text { spacing, code_background, .. } = &mut fragment.kind else { unreachable!("glyph advances belong to text"); };
            if let Some(background) = code_background { background.bounds.size.width += delta; }
            *spacing = Some(plan.into());
        }
        RowSpacing { width: width - hanging, hanging }
    }
}
