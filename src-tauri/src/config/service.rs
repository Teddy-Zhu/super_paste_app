use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::{Map, Value, json};

use crate::{
    credentials::{CredentialError, CredentialStore, is_credential_reference},
    events::{EventSink, now_ms},
    storage::{SettingsSnapshot, Storage},
};

use super::{
    types::{
        ConfigBootstrap, ConfigChange, ConfigEffectHooks, ConfigReadResult, ConfigSnapshot,
        CredentialAvailability, CredentialChange, StoreMutationMetadata, StoreSnapshot,
        StoreWriteResult, WindowAccess,
    },
    validation,
};

const CLEANUP_META_KEY: &str = "config.credential_cleanup.v1";
const PENDING_CREDENTIALS_META_KEY: &str = "config.credential_pending.v1";
const REMOTE_SETTINGS_FIELDS: &[&str] = &[
    "colorMode",
    "maxHistorySize",
    "maxKeepTime",
    "enableLocalOCR",
    "ocrMode",
    "ocrLocalModel",
    "ocrRemoteImageFieldName",
    "ocrRemoteRequestFormat",
    "ocrRemoteResultPath",
];
const REMOTE_SYNC_FIELDS: &[&str] = &["enabled", "provider", "interval", "direction"];
const SECRET_FIELDS: &[(&str, &[&str])] = &[
    (
        "settings.ocrAIConfigs.openai.apiKey",
        &["ocrAIConfigs", "openai", "apiKey"],
    ),
    (
        "settings.ocrAIConfigs.claude.apiKey",
        &["ocrAIConfigs", "claude", "apiKey"],
    ),
    (
        "sync.params.webdav.password",
        &["params", "webdav", "password"],
    ),
];

#[derive(Clone)]
pub struct ConfigService {
    storage: Arc<Storage>,
    credentials: Arc<CredentialStore>,
    events: EventSink,
    effects: Arc<dyn ConfigEffectHooks>,
    write_gate: Arc<tokio::sync::Mutex<()>>,
}

