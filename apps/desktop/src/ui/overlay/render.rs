use std::{ops::Range, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Easing, MotionStatus, Transition, h_virtual_list, v_virtual_list, transition_with_status};
use gpui_kit::component::input::Input;
use super_paste_format::OverlayPosition;
use crate::ui::{configuration::ConfigurationGlobal, icon, palette::Palette, widgets::{self, TextButton, ToolbarButton}};
use super::{OverlayView, Phase};

impl Render for OverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::for_window(cx.global::<ConfigurationGlobal>().0.read(cx).projected.settings.color_mode, window);
        if self.phase == Phase::Hidden { return div().size_full().into_any_element(); }
        if self.entries.is_empty() && !self.previews.is_empty() { self.clear_previews(window); }
        let visible = matches!(self.phase, Phase::Opening | Phase::Opened);
        // Keep one curve across reversals: the Kit transition samples its old
        // timeline with the current policy before retargeting it.
        let easing = Easing::CubicBezier { x1: 0.25, y1: 0.46, x2: 0.45, y2: 0.94 };
        let motion = transition_with_status("overlay-slide", if visible { 0.0_f32 } else { 1.0 },
            Transition::new(Duration::from_millis(if visible { 100 } else { 180 })).easing(easing), window, cx);
        if matches!(motion.status, MotionStatus::Finished | MotionStatus::Idle) {
            if self.phase == Phase::Opening { self.phase = Phase::Opened; }
            else if self.phase == Phase::Closing {
                if let Err(error) = self.finish_closing(window, cx) { self.fail(error, window, cx); }
            }
        }
        self.advance_scroll(window, cx);
        let offset = if self.phase == Phase::Preparing { point(px(0.0), px(0.0)) } else { match self.position {
            OverlayPosition::Top => point(px(0.0), -self.dimensions.height * motion.value),
            OverlayPosition::Bottom => point(px(0.0), self.dimensions.height * motion.value),
            OverlayPosition::Left => point(-self.dimensions.width * motion.value, px(0.0)),
            OverlayPosition::Right => point(self.dimensions.width * motion.value, px(0.0)),
        } };
        let list = if self.vertical() { v_virtual_list(cx.entity(), "history", self.sizes.clone(), Self::render_range) }
            else { h_virtual_list(cx.entity(), "history", self.sizes.clone(), Self::render_range) }
            .with_fixed_item_size(self.list_item_size()).track_scroll(&self.scroll)
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| { if event.button == MouseButton::Left { this.scrolling.press(); } }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::drag_start))
            .when(self.scrolling.dragging(), |list| list.cursor(CursorStyle::ClosedHand));
        let empty = self.entries.is_empty(); let filtered = !self.query.is_empty() || self.tags.selected.is_some();
        div().size_full().relative().overflow_hidden().child(div().id("overlay").size_full().relative().left(offset.x).top(offset.y)
            .opacity(if self.phase == Phase::Preparing { 0.0 } else { (1.0 - motion.value).clamp(0.0, 1.0) }).track_focus(&self.focus).key_context("SuperPasteOverlay")
            .capture_key_down(cx.listener(Self::key_down))
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.shortcuts_visible = event.modifiers.platform || event.modifiers.control; cx.notify();
            }))
            .flex().flex_col().bg(p.window).border_1().border_color(p.border_strong).text_color(p.text_primary)
            .child(self.toolbar(p, window, cx))
            .when_some(self.error.clone().or_else(|| self.tags.error.clone()), |root, error| root.child(div().flex().items_center().justify_between().gap(px(8.0)).px(px(12.0)).py(px(6.0)).text_size(px(12.0)).text_color(p.danger)
                .child(div().id("history-error").role(Role::Alert).flex_1().child(error))
                .child(TextButton::new("retry-history", "重试", p).on_click(cx.listener(|this, _, window, cx| { this.refresh_tags(window, cx); this.refresh_ocr(window, cx); this.refresh(window, cx); })))))
            .child(div().flex_1().min_h_0().min_w_0().px(px(12.0)).py(px(10.0)).when(!empty, |body| body.child(list))
                .when(empty, |body| body.child(div().size_full().flex().flex_col().items_center().justify_center().gap(px(8.0))
                    .when(self.loading || self.debounce.is_some(), |empty| empty.child(widgets::spinner("history-loading", p, window)))
                    .when(!self.loading && self.debounce.is_none(), |empty| empty
                        .child(div().size(px(44.0)).rounded(px(8.0)).border_1().border_color(p.border_medium).bg(p.bg_primary).flex().items_center().justify_center().text_color(p.text_tertiary).child(icon("clipboard-outline", 22.0)))
                        .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).child(if filtered { "未找到匹配的记录" } else { "暂无剪贴板记录" }))
                        .child(div().text_size(px(12.0)).text_color(p.text_secondary).child(if filtered { "尝试其他关键词或切换分类" } else { "复制内容后将在此处显示" }))))))
            .child(self.scroll_events(cx))
            .when_some(self.menu_layer(window), |root, menu| root.child(menu))
        ).into_any_element()
    }
}
impl OverlayView {
    fn toolbar(&mut self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let motion = self.search_motion.advance(self.search_visible, self.dimensions.width, window, cx);
        let search_in_flow = self.search_visible || motion.animating;
        let focused = self.search.read(cx).focus_handle(cx).contains_focused(window, cx);
        let border = gpui_kit::base::transition((ElementId::View(cx.entity_id()), "history-search-border"),
            if focused || (search_in_flow && self.search_hovered) { p.primary } else { p.control_border },
            Transition::new(std::time::Duration::from_millis(100)), window, cx);
        let search = div().id("history-search-container").min_w_0().max_w_full().overflow_hidden()
            // Keep a nonzero caret area for native IME while search is collapsed.
            // Opacity still hides the field until input reveals it.
            .w(if search_in_flow { motion.search.width } else { px(1.0) }).px(motion.search.padding).opacity(motion.search.opacity)
            .aria_hidden(!self.search_visible)
            .when(!search_in_flow, |search| search.absolute().left(px(if self.vertical() { 40.0 } else { 48.0 })).top(px(8.0)))
            .child(div().id("history-search-field").w_full().min_w_0().h(px(28.0)).rounded(px(4.0)).border_1()
                .border_color(border).bg(p.bg_primary)
                .when(!search_in_flow, |field| field.border_0())
                .on_hover(cx.listener(|this, hovered, _, cx| {
                    if this.search_hovered != *hovered { this.search_hovered = *hovered; cx.notify(); }
                }))
                .child(Input::new(&self.search).text_direction(gpui::TextDirection::Ltr).aria_label("搜索剪贴板历史").tab_index(if self.search_visible { 0 } else { -1 })
                    .appearance(false).bordered(false).focus_bordered(false).w_full().min_w_0().h_full()
                    .px(px(if search_in_flow { 9.0 } else { 0.0 })).py(px(if search_in_flow { 4.0 } else { 0.0 })).rounded(px(4.0)).text_size(px(12.0))));
        div().relative().h(px(44.0)).flex_shrink_0().px(px(if self.vertical() { 8.0 } else { 12.0 })).flex().items_center().gap(px(if self.vertical() { 4.0 } else { 8.0 }))
            .bg(p.titlebar).border_b_1().border_color(p.border_medium)
            .child(ToolbarButton::new("search-toggle", "搜索剪贴板历史", icon("magnify", 18.0), p)
                .expanded(self.search_visible).on_click(cx.listener(|this, _, window, cx| {
                    this.set_search_visible(!this.search_visible, window, cx);
                })))
            .child(scaled(search, size(if search_in_flow { motion.search.scale } else { 1.0 }, 1.0)))
            .when(!self.search_visible || motion.animating, |bar| bar.child(scaled(self.tag_bar(p, window, cx)
                .max_w(relative(motion.tabs.max_width)).opacity(motion.tabs.opacity), size(motion.tabs.scale, motion.tabs.scale))))
            .child(div().flex_shrink_0().ml_auto().pl(px(8.0)).border_l_1().border_color(p.border_medium)
                .child(div().relative().size(px(28.0)).flex().items_start().justify_start()
                    .child(ToolbarButton::new("overlay-more", "更多操作", svg().path("superpaste/feedback/ellipsis.svg").size(px(16.0)), p)
                    .expanded(self.menu.as_ref().is_some_and(|menu| menu.record.is_none()))
                    .on_click(cx.listener(move |this, _, window, cx| this.toolbar_menu(p, window, cx))))
                    .when_some(self.toolbar_menu_layer(), |container, menu| container.child(menu))))
    }
    fn render_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = Palette::for_window(cx.global::<ConfigurationGlobal>().0.read(cx).projected.settings.color_mode, window);
        if range.end + 5 >= self.entries.len() && self.cursor.is_some() && !self.loading && !self.reload_pending && !self.filter_changed && self.debounce.is_none() && self.error.is_none() {
            cx.defer_in(window, |this, window, cx| this.request(true, window, cx));
        }
        let extent = self.list_item_size();
        let entries: Vec<_> = range.filter_map(|index| self.entries.get(index).cloned().map(|entry| (index, entry))).collect();
        self.reconcile_previews(&entries, window, cx);
        entries.into_iter().map(|(index, entry)| {
            let card = self.render_card(index, &entry, p, window, cx);
            div().w(extent.width).h(extent.height).flex().justify_center().child(card).into_any_element()
        }).collect()
    }
}
