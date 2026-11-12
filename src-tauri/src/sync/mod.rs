mod document;
mod webdav;

use document::{MAX_DOCUMENT_BYTES, RemoteEnvelope};
use webdav::{RemoteDocument, WebDavTarget, is_strong_etag, parse_webdav_config};

use std::{path::PathBuf, sync::Arc, time::Duration};

use reqwest::{
    Method, StatusCode,
    header::{CONTENT_TYPE, HeaderMap, HeaderValue, IF_MATCH, IF_NONE_MATCH},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, sync::Mutex, time::sleep};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    config::{
        ConfigService, ConfigSnapshot, CredentialAvailability, WindowAccess, remote_projection,
        validate_remote_projection_value,
    },
    events::{EventSink, now_ms},
    storage::Storage,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyncDirection {
    Upload,
    Download,
    Bidirectional,
}

impl SyncDirection {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "upload" => Ok(Self::Upload),
            "download" => Ok(Self::Download),
            "bidirectional" => Ok(Self::Bidirectional),
            _ => Err("同步方向无效".to_owned()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRunResult {
    pub success: bool,
    pub message: String,
    pub errors: Vec<String>,
    pub uploaded_files: Vec<String>,
    pub downloaded_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
    #[serde(skip)]
    already_reported_conflict: bool,
}

impl SyncRunResult {
    fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            errors: Vec::new(),
            uploaded_files: Vec::new(),
            downloaded_files: Vec::new(),
            conflict: None,
            already_reported_conflict: false,
        }
    }

    fn failure(error: impl Into<String>) -> Self {
        let error = error.into();
        Self {
            success: false,
            message: error.clone(),
            errors: vec![error],
            uploaded_files: Vec::new(),
            downloaded_files: Vec::new(),
            conflict: None,
            already_reported_conflict: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncTestResult {
    pub success: bool,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncState {
    #[default]
    Idle,
    Syncing,
    Success,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub is_running: bool,
    pub last_sync_time: i64,
    pub next_sync_time: i64,
    pub sync_status: SyncState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncBaseline {
    revision: String,
    payload_sha256: String,
    etag: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingUpload {
    envelope: RemoteEnvelope,
    target_parent: Option<String>,
}

struct ResolvedOperation {
    snapshot: ConfigSnapshot,
    payload: Value,
    payload_sha256: String,
    direction: SyncDirection,
}

struct Scheduler {
    cancel: CancellationToken,
}

#[derive(Clone, Copy)]
enum RunKind {
    Scheduled,
    Manual,
}

pub struct SyncService {
    config: Arc<ConfigService>,
    storage: Arc<Storage>,
    events: EventSink,
    app_data_dir: PathBuf,
    operation_gate: Mutex<()>,
    configure_gate: Mutex<()>,
    scheduler: Mutex<Option<Scheduler>>,
    status: Mutex<SyncStatus>,
    shutting_down: CancellationToken,
}

impl SyncService {
    pub fn new(
        config: Arc<ConfigService>,
        storage: Arc<Storage>,
        events: EventSink,
        app_data_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            storage,
            events,
            app_data_dir,
            operation_gate: Mutex::new(()),
            configure_gate: Mutex::new(()),
            scheduler: Mutex::new(None),
            status: Mutex::new(SyncStatus {
                is_running: false,
                last_sync_time: 0,
                next_sync_time: 0,
                sync_status: SyncState::Idle,
            }),
            shutting_down: CancellationToken::new(),
        })
    }

    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        self.configure().await
    }

    pub async fn configure(self: &Arc<Self>) -> Result<(), String> {
        let _configure = self.configure_gate.lock().await;
        if self.shutting_down.is_cancelled() {
            return Err("同步服务正在关闭".to_owned());
        }
        let config = self.config.get_snapshot("sync", WindowAccess::Main).await?;
        let sync = config.snapshot.value;
        let requested_enabled = sync
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let provider = sync
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let interval = sync.get("interval").and_then(Value::as_i64).unwrap_or(0);
        if requested_enabled && !provider.is_empty() && provider != "webdav" {
            return Err("当前只支持 WebDAV 配置同步".to_owned());
        }
        if requested_enabled && interval <= 0 {
            return Err("同步间隔必须大于零".to_owned());
        }
        let webdav = sync.pointer("/params/webdav").and_then(Value::as_object);
        let configured_password = config
            .credential_fields
            .get("sync.params.webdav.password")
            .is_some_and(|state| {
                matches!(
                    *state,
                    CredentialAvailability::Configured | CredentialAvailability::Available
                )
            });
        let configuration_complete = webdav.is_some_and(|webdav| {
            webdav
                .get("serverUrl")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
                && webdav
                    .get("username")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
                && configured_password
                && !provider.is_empty()
        });
        let enabled = requested_enabled && configuration_complete;

        let previous = self.scheduler.lock().await.take();
        if let Some(previous) = previous {
            previous.cancel.cancel();
        }

        let mut status = self.status.lock().await;
        status.is_running = enabled;
        status.next_sync_time = if enabled {
            now_ms().saturating_add(interval)
        } else {
            0
        };
        let next_sync_time = status.next_sync_time;
        drop(status);
        if enabled {
            let cancel = CancellationToken::new();
            let task_cancel = cancel.clone();
            let service = self.clone();
            let period = Duration::from_millis(interval as u64);
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = task_cancel.cancelled() => break,
                        _ = sleep(period) => {}
                    }
                    if task_cancel.is_cancelled() {
                        break;
                    }
                    let mut status = service.status.lock().await;
                    if !status.is_running || task_cancel.is_cancelled() {
                        break;
                    }
                    status.next_sync_time = now_ms().saturating_add(interval);
                    drop(status);
                    let _ = service
                        .run_once(RunKind::Scheduled, None, None, Some(task_cancel.clone()))
                        .await;
                }
            });
            *self.scheduler.lock().await = Some(Scheduler { cancel });
            (self.events)(
                "sync-status-updated",
                json!({"type":"sync-scheduled-started","interval":interval,"nextSyncTime":next_sync_time}),
            );
        } else {
            (self.events)(
                "sync-status-updated",
                json!({
                    "type":"sync-scheduled-stopped",
                    "reason": if requested_enabled { "configuration-incomplete" } else { "disabled" },
                }),
            );
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.shutting_down.cancel();
        let _configure = self.configure_gate.lock().await;
        if let Some(scheduler) = self.scheduler.lock().await.take() {
            scheduler.cancel.cancel();
        }
        {
            let mut status = self.status.lock().await;
            status.is_running = false;
            status.next_sync_time = 0;
        }
        drop(_configure);
        let _operation = self.operation_gate.lock().await;
    }

