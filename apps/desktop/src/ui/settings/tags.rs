use std::{cell::Cell, rc::Rc, sync::Arc};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::{WindowExt, dialog::DialogButtonProps, tooltip::Tooltip};
use super_paste_core::{Error, events::AppEvent};
use super_paste_format::{Tag, TagId};
use tokio_util::{sync::{CancellationToken, DropGuard}, task::AbortOnDropHandle};
use super::PaneContext;
use crate::ui::{forms, icon, palette::Palette, widgets::{IconButton, TextButton, spinner}};
mod editor;

pub struct TagsPane {
    context: PaneContext, tags: Arc<[Tag]>, loading: bool, reload_requested: bool,
    visible: bool, read_generation: u64, read_cancel: Option<DropGuard>, read_task: Option<Task<()>>,
    error: Option<SharedString>, operation_error: Option<SharedString>, deleting: Option<TagId>, subscriptions: Vec<Subscription>, event_task: Task<()>,
}
impl TagsPane {
    pub fn new(context: PaneContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut updates = context.services.events.subscribe();
        let event_task = cx.spawn(async move |weak, cx| {
            loop {
                match updates.recv().await {
                    Ok(AppEvent::TagsChanged { .. }) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if weak.update(cx, |this, cx| this.reload(cx)).is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
            }
        });
        let subscriptions = vec![
            cx.observe(&context.configuration, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            cx.observe_window_visibility(window, |this, _, window, cx| this.set_visible(window.is_visible(), cx)),
        ];
        let mut pane = Self { context, tags: Arc::from([]), loading: false, reload_requested: false,
            visible: window.is_visible(), read_generation: 0, read_cancel: None, read_task: None,
            error: None, operation_error: None, deleting: None, subscriptions, event_task };
        pane.reload(cx);
        pane
    }
    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible { return; }
        self.visible = visible;
        if visible {
            if self.reload_requested { self.reload(cx); }
        } else if self.read_task.is_some() {
            self.read_cancel = None;
            self.read_generation = self.read_generation.wrapping_add(1);
            // Keep the read slot until the ordered database worker returns.
            self.reload_requested = true;
        }
        cx.notify();
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_requested = true;
        if self.read_task.is_some() || !self.visible { return; }
        self.reload_requested = false;
        self.read_generation = self.read_generation.wrapping_add(1);
        let generation = self.read_generation;
        self.loading = true; self.error = None; cx.notify();
        let tags = self.context.services.tags.clone();
        let cancel = CancellationToken::new();
        self.read_cancel = Some(cancel.clone().drop_guard());
        let task = AbortOnDropHandle::new(self.context.services.runtime.spawn(async move { tags.list(&cancel).await }));
        self.read_task = Some(cx.spawn(async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update(cx, |this, cx| {
                this.read_cancel = None; this.read_task = None; this.loading = false;
                if this.read_generation != generation {
                    if this.visible && this.reload_requested { this.reload(cx); }
                    return;
                }
                if this.reload_requested { this.reload(cx); return; }
                match result { Ok(tags) => this.tags = tags.into(), Err(error) => this.error = Some(error.to_string().into()) }
                cx.notify();
            });
        }));
    }
    fn edit(&self, tag: Option<Tag>, window: &mut Window, cx: &mut Context<Self>) {
        editor::open(self.context.clone(), tag, cx.entity().downgrade(), window, cx);
    }
    fn confirm_delete(&mut self, tag: Tag, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting.is_some() { return; }
        self.deleting = Some(tag.id); cx.notify();
        let id = tag.id;
        let weak = cx.entity().downgrade();
        let closed = weak.clone();
        let accepted = Rc::new(Cell::new(false));
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let accepted_ok = accepted.clone(); let accepted_close = accepted.clone(); let weak = weak.clone(); let closed = closed.clone();
            dialog.title("Super Paste").child(format!("删除标签“{}”？相关剪贴板内容会保留，标签绑定将被移除。", tag.name))
                .button_props(DialogButtonProps::default().ok_text("确定").cancel_text("取消").show_cancel(true)
                    .on_ok(move |_, window, cx| { accepted_ok.set(true); let _ = weak.update(cx, |this, cx| this.delete(id, window, cx)); true }))
                .on_close(move |_, _, cx| { if !accepted_close.get() { let _ = closed.update(cx, |this, cx| { if this.deleting == Some(id) { this.deleting = None; cx.notify(); } }); } })
        });
    }
    fn delete(&mut self, id: TagId, window: &mut Window, cx: &mut Context<Self>) {
        if self.context.services.is_shutting_down() { return; }
        let services = self.context.services.clone(); let tags = services.tags.clone();
        let task = services.runtime.spawn(async move { tags.delete(id).await });
        let owner = window.window_handle();
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let failure = result.as_ref().err().cloned();
            let delivered = weak.update(cx, |this, cx| {
                // A leaving page can still exist during its exit animation.
                // Its liveness alone cannot establish visible error delivery.
                let active = this.context.is_active(cx.entity_id(), cx);
                if this.deleting == Some(id) { this.deleting = None; }
                if let Err(error) = result { this.operation_error = Some(error.to_string().into()); }
                else {
                    this.operation_error = None;
                    this.tags = this.tags.iter().filter(|tag| tag.id != id).cloned().collect::<Vec<_>>().into();
                    if active { this.reload(cx); }
                }
                cx.notify();
                active
            }).unwrap_or(false);
            if !delivered && !services.is_shutting_down() {
                if let Some(error) = failure {
                    let _ = owner.update(cx, |_, window, cx| crate::ui::dialogs::message(format!("删除标签失败：{error}"), window, cx));
                }
            }
        }).detach();
    }
}
impl Render for TagsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(self.context.configuration.read(cx).projected.settings.color_mode, window);
        let weak = cx.entity().downgrade(); let create = weak.clone(); let empty_create = weak.clone(); let retry = weak.clone();
        let busy = self.deleting.is_some();
        let error = self.operation_error.clone().or_else(|| self.error.clone());
        let empty = !self.loading && error.is_none() && self.tags.is_empty();
        let rows: Vec<_> = self.tags.iter().cloned().enumerate().map(|(index, tag)| {
            let edit = weak.clone(); let delete = weak.clone(); let double = weak.clone();
            let editing = tag.clone(); let deleting = tag.clone(); let double_tag = tag.clone();
            let color = parse_color(&tag.color);
            div().id(tag.id.0).flex().items_center().gap(px(12.0)).py(px(13.0)).px(px(16.0)).min_h(px(56.0))
                .when(index > 0, |row| row.border_t_1().border_color(p.border_light)).opacity(if self.deleting == Some(tag.id) { 0.5 } else { 1.0 })
                .child(div().id("tag-color").aria_hidden(true).size(px(10.0)).flex_shrink_0().rounded_full()
                    .when_some(color, |dot, color| dot.bg(color))
                    .when(color.is_none(), |dot| dot.border_1().border_color(p.danger).tooltip(|window, cx| Tooltip::new("标签颜色无效").build(window, cx))))
                .child(div().id("tag-info").flex_1().min_w_0()
                    .tooltip(move |window, cx| Tooltip::new(format!("创建于 {}\n更新于 {}", forms::local_timestamp(tag.created_at), forms::local_timestamp(tag.updated_at))).build(window, cx))
                    .child(div().text_size(px(13.0)).line_height(relative(1.4)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary).child(SharedString::from(tag.name.clone())))
                    .when_some(tag.description.clone(), |info, description| info.child(div().mt(px(3.0)).text_size(px(12.0)).line_height(px(18.0)).text_color(p.text_secondary).child(SharedString::from(description)))))
                .child(div().flex().gap(px(4.0)).flex_shrink_0()
                    .child(IconButton::new("edit-tag", format!("编辑标签：{}", tag.name), "pencil-outline", p).disabled(busy).tooltip("编辑标签")
                        .on_click(move |_, window, cx| { let _ = edit.update(cx, |this, cx| this.edit(Some(editing.clone()), window, cx)); cx.stop_propagation(); }))
                    .child(IconButton::new("delete-tag", format!("删除标签：{}", tag.name), "trash-can-outline", p).danger().disabled(busy).tooltip("删除标签")
                        .on_click(move |_, window, cx| { let _ = delete.update(cx, |this, cx| this.confirm_delete(deleting.clone(), window, cx)); cx.stop_propagation(); })))
                .on_click(move |event, window, cx| { if !busy && event.click_count() == 2 { let _ = double.update(cx, |this, cx| this.edit(Some(double_tag.clone()), window, cx)); } })
                .into_any_element()
        }).collect();
        let body = div().id("tag-settings-body").role(Role::Group).aria_label("标签管理").aria_busy(self.loading)
            .w_full().min_w_0().line_height(relative(1.4)).child(forms::heading("标签管理", "整理剪贴板内容的分类标签", p))
            .child(div().flex().items_center().justify_between().gap(px(12.0)).mb(px(14.0))
                .child(div().text_size(px(12.0)).font_features(forms::tabular()).text_color(p.text_secondary).child(format!("{} 个标签", self.tags.len())))
                .child(TextButton::new("create-tag", "新建标签", p).icon("plus").form(true).disabled_shadow(false).disabled(self.loading || busy)
                    .on_click(move |_, window, cx| { let _ = create.update(cx, |this, cx| this.edit(None, window, cx)); })))
            .when_some(error, |page, error| page.child(div().id("tag-list-error").role(Role::Alert).flex().items_center().justify_between().gap(px(12.0)).p(px(12.0)).mb(px(12.0)).border_1().border_color(p.danger).rounded(px(6.0)).text_size(px(12.0)).text_color(p.danger)
                .child(error).child(TextButton::new("reload-tags", "重新加载", p).form(false).disabled(self.loading || busy).on_click(move |_, _, cx| { let _ = retry.update(cx, |this, cx| { this.operation_error = None; this.reload(cx); }); }))))
            .when(self.loading && self.tags.is_empty(), |page| page.child(div().id("tag-list-loading").role(Role::Status).flex().flex_col().items_center().justify_center().py(px(48.0)).px(px(20.0))
                .child(div().mb(px(12.0)).child(spinner(("tag-loading", cx.entity_id()), p, window))).child("正在加载标签…")))
            .when(empty, |page| page.child(div().flex().flex_col().items_center().justify_center().py(px(48.0)).px(px(20.0))
                .child(div().id("tag-empty-icon").aria_hidden(true).size(px(48.0)).flex().items_center().justify_center().mb(px(14.0)).border_1().border_color(p.border_medium).rounded(px(12.0)).text_color(p.text_tertiary).bg(p.bg_secondary).child(icon("tag-outline", 24.0)))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).mb(px(6.0)).child("暂无标签"))
                .child(TextButton::new("empty-create-tag", "新建标签", p).form(false).on_click(move |_, window, cx| { let _ = empty_create.update(cx, |this, cx| this.edit(None, window, cx)); }))))
            .when(!self.tags.is_empty(), |page| page.child(forms::settings_group(p).children(rows)));
        forms::page("tag-settings", window).child(body)
    }
}
pub(super) fn parse_color(value: &str) -> Option<Hsla> {
    if value.len() != 7 || !value.starts_with('#') { return None; }
    u32::from_str_radix(&value[1..], 16).ok().map(|color| rgb(color).into())
}