impl ConfigService {
    pub fn new(
        storage: Arc<Storage>,
        credentials: Arc<CredentialStore>,
        events: EventSink,
        effects: Arc<dyn ConfigEffectHooks>,
    ) -> Self {
        Self {
            storage,
            credentials,
            events,
            effects,
            write_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub async fn get_bootstrap(&self, access: WindowAccess) -> Result<ConfigBootstrap, String> {
        let stored = self.storage.read_settings_snapshot().await?;
        let (settings, sync, credential_fields) = self.public_roots(&stored, access).await?;
        let settings = self.read_result("settings", settings, &stored.versions, &credential_fields);
        let sync = self.read_result("sync", sync, &stored.versions, &credential_fields);
        Ok(ConfigBootstrap { settings, sync })
    }

    pub async fn get_snapshot(
        &self,
        store_id: &str,
        access: WindowAccess,
    ) -> Result<ConfigReadResult, String> {
        if store_id != "settings" && store_id != "sync" {
            return Err("配置分组无效".to_owned());
        }
        let bootstrap = self.get_bootstrap(access).await?;
        Ok(if store_id == "settings" {
            bootstrap.settings
        } else {
            bootstrap.sync
        })
    }

    pub async fn patch(
        &self,
        store_id: &str,
        patch: Value,
        metadata: StoreMutationMetadata,
        access: WindowAccess,
    ) -> StoreWriteResult {
        let fields = match validation::validate_patch(store_id, &patch) {
            Ok(fields) => fields,
            Err(error) => return self.failed_result(store_id, access, error).await,
        };
        if metadata.client_id.trim().is_empty() || metadata.mutation_id.trim().is_empty() {
            return self
                .failed_result(store_id, access, "配置变更缺少客户端或操作标识")
                .await;
        }
        let credential_prefix = format!("{store_id}.");
        if metadata
            .credential_changes
            .keys()
            .any(|path| !path.starts_with(&credential_prefix))
        {
            return self
                .failed_result(store_id, access, "凭据变更与配置分组不匹配")
                .await;
        }

        let mut changes = BTreeMap::new();
        let patch_object = patch.as_object().expect("validated object");
        for field in fields {
            if let Some(value) = patch_object.get(&field) {
                changes.insert(format!("{store_id}.{field}"), value.clone());
            }
        }
        let result = self
            .apply_changes(
                changes,
                metadata.base_versions,
                metadata.credential_changes,
                access,
                ChangeOrigin::Renderer {
                    client_id: metadata.client_id,
                    mutation_id: metadata.mutation_id,
                },
            )
            .await;
        match result {
            Ok((snapshot, credential_fields, warning)) => StoreWriteResult {
                success: true,
                snapshot: Some(self.store_snapshot(store_id, &snapshot)),
                field_versions: Some(field_versions_for(store_id, &snapshot.versions)),
                credential_fields: Some(credential_fields),
                error: warning,
            },
            Err(error) => self.failed_result(store_id, access, error).await,
        }
    }

    pub async fn runtime_settings(&self) -> Result<Value, String> {
        let stored = self.storage.read_settings_snapshot().await?;
        let (settings, _) = roots_from_stored(&stored)?;
        Ok(safe_root(&settings))
    }

    pub async fn runtime_ocr_settings(&self) -> Result<Value, String> {
        let stored = self.storage.read_settings_snapshot().await?;
        let (stored_settings, _) = roots_from_stored(&stored)?;
        let mut settings = safe_root(&stored_settings);
        let credential_version = stored
            .versions
            .get("settings.ocrAIConfigs")
            .copied()
            .unwrap_or(0);
        settings
            .as_object_mut()
            .ok_or_else(|| "OCR 配置必须是对象".to_owned())?
            .insert(
                "_ocrCredentialVersion".to_owned(),
                Value::from(credential_version),
            );
        Ok(settings)
    }

    pub async fn runtime_ai_config(&self, provider: &str) -> Result<Value, String> {
        let secret_field = match provider {
            "openai" => SECRET_FIELDS[0],
            "claude" => SECRET_FIELDS[1],
            _ => return Err("AI OCR 提供商无效".to_owned()),
        };
        let nested = &secret_field.1[2..];
        let stored = self.storage.read_settings_snapshot().await?;
        let (settings, _) = roots_from_stored(&stored)?;
        let mut config = settings
            .pointer(&format!("/ocrAIConfigs/{provider}"))
            .cloned()
            .ok_or_else(|| format!("AI OCR 配置不存在: {provider}"))?;
        let secret = self
            .read_secret(&settings, secret_field.1, "AI OCR 凭据")
            .await?;
        set_nested(&mut config, nested, Value::String(secret))?;
        Ok(config)
    }

    pub async fn runtime_webdav_config(&self) -> Result<Value, String> {
        let stored = self.storage.read_settings_snapshot().await?;
        let (_, sync) = roots_from_stored(&stored)?;
        let (_, nested) = SECRET_FIELDS[2];
        let mut webdav = sync
            .pointer("/params/webdav")
            .cloned()
            .ok_or_else(|| "WebDAV 配置缺失".to_owned())?;
        let password = self.read_secret(&sync, nested, "WebDAV 凭据").await?;
        set_nested(&mut webdav, &["password"], Value::String(password))?;
        Ok(webdav)
    }

    pub async fn runtime_webdav_test_config(&self, candidate: &Value) -> Result<Value, String> {
        let mut webdav = self.runtime_webdav_config().await?;
        let candidate = candidate
            .as_object()
            .ok_or_else(|| "WebDAV 测试配置必须是对象".to_owned())?;
        if candidate
            .get("password")
            .is_some_and(|password| !password.is_null() && password.as_str() != Some(""))
        {
            return Err("WebDAV 测试配置不能携带密码；请先保存凭据".to_owned());
        }
        let allowed = [
            "serverUrl",
            "username",
            "remotePath",
            "timeout",
            "useHttps",
            "syncDirection",
        ];
        if let Some(unknown) = candidate
            .keys()
            .find(|key| !allowed.contains(&key.as_str()) && key.as_str() != "password")
        {
            return Err(format!("WebDAV 测试配置字段无效: {unknown}"));
        }
        let webdav = webdav
            .as_object_mut()
            .ok_or_else(|| "WebDAV 配置缺失".to_owned())?;
        for key in allowed {
            if let Some(value) = candidate.get(key) {
                webdav.insert(key.to_owned(), value.clone());
            }
        }
        Ok(Value::Object(webdav.clone()))
    }

    async fn read_secret(
        &self,
        root: &Value,
        nested: &[&str],
        label: &str,
    ) -> Result<String, String> {
        let persisted = nested_value(root, nested)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if persisted.is_empty() {
            return Ok(String::new());
        }
        if !is_credential_reference(&persisted) {
            return Err(format!("{label}引用无效"));
        }
        let credentials = self.credentials.clone();
        run_keyring(move || credentials.get_secret(&persisted))
            .await
            .map_err(|error| format!("读取{label}失败: {error}"))
    }

    pub async fn snapshot(&self) -> Result<ConfigSnapshot, String> {
        let stored = self.storage.read_settings_snapshot().await?;
        self.safe_snapshot(&stored)
    }

    pub async fn wait_idle(&self) {
        let _write = self.write_gate.lock().await;
    }

    pub async fn apply_remote_projection(
        &self,
        expected_versions: BTreeMap<String, i64>,
        projection: Value,
    ) -> Result<ConfigSnapshot, String> {
        let (settings, sync) = validate_remote_projection(&projection)?;
        let mut changes = BTreeMap::new();
        for (field, value) in settings {
            changes.insert(format!("settings.{field}"), value);
        }
        for (field, value) in sync {
            changes.insert(format!("sync.{field}"), value);
        }
        let mutation_id = uuid::Uuid::new_v4().to_string();
        self.apply_changes(
            changes,
            expected_versions,
            BTreeMap::new(),
            WindowAccess::Main,
            ChangeOrigin::Main {
                id: "webdav-sync".to_owned(),
                mutation_id,
            },
        )
        .await
        .map(|(snapshot, _, warning)| {
            if let Some(warning) = warning {
                tracing::warn!(%warning, "remote configuration applied with a credential cleanup warning");
            }
            snapshot
        })
    }

    pub async fn retry_credential_cleanup(&self) -> Result<(), String> {
        let _write = self.write_gate.lock().await;
        self.retry_credential_cleanup_inner().await
    }

    async fn retry_credential_cleanup_inner(&self) -> Result<(), String> {
        let stored = self.storage.read_settings_snapshot().await?;
        let (settings, sync) = roots_from_stored(&stored)?;
        let referenced = referenced_credentials(&settings, &sync);
        let mut failed = BTreeSet::new();
        let mut first_error = None;
        if let Some(value) = self.storage.read_meta(CLEANUP_META_KEY).await? {
            let queued = parse_credential_queue(value, "凭据清理记录格式无效")?;
            for reference in queued.difference(&referenced) {
                if let Err(error) = self.delete_credential(reference.clone()).await {
                    tracing::warn!(%error, "unable to remove an unreferenced system credential");
                    failed.insert(reference.clone());
                    first_error.get_or_insert(error);
                }
            }
            let keep = queued
                .intersection(&referenced)
                .cloned()
                .chain(failed)
                .collect::<BTreeSet<_>>();
            self.write_cleanup_queue(keep.into_iter().collect()).await?;
        }
        if let Some(value) = self.storage.read_meta(PENDING_CREDENTIALS_META_KEY).await? {
            let pending = parse_credential_queue(value, "凭据待处理记录格式无效")?;
            let mut keep = BTreeSet::new();
            for reference in pending.difference(&referenced) {
                if let Err(error) = self.delete_credential(reference.clone()).await {
                    tracing::warn!(%error, "unable to remove a pending system credential");
                    keep.insert(reference.clone());
                    first_error.get_or_insert(error);
                }
            }
            self.write_pending_credentials(keep.into_iter().collect())
                .await?;
        }
        if let Some(error) = first_error {
            Err(format!("系统凭据清理待重试: {error}"))
        } else {
            Ok(())
        }
    }

    async fn apply_changes(
        &self,
        changes: BTreeMap<String, Value>,
        expected_versions: BTreeMap<String, i64>,
        credential_changes: BTreeMap<String, CredentialChange>,
        access: WindowAccess,
        origin: ChangeOrigin,
    ) -> Result<
        (
            ConfigSnapshot,
            BTreeMap<String, CredentialAvailability>,
            Option<String>,
        ),
        String,
    > {
        let _write = self.write_gate.lock().await;
        let stored = self.storage.read_settings_snapshot().await?;
        let (before_settings, before_sync) = roots_from_stored(&stored)?;
        ensure_no_unmanaged_secrets(&before_settings, &before_sync)?;
        validation::validate_roots(&before_settings, &before_sync)?;

        validate_credential_intents(&changes, &credential_changes, access)?;
        validate_credential_inputs(
            &changes,
            &credential_changes,
            &before_settings,
            &before_sync,
        )?;
        let mut settings = before_settings.clone();
        let mut sync = before_sync.clone();
        for (path, patch_value) in &changes {
            let (store_id, field) = split_field_path(path)?;
            let target_root = if store_id == "settings" {
                &mut settings
            } else {
                &mut sync
            };
            let current = target_root
                .get_mut(field)
                .ok_or_else(|| format!("配置字段缺失: {path}"))?;
            merge_partial(current, patch_value);
        }

        let mut created_refs = Vec::new();
        let mut retired_refs = Vec::new();
        for (path, intent) in &credential_changes {
            let (_, nested) = secret_field(path).ok_or_else(|| format!("凭据字段无效: {path}"))?;
            let target_root = if path.starts_with("settings.") {
                &mut settings
            } else {
                &mut sync
            };
            let old_value = nested_value(
                if path.starts_with("settings.") {
                    &before_settings
                } else {
                    &before_sync
                },
                nested,
            )
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
            match intent {
                CredentialChange::Set => {
                    let input = nested_value_from_changes(&changes, path)
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| format!("凭据变更缺少非空值: {path}"))?
                        .to_owned();
                    if input.starts_with("credential:v1:") {
                        return Err(format!("凭据字段不接受内部引用: {path}"));
                    }
                    let reference = CredentialStore::new_reference();
                    self.enqueue_pending_credentials(std::slice::from_ref(&reference))
                        .await?;
                    created_refs.push(reference.clone());
                    let credentials = self.credentials.clone();
                    let store_reference = reference.clone();
                    if let Err(error) =
                        run_keyring(move || credentials.store_secret(&store_reference, &input))
                            .await
                    {
                        let cleanup = self.cleanup_created_refs(&created_refs).await;
                        return Err(with_cleanup_error(error, cleanup));
                    }
                    if let Err(error) = set_nested(target_root, nested, Value::String(reference)) {
                        let cleanup = self.cleanup_created_refs(&created_refs).await;
                        return Err(with_cleanup_error(error, cleanup));
                    }
                    if is_credential_reference(&old_value) {
                        retired_refs.push(old_value);
                    }
                }
                CredentialChange::Unset => {
                    if let Some(input) = nested_value_from_changes(&changes, path) {
                        if input.as_str().is_some_and(|value| !value.is_empty()) {
                            return Err(format!("清除凭据时不能传入凭据值: {path}"));
                        }
                    }
                    set_nested(target_root, nested, Value::String(String::new()))?;
                    if is_credential_reference(&old_value) {
                        retired_refs.push(old_value);
                    } else if !old_value.is_empty() {
                        return Err(format!("旧凭据引用格式无效: {path}"));
                    }
                }
            }
        }

        if let Err(error) = ensure_secret_refs(&settings, &sync)
            .and_then(|_| validation::validate_roots(&settings, &sync))
        {
            let cleanup = self.cleanup_created_refs(&created_refs).await;
            return Err(with_cleanup_error(error, cleanup));
        }

        let mut entries = Vec::new();
        let mut changes_for_hooks = Vec::new();
        let mut expected = BTreeMap::new();
        let affected_fields = changes
            .keys()
            .cloned()
            .chain(credential_changes.keys().map(|path| {
                let (store_id, field) = top_level_path(path).expect("validated credential path");
                format!("{store_id}.{field}")
            }))
            .collect::<BTreeSet<_>>();
        for path in &affected_fields {
            let (store_id, field) = split_field_path(path)?;
            let root = if store_id == "settings" {
                &settings
            } else {
                &sync
            };
            let before_root = if store_id == "settings" {
                &before_settings
            } else {
                &before_sync
            };
            let after_value = root
                .get(field)
                .cloned()
                .ok_or_else(|| format!("配置字段缺失: {path}"))?;
            let before_value = before_root
                .get(field)
                .cloned()
                .ok_or_else(|| format!("配置字段缺失: {path}"))?;
            if after_value == before_value {
                continue;
            }
            if !entries.iter().any(|(key, _): &(String, Value)| key == path) {
                entries.push((path.clone(), after_value));
                expected.insert(
                    path.clone(),
                    *expected_versions
                        .get(path)
                        .ok_or_else(|| format!("配置变更缺少字段版本: {path}"))?,
                );
                changes_for_hooks.push(ConfigChange {
                    store_id: store_id.to_owned(),
                    key: field.to_owned(),
                });
            }
        }

        let credential_fields = credential_changes
            .iter()
            .map(|(path, intent)| {
                (
                    path.clone(),
                    match intent {
                        CredentialChange::Set => CredentialAvailability::Available,
                        CredentialChange::Unset => CredentialAvailability::Unset,
                    },
                )
            })
            .collect();

        if entries.is_empty() {
            let snapshot = self.safe_snapshot(&stored)?;
            return Ok((snapshot, credential_fields, None));
        }

        let before_snapshot = ConfigSnapshot {
            settings: safe_root(&before_settings),
            sync: safe_root(&before_sync),
            versions: stored.versions.clone(),
        };
        let after_versions = predicted_versions(&stored.versions, &expected);
        let after_snapshot = ConfigSnapshot {
            settings: safe_root(&settings),
            sync: safe_root(&sync),
            versions: after_versions,
        };
        let mut prepared = match self
            .effects
            .prepare(&before_snapshot, &after_snapshot, &changes_for_hooks)
            .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                let cleanup = self.cleanup_created_refs(&created_refs).await;
                return Err(with_cleanup_error(error, cleanup));
            }
        };