    pub async fn test_connection(&self, candidate: Option<Value>) -> SyncTestResult {
        let result = self.test_connection_inner(candidate.as_ref()).await;
        let timestamp = now_ms();
        match &result {
            Ok(()) => (self.events)(
                "sync-status-updated",
                json!({"type":"connection-test-success","timestamp":timestamp}),
            ),
            Err(error) => (self.events)(
                "sync-status-updated",
                json!({"type":"connection-test-error","timestamp":timestamp,"error":error}),
            ),
        }
        SyncTestResult {
            success: result.is_ok(),
            message: result.err().unwrap_or_else(|| "连接测试成功".to_owned()),
        }
    }

    pub async fn manual_sync(&self) -> SyncRunResult {
        self.run_once(RunKind::Manual, None, None, None).await
    }

    pub async fn manual_sync_with_config(&self, config: Value) -> SyncRunResult {
        self.run_once(RunKind::Manual, None, Some(config), None)
            .await
    }

    pub async fn status(&self) -> SyncStatus {
        self.status.lock().await.clone()
    }

    async fn test_connection_inner(&self, candidate: Option<&Value>) -> Result<(), String> {
        let _guard = self.operation_lock().await?;
        let token = self.shutting_down.child_token();
        let (_, mut target) = self.resolve_operation(candidate, None).await?;
        let result = target.test_conditionals(&token).await;
        let cleanup = target.cleanup_created(&token).await;
        match result {
            Ok(()) => {
                cleanup?;
                self.set_conditionals_supported(&target.target_key).await?;
                Ok(())
            }
            Err(error) => {
                self.set_conditionals_unsupported(&target.target_key)
                    .await?;
                match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => {
                        Err(format!("{error}; 临时资源清理失败: {cleanup_error}"))
                    }
                }
            }
        }
    }

    async fn operation_lock(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
        tokio::select! {
            _ = self.shutting_down.cancelled() => Err("同步服务正在关闭".to_owned()),
            guard = self.operation_gate.lock() => Ok(guard),
        }
    }

    async fn run_once(
        &self,
        kind: RunKind,
        requested_direction: Option<SyncDirection>,
        candidate: Option<Value>,
        operation_cancel: Option<CancellationToken>,
    ) -> SyncRunResult {
        let guard = match self.operation_lock().await {
            Ok(guard) => guard,
            Err(error) => return SyncRunResult::failure(error),
        };
        {
            self.status.lock().await.sync_status = SyncState::Syncing;
        }
        let manual = matches!(kind, RunKind::Manual);
        let token = operation_cancel
            .map(|cancel| cancel.child_token())
            .unwrap_or_else(|| self.shutting_down.child_token());
        let prepared: Result<Option<(ResolvedOperation, WebDavTarget)>, String> = async {
            if token.is_cancelled() {
                return Err("同步已取消".to_owned());
            }
            if matches!(kind, RunKind::Scheduled) {
                let snapshot = self.config.snapshot().await?;
                if snapshot.sync.get("enabled").and_then(Value::as_bool) != Some(true) {
                    return Ok(None);
                }
            }
            self.resolve_operation(candidate.as_ref(), requested_direction)
                .await
                .map(Some)
        }
        .await;
        let suppress_repeated_schedule = match &prepared {
            Ok(Some((_, target))) if !manual => {
                match self
                    .storage
                    .read_meta(&conflict_meta_key(&target.target_key))
                    .await
                {
                    Ok(Some(Value::String(_))) => true,
                    Ok(_) => false,
                    Err(error) => {
                        tracing::warn!(%error, "unable to read scheduled sync conflict state");
                        false
                    }
                }
            }
            _ => false,
        };
        if !suppress_repeated_schedule {
            (self.events)(
                "sync-status-updated",
                json!({
                    "type": if manual { "manual-sync-started" } else { "sync-started" },
                    "timestamp": now_ms(),
                }),
            );
        }

        let result = match prepared {
            Ok(None) => SyncRunResult::success("同步已关闭"),
            Ok(Some((operation, mut target))) => match self
                .run_core(kind, requested_direction, &operation, &mut target, &token)
                .await
            {
                Ok(result) => result,
                Err(error) => SyncRunResult::failure(error),
            },
            Err(error) => SyncRunResult::failure(error),
        };
        let timestamp = now_ms();
        {
            let mut status = self.status.lock().await;
            status.last_sync_time = timestamp;
            status.sync_status = if result.success {
                SyncState::Success
            } else {
                SyncState::Error
            };
        }
        let event_type = match (manual, result.success) {
            (true, true) => "manual-sync-completed",
            (true, false) => "manual-sync-failed",
            (false, true) => "sync-completed",
            (false, false) => "sync-failed",
        };
        let next_sync_time = self.status.lock().await.next_sync_time;
        if suppress_repeated_schedule && result.already_reported_conflict {
            (self.events)(
                "sync-status-updated",
                json!({"type":"sync-conflict-repeat","timestamp":timestamp}),
            );
        } else {
            (self.events)(
                "sync-status-updated",
                json!({
                    "type":event_type,
                    "timestamp":timestamp,
                    "nextSyncTime":next_sync_time,
                    "result":result.clone(),
                }),
            );
        }
        drop(guard);
        result
    }

    async fn run_core(
        &self,
        kind: RunKind,
        requested_direction: Option<SyncDirection>,
        operation: &ResolvedOperation,
        target: &mut WebDavTarget,
        cancel: &CancellationToken,
    ) -> Result<SyncRunResult, String> {
        if cancel.is_cancelled() {
            return Err("同步已取消".to_owned());
        }
        let direction = requested_direction.unwrap_or(operation.direction);
        let require_conditionals = match kind {
            RunKind::Scheduled => direction != SyncDirection::Download,
            RunKind::Manual => direction == SyncDirection::Bidirectional,
        };
        if require_conditionals {
            match self.conditionals_supported(&target.target_key).await? {
                Some(true) => {}
                Some(false) => {
                    return Err(
                        "WebDAV 服务未通过条件写入检查，请手动测试连接后重试自动同步".to_owned(),
                    );
                }
                None => {
                    let test = target.test_conditionals(cancel).await;
                    let cleanup = target.cleanup_created(cancel).await;
                    match (test, cleanup) {
                        (Ok(()), Ok(())) => {}
                        (Err(error), Ok(())) => {
                            self.set_conditionals_unsupported(&target.target_key)
                                .await?;
                            return Err(error);
                        }
                        (Ok(()), Err(error)) => return Err(error),
                        (Err(error), Err(cleanup_error)) => {
                            self.set_conditionals_unsupported(&target.target_key)
                                .await?;
                            return Err(format!("{error}; 临时资源清理失败: {cleanup_error}"));
                        }
                    }
                    self.set_conditionals_supported(&target.target_key).await?;
                }
            }
        }
        target.ensure_collection(cancel).await?;
        let remote = target.read_document(cancel).await?;
        let baseline = self.read_baseline(&target.target_key).await?;

        if matches!(kind, RunKind::Manual) {
            return match direction {
                SyncDirection::Upload => {
                    self.upload_local(target, operation, remote, baseline, true, cancel)
                        .await
                }
                SyncDirection::Download => {
                    self.download_remote(target, operation, remote, cancel)
                        .await
                }
                SyncDirection::Bidirectional => {
                    self.reconcile(target, operation, remote, baseline, direction, cancel)
                        .await
                }
            };
        }

        self.reconcile(target, operation, remote, baseline, direction, cancel)
            .await
    }

    async fn reconcile(
        &self,
        target: &mut WebDavTarget,
        operation: &ResolvedOperation,
        remote: Option<RemoteDocument>,
        baseline: Option<SyncBaseline>,
        direction: SyncDirection,
        cancel: &CancellationToken,
    ) -> Result<SyncRunResult, String> {
        let local_hash = &operation.payload_sha256;
        let Some(baseline) = baseline else {
            return match remote {
                None if direction != SyncDirection::Download => {
                    self.upload_local(target, operation, None, None, false, cancel)
                        .await
                }
                None => Err("远端配置不存在，无法按下载方向同步".to_owned()),
                Some(remote) if remote.envelope.payload_sha256 == *local_hash => {
                    self.save_baseline(&target.target_key, &remote).await?;
                    self.clear_pending_upload(&target.target_key).await?;
                    self.clear_conflict(&target.target_key).await?;
                    Ok(SyncRunResult::success("本机与远端配置相同，已建立同步基线"))
                }
                Some(remote) => {
                    let already_reported = self
                        .record_conflict(
                            &target.target_key,
                            &operation.payload,
                            Some(&remote.envelope),
                            None,
                        )
                        .await?;
                    Ok(conflict_result(
                        "首次连接发现不同配置，双方内容已保留",
                        already_reported,
                    ))
                }
            };
        };

        let Some(remote) = remote else {
            let already_reported = self
                .record_conflict(
                    &target.target_key,
                    &operation.payload,
                    None,
                    Some(&baseline),
                )
                .await?;
            return Ok(conflict_result(
                "远端配置已删除，本机内容已保留",
                already_reported,
            ));
        };
        let remote_hash = &remote.envelope.payload_sha256;
        if local_hash == remote_hash {
            self.save_baseline(&target.target_key, &remote).await?;
            self.clear_pending_upload(&target.target_key).await?;
            self.clear_conflict(&target.target_key).await?;
            return Ok(SyncRunResult::success("本机与远端配置相同"));
        }

        let local_changed = local_hash != &baseline.payload_sha256;
        let remote_changed = remote_hash != &baseline.payload_sha256;
        if local_changed && remote_changed {
            let already_reported = self
                .record_conflict(
                    &target.target_key,
                    &operation.payload,
                    Some(&remote.envelope),
                    Some(&baseline),
                )
                .await?;
            return Ok(conflict_result(
                "本机和远端配置均已变化，未覆盖任一方",
                already_reported,
            ));
        }

        if local_changed {
            if direction == SyncDirection::Download {
                let already_reported = self
                    .record_conflict(
                        &target.target_key,
                        &operation.payload,
                        Some(&remote.envelope),
                        Some(&baseline),
                    )
                    .await?;
                return Ok(conflict_result(
                    "本机配置已变化，当前下载方向不会覆盖本机内容",
                    already_reported,
                ));
            }
            return self
                .upload_local(
                    target,
                    operation,
                    Some(remote),
                    Some(baseline),
                    false,
                    cancel,
                )
                .await;
        }

        if remote_changed {
            if direction == SyncDirection::Upload {
                let already_reported = self
                    .record_conflict(
                        &target.target_key,
                        &operation.payload,
                        Some(&remote.envelope),
                        Some(&baseline),
                    )
                    .await?;
                return Ok(conflict_result(
                    "远端配置已变化，当前上传方向不会覆盖远端内容",
                    already_reported,
                ));
            }
            return self
                .download_remote(target, operation, Some(remote), cancel)
                .await;
        }

        self.save_baseline(&target.target_key, &remote).await?;
        Ok(SyncRunResult::success("配置没有变化"))
    }

    async fn upload_local(
        &self,
        target: &mut WebDavTarget,
        operation: &ResolvedOperation,
        remote: Option<RemoteDocument>,
        baseline: Option<SyncBaseline>,
        manual: bool,
        cancel: &CancellationToken,
    ) -> Result<SyncRunResult, String> {
        self.ensure_operation_current(operation).await?;
        if let Some(remote) = remote.as_ref() {
            if remote.envelope.payload_sha256 == operation.payload_sha256 {
                self.save_baseline(&target.target_key, remote).await?;
                self.clear_pending_upload(&target.target_key).await?;
                self.clear_conflict(&target.target_key).await?;
                return Ok(SyncRunResult::success("本机与远端配置相同"));
            }
        }
        let parent = remote
            .as_ref()
            .map(|document| document.envelope.revision.clone());
        let envelope = self
            .pending_envelope(&target.target_key, &operation.payload, parent.clone())
            .await?;
        let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err("同步配置超过远端文档大小限制".to_owned());
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        match remote.as_ref() {
            Some(remote) => {
                let baseline_etag = baseline
                    .as_ref()
                    .filter(|baseline| {
                        baseline.revision == remote.envelope.revision
                            && baseline.payload_sha256 == remote.envelope.payload_sha256
                    })
                    .and_then(|baseline| baseline.etag.as_deref())
                    .filter(|etag| is_strong_etag(etag));
                if let Some(etag) = remote
                    .etag
                    .as_deref()
                    .filter(|etag| is_strong_etag(etag))
                    .or(baseline_etag)
                {
                    headers.insert(
                        IF_MATCH,
                        HeaderValue::from_str(etag).map_err(|_| "远端 ETag 无效")?,
                    );
                } else if !manual {
                    return Err("远端没有强 ETag，自动上传无法安全覆盖".to_owned());
                }
            }
            None => {
                headers.insert(IF_NONE_MATCH, HeaderValue::from_static("*"));
            }
        }
        let put_result = target
            .send(Method::PUT, &target.document, headers, Some(bytes), cancel)
            .await;
        let (status_error, precondition_failed) = match put_result {
            Ok(response) if response.status().is_success() => (None, false),
            Ok(response) if response.status() == StatusCode::PRECONDITION_FAILED => {
                (Some("远端配置在同步期间发生变化".to_owned()), true)
            }
            Ok(response) => (
                Some(format!("WebDAV 上传失败，HTTP {}", response.status())),
                false,
            ),
            Err(error) => (Some(error), false),
        };

        let actual = target.read_document(cancel).await;
        match actual {
            Ok(Some(actual)) if actual.envelope.revision == envelope.revision => {
                self.save_baseline(&target.target_key, &actual).await?;
                self.clear_pending_upload(&target.target_key).await?;
                self.clear_conflict(&target.target_key).await?;
                let mut result = SyncRunResult::success("配置已上传");
                result.uploaded_files.push("config.json".to_owned());
                Ok(result)
            }
            Ok(actual) if precondition_failed => {
                self.clear_pending_upload(&target.target_key).await?;
                let already_reported = self
                    .record_conflict(
                        &target.target_key,
                        &operation.payload,
                        actual.as_ref().map(|document| &document.envelope),
                        baseline.as_ref(),
                    )
                    .await?;
                Ok(conflict_result(
                    status_error.as_deref().unwrap_or("远端提交未确认"),
                    already_reported,
                ))
            }
            Ok(actual) if status_error.is_some() => {
                let unchanged = match (baseline.as_ref(), actual.as_ref()) {
                    (Some(baseline), Some(remote)) => {
                        baseline.revision == remote.envelope.revision
                            && baseline.payload_sha256 == remote.envelope.payload_sha256
                    }
                    (None, None) => true,
                    _ => false,
                };
                if unchanged {
                    return Err(status_error.unwrap_or_else(|| "WebDAV 上传失败".to_owned()));
                }
                self.clear_pending_upload(&target.target_key).await?;
                let already_reported = self
                    .record_conflict(
                        &target.target_key,
                        &operation.payload,
                        actual.as_ref().map(|document| &document.envelope),
                        baseline.as_ref(),
                    )
                    .await?;
                Ok(conflict_result(
                    "上传期间远端配置发生变化，双方内容已保留",
                    already_reported,
                ))
            }
            Ok(Some(actual)) => {
                self.clear_pending_upload(&target.target_key).await?;
                let already_reported = self
                    .record_conflict(
                        &target.target_key,
                        &operation.payload,
                        Some(&actual.envelope),
                        baseline.as_ref(),
                    )
                    .await?;
                Ok(conflict_result(
                    "上传后远端版本已变化，双方内容已保留",
                    already_reported,
                ))
            }
            Ok(None) => Err("远端未确认本次配置提交，保留待核实版本".to_owned()),
            Err(read_error) => Err(format!("上传结果不确定，读取远端核实失败: {read_error}")),
        }
    }

    async fn ensure_operation_current(&self, operation: &ResolvedOperation) -> Result<(), String> {
        let current = self.config.snapshot().await?;
        let mut fields = Vec::new();
        for root in ["settings", "sync"] {
            if let Some(projected) = operation.payload.get(root).and_then(Value::as_object) {
                fields.extend(projected.keys().map(|field| format!("{root}.{field}")));
            }
        }
        fields.push("sync.params".to_owned());
        if fields.into_iter().any(|field| {
            operation
                .snapshot
                .versions
                .get(&field)
                .copied()
                .unwrap_or(0)
                != current.versions.get(&field).copied().unwrap_or(0)
        }) {
            return Err("同步期间配置发生变化，请重新发起操作".to_owned());
        }
        Ok(())
    }

    async fn download_remote(
        &self,
        target: &WebDavTarget,
        operation: &ResolvedOperation,
        remote: Option<RemoteDocument>,
        _cancel: &CancellationToken,
    ) -> Result<SyncRunResult, String> {
        let remote = remote.ok_or_else(|| "远端配置不存在，无法下载".to_owned())?;
        let before = operation.snapshot.clone();
        match self
            .config
            .apply_remote_projection(before.versions.clone(), remote.envelope.payload.clone())
            .await
        {
            Ok(after) => {
                let mut result = SyncRunResult::success("配置已下载并应用");
                result.downloaded_files.push("config.json".to_owned());
                if before.settings != after.settings || before.sync != after.sync {
                    (self.events)(
                        "sync-requires-restart",
                        json!({
                            "downloadedFiles": result.downloaded_files,
                            "message": "已下载配置，需要重新启动应用以应用更改",
                        }),
                    );
                }
                self.save_baseline(&target.target_key, &remote).await?;
                self.clear_pending_upload(&target.target_key).await?;
                self.clear_conflict(&target.target_key).await?;
                Ok(result)
            }
            Err(error) => Err(format!("远端配置未应用，当前配置保持不变: {error}")),
        }
    }

    async fn resolve_operation(
        &self,
        candidate: Option<&Value>,
        requested_direction: Option<SyncDirection>,
    ) -> Result<(ResolvedOperation, WebDavTarget), String> {
        let before = self.config.snapshot().await?;
        let provider = before
            .sync
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let config_value = if let Some(candidate) = candidate {
            let (provider_type, provider_config) = candidate_sync_config(candidate)?;
            if provider_type != "webdav" {
                return Err("当前只支持 WebDAV 同步".to_owned());
            }
            self.config
                .runtime_webdav_test_config(provider_config)
                .await?
        } else {
            if provider != "webdav" {
                return Err("当前同步提供者不是 WebDAV".to_owned());
            }
            self.config.runtime_webdav_config().await?
        };
        let after = self.config.snapshot().await?;
        if [
            "sync.enabled",
            "sync.provider",
            "sync.interval",
            "sync.direction",
            "sync.params",
        ]
        .iter()
        .any(|key| before.versions.get(*key) != after.versions.get(*key))
        {
            return Err("同步期间配置发生变化，请重新发起操作".to_owned());
        }
        let stored_direction = after
            .sync
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("bidirectional");
        let candidate_direction = config_value
            .get("syncDirection")
            .and_then(Value::as_str)
            .map(SyncDirection::parse)
            .transpose()?;
        let direction = requested_direction
            .or(candidate_direction)
            .unwrap_or(SyncDirection::parse(stored_direction)?);
        let webdav = parse_webdav_config(&config_value)?;
        let target = WebDavTarget::new(webdav)?;
        let payload = remote_projection(&after.settings, &after.sync)?;
        validate_remote_projection_value(&payload)?;
        let bytes = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
        let payload_sha256 = data_format::sha256_hex(&bytes);
        Ok((
            ResolvedOperation {
                snapshot: after,
                payload,
                payload_sha256,
                direction,
            },
            target,
        ))
    }

    async fn read_baseline(&self, target_key: &str) -> Result<Option<SyncBaseline>, String> {
        let Some(value) = self
            .storage
            .read_meta(&baseline_meta_key(target_key))
            .await?
        else {
            return Ok(None);
        };
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| format!("本机 WebDAV 同步基线无效: {error}"))
    }

    async fn conditionals_supported(&self, target_key: &str) -> Result<Option<bool>, String> {
        Ok(self
            .storage
            .read_meta(&conditionals_meta_key(target_key))
            .await?
            .and_then(|value| value.as_bool()))
    }

    async fn set_conditionals_supported(&self, target_key: &str) -> Result<(), String> {
        compare_meta(
            self.storage.as_ref(),
            &conditionals_meta_key(target_key),
            &Value::Bool(true),
        )
        .await
    }

    async fn set_conditionals_unsupported(&self, target_key: &str) -> Result<(), String> {
        compare_meta(
            self.storage.as_ref(),
            &conditionals_meta_key(target_key),
            &Value::Bool(false),
        )
        .await
    }

    async fn save_baseline(&self, target_key: &str, remote: &RemoteDocument) -> Result<(), String> {
        let next = serde_json::to_value(SyncBaseline {
            revision: remote.envelope.revision.clone(),
            payload_sha256: remote.envelope.payload_sha256.clone(),
            etag: remote.etag.clone().filter(|value| is_strong_etag(value)),
        })
        .map_err(|error| error.to_string())?;
        compare_meta(self.storage.as_ref(), &baseline_meta_key(target_key), &next).await
    }

    async fn pending_envelope(
        &self,
        target_key: &str,
        payload: &Value,
        parent_revision: Option<String>,
    ) -> Result<RemoteEnvelope, String> {
        let key = pending_meta_key(target_key);
        if let Some(value) = self.storage.read_meta(&key).await? {
            if let Ok(pending) = serde_json::from_value::<PendingUpload>(value) {
                if pending.envelope.payload == *payload && pending.target_parent == parent_revision
                {
                    return Ok(pending.envelope);
                }
            }
        }
        let pending = PendingUpload {
            envelope: RemoteEnvelope::new(payload.clone(), parent_revision.clone())?,
            target_parent: parent_revision,
        };
        let value = serde_json::to_value(&pending).map_err(|error| error.to_string())?;
        compare_meta(self.storage.as_ref(), &key, &value).await?;
        Ok(pending.envelope)
    }

    async fn clear_pending_upload(&self, target_key: &str) -> Result<(), String> {
        clear_meta(self.storage.as_ref(), &pending_meta_key(target_key)).await
    }

    async fn record_conflict(
        &self,
        target_key: &str,
        local: &Value,
        remote: Option<&RemoteEnvelope>,
        baseline: Option<&SyncBaseline>,
    ) -> Result<bool, String> {
        let local_hash =
            data_format::sha256_hex(&serde_json::to_vec(local).map_err(|error| error.to_string())?);
        let remote_revision = remote
            .map(|value| value.revision.as_str())
            .unwrap_or("missing");
        let remote_hash = remote
            .map(|value| value.payload_sha256.as_str())
            .unwrap_or("missing");
        let baseline_revision = baseline
            .map(|value| value.revision.as_str())
            .unwrap_or("none");
        let conflict_id = data_format::sha256_hex(
            format!("{local_hash}\0{remote_revision}\0{remote_hash}\0{baseline_revision}")
                .as_bytes(),
        );
        let meta_key = conflict_meta_key(target_key);
        let previous = self.storage.read_meta(&meta_key).await?;
        let already_reported = previous.as_ref().and_then(Value::as_str) == Some(&conflict_id);
        let conflict_dir = self.app_data_dir.join("sync-conflicts");
        tokio::fs::create_dir_all(&conflict_dir)
            .await
            .map_err(|error| format!("无法创建同步冲突目录: {error}"))?;
        let path = conflict_dir.join(format!("{conflict_id}.json"));
        if tokio::fs::metadata(&path).await.is_err() {
            let record = json!({
                "id": conflict_id,
                "createdAt": now_ms(),
                "local": local,
                "remote": remote,
                "baseline": baseline,
            });
            let bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
            write_atomic(&path, &bytes).await?;
        }
        let next = Value::String(conflict_id);
        if !already_reported {
            let stored = self
                .storage
                .compare_and_set_meta(&meta_key, previous.as_ref(), &next)
                .await?;
            if !stored {
                return Err("同步冲突状态并发变化，请重试".to_owned());
            }
        }
        Ok(already_reported)
    }

    async fn clear_conflict(&self, target_key: &str) -> Result<(), String> {
        clear_meta(self.storage.as_ref(), &conflict_meta_key(target_key)).await
    }
}

