use std::{collections::{BTreeMap, BTreeSet}, sync::Arc};
use super_paste_format::{ConfigChange, ConfigField, ConfigSnapshot, Configuration, CredentialId, OcrMode};
use tokio::{runtime::Handle, sync::{Mutex, Semaphore, watch}};
use tokio_util::task::TaskTracker;
use crate::{Error, Result, events::{AppEvent, EventBus}, storage::Storage};
use crate::credentials::{CredentialChange, CredentialReferences, CredentialVault, Secret, SecretChange, read_references};

#[derive(Clone, Debug)]
pub struct ConfigurationState {
    pub snapshot: ConfigSnapshot,
    pub fields: BTreeMap<ConfigField, u64>,
    pub credentials: CredentialReferences,
    /// Runtime ownership proves that a queued immediate edit follows our own
    /// successful write. An unrelated writer must still cause a version conflict.
    pub writers: BTreeMap<ConfigField, uuid::Uuid>,
}
fn ocr_fields(mode: OcrMode) -> &'static [ConfigField] {
    use ConfigField::*;
    use super_paste_format::OcrMode as Mode;
    match mode {
        Mode::Local => &[EnableLocalOcr, ConfigField::OcrMode, OcrLocalModel],
        Mode::Remote => &[EnableLocalOcr, ConfigField::OcrMode, OcrRemoteUrl, OcrRemoteExtraParams, OcrRemoteImageFieldName, OcrRemoteRequestFormat, OcrRemoteResultPath],
        Mode::Openai => &[EnableLocalOcr, ConfigField::OcrMode, OpenaiConfig, OpenaiApiKey],
        Mode::Claude => &[EnableLocalOcr, ConfigField::OcrMode, ClaudeConfig, ClaudeApiKey],
    }
}
impl ConfigurationState {
    pub fn ocr_revision(&self) -> u64 { ocr_fields(self.snapshot.value.settings.ocr_mode).iter().filter_map(|field| self.fields.get(field)).copied().max().unwrap_or(0) }
}
pub(crate) fn ocr_revision_in(connection: &rusqlite::Connection) -> Result<u64> {
    #[derive(serde::Deserialize)] struct Mode { #[serde(rename = "ocrMode")] mode: OcrMode }
    #[derive(serde::Deserialize)] struct Projection { settings: Mode }
    let json: String = connection.query_row("SELECT value_json FROM settings WHERE key='configuration'", [], |row| row.get(0))?;
    let projection: Projection = serde_json::from_str(&json)?;
    let fields = ocr_fields(projection.settings.mode);
    let mut statement = connection.prepare_cached("SELECT key,revision FROM config_field_versions")?;
    let mut rows = statement.query([])?;
    let mut revision = 0;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        if fields.iter().any(|field| field.as_str() == key) { revision = revision.max(row.get::<_, u64>(1)?); }
    }
    Ok(revision)
}
#[derive(Clone)]
pub struct RuntimeConfiguration {
    pub value: Arc<Configuration>, pub credentials: CredentialReferences, vault: Arc<CredentialVault>,
}
impl RuntimeConfiguration {
    pub async fn credential(&self, id: CredentialId) -> Result<Option<Arc<Secret>>> {
        match self.credentials.get(&id) { Some(reference) => self.vault.read(*reference).await, None => Ok(None) }
    }
}
#[derive(Clone, Debug)]
pub struct ConfigurationMutation {
    pub client_id: uuid::Uuid,
    pub mutation_id: uuid::Uuid,
    /// Includes every owned field; optional dependency fields are optimistic preconditions.
    pub base_versions: BTreeMap<ConfigField, u64>,
    pub changes: Vec<ConfigChange>,
    pub credentials: Vec<CredentialChange>,
}
#[derive(Clone, Debug)]
pub struct ConfigurationCommit {
    pub client_id: uuid::Uuid, pub mutation_id: uuid::Uuid, pub state: ConfigurationState,
    /// Configuration is committed even if retiring an obsolete system credential needs a retry.
    pub credential_cleanup_error: Option<Error>,
}

