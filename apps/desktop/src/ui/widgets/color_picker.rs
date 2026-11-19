// Adapted from Super Paste ColorPicker.vue (MIT); see THIRD-PARTY-NOTICES.md.
use std::{cell::Cell, rc::Rc};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, PopoverState, Positioner, Scrollbar, ScrollbarMode, Slider, SliderIndicator, SliderThumb, SliderTrack, actions::Cancel, slider::{SliderEvent, SliderState}};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use super::{color_motion, control_feedback, motion, pointer_state, shadow};
use crate::ui::{icon, palette::Palette};

const CONTEXT: &str = "SuperPasteColorPicker";
const INVALID_COLOR: &str = "请输入有效颜色（例如 #3B82F6）";

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", NoAction, Some(CONTEXT)),
        KeyBinding::new("space", NoAction, Some(CONTEXT)),
    ]);
}

/// RGB is the selected value; the input owns its uncommitted draft. Native
/// HSL components retain source precision; native sliders own interaction and
/// display. Changing one component does not quantize the other two.
pub struct ColorPickerState {
    value: Option<u32>,
    hex: SharedString,
    components: [f64; 3],
    input: Entity<InputState>,
    channels: [Entity<SliderState>; 3],
    channel_focus: [FocusHandle; 3],
    trigger_focus: FocusHandle,
    popover: Entity<PopoverState>,
    open: bool,
    invalid: bool,
    skip_blur: bool,
    panel_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl ColorPickerState {
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = parse_rgb(text);
        let hex: SharedString = value.map(|value| format!("#{value:06x}")).unwrap_or_else(|| text.to_ascii_lowercase()).into();
        // Invalid colors stay invalid. Initial slider positions only reproduce
        // the reference's empty-picker presentation; they never become a value.
        let components = value.map(rgb_to_hsl).unwrap_or([0.0, 72.0, 52.0]);
        let input = cx.new(|cx| InputState::new(window, cx).max_utf16_length(7).default_value(hex.to_ascii_uppercase()));
        let channels = [(360.0, components[0]), (100.0, components[1]), (100.0, components[2])]
            .map(|(max, value)| cx.new(|_| SliderState::new().min(0.0).max(max).step(1.0).default_value(value as f32)));
        let channel_focus: [FocusHandle; 3] = std::array::from_fn(|_| cx.focus_handle());
        let popover = cx.new(|cx| PopoverState::new(false, cx));
        let owner = cx.entity().downgrade();
        popover.update(cx, |state, _| {
            state.track_focus(Some(channel_focus[0].clone()));
            // Native dismissal already updated the popover. Do not reborrow
            // it from this callback; just finish the application's close.
            state.set_on_open_change(Some(Rc::new(move |open, window, cx| {
                if !open { let _ = owner.update(cx, |this, cx| this.finish_close(false, window, cx)); }
            })));
        });
        let panel_focus = popover.focus_handle(cx);
        let mut subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if !this.open { return; }
                match event {
                    InputEvent::Change => { this.skip_blur = false; if this.invalid { this.invalid = false; cx.notify(); } }
                    InputEvent::Focus => { this.skip_blur = false; cx.notify(); }
                    InputEvent::PressEnter { .. } => { if !this.is_composing(cx) { this.commit_draft(window, cx); } }
                    InputEvent::Blur => { if !std::mem::take(&mut this.skip_blur) { this.commit_draft(window, cx); } }
                }
            }),
            cx.on_focus_out(&panel_focus, window, |this, _, window, cx| {
                if this.open && !this.trigger_focus.is_focused(window) { this.close(false, window, cx); }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() { this.close(false, window, cx); }
            }),
            cx.observe_window_visibility(window, |this, visibility, window, cx| {
                if visibility == WindowVisibility::Hidden { this.close(false, window, cx); }
            }),
        ];
        subscriptions.extend(channels.iter().enumerate().map(|(index, channel)| cx.subscribe_in(channel, window, move |this, _, event: &SliderEvent, window, cx| {
            if let SliderEvent::Change(value) = event { if this.open {
                this.components[index] = f64::from(value.start());
                // Do not write the sliders back through rounded RGB: that
                // would move another component or fight an active drag.
                this.write_color(hsl_to_rgb(this.components), window, cx);
            } }
        })));
        Self { value, hex, components, input, channels, channel_focus,
            trigger_focus: cx.focus_handle(), popover, open: false, invalid: false, skip_blur: false,
            panel_bounds: Rc::new(Cell::new(None)), scroll: ScrollHandle::new(), _subscriptions: subscriptions }
    }
    pub fn value(&self) -> Option<u32> { self.value }
    pub fn hex(&self) -> SharedString { self.hex.clone() }
    pub fn is_composing(&self, cx: &App) -> bool { self.input.read(cx).is_composing() }
    pub fn set_value(&mut self, value: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.components = rgb_to_hsl(value);
        for (channel, value) in self.channels.iter().zip(self.components) {
            channel.update(cx, |channel, cx| channel.set_value(value as f32, window, cx));
        }
        self.write_color(value, window, cx);
    }
    fn write_color(&mut self, value: u32, window: &mut Window, cx: &mut Context<Self>) {
        if self.value != Some(value) { self.value = Some(value); self.hex = format!("#{value:06x}").into(); }
        self.invalid = false;
        self.input.update(cx, |input, cx| input.set_value(format!("#{value:06X}"), window, cx));
        cx.notify();
    }
    fn commit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.input.read(cx).value();
        match parse_rgb(&draft) {
            Some(value) => self.set_value(value, window, cx),
            None => { if !self.invalid { self.invalid = true; cx.notify(); } }
        }
    }
    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open { self.close(false, window, cx); return; }
        self.open = true;
        self.popover.update(cx, |state, cx| state.sync_open(true, window, cx));
        cx.notify();
    }
    pub fn close(&mut self, discard_draft: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open { return; }
        self.finish_close(discard_draft, window, cx);
        self.popover.update(cx, |state, cx| state.sync_open(false, window, cx));
        if discard_draft { self.trigger_focus.focus(window, cx); }
    }
    fn finish_close(&mut self, discard_draft: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open { return; }
        // Clear open before focus restoration can enqueue a field Blur.
        self.open = false;
        if !discard_draft {
            let draft = self.input.read(cx).value();
            if let Some(value) = parse_rgb(&draft) { self.set_value(value, window, cx); }
        }
        for channel in &self.channels { channel.update(cx, |channel, cx| channel.cancel_interaction(cx)); }
        self.invalid = false;
        self.skip_blur = false;
        self.panel_bounds.set(None);
        self.scroll.set_offset(Point::default());
        self.input.update(cx, |input, cx| input.set_value(self.hex.to_ascii_uppercase(), window, cx));
        cx.notify();
    }
}