fn candidate_sync_config<'a>(candidate: &'a Value) -> Result<(&'a str, &'a Value), String> {
    let object = candidate
        .as_object()
        .ok_or_else(|| "同步配置必须是对象".to_owned())?;
    let provider = object
        .get("providerType")
        .and_then(Value::as_str)
        .ok_or_else(|| "同步配置缺少 providerType".to_owned())?;
    let config = object
        .get("providerConfig")
        .ok_or_else(|| "同步配置缺少 providerConfig".to_owned())?;
    Ok((provider, config))
}

fn baseline_meta_key(target_key: &str) -> String {
    format!("sync.webdav.baseline.v1.{target_key}")
}

fn pending_meta_key(target_key: &str) -> String {
    format!("sync.webdav.pending.v1.{target_key}")
}

fn conflict_meta_key(target_key: &str) -> String {
    format!("sync.webdav.conflict.v1.{target_key}")
}

fn conditionals_meta_key(target_key: &str) -> String {
    format!("sync.webdav.conditionals.v1.{target_key}")
}

async fn compare_meta(storage: &Storage, key: &str, next: &Value) -> Result<(), String> {
    for _ in 0..8 {
        let previous = storage.read_meta(key).await?;
        if storage
            .compare_and_set_meta(key, previous.as_ref(), next)
            .await?
        {
            return Ok(());
        }
    }
    Err(format!("同步元数据并发更新失败: {key}"))
}