pub struct ConfigurationService {
    storage: Arc<Storage>, gate: Mutex<()>, state: watch::Sender<ConfigurationState>, events: EventBus,
    runtime: Handle, pending: Arc<Semaphore>, credentials: Arc<CredentialVault>,
    admission: std::sync::Mutex<bool>, tasks: TaskTracker,
}
impl ConfigurationService {
    pub async fn open(storage: Arc<Storage>, credentials: Arc<CredentialVault>, events: EventBus, runtime: Handle) -> Result<Arc<Self>> {
        let state = storage.configuration_state().await?;
        let (state, _) = watch::channel(state);
        Ok(Arc::new(Self { storage, gate: Mutex::new(()), state, events, runtime, pending: Arc::new(Semaphore::new(32)), credentials, admission: std::sync::Mutex::new(true), tasks: TaskTracker::new() }))
    }
    pub fn snapshot(&self) -> ConfigurationState { self.state.borrow().clone() }
    pub fn subscribe(&self) -> watch::Receiver<ConfigurationState> { self.state.subscribe() }
    pub async fn credential(&self, id: CredentialId) -> Result<Option<Arc<Secret>>> {
        // Keep the chosen reference alive until the read completes; retirement runs inside commits.
        let _gate = self.gate.lock().await;
        match self.snapshot().credentials.get(&id) { Some(reference) => self.credentials.read(*reference).await, None => Ok(None) }
    }
    pub(crate) async fn guarded_credential(&self, id: CredentialId, expected: &ConfigurationState, fields: &[ConfigField]) -> Result<(Arc<Configuration>, Arc<Secret>)> {
        let _gate = self.gate.lock().await;
        let state = self.snapshot();
        if fields.iter().any(|field| state.fields.get(field).copied().unwrap_or(0) != expected.fields.get(field).copied().unwrap_or(0)) {
            return Err(Error::Conflict("接口配置已变化，请重新发起请求".into()));
        }
        let label = match id { CredentialId::WebDavPassword => "WebDAV 密码", CredentialId::McpApiKey => "MCP 密钥", _ => "AI OCR 密钥" };
        let reference = state.credentials.get(&id).ok_or_else(|| Error::Invalid(format!("请先设置{label}").into()))?;
        let secret = self.credentials.read(*reference).await?.ok_or_else(|| Error::Invalid(format!("已配置的{label}不可读取，请重新设置").into()))?;
        Ok((state.snapshot.value, secret))
    }

    /// Runtime side effects are prepared/applied by the host inside the same serialized commit.
    pub async fn commit<E: ConfigurationEffects + 'static>(self: &Arc<Self>, mutation: ConfigurationMutation, effects: Arc<E>) -> Result<ConfigurationCommit> {
        let task = {
            let accepting = self.admission.lock().map_err(|_| Error::Task("配置接入锁损坏".into()))?;
            if !*accepting { return Err(Error::Cancelled); }
            let permit = self.pending.clone().try_acquire_owned().map_err(|error| match error {
                tokio::sync::TryAcquireError::Closed => Error::Cancelled,
                tokio::sync::TryAcquireError::NoPermits => Error::ResourceLimit("配置提交队列已满".into()),
            })?;
            let service = self.clone();
            // A dropped view must not interrupt side effects/persistence. Exit drains accepted commits.
            self.tasks.spawn_on(async move {
                let _permit = permit;
                service.commit_inner(mutation, effects.as_ref()).await
            }, &self.runtime)
        };
        task.await?
    }

    pub async fn shutdown(&self) -> Result<()> {
        let failure = match self.admission.lock() {
            Ok(mut accepting) => { *accepting = false; None }
            Err(poisoned) => { *poisoned.into_inner() = false; Some(Error::Task("配置接入锁损坏".into())) }
        };
        self.pending.close();
        self.tasks.close();
        self.tasks.wait().await;
        match failure { Some(error) => Err(error), None => Ok(()) }
    }

    async fn commit_inner<E: ConfigurationEffects>(&self, mutation: ConfigurationMutation, effects: &E) -> Result<ConfigurationCommit> {
        let _gate = self.gate.lock().await;
        let before = self.snapshot();
        let mut candidate = (*before.snapshot.value).clone();
        let mut changed = BTreeSet::new();
        let mut owned = BTreeSet::new();
        for change in &mutation.changes {
            let field = change.field();
            if !owned.insert(field) { return Err(Error::Invalid("同一提交包含重复配置字段".into())); }
            if change.differs(&before.snapshot.value) { changed.insert(field); }
            change.apply(&mut candidate);
        }
        let mut credential_ids = BTreeSet::new();
        let mut credential_changes = false;
        for change in &mutation.credentials {
            if !credential_ids.insert(change.id) { return Err(Error::Invalid("同一提交包含重复凭据字段".into())); }
            if matches!(change.change, SecretChange::Keep) { continue; }
            owned.insert(change.id.field());
            let actual = matches!(change.change, SecretChange::Set(_)) || before.credentials.contains_key(&change.id);
            if actual { changed.insert(change.id.field()); }
            credential_changes |= actual;
        }
        for field in &owned {
            let expected = mutation.base_versions.get(field).copied().ok_or_else(|| Error::Invalid(format!("配置字段缺少基础版本：{}", field.as_str()).into()))?;
            if expected != before.fields.get(field).copied().unwrap_or(0) { return Err(Error::Conflict(field.as_str().into())); }
        }
        for (field, expected) in &mutation.base_versions {
            if *expected != before.fields.get(field).copied().unwrap_or(0) { return Err(Error::Conflict(field.as_str().into())); }
        }
        candidate.validate()?;
        if candidate == *before.snapshot.value && !credential_changes {
            return Ok(ConfigurationCommit { client_id: mutation.client_id, mutation_id: mutation.mutation_id, state: before, credential_cleanup_error: None });
        }
        let credentials = self.credentials.prepare(&before.credentials, &mutation.credentials).await?;
        let runtime_before = RuntimeConfiguration { value: before.snapshot.value.clone(), credentials: before.credentials.clone(), vault: self.credentials.clone() };
        let runtime_after = RuntimeConfiguration { value: Arc::new(candidate.clone()), credentials: credentials.after.clone(), vault: self.credentials.clone() };
        let mut prepared = match effects.prepare(runtime_before, runtime_after, changed.clone()).await {
            Ok(prepared) => prepared, Err(error) => return Err(credentials.abort(error).await),
        };
        if let Err(error) = prepared.apply().await {
            return Err(credentials.abort(rollback_error(error, prepared.rollback().await)).await);
        }
        let result = self.storage.commit_configuration(candidate, credentials.after.clone(), before.snapshot.revision, changed.clone()).await;
        let mut committed = match result {
            Ok(value) => value,
            Err(error) => return Err(credentials.abort(rollback_error(error, prepared.rollback().await)).await),
        };
        committed.writers = before.writers;
        for field in changed { committed.writers.insert(field, mutation.client_id); }
        self.state.send_replace(committed.clone());
        self.events.publish(AppEvent::ConfigurationCommitted(committed.snapshot.clone()));
        let credential_cleanup_error = credentials.finish().await.err();
        if let Some(error) = &credential_cleanup_error { self.events.publish(AppEvent::CredentialCleanupFailed(error.clone())); }
        Ok(ConfigurationCommit { client_id: mutation.client_id, mutation_id: mutation.mutation_id, state: committed, credential_cleanup_error })
    }
}