        let written_versions = match self
            .storage
            .write_settings_versioned(&entries, &expected)
            .await
        {
            Ok(versions) => versions,
            Err(error) => {
                let rollback_error = prepared.rollback().await.err();
                let cleanup_error = self.cleanup_created_refs(&created_refs).await;
                return Err(combine_errors(
                    error,
                    rollback_error,
                    cleanup_error,
                    "配置提交",
                ));
            }
        };

        if let Err(apply_error) = prepared.apply().await {
            let old_entries = entries
                .iter()
                .map(|(path, _)| {
                    let (store_id, field) = split_field_path(path).expect("validated path");
                    let root = if store_id == "settings" {
                        &before_settings
                    } else {
                        &before_sync
                    };
                    (
                        path.clone(),
                        root.get(field).cloned().unwrap_or(Value::Null),
                    )
                })
                .collect::<Vec<_>>();
            let storage_rollback = self
                .storage
                .write_settings_versioned(&old_entries, &written_versions)
                .await;
            let effect_rollback_error = prepared.rollback().await.err();
            match storage_rollback {
                Ok(rollback_versions) => {
                    self.emit_changes(
                        &changes_for_hooks,
                        &rollback_versions,
                        &before_settings,
                        &before_sync,
                        &origin,
                    );
                    let cleanup_error = self.cleanup_created_refs(&created_refs).await;
                    return Err(combine_errors(
                        apply_error,
                        effect_rollback_error,
                        cleanup_error,
                        "副作用应用失败，配置已回滚",
                    ));
                }
                Err(storage_rollback_error) => {
                    let authoritative = self.snapshot().await;
                    let event_error = match authoritative {
                        Ok(snapshot) => {
                            self.emit_changes(
                                &changes_for_hooks,
                                &snapshot.versions,
                                &snapshot.settings,
                                &snapshot.sync,
                                &origin,
                            );
                            None
                        }
                        Err(error) => Some(error),
                    };
                    let credential_queue_error = if created_refs.is_empty() {
                        None
                    } else {
                        match self.enqueue_cleanup(&created_refs).await {
                            Err(error) => Some(error),
                            Ok(()) => self.retry_credential_cleanup_inner().await.err(),
                        }
                    };
                    return Err(format!(
                        "副作用应用失败: {apply_error}; 配置回滚失败: {storage_rollback_error}{}{}{}",
                        effect_rollback_error
                            .map(|error| format!("; 副作用回滚失败: {error}"))
                            .unwrap_or_default(),
                        event_error
                            .map(|error| format!("; 权威配置回读失败: {error}"))
                            .unwrap_or_default(),
                        credential_queue_error
                            .map(|error| format!("; 凭据清理待处理: {error}"))
                            .unwrap_or_default()
                    ));
                }
            }
        }