async fn clear_meta(storage: &Storage, key: &str) -> Result<(), String> {
    for _ in 0..8 {
        let previous = storage.read_meta(key).await?;
        let Some(previous) = previous else {
            return Ok(());
        };
        if storage
            .compare_and_set_meta(key, Some(&previous), &Value::Null)
            .await?
        {
            return Ok(());
        }
    }
    Err(format!("同步元数据清理并发更新失败: {key}"))
}

async fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "冲突文件路径无效".to_owned())?;
    let temporary = parent.join(format!(".{}.{}.tmp", Uuid::new_v4(), std::process::id()));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
            .map_err(|error| format!("无法创建冲突临时文件: {error}"))?;
        file.write_all(bytes)
            .await
            .map_err(|error| format!("无法写入冲突文件: {error}"))?;
        file.sync_all()
            .await
            .map_err(|error| format!("无法同步冲突文件: {error}"))?;
        drop(file);
        tokio::fs::rename(&temporary, path)
            .await
            .map_err(|error| format!("无法保存冲突文件: {error}"))?;
        Ok::<_, String>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

fn conflict_result(message: &str, already_reported: bool) -> SyncRunResult {
    let mut result = SyncRunResult::failure(message);
    result.conflict = Some(message.to_owned());
    result.already_reported_conflict = already_reported;
    result
}
