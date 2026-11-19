#[cfg(test)]
use crate::SharedString;
use crate::window::PreparedGlyph;
use crate::{
    App, Bounds, DevicePixels, GlyphInk, Half, Hsla, LineLayout, Pixels, Point, RenderGlyphParams, Result,
    ScaledPixels, TextDecorationPattern, ClusterSpacing, TextDirection,
    StrikethroughStyle, TextAlign, UnderlineStyle, Window, WrapBoundary,
    WrappedLineLayout, black, fill, point, px, size, underline_y_offset,
};
use derive_more::{Deref, DerefMut};
use smallvec::SmallVec;
use std::{ops::Range, sync::Arc};
use unicode_script::{Script, UnicodeScript as _};
use super::line_layout::CursorBidi;

/// Pre-computed glyph data for efficient painting without per-glyph cache lookups.
///
/// This is produced by `ShapedLine::compute_glyph_raster_data` during prepaint
/// and consumed by `ShapedLine::paint_with_raster_data` during paint.
#[derive(Clone, Debug)]
pub struct GlyphRasterData {
    /// The raster bounds for each glyph, in paint order.
    pub bounds: Vec<Bounds<DevicePixels>>,
    /// The render params for each glyph (needed for sprite atlas lookup).
    pub params: Vec<RenderGlyphParams>,
}

/// Set the text decoration for a run of text.
#[derive(Debug, Clone, Default)]
pub struct DecorationRun {
    /// The length of the run in utf-8 bytes.
    pub len: u32,

    /// The color for this run
    pub color: Hsla,

    /// The background color for this run
    pub background_color: Option<Hsla>,

    /// The underline style for this run
    pub underline: Option<UnderlineStyle>,
    /// The overline style for this run.
    pub overline: Option<UnderlineStyle>,

    /// The strikethrough style for this run
    pub strikethrough: Option<StrikethroughStyle>,
    /// Complete decorating-box ancestry. When present, this replaces the
    /// three legacy slots; glyph color and background still come from the run.
    pub decoration_layers: Option<DecorationLayers>,
}

/// One immutable decorating box. Parent links share ancestors across text
/// runs instead of flattening every nested box into another retained array.
#[derive(Debug)]
pub struct DecorationLayer {
    /// Stable source identity, shared across resolutions and retained layouts.
    pub identity: DecorationLayerIdentity,
    /// The next outer decorating box.
    pub parent: Option<Arc<DecorationLayer>>,
    /// This box's underline, relative to the unshifted paragraph baseline.
    pub underline: Option<UnderlineStyle>,
    /// This box's overline, relative to the unshifted paragraph baseline.
    pub overline: Option<UnderlineStyle>,
    /// This box's line-through, relative to the unshifted paragraph baseline.
    pub strikethrough: Option<StrikethroughStyle>,
}

/// An existing source allocation supplies identity and lifetime ownership;
/// creating this handle allocates neither a token nor a second source copy.
#[derive(Clone)]
pub struct DecorationLayerIdentity(Arc<dyn std::any::Any + Send + Sync>);

impl DecorationLayerIdentity {
    /// Retains one immutable source box for identity comparisons.
    pub fn new<T: std::any::Any + Send + Sync>(source: Arc<T>) -> Self { Self(source) }

    fn address(&self) -> usize { Arc::as_ptr(&self.0) as *const () as usize }
}

impl std::fmt::Debug for DecorationLayerIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("DecorationLayerIdentity").finish_non_exhaustive()
    }
}

impl PartialEq for DecorationLayerIdentity {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}

impl Eq for DecorationLayerIdentity {}

/// A resolved chain plus the current text fragment's upward displacement.
/// Identity distinguishes independent boxes with otherwise identical strokes.
#[derive(Debug, Clone)]
pub struct DecorationLayers {
    /// The innermost decorating box.
    pub head: Arc<DecorationLayer>,
    /// Added to baseline-relative offsets to cancel the fragment's movement.
    pub baseline_shift: Pixels,
}

impl PartialEq for DecorationLayers {
    fn eq(&self, other: &Self) -> bool {
        if self.baseline_shift != other.baseline_shift { return false; }
        let mut left = &self.head;
        let mut right = &other.head;
        loop {
            if Arc::ptr_eq(left, right) { return true; }
            if left.identity != right.identity || left.underline != right.underline || left.overline != right.overline
                || left.strikethrough != right.strikethrough { return false; }
            match (&left.parent, &right.parent) {
                (None, None) => return true,
                (Some(next_left), Some(next_right)) => { left = next_left; right = next_right; }
                _ => return false,
            }
        }
    }
}

impl Eq for DecorationLayers {}

/// A line of text that has been shaped and decorated.
#[derive(Clone, Default, Debug, Deref, DerefMut)]
pub struct ShapedLine {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<LineLayout>,
    /// The text that was shaped for this line.
    pub text: super::ShapedText,
    // Most measured hard lines have one paint run. Keeping 32 inline entries
    // makes each retained line reserve several KiB even for plain text.
    pub(crate) decoration_runs: SmallVec<[DecorationRun; 1]>,
}

/// Used tab geometry after resolving the font and size. Measurement and
/// native glyph placement share this rule without replacing source bytes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStops {
    interval: f64,
    minimum: f64,
}

/// Font-resolved tab geometry over a hard-paragraph UTF-8 source interval.
/// Sorted disjoint runs may bridge text without tabs when their used tab
/// geometry matches. A host need not store one metric pair per occurrence.
#[derive(Clone, Debug)]
pub struct TabStopRun {
    /// UTF-8 source interval in the containing hard paragraph.
    pub range: Range<usize>,
    /// Used geometry shared by preserved tabs within this interval.
    pub stops: TabStops,
}

impl TabStops {
    /// A zero interval collapses the tab. Invalid metrics cannot form stops.
    pub fn new(interval: Pixels, minimum: Pixels) -> Option<Self> {
        if !interval.as_f32().is_finite() || interval < Pixels::ZERO
            || !minimum.as_f32().is_finite() || minimum < Pixels::ZERO { return None; }
        Some(Self { interval: f64::from(interval.as_f32()), minimum: f64::from(minimum.as_f32()) })
    }

    /// Logical distance from the soft row's start edge to the tab's end.
    /// Keep repeated stop calculations in f64 until entering pixel geometry.
    pub fn next_position(self, start: f64) -> f64 {
        if self.interval == 0. { return start; }
        let mut target = self.interval * ((start / self.interval).floor() + 1.);
        if target - start < self.minimum { target += self.interval; }
        target
    }
}

impl ShapedLine {
    /// Source-order native advances with sorted, unique host objects merged
    /// into their actual source positions. Objects replace native placeholder
    /// advances and are also emitted when the backend omitted that glyph.
    /// Object indices are relative to the hard paragraph; this line starts
    /// at `source_offset` within it. No retained index or source copy is made.
    pub fn source_advances_with_objects<'a>(&'a self, objects: &'a [crate::InlineObjectAdvance], source_offset: usize)
        -> impl Iterator<Item = (usize, f64)> + 'a
    {
        let first = objects.partition_point(|object| object.index < source_offset);
        let last = first + objects[first..].partition_point(|object| object.index < source_offset + self.len());
        let mut objects = objects[first..last].iter().peekable();
        let mut advances = self.layout.source_advances().peekable();
        std::iter::from_fn(move || {
            let glyph_index = advances.peek().map(|(index, _)| *index);
            match (glyph_index, objects.peek().map(|object| object.index - source_offset)) {
                (Some(glyph), Some(object)) if glyph < object => advances.next(),
                (_, Some(index)) => {
                    let object = objects.next()?;
                    if glyph_index == Some(index) { advances.next(); }
                    Some((index, f64::from(object.width.as_f32())))
                },
                _ => advances.next(),
            }
        })
    }

    /// Expand tabs after bidi ordering, relative to the row's start edge.
    /// Pass the hard paragraph's direction before splitting paint contexts.
    /// None means native shaping omitted a tab's independent advance cell.
    /// The source and shared native shaping cache are never modified.
    pub fn with_tab_stops(self, stop: Pixels, minimum: Pixels, direction: TextDirection) -> Option<Self> {
        self.with_tab_stops_for_objects(stop, minimum, direction, &[], 0)
    }

    /// Object indices refer to the containing hard paragraph; `source_offset`
    /// locates this soft row within it. Hosts replace those font placeholder
    /// cells after shaping, so their actual widths also determine tab stops.
    pub fn with_tab_stops_for_objects(self, stop: Pixels, minimum: Pixels, direction: TextDirection,
        objects: &[crate::InlineObjectAdvance], source_offset: usize) -> Option<Self> {
        if !self.text.contains('\t') { return Some(self); }
        let stops = TabStops::new(stop, minimum)?;
        let resolved = [TabStopRun { range: source_offset..source_offset + self.len(), stops }];
        self.with_resolved_tab_stops(&resolved, direction, objects, source_offset)
    }

    /// Resolve each tab against its authored font, using sorted disjoint
    /// hard-paragraph intervals. A soft row borrows the containing paragraph's
    /// geometry; it does not resolve fonts again or rebase source metadata.
    pub fn with_resolved_tab_stops(mut self, resolved: &[TabStopRun], direction: TextDirection,
        objects: &[crate::InlineObjectAdvance], source_offset: usize) -> Option<Self> {
        if !self.text.contains('\t') { return Some(self); }
        let first = resolved.partition_point(|run| run.range.end <= source_offset);
        let resolved = &resolved[first..];
        let rtl = direction.resolve(&self.text) == TextDirection::Rtl;
        let mut tabs = self.layout.visual_cluster_advances()
            .filter_map(|(index, _)| (self.text.as_bytes().get(index) == Some(&b'\t')).then_some(index))
            .collect::<SmallVec<[_; 4]>>();
        if tabs.len() != self.text.bytes().filter(|byte| *byte == b'\t').count() { return None; }
        if rtl { tabs.reverse(); }
        if !tabs.iter().copied().is_sorted() { return None; }
        let mut source_tabs = SmallVec::<[(usize, f64, f64); 4]>::new();
        let mut source_width = 0_f64;
        for (index, advance) in self.source_advances_with_objects(objects, source_offset) {
            let end = source_width + advance;
            if !end.is_finite() { return None; }
            if self.text.as_bytes().get(index) == Some(&b'\t') {
                source_tabs.push((index, source_width, end));
            }
            source_width = end;
        }
        if source_tabs.len() != tabs.len() { return None; }
        let mut source_shift = 0_f64;
        let mut stop_run = 0;
        let mut spacing = SmallVec::<[ClusterSpacing; 4]>::new();
        for (index, (source, before, after)) in tabs.into_iter().zip(source_tabs) {
            if index != source { return None; }
            let at = source_offset + index;
            while resolved.get(stop_run).is_some_and(|run| run.range.end <= at) { stop_run += 1; }
            let stops = resolved.get(stop_run).filter(|run| run.range.contains(&at))?.stops;
            // CSS preserves at least half a '0' glyph before choosing the
            // next stop. Choose the grid with the same source advances as
            // wrapping: subtracting rounded visual coordinates near a stop
            // can otherwise jump an entire extra cell after a soft wrap.
            let target = stops.next_position(before + source_shift);
            let delta = px((target - after - source_shift) as f32);
            source_shift = target - after;
            if delta != Pixels::ZERO {
                spacing.push(ClusterSpacing { index, before: Pixels::ZERO, after: delta });
            }
        }
        if !spacing.is_empty() {
            spacing.sort_unstable_by_key(|gap| gap.index);
            Arc::make_mut(&mut self.layout).add_spacing(&spacing);
        }
        Some(self)
    }

    /// Share glyph geometry independently of its current paint decorations.
    /// The owner must invalidate it when fonts or shaping inputs change.
    pub fn shared_layout(&self) -> Arc<LineLayout> {
        self.layout.clone()
    }

