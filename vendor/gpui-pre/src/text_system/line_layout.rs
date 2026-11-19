use crate::{ClusterSpacing, OverflowWrap, FontId, GlyphId, Pixels, PlatformTextSystem, Point, SharedString, Size, TextAlign, TextDirection, point, px};
use collections::{FxHashMap, FxHashSet};
use itertools::Itertools;
use smallvec::SmallVec;
use std::{
    borrow::Borrow,
    cell::RefCell,
    hash::{Hash, Hasher},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::LineWrapper;
use unicode_segmentation::GraphemeCursor;

mod bidi;
pub(crate) use bidi::CursorBidi;
pub use bidi::VisualSourceRuns;

/// A laid out and styled line of text
#[derive(Clone, Default, Debug)]
pub struct LineLayout {
    /// The font size for this line
    pub font_size: Pixels,
    /// The width of the line
    pub width: Pixels,
    /// The ascent of the line
    pub ascent: Pixels,
    /// The descent of the line
    pub descent: Pixels,
    /// The shaped runs that make up this line
    pub runs: Vec<ShapedRun>,
    /// The length of the line in utf-8 bytes
    pub len: usize,
    /// Grapheme insertions strictly inside shaping clusters, sorted by source index.
    /// Ordinary one-grapheme clusters require no entry or allocation.
    pub interior_carets: Vec<ShapedCaret>,
    /// Presentation isolation shared with native shaping and soft-row bidi
    /// resolution. Its source window follows original UTF-8 layout slices.
    pub isolation: Option<super::TextIsolationContext>,
    /// Physical left/right ink cuts for a retained piece of one native cluster.
    /// The outer ink edge stays open; ordinary layouts need neither cut.
    pub ink_clip: Option<(bool, bool)>,
}

/// An insertion inside a ligature or other multi-grapheme shaping cluster.
#[derive(Clone, Copy, Debug)]
pub struct ShapedCaret {
    /// UTF-8 source offset of the insertion.
    pub index: usize,
    /// Source start of the owning shaping cluster.
    pub cluster_start: usize,
    /// Physical advance coordinate in this line.
    pub x: Pixels,
}

/// A run of text that has been shaped .
#[derive(Debug, Clone)]
pub struct ShapedRun {
    /// The font id for this run
    pub font_id: FontId,
    /// An authored run size; `None` inherits the containing line's size.
    /// Retained slices must preserve it alongside the glyphs.
    pub font_size: Option<Pixels>,
    /// The glyphs that make up this run
    pub glyphs: Vec<ShapedGlyph>,
}

/// A single glyph, ready to paint.
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
    /// The ID for this glyph, as determined by the text system.
    pub id: GlyphId,

    /// The position of this glyph in its containing line.
    pub position: Point<Pixels>,

    /// The index of this glyph in the original text.
    pub index: usize,

    /// Whether this glyph is an emoji
    pub is_emoji: bool,

    /// The advance supplied by the shaper, independent of ink offsets or
    /// visual ordering. Marks may have a zero advance. An explicit fixed-cell
    /// layout can replace this with its cell advance.
    pub advance: Pixels,

    /// Whether the shaper resolved this glyph's cluster right-to-left.
    pub is_rtl: bool,
}

/// Which logical side of a source boundary supplies its visual caret.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CaretAffinity {
    /// The trailing edge of the preceding source cluster.
    Upstream,
    /// The leading edge of the following source cluster.
    #[default]
    Downstream,
}

/// A native hit retains which visual side of its source offset was reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextHit {
    /// UTF-8 source offset.
    pub index: usize,
    /// The visual caret to use at a bidi or soft-wrap boundary.
    pub affinity: CaretAffinity,
}

/// One native shaping cluster in physical paint order. Several glyphs can
/// share a cluster; its cell excludes their ink offsets and overhangs.
#[derive(Clone, Debug)]
pub struct ShapedCluster {
    /// The UTF-8 source interval drawn by this cluster.
    pub source: Range<usize>,
    /// The physical advance cell in the unwrapped line.
    pub cell: Range<Pixels>,
    /// Its first glyph, used to locate the existing soft row without assuming
    /// that source offsets increase in paint order.
    pub first_glyph: WrapBoundary,
    /// The resolved direction of the cluster.
    pub is_rtl: bool,
}

#[derive(Clone, Copy)]
struct VisualCluster {
    index: usize,
    first_glyph: WrapBoundary,
    left: Pixels,
    right: Pixels,
    is_rtl: bool,
}

#[derive(Clone, Copy)]
struct VisualCaret {
    x: Pixels,
    glyph: WrapBoundary,
}

impl VisualCluster {
    fn caret(self, leading: bool) -> VisualCaret {
        VisualCaret {
            x: if leading != self.is_rtl { self.left } else { self.right },
            glyph: self.first_glyph,
        }
    }
}

