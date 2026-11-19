use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use gpui_kit::*;
use super_paste_core::{Error, Result, events::AppEvent, statistics::{StatisticsKind, StatisticsService}};
use tokio_util::{sync::{CancellationToken, DropGuard}, task::AbortOnDropHandle};
use crate::services::Services;

pub(super) type FetchResult<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;
type Fetch<Q, T> = fn(Arc<StatisticsService>, Q, CancellationToken) -> FetchResult<T>;

/// One read and one latest requested query. Hidden pages retain a dirty flag,
/// while entity-owned tasks release event subscriptions on tab/page changes.
pub struct Live<Q, T> {
    services: Arc<Services>, owner: WeakEntity<super::StatisticsPane>, origin: EntityId, kind: StatisticsKind, query: Option<Q>, version: u64,
    pub value: Option<Arc<T>>, pub error: Option<SharedString>, pub operation_error: Option<SharedString>, pub loading: bool, pub clearing: bool, pub confirming: bool,
    visible: bool, dirty: bool, keep_value_on_error: bool, fetch: Fetch<Q, T>,
    read_cancel: Option<DropGuard>, read_task: Option<Task<()>>, refresh_timer: Option<Task<()>>, _events: Task<()>, _visibility: Subscription,
}
impl<Q: Clone + Send + 'static, T: Send + Sync + 'static> Live<Q, T> {
    pub fn new(services: Arc<Services>, owner: WeakEntity<super::StatisticsPane>, origin: EntityId, kind: StatisticsKind, query: Result<Q>, keep_value_on_error: bool, fetch: Fetch<Q, T>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (query, error) = match query { Ok(query) => (Some(query), None), Err(error) => (None, Some(error.to_string().into())) };
        let mut events = services.events.subscribe();
        let event_task = cx.spawn_in(window, async move |weak, cx| {
            loop {
                let changed = match events.recv().await {
                    Ok(AppEvent::CopyStatisticsChanged) => kind == StatisticsKind::Copy,
                    Ok(AppEvent::AiUsageChanged) => kind == StatisticsKind::Ai,
                    Ok(_) => false,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if changed && weak.update_in(cx, |this, window, cx| this.schedule(window, cx)).is_err() { break; }
            }
        });
        let visibility = cx.observe_window_visibility(window, |this, _, window, cx| {
            let visible = window.is_visible();
            if this.visible == visible { return; }
            this.visible = visible;
            if visible {
                // Relative date ranges are evaluated by fetch. Reopening after
                // midnight needs a fresh snapshot even without storage events.
                this.refresh(window, cx);
            } else {
                this.refresh_timer = None;
                this.dirty = true;
                this.version = this.version.wrapping_add(1);
                this.read_cancel = None;
                // Retain the sole read until its physical worker drains.
                // Reopening then starts only the latest requested query.
            }
        });
        let mut state = Self {
            services, owner, origin, kind, query, version: 0, value: None, error, operation_error: None, loading: false, clearing: false, confirming: false,
            visible: window.is_visible(), dirty: true, keep_value_on_error, fetch,
            read_cancel: None, read_task: None, refresh_timer: None, _events: event_task, _visibility: visibility,
        };
        state.refresh(window, cx);
        state
    }
    pub fn set_query(&mut self, query: Result<Q>, window: &mut Window, cx: &mut Context<Self>) {
        self.read_cancel = None;
        self.version = self.version.wrapping_add(1);
        self.operation_error = None;
        if self.kind == StatisticsKind::Copy { self.confirming = false; }
        match query {
            Ok(query) => { self.query = Some(query); self.error = None; self.refresh(window, cx); }
            Err(error) => {
                self.query = None; self.dirty = false; self.refresh_timer = None;
                self.error = Some(error.to_string().into());
                if !self.keep_value_on_error { self.value = None; }
            }
        }
        cx.notify();
    }
    fn schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dirty = true;
        if !self.visible || self.clearing || self.query.is_none()
            || self.read_task.is_some() || self.refresh_timer.is_some()
        { return; }
        // Coalesce onto the first scheduled refresh. Replacing this timer on
        // each event would starve updates during sustained clipboard activity.
        let delay = Duration::from_millis(if self.kind == StatisticsKind::Copy { 250 } else { 150 });
        self.refresh_timer = Some(cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor().timer(delay).await;
            let _ = weak.update_in(cx, |this, window, cx| { this.refresh_timer = None; this.refresh(window, cx); });
        }));
    }
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_timer = None;
        self.dirty = true;
        if self.read_task.is_some() || !self.visible || self.clearing { return; }
        let Some(query) = self.query.clone() else { return; };
        self.dirty = false; self.loading = true; self.error = None;
        let version = self.version;
        let cancel = CancellationToken::new();
        self.read_cancel = Some(cancel.clone().drop_guard());
        let task = AbortOnDropHandle::new(self.services.runtime.spawn((self.fetch)(self.services.statistics.clone(), query, cancel)));
        self.read_task = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update_in(cx, |this, window, cx| {
                this.read_cancel = None; this.read_task = None; this.loading = false;
                if version == this.version {
                    match result {
                        Ok(value) => { this.value = Some(Arc::new(value)); this.error = None; }
                        Err(error) => { this.error = Some(error.to_string().into()); if !this.keep_value_on_error { this.value = None; } }
                    }
                }
                if this.dirty && this.query.is_some() && this.visible && !this.clearing { this.refresh(window, cx); }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn begin_clear(&mut self, cx: &mut Context<Self>) {
        self.read_cancel = None;
        self.clearing = true; self.version = self.version.wrapping_add(1); self.refresh_timer = None; self.error = None; self.operation_error = None;
        cx.notify();
    }
    fn finish_clear(&mut self, result: Result<()>, window: &mut Window, cx: &mut Context<Self>) {
        self.clearing = false;
        match result {
            Ok(()) => { self.confirming = false; self.refresh(window, cx); }
            Err(error) => { self.operation_error = Some(error.to_string().into()); if self.dirty { self.refresh(window, cx); } }
        }
        cx.notify();
    }
    pub fn request_clear(&mut self, cx: &mut Context<Self>) {
        if self.clearing || self.services.is_shutting_down() { return; }
        self.confirming = true; cx.notify();
    }
    pub fn cancel_clear(&mut self, cx: &mut Context<Self>) {
        if !self.clearing { self.confirming = false; cx.notify(); }
    }
    pub fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clearing || !self.confirming || self.services.is_shutting_down() { return; }
        self.begin_clear(cx);
        let services = self.services.clone();
        let service = self.services.statistics.clone();
        let kind = self.kind;
        let task = self.services.runtime.spawn(async move { service.clear(kind).await });
        let owner = window.window_handle();
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let failure = result.as_ref().err().cloned();
            let handled = weak.update_in(cx, |this, window, cx| {
                // Statistics has its own tabs below the animated settings page.
                // Check both owners; a retained child is not delivery evidence.
                let active = this.owner.upgrade().is_some_and(|owner| {
                    let pane = owner.read(cx);
                    pane.context.is_active(owner.entity_id(), cx) && pane.body.entity_id() == this.origin
                });
                this.finish_clear(result, window, cx);
                active
            }).unwrap_or(false);
            if !handled && !services.is_shutting_down() {
                if let Some(error) = failure {
                    let _ = owner.update(cx, |_, window, cx| crate::ui::dialogs::message(format!("清空统计失败：{error}"), window, cx));
                }
            }
        }).detach();
    }
}