        let mut committed = after_snapshot;
        committed.versions.extend(written_versions.clone());
        self.emit_changes(
            &changes_for_hooks,
            &written_versions,
            &settings,
            &sync,
            &origin,
        );

        let warning = if retired_refs.is_empty() && created_refs.is_empty() {
            None
        } else {
            let cleanup_error = if retired_refs.is_empty() {
                self.retry_credential_cleanup_inner().await.err()
            } else {
                match self.enqueue_cleanup(&retired_refs).await {
                    Err(error) => Some(error),
                    Ok(()) => self.retry_credential_cleanup_inner().await.err(),
                }
            };
            cleanup_error.map(|error| format!("配置已保存，但凭据清理未完成: {error}"))
        };
        Ok((committed, credential_fields, warning))
    }

    async fn public_roots(
        &self,
        stored: &SettingsSnapshot,
        access: WindowAccess,
    ) -> Result<(Value, Value, BTreeMap<String, CredentialAvailability>), String> {
        let (mut settings, mut sync) = roots_from_stored(stored)?;
        let mut statuses = BTreeMap::new();
        match access {
            WindowAccess::Overlay => {
                for (path, nested) in SECRET_FIELDS {
                    let root = if path.starts_with("settings.") {
                        &mut settings
                    } else {
                        &mut sync
                    };
                    set_nested(root, nested, Value::String(String::new()))?;
                    statuses.insert((*path).to_owned(), CredentialAvailability::Unavailable);
                }
            }
            WindowAccess::Main => {
                statuses = project_credentials(&mut settings, &mut sync)?;
            }
        }
        Ok((settings, sync, statuses))
    }

    fn read_result(
        &self,
        store_id: &str,
        value: Value,
        versions: &BTreeMap<String, i64>,
        credential_fields: &BTreeMap<String, CredentialAvailability>,
    ) -> ConfigReadResult {
        ConfigReadResult {
            snapshot: StoreSnapshot {
                value,
                version: versions_for(store_id, versions)
                    .values()
                    .copied()
                    .max()
                    .unwrap_or(0),
            },
            field_versions: field_versions_for(store_id, versions),
            credential_fields: credential_fields
                .iter()
                .filter(|(path, _)| path.starts_with(&format!("{store_id}.")))
                .map(|(path, status)| (path.clone(), *status))
                .collect(),
        }
    }

    fn store_snapshot(&self, store_id: &str, snapshot: &ConfigSnapshot) -> StoreSnapshot {
        StoreSnapshot {
            value: if store_id == "settings" {
                snapshot.settings.clone()
            } else {
                snapshot.sync.clone()
            },
            version: versions_for(store_id, &snapshot.versions)
                .values()
                .copied()
                .max()
                .unwrap_or(0),
        }
    }

    fn emit_changes(
        &self,
        changes: &[ConfigChange],
        versions: &BTreeMap<String, i64>,
        settings: &Value,
        sync: &Value,
        origin: &ChangeOrigin,
    ) {
        for change in changes {
            let root = if change.store_id == "settings" {
                settings
            } else {
                sync
            };
            let Some(value) = root.get(&change.key) else {
                continue;
            };
            let key = format!("{}.{}", change.store_id, change.key);
            let version = versions
                .get(&format!("{}.{}", change.store_id, change.key))
                .copied()
                .unwrap_or(0);
            let (kind, id, mutation_id) = origin.event_fields();
            (self.events)(
                "store-value-changed",
                json!({
                    "key": key,
                    "value": event_projection(&change.store_id, &change.key, value),
                    "source": { "kind": kind, "id": id },
                    "clientId": id,
                    "mutationId": mutation_id,
                    "version": version,
                    "timestamp": now_ms(),
                    "credentialFields": event_credential_fields(&change.store_id, &change.key),
                }),
            );
        }
    }

    fn safe_snapshot(&self, stored: &SettingsSnapshot) -> Result<ConfigSnapshot, String> {
        let (settings, sync) = roots_from_stored(stored)?;
        Ok(ConfigSnapshot {
            settings: safe_root(&settings),
            sync: safe_root(&sync),
            versions: stored.versions.clone(),
        })
    }

    async fn failed_result(
        &self,
        store_id: &str,
        _access: WindowAccess,
        error: impl Into<String>,
    ) -> StoreWriteResult {
        let error = error.into();
        if store_id != "settings" && store_id != "sync" {
            return StoreWriteResult::failure(error);
        }
        let result = async {
            let stored = self.storage.read_settings_snapshot().await?;
            let snapshot = self.safe_snapshot(&stored)?;
            Ok::<_, String>(StoreWriteResult {
                success: false,
                snapshot: Some(self.store_snapshot(store_id, &snapshot)),
                field_versions: Some(field_versions_for(store_id, &snapshot.versions)),
                credential_fields: Some(BTreeMap::new()),
                error: Some(error.clone()),
            })
        }
        .await;
        match result {
            Ok(result) => result,
            Err(read_error) => {
                StoreWriteResult::failure(format!("{error}; 无法读取权威配置: {read_error}"))
            }
        }
    }

    async fn enqueue_cleanup(&self, references: &[String]) -> Result<(), String> {
        if references.is_empty() {
            return Ok(());
        }
        for _ in 0..8 {
            let previous = self.storage.read_meta(CLEANUP_META_KEY).await?;
            let mut queued = previous
                .as_ref()
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            queued.extend(references.iter().cloned());
            let next = Value::Array(queued.into_iter().map(Value::String).collect());
            if self
                .storage
                .compare_and_set_meta(CLEANUP_META_KEY, previous.as_ref(), &next)
                .await?
            {
                return Ok(());
            }
        }
        Err("凭据清理记录并发更新过多".to_owned())
    }

    async fn enqueue_pending_credentials(&self, references: &[String]) -> Result<(), String> {
        if references.is_empty() {
            return Ok(());
        }
        for _ in 0..8 {
            let previous = self.storage.read_meta(PENDING_CREDENTIALS_META_KEY).await?;
            let mut pending = previous
                .clone()
                .map(|value| parse_credential_queue(value, "凭据待处理记录格式无效"))
                .transpose()?
                .unwrap_or_default();
            pending.extend(references.iter().cloned());
            let next = Value::Array(pending.into_iter().map(Value::String).collect());
            if self
                .storage
                .compare_and_set_meta(PENDING_CREDENTIALS_META_KEY, previous.as_ref(), &next)
                .await?
            {
                return Ok(());
            }
        }
        Err("凭据待处理记录并发更新过多".to_owned())
    }

    async fn cleanup_created_refs(&self, references: &[String]) -> Option<String> {
        if references.is_empty() {
            return None;
        }
        if let Err(error) = self.enqueue_cleanup(references).await {
            return Some(error);
        }
        self.retry_credential_cleanup_inner().await.err()
    }

    async fn write_cleanup_queue(&self, references: Vec<String>) -> Result<(), String> {
        for _ in 0..8 {
            let previous = self.storage.read_meta(CLEANUP_META_KEY).await?;
            let next = Value::Array(references.iter().cloned().map(Value::String).collect());
            if self
                .storage
                .compare_and_set_meta(CLEANUP_META_KEY, previous.as_ref(), &next)
                .await?
            {
                return Ok(());
            }
        }
        Err("凭据清理记录并发更新过多".to_owned())
    }

    async fn write_pending_credentials(&self, references: Vec<String>) -> Result<(), String> {
        for _ in 0..8 {
            let previous = self.storage.read_meta(PENDING_CREDENTIALS_META_KEY).await?;
            let next = Value::Array(references.iter().cloned().map(Value::String).collect());
            if self
                .storage
                .compare_and_set_meta(PENDING_CREDENTIALS_META_KEY, previous.as_ref(), &next)
                .await?
            {
                return Ok(());
            }
        }
        Err("凭据待处理记录并发更新过多".to_owned())
    }

    async fn delete_credential(&self, reference: String) -> Result<(), String> {
        let credentials = self.credentials.clone();
        run_keyring(move || credentials.delete_secret(&reference)).await
    }
}

