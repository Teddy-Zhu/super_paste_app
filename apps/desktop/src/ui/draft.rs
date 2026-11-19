use std::{collections::{BTreeMap, BTreeSet}, rc::Rc, sync::Arc};
use gpui_kit::{App, Entity};
use super_paste_core::{Error, Result, configuration::ConfigurationState, credentials::{CredentialChange, Secret, SecretChange}};
use super_paste_format::{ConfigChange, ConfigField, Configuration, CredentialId};
use uuid::Uuid;
use super::configuration::{ConfigurationModel, Feedback};

struct Submission {
    scope: BTreeSet<ConfigField>, changes: Vec<ConfigChange>, credentials: Vec<CredentialChange>, revisions: BTreeMap<ConfigField, u64>,
}
pub struct Acknowledgement {
    pub mutation_id: Uuid, pub result: Result<ConfigurationState>, pub cleared_credentials: BTreeSet<CredentialId>,
}
/// Settings pages own local edits; only acknowledged fields advance their base.
/// Neither unrelated commits nor an older response may erase newer input.
pub struct Draft {
    pub client_id: Uuid, pub value: Configuration,
    model: Entity<ConfigurationModel>, base: ConfigurationState, owned: BTreeSet<ConfigField>, dirty: BTreeSet<ConfigField>,
    revisions: BTreeMap<ConfigField, u64>, secrets: BTreeMap<CredentialId, SecretChange>, pending: BTreeMap<Uuid, Submission>, _lifetime: Rc<()>,
}
impl Draft {
    pub fn new(model: Entity<ConfigurationModel>, owned: impl IntoIterator<Item = ConfigField>, cx: &mut App) -> Self {
        let client_id = Uuid::new_v4();
        let lifetime = model.update(cx, |model, _| model.register_inline_client(client_id));
        let base = model.read(cx).state.clone();
        Self { client_id, value: (*base.snapshot.value).clone(), model, base, owned: owned.into_iter().collect(), dirty: BTreeSet::new(), revisions: BTreeMap::new(), secrets: BTreeMap::new(), pending: BTreeMap::new(), _lifetime: lifetime }
    }
    pub fn has_unsaved(&self) -> bool { !self.dirty.is_empty() }
    pub fn pending(&self) -> bool { !self.pending.is_empty() }
    pub fn pending_field(&self, field: ConfigField) -> bool { self.pending.values().any(|pending| pending.scope.contains(&field)) }
    pub fn has_saved_secret(&self, id: CredentialId) -> bool {
        !matches!(self.secrets.get(&id), Some(SecretChange::Unset)) && self.base.credentials.contains_key(&id)
    }
    pub fn secret_is_keep(&self, id: CredentialId) -> bool { matches!(self.secrets.get(&id), None | Some(SecretChange::Keep)) }
    pub fn secret_is_set(&self, id: CredentialId) -> bool { matches!(self.secrets.get(&id), Some(SecretChange::Set(_))) }
    pub fn set(&mut self, change: ConfigChange) -> Result<bool> {
        let field = change.field(); self.require_owned(field)?;
        if !change.differs(&self.value) { return Ok(false); }
        change.apply(&mut self.value); self.bump(field); self.update_dirty(field);
        Ok(true)
    }
    pub fn set_secret_input(&mut self, id: CredentialId, value: &str) -> Result<bool> {
        self.require_owned(id.field())?;
        let change = if !value.is_empty() { SecretChange::Set(Arc::new(Secret::new(value.to_owned()))) }
            else if matches!(self.secrets.get(&id), Some(SecretChange::Unset)) { SecretChange::Unset } else { SecretChange::Keep };
        self.set_secret(id, change)
    }
    pub fn clear_secret(&mut self, id: CredentialId) -> Result<bool> { self.set_secret(id, SecretChange::Unset) }
    fn set_secret(&mut self, id: CredentialId, change: SecretChange) -> Result<bool> {
        let field = id.field(); self.require_owned(field)?;
        let same = match (self.secrets.get(&id), &change) {
            (None | Some(SecretChange::Keep), SecretChange::Keep) | (Some(SecretChange::Unset), SecretChange::Unset) => true,
            (Some(SecretChange::Set(left)), SecretChange::Set(right)) => left.expose() == right.expose(),
            _ => false,
        };
        if same { return Ok(false); }
        let dirty = !matches!(&change, SecretChange::Keep);
        self.secrets.insert(id, change); self.bump(field);
        if dirty { self.dirty.insert(field); } else { self.dirty.remove(&field); }
        Ok(true)
    }
    pub fn synchronize(&mut self, state: &ConfigurationState) -> BTreeSet<ConfigField> {
        let mut changed = BTreeSet::new();
        let fields: Vec<_> = self.owned.iter().copied().collect();
        for field in fields {
            if self.dirty.contains(&field) || self.pending_field(field) { continue; }
            if state.fields.get(&field).copied().unwrap_or(0) < self.base.fields.get(&field).copied().unwrap_or(0) { continue; }
            if let Some(change) = field.change(&state.snapshot.value) {
                if change.differs(&self.value) { change.apply(&mut self.value); self.bump(field); changed.insert(field); }
            }
            if let Some(id) = credential_id(field) {
                if self.base.credentials.get(&id) != state.credentials.get(&id) { self.bump(field); changed.insert(field); }
            }
            self.adopt_base(field, state);
        }
        changed
    }
    pub fn reset(&mut self, state: &ConfigurationState) -> Result<()> {
        if self.pending() { return Err(Error::Conflict("配置正在保存，请稍后重试".into())); }
        self.reset_fields(self.owned.clone(), state)
    }
    pub fn reset_fields(&mut self, fields: impl IntoIterator<Item = ConfigField>, state: &ConfigurationState) -> Result<()> {
        let fields: Vec<_> = fields.into_iter().collect();
        for field in &fields {
            self.require_owned(*field)?;
            if self.pending_field(*field) { return Err(Error::Conflict("配置正在保存，请稍后重试".into())); }
        }
        for field in fields {
            if let Some(change) = field.change(&state.snapshot.value) { change.apply(&mut self.value); }
            if let Some(id) = credential_id(field) { self.secrets.remove(&id); }
            self.adopt_base(field, state); self.bump(field); self.dirty.remove(&field);
        }
        Ok(())
    }
    pub fn submit(&mut self, scope: impl IntoIterator<Item = ConfigField>, guards: impl IntoIterator<Item = ConfigField>, cx: &mut App) -> Result<Uuid> {
        let scope: BTreeSet<_> = scope.into_iter().collect();
        for field in &scope { self.require_owned(*field)?; }
        if self.pending.values().any(|pending| !pending.scope.is_disjoint(&scope)) { return Err(Error::Conflict("配置正在保存，请稍后重试".into())); }
        let changes: Vec<_> = scope.iter().filter(|field| self.dirty.contains(field)).filter_map(|field| field.change(&self.value)).collect();
        let credentials: Vec<_> = scope.iter().filter_map(|field| credential_id(*field)).filter_map(|id| self.secrets.get(&id).filter(|change| !matches!(change, SecretChange::Keep)).map(|change| CredentialChange { id, change: change.clone() })).collect();
        let versions: BTreeMap<_, _> = scope.iter().copied().chain(guards).map(|field| (field, self.base.fields.get(&field).copied().unwrap_or(0))).collect();
        let id = self.model.update(cx, |model, cx| model.submit(self.client_id, changes.clone(), credentials.clone(), Some(&versions), cx))?;
        let revisions = scope.iter().map(|field| (*field, self.revision(*field))).collect();
        self.pending.insert(id, Submission { scope, changes, credentials, revisions });
        Ok(id)
    }
    pub fn acknowledge(&mut self, feedback: &Feedback) -> Option<Acknowledgement> {
        if feedback.client_id != self.client_id { return None; }
        let submission = self.pending.remove(&feedback.mutation_id)?;
        let mut cleared_credentials = BTreeSet::new();
        if let Ok(state) = &feedback.result {
            for change in &submission.changes {
                let field = change.field();
                if self.revision(field) == submission.revisions.get(&field).copied().unwrap_or(0) {
                    if let Some(change) = field.change(&state.snapshot.value) { change.apply(&mut self.value); }
                }
                self.adopt_base(field, state); self.update_dirty(field);
            }
            for credential in &submission.credentials {
                let field = credential.id.field();
                self.adopt_base(field, state);
                if self.revision(field) == submission.revisions.get(&field).copied().unwrap_or(0) {
                    self.secrets.remove(&credential.id); self.dirty.remove(&field); cleared_credentials.insert(credential.id);
                }
            }
        }
        Some(Acknowledgement { mutation_id: feedback.mutation_id, result: feedback.result.clone(), cleared_credentials })
    }
    fn require_owned(&self, field: ConfigField) -> Result<()> {
        if self.owned.contains(&field) { Ok(()) } else { Err(Error::Invalid(format!("表单未持有字段 {}", field.as_str()).into())) }
    }
    fn bump(&mut self, field: ConfigField) { let revision = self.revisions.entry(field).or_default(); *revision = revision.wrapping_add(1); }
    fn revision(&self, field: ConfigField) -> u64 { self.revisions.get(&field).copied().unwrap_or(0) }
    fn update_dirty(&mut self, field: ConfigField) {
        if field.differs(&self.value, &self.base.snapshot.value) { self.dirty.insert(field); } else { self.dirty.remove(&field); }
    }
    fn adopt_base(&mut self, field: ConfigField, state: &ConfigurationState) {
        if let Some(change) = field.change(&state.snapshot.value) { change.apply(Arc::make_mut(&mut self.base.snapshot.value)); }
        if let Some(id) = credential_id(field) {
            match state.credentials.get(&id) { Some(reference) => { self.base.credentials.insert(id, *reference); } None => { self.base.credentials.remove(&id); } }
        }
        self.base.fields.insert(field, state.fields.get(&field).copied().unwrap_or(0));
    }
}
fn credential_id(field: ConfigField) -> Option<CredentialId> { CredentialId::ALL.iter().copied().find(|id| id.field() == field) }
