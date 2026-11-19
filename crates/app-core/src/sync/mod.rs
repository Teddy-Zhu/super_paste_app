//! Owned WebDAV operations. Settings and credentials are captured under the
//! configuration gate; network bytes never pass through a renderer DTO.
mod document;
mod webdav;
mod reconcile;
mod conflict;

use std::{collections::BTreeMap, panic::AssertUnwindSafe, path::PathBuf, sync::{Arc, Mutex}, time::Duration};
use futures_util::FutureExt;
use reqwest::Client;
use super_paste_format::{ConfigField, CredentialId, SyncProjection, SyncState, SyncStatus, SyncOutcome};
use tokio::{runtime::Handle, sync::{Semaphore, watch}, time::Instant};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use crate::{Error, Result, configuration::{ConfigurationEffects, ConfigurationService, ConfigurationState}, events::{AppEvent, EventBus}, storage::Storage};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunKind { Test, Manual, Scheduled }
struct Active { versions: BTreeMap<ConfigField, u64>, cancel: CancellationToken }
pub struct SyncService<E: ConfigurationEffects + 'static> {
    storage: Arc<Storage>, configuration: Arc<ConfigurationService>, effects: Arc<E>, events: EventBus,
    client: Client, conflicts: PathBuf, runtime: Handle, root: CancellationToken, tasks: TaskTracker,
    admission: Mutex<bool>, active: Mutex<Option<Active>>, operation: Arc<Semaphore>, status: watch::Sender<Arc<SyncStatus>>,
}
impl<E: ConfigurationEffects + 'static> SyncService<E> {
    pub async fn new(storage: Arc<Storage>, configuration: Arc<ConfigurationService>, effects: Arc<E>, events: EventBus, conflicts: PathBuf, runtime: Handle) -> Result<Arc<Self>> {
        let client = Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(15)).build()
            .map_err(|error| Error::Platform(error.without_url().to_string().into()))?;
        let status = SyncStatus { last_sync_time: storage.sync_last_success().await?, ..SyncStatus::default() };
        let (status, _) = watch::channel(Arc::new(status));
        let service = Arc::new(Self { storage, configuration, effects, events, client, conflicts, runtime, root: CancellationToken::new(),
            tasks: TaskTracker::new(), admission: Mutex::new(true), active: Mutex::new(None), operation: Arc::new(Semaphore::new(1)), status });
        let scheduler = service.clone();
        service.tasks.spawn_on(async move { scheduler.schedule().await }, &service.runtime);
        Ok(service)
    }
    pub fn snapshot(&self) -> Arc<SyncStatus> { self.status.borrow().clone() }
    pub fn subscribe(&self) -> watch::Receiver<Arc<SyncStatus>> { self.status.subscribe() }
    pub async fn test_connection(self: &Arc<Self>, expected: ConfigurationState, caller: CancellationToken) -> Result<()> {
        self.wait(self.accept(RunKind::Test, expected, caller)?).await.map(|_| ())
    }
    pub async fn synchronize(self: &Arc<Self>, expected: ConfigurationState, caller: CancellationToken) -> Result<Arc<SyncOutcome>> {
        self.wait(self.accept(RunKind::Manual, expected, caller)?).await
    }
    async fn wait(&self, mut receiver: watch::Receiver<Option<Result<Arc<SyncOutcome>>>>) -> Result<Arc<SyncOutcome>> {
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() { return result; }
            // Cancellation reaches the owned operation. Its final result must
            // remain authoritative once configuration has accepted a commit.
            receiver.changed().await.map_err(|_| Error::Task("同步任务意外停止".into()))?;
        }
    }
    fn accept(self: &Arc<Self>, kind: RunKind, expected: ConfigurationState, caller: CancellationToken) -> Result<watch::Receiver<Option<Result<Arc<SyncOutcome>>>>> {
        if caller.is_cancelled() { return Err(Error::Cancelled); }
        let accepting = self.admission.lock().map_err(|_| lock_error())?;
        if !*accepting { return Err(Error::Cancelled); }
        let permit = self.operation.clone().try_acquire_owned().map_err(|_| Error::ResourceLimit("同步操作正在进行".into()))?;
        let cancel = self.root.child_token();
        let versions = scope(kind).into_iter().map(|field| (field, version(&expected, field))).collect();
        *self.active.lock().map_err(|_| lock_error())? = Some(Active { versions, cancel: cancel.clone() });
        if kind != RunKind::Test { self.update_status(|status| { status.state = SyncState::Syncing; status.error = None; }); }
        let (sender, receiver) = watch::channel(None);
        let service = self.clone();
        self.tasks.spawn_on(async move {
            let _permit = permit;
            let work = service.execute(kind, &expected, &cancel);
            tokio::pin!(work);
            // Observe cancellation before polling more I/O, but do not drop a
            // configuration commit already accepted by its owning service.
            let result = AssertUnwindSafe(async {
                tokio::select! { biased; _ = caller.cancelled() => { cancel.cancel(); work.await }, result = &mut work => result }
            }).catch_unwind().await.unwrap_or_else(|_| Err(Error::Task("同步任务发生异常".into()))).map(Arc::new);
            if let Ok(mut active) = service.active.lock() { *active = None; }
            service.finish(kind, &result);
            sender.send_replace(Some(result));
        }, &self.runtime);
        Ok(receiver)
    }
    async fn execute(&self, kind: RunKind, expected: &ConfigurationState, cancel: &CancellationToken) -> Result<SyncOutcome> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        if kind == RunKind::Scheduled && !schedule_enabled(expected) { return Err(Error::Cancelled); }
        let credentials = self.configuration.guarded_credential(CredentialId::WebDavPassword, expected, &scope(kind)).await;
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let (configuration, password) = credentials?;
        if configuration.sync.provider != "webdav" { return Err(Error::Invalid("请选择 WebDAV 同步服务".into())); }
        let target = webdav::Target::new(self.client.clone(), &configuration.sync.webdav, password)?;
        if kind == RunKind::Test {
            let test = target.test_conditionals(cancel).await;
            if let Some(supported) = test.supported { self.storage.sync_conditionals(target.key, supported).await?; }
            return test.result.map(|_| outcome("连接测试成功"));
        }
        self.reconcile(&target, kind, expected, configuration.sync.direction, cancel).await
    }
    fn finish(&self, kind: RunKind, result: &Result<Arc<SyncOutcome>>) {
        if let Err(error) = result { if !matches!(error, Error::Cancelled) { tracing::warn!(%error, connection_test = kind == RunKind::Test, "WebDAV 操作失败"); } }
        if kind == RunKind::Test { return; }
        self.update_status(|status| match result {
            Ok(outcome) => {
                status.state = if outcome.conflict.is_some() { SyncState::Error } else { SyncState::Success };
                status.error = outcome.conflict.clone();
                status.restart_required |= outcome.restart_required;
            },
            Err(Error::Cancelled) => { status.state = SyncState::Idle; status.error = None; },
            Err(error) => { status.state = SyncState::Error; status.error = Some(error.to_string()); },
        });
        if !matches!(result, Err(Error::Cancelled)) { self.events.publish(AppEvent::SyncFinished { manual: kind == RunKind::Manual, result: result.clone() }); }
    }
    fn update_status(&self, update: impl FnOnce(&mut SyncStatus)) {
        self.status.send_modify(|value| update(Arc::make_mut(value)));
        self.events.publish(AppEvent::SyncStatusChanged(self.snapshot()));
    }
    fn ensure_current(&self, expected: &ConfigurationState, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let current = self.configuration.snapshot();
        if scope(RunKind::Manual).into_iter().any(|field| version(&current, field) != version(expected, field)) {
            return Err(Error::Conflict("同步配置或本地设置已变化，请重新同步".into()));
        }
        Ok(())
    }
    async fn schedule(self: Arc<Self>) {
        let mut configuration = self.configuration.subscribe();
        let mut previous = None;
        let mut deadline = None;
        loop {
            let state = configuration.borrow_and_update().clone();
            match self.active.lock() {
                Ok(active) => { if let Some(active) = active.as_ref() { if active.versions.iter().any(|(field, expected)| version(&state, *field) != *expected) { active.cancel.cancel(); } } },
                Err(_) => { self.update_status(|status| { status.state = SyncState::Error; status.error = Some(lock_error().to_string()); }); return; }
            }
            let schedule = (state.snapshot.value.sync.clone(), state.credentials.get(&CredentialId::WebDavPassword).copied());
            if previous.as_ref() != Some(&schedule) {
                deadline = self.next_deadline(&state);
                previous = Some(schedule);
            }
            let timer = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } };
            tokio::select! {
                biased;
                _ = self.root.cancelled() => return,
                changed = configuration.changed() => if changed.is_err() { return; },
                _ = timer => {
                    let state = self.configuration.snapshot();
                    // Busy manual/test work owns the sole network slot. Skip
                    // this tick rather than queueing another identical job.
                    if let Err(error) = self.accept(RunKind::Scheduled, state.clone(), CancellationToken::new()) {
                        if !matches!(error, Error::ResourceLimit(_) | Error::Cancelled) { self.finish(RunKind::Scheduled, &Err(error)); }
                    }
                    deadline = self.next_deadline(&state);
                }
            }
        }
    }
    fn next_deadline(&self, state: &ConfigurationState) -> Option<Instant> {
        let enabled = schedule_enabled(state);
        let timing = if enabled { crate::now_ms().ok().zip(i64::try_from(state.snapshot.value.sync.interval).ok()).and_then(|(now, interval)| now.checked_add(interval))
            .zip(Instant::now().checked_add(Duration::from_millis(state.snapshot.value.sync.interval))) } else { None };
        self.update_status(|status| { status.is_running = enabled && timing.is_some(); status.next_sync_time = timing.map(|(timestamp, _)| timestamp).unwrap_or(0);
            if enabled && timing.is_none() { status.state = SyncState::Error; status.error = Some("同步间隔超出支持范围".into()); } });
        timing.map(|(_, deadline)| deadline)
    }
    pub async fn shutdown(&self) -> Result<()> {
        { let mut accepting = self.admission.lock().map_err(|_| lock_error())?; *accepting = false; self.operation.close(); self.tasks.close(); self.root.cancel(); }
        self.tasks.wait().await;
        Ok(())
    }
}
fn scope(kind: RunKind) -> Vec<ConfigField> {
    let mut fields = vec![ConfigField::WebDav, ConfigField::WebDavPassword, ConfigField::SyncProvider];
    if kind != RunKind::Test { fields.extend(SyncProjection::FIELDS); }
    fields.sort(); fields.dedup(); fields
}
fn version(state: &ConfigurationState, field: ConfigField) -> u64 { state.fields.get(&field).copied().unwrap_or(0) }
fn schedule_enabled(state: &ConfigurationState) -> bool {
    let sync = &state.snapshot.value.sync;
    sync.enabled && sync.provider == "webdav" && !sync.webdav.server_url.trim().is_empty() && !sync.webdav.username.trim().is_empty() && state.credentials.contains_key(&CredentialId::WebDavPassword)
}
fn outcome(message: &str) -> SyncOutcome { SyncOutcome { message: message.into(), uploaded: false, downloaded: false, conflict: None, repeated_conflict: false, restart_required: false } }
fn lock_error() -> Error { Error::Task("同步接入锁损坏".into()) }

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use crate::{configuration::{ConfigurationMutation, PreparedConfigurationEffects, RuntimeConfiguration},
        credentials::{CredentialBackend, CredentialChange, CredentialReference, CredentialVault, Secret, SecretChange}};
    use super_paste_format::{ConfigChange, WebDavConfig};

    struct Credentials { reads: AtomicUsize, delayed_failure: AtomicBool, entered: tokio::sync::Notify, release: tokio::sync::Notify }
    #[async_trait::async_trait]
    impl CredentialBackend for Credentials {
        async fn read(&self, _: CredentialReference) -> Result<Option<Arc<Secret>>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.delayed_failure.load(Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
                return Err(Error::CredentialUnavailable);
            }
            Ok(Some(Arc::new(Secret::new("fixture".into()))))
        }
        async fn write(&self, _: CredentialReference, _: Arc<Secret>) -> Result<()> { Ok(()) }
        async fn delete(&self, _: CredentialReference) -> Result<()> { Ok(()) }
    }
    struct Effects;
    impl ConfigurationEffects for Effects {
        async fn prepare(&self, _: RuntimeConfiguration, _: RuntimeConfiguration, _: std::collections::BTreeSet<ConfigField>) -> Result<impl PreparedConfigurationEffects + 'static> { Ok(Self) }
    }
    impl PreparedConfigurationEffects for Effects {
        async fn apply(&mut self) -> Result<()> { Ok(()) }
        async fn rollback(&mut self) -> Result<()> { Ok(()) }
    }
    struct CommitGate {
        entered: tokio::sync::Notify, release: tokio::sync::Notify,
        applied: AtomicUsize, rolled_back: AtomicUsize, fail: bool,
    }
    struct HeldEffects(Arc<CommitGate>);
    impl ConfigurationEffects for HeldEffects {
        async fn prepare(&self, _: RuntimeConfiguration, _: RuntimeConfiguration, _: std::collections::BTreeSet<ConfigField>) -> Result<impl PreparedConfigurationEffects + 'static> {
            Ok(Self(self.0.clone()))
        }
    }
    impl PreparedConfigurationEffects for HeldEffects {
        async fn apply(&mut self) -> Result<()> {
            self.0.applied.fetch_add(1, Ordering::SeqCst);
            self.0.entered.notify_one();
            self.0.release.notified().await;
            if self.0.fail { Err(Error::Platform("受控配置应用失败".into())) } else { Ok(()) }
        }
        async fn rollback(&mut self) -> Result<()> {
            self.0.rolled_back.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) { std::fs::remove_dir_all(&self.0).expect("remove isolated sync directory"); }
    }

    #[tokio::test]
    async fn cancellation_and_shutdown_drain_accepted_download_commits_and_rollbacks() {
        use axum::{Router, body::Body, extract::Request, http::{Response, StatusCode}};
        use super_paste_format::{ColorMode, SyncDirection};

        tokio::time::timeout(Duration::from_secs(10), async {
            for (stop_service, fail_commit) in [(false, false), (true, false), (false, true), (true, true)] {
                let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-sync-commit-{}", uuid::Uuid::new_v4())));
                std::fs::create_dir(&directory.0).expect("isolated sync directory");
                let storage = Storage::open(directory.0.join("history.sqlite3")).await.expect("database");
                let credentials = Arc::new(Credentials { reads: AtomicUsize::new(0), delayed_failure: AtomicBool::new(false), entered: tokio::sync::Notify::new(), release: tokio::sync::Notify::new() });
                let vault = CredentialVault::new(storage.clone(), credentials.clone());
                let events = EventBus::new(32).expect("events");
                let runtime = Handle::current();
                let configuration = ConfigurationService::open(storage.clone(), vault, events.clone(), runtime.clone()).await.expect("configuration");
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("local WebDAV listener");
                let initial = configuration.snapshot();
                let expected = configuration.commit(ConfigurationMutation {
                    client_id: uuid::Uuid::new_v4(), mutation_id: uuid::Uuid::new_v4(),
                    base_versions: [ConfigField::WebDav, ConfigField::SyncProvider, ConfigField::SyncDirection, ConfigField::WebDavPassword].into_iter().map(|field| (field, version(&initial, field))).collect(),
                    changes: vec![ConfigChange::SyncProvider("webdav".into()), ConfigChange::SyncDirection(SyncDirection::Download), ConfigChange::WebDav(WebDavConfig {
                        server_url: format!("http://{}/dav/", listener.local_addr().expect("WebDAV address")), username: "fixture".into(), ..Default::default()
                    })], credentials: vec![CredentialChange { id: CredentialId::WebDavPassword, change: SecretChange::Set(Arc::new(Secret::new("fixture".into()))) }],
                }, Arc::new(Effects)).await.expect("download configuration").state;
                let mut projection = SyncProjection::capture(&expected.snapshot.value);
                projection.settings.color_mode = ColorMode::Dark;
                let payload = document::Payload::new(projection).expect("remote payload");
                let remote_revision = document::Metadata::new(None).expect("remote revision");
                let body = document::Document::encode(remote_revision, &payload).expect("remote document");
                let gets = Arc::new(AtomicUsize::new(0)); let requests = gets.clone();
                let router = Router::new().fallback(move |request: Request| {
                    let body = body.clone(); let requests = requests.clone();
                    async move {
                        match request.method().as_str() {
                            "MKCOL" => Response::builder().status(StatusCode::CREATED).body(Body::empty()).expect("collection response"),
                            "GET" if request.uri().path().ends_with("/super_paste/v1/config.json") => {
                                requests.fetch_add(1, Ordering::SeqCst);
                                Response::builder().header("content-type", "application/json").header("etag", "\"remote-one\"").body(Body::from(body)).expect("remote settings response")
                            },
                            _ => Response::builder().status(StatusCode::METHOD_NOT_ALLOWED).body(Body::empty()).expect("reject unexpected write"),
                        }
                    }
                });
                let _server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move { axum::serve(listener, router).await.expect("WebDAV server"); }));
                let gate = Arc::new(CommitGate { entered: tokio::sync::Notify::new(), release: tokio::sync::Notify::new(), applied: AtomicUsize::new(0), rolled_back: AtomicUsize::new(0), fail: fail_commit });
                let service = SyncService::new(storage.clone(), configuration.clone(), Arc::new(HeldEffects(gate.clone())), events, directory.0.join("conflicts"), runtime).await.expect("sync service");
                let target = webdav::Target::new(service.client.clone(), &expected.snapshot.value.sync.webdav, Arc::new(Secret::new("fixture".into()))).expect("target identity");
                let caller = CancellationToken::new();
                let result = service.accept(RunKind::Manual, expected.clone(), caller.clone()).expect("accepted download");
                tokio::time::timeout(Duration::from_secs(2), gate.entered.notified()).await.expect("download reached an owned configuration commit");
                assert_eq!(gets.load(Ordering::SeqCst), 1);
                assert_eq!(gate.applied.load(Ordering::SeqCst), 1);
                assert_eq!(configuration.snapshot().snapshot.revision, expected.snapshot.revision, "the held commit has not published a result");
                let closing = service.shutdown(); tokio::pin!(closing);
                if stop_service {
                    assert!(futures_util::poll!(closing.as_mut()).is_pending(), "shutdown must drain the held configuration commit");
                    assert!(service.root.is_cancelled());
                } else { caller.cancel(); }
                let waiting = service.wait(result); tokio::pin!(waiting);
                assert!(futures_util::poll!(waiting.as_mut()).is_pending(), "an accepted configuration commit must not be reported as cancelled before it finishes");
                gate.release.notify_one();
                let result = tokio::time::timeout(Duration::from_secs(2), waiting).await.expect("real commit result");
                let saved = storage.configuration_state().await.expect("persisted configuration");
                assert_eq!(saved.snapshot.revision, configuration.snapshot().snapshot.revision);
                let sync = storage.sync_target(target.key).await.expect("persisted sync baseline");
                if fail_commit {
                    assert!(matches!(result, Err(Error::Platform(message)) if message.as_ref() == "受控配置应用失败"));
                    assert_eq!(saved.snapshot.value.settings.color_mode, expected.snapshot.value.settings.color_mode);
                    assert_eq!(saved.snapshot.revision, expected.snapshot.revision);
                    assert_eq!(gate.rolled_back.load(Ordering::SeqCst), 1);
                    assert!(sync.baseline.is_none());
                    assert_eq!(service.snapshot().state, SyncState::Error);
                    assert!(!service.snapshot().restart_required);
                } else {
                    let result = result.expect("completed download remains successful after cancellation");
                    assert!(result.downloaded && result.restart_required);
                    assert_eq!(saved.snapshot.value.settings.color_mode, ColorMode::Dark);
                    assert!(saved.snapshot.revision > expected.snapshot.revision);
                    assert_eq!(sync.baseline.expect("successful download baseline").revision, remote_revision.revision);
                    assert_eq!(gate.rolled_back.load(Ordering::SeqCst), 0);
                    assert_eq!(service.snapshot().state, SyncState::Success);
                    assert!(service.snapshot().restart_required);
                    assert_eq!(service.snapshot().restart_revision, saved.snapshot.revision);
                }
                tokio::time::timeout(Duration::from_secs(2), closing).await.expect("drained sync shutdown").expect("shutdown sync");
                configuration.shutdown().await.expect("shutdown configuration");
                storage.close_for_test().await.expect("close database");
            }
        }).await.expect("four isolated commit outcomes complete within ten seconds");
    }

    #[tokio::test]
    async fn cancelled_callers_do_not_read_credentials_or_leave_syncing_status() {
        let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-sync-cancel-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&directory.0).expect("isolated sync directory");
        let storage = Storage::open(&directory.0.join("history.sqlite3")).await.expect("database");
        let credentials = Arc::new(Credentials { reads: AtomicUsize::new(0), delayed_failure: AtomicBool::new(false), entered: tokio::sync::Notify::new(), release: tokio::sync::Notify::new() });
        let vault = CredentialVault::new(storage.clone(), credentials.clone());
        let events = EventBus::new(32).expect("events");
        let runtime = Handle::current();
        let configuration = ConfigurationService::open(storage.clone(), vault, events.clone(), runtime.clone()).await.expect("configuration");
        let effects = Arc::new(Effects);
        let initial = configuration.snapshot();
        let expected = configuration.commit(ConfigurationMutation {
            client_id: uuid::Uuid::new_v4(), mutation_id: uuid::Uuid::new_v4(),
            base_versions: [ConfigField::WebDav, ConfigField::SyncProvider, ConfigField::WebDavPassword].into_iter().map(|field| (field, version(&initial, field))).collect(),
            changes: vec![ConfigChange::SyncProvider("webdav".into()), ConfigChange::WebDav(WebDavConfig {
                server_url: "http://127.0.0.1:1/dav/".into(), username: "fixture".into(), ..Default::default()
            })], credentials: vec![CredentialChange { id: CredentialId::WebDavPassword, change: SecretChange::Set(Arc::new(Secret::new("fixture".into()))) }],
        }, effects.clone()).await.expect("saved manual-sync configuration").state;
        let service = SyncService::new(storage.clone(), configuration.clone(), effects, events, directory.0.join("conflicts"), runtime).await.expect("sync service");
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(service.synchronize(expected.clone(), cancelled).await, Err(Error::Cancelled)));
        assert_eq!(service.snapshot().state, SyncState::Idle);

        // The current-thread executor cannot poll the owned task until wait().
        // This cancels an admitted request before its first credential access.
        let caller = CancellationToken::new();
        let pending = service.accept(RunKind::Manual, expected.clone(), caller.clone()).expect("admitted manual request");
        assert_eq!(service.snapshot().state, SyncState::Syncing);
        caller.cancel();
        assert!(matches!(service.wait(pending).await, Err(Error::Cancelled)));
        assert_eq!(service.snapshot().state, SyncState::Idle);
        assert_eq!(credentials.reads.load(Ordering::SeqCst), 0);

        // A native credential read already in progress must finish; its stale
        // failure must not turn a cancelled request into a new sync error.
        credentials.delayed_failure.store(true, Ordering::SeqCst);
        let caller = CancellationToken::new();
        let pending = service.accept(RunKind::Manual, expected, caller.clone()).expect("admitted credential read");
        tokio::time::timeout(Duration::from_secs(3), credentials.entered.notified()).await.expect("credential read started");
        caller.cancel();
        credentials.release.notify_one();
        assert!(matches!(service.wait(pending).await, Err(Error::Cancelled)));
        assert_eq!(credentials.reads.load(Ordering::SeqCst), 1);
        assert_eq!(service.snapshot().state, SyncState::Idle);
        assert!(service.snapshot().error.is_none());
        service.shutdown().await.expect("shutdown sync");
        configuration.shutdown().await.expect("shutdown configuration");
        storage.close_for_test().await.expect("close database");
    }
}
