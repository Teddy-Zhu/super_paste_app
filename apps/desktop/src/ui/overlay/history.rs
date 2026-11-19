use std::{collections::HashSet, rc::Rc, sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::component::WindowExt;
use super_paste_core::{Error, clipboard::{RestoreMode, QrAction}};
use super_paste_format::{HistoryQuery, RecordId};
use tokio_util::sync::CancellationToken;
use super::{OverlayView, Phase, reader::Reply};

#[derive(Clone, Copy)]
enum Anchor { Current, First, Last }
#[derive(Clone, Copy)]
struct SelectionStep { anchor: Anchor, delta: isize }
#[derive(Default)]
pub(super) struct PendingSelection { steps: Vec<SelectionStep> }
impl PendingSelection {
    fn push(&mut self, step: SelectionStep) {
        if !matches!(step.anchor, Anchor::Current) { self.steps.clear(); }
        // Same-direction moves compose under clamping. Reversals must keep
        // their order until the new result length and selection are known.
        if matches!(step.anchor, Anchor::Current) {
            if let Some(last) = self.steps.last_mut().filter(|last| last.delta.signum() == step.delta.signum()) {
                last.delta = last.delta.saturating_add(step.delta); return;
            }
        }
        self.steps.push(step);
    }
}
pub(super) struct PendingRestore { mode: RestoreMode, index: Option<usize> }
#[derive(Clone, Copy)]
pub(super) enum RecordAction { Restore(RestoreMode), Qr(QrAction) }

/// Match the reference's logical digit first, then its physical Digit key.
/// Keypad digits remain valid through their logical value, not their position.
fn quick_selection_index(event: &KeyDownEvent) -> Option<usize> {
    let digit = |text: &str| match text.as_bytes() {
        [value @ b'1'..=b'9'] => Some(usize::from(*value - b'1')),
        _ => None,
    };
    if let Some(index) = digit(&event.keystroke.key) { return Some(index); }
    let mapping = match event.physical_key? {
        NativeKeyCode::Xkb(code) => keycode::KeyMapping::Xkb(u16::try_from(code).ok()?),
        NativeKeyCode::Windows(code) => keycode::KeyMapping::Win(code),
        NativeKeyCode::MacOS(code) => keycode::KeyMapping::Mac(code),
    };
    let code = keycode::KeyMap::try_from(mapping).ok()?.code?.to_string();
    digit(code.strip_prefix("Digit")?)
}

impl OverlayView {
    /// A notification can overtake the UI delivery of an older SQLite page.
    /// Retire its identity before applying removals so that reply cannot rebuild
    /// deleted cards or their cancelled previews. The reader still bounds work
    /// to its accepted read and one replaceable latest request.
    pub(super) fn invalidate_history_read(&mut self) {
        self.reader.suspend();
        self.query_generation = None;
        self.loading = false;
        self.appending = false;
        if self.phase != Phase::Hidden { self.reload_pending = true; }
        if self.phase == Phase::Preparing { self.preparation_dirty = true; }
    }

    pub(super) fn removed(&mut self, ids: &[RecordId], window: &mut Window, cx: &mut Context<Self>) -> bool {
        if ids.is_empty() { return false; }
        self.invalidate_history_read();
        let removed: HashSet<_> = ids.iter().copied().collect();
        if self.details.as_ref().is_some_and(|details| removed.contains(&details.id)) { self.dismiss_details(window, cx); }
        if self.menu.as_ref().and_then(|menu| menu.record).is_some_and(|id| removed.contains(&id)) { self.close_menu(window, cx); }
        if !self.entries.iter().any(|entry| removed.contains(&entry.id)) { return false; }
        self.previews.retain(|id, _| !removed.contains(id));
        let offset = self.scroll.offset();
        let extent = if self.vertical() { self.card_size().height + px(10.0) } else { self.card_size().width + px(10.0) };
        let distance = -f32::from(if self.vertical() { offset.y } else { offset.x });
        let anchor_index = (distance.max(0.0) / f32::from(extent)).floor() as usize;
        let preceding = |index: usize| self.entries.iter().take(index).filter(|entry| !removed.contains(&entry.id)).count();
        let selected_index = self.entries.iter().position(|entry| Some(entry.id) == self.selected).map(preceding).unwrap_or(0);
        let next_anchor = preceding(anchor_index);
        if self.selected.is_some_and(|id| removed.contains(&id)) { self.selected = None; self.pending_confirm = None; }
        self.entries.retain(|entry| !removed.contains(&entry.id));
        self.prune_removed_source_icons(cx); self.prune_rasters(window);
        if self.selected.is_none() { self.selected = self.entries.get(selected_index.min(self.entries.len().saturating_sub(1))).map(|entry| entry.id); }
        self.reconcile_ocr(window, cx);
        self.reconcile_details(window, cx);
        self.update_sizes();
        let distance = px(next_anchor.min(self.entries.len().saturating_sub(1)) as f32 * f32::from(extent) + distance - anchor_index as f32 * f32::from(extent));
        self.scroll.set_offset(if self.vertical() { point(offset.x, -distance) } else { point(-distance, offset.y) });
        true
    }
    pub(super) fn request(&mut self, append: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.services.is_shutting_down() || self.phase == Phase::Hidden { return; }
        if self.loading { self.reload_pending |= !append; return; }
        if append && (self.cursor.is_none() || self.filter_changed || self.reload_pending || self.debounce.is_some()) { return; }
        let target_len = if append || self.filter_changed { 24 } else { self.entries.len().max(self.initial_limit).max(24) };
        let query = HistoryQuery { search: self.query.clone(),
            tag_ids: self.tags.selected.into_iter().collect(),
            cursor: if append { self.cursor.clone() } else { None },
            limit: target_len.min(1000) as u32,
            ..HistoryQuery::default() };
        let notify = self.error.is_some() || self.entries.is_empty() || self.tags.switching;
        self.reload_pending = false; self.error = None;
        let known = Arc::from(if append { &[][..] } else { self.entries.as_slice() });
        tracing::debug!(append, filter_changed = self.filter_changed, rows = self.entries.len(), "请求历史窗口");
        match self.reader.request(query, append, target_len, known) {
            Ok(generation) => { self.query_generation = Some(generation); self.loading = true; self.appending = append; },
            Err(error) => { self.query_generation = None; self.pending_selection = None; self.pending_confirm = None; self.tags.switching = false; self.error = Some(error.to_string().into()); if self.phase == Phase::Preparing { if let Err(error) = self.prepared(window, cx) { self.fail(error, window, cx); } } },
        }
        if notify || self.error.is_some() { cx.notify(); }
    }
    pub(super) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Hidden || self.services.is_shutting_down() { return; }
        if self.debounce.is_some() || self.search.read(cx).is_composing() { self.reload_pending = true; return; }
        if self.phase == Phase::Preparing && self.frame.is_some() { self.reload_pending = true; return; }
        self.request(false, window, cx);
    }
    pub(super) fn accept(&mut self, reply: Reply, window: &mut Window, cx: &mut Context<Self>) {
        if self.query_generation != Some(reply.generation) || self.phase == Phase::Hidden { return; }
        let mut notify = self.entries.is_empty() || self.tags.switching;
        self.loading = false;
        self.tags.switching = false;
        match reply.result {
            Ok(page) => {
                notify |= self.cursor != page.next_cursor;
                self.cursor = page.next_cursor.clone();
                let mut changed = false;
                if reply.append {
                    let mut ids: HashSet<RecordId> = self.entries.iter().map(|entry| entry.id).collect();
                    let before = self.entries.len();
                    self.entries.extend(page.entries.iter().filter(|entry| ids.insert(entry.id)).cloned());
                    changed = self.entries.len() != before;
                } else {
                    let selected_index = self.entries.iter().position(|entry| Some(entry.id) == self.selected).unwrap_or(0);
                    let offset = self.scroll.offset(); let extent = if self.vertical() { self.card_size().height + px(10.0) } else { self.card_size().width + px(10.0) };
                    let distance = -f32::from(if self.vertical() { offset.y } else { offset.x });
                    let anchor_index = (distance.max(0.0) / f32::from(extent)).floor() as usize;
                    let anchor = self.entries.get(anchor_index).map(|entry| entry.id);
                    let follow_head = offset == Point::default() && self.entries.first().is_some_and(|entry| Some(entry.id) == self.selected);
                    let reordered = self.entries.len() != page.entries.len() || self.entries.iter().zip(&page.entries).any(|(old, new)| old.id != new.id);
                    let prefix = self.entries.iter().zip(&page.entries).take_while(|(old, new)| Arc::ptr_eq(old, new)).count();
                    if prefix != self.entries.len() || prefix != page.entries.len() {
                        let suffix = self.entries[prefix..].iter().rev().zip(page.entries[prefix..].iter().rev()).take_while(|(old, new)| Arc::ptr_eq(old, new)).count();
                        self.entries.splice(prefix..self.entries.len() - suffix, page.entries[prefix..page.entries.len() - suffix].iter().cloned());
                        changed = true;
                    }
                    if self.filter_changed || reordered && follow_head {
                        self.selected = self.entries.first().map(|entry| entry.id); self.scroll.set_offset(Point::default());
                        notify = true;
                    } else if reordered {
                        if !self.entries.iter().any(|entry| Some(entry.id) == self.selected) { self.selected = self.entries.get(selected_index.min(self.entries.len().saturating_sub(1))).map(|entry| entry.id); }
                        if let Some(index) = anchor.and_then(|id| self.entries.iter().position(|entry| entry.id == id)) {
                            let distance = px(index as f32 * f32::from(extent) + distance - anchor_index as f32 * f32::from(extent));
                            self.scroll.set_offset(if self.vertical() { point(offset.x, -distance) } else { point(-distance, offset.y) });
                        }
                    }
                }
                self.filter_changed = false;
                if changed {
                    self.reconcile_ocr(window, cx);
                    self.reconcile_details(window, cx);
                    self.update_sizes();
                }
                notify |= changed;
                tracing::debug!(changed, rows = self.entries.len(), "应用历史窗口差异");
                if let Some(pending) = self.pending_selection.take() { notify = true; if !self.apply_selection(pending.steps, cx) { self.pending_confirm = None; } }
            }
            Err(error) => {
                tracing::warn!(%error, "读取剪贴板历史失败"); self.error = Some(error.to_string().into());
                self.pending_selection = None; self.pending_confirm = None;
                notify = true;
            }
        }
        if self.phase == Phase::Preparing { if let Err(error) = self.begin_preview_preparation(window, cx) { self.fail(error, window, cx); return; } }
        if self.reload_pending && self.phase != Phase::Preparing { self.refresh(window, cx); }
        else if !self.loading { if let Some(pending) = self.pending_confirm.take() { self.restore_at(pending.mode, pending.index, window, cx); } }
        if notify { cx.notify(); }
    }
    pub(super) fn update_sizes(&mut self) {
        let size = self.list_item_size();
        if self.sizes.len() == self.entries.len() && self.sizes.first().is_none_or(|current| *current == size) { return; }
        self.sizes = Rc::new(vec![size; self.entries.len()]);
    }
    pub(super) fn list_item_size(&self) -> Size<Pixels> {
        let card = self.card_size();
        if self.vertical() { size((self.dimensions.width - px(26.0)).max(px(1.0)), card.height + px(10.0)) }
            else { size(card.width + px(10.0), card.height) }
    }
    pub(super) fn card_size(&self) -> Size<Pixels> {
        let width = (self.dimensions.width - px(26.0)).min(px(300.0)).max(px(1.0));
        size(width, if self.vertical() { width } else { (self.dimensions.height - px(66.0)).max(px(1.0)) })
    }
    pub(super) fn search_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search.read(cx).value().is_empty() { self.reveal_search_for_input(window, cx); }
        // Preedit text never becomes a query identity or clears the committed
        // result set. Commit resumes a cancelled debounce when needed.
        if self.search.read(cx).is_composing() { self.debounce = None; cx.notify(); return; }
        let query = self.search.read(cx).value().trim_matches(super_paste_core::preview::text_whitespace).to_owned();
        if query == self.query && (self.debounce.is_some() || self.loading || !self.filter_changed) {
            if self.reload_pending && self.debounce.is_none() { self.refresh(window, cx); }
            return;
        }
        if query != self.query { self.query = query; self.reset_query(window); }
        if self.phase == Phase::Hidden { cx.notify(); return; }
        self.debounce = Some(cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let _ = weak.update_in(cx, |this, window, cx| { this.debounce = None; this.request(false, window, cx); });
        })); cx.notify();
    }
    pub(super) fn clear_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search.read(cx).value().is_empty() || self.search.read(cx).is_composing() {
            self.search.update(cx, |state, cx| state.set_value("", window, cx));
        }
        if self.query.is_empty() { return; }
        self.query.clear(); self.reset_query(window); self.request(false, window, cx);
    }
    pub(super) fn reset_query(&mut self, _window: &mut Window) {
        self.scrolling.stop();
        self.hovered_card = None; self.pressed_card = None;
        self.debounce = None; self.reader.suspend(); self.query_generation = None; self.loading = false;
        self.tags.switching = false;
        self.pending_selection = None; self.pending_confirm = None; self.reload_pending = false;
        // Keep the displayed snapshot and its previews through debounce/read.
        // Pagination resumes only after the new filter's first page is accepted.
        self.filter_changed = true; self.cursor = None; self.initial_limit = 24;
        self.error = None;
    }
    fn apply_selection(&mut self, steps: impl IntoIterator<Item = SelectionStep>, cx: &App) -> bool {
        if self.entries.is_empty() { return false; }
        let last = self.entries.len().saturating_sub(1);
        let mut index = self.entries.iter().position(|entry| Some(entry.id) == self.selected).unwrap_or(0);
        for step in steps {
            let base = match step.anchor { Anchor::Current => index, Anchor::First => 0, Anchor::Last => last };
            index = base.saturating_add_signed(step.delta).min(last);
        }
        self.selected = self.entries.get(index).map(|entry| entry.id);
        self.reveal_selection(index, cx);
        true
    }
    fn navigate(&mut self, anchor: Anchor, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.restoring.is_some() || self.pending_confirm.is_some() { return; }
        let pending = SelectionStep { anchor, delta };
        if self.loading && !self.appending || self.debounce.is_some() {
            self.pending_selection.get_or_insert_with(PendingSelection::default).push(pending);
            if self.debounce.take().is_some() { self.request(false, window, cx); }
        } else { self.apply_selection(std::iter::once(pending), cx); }
        if !self.loading && self.entries.len().saturating_sub(self.entries.iter().position(|entry| Some(entry.id) == self.selected).unwrap_or(0)) <= 5 { self.request(true, window, cx); }
        cx.notify();
    }
    pub(super) fn restore_selected(&mut self, mode: RestoreMode, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_at(mode, None, window, cx);
    }
    fn restore_at(&mut self, mode: RestoreMode, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if self.restoring.is_some() || self.pending_confirm.is_some() || !matches!(self.phase, Phase::Opening | Phase::Opened) || self.search.read(cx).is_composing() { return; }
        if self.loading && !self.appending || self.debounce.is_some() {
            self.pending_confirm = Some(PendingRestore { mode, index });
            if self.debounce.take().is_some() { self.request(false, window, cx); }
            return;
        }
        let Some(id) = (match index { Some(index) => self.entries.get(index).map(|entry| entry.id), None => self.selected }) else { return; };
        self.restore_record(id, mode, window, cx);
    }
    pub(super) fn restore_record(&mut self, id: RecordId, mode: RestoreMode, window: &mut Window, cx: &mut Context<Self>) {
        self.record_action(id, RecordAction::Restore(mode), window, cx);
    }
    pub(super) fn record_action(&mut self, id: RecordId, action: RecordAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.restoring.is_some() || self.services.is_shutting_down() || !matches!(self.phase, Phase::Opening | Phase::Opened) || self.search.read(cx).is_composing() { return; }
        self.pending_selection = None; self.pending_confirm = None;
        let generation = self.generation; let cancel = CancellationToken::new(); self.restoring = Some((generation, cancel.clone()));
        let services = self.services.clone(); let desktop = cx.global::<crate::host::DesktopGlobal>().0.downgrade();
        let task = self.services.runtime.spawn({ let services = services.clone(); async move {
            let result = match action {
                RecordAction::Restore(mode) => services.clipboard.restore(id, mode, &cancel).await,
                RecordAction::Qr(action) => services.clipboard.copy_qr(id, action, &cancel).await,
            };
            if let Err(error) = &result { if !matches!(error, Error::Cancelled) { tracing::warn!(%error, "还原剪贴板失败"); } }
            result
        }});
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let failure: Option<SharedString> = match &result {
                Ok(()) | Err(Error::Cancelled) => None,
                Err(error) => Some(format!("{}失败：{error}", match action {
                    RecordAction::Restore(_) => "还原", RecordAction::Qr(QrAction::Generate) => "生成二维码", RecordAction::Qr(QrAction::Parse) => "解析二维码",
                }).into()),
            };
            let delivered = weak.update_in(cx, |this, window, cx| {
                if this.restoring.as_ref().is_some_and(|(current, _)| *current == generation) { this.restoring = None; }
                if this.generation != generation || !this.can_present_feedback(window) || services.is_shutting_down() { cx.notify(); return false; }
                if result.is_ok() {
                    this.selected = this.entries.first().map(|entry| entry.id); this.scroll.set_offset(Point::default());
                    this.close(window, cx); this.normalize_after_close = Some(this.generation);
                }
                if let Some(message) = &failure { crate::ui::dialogs::message(message.clone(), window, cx); }
                cx.notify(); failure.is_some()
            }).unwrap_or(false);
            if !delivered { if let Some(message) = failure { super::report_operation_failure(&desktop, message, cx); } }
        }).detach(); cx.notify();
    }
    pub(super) fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Opening | Phase::Opened) || window.has_active_dialog(cx) || self.menu.is_some() || self.search.read(cx).is_composing() { return; }
        let key = &event.keystroke; let modifier = key.modifiers.platform || key.modifiers.control;
        let input = self.search.read(cx).focus_handle(cx).contains_focused(window, cx);
        if key.modifiers.shift && input && matches!(key.key.as_str(), "left" | "right" | "up" | "down" | "home" | "end") { return; }
        if let Some(index) = modifier.then(|| quick_selection_index(event)).flatten() {
            if !event.is_held { self.restore_at(RestoreMode::Original, Some(index), window, cx); }
            window.prevent_default(); cx.stop_propagation(); cx.notify(); return;
        }
        let handled = match key.key.as_str() {
            "escape" => {
                if !self.search.read(cx).value().is_empty() { self.set_search_visible(true, window, cx); self.clear_query(window, cx); self.search.update(cx, |state, cx| state.focus(window, cx)); }
                else if self.search_visible { self.set_search_visible(false, window, cx); } else { self.close(window, cx); } true
            },
            "left" | "right" => { let delta = if key.key == "left" { -1 } else { 1 } * if modifier { 5 } else { 1 }; self.navigate(Anchor::Current, delta, window, cx); true },
            "up" | "down" => { self.navigate(if modifier { if key.key == "up" { Anchor::First } else { Anchor::Last } } else { Anchor::Current }, if modifier { 0 } else if key.key == "up" { -1 } else { 1 }, window, cx); true },
            "home" | "end" if modifier && !input => { self.navigate(if key.key == "home" { Anchor::First } else { Anchor::Last }, 0, window, cx); true },
            "enter" if input || self.focus.is_focused(window) => { if !event.is_held { self.restore_selected(RestoreMode::Original, window, cx); } true },
            _ => false,
        };
        if handled { window.prevent_default(); cx.stop_propagation(); cx.notify(); return; }
        if !input && (self.focus.is_focused(window) || key.key_char.as_deref() != Some(" ")) && (!key.modifiers.control && !key.modifiers.platform && !key.modifiers.alt || event.prefer_character_input) {
            if let Some(text) = key.key_char.as_ref().filter(|text| !text.is_empty() && !text.chars().any(char::is_control)) {
                self.set_search_visible(true, window, cx);
                self.search.update(cx, |state, cx| { state.focus(window, cx); state.replace_text_in_range(None, text, window, cx); });
                window.prevent_default(); cx.stop_propagation(); cx.notify();
            }
        }
    }
}
