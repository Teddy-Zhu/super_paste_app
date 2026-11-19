use std::{rc::Rc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, Easing, PremultipliedColor, Switch, Transition, transition};
use super::palette::Palette;
mod position;
mod value_slider;
mod icon_button;
mod color_picker;
mod date_input;
pub use color_picker::{ColorPicker, ColorPickerState};
pub use date_input::{DateInput, DateInputChanged, DateInputState};
pub use icon_button::{ColorSwatch, IconButton};
pub use position::PositionSelector;
pub use value_slider::{SliderMark, ValueSlider};

pub fn init(cx: &mut App) { color_picker::init(cx); }

pub fn thin_scrollbar_styles(p: Palette) -> gpui_kit::base::ScrollbarStyles {
    gpui_kit::base::ScrollbarStyles::default()
        .track(|track| track.width(px(6.0)).bg(transparent_black()).border_color(transparent_black()))
        .track_hover(|track| track.width(px(6.0)).bg(transparent_black()).border_color(transparent_black()))
        .track_active(|track| track.width(px(6.0)).bg(transparent_black()).border_color(transparent_black()))
        .thumb(move |thumb| thumb.width(px(6.0)).inset(px(0.0)).radius(px(3.0)).bg(p.scrollbar))
        .thumb_hover(move |thumb| thumb.width(px(6.0)).inset(px(0.0)).radius(px(3.0)).bg(p.scrollbar_hover))
        .thumb_active(move |thumb| thumb.width(px(6.0)).inset(px(0.0)).radius(px(3.0)).bg(p.scrollbar_hover))
}

pub fn spinner(id: impl Into<ElementId>, p: Palette, window: &Window) -> Div {
    spinner_sized(id, p, 24.0, window)
}
pub fn spinner_sized(id: impl Into<ElementId>, p: Palette, size: f32, window: &Window) -> Div {
    let arc = svg().path(if size <= 12.0 { "superpaste/feedback/loading-arc-small.svg" } else { "superpaste/feedback/loading-arc.svg" }).size(px(size)).text_color(p.primary);
    let arc = if window.is_visible() {
        arc.with_animation(id, Animation::new(Duration::from_secs(1)).repeat().with_easing(|value| value), |arc, value| arc.with_transformation(Transformation::rotate(percentage(value)))).into_any_element()
    } else { arc.into_any_element() };
    div().relative().size(px(size)).border_2().border_color(p.border_medium).rounded_full()
        .child(div().absolute().top(px(-2.0)).left(px(-2.0)).size(px(size)).child(arc))
}

