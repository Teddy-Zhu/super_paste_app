mod about;
mod debug;
mod general;
mod mcp;
mod ocr;
mod shortcuts;
mod statistics;
mod sync;
mod tags;

use std::{sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, Easing, Presence, PresencePhase, Transition};
use crate::services::Services;
use super::{configuration::{ConfigurationGlobal, ConfigurationModel, Feedback}, dialogs, icon, palette::Palette, theme};

pub(crate) fn show_window(window: &mut Window, token: Option<&str>, cx: &mut App) -> anyhow::Result<()> {
    if cfg!(target_os = "macos") { cx.set_activation_policy(ActivationPolicy::Regular); }
    window.set_visible(true, cx)?;
    window.activate_window_with_token(token)?;
    if cfg!(target_os = "macos") { cx.activate(true); }
    Ok(())
}

pub(crate) fn hide_window(window: &mut Window, cx: &mut App) -> anyhow::Result<()> {
    window.set_visible(false, cx)?;
    if cfg!(target_os = "macos") { cx.set_activation_policy(ActivationPolicy::Accessory); }
    Ok(())
}

#[derive(Clone)]
pub struct PaneContext { pub services: Arc<Services>, pub configuration: Entity<ConfigurationModel>, owner: WeakEntity<SettingsView> }
impl PaneContext {
    fn is_active(&self, pane: EntityId, cx: &App) -> bool {
        self.owner.upgrade().is_some_and(|owner| {
            let owner = owner.read(cx);
            owner.section == owner.body_section && owner.body.entity_id() == pane
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section { General, Sync, Ocr, Mcp, Tags, Statistics, Shortcuts, Debug, About }
impl Section {
    const ALL: [Self; 9] = [Self::General, Self::Sync, Self::Ocr, Self::Mcp, Self::Tags, Self::Statistics, Self::Shortcuts, Self::Debug, Self::About];
    fn title(self) -> &'static str {
        match self {
            Self::General => "通用设置", Self::Sync => "同步设置", Self::Ocr => "OCR 设置", Self::Mcp => "MCP 服务",
            Self::Tags => "标签管理", Self::Statistics => "统计", Self::Shortcuts => "快捷键", Self::Debug => "调试设置", Self::About => "关于",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::General => "settings", Self::Sync => "sync", Self::Ocr => "text-recognition", Self::Mcp => "connection",
            Self::Tags => "tag", Self::Statistics => "chart-line", Self::Shortcuts => "keyboard", Self::Debug => "bug", Self::About => "information",
        }
    }
    fn create(self, context: PaneContext, window: &mut Window, cx: &mut App) -> AnyView {
        match self {
            Self::General => cx.new(|cx| general::GeneralPane::new(context, window, cx)).into(),
            Self::Sync => cx.new(|cx| sync::SyncPane::new(context, window, cx)).into(),
            Self::Ocr => cx.new(|cx| ocr::OcrPane::new(context, window, cx)).into(),
            Self::Mcp => cx.new(|cx| mcp::McpPane::new(context, window, cx)).into(),
            Self::Tags => cx.new(|cx| tags::TagsPane::new(context, window, cx)).into(),
            Self::Statistics => cx.new(|cx| statistics::StatisticsPane::new(context, window, cx)).into(),
            Self::Shortcuts => cx.new(|cx| shortcuts::ShortcutsPane::new(context, window, cx)).into(),
            Self::Debug => cx.new(|cx| debug::DebugPane::new(context, window, cx)).into(),
            Self::About => cx.new(|cx| about::AboutPane::new(context, window, cx)).into(),
        }
    }
}

pub struct SettingsView {
    context: PaneContext,
    section: Section,
    body_section: Section,
    body: AnyView,
    body_generation: u64,
    animate_body: bool,
    subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(services: Arc<Services>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let configuration = cx.global::<ConfigurationGlobal>().0.clone();
        let context = PaneContext { services, configuration: configuration.clone(), owner: cx.entity().downgrade() };
        theme::apply(configuration.read(cx).projected.settings.color_mode, window.appearance(), cx);
        let subscriptions = vec![
            cx.observe(&configuration, |this, _, cx| {
                let mode = this.context.configuration.read(cx).projected.settings.color_mode;
                theme::apply(mode, cx.window_appearance(), cx);
                cx.notify();
            }),
            cx.subscribe_in(&configuration, window, |_, _, feedback: &Feedback, window, cx| {
                // Immediate failures do not open a generic dialog. Completion
                // still settles shortcut recording and tray projections.
                if !feedback.immediate && !feedback.inline { if let Err(error) = &feedback.result { dialogs::message(format!("保存设置失败：{error}"), window, cx); } }
            }),
            cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            cx.observe_window_visibility(window, |_, _, window, cx| { window.refresh(); cx.notify(); }),
            cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |this, window, cx| {
                theme::apply(this.context.configuration.read(cx).projected.settings.color_mode, window.appearance(), cx);
                cx.notify();
            }),
        ];
        let body = Section::General.create(context.clone(), window, cx);
        Self { context, section: Section::General, body_section: Section::General, body, body_generation: 0, animate_body: false, subscriptions }
    }

    fn select(&mut self, section: Section, cx: &mut Context<Self>) { self.section = section; cx.notify(); }

    fn titlebar(&self, p: Palette, active: bool, window: &Window) -> Div {
        let controls = [
            ("minimize", "最小化窗口", WindowButton::Minimize),
            ("maximize", if window.is_maximized() { "还原窗口" } else { "最大化窗口" }, WindowButton::Maximize),
            ("close", "关闭窗口", WindowButton::Close),
        ].into_iter().map(move |(id, label, action)| {
            let close = matches!(action, WindowButton::Close);
            Button::new(id).accessibility_label(label).w(px(44.0)).h_full().flex().items_center().justify_center()
                .text_color(p.text_secondary).cursor_pointer()
                .hover(move |style| style.bg(if close { rgb(0xc94040).into() } else { p.control_hover }).text_color(if close { rgb(0xffffff).into() } else { p.text_primary }))
                .active(move |style| style.bg(if close { rgb(0xad3434).into() } else { p.control_active }))
                .child(window_button_icon(action, window.is_maximized()))
                .on_click(move |_, window, cx| match action {
                    WindowButton::Minimize => window.minimize_window(),
                    WindowButton::Maximize => window.zoom_window(),
                    WindowButton::Close => if let Err(error) = hide_window(window, cx) { dialogs::message(error.to_string(), window, cx); },
                })
        }).collect::<Vec<_>>();
        div().h(px(38.0)).flex_shrink_0().flex().items_center().justify_between()
            .bg(if active { p.titlebar } else { p.window }).border_b_1().border_color(p.border_medium)
            .child(div().h_full().flex_1().min_w_0().pl(px(14.0)).flex().items_center()
                .on_mouse_down(MouseButton::Left, |event, window, _| {
                    if event.click_count == 2 { window.zoom_window(); } else { window.start_window_move(); }
                })
                .child(div().flex().items_center().gap(px(9.0)).min_w_0()
                    .child(icon("content-paste", 16.0).text_color(p.primary).opacity(if active { 1.0 } else { 0.4 }))
                    .child(div().text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).child("Super Paste"))
                    .child(div().w(px(1.0)).h(px(12.0)).bg(p.border_strong))
                    .child(div().text_size(px(12.0)).text_color(p.text_secondary).truncate().child(self.section.title()))))
            .child(div().h_full().flex().children(controls))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let configuration_error = self.context.configuration.read(cx).error();
        let credential_cleanup_warning = self.context.configuration.read(cx).credential_cleanup_warning();
        let active = window.is_window_active();
        let compact = window.viewport_size().width < px(900.0);
        let mut leaving = self.body_section != self.section;
        let duration = if leaving { 100 } else if self.animate_body { 150 } else { 0 };
        let mut presence = Presence::new((self.body_generation, "settings-page"), !leaving)
            .transition(Transition::new(Duration::from_millis(duration)).easing(Easing::Ease)).sample(window, cx);
        if !presence.should_render() {
            self.body = self.section.create(self.context.clone(), window, cx);
            self.body_section = self.section;
            self.body_generation += 1;
            self.animate_body = true;
            leaving = false;
            presence = Presence::new((self.body_generation, "settings-page"), true)
                .transition(Transition::new(Duration::from_millis(150)).easing(Easing::Ease)).sample(window, cx);
        }
        let navigation = Section::ALL.into_iter().enumerate().map(|(index, section)| {
            let selected = self.section == section;
            let color = if selected && active { p.primary } else { p.text_secondary };
            let background = if selected { if active { p.selection } else { p.selection_inactive } } else { Hsla::transparent_black() };
            Button::new(("section", index)).relative().w_full().min_h(px(34.0)).py(px(7.0)).px(px(10.0))
                .flex().items_center().border_1().rounded(px(6.0)).cursor_pointer()
                .border_color(if selected && active { p.border_medium } else { Hsla::transparent_black() })
                .bg(background).text_color(color)
                .hover(move |style| style.bg(if selected { p.selection_hover } else { p.control_active })
                    .border_color(if selected { p.primary } else { Hsla::transparent_black() }))
                .active(move |style| style.bg(p.control_active))
                .when(section == Section::Debug, |button| button.mt(px(18.0)).child(div().absolute().top(px(-11.0)).left(px(8.0)).right(px(8.0)).h(px(1.0)).bg(p.border_medium)))
                .child(icon(section.icon(), 17.0).mr(px(9.0)).text_color(color))
                .child(div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).child(section.title()))
                .when(selected, move |button| button.child(div().absolute().left(px(-1.0)).top(px(9.0)).bottom(px(9.0))
                    .w(px(3.0)).rounded(px(2.0)).bg(p.primary).opacity(if active { 1.0 } else { 0.4 })))
                .on_click(cx.listener(move |this, _, _, cx| this.select(section, cx)))
        }).collect::<Vec<_>>();
        div().size_full().relative().flex().flex_col().overflow_hidden().bg(p.window).text_color(p.text_primary)
            .text_size(px(13.0)).line_height(relative(1.4))
            .child(self.titlebar(p, active, window))
            .child(div().flex_1().min_h_0().flex().overflow_hidden()
                .child(div().id("settings-sidebar").w(px(if compact { 158.0 } else { 184.0 })).flex_shrink_0().h_full().flex().flex_col()
                    .pt(px(18.0)).pb(px(12.0)).px(px(if compact { 8.0 } else { 10.0 })).bg(p.sidebar)
                    .border_r_1().border_color(p.border_medium).overflow_y_scroll()
                    .child(div().px(px(10.0)).pb(px(12.0)).text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_secondary).child("偏好设置"))
                    .child(div().flex_1().flex().flex_col().gap(px(3.0)).children(navigation)))
                .child(div().relative().flex_1().min_w_0().min_h_0().flex().flex_col().overflow_hidden().bg(p.bg_tertiary)
                    .when(!window.is_maximized(), |content| content.pr(px(5.0)).pb(px(5.0)))
                    .when_some(configuration_error, |content, error| content.child(div().id("configuration-error").role(Role::Alert).py(px(12.0)).px(px(16.0)).mb(px(16.0))
                        .border_1().border_color(p.danger).rounded(px(8.0)).text_color(p.danger).child(format!("配置保存失败：{error}"))))
                    .when_some(credential_cleanup_warning, |content, warning| content.child(div().id("credential-cleanup-warning").role(Role::Alert).py(px(12.0)).px(px(16.0)).mb(px(16.0))
                        .border_1().border_color(p.warning_border).rounded(px(8.0)).bg(p.warning_bg).text_color(p.warning_text).child(warning)))
                    .child(div().relative().flex_1().w_full().min_h_0().opacity(presence.progress)
                        .when(presence.phase == PresencePhase::Entering, |content| content.top(px(4.0 * (1.0 - presence.progress))))
                        .child(self.body.clone())
                        .when(leaving, |content| content.child(div().absolute().inset_0().occlude())))))
            .when(!window.is_maximized(), |root| root.children(resize_handles()))
    }
}

