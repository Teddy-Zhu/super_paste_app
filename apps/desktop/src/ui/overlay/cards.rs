use std::time::Duration;
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, Easing, PremultipliedColor, Transition, transition};
use gpui_kit::component::tooltip::Tooltip;
use super_paste_core::{clipboard::RestoreMode, preview::{PreviewBody, PreviewKind, PreviewModel}};
use super_paste_format::{HistoryEntry, PrimaryType, RecordId};
use crate::ui::{icon, palette::Palette};
use super::OverlayView;

impl OverlayView {
    pub(super) fn render_card(&self, index: usize, entry: &HistoryEntry, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let id = entry.id; let selected = self.selected == Some(id); let hovered = self.hovered_card == Some(id); let pressed = self.pressed_card == Some(id);
        let extent = self.card_size(); let model = self.previews.get(&id).and_then(|preview| preview.model.as_deref());
        let kind = model.map_or_else(|| primary_kind(entry.primary_type), |model| model.kind);
        let card_id = SharedString::from(format!("card-{id}"));
        let policy = Transition::new(Duration::from_millis(100)).easing(Easing::CubicBezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 });
        let target = if pressed { p.primary_active } else if selected && hovered { p.primary_hover } else if selected || hovered { p.primary } else { p.control_border };
        let color = transition((card_id.clone(), "border-color"), PremultipliedColor(target.into()), policy.clone(), window, cx).0;
        let shadow = transition((card_id.clone(), "shadow"), if !pressed && (hovered || selected) { 1.0_f32 } else { 0.0 }, policy, window, cx);
        let alpha = if p.dark { 0.16 + 0.08 * shadow } else { 0.04 + 0.04 * shadow };
        let frame = div().absolute().inset_0().rounded(px(4.0)).border_color(Hsla::from(color)).border(px(if selected { 3.0 } else if hovered { 2.0 } else { 1.0 }));
        div().id(card_id).relative().w(extent.width).h(extent.height).p(px(1.0)).rounded(px(4.0)).overflow_hidden().bg(p.bg_primary)
            .shadow(vec![BoxShadow { color: black().opacity(alpha), offset: point(px(0.0), px(1.0)), blur_radius: px(2.0 + shadow), spread_radius: px(0.0), inset: false }])
            .cursor(if self.scrolling.dragging() { CursorStyle::ClosedHand } else { CursorStyle::PointingHand }).flex().flex_col()
            .hover_listener_mode(HoverListenerMode::InputModalityIndependent)
            .on_hover(cx.listener(move |this, hovered, _, cx| this.card_hover(id, *hovered, cx)))
            .capture_any_mouse_down(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left { this.pressed_card = Some(id); cx.notify(); }
            }))
            .on_click(cx.listener(move |this, event, window, cx| {
                if this.scrolling.suppress(event) { cx.stop_propagation(); return; }
                this.selected = Some(id); this.restore_record(id, RestoreMode::Original, window, cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, window, cx| { this.record_menu(id, event.position, p, window, cx); cx.stop_propagation(); }))
            .child(self.card_header(entry, kind, p, if selected || pressed { p.selection_hover } else if hovered { p.selection } else { p.bg_primary }))
            .child(self.card_toolbar(entry, kind.label(), p, window, cx))
            .child(div().flex_1().min_h_0().overflow_hidden().p(px(12.0)).text_size(px(13.0)).line_height(px(19.5)).child(self.card_content(entry, p, window, cx)))
            .child(self.card_footer(index, entry, model, p, window, cx))
            .child(frame).into_any_element()
    }
    fn card_header(&self, entry: &HistoryEntry, kind: PreviewKind, p: Palette, background: Hsla) -> Div {
        let color = type_color(kind, p);
        div().h(px(68.0)).min_h_0().flex_shrink_0().px(px(12.0)).pt(px(10.0)).pb(px(8.0)).flex().items_center().justify_between().gap(px(12.0)).bg(background)
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(5.0))
                .child(div().min_w_0().flex().items_center().gap(px(6.0))
                    .child(div().flex_shrink_0().flex().items_center().gap(px(6.0)).text_size(px(11.0)).line_height(px(15.0)).font_weight(FontWeight::SEMIBOLD).text_color(color)
                        .child(div().size(px(5.0)).rounded(px(2.0)).bg(color)).child(kind.label()))
                    .when_some(self.ocr.feedback(entry.id), |header, (text, failed)| header.child(div().id("ocr-feedback").min_w_0().text_ellipsis().text_size(px(11.0)).line_height(px(15.0))
                        .text_color(if failed { p.danger } else { p.text_secondary }).role(Role::Status).when(failed, |status| status.tooltip(hint(text.clone()))).child(text))))
                .child(div().id("timestamp").text_size(px(11.0)).line_height(px(15.0)).text_color(p.text_secondary)
                    .tooltip(hint(absolute_time(entry.timestamp))).child(relative_time(entry.timestamp))))
            .when_some(entry.source.as_ref(), |header, source| header.child(self.source_label(source, p)))
    }
    fn card_toolbar(&self, entry: &HistoryEntry, label: &'static str, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let id = entry.id; let formats = format_labels(entry);
        let title = format!("内容格式 · {}", if formats.is_empty() { label } else { &formats });
        let focus = window.use_keyed_state(format!("card-{id}-details-focus"), cx, |_, cx| cx.focus_handle()).read(cx).clone();
        let loading = self.details_loading(id);
        div().h(px(26.0)).min_h_0().flex_shrink_0().px(px(10.0)).pb(px(5.0)).flex().items_center().gap(px(6.0)).border_b_1().border_color(p.border_light)
            .child(div().id("formats").flex_1().min_w_0().text_ellipsis().text_size(px(11.0)).text_color(p.text_tertiary).tooltip(hint(title.into())).child(formats))
            .when(!entry.formats.is_empty(), |bar| bar.child(Button::new("details").accessibility_label("详情").disabled(loading).track_focus(&focus)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .relative().flex_shrink_0().flex().items_center().gap(px(4.0)).px(px(5.0)).py(px(2.0)).rounded(px(4.0)).text_size(px(11.0)).line_height(relative(1.5)).text_color(p.text_secondary)
                .hover(move |style| style.text_color(p.primary).bg(p.selection))
                .child(if loading { "加载中" } else { "详情" }).child(icon("arrow-expand", 11.0))
                .child(canvas(|_, _, _| (), move |bounds, _, window, _| {
                    if !loading && focus.is_focused(window) && window.last_input_was_keyboard() {
                        window.paint_quad(outline(bounds, p.primary, BorderStyle::default()).border_widths(px(2.0)).corner_radii(px(7.0)));
                    }
                }).absolute().top(px(-3.0)).right(px(-3.0)).bottom(px(-3.0)).left(px(-3.0)))
                .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.open_details(id, window, cx); }))))
    }
    fn card_footer(&self, index: usize, entry: &HistoryEntry, model: Option<&PreviewModel>, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let summary = card_summary(entry, model);
        let tags: SharedString = entry.tags.iter().map(|tag| tag.name.as_ref()).collect::<Vec<_>>().join("、").into();
        div().h(px(40.0)).min_h_0().flex_shrink_0().px(px(10.0)).py(px(8.0)).flex().items_center().gap(px(7.0)).border_t_1().border_color(p.border_light).bg(p.bg_tertiary)
            .when(!entry.tags.is_empty(), |footer| footer.child(div().id("tags").flex().items_center().gap(px(3.0)).flex_shrink_0().tooltip(hint(tags))
                .children(entry.tags.iter().take(3).map(|tag| div().size(px(7.0)).rounded_full().border_1().border_color(p.bg_primary)
                    .bg(u32::from_str_radix(tag.color.trim_start_matches('#'), 16).map(rgb).map(Hsla::from).unwrap_or(p.primary))))
                .when(entry.tags.len() > 3, |dots| dots.child(div().text_size(px(9.0)).text_color(p.text_secondary).child(format!("+{}", entry.tags.len() - 3))))))
            .child(div().id("summary").flex_1().min_w_0().text_ellipsis().text_left().text_size(px(11.0)).text_color(p.text_secondary)
                .when(!summary.is_empty(), |label| label.tooltip(hint(summary.clone()))).child(summary))
            .when(index < 9, |footer| footer.child(self.card_shortcut(index, entry.id, p, window, cx)))
    }
    fn card_shortcut(&self, index: usize, id: RecordId, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let policy = Transition::new(Duration::from_millis(100)).easing(Easing::CubicBezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 });
        let progress = transition(format!("card-{id}-shortcut"), if self.shortcuts_visible { 1.0_f32 } else { 0.0 }, policy, window, cx);
        div().id("quick-select").role(Role::Label).aria_hidden(!self.shortcuts_visible)
            .relative().top(px(2.0 * (1.0 - progress))).opacity(progress).flex_shrink_0().px(px(4.0)).py(px(2.0)).rounded(px(3.0)).border_1().border_color(p.control_border).bg(p.bg_primary)
            .shadow(vec![BoxShadow { color: p.control_border, offset: point(px(0.0), px(1.0)), blur_radius: px(0.0), spread_radius: px(0.0), inset: false }])
            .text_size(px(11.0)).line_height(px(16.0)).text_color(p.text_secondary)
            .child(format!("{} + {}", if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }, index + 1)).into_any_element()
    }
    fn card_hover(&mut self, id: RecordId, hovered: bool, cx: &mut Context<Self>) {
        if self.phase == super::Phase::Hidden { return; }
        let next = if hovered { Some(id) } else if self.hovered_card == Some(id) { None } else { self.hovered_card };
        if next != self.hovered_card { self.hovered_card = next; cx.notify(); }
    }
}
fn hint(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}
fn absolute_time(timestamp: i64) -> SharedString {
    chrono::DateTime::from_timestamp_millis(timestamp).map(|date| date.with_timezone(&chrono::Local).format("%Y/%-m/%-d %H:%M:%S").to_string()).unwrap_or_default().into()
}
fn primary_kind(kind: PrimaryType) -> PreviewKind {
    match kind {
        PrimaryType::Text => PreviewKind::Text, PrimaryType::Image => PreviewKind::Image, PrimaryType::Html => PreviewKind::Html, PrimaryType::Rtf => PreviewKind::Rtf,
        PrimaryType::Url => PreviewKind::Url, PrimaryType::Files => PreviewKind::Files, PrimaryType::Email => PreviewKind::Email, PrimaryType::Phone => PreviewKind::Phone,
        PrimaryType::Code => PreviewKind::Code, PrimaryType::Color => PreviewKind::Color, PrimaryType::Empty => PreviewKind::Empty,
    }
}
fn type_color(kind: PreviewKind, p: Palette) -> Hsla {
    match kind {
        PreviewKind::Text => p.type_blue, PreviewKind::Image | PreviewKind::Color => p.type_green,
        PreviewKind::Html | PreviewKind::Rtf | PreviewKind::Code | PreviewKind::Json => p.type_purple,
        PreviewKind::Url => p.type_orange, PreviewKind::Files | PreviewKind::Table => p.type_teal,
        PreviewKind::Email | PreviewKind::Phone => p.type_red, PreviewKind::Empty => p.type_gray,
    }
}
fn relative_time(timestamp: i64) -> String {
    let minutes = chrono::Utc::now().timestamp_millis().saturating_sub(timestamp).max(0) / 60_000;
    match minutes { 0 => "刚刚".into(), 1..60 => format!("{minutes} 分钟前"), 60..1440 => format!("{} 小时前", minutes / 60), 1440..10080 => format!("{} 天前", minutes / 1440), _ => chrono::DateTime::from_timestamp_millis(timestamp).map(|date| date.with_timezone(&chrono::Local).format("%-m月%-d日").to_string()).unwrap_or_default() }
}
fn card_summary(entry: &HistoryEntry, model: Option<&PreviewModel>) -> SharedString {
    let image = model.map_or(entry.primary_type == PrimaryType::Image, |model| matches!(model.body, PreviewBody::Image));
    let files = model.is_some_and(|model| matches!(model.body, PreviewBody::Files));
    if image {
        return entry.image.as_ref().map(|image| {
            let bytes = image.encoded_bytes;
            let size = if bytes < 1024 { format!("{bytes} B") } else if bytes < 1024 * 1024 { format!("{:.1} KB", bytes as f64 / 1024.0) } else { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) };
            SharedString::from(format!("图片 · {size}"))
        }).unwrap_or_else(|| "图片".into());
    }
    if files { return format!("{} 个文件", grouped(entry.files_count)).into(); }
    model.map(|model| SharedString::from(model.summary.clone())).unwrap_or_default()
}
fn grouped(value: u64) -> String {
    let raw = value.to_string(); let mut result = String::new();
    for (index, byte) in raw.bytes().enumerate() { if index > 0 && (raw.len() - index) % 3 == 0 { result.push(','); } result.push(char::from(byte)); } result
}
fn format_labels(entry: &HistoryEntry) -> String {
    use super_paste_format::Formats as F;
    [(F::TEXT, "文本"), (F::HTML, "HTML"), (F::RTF, "RTF"), (F::IMAGE, "图片"), (F::FILES, "文件")].into_iter().filter(|(format, _)| entry.formats.contains(*format)).map(|(_, label)| label).collect::<Vec<_>>().join(" · ")
}