impl LineLayout {
    /// Native cluster advances in UTF-8 source order. Ordered glyphs stream
    /// without an index allocation; reordered glyphs use a temporary index.
    /// Keep original f32 advances in f64 while summing fallback glyphs rather
    /// than inferring widths from ink positions or reshaping the source.
    pub fn source_advances(&self) -> impl Iterator<Item = (usize, f64)> + '_ {
        let glyphs = self.runs.iter().flat_map(|run| &run.glyphs);
        let ordered = glyphs.clone().map(|glyph| glyph.index).is_sorted();
        let mut reordered = if ordered { Vec::new() } else {
            glyphs.clone().map(|glyph| (glyph.index, f64::from(glyph.advance.as_f32()))).collect::<Vec<_>>()
        };
        if !ordered { reordered.sort_unstable_by_key(|entry| entry.0); }
        let mut reordered = reordered.into_iter();
        let mut direct = glyphs.map(|glyph| (glyph.index, f64::from(glyph.advance.as_f32())));
        let mut source = std::iter::from_fn(move || if ordered { direct.next() } else { reordered.next() }).peekable();
        std::iter::from_fn(move || {
            let (index, mut advance) = source.next()?;
            while source.peek().is_some_and(|entry| entry.0 == index) {
                advance += source.next().expect("a peeked native glyph").1;
            }
            Some((index, advance))
        })
    }

    /// Discover only intra-cluster grapheme insertions. Backends replace the
    /// proportional positions with native layout/font caret positions when
    /// available. The complete source supplies Unicode context across fallback
    /// font clusters; no per-character boundary table is retained.
    pub fn interior_caret_candidates(&self, text: &str) -> Vec<ShapedCaret> {
        assert_eq!(self.len, text.len(), "caret source must match its layout");
        let mut result = Vec::new();
        let mut boundaries = SmallVec::<[usize; 4]>::new();
        let mut cursor = GraphemeCursor::new(0, text.len(), true);
        for cluster in self.clusters() {
            boundaries.clear();
            if cursor.cur_cursor() != cluster.source.start {
                cursor = GraphemeCursor::new(cluster.source.start, text.len(), true);
            }
            // Keep preceding Unicode context, but do not scan past this
            // cluster into a long combining sequence owned by later glyphs.
            // Consecutive source clusters resume the cursor, including its RI
            // parity, instead of recounting every preceding regional indicator.
            loop {
                match cursor.next_boundary(&text[..cluster.source.end], 0) {
                    Ok(Some(index)) if index >= cluster.source.end => break,
                    Ok(Some(index)) => {
                        if index > cluster.source.start { boundaries.push(index); }
                    }
                    Ok(None) | Err(unicode_segmentation::GraphemeIncomplete::NextChunk) => break,
                    Err(error) => panic!("source prefix must supply Unicode context: {error:?}"),
                }
            }
            let steps = (boundaries.len() + 1) as f32;
            for (ordinal, index) in boundaries.iter().copied().enumerate() {
                let fraction = (ordinal + 1) as f32 / steps;
                let advance = (cluster.cell.end - cluster.cell.start) * fraction;
                result.push(ShapedCaret {
                    index,
                    cluster_start: cluster.source.start,
                    x: if cluster.is_rtl { cluster.cell.end - advance } else { cluster.cell.start + advance },
                });
            }
        }
        result.sort_unstable_by(|a, b| a.index.cmp(&b.index)
            .then_with(|| a.x.as_f32().total_cmp(&b.x.as_f32())));
        result.dedup_by_key(|caret| caret.index);
        result
    }

    /// Copy one retained cluster's insertions alongside its existing glyphs.
    /// Source order lets slicing avoid rescanning every caret for every glyph.
    pub(crate) fn append_cluster_carets(
        &self,
        cluster: usize,
        source: Range<usize>,
        shift: Pixels,
        output: &mut Vec<ShapedCaret>,
    ) {
        let start = self.interior_carets.partition_point(|caret| caret.cluster_start < cluster);
        for caret in self.interior_carets[start..].iter()
            .take_while(|caret| caret.cluster_start == cluster)
            .filter(|caret| source.contains(&caret.index))
        {
            output.push(ShapedCaret {
                index: caret.index - source.start,
                cluster_start: caret.cluster_start - source.start,
                x: caret.x + shift,
            });
        }
    }

    /// Selection cells within one native cluster, split only at retained
    /// grapheme insertions. Source order can run opposite to physical order.
    pub fn cluster_cells<'a>(
        &'a self,
        cluster: &'a ShapedCluster,
    ) -> impl Iterator<Item = (Range<usize>, Range<Pixels>)> + 'a {
        let first = self.interior_carets.partition_point(|caret| caret.cluster_start < cluster.source.start);
        let mut points = self.interior_carets[first..].iter()
            .take_while(move |caret| caret.cluster_start == cluster.source.start)
            .map(|caret| (caret.index, caret.x))
            .chain(std::iter::once((
                cluster.source.end,
                if cluster.is_rtl { cluster.cell.start } else { cluster.cell.end },
            )));
        let mut start = cluster.source.start;
        let mut x = if cluster.is_rtl { cluster.cell.end } else { cluster.cell.start };
        std::iter::from_fn(move || {
            let (end, next_x) = points.next()?;
            let cell = (start..end, x.min(next_x)..x.max(next_x));
            start = end;
            x = next_x;
            Some(cell)
        })
    }

    /// Adds advance to source clusters without mutating the native shaping
    /// cache. Ink, caret cells, decoration extents and hit testing then share
    /// the same expanded geometry. Entries are sorted, unique source indices.
    pub(crate) fn add_spacing(&mut self, spacing: &[ClusterSpacing]) {
        let mut shift = 0_f64;
        let mut previous_index = None;
        let mut shrink = Pixels::ZERO;
        let mut glyphs = self.runs.iter_mut().flat_map(|run| &mut run.glyphs).peekable();
        while let Some(glyph) = glyphs.next() {
            let (before, after) = spacing.binary_search_by_key(&glyph.index, |gap| gap.index)
                .map_or((Pixels::ZERO, Pixels::ZERO), |index| {
                    let gap = spacing[index];
                    (gap.before, gap.after)
                });
            let ink_shift = if glyph.is_rtl { after } else { before };
            if previous_index != Some(glyph.index) {
                shrink = (-after).max(Pixels::ZERO);
                previous_index = Some(glyph.index);
                let start = self.interior_carets.partition_point(|caret| caret.cluster_start < glyph.index);
                for caret in self.interior_carets[start..].iter_mut()
                    .take_while(|caret| caret.cluster_start == glyph.index)
                {
                    caret.x += px(shift as f32) + ink_shift;
                }
            }
            // Logical leading/trailing padding swaps physical sides for RTL.
            // The cell keeps both sides while ink and interior ligature carets
            // move together; no reshaping or synthetic source spaces are used.
            glyph.position.x += px(shift as f32) + ink_shift;
            let reduction = glyph.advance.max(Pixels::ZERO).min(shrink);
            glyph.advance -= reduction;
            shrink -= reduction;
            if glyphs.peek().is_none_or(|next| next.index != glyph.index) {
                glyph.advance += before + after.max(Pixels::ZERO);
                shift += f64::from((before + after + shrink).as_f32());
            }
        }
        self.width += px(shift as f32);
    }

    /// Glyphs stay in visual order; cluster indices remain source offsets.
    /// Caret cells use advances, not the ink offsets used when painting marks.
    fn visual_clusters(&self) -> impl Iterator<Item = VisualCluster> + '_ {
        let mut glyphs = self.runs.iter().enumerate().flat_map(|(run_ix, run)| {
            run.glyphs.iter().enumerate().map(move |(glyph_ix, glyph)| {
                (WrapBoundary { run_ix, glyph_ix }, glyph)
            })
        }).peekable();
        let mut pen = Pixels::ZERO;
        std::iter::from_fn(move || {
            let (first_glyph, glyph) = glyphs.next()?;
            let left = pen;
            pen += glyph.advance;
            while glyphs.peek().is_some_and(|(_, next)| next.index == glyph.index) {
                pen += glyphs.next().unwrap().1.advance;
            }
            Some(VisualCluster {
                index: glyph.index,
                first_glyph,
                left,
                right: if glyphs.peek().is_none() { self.width } else { pen },
                is_rtl: glyph.is_rtl,
            })
        })
    }

    /// Native cluster starts and advance cells in paint order. Callers which
    /// do not need source range ends avoid the sorted index needed by `clusters`
    /// for bidirectional text.
    pub fn visual_cluster_advances(&self) -> impl Iterator<Item = (usize, Range<Pixels>)> + '_ {
        self.visual_clusters().map(|cluster| (cluster.index, cluster.left..cluster.right))
    }

    /// Source-aware cells in paint order. Only nonmonotonic (bidi) layouts
    /// need a temporary source index; ordinary text needs no index allocation.
    pub fn clusters(&self) -> impl Iterator<Item = ShapedCluster> + '_ {
        let ordered = self.runs.iter().flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.index).tuple_windows().all(|(a, b)| a <= b);
        let mut source_starts = SmallVec::<[usize; 16]>::new();
        if !ordered {
            source_starts.extend(self.runs.iter().flat_map(|run| &run.glyphs).map(|glyph| glyph.index));
            source_starts.sort_unstable();
            source_starts.dedup();
        }
        let mut clusters = self.visual_clusters().peekable();
        std::iter::from_fn(move || {
            let cluster = clusters.next()?;
            let end = if ordered {
                clusters.peek().map_or(self.len, |next| next.index)
            } else {
                source_starts.get(source_starts.partition_point(|index| *index <= cluster.index))
                    .copied().unwrap_or(self.len)
            };
            Some(ShapedCluster {
                source: cluster.index..end,
                cell: cluster.left..cluster.right,
                first_glyph: cluster.first_glyph,
                is_rtl: cluster.is_rtl,
            })
        })
    }

    /// Physical row cells for the existing wrap boundaries, using advances
    /// rather than glyph ink positions. The iterator walks the glyphs once
    /// and allocates nothing; boundaries must be valid and in paint order.
    pub fn visual_row_ranges<'a>(
        &'a self,
        boundaries: &'a [WrapBoundary],
    ) -> impl Iterator<Item = Range<Pixels>> + 'a {
        let mut glyphs = self.runs.iter().enumerate().flat_map(|(run_ix, run)| {
            run.glyphs.iter().enumerate().map(move |(glyph_ix, glyph)| {
                (WrapBoundary { run_ix, glyph_ix }, glyph)
            })
        }).peekable();
        let mut pen = Pixels::ZERO;
        let ends = boundaries.iter().map(move |boundary| {
            loop {
                let (location, glyph) = glyphs.peek().expect("wrap boundary must identify a shaped glyph");
                if location == boundary { return pen; }
                pen += glyph.advance;
                glyphs.next();
            }
        }).chain([self.width]);
        let mut start = Pixels::ZERO;
        ends.map(move |end| {
            let range = start..end;
            start = end;
            range
        })
    }

    fn carets_for_index(&self, index: usize) -> (Option<VisualCaret>, Option<VisualCaret>) {
        let mut preceding: Option<VisualCluster> = None;
        let mut following = None;
        let mut first: Option<VisualCluster> = None;
        for cluster in self.visual_clusters() {
            if first.is_none_or(|old| cluster.index < old.index) { first = Some(cluster); }
            if cluster.index == index && following.is_none() { following = Some(cluster); }
            if cluster.index < index && preceding.is_none_or(|old| cluster.index > old.index) {
                preceding = Some(cluster);
            }
        }
        let interior = self.interior_carets.binary_search_by_key(&index, |caret| caret.index)
            .ok().map(|position| self.interior_carets[position].x);
        let before = preceding.map(|cluster| {
            let mut caret = cluster.caret(false);
            if let Some(x) = interior { caret.x = x; }
            caret
        });
        let after = following.map(|cluster: VisualCluster| cluster.caret(true));
        let fallback = first.map(|cluster| cluster.caret(true));
        (before.or(after).or(fallback), after.or(before).or(fallback))
    }

    fn next_cluster_index(&self, index: usize) -> usize {
        self.runs.iter().flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.index)
            .filter(|start| *start > index)
            .min()
            .unwrap_or(self.len)
    }

    /// Resolve a source prefix omitted by the native shaper to the same
    /// leading cluster caret used when drawing that source position. This
    /// changes neither source text nor selection anchors. A nonempty row
    /// without any native glyphs has no cluster caret to resolve.
    pub fn resolve_visual_caret(&self, hit: TextHit) -> Option<TextHit> {
        if hit.index > self.len { return None; }
        let first = self.runs.iter().flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.index).min();
        match first {
            Some(first) if hit.index < first
                || (hit.index == first && hit.affinity == CaretAffinity::Upstream) => {
                Some(TextHit { index: first, affinity: CaretAffinity::Downstream })
            }
            Some(_) => Some(hit),
            None if self.len == 0 => Some(hit),
            None => None,
        }
    }

    /// Move through native clusters in physical order. `None` enters the line
    /// from outside and crosses the first grapheme, rather than stopping twice
    /// at a shared run edge. The predicate excludes application-level stops
    /// inside a grapheme or an atomic inline object.
    pub fn adjacent_visual_caret(
        &self,
        from: Option<TextHit>,
        right: bool,
        mut valid: impl FnMut(usize) -> bool,
    ) -> Option<TextHit> {
        let from = from.map(|mut hit| {
            if hit.index == 0 { hit.affinity = CaretAffinity::Downstream; }
            else if hit.index == self.len { hit.affinity = CaretAffinity::Upstream; }
            hit
        });
        let mut previous = None;
        let mut entered = from.is_none();
        for cluster in self.clusters() {
            let owns = from.is_some_and(|hit| match hit.affinity {
                CaretAffinity::Upstream => cluster.source.start < hit.index && hit.index <= cluster.source.end,
                CaretAffinity::Downstream => cluster.source.start <= hit.index && hit.index < cluster.source.end,
            });
            if owns {
                if let Some(target) = self.cluster_navigation_caret(&cluster, from.map(|hit| hit.index), right, false, &mut valid) {
                    return Some(target);
                }
                if !right { return previous; }
                entered = true;
                continue;
            }
            if entered || !right {
                let target = self.cluster_navigation_caret(&cluster, None, right, false, &mut valid);
                if right && target.is_some() { return target; }
                if !right && target.is_some() { previous = target; }
            }
        }
        if from.is_none() && !right { previous } else { None }
    }

    /// The outer insertion at a physical line edge, retaining its owning side.
    pub fn visual_edge_caret(&self, right: bool, mut valid: impl FnMut(usize) -> bool) -> Option<TextHit> {
        let mut edge = None;
        for cluster in self.clusters() {
            let target = self.cluster_navigation_caret(&cluster, None, !right, true, &mut valid);
            if !right && target.is_some() { return target; }
            if right && target.is_some() { edge = target; }
        }
        edge.or_else(|| (self.len == 0 && valid(0)).then_some(TextHit { index: 0, affinity: CaretAffinity::Downstream }))
    }

    fn cluster_navigation_caret(
        &self,
        cluster: &ShapedCluster,
        from: Option<usize>,
        right: bool,
        include_edge: bool,
        valid: &mut impl FnMut(usize) -> bool,
    ) -> Option<TextHit> {
        let forward = right != cluster.is_rtl;
        let from = from.unwrap_or(if forward { cluster.source.start } else { cluster.source.end });
        let start = self.interior_carets.partition_point(|caret| caret.cluster_start < cluster.source.start);
        let stops = std::iter::once(cluster.source.start)
            .chain(self.interior_carets[start..].iter().take_while(|caret| caret.cluster_start == cluster.source.start).map(|caret| caret.index))
            .chain(std::iter::once(cluster.source.end));
        let mut target = None;
        for index in stops {
            if (if forward { index > from } else { index < from }) || (include_edge && index == from) {
                if valid(index) {
                    let affinity = if index == cluster.source.start { CaretAffinity::Downstream }
                        else if index == cluster.source.end || forward { CaretAffinity::Upstream }
                        else { CaretAffinity::Downstream };
                    target = Some(TextHit { index, affinity });
                    if forward { break; }
                }
            }
        }
        target
    }

    /// Sum complete shaped clusters whose source start is in `range`.
    /// Unlike subtracting caret positions, this remains valid when the source
    /// interval occupies multiple visual regions or contains offset marks.
    pub fn advance_for_range(&self, range: Range<usize>) -> Pixels {
        self.runs.iter().flat_map(|run| &run.glyphs)
            .filter(|glyph| range.contains(&glyph.index))
            .map(|glyph| glyph.advance).sum()
    }

    /// The index for the character at the given x coordinate
    pub fn index_for_x(&self, x: Pixels) -> Option<usize> {
        if x >= self.width {
            None
        } else {
            self.visual_clusters()
                .find(|cluster| x < cluster.right)
                .map(|cluster| {
                    let mut index = cluster.index;
                    let start = self.interior_carets.partition_point(|caret| caret.cluster_start < cluster.index);
                    for caret in self.interior_carets[start..].iter()
                        .take_while(|caret| caret.cluster_start == cluster.index)
                    {
                        if (caret.x <= x) != cluster.is_rtl { index = caret.index; }
                    }
                    index
                })
                .or(Some(0))
        }
    }

    /// closest_index_for_x returns the character boundary closest to the given x coordinate
    /// (e.g. to handle aligning up/down arrow keys)
    pub fn closest_index_for_x(&self, x: Pixels) -> usize {
        self.closest_caret_for_x(x).index
    }

    /// The nearest native caret, retaining the side reached by the pointer.
    pub fn closest_caret_for_x(&self, x: Pixels) -> TextHit {
        self.closest_caret_for_x_in_range(x, Pixels::ZERO..self.width)
    }

    fn closest_caret_for_x_in_range(&self, x: Pixels, range: Range<Pixels>) -> TextHit {
        let mut selected = None;
        for cluster in self.visual_clusters().filter(|cluster| cluster.right > range.start && cluster.left < range.end) {
            let reached = x <= cluster.right && cluster.right > cluster.left;
            selected = Some(cluster);
            if reached { break; }
        }
        let Some(cluster) = selected else {
            return TextHit { index: self.len, affinity: CaretAffinity::Downstream };
        };
        let left_half = x <= (cluster.left + cluster.right) / 2.;
        let mut hit = if left_half != cluster.is_rtl {
            TextHit { index: cluster.index, affinity: CaretAffinity::Downstream }
        } else {
            TextHit { index: self.next_cluster_index(cluster.index), affinity: CaretAffinity::Upstream }
        };
        let mut nearest_x = if left_half { cluster.left } else { cluster.right };
        let mut distance = (x - nearest_x).abs();
        let start = self.interior_carets.partition_point(|caret| caret.cluster_start < cluster.index);
        for caret in self.interior_carets[start..].iter()
            .take_while(|caret| caret.cluster_start == cluster.index)
        {
            let candidate_distance = (x - caret.x).abs();
            if candidate_distance < distance || (candidate_distance == distance && caret.x < nearest_x) {
                nearest_x = caret.x;
                distance = candidate_distance;
                hit = TextHit {
                    index: caret.index,
                    affinity: if (x <= caret.x) != cluster.is_rtl {
                        CaretAffinity::Upstream
                    } else {
                        CaretAffinity::Downstream
                    },
                };
            }
        }
        hit
    }

    /// The x position of the character at the given index
    pub fn x_for_index(&self, index: usize) -> Pixels {
        self.x_for_index_with_affinity(index, CaretAffinity::Downstream)
    }

    /// Resolves either visual caret at a bidirectional source boundary.
    pub fn x_for_index_with_affinity(&self, index: usize, affinity: CaretAffinity) -> Pixels {
        let (upstream, downstream) = self.carets_for_index(index);
        match affinity {
            CaretAffinity::Upstream => upstream,
            CaretAffinity::Downstream => downstream,
        }.map_or(Pixels::ZERO, |caret| caret.x)
    }

    /// The corresponding Font at the given index
    pub fn font_id_for_index(&self, index: usize) -> Option<FontId> {
        if index >= self.len { return None; }
        self.runs.iter().flat_map(|run| run.glyphs.iter().map(move |glyph| (glyph.index, run.font_id)))
            .filter(|(start, _)| *start <= index)
            // Keep the first font at a shared cluster start; a fallback mark
            // must not replace its base glyph's font in this lookup.
            .min_by_key(|(start, _)| std::cmp::Reverse(*start))
            .map(|(_, font)| font)
    }

    /// Split this layout at a byte index, returning `(prefix, suffix)`.
    ///
    /// - Each half retains the glyphs whose source starts belong to its byte range.
    /// - Visual order and ink offsets are preserved, packing only the retained
    ///   advances. A logical interval need not be contiguous on screen.
    /// - Suffix byte indices are rebased to zero.
    /// - The last logical half retains any native metric remainder, so both
    ///   widths add up to the original line width.
    /// - `font_size`, `ascent`, and `descent` are copied to both halves.
    pub fn split_at(&self, byte_index: usize) -> (LineLayout, LineLayout) {
        assert!(byte_index <= self.len, "split boundary exceeds line length");
        let mut left_runs = Vec::new();
        let mut right_runs = Vec::new();
        let mut left_carets = Vec::new();
        let mut right_carets = Vec::new();
        let mut original_pen = Pixels::ZERO;
        let mut left_pen = Pixels::ZERO;
        let mut right_pen = Pixels::ZERO;
        let mut previous_cluster = None;
        for run in &self.runs {
            let mut left_glyphs = Vec::new();
            let mut right_glyphs = Vec::new();
            for glyph in &run.glyphs {
                if previous_cluster != Some(glyph.index) {
                    if glyph.index < byte_index {
                        self.append_cluster_carets(glyph.index, 0..byte_index, left_pen - original_pen, &mut left_carets);
                    } else {
                        self.append_cluster_carets(glyph.index, byte_index..self.len, right_pen - original_pen, &mut right_carets);
                    }
                    previous_cluster = Some(glyph.index);
                }
                let mut retained = glyph.clone();
                let offset = glyph.position.x - original_pen;
                original_pen += glyph.advance;
                if glyph.index < byte_index {
                    retained.position.x = left_pen + offset;
                    left_pen += glyph.advance;
                    left_glyphs.push(retained);
                } else {
                    retained.position.x = right_pen + offset;
                    retained.index -= byte_index;
                    right_pen += glyph.advance;
                    right_glyphs.push(retained);
                }
            }
            if !left_glyphs.is_empty() { left_runs.push(ShapedRun { font_size: run.font_size, font_id: run.font_id, glyphs: left_glyphs }); }
            if !right_glyphs.is_empty() { right_runs.push(ShapedRun { font_size: run.font_size, font_id: run.font_id, glyphs: right_glyphs }); }
        }
        // Retain the native metric remainder on the last logical half, so
        // repeated slicing neither loses it nor charges it more than once.
        // Native widths can include rounding or nonpainting source content.
        let remainder = self.width - original_pen;
        if byte_index == self.len {
            left_pen += remainder;
        } else {
            right_pen += remainder;
        }
        left_carets.sort_unstable_by_key(|caret| caret.index);
        right_carets.sort_unstable_by_key(|caret| caret.index);
        let left = LineLayout {
            font_size: self.font_size,
            width: left_pen,
            ascent: self.ascent,
            descent: self.descent,
            runs: left_runs,
            len: byte_index,
            interior_carets: left_carets,
            isolation: self.isolation.as_ref().map(|context| context.subrange(0..byte_index)),
            ink_clip: (byte_index == self.len).then_some(self.ink_clip).flatten(),
        };

        let right = LineLayout {
            font_size: self.font_size,
            width: right_pen,
            ascent: self.ascent,
            descent: self.descent,
            runs: right_runs,
            len: self.len - byte_index,
            interior_carets: right_carets,
            isolation: self.isolation.as_ref().map(|context| context.subrange(byte_index..self.len)),
            ink_clip: (byte_index == 0).then_some(self.ink_clip).flatten(),
        };

        (left, right)
    }

    /// The widest indivisible source segment, using the same shaped advances
    /// as display. Source sorting handles bidi/fallback reordering; a legal
    /// break inside a shaping cluster is deliberately ignored.
    pub fn min_content_width(&self, text: &str, policy: OverflowWrap) -> Pixels {
        use unicode_segmentation::UnicodeSegmentation as _;
        let breaks: SmallVec<[usize; 32]> = match policy {
            OverflowWrap::Normal => LineWrapper::break_indices(text).collect(),
            OverflowWrap::Anywhere => text.grapheme_indices(true).map(|(index, _)| index).collect(),
        };
        let mut clusters: SmallVec<[VisualCluster; 32]> = self.visual_clusters().collect();
        if clusters.windows(2).any(|pair| pair[0].index > pair[1].index) {
            clusters.sort_unstable_by_key(|cluster| cluster.index);
        }
        let mut previous = None;
        let mut used = Pixels::ZERO;
        let mut trimmed = Pixels::ZERO;
        let mut maximum = Pixels::ZERO;
        for cluster in clusters {
            if previous != Some(cluster.index) && breaks.binary_search(&cluster.index).is_ok() {
                maximum = maximum.max(trimmed);
                used = Pixels::ZERO;
                trimmed = Pixels::ZERO;
            }
            used += cluster.right - cluster.left;
            // Collapsing edge spaces do not increase a word's minimum. NBSP
            // and other non-ASCII separators retain their shaped advances.
            if !matches!(text.as_bytes().get(cluster.index), Some(b' ' | b'\t' | b'\r' | b'\n' | 0x0c)) {
                trimmed = used;
            }
            previous = Some(cluster.index);
        }
        maximum.max(trimmed)
    }

    fn compute_wrap_boundaries(
        &self,
        text: &str,
        wrap_width: Pixels,
        max_lines: Option<usize>,
        scratch: &mut WrapScratch,
        overflow_wrap: Option<OverflowWrap>,
    ) -> SmallVec<[WrapBoundary; 1]> {
        let mut boundaries = SmallVec::new();
        let mut current_segment: Option<Range<usize>> = None;
        let mut last_candidate_ix = None;
        let mut last_candidate_x = px(0.);
        let mut last_boundary = WrapBoundary {
            run_ix: 0,
            glyph_ix: 0,
        };
        let mut last_boundary_x = px(0.);
        // Shaping can combine characters across a line break opportunity, so we ignore those
        // break opportunities that would separate them.
        let WrapScratch {
            glyph_indices,
            break_indices,
        } = scratch;
        glyph_indices.clear();
        glyph_indices.extend(
            self.runs
                .iter()
                .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index)),
        );
        break_indices.clear();
        break_indices
            .extend(LineWrapper::break_indices(text).filter(|index| glyph_indices.contains(index)));
        let grapheme_edges: SmallVec<[usize; 64]> = if overflow_wrap.is_some() {
            use unicode_segmentation::UnicodeSegmentation as _;
            text.grapheme_indices(true).map(|(index, _)| index).collect()
        } else { SmallVec::new() };
        if overflow_wrap.is_some() {
            break_indices.retain(|index| grapheme_edges.binary_search(index).is_ok());
        }
        let mut glyphs = self.visual_clusters()
            .map(|cluster| (cluster.first_glyph, cluster.index, cluster.left))
            .peekable();

        while let Some((boundary, index, x)) = glyphs.next() {
            let byte = text.as_bytes()[index];
            if byte == b'\n' {
                continue;
            }

            if byte != b' '
                && current_segment
                    .as_ref()
                    .is_none_or(|segment| !segment.contains(&index))
            {
                // Detect transitions between line-break segments rather than matching glyph
                // indices directly, to accommodate RTL text. Workaround for the lack of complete
                // UAX #9 support.
                let segment = break_indices.partition_point(|break_index| *break_index <= index);
                if current_segment.is_some() {
                    last_candidate_ix = Some(boundary);
                    last_candidate_x = x;
                }
                let start = segment
                    .checked_sub(1)
                    .and_then(|previous| break_indices.get(previous))
                    .copied()
                    .unwrap_or(0);
                let end = break_indices.get(segment).copied().unwrap_or(text.len());
                current_segment = Some(start..end);
            }

            let next_x = glyphs.peek().map_or(self.width, |(_, _, x)| *x);
            let width = next_x - last_boundary_x;

            if width > wrap_width && boundary > last_boundary
                && (last_candidate_ix.is_some() || match overflow_wrap {
                    Some(OverflowWrap::Normal) => false,
                    Some(OverflowWrap::Anywhere) => grapheme_edges.binary_search(&index).is_ok(),
                    None => true,
                }) {
                // When used line_clamp, we should limit the number of lines.
                if let Some(max_lines) = max_lines
                    && boundaries.len() >= max_lines.saturating_sub(1)
                {
                    break;
                }

                if let Some(last_candidate_ix) = last_candidate_ix.take() {
                    last_boundary = last_candidate_ix;
                    last_boundary_x = last_candidate_x;
                } else {
                    last_boundary = boundary;
                    last_boundary_x = x;
                }
                boundaries.push(last_boundary);
            }
        }

        boundaries
    }
}

