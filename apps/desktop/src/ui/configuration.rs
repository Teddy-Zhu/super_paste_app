use std::{collections::{BTreeMap, BTreeSet}, rc::{Rc, Weak}, sync::Arc};
use gpui_kit::*;
use super_paste_core::{Error, Result, configuration::{ConfigurationMutation, ConfigurationState}, credentials::{CredentialChange, SecretChange}};
use super_paste_format::{ConfigChange, ConfigField, Configuration};
use uuid::Uuid;
use crate::services::Services;

struct Pending {
    immediate: bool,
    changes: Vec<ConfigChange>,
    fields: BTreeSet<ConfigField>,
    base_versions: BTreeMap<ConfigField, u64>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group { Settings, Sync }
impl Pending {
    fn groups(&self) -> BTreeSet<Group> { self.fields.iter().chain(self.base_versions.keys()).filter_map(|field| group(*field)).collect() }
}
fn group(field: ConfigField) -> Option<Group> {
    if field.as_str().starts_with("sync.") || field == ConfigField::WebDavPassword { Some(Group::Sync) }
    else if field.as_str().starts_with("settings.") || matches!(field, ConfigField::OpenaiApiKey | ConfigField::ClaudeApiKey) { Some(Group::Settings) }
    else { None }
}
pub struct ConfigurationModel {
    pub state: ConfigurationState,
    pub projected: Arc<Configuration>,
    services: Arc<Services>,
    pending: BTreeMap<Uuid, Pending>,
    inline_clients: BTreeMap<Uuid, Weak<()>>,
    errors: BTreeMap<Group, SharedString>,
    credential_cleanup_warning: Option<SharedString>,
}
pub struct Feedback { pub client_id: Uuid, pub mutation_id: Uuid, pub inline: bool, pub immediate: bool, pub result: Result<ConfigurationState> }
impl EventEmitter<Feedback> for ConfigurationModel {}
pub struct ConfigurationGlobal(pub Entity<ConfigurationModel>);
impl Global for ConfigurationGlobal {}

fn cleanup_warning(error: Option<&Error>) -> Option<SharedString> {
    error.map(|error| format!("待移除的系统凭据尚未清理，应用会自动重试：{error}").into())
}

impl ConfigurationModel {
    pub fn new(services: Arc<Services>, cx: &mut Context<Self>) -> Self {
        // Subscribe before reading: a commit between these operations cannot be lost.
        let mut updates = services.configuration.subscribe();
        let state = updates.borrow_and_update().clone();
        let projected = state.snapshot.value.clone();
        let mut cleanup = services.subscribe_credential_cleanup();
        let credential_cleanup_warning = cleanup_warning(cleanup.borrow_and_update().as_ref());
        cx.spawn(async move |weak, cx| {
            while cleanup.changed().await.is_ok() {
                let warning = cleanup_warning(cleanup.borrow_and_update().as_ref());
                if weak.update(cx, |model, cx| {
                    model.credential_cleanup_warning = warning;
                    cx.notify();
                }).is_err() { break; }
            }
        }).detach();
        cx.spawn(async move |weak, cx| {
            while updates.changed().await.is_ok() {
                let state = updates.borrow_and_update().clone();
                if weak.update(cx, |model, cx| {
                    if state.snapshot.revision >= model.state.snapshot.revision {
                        model.state = state;
                        model.reproject();
                        cx.notify();
                    }
                }).is_err() { break; }
            }
        }).detach();
        let mut immediate = services.immediate_configuration.subscribe();
        cx.spawn(async move |weak, cx| {
            while immediate.changed().await.is_ok() {
                let outcomes = immediate.borrow_and_update().clone();
                if weak.update(cx, |model, cx| {
                    for outcome in outcomes.values() {
                        let Some(pending) = model.pending.remove(&outcome.mutation_id) else { continue; };
                        model.record_result(&pending, outcome.result.as_ref().map(|_| ()).map_err(Clone::clone));
                        if let Ok(commit) = &outcome.result {
                            if commit.state.snapshot.revision >= model.state.snapshot.revision { model.state = commit.state.clone(); }
                        }
                        cx.emit(Feedback { client_id: outcome.client_id, mutation_id: outcome.mutation_id, inline: model.inline_client_active(outcome.client_id), immediate: true, result: outcome.result.as_ref().map(|commit| commit.state.clone()).map_err(Clone::clone) });
                    }
                    model.reproject();
                    cx.notify();
                }).is_err() { break; }
            }
        }).detach();
        Self { state, projected, services, pending: BTreeMap::new(), inline_clients: BTreeMap::new(), errors: BTreeMap::new(), credential_cleanup_warning }
    }

