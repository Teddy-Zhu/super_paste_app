use std::{collections::VecDeque, sync::{Arc, Mutex}};
use serde::Serialize;
use tracing::{Event, Subscriber};
use tracing_subscriber::{Layer, layer::Context};

const MAX_EVENTS: usize = 1000;
#[derive(Clone, Serialize)]
pub struct LogEntry {
    timestamp_ms: Option<i64>, level: &'static str, target: &'static str,
    file: Option<&'static str>, line: Option<u32>,
}
pub struct EventLog { entries: Mutex<VecDeque<LogEntry>> }
pub struct EventLogLayer(Arc<EventLog>);
impl EventLog {
    pub fn new() -> Arc<Self> { Arc::new(Self { entries: Mutex::new(VecDeque::with_capacity(MAX_EVENTS)) }) }
    pub fn layer(self: &Arc<Self>) -> EventLogLayer { EventLogLayer(self.clone()) }
    pub fn snapshot(&self) -> Vec<LogEntry> { self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().cloned().collect() }
}
impl<S: Subscriber> Layer<S> for EventLogLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let metadata = event.metadata();
        // Never visit event fields or formatted messages: they may contain
        // clipboard text, server responses, credentials or user-selected paths.
        let entry = LogEntry { timestamp_ms: super_paste_core::now_ms().ok(), level: metadata.level().as_str(),
            target: metadata.target(), file: metadata.file().and_then(|file| file.rsplit(['/', '\\']).next()), line: metadata.line() };
        let mut entries = self.0.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.len() == MAX_EVENTS { entries.pop_front(); } entries.push_back(entry);
    }
}