/// A line of text that has been wrapped to fit a given width
#[derive(Clone, Default, Debug)]
pub struct WrappedLineLayout {
    /// Shaped glyphs in paint order. Wrapped bidi text stores successive visual
    /// rows here while retaining the original logical UTF-8 source indices.
    pub unwrapped_layout: Arc<LineLayout>,

    /// The boundaries at which the line was wrapped
    pub wrap_boundaries: SmallVec<[WrapBoundary; 1]>,

    /// The width of the line, if it was wrapped
    pub wrap_width: Option<Pixels>,
}

/// A boundary at which a line was wrapped
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WrapBoundary {
    /// The index in the run just before the line was wrapped
    pub run_ix: usize,
    /// The index of the glyph just before the line was wrapped
    pub glyph_ix: usize,
}

impl WrappedLineLayout {
    /// The length of the underlying text, in utf8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.unwrapped_layout.len
    }

    /// The width of this line, in pixels, whether or not it was wrapped.
    pub fn width(&self) -> Pixels {
        self.wrap_width
            .unwrap_or(Pixels::MAX)
            .min(self.unwrapped_layout.width)
    }

    /// The size of the whole wrapped text, for the given line_height.
    /// can span multiple lines if there are multiple wrap boundaries.
    pub fn size(&self, line_height: Pixels) -> Size<Pixels> {
        Size {
            width: self.width(),
            height: line_height * (self.wrap_boundaries.len() + 1),
        }
    }

    /// The ascent of a line in this layout
    pub fn ascent(&self) -> Pixels {
        self.unwrapped_layout.ascent
    }

    /// The descent of a line in this layout
    pub fn descent(&self) -> Pixels {
        self.unwrapped_layout.descent
    }

    /// The wrap boundaries in this layout
    pub fn wrap_boundaries(&self) -> &[WrapBoundary] {
        &self.wrap_boundaries
    }

    /// Logical source intervals, in row order. A bidi row's first painted
    /// glyph can be its last source cluster, so it cannot supply the cut.
    /// This derives only row metadata and never rewrites the source text.
    pub fn source_row_ranges(&self) -> SmallVec<[Range<usize>; 1]> {
        let mut rows: SmallVec<[Range<usize>; 1]> = smallvec::smallvec![0..self.len()];
        let mut boundaries = self.wrap_boundaries.iter().peekable();
        let mut row = 0;
        for (run_ix, run) in self.unwrapped_layout.runs.iter().enumerate() {
            for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                let location = WrapBoundary { run_ix, glyph_ix };
                if boundaries.peek().is_some_and(|boundary| **boundary == location) {
                    boundaries.next();
                    row += 1;
                    rows.push(self.len()..self.len());
                }
                rows[row].start = rows[row].start.min(glyph.index);
            }
        }
        for index in 0..rows.len().saturating_sub(1) { rows[index].end = rows[index + 1].start; }
        rows
    }

    /// The font size of this layout
    pub fn font_size(&self) -> Pixels {
        self.unwrapped_layout.font_size
    }

    /// The runs in this layout, sans wrapping
    pub fn runs(&self) -> &[ShapedRun] {
        &self.unwrapped_layout.runs
    }

    /// The physical left of a soft row within the aligned container.
    pub fn alignment_offset(&self, row: usize, align: TextAlign, container_width: Pixels) -> Option<Pixels> {
        let range = self.unwrapped_layout.visual_row_ranges(&self.wrap_boundaries).nth(row)?;
        let width = range.end - range.start;
        Some(match align {
            TextAlign::Left => Pixels::ZERO,
            TextAlign::Center => (container_width - width) / 2.,
            TextAlign::Right => container_width - width,
        })
    }

    /// The index corresponding to a given position in this layout for the given line height.
    ///
    /// See also [`Self::closest_index_for_position`].
    pub fn index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        let layout = &self.unwrapped_layout;
        let wrapped_line_ix = (position.y / line_height) as usize;
        let Some(row) = layout.visual_row_ranges(&self.wrap_boundaries).nth(wrapped_line_ix) else {
            return Err(self.len());
        };
        let x = position.x + row.start;
        if x < row.start || x >= row.end {
            return Err(layout.closest_caret_for_x_in_range(x, row).index);
        }
        layout.index_for_x(x).ok_or(self.len())
    }

    /// The closest index to a given position in this layout for the given line height.
    ///
    /// Closest means the character boundary closest to the given position.
    ///
    /// See also [`LineLayout::closest_index_for_x`].
    pub fn closest_index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self.closest_caret_for_position(position, line_height)
            .map(|hit| hit.index).map_err(|hit| hit.index)
    }

    /// The nearest visual caret on the actual row; both in-bounds and clamped
    /// hits retain their side of a bidi/soft-wrap source boundary.
    pub fn closest_caret_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<TextHit, TextHit> {
        let wrapped_line_ix = (position.y / line_height) as usize;

        let layout = &self.unwrapped_layout;
        let Some(row) = layout.visual_row_ranges(&self.wrap_boundaries).nth(wrapped_line_ix) else {
            return Err(TextHit { index: self.len(), affinity: CaretAffinity::Upstream });
        };
        let x = position.x + row.start;
        // A row's physical edges need not be its logical start/end, including
        // out-of-bounds drag positions. Clamp against the actual row's cells.
        let outside = x < row.start || x >= row.end;
        let hit = layout.closest_caret_for_x_in_range(x, row);
        if outside { Err(hit) } else { Ok(hit) }
    }

    /// Returns the pixel position for the given byte index.
    pub fn position_for_index(&self, index: usize, line_height: Pixels) -> Option<Point<Pixels>> {
        self.position_for_index_inner(index, line_height, None)
    }

    /// Resolves the requested logical side at bidi and soft-wrap boundaries.
    pub fn position_for_index_with_affinity(
        &self,
        index: usize,
        line_height: Pixels,
        affinity: CaretAffinity,
    ) -> Option<Point<Pixels>> {
        self.position_for_index_inner(index, line_height, Some(affinity))
    }

    fn position_for_index_inner(
        &self,
        index: usize,
        line_height: Pixels,
        affinity: Option<CaretAffinity>,
    ) -> Option<Point<Pixels>> {
        if index > self.len() { return None; }
        let (upstream, downstream) = self.unwrapped_layout.carets_for_index(index);
        let row_for = |caret: VisualCaret| {
            self.wrap_boundaries.partition_point(|boundary| *boundary <= caret.glyph)
        };
        let caret = match affinity {
            Some(CaretAffinity::Upstream) => upstream,
            Some(CaretAffinity::Downstream) => downstream,
            // Preserve the existing upstream default at a soft break, and
            // the following-cluster default at an in-row bidi boundary.
            None => match (upstream, downstream) {
                (Some(before), Some(after)) if row_for(before) != row_for(after) => Some(before),
                _ => downstream,
            },
        };
        let Some(caret) = caret else { return Some(point(Pixels::ZERO, Pixels::ZERO)); };
        let row_ix = row_for(caret);
        let row = self.unwrapped_layout.visual_row_ranges(&self.wrap_boundaries).nth(row_ix)?;
        Some(point(caret.x - row.start, line_height * row_ix))
    }
}

