use std::{collections::BTreeMap, sync::{Arc, Mutex}};
use super_paste_core::{Error, Result, configuration::{ConfigurationCommit, ConfigurationMutation, ConfigurationService}};
use super_paste_format::ConfigField;
use tokio::{runtime::Handle, sync::watch};
use tokio_util::task::TaskTracker;
use uuid::Uuid;
use crate::effects::DesktopEffects;

pub struct Outcome { pub client_id: Uuid, pub mutation_id: Uuid, pub result: Result<ConfigurationCommit> }
pub type Outcomes = BTreeMap<ConfigField, Arc<Outcome>>;
struct Job { mutation: ConfigurationMutation, client_id: Uuid }
#[derive(Default)]
struct Slot { active: bool, queued: Option<Job> }
#[derive(Default)]
struct State { closed: bool, slots: BTreeMap<ConfigField, Slot> }

/// Each immediate field owns one active write and one replaceable target. A
/// slider cannot build an unbounded persistence queue or lose its final value
/// when a pane is destroyed. Drafts and credentials use the strict commit API.
pub struct ImmediateConfiguration {
    pub writer_id: Uuid,
    configuration: Arc<ConfigurationService>, effects: Arc<DesktopEffects>, runtime: Handle,
    state: Mutex<State>, tasks: TaskTracker, outcomes: watch::Sender<Outcomes>,
}
impl ImmediateConfiguration {
    pub fn new(configuration: Arc<ConfigurationService>, effects: Arc<DesktopEffects>, runtime: Handle) -> Arc<Self> {
        let (outcomes, _) = watch::channel(BTreeMap::new());
        Arc::new(Self { writer_id: Uuid::new_v4(), configuration, effects, runtime, state: Mutex::new(State::default()), tasks: TaskTracker::new(), outcomes })
    }
    pub fn subscribe(&self) -> watch::Receiver<Outcomes> { self.outcomes.subscribe() }
    pub fn enqueue(self: &Arc<Self>, mut mutation: ConfigurationMutation) -> Result<()> {
        if mutation.changes.len() != 1 || !mutation.credentials.is_empty() || mutation.base_versions.len() != 1 {
            return Err(Error::Invalid("即时设置必须只修改一个无凭据字段".into()));
        }
        let field = mutation.changes[0].field();
        if !mutation.base_versions.contains_key(&field) { return Err(Error::Invalid("即时设置缺少字段版本".into())); }
        let client_id = mutation.client_id;
        mutation.client_id = self.writer_id;
        let mut state = self.state.lock().map_err(|_| Error::Task("即时设置队列不可用".into()))?;
        if state.closed { return Err(Error::Task("应用正在退出，无法接受新的设置".into())); }
        let slot = state.slots.entry(field).or_default();
        slot.queued = Some(Job { mutation, client_id });
        if !slot.active {
            slot.active = true;
            let writer = self.clone();
            self.tasks.spawn_on(async move { writer.run(field).await }, &self.runtime);
        }
        Ok(())
    }
    async fn run(self: Arc<Self>, field: ConfigField) {
        loop {
            let next = match self.state.lock() {
                Ok(mut state) => {
                    let slot = state.slots.entry(field).or_default();
                    let job = slot.queued.take();
                    if job.is_none() { slot.active = false; }
                    job
                }
                Err(poisoned) => {
                    // Recover ownership only to reject work, never to continue
                    // mutating a poisoned queue or report success.
                    let mut state = poisoned.into_inner();
                    state.closed = true;
                    let jobs: Vec<_> = state.slots.iter_mut().filter_map(|(field, slot)| slot.queued.take().map(|job| (*field, job))).collect();
                    drop(state);
                    for (field, job) in jobs { self.publish(field, job, Err(Error::Task("即时设置队列异常，未提交该设置".into()))); }
                    return;
                }
            };
            let Some(mut job) = next else { return; };
            let current = self.configuration.snapshot();
            // Only our own preceding writes authorize rebasing. Sync, MCP and
            // draft writes retain ordinary optimistic conflict detection.
            if current.writers.get(&field) == Some(&self.writer_id) {
                let expected = job.mutation.base_versions[&field];
                let version = current.fields.get(&field).copied().unwrap_or(0);
                if version >= expected { job.mutation.base_versions.insert(field, version); }
            }
            let mutation = ConfigurationMutation {
                client_id: job.mutation.client_id, mutation_id: job.mutation.mutation_id,
                base_versions: std::mem::take(&mut job.mutation.base_versions),
                changes: std::mem::take(&mut job.mutation.changes), credentials: Vec::new(),
            };
            let result = self.configuration.commit(mutation, self.effects.clone()).await;
            self.publish(field, job, result);
        }
    }
    fn publish(&self, field: ConfigField, job: Job, result: Result<ConfigurationCommit>) {
        if let Err(error) = &result { tracing::warn!(%error, ?field, "即时设置更新失败"); }
        let outcome = Arc::new(Outcome { client_id: job.client_id, mutation_id: job.mutation.mutation_id, result });
        self.outcomes.send_modify(|outcomes| { outcomes.insert(field, outcome); });
    }
    pub async fn shutdown(&self) -> Result<()> {
        let result = self.state.lock().map(|mut state| state.closed = true).map_err(|_| Error::Task("即时设置队列异常".into()));
        self.tasks.close();
        self.tasks.wait().await;
        result
    }
}
