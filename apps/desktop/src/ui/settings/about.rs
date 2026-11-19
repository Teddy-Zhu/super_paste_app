use gpui_kit::{prelude::*, *};
use super_paste_format::APPLICATION_NAME;
use super::PaneContext;
use crate::ui::{forms, palette::Palette};

pub struct AboutPane { context: PaneContext, subscriptions: Vec<Subscription> }
impl AboutPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&context.configuration, |_, _, cx| cx.notify()), cx.observe_window_appearance(window, |_, _, cx| cx.notify())];
        Self { context, subscriptions }
    }
}
impl Render for AboutPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        forms::page("about-settings", window)
            .child(forms::heading(format!("关于 {APPLICATION_NAME}"), "应用信息和版本详情", p))
            .child(div().px(px(20.0)).py(px(28.0)).border_1().border_color(p.border_medium).rounded(px(8.0)).bg(p.bg_primary).text_center()
                .child(div().id("about-app-name").role(Role::Heading).aria_level(3).text_size(px(20.0)).font_weight(FontWeight::SEMIBOLD).mb(px(8.0)).text_color(p.text_primary).child(APPLICATION_NAME))
                .child(div().text_size(px(12.0)).text_color(p.text_secondary).mb(px(12.0)).child(concat!("版本 ", env!("CARGO_PKG_VERSION"))))
                .child(div().text_size(px(13.0)).line_height(px(19.5)).text_color(p.text_secondary).child("智能剪贴板与 OCR 工具")))
    }
}
