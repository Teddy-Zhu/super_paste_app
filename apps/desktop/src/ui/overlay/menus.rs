use gpui_kit::component::Sizable as _;
use gpui_kit::{prelude::*, *};
use std::time::Duration;
use std::sync::Arc;
use gpui_kit::component::{Icon, WindowExt, menu::{MenuItemAppearance, PopupMenu, PopupMenuAppearance, PopupMenuItem}};
use super_paste_core::{Error, clipboard::{ClipboardSnapshot, RestoreMode, QrAction}, preview::{PreviewBody}};
use super_paste_format::{Formats, PrimaryType, RecordId, TagId, TextContent};
use crate::ui::{dialogs, palette::Palette};
use super::{OverlayView, Phase};

pub(super) struct Menu {
    view: Entity<PopupMenu>, position: Point<Pixels>, anchor: Anchor, return_focus: FocusHandle,
    pub record: Option<RecordId>,
    waiting_preview: bool,
    ocr_item: Option<usize>,
    dark: bool,
    _dismiss: Subscription,
}
#[derive(Clone)]
enum Mutation { Delete, Binding { tag: TagId, bound: bool }, Color(Arc<str>) }
impl Mutation {
    fn label(&self) -> &'static str { match self { Self::Delete => "删除", Self::Binding { bound: true, .. } => "绑定标签", Self::Binding { bound: false, .. } => "解绑标签", Self::Color(_) => "复制颜色" } }
}
impl OverlayView {
    pub(super) fn toolbar_menu(&mut self, p: Palette, window: &mut Window, cx: &mut Context<Self>) {
        self.scrolling.stop();
        if self.menu.as_ref().is_some_and(|menu| menu.record.is_none()) { self.close_menu(window, cx); return; }
        if !matches!(self.phase, Phase::Opening | Phase::Opened) { return; }
        self.close_menu(window, cx);
        let focus = window.focused(cx).unwrap_or_else(|| self.focus.clone());
        let weak = cx.entity().downgrade(); let clearing = self.clearing;
        let view = PopupMenu::build(window, cx, move |menu, _, _| {
            surface(menu, p, true).action_context(focus.clone()).min_w(px(150.0))
                .item(PopupMenuItem::new("设置").icon(menu_icon("cog", None))
                    .on_click(|_, _, cx| cx.dispatch_action(&crate::host::ShowSettings)))
                .item(PopupMenuItem::new("清空历史").icon(menu_icon("delete", None)).appearance(toolbar_danger(p))
                    .disabled(clearing).on_click(move |_, window, cx| {
                        let _ = weak.update(cx, |this, cx| this.confirm_clear(window, cx));
                    }))
        });
        self.show_menu(view, None, Point::default(), Anchor::TopRight, p.dark, window, cx);
    }
    fn show_menu(&mut self, view: Entity<PopupMenu>, record: Option<RecordId>, position: Point<Pixels>, anchor: Anchor, dark: bool, window: &mut Window, cx: &mut Context<Self>) {
        let position = if record.is_some() {
            let estimate = view.read(cx).estimated_size(window);
            let viewport = window.viewport_size(); let margin = px(10.0);
            point(position.x.clamp(margin, (viewport.width - estimate.width - margin).max(margin)),
                position.y.clamp(margin, (viewport.height - estimate.height - margin).max(margin)))
        } else { position };
        let focus = window.focused(cx).unwrap_or_else(|| self.focus.clone());
        let id = view.entity_id();
        let dismiss = cx.subscribe_in(&view, window, move |this, _, _: &DismissEvent, _, cx| {
            if this.menu.as_ref().is_some_and(|menu| menu.view.entity_id() == id) { this.menu = None; cx.notify(); }
        });
        window.focus(&view.focus_handle(cx), cx);
        self.menu = Some(Menu { view, position, anchor, record, return_focus: focus, waiting_preview: false, ocr_item: None, dark, _dismiss: dismiss });
        cx.notify();
    }
    pub(super) fn record_menu(&mut self, id: RecordId, position: Point<Pixels>, p: Palette, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Opening | Phase::Opened) || window.has_active_dialog(cx) { return; }
        self.scrolling.stop();
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id).cloned() else { return; };
        self.close_menu(window, cx); self.selected = Some(id);
        let model = self.previews.get(&id).and_then(|preview| preview.model.clone());
        if model.is_none() && self.previews.get(&id).is_some_and(|preview| preview.loading) {
            let focus = window.focused(cx).unwrap_or_else(|| self.focus.clone());
            let view = PopupMenu::build(window, cx, move |menu, _, _| surface(menu, p, false).action_context(focus).min_w(px(158.0)).max_w(px(158.0))
                .dismiss_on_pointer_leave(Duration::from_millis(150)).label("加载中…"));
            self.show_menu(view, Some(id), position, Anchor::TopLeft, p.dark, window, cx);
            if let Some(menu) = &mut self.menu { menu.waiting_preview = true; }
            return;
        }
        let image = model.as_ref().map_or(entry.primary_type != PrimaryType::Files && entry.image.is_some(), |model| matches!(&model.body, PreviewBody::Image));
        let has_ocr = entry.has_ocr;
        let ocr_item = image.then_some(1 + usize::from(has_ocr));
        let weak = cx.entity().downgrade(); let tags = self.tags.items.clone(); let active = self.tags.selected;
        let busy = self.clearing || self.mutating.contains(&id); let stale = self.loading || self.reload_pending;
        let focus = window.focused(cx).unwrap_or_else(|| self.focus.clone());
        let restoring = self.restoring.is_some();
        let ocr_busy = self.ocr.busy(id);
        let view = PopupMenu::build(window, cx, move |menu, window, cx| {
            let restore = weak.clone();
            let mut menu = surface(menu, p, false).action_context(focus).min_w(px(158.0)).max_w(px(158.0))
                .dismiss_on_pointer_leave(Duration::from_millis(150))
                .item(PopupMenuItem::new("粘贴为纯文本").icon(menu_icon("text-box", None))
                    .disabled(restoring || !entry.formats.intersects(Formats::TEXT | Formats::HTML | Formats::RTF | Formats::FILES))
                    .on_click(move |_, window, cx| { let _ = restore.update(cx, |this, cx| this.restore_record(id, RestoreMode::PlainText, window, cx)); }));
            if let Some(model) = &model {
                if let PreviewBody::Color(color) = &model.body {
                    for (label, text) in [("复制为 HEX", color.hex.clone()), ("复制为 RGB", color.rgb.clone())] {
                        let copy = weak.clone();
                        menu = menu.item(PopupMenuItem::new(label).icon(menu_icon("text-box", None)).disabled(busy)
                            .on_click(move |_, window, cx| { let _ = copy.update(cx, |this, cx| this.mutate_record(id, Mutation::Color(text.clone()), window, cx)); }));
                    }
                }
            }
            if image {
                if has_ocr {
                    let restore = weak.clone();
                    menu = menu.item(PopupMenuItem::new("复制 OCR 文字").icon(menu_icon("text-box", None)).disabled(restoring)
                        .on_click(move |_, window, cx| { let _ = restore.update(cx, |this, cx| this.restore_record(id, RestoreMode::Ocr, window, cx)); }));
                }
                let ocr = weak.clone();
                menu = menu.item(PopupMenuItem::new(if has_ocr { "重新 OCR 识别" } else { "解析 OCR" })
                    .icon(menu_icon("text-box", None)).disabled(ocr_busy)
                    .on_click(move |_, window, cx| { let _ = ocr.update(cx, |this, cx| this.recognize_record(id, window, cx)); }));
                let qr = weak.clone();
                menu = menu.item(PopupMenuItem::new("解析二维码").icon(menu_icon("text-box", None)).disabled(restoring)
                    .on_click(move |_, window, cx| { let _ = qr.update(cx, |this, cx| this.record_action(id, super::history::RecordAction::Qr(QrAction::Parse), window, cx)); }));
            }
            if matches!(entry.primary_type, PrimaryType::Text | PrimaryType::Html | PrimaryType::Url) {
                let qr = weak.clone();
                menu = menu.item(PopupMenuItem::new("生成二维码").icon(menu_icon("text-box", None)).disabled(restoring)
                    .on_click(move |_, window, cx| { let _ = qr.update(cx, |this, cx| this.record_action(id, super::history::RecordAction::Qr(QrAction::Generate), window, cx)); }));
            }
            if entry.primary_type == PrimaryType::Url || model.as_ref().is_some_and(|model| matches!(&model.body, PreviewBody::Link(_))) {
                let open = weak.clone();
                menu = menu.item(PopupMenuItem::new("在浏览器中打开").icon(menu_icon("open-in-new", None))
                    .on_click(move |_, window, cx| { let _ = open.update(cx, |this, cx| this.open_link(id, window, cx)); }));
            }
            if !tags.is_empty() {
                let bind = weak.clone();
                menu = menu.submenu_with_icon(Some(menu_icon("tag", None)), "标签管理", window, cx, move |submenu, _, _| {
                    let mut submenu = surface(submenu, p, false).min_w(px(118.0)).max_w(px(118.0));
                    for tag in tags.iter() {
                        let tagged = entry.tags.iter().any(|current| current.id == tag.id); let tag_id = tag.id;
                        let color = u32::from_str_radix(tag.color.trim_start_matches('#'), 16).map(rgb).map(Hsla::from).unwrap_or(p.primary);
                        let name = SharedString::from(tag.name.clone()); let bind = bind.clone();
                        submenu = submenu.option(PopupMenuItem::new(name).icon(menu_icon(if tagged { "check" } else { "circle" }, Some(color)))
                            .checked(tagged).disabled(busy || stale).on_click(move |_, window, cx| {
                                let _ = bind.update(cx, |this, cx| this.mutate_record(id, Mutation::Binding { tag: tag_id, bound: !tagged }, window, cx));
                            }));
                    }
                    if let Some(tag) = active.filter(|_| !entry.tags.is_empty()) {
                        let bind = bind.clone();
                        submenu = submenu.separator().item(PopupMenuItem::new("移除当前标签").icon(menu_icon("delete", None))
                            .disabled(busy || stale).on_click(move |_, window, cx| {
                                let _ = bind.update(cx, |this, cx| this.mutate_record(id, Mutation::Binding { tag, bound: false }, window, cx));
                            }));
                    }
                    submenu
                });
            }
            menu.separator().item(PopupMenuItem::new("删除").icon(menu_icon("delete", None)).disabled(busy)
                .on_click(move |_, window, cx| { let _ = weak.update(cx, |this, cx| this.mutate_record(id, Mutation::Delete, window, cx)); }))
        });
        self.show_menu(view, Some(id), position, Anchor::TopLeft, p.dark, window, cx);
        if let Some(menu) = &mut self.menu { menu.ocr_item = ocr_item; }
    }
    pub(super) fn sync_ocr_menu(&self, cx: &mut Context<Self>) {
        if let Some(menu) = &self.menu {
            if let (Some(id), Some(index)) = (menu.record, menu.ocr_item) {
                menu.view.update(cx, |menu, cx| menu.set_item_disabled(index, self.ocr.busy(id), cx));
            }
        }
    }
    pub(super) fn refresh_menu_palette(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_mut() else { return; };
        let mode = cx.global::<crate::ui::configuration::ConfigurationGlobal>().0.read(cx).projected.settings.color_mode;
        let dark = Palette::uses_dark(mode, window.appearance());
        if menu.dark == dark { return; }
        let palette = Palette::new(dark);
        let toolbar = menu.record.is_none();
        menu.view.update(cx, |menu, cx| refresh_palette(menu, palette, toolbar, cx));
        menu.dark = dark;
    }
    pub(super) fn complete_menu_preview(&mut self, id: RecordId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_ref().filter(|menu| menu.record == Some(id) && menu.waiting_preview) else { return; };
        let position = menu.position;
        let p = Palette::for_window(cx.global::<crate::ui::configuration::ConfigurationGlobal>().0.read(cx).projected.settings.color_mode, window);
        self.record_menu(id, position, p, window, cx);
    }
    fn open_link(&mut self, id: RecordId, window: &mut Window, cx: &mut Context<Self>) {
        if self.services.is_shutting_down() { return; }
        let service = self.services.previews.clone(); let generation = self.generation;
        let desktop = cx.global::<crate::host::DesktopGlobal>().0.downgrade();
        let task = self.services.runtime.spawn(async move { service.external_link(id).await });
        cx.spawn_in(window, async move |weak, cx| {
            match task.await.map_err(Error::from).and_then(|result| result) {
                Ok(link) => { let _ = desktop.update(cx, |desktop, cx| desktop.open_external_link(&link, cx)); },
                Err(Error::Cancelled) => {},
                Err(error) => {
                    tracing::warn!(%error, "打开链接失败");
                    let message: SharedString = format!("打开链接失败：{error}").into();
                    let delivered = weak.update_in(cx, |this, window, cx| {
                        if this.generation != generation || !this.can_present_feedback(window) || this.services.is_shutting_down() { return false; }
                        dialogs::message(message.clone(), window, cx); true
                    }).unwrap_or(false);
                    if !delivered { super::report_operation_failure(&desktop, message, cx); }
                },
            }
        }).detach();
    }
    fn mutate_record(&mut self, id: RecordId, operation: Mutation, window: &mut Window, cx: &mut Context<Self>) {
        if self.clearing || self.services.is_shutting_down() || !self.mutating.insert(id) { return; }
        let services = self.services.clone();
        let desktop = cx.global::<crate::host::DesktopGlobal>().0.downgrade();
        let execute = operation.clone();
        let task = self.services.runtime.spawn({ let services = services.clone(); async move {
            let label = execute.label();
            let result = match execute {
                Mutation::Delete => services.clipboard.delete(id).await,
                Mutation::Binding { tag, bound } => services.tags.set_binding(id, tag, bound).await.map(|()| true),
                Mutation::Color(text) => services.clipboard.write(ClipboardSnapshot { content: TextContent { text: Some(text), ..TextContent::default() }, image: None }, &tokio_util::sync::CancellationToken::new()).await.map(|()| true),
            };
            if let Err(error) = &result { tracing::warn!(%error, operation = label, "剪贴板项目操作失败"); }
            result
        }});
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let failure: Option<SharedString> = match &result {
                Ok(true) => None, Ok(false) => Some("删除失败".into()),
                Err(error) => Some(format!("{}失败：{error}", operation.label()).into()),
            };
            let delivered = weak.update_in(cx, |this, window, cx| {
                this.mutating.remove(&id);
                if !this.is_open() || services.is_shutting_down() { cx.notify(); return false; }
                // Committed mutations publish authoritative history/tag events.
                // The completion only releases busy state and reports failures.
                if !this.can_present_feedback(window) { cx.notify(); return false; }
                if let Some(message) = &failure { dialogs::message(message.clone(), window, cx); }
                cx.notify(); failure.is_some()
            }).unwrap_or(false);
            if !delivered { if let Some(message) = failure { super::report_operation_failure(&desktop, message, cx); } }
        }).detach(); cx.notify();
    }
    pub(super) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() { window.focus(&menu.return_focus, cx); cx.notify(); }
    }
    pub(super) fn menu_layer(&self, window: &Window) -> Option<AnyElement> {
        let menu = self.menu.as_ref().filter(|menu| menu.record.is_some())?;
        Some(deferred(anchored().child(div().w(window.viewport_size().width).h(window.viewport_size().height)
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .child(anchored().position(menu.position).anchor(menu.anchor).keep_position().child(menu.view.clone()))))
            .with_priority(gpui_kit::base::POPUP_PRIORITY).into_any_element())
    }
    pub(super) fn toolbar_menu_layer(&self) -> Option<AnyElement> {
        let menu = self.menu.as_ref().filter(|menu| menu.record.is_none())?;
        // Anchor to the actual 28px trigger container, not a window coordinate
        // inferred from toolbar padding. This follows layout and slide motion.
        Some(deferred(anchored().position_mode(AnchoredPositionMode::Local)
            .position(point(px(28.0), px(34.0))).anchor(Anchor::TopRight).keep_position()
            .child(div().on_scroll(|_, _, cx| cx.stop_propagation()).child(menu.view.clone())))
            .with_priority(gpui_kit::base::POPUP_PRIORITY).into_any_element())
    }
    fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clearing { return; }
        self.menu = None;
        let weak = cx.entity().downgrade();
        dialogs::confirm("确定要清空所有剪贴板历史记录吗？此操作不可撤销。", window, cx, move |window, cx| {
            let _ = weak.update(cx, |this, cx| this.clear_history(window, cx));
        });
        cx.notify();
    }
    fn clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clearing || self.services.is_shutting_down() { return; }
        self.clearing = true;
        let services = self.services.clone();
        let desktop = cx.global::<crate::host::DesktopGlobal>().0.downgrade();
        let clipboard = services.clipboard.clone();
        let task = self.services.runtime.spawn(async move {
            let result = clipboard.clear_history().await;
            if let Err(error) = &result { tracing::warn!(%error, "清空剪贴板历史失败"); }
            result
        });
        // The service owns an accepted mutation. Hiding a view must not cancel
        // a committed clear or detach it from shutdown draining.
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let failure: Option<SharedString> = result.as_ref().err().map(|error| format!("清空历史失败：{error}").into());
            let delivered = weak.update_in(cx, |this, window, cx| {
                this.clearing = false;
                if !this.is_open() || services.is_shutting_down() { cx.notify(); return false; }
                if !this.can_present_feedback(window) { cx.notify(); return false; }
                if let Some(message) = &failure { dialogs::message(message.clone(), window, cx); }
                cx.notify(); failure.is_some()
            }).unwrap_or(false);
            if !delivered { if let Some(message) = failure { super::report_operation_failure(&desktop, message, cx); } }
        }).detach();
        cx.notify();
    }
}
fn menu_icon(name: &str, color: Option<Hsla>) -> Icon {
    let icon = Icon::default().path(format!("superpaste/icons/mdi/{name}.svg"));
    match color { Some(color) => icon.text_color(color), None => icon }
}
fn shadow(p: Palette, blur: f32, y: f32, alpha: f32) -> BoxShadow {
    BoxShadow { color: black().opacity(if p.dark { alpha * 3.0 } else { alpha }), offset: point(px(0.0), px(y)), blur_radius: px(blur), spread_radius: px(0.0), inset: false }
}
fn surface(menu: PopupMenu, p: Palette, toolbar: bool) -> PopupMenu {
    menu.item_height(px(if toolbar { 28.0 } else { 24.0 })).item_gap(px(0.0)).item_font_size(px(12.0))
        .icon_size(px(if toolbar { 16.0 } else { 12.0 })).icon_gap(px(if toolbar { 8.0 } else { 6.0 }))
        .content_padding(px(4.0)).pointer_submenus(px(2.0), px(10.0), px(10.0))
        .appearance(menu_appearance(p, toolbar))
        .entrance(Duration::from_millis(if toolbar { 150 } else { 100 }), point(px(0.0), px(if toolbar { -3.0 } else { 0.0 })))
}

