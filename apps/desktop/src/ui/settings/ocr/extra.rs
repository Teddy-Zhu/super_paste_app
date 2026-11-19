use super::*;
use gpui_kit::component::{ActiveTheme, WindowExt, input::Textarea};

impl OcrPane {
    pub(super) fn extra_params(&self, disabled: bool, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let focused = self.extra.read(cx).focus_handle(cx).contains_focused(window, cx);
        let mono = cx.theme().mono_font_family.clone();
        let owner = cx.entity().downgrade();
        let capture = owner.clone();
        let listeners = canvas(|_, _, _| (), move |_, _, window, _| {
            let movement = capture.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if !phase.capture() { return; }
                let _ = movement.update(cx, |this, cx| {
                    let Some((origin, height)) = this.extra_drag else { return; };
                    if !event.dragging() || this.saving.is_some() || window.has_active_dialog(cx) {
                        this.extra_drag = None; cx.notify(); return;
                    }
                    let next = (height + event.position.y - origin).max(px(110.0));
                    if next != this.extra_height { this.extra_height = next; cx.notify(); }
                    window.prevent_default(); cx.stop_propagation();
                });
            });
            let release = capture.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase.capture() && event.button == MouseButton::Left {
                    let _ = release.update(cx, |this, cx| {
                        if this.extra_drag.take().is_some() { cx.stop_propagation(); cx.notify(); }
                    });
                }
            });
        }).absolute().inset_0();
        super::render::field_frame(self.extra.entity_id(), disabled, focused, p, window, cx).relative().h(self.extra_height)
            .child(Textarea::new(&self.extra).text_direction(gpui::TextDirection::Ltr).disabled(disabled).appearance(false).bordered(false)
                .aria_label("额外调用参数 (JSON)").h((self.extra_height - px(2.0)).max(px(108.0)))
                .content_padding(Edges { top: px(5.0), right: px(9.0), bottom: px(5.0), left: px(9.0) })
                .w_full().font_family(mono).text_size(px(13.0)).line_height(relative(1.4)).bg(transparent_black()))
            .child(listeners)
            .when(!disabled, |body| body.child(div().id("ocr-extra-resize").absolute().right_0().bottom_0().size(px(13.0))
                .cursor(CursorStyle::ResizeUpDown)
                .child(canvas(|bounds, _, _| bounds, move |bounds, _, window, _| {
                    let mut grip = PathBuilder::stroke(px(1.0));
                    for offset in [5.0, 9.0] {
                        grip.move_to(point(bounds.right() - px(offset), bounds.bottom() - px(2.0)));
                        grip.line_to(point(bounds.right() - px(2.0), bounds.bottom() - px(offset)));
                    }
                    match grip.build() {
                        Ok(path) => window.paint_path(path, p.text_secondary),
                        Err(error) => tracing::error!(%error, "绘制文本框调整手柄失败"),
                    }
                }).size_full())
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if this.saving.is_some() { return; }
                        this.extra_drag = Some((event.position.y, this.extra_height));
                        window.prevent_default(); cx.stop_propagation(); cx.notify();
                    });
                })))
    }
}