    /// Replace the vertical extent of a source slice without reshaping its
    /// glyphs. Glyph positions are baseline-relative; horizontal cells and
    /// interior carets remain unchanged. An owned slice updates in place,
    /// while a shared layout retains its original metrics.
    pub fn with_vertical_metrics(mut self, ascent: Pixels, descent: Pixels) -> Self {
        assert!(ascent.as_f32().is_finite() && ascent >= Pixels::ZERO);
        assert!(descent.as_f32().is_finite() && descent >= Pixels::ZERO);
        if self.layout.ascent == ascent && self.layout.descent == descent { return self; }
        let layout = Arc::make_mut(&mut self.layout);
        layout.ascent = ascent;
        layout.descent = descent;
        self
    }

    /// Rebase a retained fragment's default size without changing any glyph's
    /// authored size or geometry. The caller supplies the fragment's vertical
    /// metrics separately; a paragraph's aggregate extents need not fit it.
    pub fn with_default_font_size(mut self, font_size: Pixels) -> Self {
        assert!(font_size.as_f32().is_finite() && font_size > Pixels::ZERO);
        if self.layout.font_size == font_size { return self; }
        let layout = Arc::make_mut(&mut self.layout);
        for run in &mut layout.runs {
            let size = run.font_size.unwrap_or(layout.font_size);
            run.font_size = (size != font_size).then_some(size);
        }
        layout.font_size = font_size;
        self
    }

    /// Returns a forward-only cursor for this shaped line.
    pub fn cursor(&self) -> ShapedLineCursor<'_> {
        self.cursor_inner(None)
    }

    /// Splits soft rows while preserving paragraph shaping and applying UAX #9
    /// L1/L2 to each row. The source must be a complete hard paragraph; a font
    /// or object fragment cannot supply the surrounding paragraph's levels.
    pub fn cursor_with_direction(&self, direction: crate::TextDirection) -> ShapedLineCursor<'_> {
        self.cursor_inner(Some(direction))
    }

    fn cursor_inner(&self, direction: Option<crate::TextDirection>) -> ShapedLineCursor<'_> {
        let direction = direction.or_else(|| self.layout.isolation.as_ref().map(|context| context.direction));
        assert_eq!(
            self.len(),
            self.text.len(),
            "cannot split a shaped line with an adjusted length"
        );
        let bidi = direction.and_then(|direction| CursorBidi::new(&self.text, direction, &self.layout)).map(Box::new);
        let glyph_order = if bidi.is_some() || !self
            .layout
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index))
            .is_sorted() { GlyphCursorOrder::Deferred } else { GlyphCursorOrder::ByteOrdered };
        ShapedLineCursor {
            line: self,
            glyph_order,
            bidi,
            paragraph_aware: direction.is_some(),
            byte_index: 0,
            run_index: 0,
            glyph_index: 0,
            decoration_index: 0,
            decoration_offset: 0,
            advance_offset: px(0.),
        }
    }

    /// First strong direction of the enclosing shaped paragraph. Isolated
    /// lines use the exact native presentation source, including source bidi
    /// controls, rather than rescanning a projection with the scopes removed.
    pub fn detected_paragraph_direction(&self) -> Option<TextDirection> {
        if let Some(context) = &self.layout.isolation { context.detected_direction() }
        else { TextDirection::detect_fragments(std::iter::once(self.text.as_ref())) }
    }

    /// The length of the line in utf-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len
    }

    /// The width of the shaped line in pixels.
    ///
    /// This is the glyph advance width computed by the text shaping system and is useful for
    /// incrementally advancing a "pen" when painting multiple fragments on the same row.
    pub fn width(&self) -> Pixels {
        self.layout.width
    }

    /// Override the len, useful if you're rendering text a
    /// as text b (e.g. rendering invisibles).
    pub fn with_len(mut self, len: usize) -> Self {
        let layout = self.layout.as_ref();
        self.layout = Arc::new(LineLayout {
            font_size: layout.font_size,
            width: layout.width,
            ascent: layout.ascent,
            descent: layout.descent,
            runs: layout.runs.clone(),
            len,
            interior_carets: if len == layout.len { layout.interior_carets.clone() } else { Vec::new() },
            isolation: if len == layout.len { layout.isolation.clone() } else { None },
            ink_clip: layout.ink_clip,
        });
        self
    }

    /// Paint the line of text to the window.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &[],
            window,
            cx,
            UnderlinePainter::Native(Some(&self.text)),
        )
    }

    /// Paint the line with a handler for each underline.
    pub fn paint_with_underline_handler(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
        mut paint_underline: impl FnMut(
            Range<usize>,
            Point<Pixels>,
            Pixels,
            &UnderlineStyle,
            &mut Window,
        ),
    ) -> Result<()> {
        paint_line(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &[],
            window,
            cx,
            UnderlinePainter::Custom(&mut paint_underline, Some(&self.text)),
        )
    }

    /// Paint the background of the line to the window.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line_background(
            origin,
            &self.layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &[],
            window,
            cx,
        )?;

        Ok(())
    }

    /// Split this shaped line at a byte index, returning `(prefix, suffix)`.
    ///
    /// - Each half retains glyphs whose source starts belong to its byte range.
    ///   Both halves preserve visual order and ink offsets while packing their
    ///   retained advances; the last logical half retains any metric remainder.
    /// - Suffix byte indices are rebased to zero.
    /// - This preserves shaping, rather than reshaping either half. Callers
    ///   reusing the glyphs should choose shaped-cluster boundaries.
    /// - Decoration runs are partitioned at the boundary; a run that straddles it is
    ///   split into two with adjusted lengths.
    /// - `font_size`, `ascent`, and `descent` are copied to both halves.
    pub fn split_at(&self, byte_index: usize) -> (ShapedLine, ShapedLine) {
        let (left_layout, right_layout) = self.layout.split_at(byte_index);

        // Partition decoration runs. A run straddling the boundary is split into two.
        let mut left_decorations = SmallVec::new();
        let mut right_decorations = SmallVec::new();
        let mut decoration_offset = 0u32;
        let split_point = byte_index as u32;

        for decoration in &self.decoration_runs {
            let run_end = decoration_offset + decoration.len;

            if run_end <= split_point {
                left_decorations.push(decoration.clone());
            } else if decoration_offset >= split_point {
                right_decorations.push(decoration.clone());
            } else {
                let left_len = split_point - decoration_offset;
                let right_len = run_end - split_point;
                left_decorations.push(DecorationRun {
                    len: left_len,
                    color: decoration.color,
                    background_color: decoration.background_color,
                    underline: decoration.underline,
                    overline: decoration.overline,
                    strikethrough: decoration.strikethrough,
                    decoration_layers: decoration.decoration_layers.clone(),
                });
                right_decorations.push(DecorationRun {
                    len: right_len,
                    color: decoration.color,
                    background_color: decoration.background_color,
                    underline: decoration.underline,
                    overline: decoration.overline,
                    strikethrough: decoration.strikethrough,
                    decoration_layers: decoration.decoration_layers.clone(),
                });
            }

            decoration_offset = run_end;
        }

        // Split text
        let left_text = if byte_index == self.text.len() {
            self.text.clone()
        } else {
            self.text.slice(0..byte_index)
        };
        let right_text = if byte_index == 0 {
            self.text.clone()
        } else {
            self.text.slice(byte_index..self.text.len())
        };

        let left = ShapedLine {
            layout: Arc::new(left_layout),
            text: left_text,
            decoration_runs: left_decorations,
        };

        let right = ShapedLine {
            layout: Arc::new(right_layout),
            text: right_text,
            decoration_runs: right_decorations,
        };

        (left, right)
    }
}

struct GlyphCursorEntry {
    run_index: usize,
    glyph_index: usize,
    pen: Pixels,
    grapheme: usize,
}

enum GlyphCursorOrder {
    ByteOrdered,
    Deferred,
    Indexed(Vec<GlyphCursorEntry>),
}

/// Incrementally splits a [`ShapedLine`] at increasing UTF-8 byte boundaries.
///
/// Each piece preserves the original glyphs and decorations, rebased as in
/// [`ShapedLine::split_at`]. Byte-ordered glyphs are advanced in linear time,
/// copying each glyph and byte at most once. Visually reordered glyphs use one
/// source index; each consumed piece is restored to visual order without
/// repeatedly cloning and splitting the remaining line.
pub struct ShapedLineCursor<'a> {
    line: &'a ShapedLine,
    /// Bidirectional shaping can put glyphs out of byte order.
    glyph_order: GlyphCursorOrder,
    bidi: Option<Box<CursorBidi<'a>>>,
    paragraph_aware: bool,
    byte_index: usize,
    run_index: usize,
    glyph_index: usize,
    decoration_index: usize,
    decoration_offset: u32,
    advance_offset: Pixels,
}

impl<'a> ShapedLineCursor<'a> {
    /// Cumulative advance consumed in source traversal order. This is not a
    /// physical caret coordinate for bidi text. At validated cluster cuts,
    /// differences measure the same source slices produced by `take_until`.
    pub fn source_advance(&self) -> Pixels {
        self.advance_offset
    }

    /// Start another source-order traversal of this same paragraph without
    /// rebuilding its glyph index or Unicode paragraph analysis. This keeps
    /// a boundary preflight separate from producing owned soft-row layouts.
    pub fn rewind(&mut self) {
        self.byte_index = 0;
        self.run_index = 0;
        self.glyph_index = 0;
        self.decoration_index = 0;
        self.decoration_offset = 0;
        self.advance_offset = px(0.);
    }