fn refresh_palette(menu: &mut PopupMenu, p: Palette, toolbar: bool, cx: &mut Context<PopupMenu>) {
    menu.set_appearance(menu_appearance(p, toolbar), cx);
    if toolbar { menu.set_item_appearance(1, toolbar_danger(p), cx); }
    for submenu in menu.submenus() {
        submenu.update(cx, |menu, cx| refresh_palette(menu, p, false, cx));
    }
}

fn menu_appearance(p: Palette, toolbar: bool) -> PopupMenuAppearance {
    let shadows = if toolbar { vec![shadow(p, 16.0, 4.0, 0.12)] }
        else { vec![shadow(p, 8.0, 2.0, 0.12), shadow(p, 3.0, 1.0, 0.08)] };
    PopupMenuAppearance {
        surface: div().bg(p.bg_elevated).border_1().border_color(p.border_strong).rounded(px(6.0)).shadow(shadows).style().clone(),
        separator: div().h(px(1.0)).border(px(0.0)).p_0().my(px(2.0)).mx(px(6.0)).bg(p.border_light).style().clone(),
        item: if toolbar { toolbar_item(p) } else { item_appearance(p) },
        submenu_indicator: Some(menu_icon("chevron-right", Some(p.text_secondary)).with_size(px(10.0))),
    }
}