fn parse_rgb(value: &str) -> Option<u32> {
    let value = value.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    if !matches!(hex.len(), 3 | 6) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(if hex.len() == 3 { ((value & 0xf00) << 8 | (value & 0xf0) << 4 | (value & 0xf)) * 17 } else { value })
}
fn rgb_to_hsl(value: u32) -> [f64; 3] {
    let [r, g, b] = [(value >> 16) & 255, (value >> 8) & 255, value & 255].map(|value| f64::from(value) / 255.0);
    let max = r.max(g).max(b); let min = r.min(g).min(b);
    let delta = max - min; let l = (max + min) / 2.0;
    if delta == 0.0 { return [0.0, 0.0, l * 100.0]; }
    let s = delta / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r { ((g - b) / delta) % 6.0 } else if max == g { (b - r) / delta + 2.0 } else { (r - g) / delta + 4.0 } * 60.0;
    [if h < 0.0 { h + 360.0 } else { h }, s * 100.0, l * 100.0]
}
fn hsl_to_rgb([h, s, l]: [f64; 3]) -> u32 {
    let s = s / 100.0;
    let l = l / 100.0;
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let segment = h / 60.0;
    let secondary = chroma * (1.0 - ((segment % 2.0) - 1.0).abs());
    let channels = if segment < 1.0 { [chroma, secondary, 0.0] } else if segment < 2.0 { [secondary, chroma, 0.0] }
        else if segment < 3.0 { [0.0, chroma, secondary] } else if segment < 4.0 { [0.0, secondary, chroma] }
        else if segment < 5.0 { [secondary, 0.0, chroma] } else { [chroma, 0.0, secondary] };
    let offset = l - chroma / 2.0;
    let [r, g, b] = channels.map(|value| ((value + offset) * 255.0).round().clamp(0.0, 255.0) as u32);
    r << 16 | g << 8 | b
}