pub(crate) struct LineLayoutCache {
    state: RefCell<LineLayoutCacheState>,
    wrap_scratch: RefCell<WrapScratch>,
    platform_text_system: Arc<dyn PlatformTextSystem>,
    /// Advances when [`TextSystem::add_fonts`] successfully changes the font database.
    font_generation: Arc<AtomicUsize>,
    /// Records the generation represented by both frame caches.
    cached_font_generation: AtomicUsize,
}

#[derive(Default)]
struct WrapScratch {
    glyph_indices: FxHashSet<usize>,
    break_indices: Vec<usize>,
}

#[derive(Default)]
struct LineLayoutCacheState {
    previous_frame: FrameCache,
    current_frame: FrameCache,
}

#[derive(Default)]
struct FrameCache {
    lines: FxHashMap<Arc<CacheKey>, Arc<LineLayout>>,
    wrapped_lines: FxHashMap<Arc<CacheKey>, Arc<WrappedLineLayout>>,
    used_lines: Vec<Arc<CacheKey>>,
    used_wrapped_lines: Vec<Arc<CacheKey>>,
}

#[derive(Clone, Default)]
pub(crate) struct LineLayoutIndex {
    font_generation: usize,
    lines_index: usize,
    wrapped_lines_index: usize,
}

impl LineLayoutCache {
    pub fn new(
        platform_text_system: Arc<dyn PlatformTextSystem>,
        font_generation: Arc<AtomicUsize>,
    ) -> Self {
        let cached_font_generation = font_generation.load(Ordering::Acquire);
        Self {
            state: RefCell::default(),
            wrap_scratch: RefCell::default(),
            platform_text_system,
            font_generation,
            cached_font_generation: AtomicUsize::new(cached_font_generation),
        }
    }