#[derive(Clone, Copy, Default)]
struct Interaction { hovered: bool, pressed: bool, key_pressed: bool }
impl Interaction {
    fn pressed(self) -> bool { self.pressed || self.key_pressed }
}
struct InteractionState { value: Interaction, _activation: Subscription, _visibility: Subscription }
impl InteractionState {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() { this.cancel_press(cx); }
        });
        let visibility = cx.observe_window_visibility(window, |this, visibility, _, cx| {
            if visibility == WindowVisibility::Hidden { this.cancel_press(cx); }
        });
        Self { value: Interaction::default(), _activation: activation, _visibility: visibility }
    }
    fn cancel_press(&mut self, cx: &mut Context<Self>) {
        if self.value.pressed() {
            self.value.pressed = false; self.value.key_pressed = false; cx.notify();
        }
    }
}
impl std::ops::Deref for InteractionState {
    type Target = Interaction;
    fn deref(&self) -> &Self::Target { &self.value }
}
impl std::ops::DerefMut for InteractionState {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.value }
}
fn channel(id: &ElementId, name: &'static str) -> ElementId { ElementId::NamedChild(id.clone().into(), name.into()) }
fn pointer_state(id: &ElementId, disabled: bool, window: &mut Window, cx: &mut App) -> Entity<InteractionState> {
    let interaction = window.use_keyed_state(channel(id, "interaction"), cx, InteractionState::new);
    if (disabled || !window.is_window_active()) && interaction.read(cx).pressed() {
        interaction.update(cx, |state, cx| {
            state.pressed = false; state.key_pressed = false; cx.notify();
        });
    }
    interaction
}
fn control_state(id: &ElementId, disabled: bool, window: &mut Window, cx: &mut App) -> (Entity<InteractionState>, FocusHandle) {
    let interaction = pointer_state(id, disabled, window, cx);
    let focus = window.use_keyed_state(channel(id, "focus"), cx, |_, cx| cx.focus_handle()).read(cx).clone();
    if interaction.read(cx).key_pressed && !focus.is_focused(window) {
        interaction.update(cx, |state, cx| { state.key_pressed = false; cx.notify(); });
    }
    (interaction, focus)
}
fn pointer_feedback<T: StatefulInteractiveElement>(element: T, interaction: Entity<InteractionState>, disabled: bool) -> T {
    let hover = interaction.clone();
    let down = interaction.clone();
    let up = interaction.clone();
    element.hover_listener_mode(HoverListenerMode::PointerPresence)
        .on_hover(move |value, _, cx| hover.update(cx, |state, cx| {
            if state.hovered != *value { state.hovered = *value; cx.notify(); }
        }))
        .capture_any_mouse_down(move |event, _, cx| {
            if event.button == MouseButton::Left && !disabled { down.update(cx, |state, cx| {
                if !state.pressed { state.pressed = true; cx.notify(); }
            }); }
        })
        // Release before a child can stop bubbling, including outside the hitbox.
        .capture_any_mouse_up(move |event, _, cx| {
            if event.button == MouseButton::Left { up.update(cx, |state, cx| {
                if state.pressed { state.pressed = false; cx.notify(); }
            }); }
        })
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| interaction.update(cx, |state, cx| {
            if state.pressed { state.pressed = false; cx.notify(); }
        }))
}
fn control_feedback<T: StatefulInteractiveElement>(element: T, interaction: Entity<InteractionState>, focus: FocusHandle, disabled: bool) -> T {
    let key_down = interaction.clone();
    let key_up = interaction.clone();
    // Observe native activation; Base still owns the click and focus behavior.
    pointer_feedback(element, interaction, disabled)
        .capture_key_down(move |event, window, cx| {
            let pressed = !disabled && focus.is_focused(window) && !window.default_prevented()
                && !event.keystroke.modifiers.modified()
                && matches!(event.keystroke.key.as_str(), "enter" | "space");
            key_down.update(cx, |state, cx| {
                if state.key_pressed != pressed { state.key_pressed = pressed; cx.notify(); }
            });
        })
        .capture_key_up(move |_, _, cx| key_up.update(cx, |state, cx| {
            if state.key_pressed { state.key_pressed = false; cx.notify(); }
        }))
}
pub(crate) fn motion<T: gpui_kit::base::Interpolate + PartialEq + 'static>(id: &ElementId, name: &'static str, target: T, milliseconds: u64, window: &mut Window, cx: &mut App) -> T {
    transition(channel(id, name), target, Transition::new(Duration::from_millis(milliseconds)).easing(Easing::Ease), window, cx)
}
pub(crate) fn color_motion(id: &ElementId, name: &'static str, target: Hsla, window: &mut Window, cx: &mut App) -> Rgba {
    motion(id, name, PremultipliedColor(target.into()), 100, window, cx).0
}
fn shadow(color: Hsla, spread: f32, blur: f32, y: f32) -> BoxShadow {
    BoxShadow { color, offset: point(px(0.0), px(y)), blur_radius: px(blur), spread_radius: px(spread), inset: false }
}
pub fn control_shadow(p: Palette) -> Vec<BoxShadow> {
    let mut inset = shadow(white().opacity(if p.dark { 0.04 } else { 0.8 }), 0.0, 0.0, 1.0);
    inset.inset = true;
    vec![shadow(if p.dark { black().opacity(0.25) } else { Hsla::from(rgb(0x1c2636)).opacity(0.05) }, 0.0, if p.dark { 2.0 } else { 1.0 }, 1.0), inset]
}

type Click = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
#[derive(IntoElement)]
pub struct ToolbarButton {
    id: ElementId, label: SharedString, icon: AnyElement, palette: Palette,
    expanded: bool, on_click: Option<Click>,
}
impl ToolbarButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, icon: impl IntoElement, palette: Palette) -> Self {
        Self { id: id.into(), label: label.into(), icon: icon.into_any_element(), palette, expanded: false, on_click: None }
    }
    pub fn expanded(mut self, expanded: bool) -> Self { self.expanded = expanded; self }
    pub fn on_click(mut self, action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self { self.on_click = Some(Rc::new(action)); self }
}
impl RenderOnce for ToolbarButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let (interaction, focus) = control_state(&self.id, false, window, cx);
        let state = interaction.read(cx).value;
        let pressed = state.pressed();
        let focused = focus.is_focused(window) && window.last_input_was_keyboard();
        let background = if pressed || self.expanded { p.control_active }
            else if state.hovered || focused { p.control_hover } else { transparent_black() };
        let foreground = if state.hovered || pressed || focused || self.expanded { p.text_primary } else { p.text_secondary };
        let background = color_motion(&self.id, "background", background, window, cx);
        let foreground = color_motion(&self.id, "foreground", foreground, window, cx);
        let border = color_motion(&self.id, "border", if focused { p.primary } else { transparent_black() }, window, cx);
        let offset = motion(&self.id, "press", if pressed { 1.0_f32 } else { 0.0 }, 100, window, cx);
        let callback = self.on_click;
        control_feedback(Button::new(self.id).accessibility_label(self.label).track_focus(&focus)
            .aria_expanded(self.expanded).selected(self.expanded)
            .relative().top(px(offset)).size(px(28.0)).flex_shrink_0().p_0().rounded(px(4.0))
            .flex().items_center().justify_center()
            .border_1().border_color(border).bg(background).text_color(foreground).cursor_pointer()
            .child(self.icon).on_click(move |event, window, cx| {
                if let Some(callback) = &callback { callback(event, window, cx); }
            }), interaction, focus, false)
    }
}

