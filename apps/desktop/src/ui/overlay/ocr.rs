use std::{collections::{HashMap, HashSet}, time::{Duration, Instant}};
use gpui_kit::{prelude::*, *};
use super_paste_core::{Error, events::{OcrTaskState, OcrTaskStatus}, ocr::{OcrTrigger, TASK_STATUS_RETENTION as RETENTION, MAX_RECENT_TASK_STATUSES as MAX_STATUSES}};
use super_paste_format::RecordId;
use tokio_util::task::AbortOnDropHandle;
use super::OverlayView;

struct Status { generation: u64, state: OcrTaskState, sequence: u64, expires: Option<Instant> }
#[derive(Default)]
pub(super) struct Ocr {
    statuses: HashMap<RecordId, Status>, known: HashSet<RecordId>, sequence: u64,
    snapshot: Option<Task<()>>, dirty: bool, expiry: Option<Task<()>>,
    submitting: HashSet<RecordId>,
}
impl Ocr {
    pub fn suspend(&mut self) { self.snapshot = None; self.expiry = None; self.statuses.clear(); self.known.clear(); self.dirty = false; }
    pub fn busy(&self, id: RecordId) -> bool {
        self.submitting.contains(&id) || self.statuses.get(&id).is_some_and(|status| matches!(status.state, OcrTaskState::Queued | OcrTaskState::Running))
    }
    pub fn feedback(&self, id: RecordId) -> Option<(SharedString, bool)> {
        let status = self.statuses.get(&id)?;
        if status.expires.is_some_and(|deadline| deadline <= Instant::now()) { return None; }
        match &status.state {
            OcrTaskState::Queued => Some(("OCR 等待中".into(), false)),
            OcrTaskState::Running => Some(("OCR 识别中".into(), false)),
            OcrTaskState::Failed(error) => Some((error.to_string().into(), true)),
            _ => None,
        }
    }
}
fn rank(state: &OcrTaskState) -> u8 { match state { OcrTaskState::Queued => 0, OcrTaskState::Running => 1, _ => 2 } }
impl OverlayView {
    pub(super) fn reconcile_ocr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: HashSet<_> = self.entries.iter().map(|entry| entry.id).collect();
        let added = ids.iter().any(|id| !self.ocr.known.contains(id));
        self.ocr.statuses.retain(|id, _| ids.contains(id)); self.ocr.known = ids;
        if !self.ocr.statuses.values().any(|status| status.expires.is_some()) { self.ocr.expiry = None; }
        if added { self.refresh_ocr(window, cx); }
    }
    pub(super) fn apply_ocr(&mut self, task: OcrTaskStatus, window: &mut Window, cx: &mut Context<Self>) {
        let OcrTaskStatus { record: id, generation, state, expires_at: expires } = task;
        if !self.is_open() || !self.entries.iter().any(|entry| entry.id == id) { return; }
        if expires.is_some_and(|deadline| deadline <= Instant::now()) {
            if self.ocr.statuses.get(&id).is_some_and(|old| old.generation <= generation) { self.ocr.statuses.remove(&id); self.sync_ocr_menu(cx); cx.notify(); }
            return;
        }
        if self.ocr.statuses.get(&id).is_some_and(|old| old.generation > generation || (old.generation == generation && rank(&old.state) > rank(&state))) { return; }
        self.ocr.sequence = self.ocr.sequence.wrapping_add(1);
        self.ocr.statuses.insert(id, Status { generation, state, expires, sequence: self.ocr.sequence });
        if self.ocr.statuses.len() > MAX_STATUSES {
            if let Some(oldest) = self.ocr.statuses.iter().min_by_key(|(_, status)| status.sequence).map(|(id, _)| *id) { self.ocr.statuses.remove(&oldest); }
        }
        self.sync_ocr_menu(cx); self.arm_ocr_expiry(window, cx); cx.notify();
    }
    pub(super) fn refresh_ocr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_open() || self.services.is_shutting_down() { return; }
        if self.ocr.snapshot.is_some() { self.ocr.dirty = true; return; }
        self.ocr.dirty = false;
        let sequence = self.ocr.sequence; let generation = self.generation;
        let records: Vec<_> = self.entries.iter().map(|entry| entry.id).collect();
        let service = self.services.ocr.clone();
        let task = AbortOnDropHandle::new(self.services.runtime.spawn(async move { service.task_statuses(&records).await }));
        self.ocr.snapshot = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.generation != generation || !this.is_open() { return; }
                this.ocr.snapshot = None;
                match result {
                    Ok(tasks) => {
                        let present: HashSet<_> = tasks.iter().map(|task| task.record).collect();
                        this.ocr.statuses.retain(|id, status| rank(&status.state) == 2 || status.sequence > sequence || present.contains(id));
                        for task in tasks {
                            if this.ocr.statuses.get(&task.record).is_some_and(|status| status.sequence > sequence) { continue; }
                            this.apply_ocr(task, window, cx);
                        }
                    },
                    Err(error) => { tracing::warn!(%error, "读取 OCR 任务状态失败"); this.error = Some(format!("读取 OCR 任务状态失败：{error}").into()); },
                }
                if this.ocr.dirty { this.refresh_ocr(window, cx); }
                this.sync_ocr_menu(cx);
                cx.notify();
            });
        }));
    }
    fn arm_ocr_expiry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ocr.expiry.is_some() || !self.ocr.statuses.values().any(|status| status.expires.is_some()) { return; }
        self.ocr.expiry = Some(cx.spawn_in(window, async move |weak, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(60)).await;
                let keep = weak.update_in(cx, |this, _, cx| {
                    let now = Instant::now(); this.ocr.statuses.retain(|_, status| status.expires.is_none_or(|deadline| deadline > now));
                    let keep = this.is_open() && this.ocr.statuses.values().any(|status| status.expires.is_some());
                    if !keep { this.ocr.expiry = None; } cx.notify(); keep
                }).unwrap_or(false);
                if !keep { break; }
            }
        }));
    }
    pub(super) fn recognize_record(&mut self, id: RecordId, window: &mut Window, cx: &mut Context<Self>) {
        if self.services.is_shutting_down() || self.ocr.busy(id) || !self.entries.iter().any(|entry| entry.id == id) { return; }
        self.ocr.submitting.insert(id);
        self.sync_ocr_menu(cx);
        let services = self.services.clone(); let service = services.ocr.clone(); let generation = self.generation;
        let desktop = cx.global::<crate::host::DesktopGlobal>().0.downgrade();
        // Admission and the accepted recognition belong to the service. Closing
        // the menu/window detaches presentation without aborting recognition.
        let task = self.services.runtime.spawn(async move {
            let result = service.enqueue_record(id, OcrTrigger::Manual).await;
            if let Err(error) = &result { tracing::warn!(%error, "提交 OCR 识别失败"); }
            result
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let failure = match result {
                Ok(Some(_)) => None, Ok(None) => Some(Error::Invalid("记录不包含可识别的图片".into())), Err(error) => Some(error),
            };
            let delivered = weak.update_in(cx, |this, window, cx| {
                this.ocr.submitting.remove(&id);
                this.sync_ocr_menu(cx);
                if !this.can_present_feedback(window) || this.generation != generation || services.is_shutting_down() { cx.notify(); return false; }
                if let Some(error) = &failure {
                    if !this.entries.iter().any(|entry| entry.id == id) { cx.notify(); return false; }
                    this.ocr_admission_failed(id, error.clone(), window, cx);
                } else {
                    this.refresh_ocr(window, cx);
                }
                cx.notify(); failure.is_some()
            }).unwrap_or(false);
            if !delivered {
                if let Some(error) = failure.filter(|error| !matches!(error, Error::Cancelled)) {
                    super::report_operation_failure(&desktop, format!("提交 OCR 识别失败：{error}").into(), cx);
                }
            }
        }).detach(); cx.notify();
    }
    fn ocr_admission_failed(&mut self, id: RecordId, error: Error, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.ocr.statuses.get(&id).map_or(0, |status| status.generation);
        self.apply_ocr(OcrTaskStatus { record: id, generation, state: OcrTaskState::Failed(error), expires_at: Some(Instant::now() + RETENTION) }, window, cx);
    }
}
