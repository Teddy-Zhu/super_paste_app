use std::time::{Duration, Instant};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Easing, Interpolate, Keyframe, Keyframes, Timing};
use super::{OverlayView, Phase};

#[derive(Clone, Copy)]
pub(super) struct SearchStyle { pub width: Pixels, pub opacity: f32, pub padding: Pixels, pub scale: f32 }
impl SearchStyle {
    fn hidden() -> Self { Self { width: px(0.0), opacity: 0.0, padding: px(0.0), scale: 0.0 } }
    fn shown(width: Pixels) -> Self { Self { width, opacity: 1.0, padding: px(8.0), scale: 1.0 } }
}
impl Interpolate for SearchStyle {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Self { width: self.width + (target.width - self.width) * progress,
            opacity: self.opacity + (target.opacity - self.opacity) * progress,
            padding: self.padding + (target.padding - self.padding) * progress,
            scale: self.scale + (target.scale - self.scale) * progress }
    }
}
#[derive(Clone, Copy)]
pub(super) struct TabsStyle { pub opacity: f32, pub max_width: f32, pub scale: f32 }
impl TabsStyle {
    fn hidden() -> Self { Self { opacity: 0.0, max_width: 0.0, scale: 0.0 } }
    fn shown() -> Self { Self { opacity: 1.0, max_width: 1.0, scale: 1.0 } }
}
impl Interpolate for TabsStyle {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Self { opacity: self.opacity + (target.opacity - self.opacity) * progress,
            max_width: self.max_width + (target.max_width - self.max_width) * progress,
            scale: self.scale + (target.scale - self.scale) * progress }
    }
}

struct Running {
    started: Instant, timing: Timing,
    search: Keyframes<SearchStyle>, tabs: Keyframes<TabsStyle>,
}
impl Running {
    fn sample(&self, now: Instant) -> (SearchStyle, TabsStyle, bool) {
        let sample = self.timing.sample(now.saturating_duration_since(self.started));
        (self.search.sample(sample.directed_progress), self.tabs.sample(sample.directed_progress), sample.finished)
    }
}
#[derive(Default)]
pub(super) struct SearchMotion { running: Option<Running> }
pub(super) struct Frame { pub search: SearchStyle, pub tabs: TabsStyle, pub animating: bool }

impl SearchMotion {
    pub fn cancel(&mut self) { self.running = None; }

    fn begin(&mut self, visible: bool, width: Pixels, now: Instant, reduced: bool) {
        if reduced { self.cancel(); return; }
        // A reversal begins at its current presentation. A fresh collapse uses
        // the reference's separate 400px cap, rather than the expand endpoint.
        let previous = self.running.as_ref().map(|running| running.sample(now)).filter(|sample| !sample.2);
        let expand_width = (width - px(140.0)).max(px(180.0)).min(px(600.0));
        let collapse_width = (width - px(180.0)).max(px(140.0)).min(px(400.0));
        let search_start = previous.map(|sample| sample.0).unwrap_or_else(|| if visible { SearchStyle::hidden() } else { SearchStyle::shown(collapse_width) });
        let tabs_start = previous.map(|sample| sample.1).unwrap_or_else(|| if visible { TabsStyle::shown() } else { TabsStyle::hidden() });
        let search = if visible {
            [Keyframe::new(0.0, search_start),
                Keyframe::new(0.7, SearchStyle { width: expand_width, opacity: 0.8, padding: px(6.0), scale: 0.95 }),
                Keyframe::new(1.0, SearchStyle::shown(expand_width))]
        } else {
            [Keyframe::new(0.0, search_start),
                Keyframe::new(0.6, SearchStyle { width: px(100.0), opacity: 0.3, padding: px(2.0), scale: 0.3 }),
                Keyframe::new(1.0, SearchStyle::hidden())]
        };
        let tabs = if visible {
            [Keyframe::new(0.0, tabs_start), Keyframe::new(0.5, TabsStyle { opacity: 0.4, max_width: 0.3, scale: 0.6 }), Keyframe::new(1.0, TabsStyle::hidden())]
        } else {
            [Keyframe::new(0.0, tabs_start), Keyframe::new(0.4, TabsStyle { opacity: 0.6, max_width: 0.7, scale: 0.7 }), Keyframe::new(1.0, TabsStyle::shown())]
        };
        self.running = Some(Running { started: now, timing: Timing::new(Duration::from_millis(250)).ease(Easing::Ease),
            search: Keyframes::try_new(search).expect("fixed search keyframe offsets"),
            tabs: Keyframes::try_new(tabs).expect("fixed tab keyframe offsets") });
    }

    pub fn advance(&mut self, visible: bool, width: Pixels, window: &mut Window, cx: &App) -> Frame {
        if cx.reduce_motion() { self.cancel(); }
        if let Some(running) = &self.running {
            let (search, tabs, finished) = running.sample(cx.background_executor().now());
            if finished { self.cancel(); } else { window.request_animation_frame(); }
            return Frame { search, tabs, animating: !finished };
        }
        Frame { search: if visible { SearchStyle::shown((width - px(140.0)).max(px(180.0)).min(px(600.0))) } else { SearchStyle::hidden() },
            tabs: if visible { TabsStyle::hidden() } else { TabsStyle::shown() }, animating: false }
    }
}

impl OverlayView {
    pub(super) fn set_search_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_visible == visible { return; }
        self.search_motion.begin(visible, self.dimensions.width, cx.background_executor().now(), cx.reduce_motion());
        self.search_visible = visible;
        if !visible { self.search_hovered = false; self.clear_query(window, cx); }
        // The collapsed editor remains mounted and focused, so typing and IME
        // input do not wait for the visual transition to finish.
        self.search.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn reveal_search_for_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Opening | Phase::Opened) { self.set_search_visible(true, window, cx); }
    }
}