#[derive(Clone)]
enum ChangeOrigin {
    Renderer {
        client_id: String,
        mutation_id: String,
    },
    Main {
        id: String,
        mutation_id: String,
    },
}

impl ChangeOrigin {
    fn event_fields(&self) -> (&'static str, &str, &str) {
        match self {
            Self::Renderer {
                client_id,
                mutation_id,
            } => ("renderer", client_id, mutation_id),
            Self::Main { id, mutation_id } => ("main", id, mutation_id),
        }
    }
}

pub fn remote_projection(settings: &Value, sync: &Value) -> Result<Value, String> {
    let mut settings_projection = Map::new();
    for field in REMOTE_SETTINGS_FIELDS {
        if let Some(value) = settings.get(*field) {
            settings_projection.insert((*field).to_owned(), value.clone());
        }
    }
    if let Some(configs) = settings.get("ocrAIConfigs").and_then(Value::as_object) {
        let mut projected_configs = Map::new();
        for provider in ["openai", "claude"] {
            let Some(config) = configs.get(provider).and_then(Value::as_object) else {
                continue;
            };
            if config.len() != 4
                || ["baseUrl", "model", "api", "apiKey"]
                    .iter()
                    .any(|field| !config.contains_key(*field))
            {
                return Err(format!("本机 AI OCR 配置结构无效: {provider}"));
            }
            let mut projected = Map::new();
            for field in ["baseUrl", "model", "api"] {
                if let Some(value) = config.get(field) {
                    projected.insert(field.to_owned(), value.clone());
                }
            }
            projected_configs.insert(provider.to_owned(), Value::Object(projected));
        }
        settings_projection.insert("ocrAIConfigs".to_owned(), Value::Object(projected_configs));
    }
    let mut sync_projection = Map::new();
    for field in REMOTE_SYNC_FIELDS {
        if let Some(value) = sync.get(*field) {
            sync_projection.insert((*field).to_owned(), value.clone());
        }
    }
    if settings_projection.len() != REMOTE_SETTINGS_FIELDS.len() + 1
        || sync_projection.len() != REMOTE_SYNC_FIELDS.len()
    {
        return Err("本机配置结构不完整，无法生成 WebDAV 配置投影".to_owned());
    }
    Ok(json!({"settings": settings_projection, "sync": sync_projection}))
}