pub trait ConfigurationEffects: Send + Sync {
    fn prepare(&self, before: RuntimeConfiguration, after: RuntimeConfiguration, fields: BTreeSet<ConfigField>)
        -> impl std::future::Future<Output = Result<impl PreparedConfigurationEffects + 'static>> + Send;
}
pub trait PreparedConfigurationEffects: Send {
    fn apply(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
    fn rollback(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
}
fn rollback_error(error: Error, rollback: Result<()>) -> Error {
    match rollback { Ok(()) => error, Err(rollback) => Error::Task(format!("{error}；配置副作用回滚失败：{rollback}").into()) }
}

impl Storage {
    pub(crate) async fn configuration_state(&self) -> Result<ConfigurationState> {
        self.call(|connection| {
            let transaction = connection.transaction()?;
            let (json, revision): (String, u64) = transaction.query_row("SELECT value_json,revision FROM settings WHERE key='configuration'", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let value: Configuration = serde_json::from_str(&json)?;
            value.validate()?;
            let mut fields = BTreeMap::new();
            {
                let mut statement = transaction.prepare("SELECT key,revision FROM config_field_versions")?;
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    let field = ConfigField::ALL.iter().find(|field| field.as_str() == key).copied()
                        .ok_or_else(|| Error::Database("数据库包含不支持的配置字段".into()))?;
                    fields.insert(field, row.get(1)?);
                }
            }
            for field in ConfigField::ALL { fields.entry(*field).or_insert(0); }
            let credentials = read_references(&transaction)?;
            transaction.commit()?;
            Ok(ConfigurationState { snapshot: ConfigSnapshot { value: Arc::new(value), revision }, fields, credentials, writers: BTreeMap::new() })
        }).await
    }
    async fn commit_configuration(&self, configuration: Configuration, credentials: CredentialReferences, expected_revision: u64, changed: BTreeSet<ConfigField>) -> Result<ConfigurationState> {
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let revision = super::storage::revision(&transaction)?;
            let json = serde_json::to_string(&configuration)?;
            if transaction.execute("UPDATE settings SET value_json=?1,revision=?2 WHERE key='configuration' AND revision=?3", rusqlite::params![json, revision, expected_revision])? == 0 {
                return Err(Error::Conflict("configuration".into()));
            }
            let previous = read_references(&transaction)?;
            for (id, reference) in previous {
                if credentials.get(&id) != Some(&reference) {
                    transaction.execute("INSERT OR IGNORE INTO credential_garbage(reference) VALUES(?1)", [reference.account()])?;
                }
            }
            transaction.execute("DELETE FROM credential_refs", [])?;
            for (id, reference) in &credentials {
                transaction.execute("INSERT INTO credential_refs(key,reference) VALUES(?1,?2)", rusqlite::params![id.account(), reference.account()])?;
                transaction.execute("DELETE FROM credential_garbage WHERE reference=?1", [reference.account()])?;
            }
            {
                let mut statement = transaction.prepare_cached("INSERT INTO config_field_versions(key,revision) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET revision=excluded.revision")?;
                for field in changed { statement.execute(rusqlite::params![field.as_str(), revision])?; }
            }
            let mut fields = BTreeMap::new();
            {
                let mut statement = transaction.prepare("SELECT key,revision FROM config_field_versions")?;
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    let key: String = row.get(0)?;
                    let field = ConfigField::ALL.iter().find(|field| field.as_str() == key).copied().ok_or_else(|| Error::Database("数据库配置字段无效".into()))?;
                    fields.insert(field, row.get(1)?);
                }
            }
            for field in ConfigField::ALL { fields.entry(*field).or_insert(0); }
            transaction.commit()?;
            Ok(ConfigurationState { snapshot: ConfigSnapshot { value: Arc::new(configuration), revision: revision as u64 }, fields, credentials, writers: BTreeMap::new() })
        }).await
    }
}
