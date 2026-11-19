use gpui_kit::{prelude::*, *};
use gpui_kit::base::Button;
use gpui_kit::component::tooltip::Tooltip;
use super::{Click, color_motion, control_feedback, control_state, motion, shadow};
use crate::ui::{icon, palette::Palette};

#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId, label: SharedString, icon: &'static str, palette: Palette,
    danger: bool, disabled: bool, tooltip: Option<SharedString>, on_click: Option<Click>,
}
impl IconButton {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, icon: &'static str, palette: Palette) -> Self {
        Self { id: id.into(), label: label.into(), icon, palette, danger: false, disabled: false, tooltip: None, on_click: None }
    }
    pub fn danger(mut self) -> Self { self.danger = true; self }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self { self.tooltip = Some(text.into()); self }
    pub fn on_click(mut self, action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(action)); self
    }
}
impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let (interaction, focus) = control_state(&self.id, self.disabled, window, cx);
        let state = interaction.read(cx).value;
        let hovered = state.hovered && !self.disabled;
        let pressed = state.pressed() && !self.disabled;
        let background = color_motion(&self.id, "background", if hovered {
            if self.danger { p.danger_bg } else { p.control_hover }
        } else { transparent_black() }, window, cx);
        let foreground = color_motion(&self.id, "foreground", if hovered {
            if self.danger { p.danger } else { p.text_primary }
        } else { p.text_secondary }, window, cx);
        let offset = motion(&self.id, "press", if pressed { 1.0_f32 } else { 0.0 }, 100, window, cx);
        let callback = self.on_click;
        control_feedback(Button::new(self.id).accessibility_label(self.label).disabled(self.disabled).track_focus(&focus)
            .relative().top(px(offset)).size(px(28.0)).flex_shrink_0().p_0().rounded(px(4.0))
            .flex().items_center().justify_center().bg(background).text_color(foreground)
            .opacity(if self.disabled { 0.5 } else { 1.0 }).cursor(if self.disabled { CursorStyle::Arrow } else { CursorStyle::PointingHand })
            .focus_visible(move |style| style.shadow(vec![shadow(p.primary, 4.0, 0.0, 0.0), shadow(p.bg_primary, 2.0, 0.0, 0.0)]))
            .when_some(self.tooltip, |button, text| button.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx)))
            .child(icon(self.icon, 16.0))
            .on_click(move |event, window, cx| { if let Some(callback) = &callback { callback(event, window, cx); } }), interaction, focus, self.disabled)
    }
}

#[derive(IntoElement)]
pub struct ColorSwatch {
    id: ElementId, color: u32, selected: bool, disabled: bool, palette: Palette, on_click: Option<Click>,
}
impl ColorSwatch {
    pub fn new(id: impl Into<ElementId>, color: u32, selected: bool, palette: Palette) -> Self {
        Self { id: id.into(), color, selected, disabled: false, palette, on_click: None }
    }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
    pub fn on_click(mut self, action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(std::rc::Rc::new(action)); self
    }
}
impl RenderOnce for ColorSwatch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let (interaction, focus) = control_state(&self.id, self.disabled, window, cx);
        let pressed = interaction.read(cx).pressed() && !self.disabled;
        let offset = motion(&self.id, "press", if pressed { 1.0_f32 } else { 0.0 }, 100, window, cx);
        let border = color_motion(&self.id, "border", if self.selected { p.bg_primary } else { transparent_black() }, window, cx);
        let selected = motion(&self.id, "selection", if self.selected { 1.0_f32 } else { 0.0 }, 100, window, cx);
        let selection_shadow = if selected > 0.0 { vec![shadow(p.primary.opacity(selected), 2.0 * selected, 0.0, 0.0)] } else { Vec::new() };
        let focus_shadow = selection_shadow.clone();
        let callback = self.on_click;
        control_feedback(Button::new(self.id).accessibility_label(format!("#{:06x}", self.color)).disabled(self.disabled)
            .selected(self.selected).aria_toggled(if self.selected { Toggled::True } else { Toggled::False }).track_focus(&focus)
            .relative().top(px(offset)).size(px(22.0)).flex_shrink_0().p_0().rounded_full().border_2().border_color(border).bg(rgb(self.color))
            .shadow(selection_shadow).opacity(if self.disabled { 0.5 } else { 1.0 })
            .cursor(if self.disabled { CursorStyle::Arrow } else { CursorStyle::PointingHand })
            .focus_visible(move |style| {
                let mut shadows = vec![shadow(p.primary, 4.0, 0.0, 0.0), shadow(p.bg_primary, 2.0, 0.0, 0.0)];
                shadows.extend(focus_shadow.clone()); style.shadow(shadows)
            })
            .on_click(move |event, window, cx| { if let Some(callback) = &callback { callback(event, window, cx); } }), interaction, focus, self.disabled)
    }
}
