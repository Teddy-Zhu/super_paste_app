use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::{Button, Easing, Transition, transition};
use super_paste_core::Error;
use super_paste_format::{Tag, TagId};
use tokio_util::{sync::{CancellationToken, DropGuard}, task::AbortOnDropHandle};
use crate::ui::{palette::Palette, widgets};
use super::{OverlayView, Phase};

pub(super) struct Tags {
    pub items: Arc<[Tag]>, pub selected: Option<TagId>, pub switching: bool, pub error: Option<SharedString>,
    loading: bool, dirty: bool, ticket: u64, cancel: Option<DropGuard>, task: Option<Task<()>>, scroll: ScrollHandle,
    geometry: Rc<RefCell<HashMap<Option<TagId>, Bounds<Pixels>>>>,
}
impl Tags {
    pub fn new(selected: Option<TagId>) -> Self {
        Self { items: Arc::default(), selected, switching: false, error: None, loading: false, dirty: false,
            ticket: 0, cancel: None, task: None, scroll: ScrollHandle::new(), geometry: Default::default() }
    }
    pub fn suspend(&mut self) {
        self.ticket = self.ticket.wrapping_add(1); self.cancel = None;
        // Keep the sole accepted read until its database work has drained.
        self.dirty = false; self.switching = false;
    }
}
impl OverlayView {
    /// A single accepted read and one dirty bit coalesce count/binding events.
    /// No polling task or additional tag-statistics request is needed.
    pub(super) fn refresh_tags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Hidden || self.services.is_shutting_down() { return; }
        if self.tags.loading { self.tags.dirty = true; return; }
        self.tags.loading = true; self.tags.dirty = false; self.tags.ticket = self.tags.ticket.wrapping_add(1);
        let ticket = self.tags.ticket; let service = self.services.tags.clone();
        let cancel = CancellationToken::new();
        self.tags.cancel = Some(cancel.clone().drop_guard());
        let task = AbortOnDropHandle::new(self.services.runtime.spawn(async move { service.list(&cancel).await }));
        self.tags.task = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                this.tags.cancel = None; this.tags.loading = false; this.tags.task = None;
                if this.phase == Phase::Hidden || this.tags.ticket != ticket {
                    if this.tags.dirty { this.refresh_tags(window, cx); }
                    return;
                }
                let mut changed = this.tags.error.is_some();
                match result {
                    Ok(items) => {
                        this.tags.error = None;
                        let missing = this.tags.selected.is_some_and(|id| !items.iter().any(|tag| tag.id == id));
                        if this.tags.items.as_ref() != items.as_slice() {
                            this.tags.items = items.into(); changed = true;
                        }
                        if missing {
                            changed = true;
                            this.tags.selected = None; this.reset_query(window);
                            if this.search.read(cx).is_composing() { this.reload_pending = true; } else { this.request(false, window, cx); }
                        }
                    },
                    Err(error) => { tracing::warn!(%error, "读取覆盖层标签失败"); this.tags.error = Some(format!("读取标签失败：{error}").into()); changed = true; },
                }
                if this.tags.dirty { this.refresh_tags(window, cx); }
                if changed { cx.notify(); }
            });
        }));
    }
    fn select_tag(&mut self, id: Option<TagId>, window: &mut Window, cx: &mut Context<Self>) {
        if self.tags.selected == id || self.tags.switching || self.search.read(cx).is_composing() || !matches!(self.phase, Phase::Opening | Phase::Opened) { return; }
        self.tags.selected = id; self.reset_query(window); self.tags.switching = true;
        self.request(false, window, cx);
    }
    pub(super) fn tag_bar(&self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let mut tabs = vec![(None, SharedString::from("All"), Hsla::from(rgb(0x10b981)), 0)];
        tabs.extend(self.tags.items.iter().map(|tag| (Some(tag.id), SharedString::from(tag.name.clone()),
            u32::from_str_radix(tag.color.trim_start_matches('#'), 16).map(rgb).map(Hsla::from).unwrap_or(p.primary), tag.count)));
        let ids: Vec<_> = tabs.iter().map(|tab| tab.0).collect();
        let bounds = self.tags.geometry.borrow().get(&self.tags.selected).copied();
        let indicator = bounds.map(|bounds| {
            let policy = Transition::new(Duration::from_millis(200)).easing(Easing::CubicBezier { x1: 0.0, y1: 0.0, x2: 0.2, y2: 1.0 });
            let x = transition("tag-indicator-x", bounds.origin.x, policy.clone(), window, cx);
            let width = transition("tag-indicator-width", bounds.size.width, policy, window, cx);
            div().absolute().top(px(3.0)).left(x).w(width).h(px(28.0)).rounded(px(4.0)).border_1().border_color(p.primary).bg(p.selection).shadow(widgets::control_shadow(p))
        });
        let geometry = self.tags.geometry.clone(); let weak = cx.entity().downgrade();
        let row = div().relative().flex().items_center().gap(px(4.0)).py(px(3.0)).px(px(1.0))
            // Paint the indicator first; layout remains independent of the
            // absolute child. Geometry below reads only the tab children.
            .when_some(indicator, |row, indicator| row.child(indicator))
            .children(tabs.into_iter().map(|(id, name, color, count)| {
                let active = self.tags.selected == id; let busy = active && self.tags.switching;
                let label = if count > 0 { format!("{name} ({count})").into() } else { name.clone() };
                Button::new(id.map_or_else(|| "tag-all".into(), |id| SharedString::from(format!("tag-{id}"))))
                    .accessibility_label(label).selected(active).aria_toggled(if active { Toggled::True } else { Toggled::False })
                    .relative().h(px(28.0)).flex_shrink_0().px(px(9.0)).py(px(4.0)).border_1().border_color(transparent_black()).rounded(px(4.0))
                    .flex().items_center().gap(px(6.0)).text_color(if active { p.primary } else { p.text_secondary })
                    .hover(move |style| style.text_color(if active { p.primary } else { p.text_primary }))
                    .child(div().size(px(6.0)).flex_shrink_0().rounded_full().bg(color))
                    .child(div().text_size(px(12.0)).font_weight(FontWeight::MEDIUM).child(name))
                    .when(count > 0, |button| button.child(div().text_size(px(10.0)).text_color(if active { p.primary } else { p.text_tertiary }).child(count.to_string())))
                    .when(busy, |button| button.child(widgets::spinner_sized("tag-loading", p, 12.0, window).absolute().left(px(7.0)).top(px(8.0))))
                    .on_click(cx.listener(move |this, _, window, cx| this.select_tag(id, window, cx)))
            }))
            .on_children_prepainted(move |bounds, _, cx| {
                let skip = bounds.len().saturating_sub(ids.len());
                let bounds = &bounds[skip..]; let Some(first) = bounds.first() else { return; };
                let measured: HashMap<_, _> = ids.iter().copied().zip(bounds.iter().map(|bounds|
                    Bounds::new(point(bounds.origin.x - first.origin.x + px(1.0), px(3.0)), bounds.size))).collect();
                if *geometry.borrow() == measured { return; }
                *geometry.borrow_mut() = measured;
                let weak = weak.clone(); cx.defer(move |cx| { let _ = weak.update(cx, |_, cx| cx.notify()); });
            });
        div().id("overlay-tag-strip").flex_1().min_w_0().overflow_x_scroll().track_scroll(&self.tags.scroll)
            .capture_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let scroll = &this.tags.scroll;
                let limit = scroll.max_offset().x;
                // Keep the reference's one-pixel overflow tolerance. An
                // overflowing strip owns the gesture even at its edge; history must not
                // consume the unused delta or apply it a second time.
                if limit <= px(1.0) { return; }
                let offset = scroll.offset();
                let next = point((offset.x + super::scrolling::wheel_delta(event.delta)).clamp(-limit, Pixels::ZERO), offset.y);
                if next != offset { scroll.set_offset(next); cx.notify(); }
                window.prevent_default(); cx.stop_propagation();
            }))
            .child(row)
    }
}