#[derive(IntoElement)]
pub struct TextButton {
    id: ElementId, label: SharedString, icon: Option<&'static str>, palette: Palette,
    kind: ButtonKind, disabled: bool, disabled_shadow: bool, on_click: Option<Click>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonKind { Accent, Neutral, Primary, Danger, Preset(bool), Form(bool) }
impl TextButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, palette: Palette) -> Self {
        Self { id: id.into(), label: label.into(), icon: None, palette, kind: ButtonKind::Accent, disabled: false, disabled_shadow: true, on_click: None }
    }
    pub fn icon(mut self, icon: &'static str) -> Self { self.icon = Some(icon); self }
    pub fn danger(mut self) -> Self { self.kind = ButtonKind::Danger; self }
    pub fn primary(mut self) -> Self { self.kind = ButtonKind::Primary; self }
    pub fn neutral(mut self) -> Self { self.kind = ButtonKind::Neutral; self }
    pub fn preset(mut self, selected: bool) -> Self { self.kind = ButtonKind::Preset(selected); self }
    pub fn form(mut self, primary: bool) -> Self { self.kind = ButtonKind::Form(primary); self }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
    pub fn disabled_shadow(mut self, enabled: bool) -> Self { self.disabled_shadow = enabled; self }
    pub fn on_click(mut self, action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self { self.on_click = Some(Rc::new(action)); self }
}
impl RenderOnce for TextButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let (interaction, focus) = control_state(&self.id, self.disabled, window, cx);
        let state = interaction.read(cx).value;
        let hovered = state.hovered && !self.disabled;
        let pressed = state.pressed() && !self.disabled;
        let form = matches!(self.kind, ButtonKind::Form(_));
        let primary = self.kind == ButtonKind::Primary || self.kind == ButtonKind::Form(true);
        let danger = self.kind == ButtonKind::Danger;
        let neutral = self.kind == ButtonKind::Neutral || self.kind == ButtonKind::Form(false);
        let preset = matches!(self.kind, ButtonKind::Preset(_));
        let selected = self.kind == ButtonKind::Preset(true);
        let background = if preset {
            if selected { if hovered { p.primary_hover } else { p.primary } }
            else if hovered { p.selection } else { p.bg_primary }
        } else if primary {
            if pressed && !form { p.primary_active } else if hovered { p.primary_hover } else { p.primary }
        } else if hovered { if danger { p.danger_bg } else if neutral { p.control_hover } else { p.selection } } else { p.bg_primary };
        let border = if preset {
            if selected { transparent_black() } else if hovered { p.primary } else { p.border_medium }
        } else if primary { if form { transparent_black() } else { background } }
        else if hovered { if danger { p.danger_border } else if neutral { p.control_border } else { p.primary } } else { p.control_border };
        let background = color_motion(&self.id, "background", background, window, cx);
        let border = color_motion(&self.id, "border", border, window, cx);
        let foreground = color_motion(&self.id, "foreground", match self.kind {
            ButtonKind::Primary => white(), ButtonKind::Danger => p.danger,
            ButtonKind::Neutral => p.text_primary, ButtonKind::Accent => p.primary,
            ButtonKind::Preset(true) => p.text_inverse,
            ButtonKind::Preset(false) => if hovered { p.primary } else { p.text_secondary },
            ButtonKind::Form(true) => white(), ButtonKind::Form(false) => p.text_primary,
        }, window, cx);
        let offset = motion(&self.id, "press", if pressed { 1.0_f32 } else { 0.0 }, 100, window, cx);
        let callback = self.on_click;
        control_feedback(Button::new(self.id).disabled(self.disabled).track_focus(&focus).relative().top(px(offset))
            .min_h(px(28.0)).px(px(12.0)).py(px(4.0))
            .flex().items_center().justify_center().rounded(px(4.0)).border_1().border_color(border).bg(background)
            .text_size(px(12.0)).line_height(px(16.8)).font_weight(FontWeight::MEDIUM)
            .text_color(foreground)
            .opacity(if self.disabled { 0.5 } else { 1.0 })
            .cursor(if self.disabled { if form { CursorStyle::Arrow } else { CursorStyle::OperationNotAllowed } } else { CursorStyle::PointingHand })
            .when(form, |button| button.line_height(relative(1.5)))
            .when(preset, |button| button.min_h(px(26.0)).min_w(px(46.0)).rounded(px(6.0))
                .line_height(relative(1.5)).font_weight(FontWeight::SEMIBOLD).selected(selected)
                .aria_toggled(if selected { Toggled::True } else { Toggled::False }))
            .shadow(if self.disabled && !self.disabled_shadow { Vec::new() } else if preset {
                if selected || hovered { control_shadow(p) } else { vec![shadow(black().opacity(if p.dark { 0.16 } else { 0.04 }), 0.0, 2.0, 1.0)] }
            } else if pressed && !form { vec![BoxShadow { inset: true, ..shadow(black().opacity(0.05), 0.0, 2.0, 1.0) }] } else { control_shadow(p) })
            .focus_visible(move |style| style.shadow(vec![shadow(p.primary, 4.0, 0.0, 0.0), shadow(p.bg_primary, 2.0, 0.0, 0.0)]))
            .when_some(self.icon, |button, name| button.gap(px(6.0)).child(super::icon(name, if form { 12.0 } else { 14.0 })))
            .child(self.label)
            .on_click(move |event, window, cx| { if let Some(callback) = &callback { callback(event, window, cx); } }), interaction, focus, self.disabled)
    }
}

