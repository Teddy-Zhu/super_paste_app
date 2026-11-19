use std::{collections::{BTreeSet, HashMap}, time::{Duration, Instant}};
use super_paste_format::RecordId;
use crate::events::{OcrTaskState, OcrTaskStatus};

pub const TASK_STATUS_RETENTION: Duration = Duration::from_secs(120);
pub const MAX_RECENT_TASK_STATUSES: usize = 500;

/// Scheduler-owned metadata. Pending entries live with their accepted tasks;
/// terminal entries have a shared deadline and never retain recognition data.
#[derive(Default)]
pub(super) struct TaskStatuses {
    records: HashMap<RecordId, OcrTaskStatus>,
    terminal: BTreeSet<(Instant, RecordId)>,
}
impl TaskStatuses {
    pub fn update(&mut self, record: RecordId, generation: u64, state: OcrTaskState, now: Instant) -> Option<OcrTaskStatus> {
        self.prune(now);
        if self.records.get(&record).is_some_and(|old| old.generation > generation || (old.generation == generation && rank(&old.state) >= rank(&state))) { return None; }
        self.remove(record);
        let expires_at = (rank(&state) == 2).then_some(now + TASK_STATUS_RETENTION);
        let status = OcrTaskStatus { record, generation, state, expires_at };
        if let Some(deadline) = expires_at { self.terminal.insert((deadline, record)); }
        self.records.insert(record, status.clone());
        self.prune(now);
        Some(status)
    }
    pub fn snapshot(&mut self, records: &[RecordId], now: Instant) -> Vec<OcrTaskStatus> {
        self.prune(now);
        records.iter().filter_map(|id| self.records.get(id).cloned()).collect()
    }
    pub fn remove(&mut self, record: RecordId) {
        if let Some(status) = self.records.remove(&record) {
            if let Some(deadline) = status.expires_at { self.terminal.remove(&(deadline, record)); }
        }
    }
    pub fn remove_generation(&mut self, record: RecordId, generation: u64) {
        if self.records.get(&record).is_some_and(|status| status.generation == generation) { self.remove(record); }
    }
    pub fn clear(&mut self) { self.records.clear(); self.terminal.clear(); }
    fn prune(&mut self, now: Instant) {
        while let Some(&(deadline, record)) = self.terminal.first() {
            if deadline > now && self.terminal.len() <= MAX_RECENT_TASK_STATUSES { break; }
            self.terminal.pop_first(); self.records.remove(&record);
        }
    }
}
fn rank(state: &OcrTaskState) -> u8 { match state { OcrTaskState::Queued => 0, OcrTaskState::Running => 1, _ => 2 } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn ocr_feedback_recovers_without_restarting_failure_retention() {
        let mut statuses = TaskStatuses::default();
        let record = RecordId::new(); let unrelated = RecordId::new(); let start = Instant::now();
        statuses.update(record, 7, OcrTaskState::Queued, start).expect("accepted task");
        statuses.update(record, 7, OcrTaskState::Running, start).expect("started task");
        // A hidden consumer has no events, and a writeback may outlive the
        // recognition phase. Pending feedback must remain until a terminal state.
        let finished = start + TASK_STATUS_RETENTION * 2;
        assert!(matches!(statuses.snapshot(&[record], finished)[0].state, OcrTaskState::Running));
        let message: std::sync::Arc<str> = "recognition failed".into();
        let event = statuses.update(record, 7, OcrTaskState::Failed(Error::Invalid(message.clone())), finished).expect("failed task");
        let deadline = event.expires_at.expect("failure deadline");
        let reopened = statuses.snapshot(&[unrelated, record], finished + Duration::from_secs(60));
        assert_eq!(reopened.len(), 1); assert_eq!(reopened[0].record, record); assert_eq!(reopened[0].expires_at, Some(deadline));
        let OcrTaskState::Failed(Error::Invalid(recovered)) = &reopened[0].state else { panic!("missing recovered error") };
        assert!(std::sync::Arc::ptr_eq(&message, recovered));
        assert!(statuses.update(record, 6, OcrTaskState::Running, finished).is_none());
        assert!(statuses.update(record, 7, OcrTaskState::Running, finished).is_none());
        assert!(statuses.update(record, 7, OcrTaskState::Failed(Error::Invalid("duplicate".into())), finished + Duration::from_secs(90)).is_none());
        assert_eq!(statuses.snapshot(&[record], deadline - Duration::from_nanos(1))[0].expires_at, Some(deadline));
        assert!(statuses.snapshot(&[record], deadline).is_empty());

        statuses.update(record, 8, OcrTaskState::Queued, deadline).expect("retry task");
        assert!(statuses.snapshot(&[record], deadline)[0].expires_at.is_none());
        assert!(statuses.update(record, 7, OcrTaskState::Failed(Error::Invalid("old attempt".into())), deadline).is_none());
        statuses.update(record, 8, OcrTaskState::Completed, deadline).expect("retry completed");
        assert!(matches!(statuses.snapshot(&[record], deadline)[0].state, OcrTaskState::Completed));
    }

    #[test]
    fn ocr_feedback_bounds_terminal_metadata_and_forgets_only_the_cancelled_generation() {
        let mut statuses = TaskStatuses::default(); let start = Instant::now(); let pending = RecordId::new();
        statuses.update(pending, 1, OcrTaskState::Running, start).expect("pending task");
        let records: Vec<_> = (0..=MAX_RECENT_TASK_STATUSES).map(|_| RecordId::new()).collect();
        for (index, &record) in records.iter().enumerate() {
            statuses.update(record, 1, OcrTaskState::Failed(Error::Invalid("failed".into())), start + Duration::from_millis(index as u64)).expect("terminal task");
        }
        let now = start + Duration::from_secs(1);
        assert_eq!(statuses.snapshot(&records, now).len(), MAX_RECENT_TASK_STATUSES);
        assert!(statuses.snapshot(&[records[0]], now).is_empty());
        assert!(matches!(statuses.snapshot(&[pending], now)[0].state, OcrTaskState::Running));

        let record = records[1];
        statuses.update(record, 2, OcrTaskState::Queued, now).expect("newer task");
        statuses.remove_generation(record, 1);
        let newer = statuses.snapshot(&[record], now);
        assert_eq!(newer.len(), 1); assert_eq!(newer[0].generation, 2);
        statuses.remove_generation(record, 2);
        assert!(statuses.snapshot(&[record], now).is_empty());
        statuses.remove(pending);
        assert!(statuses.snapshot(&[pending], now).is_empty());
        statuses.clear();
        assert!(statuses.snapshot(&records, now).is_empty());
    }
}