#[derive(IntoElement)]
pub struct ColorPicker { state: Entity<ColorPickerState>, palette: Palette, disabled: bool }
impl ColorPicker {
    pub fn new(state: &Entity<ColorPickerState>, palette: Palette) -> Self { Self { state: state.clone(), palette, disabled: false } }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
}
impl RenderOnce for ColorPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.disabled { self.state.update(cx, |state, cx| state.close(false, window, cx)); }
        let state = self.state.read(cx);
        let p = self.palette;
        let id: ElementId = ("color-picker", self.state.entity_id()).into();
        let focus = state.trigger_focus.clone();
        let value = state.value;
        let open = state.open;
        let interaction = pointer_state(&id, self.disabled, window, cx);
        let hovered = interaction.read(cx).hovered && !self.disabled;
        let pressed = interaction.read(cx).pressed() && !self.disabled;
        let border = color_motion(&id, "border", if hovered { p.primary } else { p.control_border }, window, cx);
        let background = color_motion(&id, "background", if hovered { p.bg_tertiary } else { p.bg_primary }, window, cx);
        let offset = motion(&id, "press", if pressed { 1.0_f32 } else { 0.0 }, 100, window, cx);
        let owner = self.state.downgrade();
        let trigger = control_feedback(Button::new("trigger").track_focus(&focus).disabled(self.disabled)
            .accessibility_label("自定义颜色").aria_expanded(open)
            .w(px(40.0)).h(px(28.0)).p(px(3.0)).gap(px(4.0)).flex().items_center().justify_center()
            .border_1().border_color(border).rounded(px(4.0)).bg(background)
            .opacity(if self.disabled { 0.5 } else { 1.0 })
            .cursor(if self.disabled { CursorStyle::OperationNotAllowed } else { CursorStyle::PointingHand })
            .focus_visible(move |style| style.border_color(p.primary).bg(p.bg_tertiary))
            .child(div().flex_1().h_full().min_w_0().border_1().border_color(black().opacity(0.14)).rounded(px(2.0)).when_some(value, |swatch, value| swatch.bg(rgb(value))))
            .child(icon("chevron-down", 12.0).text_color(p.text_secondary))
            .on_click(move |_, window, cx| { let _ = owner.update(cx, |state, cx| state.toggle(window, cx)); }), interaction, focus, self.disabled);
        let owner = self.state.downgrade();
        let scroll_owner = owner.clone();
        div().id(id).relative().top(px(offset)).flex().flex_shrink_0().child(trigger)
            .when(open, |root| root.child(canvas(move |bounds, window, cx| {
                let bounds = window.to_window_bounds(bounds);
                let _ = owner.update(cx, |state, cx| {
                    let viewport = window.viewport_size();
                    if !state.open { return; }
                    if bounds.bottom() <= px(0.0) || bounds.top() >= viewport.height || bounds.right() <= px(0.0) || bounds.left() >= viewport.width {
                        state.close(false, window, cx); return;
                    }
                    let below = (viewport.height - bounds.bottom() - px(12.0)).max(px(0.0));
                    let above = (bounds.top() - px(12.0)).max(px(0.0));
                    // The normal panel is 180px tall; errors can only add
                    // height. Match the reference's 180px flip threshold.
                    let upwards = below < px(180.0) && above > below;
                    let available = if upwards { above } else { below };
                    if available <= px(0.0) { state.close(false, window, cx); return; }
                    let width = px(268.0).min((viewport.width - px(16.0)).max(px(0.0)));
                    let anchor = if upwards { Anchor::BottomLeft } else { Anchor::TopLeft };
                    let position = point(bounds.left(), if upwards { bounds.top() - px(4.0) } else { bounds.bottom() + px(4.0) });
                    let measured = state.panel_bounds.clone();
                    let panel = render_panel(cx.entity().clone(), state, bounds, width, available, p, window, cx);
                    let mut popup = Positioner::corner(anchor, position).margin(px(8.0)).occlude()
                        .on_position(move |position| measured.set(Some(position.bounds))).child(panel).into_any_element();
                    // Measure after the real trigger bounds are known, then
                    // let native deferred drawing retain dispatch/focus ancestry.
                    popup.layout_as_root(size(AvailableSpace::MaxContent, AvailableSpace::MaxContent), window, cx);
                    window.defer_draw(popup, point(px(0.0), px(0.0)), gpui_kit::base::POPUP_PRIORITY, None);
                });
            }, move |_, _, window, _| {
                window.on_mouse_event::<ScrollEvent>(move |event, phase, window, cx| {
                    if !phase.capture() { return; }
                    let _ = scroll_owner.update(cx, |state, cx| {
                        if state.open && !state.panel_bounds.get().is_some_and(|bounds| bounds.contains(&event.position)) { state.close(false, window, cx); }
                    });
                });
            }).absolute().top_0().left_0().size_full()))
    }
}