pub fn validate_remote_projection_value(projection: &Value) -> Result<(), String> {
    validate_remote_projection(projection).map(|_| ())
}

fn validate_remote_projection(
    projection: &Value,
) -> Result<(Map<String, Value>, Map<String, Value>), String> {
    let object = projection
        .as_object()
        .ok_or_else(|| "远端配置必须是对象".to_owned())?;
    if object.len() != 2 || !object.contains_key("settings") || !object.contains_key("sync") {
        return Err("远端配置结构无效".to_owned());
    }
    let settings = object["settings"]
        .as_object()
        .ok_or_else(|| "远端 settings 配置无效".to_owned())?;
    let sync = object["sync"]
        .as_object()
        .ok_or_else(|| "远端 sync 配置无效".to_owned())?;
    for field in settings.keys() {
        if !REMOTE_SETTINGS_FIELDS.contains(&field.as_str()) && field != "ocrAIConfigs" {
            return Err(format!("远端设置字段不在同步白名单: {field}"));
        }
    }
    if settings.len() != REMOTE_SETTINGS_FIELDS.len() + 1 {
        return Err("远端设置字段不完整".to_owned());
    }
    let ai = settings
        .get("ocrAIConfigs")
        .and_then(Value::as_object)
        .ok_or_else(|| "远端 AI OCR 设置无效".to_owned())?;
    if ai.len() != 2 || !ai.contains_key("openai") || !ai.contains_key("claude") {
        return Err("远端 AI OCR 配置不完整".to_owned());
    }
    for (provider, value) in ai {
        let config = value
            .as_object()
            .ok_or_else(|| format!("远端 AI OCR 配置无效: {provider}"))?;
        if config.len() != 3
            || ["baseUrl", "model", "api"]
                .iter()
                .any(|key| !config.contains_key(*key))
        {
            return Err(format!("远端 AI OCR 字段不在同步白名单: {provider}"));
        }
    }
    for field in sync.keys() {
        if !REMOTE_SYNC_FIELDS.contains(&field.as_str()) {
            return Err(format!("远端同步字段不在同步白名单: {field}"));
        }
    }
    if sync.len() != REMOTE_SYNC_FIELDS.len() {
        return Err("远端同步计划不完整".to_owned());
    }
    let mut settings_values = settings.clone();
    let mut sync_values = sync.clone();
    settings_values.insert("ocrAIConfigs".to_owned(), ai_projection_for_patch(&ai)?);
    let defaults = data_format::default_preferences();
    let settings_schema = defaults["settings"]
        .as_object()
        .expect("default settings object");
    for (field, value) in &settings_values {
        validation::validate_patch("settings", &json!({field:value}))?;
        if !settings_schema.contains_key(field) {
            return Err(format!("远端设置字段无效: {field}"));
        }
    }
    for (field, value) in &sync_values {
        validation::validate_patch("sync", &json!({field:value}))?;
    }
    Ok((
        std::mem::take(&mut settings_values),
        std::mem::take(&mut sync_values),
    ))
}