    /// Borrow the paragraph's L1/L2 source runs for a nonempty row without
    /// advancing the glyph cursor. Source offsets remain paragraph-relative.
    /// Requires a cursor created by [`ShapedLine::cursor_with_direction`] from
    /// a complete hard paragraph. Embedding levels survive soft rows; querying
    /// the row just consumed reuses its prepared ordering scratch.
    pub fn visual_source_runs(&mut self, source: Range<usize>) -> super::VisualSourceRuns<'_> {
        assert!(self.paragraph_aware, "visual source runs require a complete-paragraph cursor");
        assert!(source.start < source.end && source.end <= self.line.len());
        assert!(self.line.text.is_char_boundary(source.start) && self.line.text.is_char_boundary(source.end));
        if let Some(bidi) = self.bidi.as_mut() {
            bidi.prepare_cursor(source);
            bidi.visual_source_runs()
        } else {
            super::VisualSourceRuns::ltr(source)
        }
    }

    /// Whether a source position at or after the cursor starts a native
    /// shaping cluster. UTF-8 boundaries inside a ligature are not sufficient.
    /// A paragraph-aware cursor also requires a grapheme boundary.
    pub fn is_cluster_boundary(&self, index: usize) -> bool {
        assert!(index >= self.byte_index && index <= self.line.len());
        if index == 0 || index == self.line.len() {
            return true;
        }
        if self.bidi.as_ref().is_some_and(|bidi| !bidi.is_grapheme_boundary(index)) {
            return false;
        }
        if let GlyphCursorOrder::Indexed(entries) = &self.glyph_order {
            return entries[self.glyph_index..].binary_search_by_key(&index, |entry| {
                self.line.layout.runs[entry.run_index].glyphs[entry.glyph_index].index
            }).is_ok();
        }
        if matches!(self.glyph_order, GlyphCursorOrder::Deferred) {
            // Before the first cut there is no consumed prefix. A boundary
            // query need not allocate the source index; take_until builds it
            // once when glyphs actually have to be sliced.
            return self.line.layout.runs.iter().flat_map(|run| &run.glyphs)
                .any(|glyph| glyph.index == index);
        }
        for (offset, run) in self.line.layout.runs[self.run_index..].iter().enumerate() {
            let first = if offset == 0 { self.glyph_index } else { 0 };
            let remaining = &run.glyphs[first..];
            let next = remaining.partition_point(|glyph| glyph.index < index);
            if let Some(glyph) = remaining.get(next) {
                return glyph.index == index;
            }
        }
        false
    }

    fn ensure_source_index(&mut self) {
        if matches!(self.glyph_order, GlyphCursorOrder::Deferred) {
            if let Some(bidi) = self.bidi.as_mut() { bidi.prepare_graphemes(); }
            let mut entries = Vec::new();
            let mut pen = Pixels::ZERO;
            for (run_index, run) in self.line.layout.runs.iter().enumerate() {
                for (glyph_index, glyph) in run.glyphs.iter().enumerate() {
                    entries.push(GlyphCursorEntry {
                        run_index, glyph_index, pen,
                        grapheme: self.bidi.as_ref().map_or(glyph.index, |bidi| bidi.grapheme(glyph.index)),
                    });
                    pen += glyph.advance;
                }
            }
            entries.sort_by_key(|entry| self.line.layout.runs[entry.run_index].glyphs[entry.glyph_index].index);
            self.glyph_order = GlyphCursorOrder::Indexed(entries);
        }
    }

    /// Advance with the same source ownership as `take_until`, without copying
    /// discarded glyphs, source text, caret arrays or decoration runs.
    pub fn skip_until(&mut self, byte_index: usize) {
        assert!(byte_index >= self.byte_index && byte_index <= self.line.len(), "invalid source skip");
        assert!(self.line.text.is_char_boundary(byte_index), "source skip must end at a UTF-8 boundary");
        self.ensure_source_index();
        if let GlyphCursorOrder::Indexed(entries) = &self.glyph_order {
            while let Some(entry) = entries.get(self.glyph_index) {
                let glyph = &self.line.layout.runs[entry.run_index].glyphs[entry.glyph_index];
                if glyph.index >= byte_index { break; }
                self.advance_offset += glyph.advance;
                self.glyph_index += 1;
            }
        } else {
            while let Some(run) = self.line.layout.runs.get(self.run_index) {
                while let Some(glyph) = run.glyphs.get(self.glyph_index) {
                    if glyph.index >= byte_index { break; }
                    self.advance_offset += glyph.advance;
                    self.glyph_index += 1;
                }
                if self.glyph_index < run.glyphs.len() { break; }
                self.run_index += 1;
                self.glyph_index = 0;
            }
        }
        if byte_index == self.line.len() { self.advance_offset = self.line.width; }
        while let Some(run) = self.line.decoration_runs.get(self.decoration_index) {
            let end = self.decoration_offset + run.len;
            if end > byte_index as u32 { break; }
            self.decoration_offset = end;
            self.decoration_index += 1;
        }
        self.byte_index = byte_index;
    }

    /// Takes the bytes since the previous boundary.
    ///
    /// Panics if the boundary precedes the previous one, exceeds the line's
    /// length, or falls inside a UTF-8 character.
    pub fn take_until(&mut self, byte_index: usize) -> ShapedLine {
        assert!(
            byte_index >= self.byte_index,
            "split boundary moved backwards"
        );
        assert!(
            byte_index <= self.line.len(),
            "split boundary exceeds line length"
        );
        assert!(
            self.line.text.is_char_boundary(byte_index),
            "split boundary is not a UTF-8 character boundary"
        );
        self.ensure_source_index();
        let previous_index = self.byte_index;
        let previous_x = self.advance_offset;
        let mut runs: Vec<crate::ShapedRun> = Vec::new();
        let mut interior_carets = Vec::new();
        let mut next_x = previous_x;
        if let GlyphCursorOrder::Indexed(entries) = &mut self.glyph_order {
            let start = self.glyph_index;
            while let Some(entry) = entries.get(self.glyph_index) {
                if self.line.layout.runs[entry.run_index].glyphs[entry.glyph_index].index >= byte_index { break; }
                self.glyph_index += 1;
            }
            // Only consumed entries are reordered; the remaining source index
            // stays sorted for the next call. No per-piece reference vector.
            let selected = &mut entries[start..self.glyph_index];
            if let Some(bidi) = &mut self.bidi {
                bidi.prepare_cursor(previous_index..byte_index);
                selected.sort_unstable_by(|left, right| {
                    let (left_run, left_rtl) = bidi.order(left.grapheme.max(previous_index));
                    let (right_run, _) = bidi.order(right.grapheme.max(previous_index));
                    left_run.cmp(&right_run).then_with(|| {
                        if left_rtl { right.grapheme.cmp(&left.grapheme) }
                        else { left.grapheme.cmp(&right.grapheme) }
                    }).then_with(|| left.run_index.cmp(&right.run_index))
                        .then_with(|| left.glyph_index.cmp(&right.glyph_index))
                });
            } else {
                // Native paint order is the run/glyph order, even when a
                // combining glyph has a zero or negative advance.
                selected.sort_unstable_by_key(|entry| (entry.run_index, entry.glyph_index));
            }
            let mut pen = Pixels::ZERO;
            let mut previous_cluster = None;
            let mut previous_grapheme = None;
            let mut group_delta = Pixels::ZERO;
            for entry in selected {
                let run = &self.line.layout.runs[entry.run_index];
                let glyph = &run.glyphs[entry.glyph_index];
                let delta = if self.bidi.is_some() {
                    if previous_grapheme != Some(entry.grapheme) {
                        previous_grapheme = Some(entry.grapheme);
                        group_delta = pen - entry.pen;
                    }
                    group_delta
                } else { pen - entry.pen };
                if previous_cluster != Some(glyph.index) {
                    self.line.layout.append_cluster_carets(
                        glyph.index,
                        previous_index..byte_index,
                        delta,
                        &mut interior_carets,
                    );
                    previous_cluster = Some(glyph.index);
                }
                let mut retained = glyph.clone();
                retained.position.x += delta;
                if let Some(bidi) = &self.bidi {
                    retained.is_rtl = bidi.order(entry.grapheme.max(previous_index)).1;
                }
                retained.index -= previous_index;
                pen += glyph.advance;
                if let Some(last) = runs.last_mut().filter(|last| last.font_id == run.font_id && last.font_size == run.font_size) {
                    last.glyphs.push(retained);
                } else {
                    runs.push(crate::ShapedRun { font_size: run.font_size, font_id: run.font_id, glyphs: vec![retained] });
                }
            }
            interior_carets.sort_unstable_by_key(|caret| caret.index);
            next_x += pen;
        } else {
            let carets = &self.line.layout.interior_carets;
            let start = carets.partition_point(|caret| caret.index < previous_index);
            let end = carets.partition_point(|caret| caret.index < byte_index);
            interior_carets.extend(carets[start..end].iter()
                .filter(|caret| caret.cluster_start >= previous_index)
                .map(|caret| crate::ShapedCaret {
                    index: caret.index - previous_index,
                    cluster_start: caret.cluster_start - previous_index,
                    x: caret.x - previous_x,
                }));
            while let Some(run) = self.line.layout.runs.get(self.run_index) {
                let start = self.glyph_index;
                while let Some(glyph) = run.glyphs.get(self.glyph_index) {
                    if glyph.index >= byte_index {
                        break;
                    }
                    self.glyph_index += 1;
                }
                let end = self.glyph_index;
                if start < end {
                    next_x += run.glyphs[start..end]
                        .iter()
                        .map(|glyph| glyph.advance)
                        .sum::<Pixels>();
                    runs.push(crate::ShapedRun { font_size: run.font_size,
                        font_id: run.font_id,
                        glyphs: run.glyphs[start..end]
                            .iter()
                            .map(|glyph| crate::ShapedGlyph {
                                id: glyph.id,
                                position: point(glyph.position.x - previous_x, glyph.position.y),
                                index: glyph.index - previous_index,
                                is_emoji: glyph.is_emoji,
                                is_rtl: glyph.is_rtl,
                                advance: glyph.advance,
                            })
                            .collect(),
                    });
                }
                if run.glyphs.get(self.glyph_index).is_some() {
                    break;
                }
                self.run_index += 1;
                self.glyph_index = 0;
            }
        }
        if byte_index == self.line.len() { next_x = self.line.layout.width; }
        let mut decorations = SmallVec::new();
        while let Some(decoration) = self.line.decoration_runs.get(self.decoration_index)
            && (self.decoration_offset < byte_index as u32
                || (decoration.len == 0 && self.decoration_offset == byte_index as u32))
        {
            let end = self.decoration_offset + decoration.len;
            let start = self.decoration_offset.max(previous_index as u32);
            let len = end.min(byte_index as u32) - start;
            if len > 0 || decoration.len == 0 {
                decorations.push(DecorationRun {
                    len,
                    color: decoration.color,
                    background_color: decoration.background_color,
                    underline: decoration.underline,
                    overline: decoration.overline,
                    strikethrough: decoration.strikethrough,
                    decoration_layers: decoration.decoration_layers.clone(),
                });
            }
            if end <= byte_index as u32 {
                self.decoration_index += 1;
                self.decoration_offset = end;
            } else {
                break;
            }
        }
        self.byte_index = byte_index;
        self.advance_offset = next_x;
        ShapedLine {
            layout: Arc::new(LineLayout {
                font_size: self.line.layout.font_size,
                width: next_x - previous_x,
                ascent: self.line.layout.ascent,
                descent: self.line.layout.descent,
                runs,
                len: byte_index - previous_index,
                interior_carets,
                isolation: self.line.layout.isolation.as_ref().map(|context| context.subrange(previous_index..byte_index)),
                ink_clip: (previous_index == 0 && byte_index == self.line.len())
                    .then_some(self.line.layout.ink_clip).flatten(),
            }),
            text: self.line.text.slice(previous_index..byte_index),
            decoration_runs: decorations,
        }
    }

    /// Returns the original line's caret x at the current source boundary.
    /// Bidirectional transitions use [`LineLayout::x_for_index`]'s affinity.
    pub fn x_offset(&self) -> Pixels {
        self.line.layout.x_for_index(self.byte_index)
    }
}

impl LineLayout {
    /// Paint this layout to the window, using the given decoration runs to color
    /// glyphs and draw underlines and strikethroughs.
    ///
    /// This is a lower-level alternative to [`ShapedLine::paint`] for callers that
    /// hold a bare layout and track decorations themselves.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        decoration_runs: &[DecorationRun],
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line(
            origin,
            self,
            line_height,
            align,
            align_width,
            decoration_runs,
            &[],
            window,
            cx,
            UnderlinePainter::Native(None),
        )
    }

    /// Paint the background of this layout to the window, using the given
    /// decoration runs to determine background colors.
    ///
    /// This is a lower-level alternative to [`ShapedLine::paint_background`] for
    /// callers that hold a bare layout and track decorations themselves.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        decoration_runs: &[DecorationRun],
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        paint_line_background(
            origin,
            self,
            line_height,
            align,
            align_width,
            decoration_runs,
            &[],
            window,
            cx,
        )
    }
}

/// A line of text that has been shaped, decorated, and wrapped by the text layout system.
#[derive(Default, Debug, Deref, DerefMut)]
pub struct WrappedLine {
    #[deref]
    #[deref_mut]
    pub(crate) layout: Arc<WrappedLineLayout>,
    /// The text that was shaped for this line.
    pub text: super::ShapedText,
    pub(crate) decoration_runs: Vec<DecorationRun>,
}

impl WrappedLine {
    /// Decorate one already-shaped, unwrapped source line without reshaping it.
    pub(crate) fn from_layout(text: super::ShapedText, layout: Arc<LineLayout>, runs: &[crate::TextRun]) -> Self {
        assert_eq!(text.len(), layout.len);
        assert!(!text.contains('\n'));
        let mut decoration_runs: Vec<DecorationRun> = Vec::new();
        let mut covered = 0;
        for run in runs.iter().filter(|run| run.len > 0) {
            covered += run.len;
            assert!(covered <= text.len() && text.is_char_boundary(covered));
            let len = u32::try_from(run.len).expect("text run exceeds supported length");
            if let Some(last) = decoration_runs.last_mut()
                && last.color == run.color && last.background_color == run.background_color
                && last.underline == run.underline && last.overline == run.overline
                && last.strikethrough == run.strikethrough && last.decoration_layers == run.decoration_layers
            {
                last.len = last.len.checked_add(len).expect("text line exceeds supported length");
            } else {
                decoration_runs.push(DecorationRun { len, color: run.color, background_color: run.background_color,
                    underline: run.underline, overline: run.overline, strikethrough: run.strikethrough, decoration_layers: run.decoration_layers.clone() });
            }
        }
        assert_eq!(covered, text.len(), "text runs must cover prepared glyphs");
        Self { layout: Arc::new(WrappedLineLayout { unwrapped_layout: layout,
            wrap_boundaries: SmallVec::new(), wrap_width: None }), text, decoration_runs }
    }

