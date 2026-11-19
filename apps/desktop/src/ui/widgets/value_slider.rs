use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, Slider, SliderIndicator, SliderThumb, SliderTrack, slider::SliderState};
use super::{Click, channel, color_motion, control_feedback, control_state, motion, pointer_feedback, pointer_state, shadow};
use crate::ui::palette::Palette;

#[derive(IntoElement)]
pub struct ValueSlider { id: ElementId, state: Entity<SliderState>, label: SharedString, palette: Palette, accessibility_value: Option<f32> }
impl ValueSlider {
    pub fn new(id: impl Into<ElementId>, state: &Entity<SliderState>, label: impl Into<SharedString>, palette: Palette) -> Self {
        Self { id: id.into(), state: state.clone(), label: label.into(), palette, accessibility_value: None }
    }
    pub fn accessibility_value(mut self, value: f32) -> Self {
        self.accessibility_value = Some(value); self
    }
}
impl RenderOnce for ValueSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let interaction = pointer_state(&self.id, false, window, cx);
        let focus = window.use_keyed_state(channel(&self.id, "focus"), cx, |_, cx| cx.focus_handle()).read(cx).clone();
        if !window.is_window_active() && self.state.read(cx).is_interacting() {
            self.state.update(cx, |state, cx| state.cancel_interaction(cx));
        }
        let state = interaction.read(cx).value;
        let pressed = state.pressed || self.state.read(cx).is_interacting();
        let background = color_motion(&self.id, "thumb-background", if pressed { p.primary_active } else if state.hovered { p.primary_hover } else { p.primary }, window, cx);
        let ring = motion(&self.id, "thumb-ring", if pressed { 5.0_f32 } else if state.hovered { 4.0 } else { 1.0 }, 100, window, cx);
        let ring_color = Hsla::from(color_motion(&self.id, "thumb-ring-color", if pressed || state.hovered { p.selection } else { p.primary }, window, cx));
        let percentage = self.state.read(cx).percentage().end;
        pointer_feedback(div().id(self.id).h(px(24.0)).mx(px(8.0)).cursor_pointer(), interaction, false)
            .child(Slider::new(&self.state).horizontal().accessibility_label(self.label).track_focus(&focus).size_full()
                .when_some(self.accessibility_value, |slider, value| slider.accessibility_value(value))
                .focus_visible(move |style| style.shadow(vec![shadow(p.primary, 6.0, 0.0, 0.0), shadow(p.bg_primary, 4.0, 0.0, 0.0)]))
                .child(SliderTrack::new(&self.state).axis(Axis::Horizontal).size_full().flex().items_center()
                    .child(SliderIndicator::new(&self.state).relative().flex_1().min_w_0().mx(px(8.0)).h(px(4.0)).rounded_full().bg(p.neutral_300)
                        .child(div().absolute().left_0().top_0().bottom_0().w(relative(percentage)).rounded_full().bg(p.primary))
                        .child(SliderThumb::new(&self.state).absolute().top(px(-6.0)).left(relative(percentage)).ml(px(-8.0)).size(px(16.0))
                            .rounded_full().border_2().border_color(p.bg_primary).bg(background).shadow(vec![shadow(ring_color, ring, 0.0, 0.0)])
                            .cursor(if pressed { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })))))
    }
}

#[derive(IntoElement)]
pub struct SliderMark {
    id: ElementId, label: SharedString, fraction: f32, selected: bool, palette: Palette, on_click: Option<Click>,
}
impl SliderMark {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, fraction: f32, selected: bool, palette: Palette) -> Self {
        Self { id: id.into(), label: label.into(), fraction, selected, palette, on_click: None }
    }
    pub fn on_click(mut self, action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(action)); self
    }
}
impl RenderOnce for SliderMark {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let (interaction, focus) = control_state(&self.id, false, window, cx);
        let state = interaction.read(cx).value;
        let foreground = color_motion(&self.id, "foreground", if self.selected || state.hovered { p.primary } else { p.text_secondary }, window, cx);
        let background = if state.pressed() || (self.selected && state.hovered) { p.selection_hover }
            else if self.selected || state.hovered { p.selection } else { transparent_black() };
        let background = color_motion(&self.id, "label-background", background, window, cx);
        let callback = self.on_click;
        // A zero-width flex anchor centers the intrinsic label box on the same
        // travel interval as the thumb, without measuring or fixing text widths.
        div().absolute().top_0().left(relative(self.fraction)).w(px(0.0)).flex().justify_center()
            .child(control_feedback(Button::new(self.id).track_focus(&focus).accessibility_label(self.label.clone()).selected(self.selected)
                .aria_toggled(if self.selected { Toggled::True } else { Toggled::False })
                .relative().flex_shrink_0().pt(px(8.0)).rounded(px(4.0)).text_size(px(10.0)).line_height(px(16.0))
                .text_color(foreground).whitespace_nowrap().cursor_pointer()
                .focus_visible(move |style| style.shadow(vec![shadow(p.primary, 4.0, 0.0, 0.0), shadow(p.bg_primary, 2.0, 0.0, 0.0)]))
                .child(div().absolute().top_0().left(relative(0.5)).ml(px(-1.0)).w(px(2.0)).h(px(4.0)).rounded(px(1.0))
                    .bg(if self.selected || state.hovered { p.primary } else { p.control_border }))
                .child(div().px(px(3.0)).py(px(1.0)).rounded(px(4.0)).bg(background).child(self.label))
                .on_click(move |event, window, cx| { if let Some(callback) = &callback { callback(event, window, cx); } }), interaction, focus, false))
    }
}