#[derive(Clone, Copy)]
enum WindowButton { Minimize, Maximize, Close }
fn window_button_icon(action: WindowButton, maximized: bool) -> Svg {
    match action {
        WindowButton::Minimize => svg().path("superpaste/window-controls/minimize.svg").size(px(10.0)),
        WindowButton::Maximize if maximized => svg().path("superpaste/window-controls/restore.svg").size(px(11.0)),
        WindowButton::Maximize => svg().path("superpaste/window-controls/maximize.svg").size(px(10.0)),
        WindowButton::Close => svg().path("superpaste/window-controls/close.svg").size(px(12.0)),
    }
}
fn resize_handles() -> Vec<Stateful<Div>> {
    use CursorStyle::*;
    use ResizeEdge::*;
    let edges = [(Top, ResizeUpDown), (Bottom, ResizeUpDown), (Left, ResizeLeftRight), (Right, ResizeLeftRight),
        (TopLeft, ResizeUpLeftDownRight), (TopRight, ResizeUpRightDownLeft), (BottomLeft, ResizeUpRightDownLeft), (BottomRight, ResizeUpLeftDownRight)];
    edges.into_iter().enumerate().map(|(index, (edge, cursor))| {
        let handle = div().id(("resize", index)).absolute().cursor(cursor);
        let handle = match edge {
            Top => handle.top_0().left(px(8.0)).right(px(8.0)).h(px(5.0)),
            Bottom => handle.bottom_0().left(px(8.0)).right(px(8.0)).h(px(5.0)),
            Left => handle.left_0().top(px(8.0)).bottom(px(8.0)).w(px(5.0)),
            Right => handle.right_0().top(px(8.0)).bottom(px(8.0)).w(px(5.0)),
            TopLeft => handle.top_0().left_0().size(px(8.0)), TopRight => handle.top_0().right_0().size(px(8.0)),
            BottomLeft => handle.bottom_0().left_0().size(px(8.0)), BottomRight => handle.bottom_0().right_0().size(px(8.0)),
        };
        handle.on_mouse_down(MouseButton::Left, move |_, window, cx| { cx.stop_propagation(); window.start_window_resize(edge); })
    }).collect()
}