    pub fn layout_index(&self) -> LineLayoutIndex {
        let font_generation = self.clear_if_font_generation_changed();
        let state = self.state.borrow();
        let frame = &state.current_frame;
        LineLayoutIndex {
            font_generation,
            lines_index: frame.used_lines.len(),
            wrapped_lines_index: frame.used_wrapped_lines.len(),
        }
    }

    pub fn reuse_layouts(&self, range: Range<LineLayoutIndex>) {
        let font_generation = self.clear_if_font_generation_changed();
        if range.start.font_generation != font_generation
            || range.end.font_generation != font_generation
        {
            return;
        }
        let mut state = self.state.borrow_mut();
        let LineLayoutCacheState {
            previous_frame,
            current_frame,
        } = &mut *state;

        for key in &previous_frame.used_lines[range.start.lines_index..range.end.lines_index] {
            if let Some((key, line)) = previous_frame.lines.remove_entry(key) {
                current_frame.lines.insert(key, line);
            }
            current_frame.used_lines.push(key.clone());
        }

        for key in &previous_frame.used_wrapped_lines
            [range.start.wrapped_lines_index..range.end.wrapped_lines_index]
        {
            if let Some((key, line)) = previous_frame.wrapped_lines.remove_entry(key) {
                current_frame.wrapped_lines.insert(key, line);
            }
            current_frame.used_wrapped_lines.push(key.clone());
        }
    }

