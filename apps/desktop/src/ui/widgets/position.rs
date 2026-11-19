use std::{rc::Rc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, Easing, Transition, transition};
use super_paste_format::OverlayPosition;
use super::{Interaction, channel, color_motion, control_feedback, control_shadow, control_state, motion};
use crate::ui::{icon, palette::Palette};

const POSITIONS: [(OverlayPosition, &str, &str); 4] = [
    (OverlayPosition::Top, "top", "上方"), (OverlayPosition::Bottom, "bottom", "下方"),
    (OverlayPosition::Left, "left", "左侧"), (OverlayPosition::Right, "right", "右侧"),
];
type Select = Rc<dyn Fn(OverlayPosition, &mut Window, &mut App)>;
#[derive(IntoElement)]
pub struct PositionSelector { id: ElementId, value: OverlayPosition, palette: Palette, wide: bool, on_change: Select }
impl PositionSelector {
    pub fn new(id: impl Into<ElementId>, value: OverlayPosition, palette: Palette, wide: bool, on_change: impl Fn(OverlayPosition, &mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), value, palette, wide, on_change: Rc::new(on_change) }
    }
}
impl RenderOnce for PositionSelector {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.palette;
        let index = POSITIONS.iter().position(|(position, _, _)| *position == self.value).unwrap_or(1);
        let offset = transition(channel(&self.id, "position"), index as f32,
            Transition::new(Duration::from_millis(200)).easing(Easing::CubicBezier { x1: 0.25, y1: 0.46, x2: 0.45, y2: 0.94 }), window, cx);
        let mut active_interaction = Interaction::default();
        let buttons: Vec<_> = POSITIONS.into_iter().map(|(position, name, label)| {
            let active = position == self.value;
            let id = channel(&self.id, name);
            let (interaction, focus) = control_state(&id, false, window, cx);
            let state = interaction.read(cx).value;
            if active { active_interaction = state; }
            let foreground = if active { p.text_inverse } else if state.hovered { p.primary } else { p.text_secondary };
            let foreground = color_motion(&id, "foreground", foreground, window, cx);
            let background = color_motion(&id, "background", if state.hovered && !active { p.selection } else { transparent_black() }, window, cx);
            let border = color_motion(&id, "border", if state.hovered && !active { p.primary } else { transparent_black() }, window, cx);
            let press = motion(&id, "press", if state.pressed() { 1.0_f32 } else { 0.0 }, 100, window, cx);
            let check = motion(&id, "check", if active { 1.0_f32 } else { 0.0 }, 150, window, cx);
            let edge = div().absolute().inset(px(2.0)).rounded(px(1.0)).bg(foreground)
                .when(matches!(position, OverlayPosition::Top), |edge| edge.h(px(3.0)).bottom_auto())
                .when(matches!(position, OverlayPosition::Bottom), |edge| edge.h(px(3.0)).top_auto())
                .when(matches!(position, OverlayPosition::Left), |edge| edge.w(px(3.0)).right_auto())
                .when(matches!(position, OverlayPosition::Right), |edge| edge.w(px(3.0)).left_auto());
            let select = self.on_change.clone();
            control_feedback(Button::new(id).selected(active).accessibility_label(label).track_focus(&focus)
                .aria_toggled(if active { Toggled::True } else { Toggled::False })
                .relative().top(px(press)).flex().flex_col().flex_1().min_w(px(52.0))
                .items_center().justify_center().gap(px(5.0)).min_h(px(56.0)).px(px(8.0)).py(px(6.0))
                .rounded(px(4.0)).border_1().border_color(border)
                .bg(background).text_color(foreground).text_size(px(12.0)).line_height(relative(1.5)).font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(state.pressed(), |button| button.shadow(vec![BoxShadow { color: black().opacity(0.05), offset: point(px(0.0), px(1.0)), blur_radius: px(2.0), spread_radius: px(0.0), inset: true }]))
                .focus_visible(move |style| style.border_color(p.primary))
                .child(div().relative().w(px(24.0)).h(px(18.0)).border_1().border_color(foreground).rounded(px(3.0)).child(edge))
                .child(label)
                .child(div().absolute().top(px(3.0)).right(px(3.0)).size(px(11.0)).flex().items_center().justify_center().opacity(check)
                    .child(icon("check", 11.0 * (0.65 + 0.35 * check))))
                .on_click(move |_, window, cx| select(position, window, cx)), interaction, focus, false).into_any_element()
        }).collect();
        let indicator_background = color_motion(&self.id, "indicator-background", if active_interaction.pressed() { p.primary_active } else if active_interaction.hovered { p.primary_hover } else { p.primary }, window, cx);
        div().id(self.id.clone()).role(Role::Group).aria_label("粘贴面板出现位置").relative().flex().gap(px(4.0)).p(px(3.0)).border_1().border_color(p.control_border).rounded(px(6.0)).bg(p.bg_secondary)
            .when(self.wide, |element| element.w_full().max_w(px(360.0))).when(!self.wide, |element| element.w(px(228.0)))
            .child(canvas(|_, _, _| (), move |bounds, _, window, _| {
                let width = (bounds.size.width - px(18.0)) / 4.0;
                let bounds = Bounds::new(point(bounds.origin.x + px(3.0) + (width + px(4.0)) * offset, bounds.origin.y + px(3.0)), size(width, bounds.size.height - px(6.0)));
                let corners = Corners::all(px(4.0));
                let shadows = control_shadow(p);
                window.paint_drop_shadows(bounds, corners, &shadows);
                let mut quad = fill(bounds, indicator_background);
                quad.corner_radii = corners; quad.border_widths = Edges::all(px(1.0)); quad.border_color = p.primary_hover;
                window.paint_quad(quad);
                window.paint_inset_shadows(bounds, corners, &shadows);
            }).absolute().inset_0().size_full())
            .children(buttons)
    }
}