    /// The length of the underlying, unwrapped layout, in utf-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.len()
    }

    /// Paint this line of text to the window.
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let align_width = match bounds {
            Some(bounds) => Some(bounds.size.width),
            None => self.layout.wrap_width,
        };

        paint_line(
            origin,
            &self.layout.unwrapped_layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &self.wrap_boundaries,
            window,
            cx,
            UnderlinePainter::Native(Some(&self.text)),
        )?;

        Ok(())
    }

    /// Paint the background of line of text to the window.
    pub fn paint_background(
        &self,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()> {
        let align_width = match bounds {
            Some(bounds) => Some(bounds.size.width),
            None => self.layout.wrap_width,
        };

        paint_line_background(
            origin,
            &self.layout.unwrapped_layout,
            line_height,
            align,
            align_width,
            &self.decoration_runs,
            &self.wrap_boundaries,
            window,
            cx,
        )?;

        Ok(())
    }
}

/// Follows source styles while glyphs arrive in visual order. The common
/// forward path needs no index; only a backward style transition builds one.
struct DecorationRunCursor<'a> {
    runs: &'a [DecorationRun],
    index: usize,
    start: usize,
    end: usize,
    ends: SmallVec<[usize; 8]>,
}

impl<'a> DecorationRunCursor<'a> {
    fn new(runs: &'a [DecorationRun]) -> Self {
        Self { runs, index: 0, start: 0, end: 0, ends: SmallVec::new() }
    }

    fn contains(&self, byte_index: usize) -> bool {
        self.start <= byte_index && byte_index < self.end
    }

    /// Returns whether the active style changed, including leaving all styles.
    fn advance_to(&mut self, byte_index: usize) -> bool {
        if self.contains(byte_index) { return false; }
        if byte_index < self.start || !self.ends.is_empty() {
            if self.ends.is_empty() {
                let mut end = 0;
                self.ends.extend(self.runs.iter().map(|run| { end += run.len as usize; end }));
            }
            self.index = self.ends.partition_point(|end| *end <= byte_index);
            self.start = if self.index == 0 { 0 } else { self.ends[self.index - 1] };
            self.end = self.ends.get(self.index).copied().unwrap_or(self.start);
            return true;
        }
        if self.index == self.runs.len() { return false; }
        let mut index = self.index + usize::from(self.end > self.start);
        let mut start = self.end;
        while let Some(run) = self.runs.get(index) {
            let end = start + run.len as usize;
            if byte_index < end {
                self.index = index; self.start = start; self.end = end;
                return true;
            }
            start = end;
            index += 1;
        }
        self.index = index; self.start = start; self.end = start;
        true
    }

    fn current(&self) -> Option<(usize, &'a DecorationRun)> {
        self.runs.get(self.index).map(|run| (self.start, run))
    }
}

// The public custom handler continues to own its drawing policy. Native text
// can inspect its borrowed source without attaching text to bare layouts or
// adding per-glyph copies of Unicode/script metadata to shaping output.
enum UnderlinePainter<'a> {
    Native(Option<&'a str>),
    Custom(&'a mut dyn FnMut(Range<usize>, Point<Pixels>, Pixels, &UnderlineStyle, &mut Window), Option<&'a str>),
}

impl UnderlinePainter<'_> {
    fn skips_ink_for(&self, index: usize, overline: bool) -> bool {
        let text = match self {
            Self::Native(text) => *text,
            Self::Custom(_, text) if overline => *text,
            _ => None,
        };
        let Some(text) = text else { return false; };
        let Some(ch) = text.get(index..).and_then(|text| text.chars().next()) else { return false; };
        if ch.is_ascii() { return true; }
        // CSS auto leaves CJK letters and their shared punctuation connected.
        // Script_Extensions covers supplementary ideographs and kana marks as
        // well as the basic blocks, without maintaining a second range table.
        !ch.script_extension().iter().any(|script| matches!(script,
            Script::Han | Script::Hangul | Script::Hiragana | Script::Katakana
                | Script::Bopomofo | Script::Yi | Script::Tangut | Script::Nushu
                | Script::Khitan_Small_Script))
    }

    fn paint(
        &mut self, range: Range<usize>, origin: Point<Pixels>, width: Pixels,
        style: &UnderlineStyle, ink: &mut UnderlineInk, window: &mut Window,
    ) {
        match self {
            Self::Native(_) => window.paint_underline_with_ink_exclusions(
                origin, width, style, [&ink.bands[0], &ink.bands[1]],
            ),
            Self::Custom(handler, _) => handler(range, origin, width, style, window),
        }
        for band in &mut ink.bands { band.clear(); }
    }

    fn paint_overline(
        &mut self, origin: Point<Pixels>, width: Pixels,
        style: &UnderlineStyle, ink: &mut UnderlineInk, window: &mut Window,
    ) {
        // The public callback owns underlines only; overlines keep native
        // painting and the borrowed source even when that callback is custom.
        window.paint_overline_with_ink_exclusions(
            origin, width, style, [&ink.bands[0], &ink.bands[1]],
        );
        for band in &mut ink.bands { band.clear(); }
    }
}

#[derive(Default)]
struct UnderlineInk {
    bands: [SmallVec<[Range<ScaledPixels>; 4]>; 2],
}

// The primitive remains a copyable style. Active segments own only transient
// ink intersections, so changing a line never creates another shaped layout.
#[derive(Clone, Copy, PartialEq)]
enum DecorationStroke {
    Underline(UnderlineStyle),
    Overline(UnderlineStyle),
    Strikethrough(StrikethroughStyle),
}

impl DecorationStroke {
    fn for_run(run: &DecorationRun) -> RunDecorations<'_> { RunDecorations::new(run) }

    fn offset(self, baseline: Pixels, defaults: [Pixels; 3]) -> Pixels {
        let (offset, fallback) = match self {
            Self::Underline(style) => (style.baseline_offset, defaults[0]),
            Self::Overline(style) => (style.baseline_offset, defaults[1]),
            Self::Strikethrough(style) => (style.baseline_offset, defaults[2]),
        };
        offset.map_or(fallback, |offset| baseline + offset)
    }

    fn bounds(self, origin: Point<Pixels>, width: Pixels, window: &Window) -> Bounds<ScaledPixels> {
        match self {
            Self::Underline(style) => window.underline_bounds(origin, width, &style),
            Self::Overline(style) => window.overline_bounds(origin, width, &style),
            Self::Strikethrough(style) => window.strikethrough_bounds(origin, width, &style),
        }
    }

    fn edge(self) -> Option<(UnderlineStyle, bool)> {
        match self {
            Self::Underline(style) => Some((style, false)),
            Self::Overline(style) => Some((style, true)),
            Self::Strikethrough(_) => None,
        }
    }
}


#[derive(Clone, Copy, PartialEq, Eq)]
struct DecorationSlot {
    box_identity: usize,
    kind: usize,
    depth: usize,
}

impl DecorationSlot {
    fn order(self) -> (usize, usize) { (self.kind, self.depth) }
}

// The chain is retained; only this short-lived path is reversed for CSS's
// ancestor-first paint order. Legacy controls use no path or heap allocation.
struct RunDecorations<'a> {
    run: &'a DecorationRun,
    boxes: SmallVec<[&'a Arc<DecorationLayer>; 4]>,
    kind: usize,
    depth: usize,
}

impl<'a> RunDecorations<'a> {
    fn new(run: &'a DecorationRun) -> Self {
        let mut boxes = SmallVec::new();
        let mut next = run.decoration_layers.as_ref().map(|layers| &layers.head);
        while let Some(layer) = next { boxes.push(layer); next = layer.parent.as_ref(); }
        boxes.reverse();
        Self { run, boxes, kind: 0, depth: 0 }
    }
}

impl Iterator for RunDecorations<'_> {
    type Item = (DecorationSlot, DecorationStroke);

    fn next(&mut self) -> Option<Self::Item> {
        while self.kind < 3 {
            let depth = self.depth;
            let kind = self.kind;
            self.depth += 1;
            if self.depth == self.boxes.len().max(1) { self.depth = 0; self.kind += 1; }
            let (underline, overline, strike, identity, shift) = if let Some(layer) = self.boxes.get(depth) {
                (layer.underline, layer.overline, layer.strikethrough, layer.identity.address(),
                    self.run.decoration_layers.as_ref().map_or(Pixels::ZERO, |layers| layers.baseline_shift))
            } else {
                (self.run.underline, self.run.overline, self.run.strikethrough, 0, Pixels::ZERO)
            };
            let stroke = match kind {
                0 | 1 => (if kind == 0 { underline } else { overline }).map(|mut style| {
                    style.color = Some(style.color.unwrap_or(self.run.color));
                    style.baseline_offset = style.baseline_offset.map(|offset| offset + shift);
                    if kind == 0 { DecorationStroke::Underline(style) } else { DecorationStroke::Overline(style) }
                }),
                _ => strike.map(|mut style| {
                    style.color = Some(style.color.unwrap_or(self.run.color));
                    style.baseline_offset = style.baseline_offset.map(|offset| offset + shift);
                    DecorationStroke::Strikethrough(style)
                }),
            };
            if let Some(stroke) = stroke { return Some((DecorationSlot { box_identity: identity, kind, depth }, stroke)); }
        }
        None
    }
}

struct DecorationSegment {
    slot: DecorationSlot,
    stroke: DecorationStroke,
    origin: Point<Pixels>,
    range: Range<usize>,
    ink: UnderlineInk,
}

struct DecorationPaint {
    segment: DecorationSegment,
    end: Pixels,
    zero_width_fallback: Pixels,
}

impl DecorationPaint {
    fn paint(mut self, painter: &mut UnderlinePainter<'_>, window: &mut Window) {
        let (origin, width) = self.segment.geometry(self.end, self.zero_width_fallback);
        let scale = window.scale_factor();
        let bounds = self.segment.stroke.bounds(origin, width, window)
            .map(|value| px(value.as_f32() / scale));
        // Separate orders preserve ancestor compositing even when one stroke
        // uses quads and another uses the patterned-underline primitive.
        window.paint_layer(bounds, |window| {
            self.segment.paint_now(self.end, self.zero_width_fallback, painter, window);
        });
    }
}

impl DecorationSegment {
    fn continues(&self, stroke: DecorationStroke, range: &Range<usize>) -> bool {
        self.stroke == stroke && (matches!(stroke, DecorationStroke::Strikethrough(_))
            || (range.start <= self.range.end && self.range.start <= range.end))
    }

    fn geometry(&self, end: Pixels, zero_width_fallback: Pixels) -> (Point<Pixels>, Pixels) {
        let mut origin = self.origin;
        if end == origin.x { origin.x -= zero_width_fallback.half(); }
        (origin, end - origin.x)
    }

    fn paint(&mut self, end: Pixels, zero_width_fallback: Pixels,
        painter: &mut UnderlinePainter<'_>, window: &mut Window,
        deferred: &mut Option<Vec<DecorationPaint>>)
    {
        if let Some(deferred) = deferred {
            deferred.push(DecorationPaint {
                segment: Self { slot: self.slot, stroke: self.stroke, origin: self.origin,
                    range: self.range.clone(), ink: std::mem::take(&mut self.ink) },
                end, zero_width_fallback,
            });
        } else {
            self.paint_now(end, zero_width_fallback, painter, window);
        }
    }

