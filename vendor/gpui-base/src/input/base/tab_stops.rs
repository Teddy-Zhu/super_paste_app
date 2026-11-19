//! Tab stops for shaped lines.
//!
//! The platform shapers do not agree on what a `\t` is: DirectWrite gives it
//! no width at all, CoreText uses its own default stops. Neither knows the
//! editor's tab size. So a line is shaped as is and its tabs are then replaced
//! with the next stop from the paragraph's start edge. Glyph positions,
//! advances and interior carets all follow the same adjustment.

use std::sync::Arc;

use gpui::{Font, LineLayout, Pixels, ShapedGlyph, ShapedLine, ShapedRun, TextDirection, WindowTextSystem, px};

/// The distance between two tab stops: `tab_size` spaces of `font`.
pub(crate) fn tab_stop_width(
    text_system: &WindowTextSystem,
    font: &Font,
    font_size: Pixels,
    tab_size: usize,
) -> Pixels {
    let font_id = text_system.resolve_font(font);
    text_system.layout_width(font_id, font_size, ' ') * tab_size as f32
}

/// Replaces each tab of `line` to end on the next multiple of `stop`.
///
/// Stops are counted from the start of `line`, so a soft-wrapped row starts
/// its own stops, the same way the wrapper measures it.
/// `direction` is resolved from the containing hard paragraph, not this row.
pub(crate) fn expand_tabs(line: &mut ShapedLine, stop: Pixels, direction: TextDirection) {
    if let Some(layout) = expanded_layout(line, &line.text, stop, direction) {
        **line = Arc::new(layout);
    }
}

/// The width of `layout` (shaped from `text`) once its tabs are expanded.
pub(crate) fn expanded_width(layout: &LineLayout, text: &str, stop: Pixels, direction: TextDirection) -> Pixels {
    if stop <= px(0.) || !text.contains('\t') {
        return layout.width;
    }

    layout.width
        + tab_shifts(layout, text, stop, tab_direction(direction))
            .last()
            .map_or(px(0.), |(_, shift)| shift)
}

/// Cumulative shift after replacing a tab, measured from the paragraph's
/// start edge. Keep the running shift in f64 until it enters pixel geometry.
pub(crate) fn next_tab_shift(start: f64, end: f64, shift: f64, stop: f64) -> f64 {
    stop * (((start + shift) / stop).floor() + 1.) - end
}

fn tab_direction(direction: TextDirection) -> bool {
    match direction {
        TextDirection::Ltr => false,
        TextDirection::Rtl => true,
        TextDirection::Auto => panic!("tab stops require the resolved hard-paragraph direction"),
    }
}

/// Cumulative shifts after each tab, in text order.
fn tab_shifts<'a>(
    layout: &'a LineLayout,
    text: &'a str,
    stop: Pixels,
    rtl: bool,
) -> impl Iterator<Item = (usize, Pixels)> + 'a {
    let mut previous = 0;
    let ordered_ltr = !rtl && layout.runs.iter().flat_map(|run| &run.glyphs).all(|glyph| {
        let ordered = !glyph.is_rtl && glyph.index >= previous;
        previous = glyph.index;
        ordered
    });
    // Native paint order need not be source order. Index only the tab cells
    // for the bidi path; ordinary LTR text keeps a streaming advance cursor.
    let mut positions = Vec::new();
    if !ordered_ltr {
        positions.extend(layout.visual_cluster_advances().filter_map(|(index, cell)| {
            if text.as_bytes().get(index) != Some(&b'\t') {
                return None;
            }
            let (start, end) = if rtl {
                (layout.width - cell.end, layout.width - cell.start)
            } else {
                (cell.start, cell.end)
            };
            Some((index, start, end))
        }));
        positions.sort_unstable_by_key(|(index, _, _)| *index);
    }
    let mut cells = layout.visual_cluster_advances().peekable();
    // LTR source boundaries increase monotonically. Keep this cursor between
    // lookups instead of restarting a line scan for every tab.
    let mut x_for_index = move |index| {
        while cells.peek().is_some_and(|(source, _)| *source < index) {
            cells.next();
        }
        cells.peek().map_or(layout.width, |(_, cell)| cell.start)
    };

    // Keep the cumulative shift in double precision. Rounding it to Pixels
    // after every tab can put a stop just below its boundary, making the next
    // tab land on that same stop instead of advancing.
    let stop = f64::from(f32::from(stop));
    text.match_indices('\t').scan(0_f64, move |shift, (ix, _)| {
        let (start, end) = if ordered_ltr {
            (x_for_index(ix), x_for_index(ix + 1))
        } else if let Ok(position) = positions.binary_search_by_key(&ix, |entry| entry.0) {
            let (_, start, end) = positions[position];
            (start, end)
        } else {
            // Use native carets when the tab has no dedicated glyph cluster.
            let start = layout.x_for_index(ix);
            let end = layout.x_for_index_with_affinity(ix + 1, gpui::CaretAffinity::Upstream);
            if rtl { (layout.width - start, layout.width - end) } else { (start, end) }
        };
        *shift = next_tab_shift(
            f64::from(f32::from(start)),
            f64::from(f32::from(end)),
            *shift,
            stop,
        );
        Some((ix, px(*shift as f32)))
    })
}

