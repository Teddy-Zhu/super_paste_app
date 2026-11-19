use super::*;
use gpui_kit::base::{Radio, RadioGroup, Easing, Transition, transition};
use gpui_kit::component::{input::Input, select::Select};
use crate::ui::{icon, widgets::{TextButton, Toggle}};

impl SyncPane {
    fn direction(&self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> RadioGroup {
        let selected = DIRECTIONS.iter().position(|(direction, _, _)| *direction == self.draft.value.sync.direction).unwrap_or(0);
        let weak = cx.entity().downgrade();
        let radios = DIRECTIONS.into_iter().enumerate().map(|(index, (direction, label, description))| {
            let checked = index == selected; let weak = weak.clone(); let hover = weak.clone();
            let target = if checked || self.radio_hover[index] { p.selection } else { Hsla::transparent_black() };
            let background = transition((index, "sync-direction-background"), target, Transition::new(Duration::from_millis(150)).easing(Easing::Ease), window, cx);
            Radio::new(("sync-direction", index)).checked(checked).accessibility_label(label).track_focus(&self.radio_focus[index]).set_position(index + 1, 3).tab_stop(checked)
                .flex().items_start().gap(px(10.0)).p(px(6.0)).rounded(px(4.0)).cursor_pointer().bg(background)
                .shadow(if checked { vec![BoxShadow { inset: true, color: p.primary, offset: point(px(0.0), px(0.0)), blur_radius: px(0.0), spread_radius: px(1.0) }] } else { Vec::new() })
                .active(move |style| style.bg(p.selection_hover)).focus_visible(move |style| style.shadow(vec![BoxShadow { inset: false, color: p.primary, offset: point(px(0.0), px(0.0)), blur_radius: px(0.0), spread_radius: px(1.0) }]))
                .on_hover(move |hovered, _, cx| { let _ = hover.update(cx, |this, cx| { if this.radio_hover[index] != *hovered { this.radio_hover[index] = *hovered; cx.notify(); } }); })
                .on_change(move |_, _, window, cx| { let _ = weak.update(cx, |this, cx| this.change(ConfigChange::SyncDirection(direction), window, cx)); })
                .child(div().mt(px(2.0)).size(px(13.0)).flex_shrink_0().rounded_full().border_1().border_color(if checked { p.primary } else { p.control_border }).bg(p.bg_primary)
                    .flex().items_center().justify_center().when(checked, |indicator| indicator.child(div().size(px(7.0)).rounded_full().bg(p.primary))))
                .child(div().min_w_0().flex().flex_col().gap(px(4.0))
                    .child(div().text_size(px(13.0)).font_weight(FontWeight::BOLD).text_color(p.text_primary).child(label))
                    .child(div().text_size(px(12.0)).text_color(p.text_secondary).child(description)))
        }).collect::<Vec<_>>();
        RadioGroup::new("sync-directions").aria_label("同步方向").flex().flex_col().gap(px(12.0)).w(px(260.0)).children(radios)
            .on_key_down(move |event, window, cx| {
                let key = event.keystroke.key.as_str();
                let next = match key { "up" | "left" => (selected + 2) % 3, "down" | "right" => (selected + 1) % 3, _ => return };
                cx.stop_propagation();
                let _ = weak.update(cx, |this, cx| { this.change(ConfigChange::SyncDirection(DIRECTIONS[next].0), window, cx); this.radio_focus[next].focus(window, cx); });
            })
    }
    fn webdav(&mut self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let saved = self.draft.has_saved_secret(CredentialId::WebDavPassword);
        let placeholder = if saved && self.draft.secret_is_keep(CredentialId::WebDavPassword) { "已保存的密码留空即可继续使用" } else { "输入密码" };
        if self.password.read(cx).placeholder_text() != placeholder { self.password.update(cx, |state, cx| state.set_placeholder(placeholder, window, cx)); }
        self.timeout.update(cx, |state, cx| state.set_value(self.draft.value.sync.webdav.timeout.min(i64::MAX as u64) as i64, false, p, window, cx));
        let clear = cx.entity().downgrade(); let https = clear.clone(); let test = clear.clone(); let manual = clear.clone(); let refresh = clear.clone();
        let valid = !self.draft.value.sync.enabled || self.valid_webdav();
        let testing = self.action.as_ref().is_some_and(|action| action.kind == Action::Test);
        let syncing = self.action.as_ref().is_some_and(|action| action.kind == Action::Manual);
        section("WebDAV 服务器配置", "配置 WebDAV 服务器连接信息", p)
            .child(forms::settings_group(p)
                .child(forms::setting_row(p, false).child(forms::setting_info("服务器地址", "WebDAV 服务器的完整 URL 地址", p)).child(text_control(&self.url, "服务器地址", p, window, cx)))
                .child(forms::setting_row(p, true).child(forms::setting_info("用户名", "WebDAV 服务器登录用户名", p)).child(text_control(&self.username, "用户名", p, window, cx)))
                .child(forms::setting_row(p, true).child(forms::setting_info("密码", "WebDAV 服务器登录密码", p))
                    .child(div().w(relative(0.45)).min_w(px(200.0)).max_w(px(360.0)).flex().flex_col().gap(px(8.0))
                        .child(text_input(&self.password, "密码", p, window, cx))
                        .child(div().flex().child(TextButton::new("sync-clear-password", "清除密码", p).neutral().on_click(move |_, window, cx| { let _ = clear.update(cx, |this, cx| this.clear_password(window, cx)); })))))
                .child(forms::setting_row(p, true).child(forms::setting_info("远程路径", "存储同步文件的远程目录路径", p)).child(text_control(&self.path, "远程路径", p, window, cx)))
                .child(forms::setting_row(p, true).child(forms::setting_info("超时时间", "网络请求超时时间（毫秒）", p))
                    .child(div().flex().items_center().gap(px(8.0)).child(self.timeout.clone()).child(div().text_size(px(12.0)).text_color(p.text_secondary).child("毫秒"))))
                .child(forms::setting_row(p, true).child(forms::setting_info("使用 HTTPS", "启用 HTTPS 安全连接", p))
                    .child(Toggle::new("sync-https", "使用 HTTPS", self.draft.value.sync.webdav.use_https, p).on_change(move |enabled, window, cx| { let _ = https.update(cx, |this, cx| this.edit_webdav(|config| config.use_https = enabled, window, cx)); }))))
            .child(div().mt(px(24.0)).pt(px(20.0)).border_t_1().border_color(p.border_light)
                .child(div().flex().flex_wrap().gap(px(8.0)).mb(px(16.0))
                    .child(TextButton::new("sync-test", if testing { "测试中..." } else { "测试连接" }, p).neutral().disabled(testing || !valid).on_click(move |_, window, cx| { let _ = test.update(cx, |this, cx| this.start(Action::Test, window, cx)); }))
                    .child(TextButton::new("sync-manual", if syncing { "同步中..." } else { "手动同步" }, p).primary().disabled(syncing || !valid).on_click(move |_, window, cx| { let _ = manual.update(cx, |this, cx| this.start(Action::Manual, window, cx)); }))
                    .child(TextButton::new("sync-refresh", "刷新状态", p).neutral().on_click(move |_, window, cx| { let _ = refresh.update(cx, |this, cx| { let status = this.context.services.sync.snapshot(); this.status_changed(status, window, cx); }); })))
                .when(!valid, |body| body.child(banner("配置不完整，请填写完整的服务器信息并确保 URL 格式正确", p.warning_text, p.warning_bg, p.warning_border).mb(px(12.0))))
                .when_some(self.test_result.clone(), |body, (success, message)| body.child(feedback(success, message, p))))
    }
    fn status_cards(&self, p: Palette, window: &Window, cx: &App) -> Div {
        let configuration = &self.context.configuration.read(cx).state.snapshot.value.sync;
        let mut cards = vec![card("同步状态", "已启用", p.success_text, p)];
        if self.phase != SyncState::Idle || self.phase_message.is_some() {
            let color = match self.phase { SyncState::Syncing => p.primary, SyncState::Success => p.success_text, SyncState::Error => p.danger, SyncState::Idle => p.text_primary };
            let mut value = div().flex().items_center().gap(px(4.0)).min_w_0().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).text_color(color);
            if self.phase == SyncState::Syncing {
                let image = icon("sync", 14.0).text_color(p.primary);
                let image = if window.is_visible() { image.with_animation("sync-status-spin", Animation::new(Duration::from_secs(1)).repeat().with_easing(|value| value), |image, value| image.with_transformation(Transformation::rotate(percentage(value)))).into_any_element() } else { image.into_any_element() };
                value = value.child(image);
            }
            cards.push(card_shell("实时状态", p).child(value.child(self.phase_message.clone().unwrap_or_else(|| "正在同步...".into()))));
        }
        cards.push(card("同步间隔", INTERVALS.iter().find(|(_, interval)| *interval == configuration.interval).map(|(label, _)| *label).unwrap_or("未设置"), p.text_primary, p));
        cards.push(card("同步方向", DIRECTIONS.iter().find(|(direction, _, _)| *direction == configuration.direction).map(|(_, label, _)| *label).unwrap_or("未设置"), p.text_primary, p));
        cards.push(card("最后同步", if self.status.last_sync_time > 0 { forms::local_timestamp(self.status.last_sync_time) } else { "暂无记录".into() }, p.text_primary, p));
        let compact = window.viewport_size().width < px(900.0);
        let width = (f32::from(window.viewport_size().width) - if compact { 158.0 } else { 184.0 }).min(1120.0) - if compact { 44.0 } else { 64.0 };
        let columns = ((width + 8.0) / 208.0).floor().max(1.0) as usize;
        section("同步状态", "查看当前同步状态和历史记录", p).child(div().grid().grid_cols(columns as u16).gap(px(8.0)).mt(px(16.0)).children(cards))
    }
}
impl Render for SyncPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let enabled = self.draft.value.sync.enabled; let provider = self.draft.value.sync.provider.clone(); let unsaved = self.draft.has_unsaved();
        let saving = self.action.as_ref().is_some_and(|action| action.kind == Action::Save);
        let toggle = cx.entity().downgrade(); let save = toggle.clone(); let reset = toggle.clone();
        let mut group = forms::settings_group(p)
            .child(forms::setting_row(p, false).child(forms::setting_info("启用同步", "开启后将自动同步应用配置到云端", p))
                .child(Toggle::new("sync-enabled", "启用同步", enabled, p).on_change(move |enabled, window, cx| { let _ = toggle.update(cx, |this, cx| this.change(ConfigChange::SyncEnabled(enabled), window, cx)); })))
            .child(forms::setting_row(p, true).child(forms::setting_info("同步提供者", "选择用于同步的云存储服务", p))
                .child(Select::new(&self.provider).intrinsic_size(true).w(px(220.0)).h(px(32.0)).accessibility_label("同步提供者")));
        if enabled {
            group = group.child(forms::setting_row(p, true).child(forms::setting_info("同步间隔", "设置自动同步的时间间隔", p))
                .child(Select::new(&self.interval).intrinsic_size(true).w(px(220.0)).h(px(32.0)).accessibility_label("同步间隔")))
                .child(forms::setting_row(p, true).child(forms::setting_info("同步方向", "选择配置同步的方向", p)).child(self.direction(p, window, cx)));
        }
        let mut page = forms::page("sync-settings", window).child(forms::heading("同步设置", "将应用配置同步到云端", p))
            .when_some(self.save_message.clone(), |body, (success, message)| body.child(feedback(success, message, p).mb(px(20.0))))
            .when(unsaved, |body| body.child(banner("有未保存的更改", p.warning_text, p.warning_bg, p.warning_border).mb(px(20.0))))
            .child(group);
        if provider == "webdav" { page = page.child(self.webdav(p, window, cx)); }
        page = page.child(section("配置管理", "保存或重置同步配置", p).child(div().mt(px(16.0)).flex().flex_wrap().gap(px(8.0))
            .child(div().min_w(px(96.0)).child(TextButton::new("sync-save", if saving { "保存中..." } else { "保存配置" }, p).primary().disabled(saving || (!unsaved && !enabled))
                .on_click(move |_, window, cx| { let _ = save.update(cx, |this, cx| this.start(Action::Save, window, cx)); })))
            .child(TextButton::new("sync-reset", "重置配置", p).neutral().disabled(saving || !unsaved).on_click(move |_, window, cx| { let _ = reset.update(cx, |this, cx| this.discard(window, cx)); }))));
        if self.context.configuration.read(cx).state.snapshot.value.sync.enabled { page = page.child(self.status_cards(p, window, cx)); }
        page
    }
}
fn section(title: &'static str, description: &'static str, p: Palette) -> Div { div().mt(px(24.0)).pt(px(20.0)).border_t_1().border_color(p.border_light).child(forms::heading(title, description, p)) }
fn card_shell(title: &'static str, p: Palette) -> Div { div().min_w_0().p(px(16.0)).border_1().border_color(p.border_light).rounded(px(6.0)).bg(p.bg_primary).flex().flex_col().gap(px(8.0))
    .child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_secondary).child(title)) }