    pub fn register_inline_client(&mut self, client_id: Uuid) -> Rc<()> {
        self.inline_clients.retain(|_, client| client.strong_count() > 0);
        let lifetime = Rc::new(());
        self.inline_clients.insert(client_id, Rc::downgrade(&lifetime));
        lifetime
    }
    fn inline_client_active(&mut self, client_id: Uuid) -> bool {
        let active = self.inline_clients.get(&client_id).is_some_and(|client| client.strong_count() > 0);
        if !active { self.inline_clients.remove(&client_id); }
        active
    }

    pub fn is_pending(&self, field: ConfigField) -> bool { self.pending.values().any(|pending| pending.fields.contains(&field)) }
    pub fn error(&self) -> Option<SharedString> { self.errors.values().next().cloned() }
    pub fn credential_cleanup_warning(&self) -> Option<SharedString> { self.credential_cleanup_warning.clone() }
    fn record_result(&mut self, pending: &Pending, result: Result<()>) {
        for group in pending.groups() {
            match &result { Ok(()) => { self.errors.remove(&group); }, Err(error) => { self.errors.insert(group, error.to_string().into()); } }
        }
    }

    pub fn submit(&mut self, client_id: Uuid, changes: Vec<ConfigChange>, credentials: Vec<CredentialChange>,
        versions: Option<&BTreeMap<ConfigField, u64>>, cx: &mut Context<Self>) -> Result<Uuid>
    {
        let fields: BTreeSet<_> = changes.iter().map(ConfigChange::field)
            .chain(credentials.iter().filter(|change| !matches!(&change.change, SecretChange::Keep)).map(|change| change.id.field())).collect();
        let immediate = versions.is_none() && credentials.is_empty() && changes.len() == 1;
        if self.pending.values().any(|pending| !pending.fields.is_disjoint(&fields) && !(immediate && pending.immediate)) {
            return Err(Error::Conflict("设置正在更新，请稍后重试".into()));
        }
        // A draft supplies exactly its owned fields and dependency guards. Missing
        // entries in that snapshot mean revision zero, never the latest revision.
        let mut base_versions = versions.cloned().unwrap_or_default();
        for field in &fields {
            base_versions.entry(*field).or_insert_with(|| if versions.is_some() { 0 } else { self.state.fields.get(field).copied().unwrap_or(0) });
        }
        let mutation_id = Uuid::new_v4();
        let mutation = ConfigurationMutation { client_id, mutation_id, base_versions: base_versions.clone(), changes: changes.clone(), credentials };
        if immediate {
            self.services.immediate_configuration.enqueue(mutation)?;
            self.pending.retain(|_, pending| pending.fields.is_disjoint(&fields));
            self.pending.insert(mutation_id, Pending { immediate: true, changes, fields, base_versions });
            self.reproject();
            cx.notify();
            return Ok(mutation_id);
        }
        let services = self.services.clone();
        let task = self.services.runtime.spawn(async move {
            let result = services.configuration.commit(mutation, services.effects.clone()).await;
            if let Err(error) = &result { tracing::warn!(%error, %mutation_id, "设置更新失败"); }
            result
        });
        self.pending.insert(mutation_id, Pending { immediate: false, changes, fields, base_versions });
        self.reproject();
        cx.notify();
        cx.spawn(async move |weak, cx| {
            let result = task.await.map_err(Error::from).and_then(|result| result);
            let _ = weak.update(cx, |model, cx| {
                // The response may arrive after a newer watch snapshot. Only this
                // mutation's projection is removed; a later authoritative value wins.
                if let Ok(commit) = &result {
                    if commit.state.snapshot.revision >= model.state.snapshot.revision { model.state = commit.state.clone(); }
                }
                if let Some(pending) = model.pending.remove(&mutation_id) { model.record_result(&pending, result.as_ref().map(|_| ()).map_err(Clone::clone)); }
                model.reproject();
                cx.emit(Feedback { client_id, mutation_id, inline: model.inline_client_active(client_id), immediate: false, result: result.map(|commit| commit.state) });
                cx.notify();
            });
        }).detach();
        Ok(mutation_id)
    }

    fn reproject(&mut self) {
        if self.pending.is_empty() { self.projected = self.state.snapshot.value.clone(); return; }
        let mut projected = (*self.state.snapshot.value).clone();
        for pending in self.pending.values() {
            for change in &pending.changes {
                let field = change.field();
                let version = self.state.fields.get(&field).copied().unwrap_or(0);
                let base = pending.base_versions.get(&field).copied().unwrap_or(0);
                if version == base || (pending.immediate && version >= base && self.state.writers.get(&field) == Some(&self.services.immediate_configuration.writer_id)) {
                    change.apply(&mut projected);
                }
            }
        }
        self.projected = Arc::new(projected);
    }
}