/// `layout` with its tabs widened, or `None` when it has none to widen.
fn expanded_layout(layout: &LineLayout, text: &str, stop: Pixels, direction: TextDirection) -> Option<LineLayout> {
    if stop <= px(0.) || !text.contains('\t') {
        return None;
    }
    let rtl = tab_direction(direction);
    let shifts: Vec<_> = tab_shifts(layout, text, stop, rtl).collect();
    let shift = shifts.last().map_or(px(0.), |(_, shift)| *shift);

    let shift_after = |index: usize| {
        let tabs_before = shifts.partition_point(|&(tab, _)| {
            if rtl { tab <= index } else { tab < index }
        });
        let before = tabs_before
            .checked_sub(1)
            .map_or(px(0.), |last| shifts[last].1);
        if rtl { shift - before } else { before }
    };
    let tab_advance = |index: usize| {
        shifts.binary_search_by_key(&index, |entry| entry.0).ok().map_or(px(0.), |tab| {
            shifts[tab].1 - tab.checked_sub(1).map_or(px(0.), |previous| shifts[previous].1)
        })
    };

    let mut previous_source = None;
    let runs = layout
        .runs
        .iter()
        .map(|run| ShapedRun { font_size: run.font_size,
            font_id: run.font_id,
            glyphs: run
                .glyphs
                .iter()
                .map(|glyph| {
                    // A fallback mark can share the tab's source cluster;
                    // attach its extra advance only once across font runs.
                    let extra = if previous_source == Some(glyph.index) {
                        px(0.)
                    } else {
                        tab_advance(glyph.index)
                    };
                    previous_source = Some(glyph.index);
                    ShapedGlyph {
                        position: gpui::point(
                            glyph.position.x + shift_after(glyph.index),
                            glyph.position.y,
                        ),
                        advance: glyph.advance + extra,
                        ..glyph.clone()
                    }
                })
                .collect(),
        })
        .collect();

    Some(LineLayout {
        font_size: layout.font_size,
        width: layout.width + shift,
        ascent: layout.ascent,
        descent: layout.descent,
        runs,
        len: layout.len,
        interior_carets: layout.interior_carets.iter().map(|caret| gpui::ShapedCaret {
            x: caret.x + shift_after(caret.cluster_start),
            ..*caret
        }).collect(),
        isolation: layout.isolation.clone(),
        ink_clip: layout.ink_clip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{FontId, GlyphId, point};

    /// A line where every byte is one glyph `advance` wide, except tabs,
    /// which are `tab_advance` wide.
    fn layout(text: &str, advance: f32, tab_advance: f32) -> LineLayout {
        let mut x = 0.;
        let glyphs = text
            .char_indices()
            .map(|(index, c)| {
                let glyph = ShapedGlyph {
                    id: GlyphId(0),
                    position: point(px(x), px(0.)),
                    index,
                    is_emoji: false,
                    is_rtl: false,
                    advance: px(if c == '\t' { tab_advance } else { advance }),
                };
                x += if c == '\t' { tab_advance } else { advance };
                glyph
            })
            .collect();
        LineLayout {
            font_size: px(10.),
            width: px(x),
            ascent: px(8.),
            descent: px(2.),
            runs: vec![ShapedRun { font_size: None,
                font_id: FontId(0),
                glyphs,
            }],
            len: text.len(),
            interior_carets: Vec::new(),
            isolation: None, ink_clip: None,
        }
    }

    fn xs(layout: &LineLayout) -> Vec<f32> {
        layout.runs[0]
            .glyphs
            .iter()
            .map(|glyph| f32::from(glyph.position.x))
            .collect()
    }

    #[test]
    fn leaves_lines_without_tabs_alone() {
        assert!(expanded_layout(&layout("abc", 10., 0.), "abc", px(40.), TextDirection::Ltr).is_none());
    }

    #[test]
    fn zero_width_tabs_reach_the_next_stop() {
        let text = "\t\tx";
        let expanded = expanded_layout(&layout(text, 10., 0.), text, px(40.), TextDirection::Ltr).unwrap();
        assert_eq!(xs(&expanded), vec![0., 40., 80.]);
        assert_eq!(expanded.width, px(90.));
    }

    #[test]
    fn a_tab_after_text_only_fills_up_to_the_stop() {
        let text = "ab\tc";
        let expanded = expanded_layout(&layout(text, 10., 0.), text, px(40.), TextDirection::Ltr).unwrap();
        assert_eq!(xs(&expanded), vec![0., 10., 20., 40.]);
    }

    #[test]
    fn a_tab_on_a_stop_takes_a_whole_stop() {
        let text = "abcd\te";
        let expanded = expanded_layout(&layout(text, 10., 0.), text, px(40.), TextDirection::Ltr).unwrap();
        assert_eq!(xs(&expanded)[5], 80.);
    }

    #[test]
    fn fractional_stops_keep_consecutive_tabs_advancing() {
        let text = format!("{}x", "\t".repeat(256));
        for stop in [14.4, 16.8, 28.8, 33.6] {
            for tab_advance in [0., 28.] {
                let shaped = layout(&text, 10., tab_advance);
                let expanded = expanded_layout(&shaped, &text, px(stop), TextDirection::Ltr).unwrap();
                for (index, glyph) in expanded.runs[0].glyphs.iter().enumerate() {
                    let expected = stop * index as f32;
                    assert!(
                        (f32::from(glyph.position.x) - expected).abs() < 0.001,
                        "stop {stop}, tab advance {tab_advance}, glyph {index}: {:?}",
                        glyph.position.x,
                    );
                }
                assert_eq!(expanded_width(&shaped, &text, px(stop), TextDirection::Ltr), expanded.width);
            }
        }
    }

    #[test]
    fn shaper_widths_for_tabs_are_replaced() {
        let text = "\tx";
        let expanded = expanded_layout(&layout(text, 10., 28.), text, px(40.), TextDirection::Ltr).unwrap();
        assert_eq!(xs(&expanded), vec![0., 40.]);
        assert_eq!(expanded.width, px(50.));
    }

    #[test]
    fn a_trailing_tab_widens_the_line() {
        let text = "a\t";
        assert_eq!(
            expanded_width(&layout(text, 10., 0.), text, px(40.), TextDirection::Ltr),
            px(40.)
        );
    }

    #[test]
    fn tab_stops_cross_shaped_runs_and_multibyte_characters() {
        let text = "é\t中\t\tz\t";
        let mut shaped = layout(text, 10., 28.);
        let remaining = shaped.runs[0].glyphs.split_off(3);
        shaped.runs.push(ShapedRun { font_size: None,
            font_id: FontId(1),
            glyphs: remaining,
        });

        let expanded = expanded_layout(&shaped, text, px(40.), TextDirection::Ltr).unwrap();
        let positions: Vec<_> = expanded
            .runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| f32::from(glyph.position.x))
            .collect();
        assert_eq!(positions, vec![0., 10., 40., 50., 80., 120., 130.]);
        assert_eq!(expanded.width, px(160.));
        assert_eq!(expanded_width(&shaped, text, px(40.), TextDirection::Ltr), expanded.width);
        assert_eq!(expanded_width(&shaped, text, px(0.)), shaped.width);
    }

    #[test]
    fn expands_long_tab_separated_lines() {
        let count = 16_384;
        let text = "\tx".repeat(count);
        let shaped = layout(&text, 10., 0.);
        let expected_width = px(count as f32 * 40. + 10.);
        let expanded = expanded_layout(&shaped, &text, px(40.), TextDirection::Ltr).unwrap();

        assert_eq!(expanded.width, expected_width);
        assert_eq!(expanded_width(&shaped, &text, px(40.), TextDirection::Ltr), expected_width);
        for (ix, glyphs) in expanded.runs[0].glyphs.chunks_exact(2).enumerate() {
            let tab_start = if ix == 0 { 0. } else { ix as f32 * 40. + 10. };
            assert_eq!(glyphs[0].position.x, px(tab_start));
            assert_eq!(glyphs[1].position.x, px((ix + 1) as f32 * 40.));
        }
    }
}