fn card(title: &'static str, text: impl Into<SharedString>, color: Hsla, p: Palette) -> Div { card_shell(title, p).child(div().text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(text.into())) }
fn banner(text: impl Into<SharedString>, color: Hsla, background: Hsla, border: Hsla) -> Div { div().py(px(12.0)).px(px(16.0)).rounded(px(6.0)).border_1().border_color(border).bg(background).text_color(color).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).child(text.into()) }
fn feedback(success: bool, text: impl Into<SharedString>, p: Palette) -> Div { banner(text, if success { p.success_text } else { p.danger }, if success { p.success_bg } else { p.danger_bg }, if success { p.success_border } else { p.danger_border }) }
fn text_input(input: &Entity<InputState>, label: &'static str, p: Palette, window: &Window, cx: &App) -> Input {
    let focused = input.read(cx).focus_handle(cx).contains_focused(window, cx);
    Input::new(input).text_direction(gpui::TextDirection::Ltr).aria_label(label).focus_bordered(false).w_full().text_size(px(13.0)).rounded(px(4.0)).bg(p.bg_primary).border_color(if focused { p.primary } else { p.control_border })
}
fn text_control(input: &Entity<InputState>, label: &'static str, p: Palette, window: &Window, cx: &App) -> Div { div().w(relative(0.45)).min_w(px(200.0)).max_w(px(360.0)).child(text_input(input, label, p, window, cx)) }
