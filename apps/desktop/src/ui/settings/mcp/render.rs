use std::rc::Rc;
use gpui_kit::{prelude::*, *};
use gpui_kit::base::Button;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use super_paste_format::{ConfigChange, CredentialId};
use super::{McpPane, SecretAction, copy, guide::Client};
use crate::ui::{forms, icon, palette::Palette, widgets::{self, TextButton, Toggle}};

impl Render for McpPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let busy = self.busy();
        self.port.update(cx, |state, cx| state.set_value(self.draft.value.mcp.port as i64, busy, p, window, cx));
        let placeholder = if self.draft.has_saved_secret(CredentialId::McpApiKey) && self.draft.secret_is_keep(CredentialId::McpApiKey) { "已保存，留空保留" } else { "输入 API Key" };
        if self.key.read(cx).placeholder_text() != placeholder { self.key.update(cx, |state, cx| state.set_placeholder(placeholder, window, cx)); }
        let weak = cx.entity().downgrade(); let refresh = weak.clone(); let enabled = weak.clone(); let auth = weak.clone(); let save = weak.clone(); let copy_endpoint = weak.clone();
        let header = div().flex().items_start().justify_between().gap(px(12.0)).mb(px(20.0))
            .child(forms::heading(copy::TITLE, copy::SUBTITLE, p).mb_0())
            .child(div().flex().items_center().gap(px(8.0)).pt(px(3.0))
                .child(div().size(px(6.0)).rounded_full().bg(if self.status.running { p.success_text } else { p.text_tertiary }))
                .child(div().text_size(px(12.0)).text_color(p.text_secondary).child(if self.status.running { copy::RUNNING } else { copy::STOPPED }))
                .child(Button::new("mcp-refresh").accessibility_label(copy::REFRESH).size(px(28.0)).rounded(px(4.0)).text_color(p.text_secondary).hover(move |style| style.bg(p.control_hover).text_color(p.text_primary))
                    .child(icon("refresh", 14.0)).on_click(move |_, window, cx| { let _ = refresh.update(cx, |this, cx| { this.status = this.context.services.mcp.snapshot(); this.synchronize(window, cx); }); })));
        let mut rows = vec![
            forms::setting_row(p, false).child(forms::setting_info(copy::ENABLED, copy::ENABLED_DESCRIPTION, p))
                .child(Toggle::new("mcp-enabled", copy::ENABLED, self.draft.value.mcp.enabled, p).disabled(busy).on_change(move |value, window, cx| { let _ = enabled.update(cx, |this, cx| this.change(ConfigChange::McpEnabled(value), window, cx)); })),
            forms::setting_row(p, true).child(forms::setting_info(copy::PORT, copy::PORT_DESCRIPTION, p)).child(self.port.clone()),
            forms::setting_row(p, true).child(forms::setting_info(copy::AUTH, copy::AUTH_DESCRIPTION, p))
                .child(Toggle::new("mcp-auth", copy::AUTH, self.draft.value.mcp.auth_enabled, p).disabled(busy).on_change(move |value, window, cx| { let _ = auth.update(cx, |this, cx| this.change(ConfigChange::McpAuthEnabled(value), window, cx)); })),
        ];
        if self.draft.value.mcp.auth_enabled { rows.push(self.key_row(p, window, cx)); }
        let valid = self.valid(cx);
        let save_bar = div().flex().justify_between().items_center().gap(px(12.0)).pt(px(12.0))
            .child(div().text_size(px(11.0)).text_color(p.text_tertiary).child(if self.draft.has_unsaved() { "有未保存的修改" } else { "修改后保存生效" }))
            .child(TextButton::new("mcp-save", if busy { copy::SAVING } else { copy::SAVE }, p).primary().disabled(busy || !valid)
                .on_click(move |_, window, cx| { let _ = save.update(cx, |this, cx| this.save(window, cx)); }));
        let endpoint = div().flex().items_center().gap(px(10.0)).mt(px(18.0)).py(px(10.0)).border_y_1().border_color(p.border_light)
            .child(div().flex_shrink_0().text_size(px(11.0)).text_color(p.text_secondary).child(copy::ADDRESS))
            .child(Input::new(&self.endpoint).text_direction(gpui::TextDirection::Ltr).readonly(true).appearance(false).bordered(false).flex_1().min_w_0().text_size(px(11.0)).font_family("monospace").text_color(p.text_secondary))
            .child(TextButton::new("mcp-copy-endpoint", copy::COPY, p).neutral().disabled(busy).on_click(move |_, window, cx| { let _ = copy_endpoint.update(cx, |this, cx| { let text = this.endpoint.read(cx).value().to_string(); this.copy_text(text, window, cx); }); }));
        forms::page("mcp-settings", window).child(header)
            .when_some(self.status.error.clone(), |body, error| body.child(div().id("mcp-service-error").role(Role::Alert).mb(px(14.0)).py(px(10.0)).px(px(12.0)).border_1().border_color(p.danger).rounded(px(4.0)).text_size(px(12.0)).text_color(p.danger).child(error)))
            .child(forms::settings_group(p).children(rows)).child(save_bar).child(endpoint)
            .child(self.client_guide(p, window, cx)).child(self.reference(p, window, cx))
            .when_some(self.message.clone(), |body, (success, message)| body.child(div().id("mcp-operation-status").role(if success { Role::Status } else { Role::Alert })
                .mt(px(12.0)).mx_auto().py(px(6.0)).px(px(14.0)).border_1().border_color(p.border_medium).rounded(px(4.0)).bg(p.bg_elevated).shadow(widgets::control_shadow(p))
                .text_size(px(12.0)).text_color(if success { p.text_secondary } else { p.danger }).child(message)))
    }
}
impl McpPane {
    fn key_row(&self, p: Palette, window: &Window, cx: &mut Context<Self>) -> Div {
        let weak = cx.entity().downgrade(); let toggle = weak.clone(); let generate = weak.clone(); let copy_key = weak.clone(); let clear = weak.clone();
        let busy = self.busy();
        let displayed = self.saved_key.as_ref().unwrap_or(&self.key);
        let focused = displayed.read(cx).focus_handle(cx).contains_focused(window, cx);
        forms::setting_row(p, true).items_start().flex_wrap().gap(px(12.0)).child(forms::setting_info(copy::KEY, copy::KEY_DESCRIPTION, p))
            .child(div().flex_1().min_w(px(250.0)).flex().flex_col().gap(px(8.0))
                .child(div().flex().min_w_0().border_1().border_color(if focused { p.primary } else { p.control_border }).rounded(px(4.0)).bg(p.bg_primary)
                    .child(Input::new(displayed).text_direction(gpui::TextDirection::Ltr).aria_label(copy::KEY).readonly(self.saved_key.is_some()).disabled(busy).appearance(false).bordered(false).focus_bordered(false)
                        .flex_1().min_w_0().font_family("monospace").text_size(px(12.0)).px(px(9.0)))
                    .child(Button::new("mcp-key-visibility").disabled(busy).px(px(9.0)).py(px(4.0)).text_size(px(11.0)).text_color(p.text_secondary)
                        .child(if self.show_key { copy::HIDE } else { copy::REVEAL }).on_click(move |_, window, cx| { let _ = toggle.update(cx, |this, cx| this.toggle_key(window, cx)); })))
                .child(div().flex().flex_wrap().gap(px(8.0))
                    .child(TextButton::new("mcp-generate", copy::GENERATE, p).neutral().disabled(busy).on_click(move |_, window, cx| { let _ = generate.update(cx, |this, cx| this.generate(window, cx)); }))
                    .child(TextButton::new("mcp-copy-key", copy::COPY_KEY, p).neutral().disabled(busy || !(self.draft.has_saved_secret(CredentialId::McpApiKey) || self.draft.secret_is_set(CredentialId::McpApiKey)))
                        .on_click(move |_, window, cx| { let _ = copy_key.update(cx, |this, cx| this.secret(SecretAction::CopyKey, window, cx)); }))
                    .child(TextButton::new("mcp-clear-key", "清除密钥", p).neutral().disabled(busy).on_click(move |_, window, cx| { let _ = clear.update(cx, |this, cx| this.clear_key(window, cx)); }))))
    }
    fn client_guide(&mut self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let weak = cx.entity().downgrade(); let keyboard = weak.clone();
        let tabs = Client::ALL.into_iter().map(|client| {
            let selected = client == self.client; let click = weak.clone(); let activate = weak.clone();
            div().id(("mcp-client", client.index())).role(Role::Tab).aria_label(client.name()).aria_selected(selected).track_focus(&self.client_focus[client.index()])
                .tab_index(if selected { 0 } else { -1 }).flex_shrink_0().relative().px(px(10.0)).py(px(12.0)).text_size(px(12.0)).cursor_pointer()
                .text_color(if selected { p.primary } else { p.text_secondary }).font_weight(if selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .focus_visible(move |style| style.bg(p.control_hover))
                .child(client.name()).when(selected, |tab| tab.child(div().absolute().bottom_0().left(px(10.0)).right(px(10.0)).h(px(2.0)).bg(p.primary)))
                .on_click(move |_, window, cx| { let _ = click.update(cx, |this, cx| this.select_client(client, window, cx)); })
                .on_key_down(move |event, window, cx| { if matches!(event.keystroke.key.as_str(), "enter" | "space") { cx.stop_propagation(); let _ = activate.update(cx, |this, cx| this.select_client(client, window, cx)); } })
        }).collect::<Vec<_>>();
        let client = self.client; let copy_config = weak.clone(); let toggle = weak.clone(); let reveal = weak.clone();
        let settings = self.context.configuration.read(cx).state.snapshot.value.mcp.clone();
        let includes_key = client.includes_key(&settings);
        let heading = div().flex().flex_wrap().items_baseline().justify_between().gap(px(12.0)).mt(px(24.0)).mb(px(10.0))
            .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).child("连接客户端"))
            .child(div().text_size(px(11.0)).text_color(p.text_tertiary).child("使用已保存的服务配置"));
        let mut panel = div().id("mcp-client-panel").role(Role::TabPanel).aria_label(client.name()).focusable().p(px(16.0))
            .child(note(client.help(), p))
            .child(div().mt(px(12.0)).flex().items_center().justify_between().gap(px(12.0))
                .child(Button::new("mcp-client-docs").text_size(px(12.0)).text_color(p.primary).child(copy::DOCS).on_click(move |_, _, cx| cx.open_url(client.docs())))
                .child(TextButton::new("mcp-copy-config", copy::COPY_CONFIG, p).neutral().icon("content-copy").disabled(self.busy())
                    .on_click(move |_, window, cx| { let _ = copy_config.update(cx, |this, cx| this.secret(SecretAction::CopyConfig(client), window, cx)); })))
            .when(includes_key, |panel| panel.child(div().mt(px(8.0)).text_size(px(11.0)).text_color(p.text_tertiary).child("配置包含 API Key")))
            .child(fold("mcp-view-config", "查看配置", self.guide_open, p, window, cx, move |window, cx| { let _ = toggle.update(cx, |this, cx| {
                this.guide_open = !this.guide_open; this.guide_revealed = false; this.guide_epoch = this.guide_epoch.wrapping_add(1);
                if this.guide_open { this.set_guide(None, window, cx); } else { this.guide_code = None; }
                cx.notify();
            }); }).mt(px(12.0)));
        if self.guide_open {
            if includes_key { panel = panel.child(div().flex().items_center().justify_between().gap(px(12.0)).mb(px(8.0)).text_size(px(11.0)).text_color(p.text_tertiary)
                .child(copy::CONFIG_SECRET_NOTICE)
                .child(Button::new("mcp-reveal-config").disabled(self.busy()).text_color(p.primary).child(if self.guide_revealed { copy::HIDE } else { copy::REVEAL })
                    .on_click(move |_, window, cx| { let _ = reveal.update(cx, |this, cx| {
                        if this.guide_revealed { this.guide_revealed = false; this.set_guide(None, window, cx); } else { this.secret(SecretAction::RevealGuide { client: this.client, epoch: this.guide_epoch }, window, cx); }
                    }); }))); }
            if let Some(code) = &self.guide_code { panel = panel.child(code_block(code, p)); }
        }
        div().child(heading).child(div().border_1().border_color(p.border_medium).rounded(px(8.0)).bg(p.bg_primary).overflow_hidden()
            .child(div().id("mcp-client-tabs").role(Role::TabList).aria_label(copy::CLIENTS).flex().gap(px(4.0)).px(px(12.0)).border_b_1().border_color(p.border_light).children(tabs)
                .on_key_down(move |event, window, cx| {
                    let _ = keyboard.update(cx, |this, cx| {
                        let index = this.client.index();
                        let next = match event.keystroke.key.as_str() { "left" => (index + 3) % 4, "right" => (index + 1) % 4, "home" => 0, "end" => 3, _ => return };
                        cx.stop_propagation(); this.select_client(Client::ALL[next], window, cx);
                    });
                })).child(panel))
    }
}
pub(super) fn note(text: impl Into<SharedString>, p: Palette) -> Div { div().text_size(px(12.0)).line_height(px(20.4)).text_color(p.text_secondary).child(text.into()) }
pub(super) fn code_block(code: &Entity<TextareaState>, p: Palette) -> Div {
    div().p(px(12.0)).rounded(px(4.0)).bg(p.bg_secondary).child(Textarea::new(code).text_direction(gpui::TextDirection::Ltr).readonly(true).appearance(false).bordered(false).w_full().font_family("monospace").text_size(px(12.0)).line_height(px(19.8)).text_color(p.text_secondary))
}
pub(super) fn fold(id: &'static str, label: impl Into<SharedString>, expanded: bool, p: Palette, window: &mut Window, cx: &mut App, on_toggle: impl Fn(&mut Window, &mut App) + 'static) -> Stateful<Div> {
    let rotation = widgets::motion(&id.into(), "rotation", if expanded { 0.25_f32 } else { 0.0 }, 150, window, cx);
    let callback = Rc::new(on_toggle); let keyboard = callback.clone();
    div().id(id).focusable().tab_index(0).role(Role::Button).aria_expanded(expanded).cursor_pointer().flex().items_center().gap(px(8.0)).py(px(11.0)).text_size(px(12.0))
        .focus_visible(move |style| style.bg(p.control_hover))
        .child(icon("chevron-right", 12.0).text_color(p.text_secondary).with_transformation(Transformation::rotate(percentage(rotation)))).child(label.into())
        .on_click(move |_, window, cx| callback(window, cx))
        .on_key_down(move |event, window, cx| { if matches!(event.keystroke.key.as_str(), "enter" | "space") { cx.stop_propagation(); keyboard(window, cx); } })
}
