use std::time::{Duration, Instant};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::WindowExt;
use super::{OverlayView, Phase};

const DRAG_THRESHOLD: f32 = 6.0;
const MAX_VELOCITY: f32 = 4.0;
const DECELERATION: f32 = 0.003;
const SMOOTHING_MS: f32 = 35.0;
const RELEASE_IDLE_MS: f32 = 100.0;
const WHEEL_WINDOW_MS: f32 = 160.0;

/// Both history and tags use the reference wheel's dominant axis and scale.
/// GPUI deltas move content, whereas DOM deltas move the viewport.
pub(super) fn wheel_delta(delta: ScrollDelta) -> Pixels {
    let delta = delta.pixel_delta(px(28.0));
    let amount = if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y } * 1.2;
    if amount.as_f32().is_finite() { amount } else { Pixels::ZERO }
}

#[derive(Default)]
pub(super) struct Scrolling {
    drag: Option<Drag>,
    animation: Option<Animation>,
    wheel: Option<(Instant, f32)>,
    suppress_click: bool,
}
struct Drag {
    origin: Point<Pixels>, last: f32, moved: Instant, velocity: f32, active: bool,
}
enum Animation {
    Wheel { remaining: f32 },
    Glide { distance: f32, duration: Duration, started: Instant, eased: f32, curve: Curve },
}
enum Curve { Quadratic, Cubic }
impl Scrolling {
    pub(super) fn stop(&mut self) {
        self.drag = None; self.animation = None; self.wheel = None;
    }
    pub(super) fn press(&mut self) { self.stop(); self.suppress_click = false; }
    pub(super) fn suppress(&mut self, event: &ClickEvent) -> bool {
        if matches!(event, ClickEvent::Mouse(_)) && self.suppress_click {
            self.suppress_click = false; true
        } else { false }
    }
    pub(super) fn dragging(&self) -> bool { self.drag.as_ref().is_some_and(|drag| drag.active) }
}
impl OverlayView {
    fn axis_position(&self, position: Point<Pixels>) -> f32 {
        f32::from(if self.vertical() { position.y } else { position.x })
    }
    fn scroll_position(&self) -> f32 { -self.axis_position(self.scroll.offset()) }
    fn scroll_limit(&self) -> f32 {
        let viewport = self.scroll.base_handle().bounds().size;
        let extent = self.sizes.first().copied().unwrap_or_default();
        let (extent, viewport) = if self.vertical() { (extent.height, viewport.height) } else { (extent.width, viewport.width) };
        (f32::from(extent) * self.entries.len() as f32 - f32::from(viewport)).max(0.0)
    }
    fn set_scroll_position(&self, position: f32) -> f32 {
        let old = self.scroll_position();
        let position = position.clamp(0.0, self.scroll_limit());
        self.scroll.set_offset(if self.vertical() { point(px(0.0), px(-position)) } else { point(px(-position), px(0.0)) });
        position - old
    }
    pub(super) fn drag_start(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase != Phase::Opened || self.menu.is_some() || window.has_active_dialog(cx) || self.scroll_limit() <= 0.0 { return; }
        window.prevent_default();
        self.scrolling.drag = Some(Drag { origin: event.position, last: self.axis_position(event.position), moved: Instant::now(), velocity: 0.0, active: false });
    }
    fn drag_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.dragging() && self.pressed_card.take().is_some() { cx.notify(); }
        let Some(mut drag) = self.scrolling.drag.take() else { return; };
        if !event.dragging() || self.phase != Phase::Opened || window.has_active_dialog(cx) { self.scrolling.stop(); cx.notify(); return; }
        if !drag.active {
            let delta = event.position - drag.origin;
            if f32::from(delta.x).hypot(f32::from(delta.y)) < DRAG_THRESHOLD { self.scrolling.drag = Some(drag); return; }
            self.menu = None; self.scrolling.animation = None; self.scrolling.wheel = None;
            self.scrolling.suppress_click = true; drag.active = true;
        }
        window.prevent_default(); cx.stop_propagation();
        let now = Instant::now(); let position = self.axis_position(event.position);
        let moved = self.set_scroll_position(self.scroll_position() + drag.last - position);
        let elapsed = (now.duration_since(drag.moved).as_secs_f32() * 1000.0).max(1.0);
        let velocity = (moved / elapsed).clamp(-MAX_VELOCITY, MAX_VELOCITY);
        let weight = 1.0 - (-elapsed / SMOOTHING_MS).exp();
        drag.velocity = if moved == 0.0 || drag.velocity == 0.0 || velocity.signum() != drag.velocity.signum() { velocity }
            else { drag.velocity + (velocity - drag.velocity) * weight };
        drag.last = position; drag.moved = now; self.scrolling.drag = Some(drag); cx.notify();
    }
    fn drag_end(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left { return; }
        if self.pressed_card.take().is_some() { cx.notify(); }
        let Some(drag) = self.scrolling.drag.take() else { return; };
        if !drag.active { return; }
        // Cancel the mouse click before GPUI dispatches a card's click handler.
        window.prevent_default(); cx.stop_propagation();
        let idle = drag.moved.elapsed().as_secs_f32() * 1000.0;
        let velocity = drag.velocity * (1.0 - idle / RELEASE_IDLE_MS).max(0.0);
        if self.phase == Phase::Opened && !window.has_active_dialog(cx) && !cx.reduce_motion() && velocity.abs() >= 0.08 {
            let start = self.scroll_position();
            let target = (start + velocity * velocity.abs() / (2.0 * DECELERATION)).clamp(0.0, self.scroll_limit());
            let distance = target - start;
            if distance != 0.0 { self.glide(distance, Duration::from_secs_f32(2.0 * distance.abs() / velocity.abs() / 1000.0), Curve::Quadratic); }
        }
        cx.notify();
    }
    fn wheel_scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.entries.is_empty() || self.menu.is_some() || window.has_active_dialog(cx)
            || !self.scroll.base_handle().bounds().contains(&event.position) { return; }
        window.prevent_default(); cx.stop_propagation();
        if self.phase != Phase::Opened { return; }
        self.scrolling.drag = None;
        let amount = -f32::from(wheel_delta(event.delta));
        if amount == 0.0 { return; }
        let now = Instant::now(); let direction = amount.signum();
        let same = self.scrolling.wheel.is_some_and(|(_, previous)| previous == direction);
        let acceleration = self.scrolling.wheel.filter(|_| same).map_or(0.0, |(time, _)|
            (1.0 - now.duration_since(time).as_secs_f32() * 1000.0 / WHEEL_WINDOW_MS).max(0.0));
        let remaining = match self.scrolling.animation.take() {
            Some(Animation::Wheel { remaining }) if same => remaining,
            _ => 0.0,
        };
        self.scrolling.wheel = Some((now, direction));
        let remaining = remaining + amount * (1.5 + acceleration);
        if cx.reduce_motion() { self.set_scroll_position(self.scroll_position() + remaining); }
        else { self.scrolling.animation = Some(Animation::Wheel { remaining }); }
        cx.notify();
    }
    fn glide(&mut self, distance: f32, duration: Duration, curve: Curve) {
        self.scrolling.animation = Some(Animation::Glide { distance, duration, started: Instant::now(), eased: 0.0, curve });
    }
    pub(super) fn reveal_selection(&mut self, index: usize, cx: &App) {
        self.scrolling.stop();
        let viewport = self.scroll.base_handle().bounds().size;
        let viewport = f32::from(if self.vertical() { viewport.height } else { viewport.width });
        if viewport <= 0.0 { self.scroll.scroll_to_item(index, ScrollStrategy::Nearest); return; }
        let card = self.card_size(); let card = f32::from(if self.vertical() { card.height } else { card.width });
        let start = self.scroll_position(); let selected = index as f32 * (card + 10.0);
        let target = if selected < start + 40.0 { (selected - 40.0).max(0.0) }
            else if selected + card > start + viewport - 40.0 { (selected + card - viewport + 40.0).min(self.scroll_limit()) }
            else { start };
        if (target - start).abs() > 1.0 {
            if cx.reduce_motion() { self.set_scroll_position(target); }
            else { self.glide(target - start, Duration::from_millis(300), Curve::Cubic); }
        }
    }
    pub(super) fn advance_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Opening | Phase::Opened) || window.has_active_dialog(cx) { self.scrolling.stop(); return; }
        let Some(mut animation) = self.scrolling.animation.take() else { return; };
        if cx.reduce_motion() {
            // A preference change can arrive between frames. Settle the
            // remaining displacement against the current history anchor once.
            let remaining = match animation {
                Animation::Wheel { remaining } => remaining,
                Animation::Glide { distance, eased, .. } => distance * (1.0 - eased),
            };
            self.scrolling.wheel = None;
            self.set_scroll_position(self.scroll_position() + remaining);
            return;
        }
        let now = Instant::now();
        let (step, finished) = match &mut animation {
            Animation::Wheel { remaining } => {
                if remaining.abs() <= 0.1 { (*remaining, true) }
                else { let step = *remaining * 0.12; *remaining *= 0.88; (step, false) }
            }
            Animation::Glide { distance, duration, started, eased, curve } => {
                let progress = (now.duration_since(*started).as_secs_f32() / duration.as_secs_f32()).min(1.0);
                let next = match curve { Curve::Quadratic => progress * (2.0 - progress), Curve::Cubic => 1.0 - (1.0 - progress).powi(3) };
                let step = *distance * (next - *eased); *eased = next;
                (step, progress >= 1.0)
            }
        };
        // Apply deltas to the current anchor, so a history refresh cannot jump
        // back to an obsolete absolute offset midway through an animation.
        let moved = self.set_scroll_position(self.scroll_position() + step);
        // Keep decaying wheel momentum at the loaded edge: pagination can add
        // the next page before that momentum is spent. A drag glide has an
        // already clamped destination and ends when it reaches the edge.
        if !finished && (matches!(&animation, Animation::Wheel { .. }) || (moved - step).abs() < 0.01) {
            self.scrolling.animation = Some(animation); window.request_animation_frame();
        }
    }
    pub(super) fn scroll_events(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        // A paint-only canvas adds window capture listeners without a hitbox
        // over the controls. Moving across virtualized cards keeps one gesture.
        canvas(|_, _, _| (), move |_, _, window, _| {
            let movement = owner.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.capture() { let _ = movement.update(cx, |this, cx| this.drag_move(event, window, cx)); }
            });
            let release = owner.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase.capture() { let _ = release.update(cx, |this, cx| this.drag_end(event, window, cx)); }
            });
            let wheel = owner.clone();
            window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
                if phase.capture() { let _ = wheel.update(cx, |this, cx| this.wheel_scroll(event, window, cx)); }
            });
        }).absolute().inset_0().into_any_element()
    }
}