fn ai_projection_for_patch(configs: &Map<String, Value>) -> Result<Value, String> {
    let mut result = Map::new();
    for provider in ["openai", "claude"] {
        let config = configs
            .get(provider)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("远端 AI OCR 配置无效: {provider}"))?;
        result.insert(provider.to_owned(), Value::Object(config.clone()));
    }
    Ok(Value::Object(result))
}

fn roots_from_stored(stored: &SettingsSnapshot) -> Result<(Value, Value), String> {
    let defaults = data_format::default_preferences();
    let mut settings = stored
        .values
        .get("settings")
        .cloned()
        .unwrap_or_else(|| defaults["settings"].clone());
    let mut sync = stored
        .values
        .get("sync")
        .cloned()
        .unwrap_or_else(|| defaults["sync"].clone());
    merge_missing(&mut settings, &defaults["settings"]);
    merge_missing(&mut sync, &defaults["sync"]);
    Ok((settings, sync))
}

fn project_credentials(
    settings: &mut Value,
    sync: &mut Value,
) -> Result<BTreeMap<String, CredentialAvailability>, String> {
    let mut statuses = BTreeMap::new();
    for (path, nested) in SECRET_FIELDS {
        let root = if path.starts_with("settings.") {
            &mut *settings
        } else {
            &mut *sync
        };
        let persisted = nested_value(root, nested)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let status = if persisted.is_empty() {
            CredentialAvailability::Unset
        } else if is_credential_reference(&persisted) {
            CredentialAvailability::Configured
        } else {
            CredentialAvailability::Error
        };
        set_nested(root, nested, Value::String(String::new()))?;
        statuses.insert((*path).to_owned(), status);
    }
    Ok(statuses)
}

fn ensure_no_unmanaged_secrets(settings: &Value, sync: &Value) -> Result<(), String> {
    for (path, nested) in SECRET_FIELDS {
        let root = if path.starts_with("settings.") {
            settings
        } else {
            sync
        };
        let value = nested_value(root, nested)
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !value.is_empty() && !is_credential_reference(value) {
            return Err(format!("配置数据库包含未托管的明文凭据: {path}"));
        }
    }
    Ok(())
}

fn ensure_secret_refs(settings: &Value, sync: &Value) -> Result<(), String> {
    ensure_no_unmanaged_secrets(settings, sync)
}

fn referenced_credentials(settings: &Value, sync: &Value) -> BTreeSet<String> {
    SECRET_FIELDS
        .iter()
        .filter_map(|(path, nested)| {
            let root = if path.starts_with("settings.") {
                settings
            } else {
                sync
            };
            nested_value(root, nested)
                .and_then(Value::as_str)
                .filter(|value| is_credential_reference(value))
                .map(str::to_owned)
        })
        .collect()
}

fn parse_credential_queue(value: Value, error: &str) -> Result<BTreeSet<String>, String> {
    value
        .as_array()
        .ok_or_else(|| error.to_owned())?
        .iter()
        .map(|value| {
            let reference = value.as_str().ok_or_else(|| error.to_owned())?;
            if !is_credential_reference(reference) {
                return Err(error.to_owned());
            }
            Ok(reference.to_owned())
        })
        .collect()
}

fn secret_field(path: &str) -> Option<(&'static str, &'static [&'static str])> {
    SECRET_FIELDS
        .iter()
        .find(|(candidate, _)| *candidate == path)
        .map(|(candidate, nested)| (*candidate, *nested))
}

fn secret_paths_for_store(store_id: &str) -> Vec<(&'static str, &'static [&'static str])> {
    SECRET_FIELDS
        .iter()
        .filter(|(path, _)| path.starts_with(&format!("{store_id}.")))
        .map(|(path, nested)| (*path, *nested))
        .collect()
}

fn validate_credential_intents(
    changes: &BTreeMap<String, Value>,
    intents: &BTreeMap<String, CredentialChange>,
    access: WindowAccess,
) -> Result<(), String> {
    if !intents.is_empty() && access != WindowAccess::Main {
        return Err("当前窗口无权修改系统凭据".to_owned());
    }
    for path in intents.keys() {
        if secret_field(path).is_none() {
            return Err(format!("凭据字段无效: {path}"));
        }
    }
    for (path, _) in SECRET_FIELDS {
        let (store_id, field) = top_level_path(path)?;
        let Some(patch) = changes.get(&format!("{store_id}.{field}")) else {
            continue;
        };
        if nested_value(patch, &secret_field(path).expect("known path").1).is_some()
            && !intents.contains_key(*path)
        {
            return Err(format!(
                "凭据字段必须通过 credentialChanges 显式更新: {path}"
            ));
        }
    }
    Ok(())
}

fn validate_credential_inputs(
    changes: &BTreeMap<String, Value>,
    intents: &BTreeMap<String, CredentialChange>,
    settings: &Value,
    sync: &Value,
) -> Result<(), String> {
    for (path, intent) in intents {
        let (_, nested) = secret_field(path).ok_or_else(|| format!("凭据字段无效: {path}"))?;
        let root = if path.starts_with("settings.") {
            settings
        } else {
            sync
        };
        let old_value = nested_value(root, nested)
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input = nested_value_from_changes(changes, path);
        match intent {
            CredentialChange::Set => {
                let input = input
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("凭据变更缺少非空值: {path}"))?;
                if input.starts_with("credential:v1:") {
                    return Err(format!("凭据字段不接受内部引用: {path}"));
                }
            }
            CredentialChange::Unset => {
                if input
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty())
                {
                    return Err(format!("清除凭据时不能传入凭据值: {path}"));
                }
                if !old_value.is_empty() && !is_credential_reference(old_value) {
                    return Err(format!("旧凭据引用格式无效: {path}"));
                }
            }
        }
    }
    Ok(())
}