    fn paint_now(&mut self, end: Pixels, zero_width_fallback: Pixels,
        painter: &mut UnderlinePainter<'_>, window: &mut Window)
    {
        let (origin, width) = self.geometry(end, zero_width_fallback);
        match self.stroke {
            DecorationStroke::Underline(style) => painter.paint(self.range.clone(), origin, width, &style, &mut self.ink, window),
            DecorationStroke::Overline(style) => painter.paint_overline(origin, width, &style, &mut self.ink, window),
            DecorationStroke::Strikethrough(style) => window.paint_strikethrough(origin, width, &style),
        }
    }
}

fn paint_line(
    origin: Point<Pixels>, layout: &LineLayout, line_height: Pixels, align: TextAlign,
    align_width: Option<Pixels>, decoration_runs: &[DecorationRun], wrap_boundaries: &[WrapBoundary],
    window: &mut Window, cx: &mut App, paint_underline: UnderlinePainter<'_>,
) -> Result<()> {
    let mask = layout.ink_clip.map(|(left, right)| {
        debug_assert!(wrap_boundaries.is_empty(), "a paint-only cluster piece must not wrap");
        let x = aligned_origin_x(origin, align_width.unwrap_or(layout.width), &(Pixels::ZERO..layout.width), &align);
        let mut bounds = window.content_mask().bounds;
        let start = if left { bounds.left().max(x) } else { bounds.left() };
        let end = if right { bounds.right().min(x + layout.width) } else { bounds.right() };
        bounds.origin.x = start;
        bounds.size.width = (end - start).max(Pixels::ZERO);
        crate::ContentMask { bounds }
    });
    window.with_content_mask(mask, |window| paint_line_inner(
        origin, layout, line_height, align, align_width, decoration_runs, wrap_boundaries,
        window, cx, paint_underline,
    ))
}

fn paint_line_inner(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    decoration_runs: &[DecorationRun],
    wrap_boundaries: &[WrapBoundary],
    window: &mut Window,
    cx: &mut App,
    mut paint_underline: UnderlinePainter<'_>,
) -> Result<()> {
    let mut line_bounds = line_paint_bounds(
        origin, layout, line_height, align, align_width, wrap_boundaries,
    );
    let padding_top = (line_height - layout.ascent - layout.descent) / 2.;
    let baseline_y = padding_top + layout.ascent;
    let defaults = [
        underline_y_offset(line_height, layout.ascent, layout.descent),
        padding_top,
        ((layout.ascent * 0.5) + baseline_y) * 0.5,
    ];
    let scale = window.scale_factor();
    let ordered_layers = decoration_runs.iter().any(|run| run.decoration_layers.is_some());
    let last_row = line_height * wrap_boundaries.len();
    for (_, stroke) in decoration_runs.iter().flat_map(DecorationStroke::for_run) {
        let y = stroke.offset(baseline_y, defaults);
        let bounds = stroke.bounds(origin + point(px(0.), y), px(1.), window);
        let top = px(bounds.top().as_f32() / scale).min(line_bounds.top());
        let bottom = (px(bounds.bottom().as_f32() / scale) + last_row).max(line_bounds.bottom());
        line_bounds.origin.y = top;
        line_bounds.size.height = bottom - top;
    }
    window.paint_layer(line_bounds, |window| {
        let baseline_offset = point(px(0.), baseline_y);
        let mut decoration_runs = DecorationRunCursor::new(decoration_runs);
        let mut wraps = wrap_boundaries.iter().peekable();
        let mut row_ranges = layout.visual_row_ranges(wrap_boundaries);
        let mut row = row_ranges.next().expect("every shaped line has a physical row");
        let mut color = black();
        // Plain text has no active segments. The ordinary three slots stay
        // inline; ink buffers live with their segment, not in another cache.
        let mut active = SmallVec::<[DecorationSegment; 3]>::new();
        let mut spare = SmallVec::<[DecorationSegment; 3]>::new();
        let mut incoming = SmallVec::<[(DecorationSlot, DecorationStroke); 3]>::new();
        // Retain only this paint call's sprites and completed stroke commands.
        // A parent finishes after its children; drawing on completion would
        // reverse their colors and restart patterns if split into prefixes.
        let mut deferred = ordered_layers.then(Vec::new);
        let mut glyphs = Vec::<PreparedGlyph>::new();
        let text_system = cx.text_system().clone();
        let mut row_left = aligned_origin_x(origin, align_width.unwrap_or(layout.width), &row, &align);
        let mut glyph_origin = point(row_left, origin.y);
        let mut pen = Pixels::ZERO;
        let mut prev_glyph_position = Point::default();
        let mut max_glyph_size = size(px(0.), px(0.));
        for (run_ix, run) in layout.runs.iter().enumerate() {
            let run_size = run.font_size.unwrap_or(layout.font_size);
            // Retained pieces own their exact native cell, including zero
            // advance; do not expand their decoration/background into a
            // synthetic font-sized cell. Raster preparation has real bounds.
            max_glyph_size = if layout.ink_clip.is_some() { size(Pixels::ZERO, Pixels::ZERO) }
                else { text_system.bounding_box(run.font_id, run_size).size };
            for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                glyph_origin.x += glyph.position.x - prev_glyph_position.x;
                if wraps.peek() == Some(&&WrapBoundary { run_ix, glyph_ix }) {
                    wraps.next();
                    let row_end_x = aligned_origin_x(origin, align_width.unwrap_or(layout.width), &row, &align)
                        + row.end - row.start;
                    row = row_ranges.next().expect("each wrap boundary starts a physical row");
                    let next_row_left = aligned_origin_x(origin, align_width.unwrap_or(layout.width), &row, &align);
                    for segment in &mut active {
                        segment.paint(row_end_x, max_glyph_size.width, &mut paint_underline, window, &mut deferred);
                        segment.origin.x = next_row_left;
                        segment.origin.y += line_height;
                    }
                    if !decoration_runs.contains(glyph.index) { active.clear(); }
                    row_left = next_row_left;
                    glyph_origin.x = next_row_left + glyph.position.x - row.start;
                    glyph_origin.y += line_height;
                }
                prev_glyph_position = glyph.position;
                // Decorations own advance cells, including justification;
                // glyph ink offsets remain reserved for raster/contour work.
                let cell_x = row_left + pen - row.start;
                pen += glyph.advance;

                if decoration_runs.advance_to(glyph.index) {
                    incoming.clear();
                    let range = if let Some((start, style_run)) = decoration_runs.current() {
                        color = style_run.color;
                        incoming.extend(DecorationStroke::for_run(style_run));
                        start..start + style_run.len as usize
                    } else { 0..0 };
                    // Both lists are ordered by kind, then ancestor depth.
                    // A linear merge preserves shared ancestor segments and
                    // flushes replaced boxes in the previous paint order.
                    let mut next_ix = 0;
                    for segment in &mut active {
                        while incoming.get(next_ix).is_some_and(|(slot, _)| slot.order() < segment.slot.order()) { next_ix += 1; }
                        let continues = incoming.get(next_ix).is_some_and(|(slot, stroke)|
                            *slot == segment.slot && segment.continues(*stroke, &range));
                        if !continues { segment.paint(cell_x, max_glyph_size.width, &mut paint_underline, window, &mut deferred); }
                    }
                    let mut previous = active.drain(..).peekable();
                    for &(slot, stroke) in &incoming {
                        while previous.peek().is_some_and(|segment| segment.slot.order() < slot.order()) { previous.next(); }
                        let stroke_origin = point(cell_x, glyph_origin.y + stroke.offset(baseline_y, defaults));
                        if previous.peek().is_some_and(|segment| segment.slot.order() == slot.order()) {
                            let mut segment = previous.next().expect("matched decoration slot");
                            if segment.slot == slot && segment.continues(stroke, &range) {
                                segment.range.start = segment.range.start.min(range.start);
                                segment.range.end = segment.range.end.max(range.end);
                            } else {
                                segment.slot = slot;
                                segment.stroke = stroke;
                                segment.origin = stroke_origin;
                                segment.range = range.clone();
                            }
                            spare.push(segment);
                        } else {
                            spare.push(DecorationSegment { slot, stroke, origin: stroke_origin,
                                range: range.clone(), ink: UnderlineInk::default() });
                        }
                    }
                    drop(previous);
                    std::mem::swap(&mut active, &mut spare);
                    spare.clear();
                }

                let max_glyph_bounds = Bounds { origin: glyph_origin, size: max_glyph_size };
                // A partial cluster may own only an overhanging ink sliver;
                // its narrowed advance cell is not a safe raster cull bound.
                if layout.ink_clip.is_some() || max_glyph_bounds.intersects(&window.content_mask().bounds) {
                    let vertical_offset = point(px(0.), glyph.position.y);
                    let baseline = glyph_origin + baseline_offset + vertical_offset;
                    if glyph.is_emoji {
                        if ordered_layers {
                            if let Some(primitive) = window.prepare_emoji(baseline, run.font_id, glyph.id, run_size)? {
                                glyphs.push(primitive);
                            }
                        } else { window.paint_emoji(baseline, run.font_id, glyph.id, run_size)?; }
                    } else {
                        let raster_bounds = if ordered_layers {
                            let primitive = window.prepare_glyph(baseline, run.font_id, glyph.id, run_size, color)?;
                            let bounds = primitive.as_ref().map(PreparedGlyph::bounds);
                            if let Some(primitive) = primitive { glyphs.push(primitive); }
                            bounds
                        } else {
                            window.paint_glyph_with_raster_bounds(baseline, run.font_id, glyph.id, run_size, color)?
                        };
                        let custom_underline = matches!(&paint_underline, UnderlinePainter::Custom(..));
                        if let Some(raster_bounds) = raster_bounds
                            && active.iter().any(|segment| segment.stroke.edge()
                                .is_some_and(|(style, overline)| style.skip_ink && style.thickness > Pixels::ZERO && (!custom_underline || overline)))
                            && paint_underline.skips_ink_for(glyph.index, true)
                        {
                            let mut bands = SmallVec::<[Range<Pixels>; 4]>::new();
                            let mut targets = SmallVec::<[(usize, usize, usize, Pixels); 4]>::new();
                            for (segment_ix, segment) in active.iter().enumerate() {
                                let Some((style, overline)) = segment.stroke.edge() else { continue; };
                                if !style.skip_ink || style.thickness <= Pixels::ZERO || (!overline && custom_underline) { continue; }
                                let bounds = segment.stroke.bounds(segment.origin, px(1.), window);
                                let double = style.effective_pattern() == TextDecorationPattern::Double;
                                let thickness = bounds.size.height / style.effective_pattern().height_factor();
                                let dilation = style.thickness.min(px(13.));
                                for index in 0..if double { 2 } else { 1 } {
                                    let top = px((bounds.top() + thickness * (index as f32 * 2.0)).as_f32() / scale) - baseline.y;
                                    let height = px((if double { thickness } else { bounds.size.height }).as_f32() / scale);
                                    let inset = px(0.5).min(height.half());
                                    let band = top + inset..top + height - inset;
                                    if raster_bounds.top() <= (baseline.y + band.end).scale(scale)
                                        && raster_bounds.bottom() >= (baseline.y + band.start).scale(scale)
                                    {
                                        // Coincident ancestors still paint independently,
                                        // but their common outline band is queried once.
                                        let existing = bands.iter().position(|existing| existing == &band);
                                        let band_ix = if let Some(index) = existing { index } else {
                                            let index = bands.len(); bands.push(band); index
                                        };
                                        targets.push((band_ix, segment_ix, index, dilation));
                                    }
                                }
                            }
                            if !bands.is_empty() {
                                let mut query = GlyphInk::new(&bands)?;
                                text_system.platform_text_system.intersect_glyph_ink(run.font_id, glyph.id, run_size, &mut query)?;
                                let intersections = query.intersections()?;
                                for (band_ix, segment_ix, index, dilation) in targets {
                                    if let Some(span) = &intersections[band_ix] {
                                        active[segment_ix].ink.bands[index].push(
                                            (baseline.x + span.start - dilation).scale(scale)
                                                ..(baseline.x + span.end + dilation).scale(scale),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let end_x = aligned_origin_x(origin, align_width.unwrap_or(layout.width), &row, &align) + row.end - row.start;
        for segment in &mut active { segment.paint(end_x, max_glyph_size.width, &mut paint_underline, window, &mut deferred); }
        if let Some(mut deferred) = deferred {
            deferred.sort_by_key(|paint| paint.segment.slot.order());
            let mut deferred = deferred.into_iter().peekable();
            while deferred.peek().is_some_and(|paint| paint.segment.slot.kind < 2) {
                deferred.next().expect("pending edge decoration").paint(&mut paint_underline, window);
            }
            window.paint_layer(line_bounds, |window| {
                for primitive in glyphs { primitive.paint(window); }
            });
            for paint in deferred { paint.paint(&mut paint_underline, window); }
        }
        Ok(())
    })
}

fn paint_line_background(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    decoration_runs: &[DecorationRun],
    wrap_boundaries: &[WrapBoundary],
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    // Ordinary glyph-only text has no background work or font-bound queries.
    if !decoration_runs.iter().any(|run| run.background_color.is_some()) { return Ok(()); }
    let line_bounds = line_paint_bounds(
        origin,
        layout,
        line_height,
        align,
        align_width,
        wrap_boundaries,
    );
    window.paint_layer(line_bounds, |window| {
        let mut decoration_runs = DecorationRunCursor::new(decoration_runs);
        let mut wraps = wrap_boundaries.iter().peekable();
        let mut row_ranges = layout.visual_row_ranges(wrap_boundaries);
        let mut row = row_ranges.next().expect("every shaped line has a physical row");
        let mut current_background: Option<(Point<Pixels>, Hsla)> = None;
        let text_system = cx.text_system().clone();
        let mut row_left = aligned_origin_x(origin, align_width.unwrap_or(layout.width), &row, &align);
        let mut row_y = origin.y;
        let mut pen = Pixels::ZERO;
        let mut max_glyph_size = size(px(0.), px(0.));
        for (run_ix, run) in layout.runs.iter().enumerate() {
            let run_size = run.font_size.unwrap_or(layout.font_size);
            // Retained pieces own their exact native cell, including zero
            // advance; do not expand their decoration/background into a
            // synthetic font-sized cell. Raster preparation has real bounds.
            max_glyph_size = if layout.ink_clip.is_some() { size(Pixels::ZERO, Pixels::ZERO) }
                else { text_system.bounding_box(run.font_id, run_size).size };

            for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                if wraps.peek() == Some(&&WrapBoundary { run_ix, glyph_ix }) {
                    wraps.next();
                    let row_end_x = aligned_origin_x(
                        origin, align_width.unwrap_or(layout.width), &row, &align,
                    ) + row.end - row.start;
                    row = row_ranges.next().expect("each wrap boundary starts a physical row");
                    let next_row_left = aligned_origin_x(
                        origin, align_width.unwrap_or(layout.width), &row, &align,
                    );
                    if let Some((background_origin, background_color)) = current_background.as_mut()
                    {
                        if row_end_x == background_origin.x {
                            background_origin.x -= max_glyph_size.width.half()
                        }
                        window.paint_quad(fill(
                            Bounds {
                                origin: *background_origin,
                                size: size(row_end_x - background_origin.x, line_height),
                            },
                            *background_color,
                        ));
                        if decoration_runs.contains(glyph.index) {
                            background_origin.x = next_row_left;
                            background_origin.y += line_height;
                        } else {
                            current_background = None;
                        }
                    }

                    row_left = next_row_left;
                    row_y += line_height;
                }
                let cell_x = row_left + pen - row.start;
                pen += glyph.advance;

                let mut finished_background: Option<(Point<Pixels>, Hsla)> = None;
                if decoration_runs.advance_to(glyph.index) {
                    if let Some((_, style_run)) = decoration_runs.current() {
                        if let Some((_, background_color)) = &mut current_background
                            && style_run.background_color.as_ref() != Some(background_color)
                        {
                            finished_background = current_background.take();
                        }
                        if let Some(run_background) = style_run.background_color {
                            current_background.get_or_insert((
                                point(cell_x, row_y),
                                run_background,
                            ));
                        }
                    } else {
                        finished_background = current_background.take();
                    }
                }

                if let Some((mut background_origin, background_color)) = finished_background {
                    let width = cell_x - background_origin.x;
                    if background_origin.x == cell_x {
                        background_origin.x -= max_glyph_size.width.half();
                    };
                    window.paint_quad(fill(
                        Bounds {
                            origin: background_origin,
                            size: size(width, line_height),
                        },
                        background_color,
                    ));
                }
            }
        }

        let last_line_end_x = aligned_origin_x(
            origin, align_width.unwrap_or(layout.width), &row, &align,
        ) + row.end - row.start;

        if let Some((mut background_origin, background_color)) = current_background.take() {
            if last_line_end_x == background_origin.x {
                background_origin.x -= max_glyph_size.width.half()
            };
            window.paint_quad(fill(
                Bounds {
                    origin: background_origin,
                    size: size(last_line_end_x - background_origin.x, line_height),
                },
                background_color,
            ));
        }

        Ok(())
    })
}

fn line_paint_bounds(
    origin: Point<Pixels>,
    layout: &LineLayout,
    line_height: Pixels,
    align: TextAlign,
    align_width: Option<Pixels>,
    wrap_boundaries: &[WrapBoundary],
) -> Bounds<Pixels> {
    let mut bounds = Bounds::new(
        origin,
        size(layout.width, line_height * (wrap_boundaries.len() + 1)),
    );
    if align == TextAlign::Left || layout.len == 0 { return bounds; }
    let align_width = align_width.unwrap_or(layout.width);
    for (row_ix, row) in layout.visual_row_ranges(wrap_boundaries).enumerate() {
        bounds = bounds.union(&Bounds::new(
            point(
                aligned_origin_x(origin, align_width, &row, &align),
                origin.y + line_height * row_ix,
            ),
            size(row.end - row.start, line_height),
        ));
    }
    bounds
}

fn aligned_origin_x(
    origin: Point<Pixels>,
    align_width: Pixels,
    row: &Range<Pixels>,
    align: &TextAlign,
) -> Pixels {
    let line_width = row.end - row.start;
    match align {
        TextAlign::Left => origin.x,
        TextAlign::Center => (origin.x * 2.0 + align_width - line_width) / 2.0,
        TextAlign::Right => origin.x + align_width - line_width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppContext as _, Context, FontId, GlyphId, IntoElement, Render, ShapedGlyph, ShapedRun,
        Styled, TestAppContext, TextRun, Underline, canvas, font, hsla,
    };
    use std::rc::Rc;

    /// Helper: build a ShapedLine from glyph descriptors without the platform text system.
    /// Each glyph is described as (byte_index, x_position).
    fn make_shaped_line(
        text: &str,
        glyphs: &[(usize, f32)],
        width: f32,
        decorations: &[DecorationRun],
    ) -> ShapedLine {
        let shaped_glyphs: Vec<ShapedGlyph> = glyphs
            .iter()
            .enumerate()
            .map(|(glyph_ix, &(index, x))| ShapedGlyph {
                id: GlyphId(0),
                position: point(px(x), px(0.0)),
                index,
                is_emoji: false,
                is_rtl: false,
                advance: px(glyphs.get(glyph_ix + 1).map_or(width, |glyph| glyph.1) - x),
            })
            .collect();

        ShapedLine {
            layout: Arc::new(LineLayout {
                font_size: px(16.0),
                width: px(width),
                ascent: px(12.0),
                descent: px(4.0),
                runs: vec![ShapedRun { font_size: None,
                    font_id: FontId(0),
                    glyphs: shaped_glyphs,
                }],
                len: text.len(),
                interior_carets: Vec::new(),
                isolation: None,
                ink_clip: None,
            }),
            text: SharedString::new(text).into(),
            decoration_runs: SmallVec::from(decorations.to_vec()),
        }
    }

    #[test]
    fn test_aligned_line_paint_bounds() {
        let line = make_shaped_line("abcd", &[(0, 0.), (1, 10.), (2, 20.), (3, 30.)], 40., &[]);
        let origin = point(px(10.), px(20.));
        let line_height = px(16.);
        for (wrapped, align_width, expected_edges) in [
            (false, None, [(10., 50.), (10., 50.), (10., 50.)]),
            (true, None, [(10., 50.), (10., 50.), (10., 50.)]),
            (false, Some(40.), [(10., 50.), (10., 50.), (10., 50.)]),
            (true, Some(40.), [(10., 50.), (10., 50.), (10., 50.)]),
            (false, Some(100.), [(10., 50.), (10., 80.), (10., 110.)]),
            (true, Some(100.), [(10., 50.), (10., 70.), (10., 110.)]),
            (false, Some(10.), [(10., 50.), (-5., 50.), (-20., 50.)]),
            (true, Some(10.), [(10., 50.), (5., 50.), (0., 50.)]),
            (false, Some(0.), [(10., 50.), (-10., 50.), (-30., 50.)]),
            (true, Some(0.), [(10., 50.), (0., 50.), (-10., 50.)]),
        ] {
            let boundaries = if wrapped {
                vec![WrapBoundary {
                    run_ix: 0,
                    glyph_ix: 2,
                }]
            } else {
                Vec::new()
            };
            for (align, (left, right)) in [TextAlign::Left, TextAlign::Center, TextAlign::Right]
                .into_iter()
                .zip(expected_edges)
            {
                assert_eq!(
                    line_paint_bounds(
                        origin,
                        &line.layout,
                        line_height,
                        align,
                        align_width.map(px),
                        &boundaries,
                    ),
                    Bounds::new(
                        point(px(left), origin.y),
                        size(px(right - left), px(if wrapped { 32. } else { 16. })),
                    ),
                    "wrapped={wrapped}, align={align:?}, align_width={align_width:?}",
                );
            }
        }
        let empty = make_shaped_line("", &[], 0., &[]);
        for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            assert_eq!(
                line_paint_bounds(
                    origin,
                    &empty.layout,
                    line_height,
                    align,
                    Some(px(100.)),
                    &[]
                ),
                Bounds::new(origin, size(Pixels::ZERO, line_height)),
            );
        }
    }

    #[gpui::test]
    fn test_underline_handler_matches_default_paint(cx: &mut TestAppContext) {
        test_underline_handler_at_scales(cx, |window, cx| {
            let first_style = UnderlineStyle {
                thickness: px(1.),
                color: Some(hsla(0., 1., 0.5, 1.)),
                wavy: true,
                skip_ink: false,
                pattern: Default::default(),
                ..Default::default()
            };
            let last_style = UnderlineStyle {
                color: Some(hsla(0.5, 1., 0.5, 1.)),
                wavy: false,
                ..first_style
            };
            let fallback_style = UnderlineStyle {
                color: Some(black()),
                ..first_style
            };
            let decoration = DecorationRun {
                len: 1,
                color: black(),
                background_color: None,
                underline: Some(first_style),
                overline: None,
                strikethrough: None,
                decoration_layers: None,
            };
            let line = underline_test_line(
                "aébcde",
                &[
                    decoration.clone(),
                    DecorationRun {
                        len: 2,
                        ..decoration.clone()
                    },
                    DecorationRun {
                        underline: None,
                        ..decoration.clone()
                    },
                    DecorationRun {
                        underline: Some(UnderlineStyle {
                            color: None,
                            ..first_style
                        }),
                        ..decoration.clone()
                    },
                    DecorationRun {
                        underline: Some(last_style),
                        ..decoration.clone()
                    },
                    DecorationRun {
                        underline: Some(last_style),
                        ..decoration
                    },
                ],
                false,
                window,
            );
            assert_eq!(line.width(), px(48.));
            for origin_x in [-3.25, 0., 4.25] {
                for (align, align_width, offset) in [
                    (TextAlign::Left, None, 0.),
                    (TextAlign::Center, None, 0.),
                    (TextAlign::Right, None, 0.),
                    (TextAlign::Left, Some(px(96.)), 0.),
                    (TextAlign::Center, Some(px(96.)), 24.),
                    (TextAlign::Right, Some(px(96.)), 48.),
                ] {
                    let origin = point(px(origin_x), px(12.25));
                    let line_height = px(20.);
                    window.next_frame.scene.clear();
                    line.paint(origin, line_height, align, align_width, window, cx)
                        .unwrap();
                    let original = window.next_frame.scene.underlines.clone();
                    window.next_frame.scene.clear();
                    line.layout
                        .paint(
                            origin,
                            line_height,
                            align,
                            align_width,
                            &line.decoration_runs,
                            window,
                            cx,
                        )
                        .unwrap();
                    assert_underline_primitives_eq(&window.next_frame.scene.underlines, &original);

                    window.next_frame.scene.clear();
                    let mut strokes = Vec::new();
                    line.paint_with_underline_handler(
                        origin,
                        line_height,
                        align,
                        align_width,
                        window,
                        cx,
                        |range, origin, width, style, window| {
                            strokes.push((range, origin, width, *style));
                            window.paint_underline(origin, width, style);
                        },
                    )
                    .unwrap();
                    let start = px(origin_x + offset);
                    let y = origin.y + underline_y_offset(line_height, line.ascent, line.descent);
                    assert_eq!(
                        strokes,
                        [
                            (0..3, point(start, y), px(16.), first_style),
                            (4..5, point(start + px(24.), y), px(8.), fallback_style),
                            (5..7, point(start + px(32.), y), px(16.), last_style),
                        ]
                    );
                    assert_underline_primitives_eq(&window.next_frame.scene.underlines, &original);

                    window.next_frame.scene.clear();
                    let mut captured = Vec::new();
                    line.paint_with_underline_handler(
                        origin,
                        line_height,
                        align,
                        align_width,
                        window,
                        cx,
                        |range, origin, width, style, _| {
                            captured.push((range, origin, width, *style))
                        },
                    )
                    .unwrap();
                    assert_eq!(captured, strokes);
                    assert_eq!(window.next_frame.scene.underlines.len(), 0);
                }
            }
            for text in ["", "abc"] {
                let line = underline_test_line(text, &[], false, window);
                let mut calls = 0;
                line.paint_with_underline_handler(
                    point(px(4.), px(10.)),
                    px(20.),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                    |_, _, _, _, _| calls += 1,
                )
                .unwrap();
                assert_eq!(calls, 0);
            }
        });
    }

    #[gpui::test]
    fn test_underline_handler_reports_zero_advance_geometry(cx: &mut TestAppContext) {
        test_underline_handler_at_scales(cx, |window, cx| {
            let first_style = UnderlineStyle {
                thickness: px(1.),
                color: Some(black()),
                wavy: true,
                skip_ink: false,
                pattern: Default::default(),
                ..Default::default()
            };
            let last_style = UnderlineStyle {
                wavy: false,
                ..first_style
            };
            let decoration = DecorationRun {
                len: 1,
                color: black(),
                background_color: None,
                underline: Some(first_style),
                overline: None,
                strikethrough: None,
                decoration_layers: None,
            };
            let line = underline_test_line(
                "ab",
                &[
                    decoration.clone(),
                    DecorationRun {
                        underline: Some(last_style),
                        ..decoration
                    },
                ],
                true,
                window,
            );
            let half_width = cx
                .text_system()
                .bounding_box(line.runs[0].font_id, line.font_size)
                .size
                .width
                / 2.;
            let origin = point(px(40.25), px(10.25));
            let line_height = px(20.);
            let y = origin.y + underline_y_offset(line_height, line.ascent, line.descent);
            for (align, offset) in [
                (TextAlign::Left, 0.),
                (TextAlign::Center, 16.),
                (TextAlign::Right, 32.),
            ] {
                window.next_frame.scene.clear();
                line.paint(origin, line_height, align, Some(px(32.)), window, cx)
                    .unwrap();
                let original = window.next_frame.scene.underlines.clone();
                window.next_frame.scene.clear();
                let mut strokes = Vec::new();
                line.paint_with_underline_handler(
                    origin,
                    line_height,
                    align,
                    Some(px(32.)),
                    window,
                    cx,
                    |range, origin, width, style, window| {
                        strokes.push((range, origin, width, *style));
                        window.paint_underline(origin, width, style);
                    },
                )
                .unwrap();
                let end = origin.x + px(offset);
                let start = point(end - half_width, y);
                let width = end - start.x;
                assert_eq!(
                    strokes,
                    [
                        (0..1, start, width, first_style),
                        (1..2, start, width, last_style),
                    ]
                );
                assert_underline_primitives_eq(&window.next_frame.scene.underlines, &original);
            }
        });
    }

    #[gpui::test]
    fn test_underline_handler_matches_wrapped_paint(cx: &mut TestAppContext) {
        test_underline_handler_at_scales(cx, |window, cx| {
            let style = UnderlineStyle {
                thickness: px(1.),
                color: Some(black()),
                wavy: true,
                skip_ink: false,
                pattern: Default::default(),
                ..Default::default()
            };
            for zero_advance in [false, true] {
                let line = underline_test_line(
                    "abcd",
                    &[DecorationRun {
                        len: 4,
                        color: black(),
                        background_color: None,
                        underline: Some(style),
                        overline: None,
                        strikethrough: None,
                        decoration_layers: None,
                    }],
                    zero_advance,
                    window,
                );
                let half_width = cx
                    .text_system()
                    .bounding_box(line.runs[0].font_id, line.font_size)
                    .size
                    .width
                    / 2.;
                let origin = point(px(40.25), px(10.25));
                let line_height = px(20.);
                let y = origin.y + underline_y_offset(line_height, line.ascent, line.descent);
                let wrapped = WrappedLine {
                    layout: Arc::new(WrappedLineLayout {
                        unwrapped_layout: line.layout,
                        wrap_boundaries: SmallVec::from_buf([WrapBoundary {
                            run_ix: 0,
                            glyph_ix: 2,
                        }]),
                        wrap_width: Some(px(16.)),
                    }),
                    text: line.text,
                    decoration_runs: line.decoration_runs.into_vec(),
                };
                window.next_frame.scene.clear();
                wrapped
                    .paint(origin, line_height, TextAlign::Left, None, window, cx)
                    .unwrap();
                let original = window.next_frame.scene.underlines.clone();
                window.next_frame.scene.clear();
                let mut strokes = Vec::new();
                paint_line(
                    origin,
                    &wrapped.unwrapped_layout,
                    line_height,
                    TextAlign::Left,
                    Some(px(16.)),
                    &wrapped.decoration_runs,
                    &wrapped.wrap_boundaries,
                    window,
                    cx,
                    UnderlinePainter::Custom(&mut |range, origin, width, style, window| {
                        strokes.push((range, origin, width, *style));
                        window.paint_underline(origin, width, style);
                    }, None),
                )
                .unwrap();
                let (start, width) = if zero_advance {
                    let start = origin.x - half_width;
                    (start, origin.x - start)
                } else {
                    (origin.x, px(16.))
                };
                assert_eq!(
                    strokes,
                    [
                        (0..4, point(start, y), width, style),
                        (0..4, point(start, y + line_height), width, style),
                    ]
                );
                assert_underline_primitives_eq(&window.next_frame.scene.underlines, &original);
            }
        });
    }

    #[test]
    fn test_split_at_invariants() {
        // Split "abcdef" at every possible byte index and verify structural invariants.
        let line = make_shaped_line(
            "abcdef",
            &[
                (0, 0.0),
                (1, 10.0),
                (2, 20.0),
                (3, 30.0),
                (4, 40.0),
                (5, 50.0),
            ],
            60.0,
            &[],
        );

        for i in 0..=6 {
            let (left, right) = line.split_at(i);

            assert_eq!(
                left.width() + right.width(),
                line.width(),
                "widths must sum at split={i}"
            );
            assert_eq!(
                left.len() + right.len(),
                line.len(),
                "lengths must sum at split={i}"
            );
            assert_eq!(
                format!("{}{}", left.text.as_ref(), right.text.as_ref()),
                "abcdef",
                "text must concatenate at split={i}"
            );
            assert_eq!(left.font_size, line.font_size, "font_size at split={i}");
            assert_eq!(right.ascent, line.ascent, "ascent at split={i}");
            assert_eq!(right.descent, line.descent, "descent at split={i}");
        }

        // Edge: split at 0 produces no left runs, full content on right
        let (left, right) = line.split_at(0);
        assert_eq!(left.runs.len(), 0);
        assert_eq!(right.runs[0].glyphs.len(), 6);

        // Edge: split at end produces full content on left, no right runs
        let (left, right) = line.split_at(6);
        assert_eq!(left.runs[0].glyphs.len(), 6);
        assert_eq!(right.runs.len(), 0);
    }

    #[test]
    fn test_split_at_glyph_rebasing() {
        // Two font runs (simulating a font fallback boundary at byte 3):
        //   run A (FontId 0): glyphs at bytes 0,1,2  positions 0,10,20
        //   run B (FontId 1): glyphs at bytes 3,4,5  positions 30,40,50
        // Successive splits simulate the incremental splitting done during wrap.
        let line = ShapedLine {
            layout: Arc::new(LineLayout {
                font_size: px(16.0),
                width: px(60.0),
                ascent: px(12.0),
                descent: px(4.0),
                runs: vec![
                    ShapedRun { font_size: None,
                        font_id: FontId(0),
                        glyphs: vec![
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(0.0), px(0.0)),
                                index: 0,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(10.0),
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(10.0), px(0.0)),
                                index: 1,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(10.0),
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(20.0), px(0.0)),
                                index: 2,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(10.0),
                            },
                        ],
                    },
                    ShapedRun { font_size: None,
                        font_id: FontId(1),
                        glyphs: vec![
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(30.0), px(0.0)),
                                index: 3,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(10.0),
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(40.0), px(0.0)),
                                index: 4,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(10.0),
                            },
                            ShapedGlyph {
                                id: GlyphId(0),
                                position: point(px(50.0), px(0.0)),
                                index: 5,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(10.0),
                            },
                        ],
                    },
                ],
                len: 6,
                interior_carets: Vec::new(),
                isolation: None,
                ink_clip: None,
            }),
            text: "abcdef".into(),
            decoration_runs: SmallVec::new(),
        };

        // First split at byte 2 — mid-run in run A
        let (first, remainder) = line.split_at(2);
        assert_eq!(first.text.as_ref(), "ab");
        assert_eq!(first.runs.len(), 1);
        assert_eq!(first.runs[0].font_id, FontId(0));

        // Remainder "cdef" should have two runs: tail of A (1 glyph) + all of B (3 glyphs)
        assert_eq!(remainder.text.as_ref(), "cdef");
        assert_eq!(remainder.runs.len(), 2);
        assert_eq!(remainder.runs[0].font_id, FontId(0));
        assert_eq!(remainder.runs[0].glyphs.len(), 1);
        assert_eq!(remainder.runs[0].glyphs[0].index, 0);
        assert_eq!(remainder.runs[0].glyphs[0].position.x, px(0.0));
        assert_eq!(remainder.runs[1].font_id, FontId(1));
        assert_eq!(remainder.runs[1].glyphs[0].index, 1);
        assert_eq!(remainder.runs[1].glyphs[0].position.x, px(10.0));

        // Second split at byte 2 within remainder — crosses the run boundary
        let (second, final_part) = remainder.split_at(2);
        assert_eq!(second.text.as_ref(), "cd");
        assert_eq!(final_part.text.as_ref(), "ef");
        assert_eq!(final_part.runs[0].glyphs[0].index, 0);
        assert_eq!(final_part.runs[0].glyphs[0].position.x, px(0.0));

        // Widths must sum across all three pieces
        assert_eq!(
            first.width() + second.width() + final_part.width(),
            line.width()
        );
    }