type Change = Rc<dyn Fn(bool, &mut Window, &mut App)>;
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId, label: SharedString, checked: bool, disabled: bool, palette: Palette, on_change: Option<Change>,
}
impl Toggle {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool, palette: Palette) -> Self {
        Self { id: id.into(), label: label.into(), checked, palette, disabled: false, on_change: None }
    }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
    pub fn on_change(mut self, action: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self { self.on_change = Some(Rc::new(action)); self }
}
impl RenderOnce for Toggle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let (interaction, focus) = control_state(&self.id, self.disabled, window, cx);
        let state = interaction.read(cx).value;
        let hovered = state.hovered && !self.disabled;
        let pressed = state.pressed() && !self.disabled;
        let background = match (self.checked, hovered) { (true, true) => p.primary_hover, (true, false) => p.primary, (false, true) => p.toggle_hover, (false, false) => p.toggle_off };
        let background = color_motion(&self.id, "background", background, window, cx);
        let border = color_motion(&self.id, "border", if hovered { p.primary } else { p.border_medium }, window, cx);
        let offset = motion(&self.id, "offset", if self.checked { 14.0_f32 } else { 0.0 }, 150, window, cx);
        let width = motion(&self.id, "width", if pressed { 16.0_f32 } else { 14.0 }, 100, window, cx);
        let left = motion(&self.id, "left", if self.checked && pressed { 0.0_f32 } else { 2.0 }, 100, window, cx);
        let halo = motion(&self.id, "halo", if hovered { 1.0_f32 } else { 0.0 }, 100, window, cx);
        let mut halo_color = p.selection; halo_color.a *= halo;
        let callback = self.on_change;
        control_feedback(Switch::new(self.id).accessibility_label(self.label).checked(self.checked).disabled(self.disabled).track_focus(&focus)
            .relative().w(px(34.0)).h(px(20.0)).flex_shrink_0().rounded_full().border_1().border_color(border).bg(background)
            .cursor_pointer().opacity(if self.disabled { 0.5 } else { 1.0 })
            .shadow(vec![shadow(halo_color, 3.0, 0.0, 0.0)])
            .focus_visible(move |style| style.shadow(vec![shadow(p.primary, 4.0, 0.0, 0.0), shadow(p.bg_primary, 2.0, 0.0, 0.0)]))
            .child(div().absolute().left(px(left + offset)).top(px(2.0)).w(px(width)).h(px(14.0)).rounded_full().bg(p.bg_primary)
                .shadow(vec![shadow(black().opacity(0.2), 0.0, 2.0, 1.0)]))
            .on_change(move |value, _, window, cx| { if let Some(callback) = &callback { callback(value, window, cx); } }), interaction, focus, self.disabled)
    }
}