fn render_panel(owner: Entity<ColorPickerState>, state: &ColorPickerState, trigger_bounds: Bounds<Pixels>, width: Pixels, available: Pixels, p: Palette, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let weak = owner.downgrade();
    let cancel = weak.clone();
    let input_focused = state.input.read(cx).focus_handle(cx).is_focused(window);
    let fields = ["色相", "饱和度", "亮度"].into_iter().enumerate().map(|(index, label)| {
        field(label, p).child(ColorChannel { owner: owner.downgrade(), state: state.channels[index].clone(), focus: state.channel_focus[index].clone(), hue: index == 0, label, palette: p })
    }).collect::<Vec<_>>();
    div().id("color-panel").role(Role::Dialog).aria_label("自定义颜色").tab_group().track_focus(&state.popover.focus_handle(cx))
        .key_context(CONTEXT).on_action(move |_: &Cancel, window, cx| {
            cx.stop_propagation(); let _ = cancel.update(cx, |state, cx| state.close(true, window, cx));
        })
        .on_mouse_down_out(move |event, window, cx| {
            if !trigger_bounds.contains(&event.position) { let _ = weak.update(cx, |state, cx| state.close(false, window, cx)); }
        })
        .relative().w(width).max_h(available).overflow_y_scroll().track_scroll(&state.scroll).p(px(12.0)).border_1().border_color(p.control_border).rounded(px(6.0))
        .bg(p.bg_elevated).text_color(p.text_primary).text_size(px(12.0)).line_height(relative(1.4)).font_weight(FontWeight::NORMAL)
        .shadow(vec![shadow(black().opacity(if p.dark { 0.36 } else { 0.12 }), 0.0, 8.0, 2.0), shadow(black().opacity(if p.dark { 0.24 } else { 0.08 }), 0.0, 3.0, 1.0)])
        .flex().flex_col().gap(px(12.0))
        .child(div().flex().flex_shrink_0().items_center().gap(px(9.0))
            .child(div().size(px(28.0)).flex_shrink_0().border_1().border_color(p.border_medium).rounded(px(4.0)).when_some(state.value, |swatch, value| swatch.bg(rgb(value))))
            .child(div().font_weight(FontWeight::BOLD).child(state.hex.clone())))
        .children(fields)
        .child(field("十六进制", p).child(Input::new(&state.input).text_direction(gpui::TextDirection::Ltr).aria_label("十六进制颜色").aria_invalid(state.invalid)
            .when(state.invalid, |input| input.aria_description(INVALID_COLOR)).appearance(false).focus_bordered(false)
            .flex_1().min_w_0().h_auto().min_h(px(30.0)).px(px(7.0)).py(px(5.0)).border_1().rounded(px(4.0)).bg(p.bg_primary)
            .border_color(if state.invalid { p.danger } else if input_focused { p.primary } else { p.control_border })
            .text_size(px(12.0)).line_height(relative(1.4)).font_family("monospace").text_color(p.text_primary)))
        .when(state.invalid, |panel| panel.child(div().id("color-error").role(Role::Alert).flex_shrink_0().mt(px(6.0)).text_size(px(11.0)).text_color(p.danger).child(INVALID_COLOR)))
        .child(Scrollbar::vertical(&state.scroll).mode(ScrollbarMode::Always).styles(move |styles| styles
            .track(|track| track.width(px(6.0)).bg(transparent_black()).border_color(transparent_black()))
            .track_hover(|track| track.width(px(6.0)).bg(transparent_black()).border_color(transparent_black()))
            .track_active(|track| track.width(px(6.0)).bg(transparent_black()).border_color(transparent_black()))
            .thumb(|thumb| thumb.width(px(6.0)).inset(px(0.0)).radius(px(999.0)).bg(p.scrollbar))
            .thumb_hover(|thumb| thumb.width(px(6.0)).bg(p.scrollbar_hover))
            .thumb_active(|thumb| thumb.width(px(6.0)).bg(p.scrollbar_hover))))
}
fn field(label: &'static str, p: Palette) -> Div {
    div().flex().flex_shrink_0().items_center().gap(px(8.0)).min_w_0().text_size(px(11.0)).line_height(relative(1.4))
        .text_color(p.text_secondary)
        .child(div().w(px(64.0)).flex_shrink_0().child(label))
}