    pub fn truncate_layouts(&self, index: LineLayoutIndex) {
        let font_generation = self.clear_if_font_generation_changed();
        if index.font_generation != font_generation {
            return;
        }
        let mut state = self.state.borrow_mut();
        let current_frame = &mut state.current_frame;
        current_frame.used_lines.truncate(index.lines_index);
        current_frame
            .used_wrapped_lines
            .truncate(index.wrapped_lines_index);
    }

    pub fn finish_frame(&self) {
        let _font_generation = self.clear_if_font_generation_changed();
        let LineLayoutCacheState {
            previous_frame,
            current_frame,
        } = &mut *self.state.borrow_mut();
        std::mem::swap(previous_frame, current_frame);
        current_frame.lines.clear();
        current_frame.wrapped_lines.clear();
        current_frame.used_lines.clear();
        current_frame.used_wrapped_lines.clear();
    }

    pub fn layout_wrapped_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[FontRun],
        wrap_width: Option<Pixels>,
        max_lines: Option<usize>,
        direction: TextDirection,
    ) -> Arc<WrappedLineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        self.layout_wrapped_line_with_policy(text, font_size, runs, wrap_width, max_lines, direction, None, true)
    }

    pub(crate) fn layout_wrapped_line_with_policy<Text>(
        &self, text: Text, font_size: Pixels, runs: &[FontRun],
        wrap_width: Option<Pixels>, max_lines: Option<usize>, direction: TextDirection,
        overflow_wrap: Option<OverflowWrap>,
        cache_wrapping: bool,
    ) -> Arc<WrappedLineLayout>
    where Text: AsRef<str>, SharedString: From<Text>,
    {
        let _font_generation = self.clear_if_font_generation_changed();
        // Unwrapped glyph layouts are identical for every overflow policy.
        let overflow_wrap = wrap_width.and(overflow_wrap);
        if !cache_wrapping {
            let text = SharedString::from(text);
            let layout = self.layout_line_with_direction::<&SharedString>(&text, font_size, runs, None, direction);
            return self.wrap_layout(layout, &text, wrap_width, max_lines, direction, overflow_wrap);
        }
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width,
            overflow_wrap,
            force_width: None,
            direction,
            max_lines,
            isolates: &[],
        } as &dyn AsCacheKeyRef;

        let state = self.state.borrow();
        if let Some(layout) = state.current_frame.wrapped_lines.get(key) {
            return layout.clone();
        }
        drop(state);

        let mut state = self.state.borrow_mut();
        let previous_frame_entry = state.previous_frame.wrapped_lines.remove_entry(key);
        if let Some((key, layout)) = previous_frame_entry {
            state
                .current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            state.current_frame.used_wrapped_lines.push(key);
            layout
        } else {
            drop(state);
            let text = SharedString::from(text);
            let unwrapped_layout = self.layout_line_with_direction::<&SharedString>(
                &text, font_size, runs, None, direction,
            );
            let layout = self.wrap_layout(unwrapped_layout, &text, wrap_width, max_lines, direction, overflow_wrap);
            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width,
                overflow_wrap,
                force_width: None,
                direction,
                max_lines,
                isolates: None,
            });

            let mut state = self.state.borrow_mut();
            state
                .current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            state.current_frame.used_wrapped_lines.push(key);

            layout
        }
    }

    pub fn layout_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
    ) -> Arc<LineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        self.layout_line_with_direction(text, font_size, runs, force_width, TextDirection::Auto)
    }

    // Min-content probes reuse glyph shaping without retaining their possibly
    // very many soft rows in the two-frame wrapped-geometry cache.
    fn wrap_layout(
        &self, unwrapped_layout: Arc<LineLayout>, text: &str,
        wrap_width: Option<Pixels>, max_lines: Option<usize>, direction: TextDirection,
        overflow_wrap: Option<OverflowWrap>,
    ) -> Arc<WrappedLineLayout> {
        let (unwrapped_layout, wrap_boundaries) = if let Some(wrap_width) = wrap_width {
            if let Some(wrapped) = bidi::wrap(&unwrapped_layout, &text, wrap_width, max_lines, direction, overflow_wrap) {
                wrapped
            } else {
                let boundaries = unwrapped_layout.compute_wrap_boundaries(
                    text.as_ref(), wrap_width, max_lines, &mut self.wrap_scratch.borrow_mut(), overflow_wrap,
                );
                (unwrapped_layout, boundaries)
            }
        } else {
            (unwrapped_layout, SmallVec::new())
        };
        Arc::new(WrappedLineLayout {
            unwrapped_layout,
            wrap_boundaries,
            wrap_width,
        })
    }

    pub fn layout_line_with_direction<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
        direction: TextDirection,
    ) -> Arc<LineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        self.layout_line_with_isolates(text, font_size, runs, force_width, direction, &[])
    }

    pub fn layout_line_with_isolates<Text>(
        &self, text: Text, font_size: Pixels, runs: &[FontRun],
        force_width: Option<Pixels>, direction: TextDirection, isolates: &[super::TextIsolation],
    ) -> Arc<LineLayout>
    where Text: AsRef<str>, SharedString: From<Text>,
    {
        let _font_generation = self.clear_if_font_generation_changed();
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width: None,
            overflow_wrap: None,
            force_width,
            direction,
            max_lines: None,
            isolates,
        } as &dyn AsCacheKeyRef;

        let state = self.state.borrow();
        if let Some(layout) = state.current_frame.lines.get(key) {
            return layout.clone();
        }
        drop(state);

        let mut state = self.state.borrow_mut();
        let LineLayoutCacheState {
            previous_frame,
            current_frame,
        } = &mut *state;
        if let Some((key, layout)) = previous_frame.lines.remove_entry(key) {
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        } else {
            let text = SharedString::from(text);
            let mut layout = if isolates.is_empty() {
                self.platform_text_system.layout_line_with_direction(&text, font_size, runs, direction)
            } else {
                let source = Arc::new(super::isolation::IsolationSource::new(&text, isolates));
                // Empty scopes carry no glyphs. Keep the backend's ordinary
                // empty-line metrics instead of sending control-only text
                // without a covering font run when the caller supplied none.
                let mut layout = if text.is_empty() {
                    self.platform_text_system.layout_line_with_direction(&text, font_size, runs, direction)
                } else {
                    let presentation_runs = source.font_runs(runs);
                    self.platform_text_system.layout_line_with_direction(
                        &source.text, font_size, &presentation_runs, direction,
                    )
                };
                source.restore_layout(&mut layout, direction);
                layout
            };

            if let Some(force_width) = force_width {
                apply_force_width_to_layout(&mut layout, force_width);
            }

            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width: None,
                overflow_wrap: None,
                force_width,
                direction,
                max_lines: None,
                isolates: (!isolates.is_empty()).then(|| Arc::from(isolates)),
            });
            let layout = Arc::new(layout);
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        }
    }

    fn clear_if_font_generation_changed(&self) -> usize {
        let font_generation = self.font_generation.load(Ordering::Acquire);
        if self.cached_font_generation.load(Ordering::Acquire) == font_generation {
            return font_generation;
        }

        let mut state = self.state.borrow_mut();
        if self.cached_font_generation.load(Ordering::Acquire) == font_generation {
            return font_generation;
        }

        *state = LineLayoutCacheState::default();
        self.cached_font_generation
            .store(font_generation, Ordering::Release);
        font_generation
    }
}

