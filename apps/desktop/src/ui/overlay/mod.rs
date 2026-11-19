mod reader;
mod history;
mod render;
mod previews;
mod preview_content;
mod rich;
mod tags;
mod menus;
mod ocr;
mod details;
mod scrolling;
mod source_icons;
mod cards;
mod search_motion;

use std::{collections::{HashMap, HashSet}, rc::Rc, sync::Arc, time::Duration};
use gpui_kit::{prelude::*, *};
use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::component::{WindowExt, input::{InputEvent, InputState}};
use super_paste_core::{Error, Result, events::{AppEvent, RecordChange}};
use super_paste_format::{HistoryCursor, HistoryEntry, OverlayPosition, RecordId, TagId};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};
use crate::{services::Services, ui::configuration::ConfigurationGlobal};
pub(crate) use reader::HistoryReader;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Hidden, Preparing, Opening, Opened, Closing }
#[derive(Clone, Default)]
pub struct SavedState { query: String, search_visible: bool, selected: Option<RecordId>, offset: Point<Pixels>, loaded: usize, tag: Option<TagId> }

pub struct OverlayView {
    services: Arc<Services>, pub(crate) reader: Arc<HistoryReader>, position: OverlayPosition, work: crate::ui::geometry::WorkArea,
    phase: Phase, generation: u64, activation_token: Option<String>, layer_shell: bool, retired: bool,
    source_freeze: Option<super_paste_platform::access::SourceFreeze>,
    focus: FocusHandle, search: Entity<InputState>, search_visible: bool, search_motion: search_motion::SearchMotion, search_hovered: bool, composing: bool, query: String,
    entries: Vec<Arc<HistoryEntry>>, selected: Option<RecordId>, cursor: Option<HistoryCursor>,
    query_generation: Option<u64>, loading: bool, appending: bool, reload_pending: bool, error: Option<SharedString>,
    pending_selection: Option<history::PendingSelection>, pending_confirm: Option<history::PendingRestore>, filter_changed: bool,
    initial_limit: usize, scroll: VirtualListScrollHandle, sizes: Rc<Vec<Size<Pixels>>>, dimensions: Size<Pixels>,
    shortcuts_visible: bool, restoring: Option<(u64, CancellationToken)>,
    hovered_card: Option<RecordId>, pressed_card: Option<RecordId>,
    normalize_after_close: Option<u64>,
    scrolling: scrolling::Scrolling,
    debounce: Option<Task<()>>, frame: Option<Task<()>>, preparation_dirty: bool, feed: Option<Task<()>>, events: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
    previews: HashMap<RecordId, previews::CardPreview>, preview_ticket: u64, preview_layout: bool,
    source_icons: HashMap<super_paste_format::SourceId, source_icons::SourceIcon>,
    preview_deadline: Option<Task<()>>, rasters: crate::ui::rasters::WindowRasters,
    tags: tags::Tags,
    ocr: ocr::Ocr,
    details: Option<details::Details>,
    menu: Option<menus::Menu>, clearing: bool, mutating: HashSet<RecordId>,
}
impl OverlayView {
    pub fn new(services: Arc<Services>, reader: Arc<HistoryReader>, position: OverlayPosition, work: crate::ui::geometry::WorkArea, layer_shell: bool,
        saved: SavedState, token: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        reader.suspend();
        let configuration = cx.global::<ConfigurationGlobal>().0.clone();
        crate::ui::theme::apply(configuration.read(cx).projected.settings.color_mode, window.appearance(), cx);
        let query = saved.query.trim_matches(super_paste_core::preview::text_whitespace).to_owned();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索剪贴板内容...").default_value(saved.query));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) { this.search_changed(window, cx); }
            }),
            cx.observe_in(&search, window, |this, input, window, cx| {
                let composing = input.read(cx).is_composing();
                if composing != this.composing {
                    this.composing = composing;
                    if composing {
                        this.debounce = None; this.pending_selection = None; this.pending_confirm = None;
                        this.reveal_search_for_input(window, cx);
                    } else { this.search_changed(window, cx); }
                }
            }),
            cx.observe_in(&configuration, window, |this, model, window, cx| {
                crate::ui::theme::apply(model.read(cx).projected.settings.color_mode, window.appearance(), cx);
                this.refresh_menu_palette(window, cx); cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                let mode = cx.global::<ConfigurationGlobal>().0.read(cx).projected.settings.color_mode;
                crate::ui::theme::apply(mode, window.appearance(), cx);
                this.refresh_menu_palette(window, cx); cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.shortcuts_visible = false;
                if window.is_window_active() { this.complete_details_return(window, cx); }
                if !window.is_window_active() { this.scrolling.stop(); this.pressed_card = None; }
                if !window.is_window_active() {
                    let owner = cx.entity().downgrade(); let generation = this.generation;
                    window.on_next_frame(move |window, cx| { let _ = owner.update(cx, |this, cx| {
                        if this.generation == generation && !window.is_window_active() && matches!(this.phase, Phase::Opened | Phase::Opening)
                            && !window.has_active_dialog(cx) && !this.details.as_ref().is_some_and(|details| details.presenting) { this.close(window, cx); }
                    }); });
                }
                cx.notify();
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                let dimensions = window.viewport_size();
                if this.dimensions != dimensions { this.scrolling.stop(); this.dimensions = dimensions; this.update_sizes(); cx.notify(); }
            }),
        ];
        let scroll = VirtualListScrollHandle::new(); scroll.set_offset(saved.offset);
        let mut view = Self { services, reader, position, work, phase: Phase::Hidden, generation: 0, activation_token: token, layer_shell, retired: false, source_freeze: None,
            focus: cx.focus_handle(), search, search_visible: saved.search_visible, search_motion: Default::default(), search_hovered: false, composing: false, query,
            entries: Vec::new(), selected: saved.selected, cursor: None, query_generation: None, loading: false, appending: false, reload_pending: false, error: None,
            pending_selection: None, pending_confirm: None, filter_changed: false,
            initial_limit: saved.loaded.max(24), scroll, sizes: Rc::new(Vec::new()), dimensions: window.viewport_size(), shortcuts_visible: false,
            restoring: None, hovered_card: None, pressed_card: None, normalize_after_close: None, scrolling: Default::default(), debounce: None, frame: None, preparation_dirty: false, feed: None, events: None, subscriptions,
            previews: HashMap::new(), source_icons: HashMap::new(), preview_ticket: 0, preview_layout: false, preview_deadline: None, rasters: Default::default(), tags: tags::Tags::new(saved.tag), ocr: Default::default(), details: None, menu: None, clearing: false, mutating: HashSet::new() };
        view.open(None, window, cx);
        view
    }
    pub fn saved(&self, cx: &App) -> SavedState { SavedState { query: self.search.read(cx).value().to_string(), search_visible: self.search_visible, selected: self.selected, offset: self.scroll.offset(), loaded: self.entries.len().max(self.initial_limit), tag: self.tags.selected } }
    pub fn is_open(&self) -> bool { matches!(self.phase, Phase::Preparing | Phase::Opening | Phase::Opened) }
    fn can_present_feedback(&self, window: &Window) -> bool { matches!(self.phase, Phase::Opening | Phase::Opened) && window.is_visible() }
    pub fn open(&mut self, token: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.retired { return; }
        if token.is_some() { self.activation_token = token; }
        if self.is_open() { if self.phase != Phase::Preparing { if let Err(error) = self.activate(window, cx) { self.fail(error, window, cx); } } return; }
        self.generation = self.generation.wrapping_add(1);
        if self.source_freeze.is_none() {
            match self.services.access.freeze_source() {
                Ok(freeze) => self.source_freeze = Some(freeze),
                Err(error) => { self.report(error, cx); return; },
            }
        }
        if self.phase == Phase::Closing {
            // Cancel the old terminal-frame waiter before reversing the same
            // transition; it must never hide a subsequently reopened panel.
            self.frame = None;
            self.phase = Phase::Opening;
            if let Err(error) = self.activate(window, cx) { self.fail(error, window, cx); }
            self.refresh_ocr(window, cx);
            if self.filter_changed && !self.loading || self.reload_pending { self.refresh(window, cx); }
            cx.notify(); return;
        }
        self.phase = Phase::Preparing; self.error = None; self.restore_cancel();
        let mut replies = self.reader.subscribe();
        self.feed = Some(cx.spawn_in(window, async move |weak, cx| {
            while replies.changed().await.is_ok() {
                let reply = replies.borrow_and_update().clone();
                if let Some(reply) = reply { if weak.update_in(cx, |this, window, cx| this.accept(reply, window, cx)).is_err() { break; } }
            }
        }));
        let mut events = self.services.events.subscribe();
        self.events = Some(cx.spawn_in(window, async move |weak, cx| {
            loop {
                let event = events.recv().await;
                let refresh = match event {
                    Ok(AppEvent::OcrTask(status)) => {
                        if weak.update_in(cx, |this, window, cx| this.apply_ocr(status, window, cx)).is_err() { break; }
                        false
                    },
                    Ok(AppEvent::HistoryChanged { change: RecordChange::Ocr { .. }, .. }) => {
                        if weak.update_in(cx, |this, window, cx| this.refresh(window, cx)).is_err() { break; }
                        false
                    },
                    Ok(AppEvent::HistoryChanged { ref ids, change: RecordChange::Removed }) => {
                        let _ = weak.update_in(cx, |this, window, cx| { if this.removed(ids, window, cx) { cx.notify(); } }); true
                    },
                    Ok(AppEvent::HistoryChanged { .. } | AppEvent::TagsChanged { .. }) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if weak.update_in(cx, |this, window, cx| {
                            // Missed notifications may include deletions. A
                            // pre-gap reply is not a recovery snapshot.
                            this.invalidate_history_read();
                            this.validate_details(window, cx);
                            this.refresh_ocr(window, cx);
                        }).is_err() { break; }
                        true
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => false,
                };
                if refresh && weak.update_in(cx, |this, window, cx| { this.refresh_tags(window, cx); this.refresh(window, cx); }).is_err() { break; }
            }
        }));
        self.refresh_tags(window, cx); self.request(false, window, cx); cx.notify();
    }
    fn prepared(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        if self.phase != Phase::Preparing || self.frame.is_some() { return Ok(()); }
        window.set_pointer_passthrough(true).map_err(platform_error)?;
        window.set_visible(true, cx).map_err(platform_error)?;
        let barrier = window.request_frame_submission().map_err(platform_error)?;
        self.preparation_dirty = false;
        let generation = self.generation; let runtime = self.services.runtime.clone();
        self.frame = Some(cx.spawn_in(window, async move |weak, cx| {
            let task = AbortOnDropHandle::new(runtime.spawn(async move {
                tokio::time::timeout(Duration::from_secs(3), barrier).await.map_err(|_| Error::Platform("覆盖层首帧提交超时".into()))?
                    .map_err(|_| Error::Platform("覆盖层首帧提交已取消".into()))?; Ok(())
            }));
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.phase != Phase::Preparing { return; }
                this.frame = None;
                // Prepaint discovers which inline images actually intersect
                // the clip. Check again after the real submission, and request
                // another barrier when their event-driven loader settles.
                if result.is_ok() && !this.entries.is_empty() && !this.previews_ready() { cx.notify(); return; }
                if result.is_ok() && this.preparation_dirty {
                    if let Err(error) = this.prepared(window, cx) { this.fail(error, window, cx); }
                    cx.notify(); return;
                }
                match result.and_then(|_| this.activate(window, cx)) {
                    Ok(()) => { this.preview_deadline = None; this.phase = Phase::Opening; if this.reload_pending { this.refresh(window, cx); } cx.notify(); },
                    Err(error) => this.fail(error, window, cx),
                }
            });
        }));
        Ok(())
    }
    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        window.set_pointer_passthrough(false).map_err(platform_error)?;
        #[cfg(target_os = "linux")]
        if self.layer_shell {
            window.set_layer_shell_keyboard_interactivity(layer_shell::KeyboardInteractivity::Exclusive).map_err(platform_error)?;
        }
        window.activate_window_with_token(self.activation_token.take().as_deref()).map_err(platform_error)?;
        self.search.update(cx, |state, cx| state.focus(window, cx));
        Ok(())
    }
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Hidden | Phase::Closing) { return; }
        self.scrolling.stop();
        self.menu = None;
        self.pending_selection = None; self.pending_confirm = None;
        self.ocr.suspend();
        self.close_details(window, cx);
        self.generation = self.generation.wrapping_add(1); self.frame = None; self.preview_deadline = None; self.debounce = None; self.shortcuts_visible = false;
        if self.phase == Phase::Preparing || !window.is_visible() { self.finish_hide(window, cx); } else { self.phase = Phase::Closing; cx.notify(); }
    }
    pub fn retire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.retired = true; self.generation = self.generation.wrapping_add(1); self.finish_hide(window, cx);
    }
    fn finish_closing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        if self.phase != Phase::Closing || self.frame.is_some() { return Ok(()); }
        // Render the transparent endpoint before unmapping the native window.
        // A deferred render callback alone can run before that scene is submitted.
        let barrier = window.request_frame_submission().map_err(platform_error)?;
        let generation = self.generation; let runtime = self.services.runtime.clone();
        self.frame = Some(cx.spawn_in(window, async move |weak, cx| {
            let task = AbortOnDropHandle::new(runtime.spawn(async move {
                tokio::time::timeout(Duration::from_secs(3), barrier).await
                    .map_err(|_| Error::Platform("覆盖层隐藏末帧提交超时".into()))?
                    .map_err(|_| Error::Platform("覆盖层隐藏末帧提交已取消".into()))?;
                Ok(())
            }));
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.phase != Phase::Closing { return; }
                match result {
                    Ok(()) => {
                        // Submission can precede compositor consumption. Keep
                        // the terminal scene mapped through the next frame turn.
                        let owner = cx.entity().downgrade();
                        window.on_next_frame(move |window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if this.generation != generation || this.phase != Phase::Closing { return; }
                                this.frame = None;
                                this.finish_hide(window, cx);
                            });
                        });
                    },
                    Err(error) => { this.frame = None; this.fail(error, window, cx); },
                }
            });
        }));
        Ok(())
    }
    fn finish_hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.scrolling.stop();
        self.hovered_card = None; self.pressed_card = None;
        self.search_motion.cancel();
        self.search_hovered = false;
        self.phase = Phase::Hidden; self.query_generation = None; self.loading = false; self.reload_pending = false;
        self.activation_token = None;
        self.pending_selection = None; self.pending_confirm = None;
        self.frame = None; self.feed = None; self.events = None; self.debounce = None; self.restore_cancel(); self.reader.suspend();
        self.tags.suspend();
        self.ocr.suspend();
        self.close_details(window, cx);
        self.menu = None;
        if self.normalize_after_close.take() == Some(self.generation) {
            self.initial_limit = 24; self.tags.selected = None; self.selected = None;
        } else { self.initial_limit = self.entries.len().max(self.initial_limit); }
        self.entries.clear(); self.sizes = Rc::new(Vec::new());
        self.clear_previews(window);
        let mut errors = Vec::new();
        #[cfg(target_os = "linux")]
        if self.layer_shell { if let Err(error) = window.set_layer_shell_keyboard_interactivity(layer_shell::KeyboardInteractivity::None) { errors.push(error.to_string()); } }
        if let Err(error) = window.set_pointer_passthrough(true) { errors.push(error.to_string()); }
        if let Err(error) = window.set_visible(false, cx) { errors.push(error.to_string()); }
        if let Some(freeze) = self.source_freeze.take() {
            if let Err(error) = freeze.release() { errors.push(error.to_string()); }
        }
        if !errors.is_empty() { let error = Error::Platform(errors.join("；").into()); if self.retired { tracing::error!(%error, "回收覆盖层窗口失败"); } else { self.report(error, cx); } }
        cx.notify();
    }
    fn fail(&mut self, error: Error, window: &mut Window, cx: &mut Context<Self>) { self.finish_hide(window, cx); self.report(error, cx); }
    fn report(&self, error: Error, cx: &mut Context<Self>) {
        tracing::error!(%error, "覆盖层操作失败");
        cx.defer(move |cx| {
            let desktop = cx.global::<crate::host::DesktopGlobal>().0.clone();
            desktop.update(cx, |desktop, cx| desktop.report_overlay_error(error, cx));
        });
    }
    fn restore_cancel(&mut self) { if let Some((_, cancel)) = self.restoring.take() { cancel.cancel(); } }
    fn vertical(&self) -> bool { matches!(self.position, OverlayPosition::Left | OverlayPosition::Right) }
}
impl Focusable for OverlayView { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }
impl Drop for OverlayView {
    fn drop(&mut self) { if !self.retired { self.reader.suspend(); } self.restore_cancel(); }
}
fn report_operation_failure(desktop: &WeakEntity<crate::host::Desktop>, message: SharedString, cx: &mut impl AppContext) {
    let _ = desktop.update(cx, |desktop, cx| desktop.report_overlay_error(Error::Invalid(Arc::from(message.as_str())), cx));
}
fn present_operation_failure(owner: &WeakEntity<OverlayView>, desktop: &WeakEntity<crate::host::Desktop>, message: SharedString, cx: &mut AsyncWindowContext) {
    let delivered = owner.update_in(cx, |this, window, cx| {
        if !this.can_present_feedback(window) || this.services.is_shutting_down() { return false; }
        crate::ui::dialogs::message(message.clone(), window, cx); true
    }).unwrap_or(false);
    if !delivered { report_operation_failure(desktop, message, cx); }
}
fn platform_error(error: anyhow::Error) -> Error { Error::Platform(error.to_string().into()) }