#[derive(IntoElement)]
struct ColorChannel { owner: WeakEntity<ColorPickerState>, state: Entity<SliderState>, focus: FocusHandle, hue: bool, label: &'static str, palette: Palette }
impl RenderOnce for ColorChannel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let percentage = self.state.read(cx).percentage().end;
        let owner = self.owner;
        let track = div().absolute().left_0().right_0().h(px(5.0)).rounded_full().overflow_hidden().bg(p.control_border)
            .when(self.hue, |track| track.flex().children([0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xff0000]
                .windows(2).map(|colors| div().flex_1().h_full().bg(linear_gradient(90.0, linear_color_stop(rgb(colors[0]), 0.0), linear_color_stop(rgb(colors[1]), 1.0)))).collect::<Vec<_>>()));
        Slider::new(&self.state).horizontal().track_focus(&self.focus).accessibility_label(self.label)
            .capture_any_mouse_down(move |event, window, cx| {
                if event.button != MouseButton::Left { return; }
                let _ = owner.update(cx, |state, cx| {
                    // Apply the draft before native pointer handling emits a
                    // slider change; a queued Blur must not undo that change.
                    if state.input.read(cx).focus_handle(cx).is_focused(window) {
                        state.commit_draft(window, cx); state.skip_blur = true;
                    }
                });
            })
            .flex_1().min_w_0().h(px(16.0)).cursor_pointer()
            .focus_visible(move |style| style.shadow(vec![shadow(p.primary, 4.0, 0.0, 0.0), shadow(p.bg_elevated, 2.0, 0.0, 0.0)]))
            .child(SliderTrack::new(&self.state).axis(Axis::Horizontal).size_full().relative().flex().items_center().child(track)
                .child(SliderIndicator::new(&self.state).relative().flex_1().mx(px(7.0)).h(px(5.0))
                    .child(SliderThumb::new(&self.state).absolute().left(relative(percentage)).ml(px(-7.0)).top(px(-4.5)).size(px(14.0))
                        .rounded_full().border_2().border_color(p.bg_primary).bg(p.primary).shadow(vec![shadow(p.primary, 1.0, 0.0, 0.0)]))))
    }
}