// Combining marks (e.g. Thai vowel signs, Arabic diacritics) are shaped by
// HarfBuzz at the same x position as their base character. The force-width
// loop must not advance the cell counter for these zero-advance glyphs,
// otherwise they get displaced into the next cell. We detect them by checking
// whether shaped x has advanced by at least half a cell beyond the last base.
fn apply_force_width_to_layout(layout: &mut LineLayout, force_width: Pixels) {
    let mut carets = std::mem::take(&mut layout.interior_carets);
    let mut original_cells = SmallVec::<[(usize, Pixels, Pixels); 4]>::new();
    if !carets.is_empty() {
        for cluster in layout.visual_clusters() {
            let start = carets.partition_point(|caret| caret.cluster_start < cluster.index);
            if carets.get(start).is_some_and(|caret| caret.cluster_start == cluster.index) {
                original_cells.push((cluster.index, cluster.left, cluster.right));
            }
        }
        original_cells.sort_unstable_by_key(|cell| cell.0);
    }
    let mut glyph_pos: usize = 0;
    // NEG_INFINITY ensures the first glyph is always classified as a base.
    let mut last_base_shaped_x = px(f32::NEG_INFINITY);
    let mut last_base_actual_x = px(0.);

    for run in layout.runs.iter_mut() {
        for glyph in run.glyphs.iter_mut() {
            let shaped_x = glyph.position.x;

            if shaped_x > last_base_shaped_x + force_width * 0.5 {
                let forced_x = glyph_pos * force_width;
                if (shaped_x - forced_x).abs() > px(1.) {
                    glyph.position.x = forced_x;
                }
                last_base_shaped_x = shaped_x;
                last_base_actual_x = glyph.position.x;
                glyph.advance = force_width;
                glyph_pos += 1;
            } else {
                glyph.position.x = last_base_actual_x + (shaped_x - last_base_shaped_x);
                glyph.advance = Pixels::ZERO;
            }
        }
    }
    // The forced cell grid owns both caret advances and the final line extent;
    // retaining the unforced width would clip or invert the last caret cell.
    layout.width = glyph_pos * force_width;
    if !carets.is_empty() {
        for cluster in layout.visual_clusters() {
            let Ok(original) = original_cells.binary_search_by_key(&cluster.index, |cell| cell.0) else { continue; };
            let (_, left, right) = original_cells[original];
            let start = carets.partition_point(|caret| caret.cluster_start < cluster.index);
            for caret in carets[start..].iter_mut()
                .take_while(|caret| caret.cluster_start == cluster.index)
            {
                let fraction = if right > left { (caret.x - left) / (right - left) } else { 0.0 };
                caret.x = cluster.left + (cluster.right - cluster.left) * fraction;
            }
        }
    }
    layout.interior_carets = carets;
}

/// A run of text with a single font.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[expect(missing_docs)]
pub struct FontRun {
    pub len: usize,
    pub font_id: FontId,
    /// An authored run size; `None` uses the paragraph's default size.
    pub font_size: Option<Pixels>,
}

/// A shaping call's source-size index. Ordinary uniformly sized text has no
/// entries or heap allocation. Native visual runs can query authored sizes
/// without preserving the tiny size perturbations used to separate ligatures.
pub struct FontRunSizeMap<'a> {
    runs: &'a [FontRun],
    ends: SmallVec<[usize; 4]>,
}

impl<'a> FontRunSizeMap<'a> {
    /// Borrow the existing run inputs; index only paragraphs with sized runs.
    pub fn new(runs: &'a [FontRun]) -> Self {
        let mut ends = SmallVec::new();
        if runs.iter().any(|run| run.font_size.is_some()) {
            let mut end = 0;
            for run in runs { end += run.len; ends.push(end); }
        }
        Self { runs, ends }
    }

    /// Whether every run inherits the paragraph size.
    pub fn is_empty(&self) -> bool { self.ends.is_empty() }