fn nested_value_from_changes<'a>(
    changes: &'a BTreeMap<String, Value>,
    secret_path: &str,
) -> Option<&'a Value> {
    let (store_id, field) = secret_path.split_once('.')?;
    let top = field.split('.').next()?;
    let patch = changes.get(&format!("{store_id}.{top}"))?;
    let nested = secret_field(secret_path)?.1;
    nested_value(patch, &nested[1..])
}

fn split_field_path(path: &str) -> Result<(&str, &str), String> {
    let (store_id, field) = path
        .split_once('.')
        .ok_or_else(|| format!("配置字段路径无效: {path}"))?;
    if (store_id != "settings" && store_id != "sync") || field.contains('.') {
        return Err(format!("配置字段路径无效: {path}"));
    }
    Ok((store_id, field))
}

fn top_level_path(path: &str) -> Result<(&str, &str), String> {
    let (store_id, fields) = path
        .split_once('.')
        .ok_or_else(|| format!("配置字段路径无效: {path}"))?;
    let field = fields
        .split('.')
        .next()
        .filter(|field| !field.is_empty())
        .ok_or_else(|| format!("配置字段路径无效: {path}"))?;
    if store_id != "settings" && store_id != "sync" {
        return Err(format!("配置字段路径无效: {path}"));
    }
    Ok((store_id, field))
}

fn versions_for(store_id: &str, versions: &BTreeMap<String, i64>) -> BTreeMap<String, i64> {
    versions
        .iter()
        .filter(|(path, _)| path.starts_with(&format!("{store_id}.")))
        .map(|(path, version)| (path.clone(), *version))
        .collect()
}

fn field_versions_for(store_id: &str, versions: &BTreeMap<String, i64>) -> BTreeMap<String, i64> {
    versions_for(store_id, versions)
}

fn predicted_versions(
    current: &BTreeMap<String, i64>,
    expected: &BTreeMap<String, i64>,
) -> BTreeMap<String, i64> {
    let mut next = current.clone();
    for path in expected.keys() {
        let version = current.get(path).copied().unwrap_or(0);
        next.insert(path.clone(), version.saturating_add(1));
    }
    next
}

fn merge_missing(target: &mut Value, defaults: &Value) {
    let (Some(target), Some(defaults)) = (target.as_object_mut(), defaults.as_object()) else {
        return;
    };
    for (key, default) in defaults {
        if let Some(existing) = target.get_mut(key) {
            merge_missing(existing, default);
        } else {
            target.insert(key.clone(), default.clone());
        }
    }
}

fn merge_partial(target: &mut Value, patch: &Value) {
    if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            if let Some(existing) = target.get_mut(key) {
                merge_partial(existing, value);
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    } else {
        *target = patch.clone();
    }
}

fn nested_value<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(root, |value, field| value.get(*field))
}

fn set_nested(root: &mut Value, path: &[&str], value: Value) -> Result<(), String> {
    let (last, parents) = path
        .split_last()
        .ok_or_else(|| "配置字段路径为空".to_owned())?;
    let mut current = root;
    for field in parents {
        current = current
            .get_mut(*field)
            .ok_or_else(|| format!("配置字段缺失: {field}"))?;
    }
    current
        .as_object_mut()
        .ok_or_else(|| "配置字段父节点不是对象".to_owned())?
        .insert((*last).to_owned(), value);
    Ok(())
}

fn safe_root(root: &Value) -> Value {
    let mut safe = root.clone();
    for (_, nested) in SECRET_FIELDS {
        let _ = set_nested(&mut safe, nested, Value::String(String::new()));
    }
    safe
}

fn event_projection(store_id: &str, field: &str, value: &Value) -> Value {
    let mut projected = value.clone();
    if store_id == "settings" && field == "ocrAIConfigs" {
        for provider in ["openai", "claude"] {
            if let Some(config) = projected.get_mut(provider).and_then(Value::as_object_mut) {
                config.remove("apiKey");
            }
        }
    }
    if store_id == "sync" && field == "params" {
        if let Some(webdav) = projected
            .pointer_mut("/webdav")
            .and_then(Value::as_object_mut)
        {
            webdav.remove("password");
        }
    }
    projected
}

fn event_credential_fields(
    store_id: &str,
    field: &str,
) -> BTreeMap<String, CredentialAvailability> {
    let prefix = format!("{store_id}.{field}");
    secret_paths_for_store(store_id)
        .into_iter()
        .filter(|(path, _)| path.starts_with(&format!("{prefix}.")))
        .map(|(path, _)| (path.to_owned(), CredentialAvailability::Unavailable))
        .collect()
}

fn with_cleanup_error(error: String, cleanup: Option<String>) -> String {
    cleanup
        .map(|cleanup| format!("{error}; 临时凭据清理失败: {cleanup}"))
        .unwrap_or(error)
}

fn combine_errors(
    primary: String,
    rollback: Option<String>,
    cleanup: Option<String>,
    context: &str,
) -> String {
    format!(
        "{context}: {primary}{}{}",
        rollback
            .map(|error| format!("; 补偿失败: {error}"))
            .unwrap_or_default(),
        cleanup
            .map(|error| format!("; 临时凭据清理失败: {error}"))
            .unwrap_or_default()
    )
}

async fn run_keyring<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CredentialError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("系统凭据工作线程失败: {error}"))?
        .map_err(|error| error.to_string())
}