    #[test]
    fn test_split_at_decorations() {
        // Three decoration runs: red [0..2), green [2..5), blue [5..6).
        // Split at byte 3 — red goes entirely left, green straddles, blue goes entirely right.
        let red = Hsla {
            h: 0.0,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        };
        let green = Hsla {
            h: 0.3,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        };
        let blue = Hsla {
            h: 0.6,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        };

        let line = make_shaped_line(
            "abcdef",
            &[
                (0, 0.0),
                (1, 10.0),
                (2, 20.0),
                (3, 30.0),
                (4, 40.0),
                (5, 50.0),
            ],
            60.0,
            &[
                DecorationRun {
                    len: 2,
                    color: red,
                    background_color: None,
                    underline: None,
                    overline: None,
                    strikethrough: None,
                    decoration_layers: None,
                },
                DecorationRun {
                    len: 3,
                    color: green,
                    background_color: None,
                    underline: None,
                    overline: None,
                    strikethrough: None,
                    decoration_layers: None,
                },
                DecorationRun {
                    len: 1,
                    color: blue,
                    background_color: None,
                    underline: None,
                    overline: None,
                    strikethrough: None,
                    decoration_layers: None,
                },
            ],
        );

        let (left, right) = line.split_at(3);

        // Left: red(2) + green(1) — green straddled, left portion has len 1
        assert_eq!(left.decoration_runs.len(), 2);
        assert_eq!(left.decoration_runs[0].len, 2);
        assert_eq!(left.decoration_runs[0].color, red);
        assert_eq!(left.decoration_runs[1].len, 1);
        assert_eq!(left.decoration_runs[1].color, green);

        // Right: green(2) + blue(1) — green straddled, right portion has len 2
        assert_eq!(right.decoration_runs.len(), 2);
        assert_eq!(right.decoration_runs[0].len, 2);
        assert_eq!(right.decoration_runs[0].color, green);
        assert_eq!(right.decoration_runs[1].len, 1);
        assert_eq!(right.decoration_runs[1].color, blue);
    }