fn toolbar_item(p: Palette) -> MenuItemAppearance {
    let mut item = item_appearance(p);
    item.style = div().h_auto().min_h(px(28.0)).px(px(8.0)).py(px(4.0)).rounded(px(4.0)).line_height(relative(1.4)).style().clone();
    item.foreground_transition = Duration::from_millis(100);
    item.border_transition = Duration::from_millis(100);
    item.press_offset = Some((px(1.0), Duration::from_millis(100)));
    item
}

fn toolbar_danger(p: Palette) -> MenuItemAppearance {
    let mut item = toolbar_item(p);
    item.style = div().h_auto().min_h(px(28.0)).px(px(8.0)).py(px(4.0)).mt(px(4.0))
        .border_t_1().border_color(p.border_light).rounded_t(px(0.0)).rounded_b(px(4.0)).line_height(relative(1.4)).style().clone();
    item.foreground = p.danger; item.hovered_foreground = p.danger;
    item.hovered_background = p.danger_bg; item.pressed_background = p.danger_bg;
    item
}

fn item_appearance(p: Palette) -> MenuItemAppearance {
    MenuItemAppearance {
        style: div().h_auto().min_h(px(24.0)).px(px(8.0)).py(px(3.0)).rounded(px(4.0)).line_height(relative(1.4)).style().clone(),
        foreground: p.text_primary, hovered_foreground: p.primary, disabled_foreground: p.text_tertiary,
        background: transparent_black(), hovered_background: p.selection, pressed_background: p.selection_hover,
        background_transition: Duration::from_millis(100),
        foreground_transition: Duration::ZERO, border_transition: Duration::ZERO, press_offset: None,
    }
}