    /// The authored size at a UTF-8 byte offset; `None` inherits the line size.
    pub fn font_size_at(&self, index: usize) -> Option<Pixels> {
        if self.ends.is_empty() { return None; }
        self.runs.get(self.ends.partition_point(|end| *end <= index)).and_then(|run| run.font_size)
    }
}

trait AsCacheKeyRef {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_>;
}

#[derive(Clone, Debug, Eq)]
struct CacheKey {
    text: SharedString,
    font_size: Pixels,
    runs: SmallVec<[FontRun; 1]>,
    wrap_width: Option<Pixels>,
    overflow_wrap: Option<OverflowWrap>,
    force_width: Option<Pixels>,
    direction: TextDirection,
    max_lines: Option<usize>,
    isolates: Option<Arc<[super::TextIsolation]>>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct CacheKeyRef<'a> {
    text: &'a str,
    font_size: Pixels,
    runs: &'a [FontRun],
    wrap_width: Option<Pixels>,
    overflow_wrap: Option<OverflowWrap>,
    force_width: Option<Pixels>,
    direction: TextDirection,
    max_lines: Option<usize>,
    isolates: &'a [super::TextIsolation],
}

impl PartialEq for dyn AsCacheKeyRef + '_ {
    fn eq(&self, other: &dyn AsCacheKeyRef) -> bool {
        self.as_cache_key_ref() == other.as_cache_key_ref()
    }
}

impl Eq for dyn AsCacheKeyRef + '_ {}

impl Hash for dyn AsCacheKeyRef + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state)
    }
}

impl AsCacheKeyRef for CacheKey {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        CacheKeyRef {
            text: &self.text,
            font_size: self.font_size,
            runs: self.runs.as_slice(),
            wrap_width: self.wrap_width,
            overflow_wrap: self.overflow_wrap,
            force_width: self.force_width,
            direction: self.direction,
            max_lines: self.max_lines,
            isolates: self.isolates.as_deref().unwrap_or(&[]),
        }
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_cache_key_ref().eq(&other.as_cache_key_ref())
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state);
    }
}

impl<'a> Borrow<dyn AsCacheKeyRef + 'a> for Arc<CacheKey> {
    fn borrow(&self) -> &(dyn AsCacheKeyRef + 'a) {
        self.as_ref() as &dyn AsCacheKeyRef
    }
}

impl AsCacheKeyRef for CacheKeyRef<'_> {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GlyphId;

    fn glyph_at(x: f32, index: usize) -> ShapedGlyph {
        ShapedGlyph {
            id: GlyphId(0),
            position: point(px(x), px(0.)),
            index,
            is_emoji: false,
            is_rtl: false,
            advance: px(10.0),
        }
    }

    fn make_layout(glyphs: Vec<ShapedGlyph>) -> LineLayout {
        LineLayout {
            font_size: px(16.),
            width: px(100.),
            ascent: px(12.),
            descent: px(4.),
            runs: vec![ShapedRun { font_size: None,
                font_id: FontId(0),
                glyphs,
            }],
            len: 0,
            interior_carets: Vec::new(),
            isolation: None,
            ink_clip: None,
        }
    }

    fn glyph_x_positions(layout: &LineLayout) -> Vec<f32> {
        layout.runs[0]
            .glyphs
            .iter()
            .map(|g| f32::from(g.position.x))
            .collect()
    }

    #[test]
    fn test_wrap_preserves_shaped_clusters() {
        let mut layout = make_layout(vec![
            glyph_at(0., 0),
            glyph_at(5., 0),
            glyph_at(10., 3),
            glyph_at(15., 3),
        ]);
        layout.width = px(20.);
        layout.len = "你好".len();
        assert_eq!(
            layout
                .compute_wrap_boundaries("你好", px(15.), None, &mut WrapScratch::default())
                .as_slice(),
            &[WrapBoundary {
                run_ix: 0,
                glyph_ix: 2
            }],
        );

        let mut layout = make_layout(vec![glyph_at(0., 0), glyph_at(10., 0)]);
        layout.width = px(20.);
        layout.len = 1;
        assert!(
            layout
                .compute_wrap_boundaries("a", px(10.), None, &mut WrapScratch::default())
                .is_empty()
        );
    }

    #[test]
    fn test_wrap_right_to_left_text() {
        for (text, indices, expected_boundaries) in [
            ("אבג דהו", vec![11, 9, 7, 6, 4, 2, 0], vec![4]),
            ("a אבג b", vec![0, 1, 6, 4, 2, 8, 9], vec![2, 6]),
        ] {
            let mut layout = make_layout(
                indices
                    .iter()
                    .enumerate()
                    .map(|(position, index)| glyph_at(position as f32 * 10., *index))
                    .collect(),
            );
            layout.width = px(indices.len() as f32 * 10.);
            layout.len = text.len();
            assert_eq!(
                layout.compute_wrap_boundaries(text, px(40.), None, &mut WrapScratch::default()),
                expected_boundaries
                    .into_iter()
                    .map(|glyph_ix| WrapBoundary {
                        run_ix: 0,
                        glyph_ix
                    })
                    .collect::<SmallVec<[WrapBoundary; 1]>>(),
                "{text:?}",
            );
        }
    }

    #[test]
    fn test_wrap_ignores_breaks_inside_shaped_clusters() {
        let text = "x a-bcde";
        let mut layout = make_layout(
            [0, 1, 2, 5, 6, 7]
                .into_iter()
                .map(|index| glyph_at(index as f32 * 10., index))
                .collect(),
        );
        layout.width = px(80.);
        layout.len = text.len();
        assert_eq!(
            layout
                .compute_wrap_boundaries(text, px(60.), None, &mut WrapScratch::default())
                .as_slice(),
            &[WrapBoundary {
                run_ix: 0,
                glyph_ix: 2
            }],
        );
    }

    #[test]
    fn test_force_width_latin_unchanged() {
        let cell_width = px(8.);
        let mut layout = make_layout(vec![glyph_at(0., 0), glyph_at(8., 1), glyph_at(16., 2)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 8., 16.]);
    }

    #[test]
    fn test_force_width_combining_marks_not_advanced() {
        let cell_width = px(8.);
        // Simulates Thai "กี" — base consonant at x=0, combining vowel also at x=0
        let mut layout = make_layout(vec![
            glyph_at(0., 0), // ก (base)
            glyph_at(0., 3), // ี (combining mark, same x)
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0.]);
    }

    #[test]
    fn test_force_width_base_after_combining_mark() {
        let cell_width = px(8.);
        let mut layout = make_layout(vec![glyph_at(0., 0), glyph_at(0., 3), glyph_at(8., 6)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0., 8.]);
    }

    #[test]
    fn test_force_width_multiple_combining_marks() {
        let cell_width = px(8.);
        // Simulates "ก้" — base + vowel + tone mark (two combining marks stacked)
        let mut layout = make_layout(vec![
            glyph_at(0., 0), // ก (base)
            glyph_at(0., 3), // vowel (combining)
            glyph_at(0., 6), // tone mark (combining)
            glyph_at(8., 9), // next base
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0., 0., 8.]);
    }

    #[test]
    fn test_force_width_corrects_drifted_base_positions() {
        let cell_width = px(8.);
        // Font metrics don't perfectly match cell grid — glyphs drift >1px from cell boundary
        let mut layout = make_layout(vec![
            glyph_at(0.5, 0),  // within 1px tolerance, kept as-is
            glyph_at(10.2, 1), // >1px off from 8.0, corrected
            glyph_at(19.8, 2), // >1px off from 16.0, corrected
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0.5, 8., 16.]);
    }

    #[test]
    fn test_force_width_combining_mark_after_within_tolerance_base() {
        let cell_width = px(8.);
        // Base glyph is within 1px of grid so it keeps its shaped position.
        // The combining mark must align to the base's actual position, not the grid slot.
        let mut layout = make_layout(vec![glyph_at(0.5, 0), glyph_at(0.5, 3)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0.5, 0.5]);
    }
}