    struct UnderlineHandlerTestView(Rc<dyn Fn(&mut Window, &mut App)>);

    impl Render for UnderlineHandlerTestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let paint = self.0.clone();
            canvas(
                |_, _, _| {},
                move |_, _, window, cx| {
                    window.with_element_opacity(Some(0.5), |window| paint(window, cx));
                },
            )
            .size_full()
        }
    }

    fn test_underline_handler_at_scales(
        cx: &mut TestAppContext,
        paint: impl Fn(&mut Window, &mut App) + 'static,
    ) {
        let window = cx.add_window(move |_, _| UnderlineHandlerTestView(Rc::new(paint)));
        for scale in [1., 1.25, 1.5, 2., 3.] {
            cx.simulate_window_scale_factor_change(window.into(), scale);
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        }
    }

    fn underline_test_line(
        text: &str,
        decorations: &[DecorationRun],
        zero_advance: bool,
        window: &Window,
    ) -> ShapedLine {
        let mut line = window.text_system().shape_line(
            SharedString::new(text),
            px(16.),
            &[TextRun {
                len: text.len(),
                font: font(".ZedMono"),
                color: black(),
                ..TextRun::default()
            }],
            None,
        );
        line.decoration_runs = SmallVec::from(decorations.to_vec());
        let layout = &line.layout;
        let mut runs = layout.runs.clone();
        let advance = if zero_advance { px(0.) } else { px(8.) };
        let mut width = px(0.);
        for glyph in runs.iter_mut().flat_map(|run| &mut run.glyphs) {
            glyph.position.x = width;
            width += advance;
        }
        line.layout = Arc::new(LineLayout {
            font_size: layout.font_size,
            width,
            ascent: layout.ascent,
            descent: layout.descent,
            runs,
            len: layout.len,
            interior_carets: Vec::new(),
            isolation: None,
            ink_clip: None,
        });
        line
    }

    fn assert_underline_primitives_eq(actual: &[Underline], expected: &[Underline]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.bounds, expected.bounds);
            assert_eq!(actual.content_mask, expected.content_mask);
            assert_eq!(actual.color, expected.color);
            assert_eq!(actual.thickness, expected.thickness);
            assert_eq!(actual.pattern, expected.pattern);
            assert_eq!(actual.order, expected.order);
            assert_eq!(actual.pattern_period, expected.pattern_period);
        }
    }

    #[test]
    fn test_cursor_preserves_shaping_metadata_across_runs() {
        let line = ShapedLine {
            layout: Arc::new(LineLayout {
                font_size: px(16.0),
                width: px(50.0),
                ascent: px(12.0),
                descent: px(4.0),
                runs: vec![
                    ShapedRun { font_size: None,
                        font_id: FontId(3),
                        glyphs: vec![
                            ShapedGlyph {
                                id: GlyphId(11),
                                position: point(px(0.0), px(1.0)),
                                index: 0,
                                is_emoji: true,
                                is_rtl: false,
                                advance: px(17.0),
                            },
                            ShapedGlyph {
                                id: GlyphId(12),
                                position: point(px(17.0), px(1.0)),
                                index: 1,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(2.0),
                            },
                            ShapedGlyph {
                                id: GlyphId(13),
                                position: point(px(19.0), px(-1.0)),
                                index: 1,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(6.0),
                            },
                        ],
                    },
                    ShapedRun { font_size: None,
                        font_id: FontId(8),
                        glyphs: vec![
                            ShapedGlyph {
                                id: GlyphId(21),
                                position: point(px(25.0), px(1.0)),
                                index: 5,
                                is_emoji: true,
                                is_rtl: false,
                                advance: px(16.0),
                            },
                            ShapedGlyph {
                                id: GlyphId(22),
                                position: point(px(41.0), px(1.0)),
                                index: 7,
                                is_emoji: false,
                                is_rtl: false,
                                advance: px(9.0),
                            },
                        ],
                    },
                ],
                len: 10,
                interior_carets: Vec::new(),
                isolation: None,
                ink_clip: None,
            }),
            text: "a😀bcdef".into(),
            decoration_runs: SmallVec::new(),
        };
        let mut cursor = line.cursor();
        let first = cursor.take_until(5);
        assert_eq!(first.text.as_ref(), "a😀");
        assert_eq!(first.runs[0].font_id, FontId(3));
        assert_eq!(first.runs[0].glyphs[0].id, GlyphId(11));
        assert!(first.runs[0].glyphs[0].is_emoji);
        assert_eq!(first.runs[0].glyphs[1].index, 1);
        assert_eq!(first.runs[0].glyphs[1].position, point(px(17.0), px(1.0)));
        assert_eq!(first.runs[0].glyphs.len(), 3);
        assert_eq!(first.runs[0].glyphs[2].index, 1);
        assert_eq!(first.runs[0].glyphs[2].position, point(px(19.0), px(-1.0)));
        assert_eq!(cursor.x_offset(), px(25.0));

        let second = cursor.take_until(7);
        assert_eq!(second.text.as_ref(), "bc");
        assert_eq!(second.runs[0].font_id, FontId(8));
        assert_eq!(second.runs[0].glyphs[0].id, GlyphId(21));
        assert_eq!(second.runs[0].glyphs[0].index, 0);
        assert_eq!(second.runs[0].glyphs[0].position, point(px(0.0), px(1.0)));
        assert_eq!(cursor.x_offset(), px(41.0));

        let final_part = cursor.take_until(10);
        assert_eq!(final_part.text.as_ref(), "def");
        assert_eq!(final_part.runs[0].font_id, FontId(8));
        assert_eq!(final_part.runs[0].glyphs[0].id, GlyphId(22));
        assert_eq!(final_part.runs[0].glyphs[0].index, 0);
        assert_eq!(
            final_part.runs[0].glyphs[0].position,
            point(px(0.0), px(1.0))
        );
    }

    #[test]
    fn test_cursor_preserves_existing_visual_order_splitting() {
        let line = make_shaped_line("abc", &[(0, 0.0), (2, 10.0), (1, 20.0)], 30.0, &[]);
        let mut cursor = line.cursor();
        let mut remainder = line.clone();
        let mut previous_boundary = 0;
        for boundary in [0, 1, 2, 3] {
            let (expected, rest) = remainder.split_at(boundary - previous_boundary);
            let actual = cursor.take_until(boundary);
            assert_eq!(actual.text, expected.text);
            assert_eq!(actual.width(), expected.width());
            assert_eq!(actual.runs.len(), expected.runs.len());
            for (actual, expected) in actual.runs.iter().zip(&expected.runs) {
                assert_eq!(actual.font_id, expected.font_id);
                assert_eq!(actual.glyphs.len(), expected.glyphs.len());
                for (actual, expected) in actual.glyphs.iter().zip(&expected.glyphs) {
                    assert_eq!(actual.id, expected.id);
                    assert_eq!(actual.index, expected.index);
                    assert_eq!(actual.position, expected.position);
                }
            }
            assert_eq!(cursor.x_offset(), line.x_for_index(boundary));
            remainder = rest;
            previous_boundary = boundary;
        }
    }

    #[test]
    fn test_cursor_partitions_one_decoration_across_three_chunks() {
        let line = make_shaped_line(
            "abcdef",
            &[
                (0, 0.0),
                (1, 10.0),
                (2, 20.0),
                (3, 30.0),
                (4, 40.0),
                (5, 50.0),
            ],
            60.0,
            &[DecorationRun {
                len: 6,
                color: Hsla {
                    h: 0.2,
                    s: 0.4,
                    l: 0.6,
                    a: 1.0,
                },
                background_color: None,
                underline: None,
                overline: None,
                strikethrough: None,
                decoration_layers: None,
            }],
        );
        let mut cursor = line.cursor();
        assert_eq!(cursor.take_until(2).decoration_runs[0].len, 2);
        assert_eq!(cursor.take_until(4).decoration_runs[0].len, 2);
        assert_eq!(cursor.take_until(6).decoration_runs[0].len, 2);
    }

    #[test]
    fn test_cursor_matches_successive_splits_at_ordered_boundaries() {
        let decorations: Vec<_> = [2, 0, 3, 1]
            .into_iter()
            .map(|len| DecorationRun {
                len,
                color: Hsla {
                    h: len as f32 / 10.0,
                    s: 0.5,
                    l: 0.5,
                    a: 1.0,
                },
                background_color: Some(black()),
                underline: None,
                overline: None,
                strikethrough: None,
                decoration_layers: None,
            })
            .collect();
        let line = make_shaped_line(
            "abcdef",
            &[(0, 5.0), (0, 5.0), (2, 15.0), (4, 25.0), (5, 35.0)],
            45.0,
            &decorations,
        );
        for first in 0..=line.len() {
            for second in first..=line.len() {
                let mut cursor = line.cursor();
                let mut remainder = line.clone();
                let mut previous_boundary = 0;
                let mut total_width = px(0.0);
                let mut text = String::new();
                for boundary in [first, second, line.len(), line.len()] {
                    let (expected, rest) = remainder.split_at(boundary - previous_boundary);
                    let actual = cursor.take_until(boundary);
                    assert_eq!(actual.text, expected.text);
                    assert_eq!(actual.len(), expected.len());
                    assert_eq!(actual.width(), expected.width());
                    assert_eq!(actual.runs.len(), expected.runs.len());
                    for (actual, expected) in actual.runs.iter().zip(&expected.runs) {
                        assert_eq!(actual.font_id, expected.font_id);
                        assert_eq!(actual.glyphs.len(), expected.glyphs.len());
                        for (actual, expected) in actual.glyphs.iter().zip(&expected.glyphs) {
                            assert_eq!(actual.id, expected.id);
                            assert_eq!(actual.index, expected.index);
                            assert_eq!(actual.position, expected.position);
                        }
                    }
                    assert_eq!(actual.decoration_runs.len(), expected.decoration_runs.len());
                    for (actual, expected) in
                        actual.decoration_runs.iter().zip(&expected.decoration_runs)
                    {
                        assert_eq!(actual.len, expected.len);
                        assert_eq!(actual.color, expected.color);
                        assert_eq!(actual.background_color, expected.background_color);
                    }
                    total_width += actual.width();
                    text.push_str(&actual.text);
                    remainder = rest;
                    previous_boundary = boundary;
                }
                assert_eq!(total_width, line.width());
                assert_eq!(text, line.text.as_ref());
            }
        }
    }

    #[test]
    fn test_cursor_empty_chunks_and_repeated_boundaries() {
        let line = make_shaped_line("ab", &[(0, 5.0), (1, 15.0)], 20.0, &[]);
        let mut cursor = line.cursor();
        assert_eq!(cursor.take_until(0).text.as_ref(), "");
        assert_eq!(cursor.take_until(0).text.as_ref(), "");
        assert_eq!(cursor.take_until(1).text.as_ref(), "a");
        assert_eq!(cursor.take_until(2).text.as_ref(), "b");
        assert_eq!(cursor.take_until(2).text.as_ref(), "");
        let empty = make_shaped_line("", &[], 0.0, &[]);
        let piece = empty.cursor().take_until(0);
        assert!(piece.text.is_empty());
        assert!(piece.runs.is_empty());
        assert_eq!(piece.width(), px(0.0));
    }

    #[test]
    fn test_cursor_rejects_invalid_boundaries() {
        let line = make_shaped_line("é", &[(0, 0.0)], 10.0, &[]);
        let mut cursor = line.cursor();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cursor.take_until(1);
            }))
            .is_err()
        );
        let mut cursor = line.cursor();
        cursor.take_until(2);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cursor.take_until(0);
            }))
            .is_err()
        );
        let mut cursor = line.cursor();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cursor.take_until(3);
            }))
            .is_err()
        );
    }
}
