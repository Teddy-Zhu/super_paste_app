use std::{collections::BTreeSet, future::Future, panic::AssertUnwindSafe, sync::{Arc, Mutex}};
use futures_util::FutureExt;
use sha2::{Digest as _, Sha256};
use super_paste_format::{ConfigField, CredentialId, McpStatus, mcp_endpoint};
use tokio::{net::TcpListener, runtime::Handle, sync::{Mutex as AsyncMutex, OwnedMutexGuard, Semaphore, watch}, task::JoinHandle};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use crate::{Error, Result, clipboard::ClipboardService, configuration::{ConfigurationService, ConfigurationState, PreparedConfigurationEffects, RuntimeConfiguration}, credentials::Secret, events::{AppEvent, EventBus}, images::ImageService, ocr::OcrService, storage::Storage};
use super::{backend::Backend, http::HttpRuntime, server::McpServerContext, types::RuntimeSettings};

pub const FIELDS: [ConfigField; 4] = [ConfigField::McpEnabled, ConfigField::McpPort, ConfigField::McpAuthEnabled, ConfigField::McpApiKey];
struct Running { listener: Arc<TcpListener>, stop: CancellationToken, server: JoinHandle<Result<()>>, notifications: JoinHandle<Result<()>> }
struct Lifecycle { settings: RuntimeSettings, running: Option<Running> }
struct PreparedServer { context: Arc<McpServerContext>, http: HttpRuntime, stop: CancellationToken }
pub struct McpService {
    configuration: Arc<ConfigurationService>, backend: Arc<Backend>, version: String, events: EventBus, runtime: Handle,
    lifecycle: Arc<AsyncMutex<Lifecycle>>, context: Mutex<Option<Arc<McpServerContext>>>, status: watch::Sender<Arc<McpStatus>>,
    admission: Mutex<bool>, permits: Arc<Semaphore>, tasks: TaskTracker, root: CancellationToken,
}
pub struct PreparedMcpChange {
    service: Arc<McpService>, state: OwnedMutexGuard<Lifecycle>, target: RuntimeSettings, prepared: Option<PreparedServer>,
    before: RuntimeSettings, before_listener: Option<Arc<TcpListener>>, before_status: Arc<McpStatus>, changed: bool, attempted: bool,
}
impl McpService {
    pub fn new(configuration: Arc<ConfigurationService>, clipboard: Arc<ClipboardService>, storage: Arc<Storage>, images: Arc<ImageService>, ocr: Arc<OcrService>, version: String, events: EventBus, runtime: Handle) -> Arc<Self> {
        let snapshot = configuration.snapshot(); let root = CancellationToken::new();
        let settings = RuntimeSettings { value: snapshot.snapshot.value.mcp.clone(), reference: snapshot.credentials.get(&CredentialId::McpApiKey).copied(), authorization_digest: None };
        let (status, _) = watch::channel(Arc::new(status_for(&settings, false, None)));
        let mut changes = clipboard.subscribe_changes(); changes.borrow_and_update();
        let service = Arc::new(Self { configuration, backend: Arc::new(Backend { clipboard, storage, images, ocr, shutdown: root.clone() }), version, events, runtime,
            lifecycle: Arc::new(AsyncMutex::new(Lifecycle { settings, running: None })), context: Mutex::new(None), status,
            admission: Mutex::new(true), permits: Arc::new(Semaphore::new(4)), tasks: TaskTracker::new(), root });
        let weak = Arc::downgrade(&service); let cancel = service.root.clone();
        service.tasks.spawn_on(async move {
            loop { tokio::select! { biased; _ = cancel.cancelled() => return, changed = changes.changed() => {
                if changed.is_err() { return; }
                let change = changes.borrow_and_update().clone();
                let Some(service) = weak.upgrade() else { return; };
                let context = service.context.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
                if let (Some(context), Some(change)) = (context, change) { context.changes.record(change); }
            } } }
        }, &service.runtime);
        service
    }
    pub fn snapshot(&self) -> Arc<McpStatus> { self.status.borrow().clone() }
    pub fn subscribe(&self) -> watch::Receiver<Arc<McpStatus>> { self.status.subscribe() }
    pub fn affects(fields: &BTreeSet<ConfigField>) -> bool { FIELDS.iter().any(|field| fields.contains(field)) }
    fn check_open(&self) -> Result<()> { if self.root.is_cancelled() { Err(Error::Cancelled) } else { Ok(()) } }
    fn ensure_current(&self, expected: &ConfigurationState) -> Result<()> {
        let current = self.configuration.snapshot();
        if FIELDS.iter().any(|field| current.fields.get(field).copied().unwrap_or(0) != expected.fields.get(field).copied().unwrap_or(0)) { Err(Error::Conflict("MCP 配置已变化，请刷新后重试".into())) } else { Ok(()) }
    }
    fn publish(&self, status: McpStatus) {
        let status = Arc::new(status); self.status.send_replace(status.clone()); self.events.publish(AppEvent::McpStatusChanged(status));
    }
    fn failure(&self, error: &Error) {
        tracing::error!(%error, "MCP 服务异常");
        let current = self.snapshot(); self.publish(McpStatus { running: false, endpoint: current.endpoint.clone(), error: Some(error.to_string()) });
    }
    pub async fn prepare(self: &Arc<Self>, configuration: RuntimeConfiguration, fields: &BTreeSet<ConfigField>) -> Result<PreparedMcpChange> {
        self.check_open()?;
        let value = configuration.value.mcp.clone();
        let key = if (value.enabled && value.auth_enabled) || fields.contains(&ConfigField::McpApiKey) { configuration.credential(CredentialId::McpApiKey).await? } else { None };
        let target = runtime_settings(value, configuration.credentials.get(&CredentialId::McpApiKey).copied(), key)?;
        let state = self.lifecycle.clone().lock_owned().await; self.check_open()?;
        self.prepare_locked(target, state).await
    }
    async fn prepare_locked(self: &Arc<Self>, target: RuntimeSettings, state: OwnedMutexGuard<Lifecycle>) -> Result<PreparedMcpChange> {
        let before = state.settings.clone(); let before_status = self.snapshot();
        let before_listener = state.running.as_ref().map(|running| running.listener.clone());
        let healthy = if target.enabled { before_status.running && state.running.as_ref().is_some_and(|running| !running.server.is_finished() && !running.notifications.is_finished()) }
            else { state.running.is_none() && before_status.error.is_none() };
        let changed = !before.same_identity(&target) || !healthy;
        let prepared = if changed && target.enabled {
            let listener = match before_listener.as_ref().filter(|_| before.port == target.port) {
                Some(listener) => listener.clone(), None => HttpRuntime::bind_listener(target.port).await.map_err(protocol_error)?,
            };
            Some(self.server(listener, target.clone()))
        } else { None };
        Ok(PreparedMcpChange { service: self.clone(), state, target, prepared, before, before_listener, before_status, changed, attempted: false })
    }
    fn server(&self, listener: Arc<TcpListener>, settings: RuntimeSettings) -> PreparedServer {
        let context = McpServerContext::new(self.backend.clone(), self.version.clone()); let stop = self.root.child_token();
        let http = HttpRuntime::with_listener(listener, settings, context.clone(), stop.clone());
        PreparedServer { context, http, stop }
    }
    fn start_locked(self: &Arc<Self>, state: &mut Lifecycle, prepared: PreparedServer) {
        let PreparedServer { context, http, stop } = prepared;
        let listener = http.listener.clone();
        *self.context.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(context.clone());
        self.publish(status_for(&state.settings, true, None));
        let notify_stop = stop.clone(); let notify_error = stop.clone(); let weak = Arc::downgrade(self);
        let notifications = self.runtime.spawn(async move {
            match AssertUnwindSafe(context.notification_worker(notify_stop)).catch_unwind().await {
                Ok(()) => Ok(()), Err(_) => {
                    let error = Error::Task("MCP 订阅通知任务异常退出".into()); notify_error.cancel();
                    if let Some(service) = weak.upgrade() { service.failure(&error); } Ok(())
                }
            }
        });
        let server_stop = stop.clone(); let weak = Arc::downgrade(self);
        let server = self.runtime.spawn(async move {
            let _stop_notifications = server_stop.clone().drop_guard();
            let result = AssertUnwindSafe(http.serve()).catch_unwind().await.map_err(|_| Error::Task("MCP HTTP 任务异常退出".into())).and_then(|result| result.map_err(protocol_error));
            if !server_stop.is_cancelled() {
                let error = result.as_ref().err().cloned().unwrap_or_else(|| Error::Task("MCP 服务已意外停止".into()));
                if let Some(service) = weak.upgrade() { service.failure(&error); }
                return Ok(());
            }
            result
        });
        state.running = Some(Running { listener, stop, server, notifications });
    }
    async fn stop_locked(&self, state: &mut Lifecycle) -> Result<()> {
        *self.context.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let mut errors = Vec::new();
        if let Some(running) = state.running.take() {
            running.stop.cancel(); let (server, notifications) = tokio::join!(running.server, running.notifications);
            for result in [server, notifications] { if let Err(error) = result.map_err(Error::from).and_then(|result| result) { errors.push(error.to_string()); } }
        }
        let error = (!errors.is_empty()).then(|| errors.join("；")); self.publish(status_for(&state.settings, false, error.clone()));
        match error { Some(error) => Err(Error::Task(error.into())), None => Ok(()) }
    }
    /// Reapply an unchanged saved configuration after a transient listener failure.
    /// Accepted retries outlive their requesting pane and serialize with commits.
    pub async fn retry(self: &Arc<Self>, expected: ConfigurationState) -> Result<Arc<McpStatus>> {
        self.owned(move |service| async move {
            let result = service.retry_inner(expected.clone()).await;
            if let Err(error) = &result { if !matches!(error, Error::Cancelled | Error::Conflict(_) | Error::ResourceLimit(_)) {
                let state = service.lifecycle.lock().await;
                if service.check_open().is_ok() && service.ensure_current(&expected).is_ok() {
                    let running = state.running.as_ref().is_some_and(|running| !running.stop.is_cancelled() && !running.server.is_finished() && !running.notifications.is_finished());
                    service.publish(McpStatus { running, endpoint: mcp_endpoint(state.settings.port), error: Some(error.to_string()) });
                }
            } }
            result
        }).await
    }
    async fn retry_inner(self: &Arc<Self>, expected: ConfigurationState) -> Result<Arc<McpStatus>> {
        self.ensure_current(&expected)?;
        let value = expected.snapshot.value.mcp.clone();
        let key = if value.enabled && value.auth_enabled { Some(self.configuration.guarded_credential(CredentialId::McpApiKey, &expected, &FIELDS).await?.1) } else { None };
        let target = runtime_settings(value, expected.credentials.get(&CredentialId::McpApiKey).copied(), key)?;
        let state = self.lifecycle.clone().lock_owned().await; self.check_open()?; self.ensure_current(&expected)?;
        let mut prepared = self.prepare_locked(target, state).await?;
        if let Err(error) = prepared.apply().await {
            return Err(match prepared.rollback().await { Ok(()) => error, Err(rollback) => Error::Task(format!("{error}；MCP 回滚失败：{rollback}").into()) });
        }
        Ok(self.snapshot())
    }
    pub async fn read_key(self: &Arc<Self>, expected: ConfigurationState) -> Result<Arc<Secret>> {
        self.owned(move |service| async move { service.configuration.guarded_credential(CredentialId::McpApiKey, &expected, &FIELDS).await.map(|(_, key)| key) }).await
    }
    pub async fn generate_key(self: &Arc<Self>) -> Result<Arc<Secret>> {
        self.owned(|_| async { Ok(Arc::new(Secret::new(format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())))) }).await
    }
    async fn owned<T, F, Fut>(self: &Arc<Self>, operation: F) -> Result<T>
    where T: Clone + Send + Sync + 'static, F: FnOnce(Arc<Self>) -> Fut + Send + 'static, Fut: Future<Output = Result<T>> + Send + 'static {
        let mut receiver = {
            let accepting = self.admission.lock().map_err(|_| Error::Task("MCP 操作接入锁损坏".into()))?;
            if !*accepting { return Err(Error::Cancelled); }
            let permit = self.permits.clone().try_acquire_owned().map_err(|_| Error::ResourceLimit("MCP 操作过多，请稍后重试".into()))?;
            let (sender, receiver) = watch::channel(None); let service = self.clone();
            self.tasks.spawn_on(async move {
                let _permit = permit;
                let result = AssertUnwindSafe(operation(service)).catch_unwind().await.unwrap_or_else(|_| Err(Error::Task("MCP 操作异常退出".into())));
                if let Err(error) = &result { if !matches!(error, Error::Cancelled) { tracing::warn!(%error, "MCP 操作失败"); } }
                sender.send_replace(Some(result));
            }, &self.runtime); receiver
        };
        loop { if let Some(result) = receiver.borrow_and_update().clone() { return result; } receiver.changed().await.map_err(|_| Error::Task("MCP 操作意外停止".into()))?; }
    }
    pub async fn shutdown(&self) -> Result<()> {
        { let mut accepting = self.admission.lock().map_err(|_| Error::Task("MCP 操作接入锁损坏".into()))?; *accepting = false; self.root.cancel(); self.tasks.close(); }
        let result = { let mut state = self.lifecycle.lock().await; self.stop_locked(&mut state).await };
        self.tasks.wait().await; result
    }
}
impl Drop for McpService { fn drop(&mut self) { self.root.cancel(); } }
impl PreparedConfigurationEffects for PreparedMcpChange {
    async fn apply(&mut self) -> Result<()> {
        if !self.changed { return Ok(()); } self.attempted = true;
        self.service.stop_locked(&mut self.state).await?; self.state.settings = self.target.clone();
        if let Some(prepared) = self.prepared.take() { self.service.start_locked(&mut self.state, prepared); }
        else { self.service.publish(status_for(&self.state.settings, false, None)); }
        Ok(())
    }
    async fn rollback(&mut self) -> Result<()> {
        if !self.attempted { return Ok(()); }
        let result = self.service.stop_locked(&mut self.state).await; self.state.settings = self.before.clone();
        if self.before_status.running {
            let listener = self.before_listener.clone().ok_or_else(|| Error::Task("MCP 回滚监听器缺失".into()))?;
            let prepared = self.service.server(listener, self.before.clone()); self.service.start_locked(&mut self.state, prepared);
        } else { self.service.publish((*self.before_status).clone()); }
        result
    }
}
fn runtime_settings(value: super_paste_format::McpSettings, reference: Option<crate::credentials::CredentialReference>, key: Option<Arc<Secret>>) -> Result<RuntimeSettings> {
    if value.port < 1024 { return Err(Error::Invalid("MCP 端口范围须为 1024–65535".into())); }
    let authorization_digest = match key {
        Some(key) => {
            let bytes = key.expose().as_bytes();
            if !(8..=256).contains(&bytes.len()) || !bytes.iter().all(|byte| (0x21..=0x7e).contains(byte)) { return Err(Error::Invalid("MCP API Key 须为 8–256 位可见 ASCII 字符且不含空格".into())); }
            let mut digest = Sha256::new(); digest.update(b"Bearer "); digest.update(bytes); Some(digest.finalize().into())
        }, None => None,
    };
    if value.enabled && value.auth_enabled && authorization_digest.is_none() { return Err(Error::Invalid("请先设置 MCP API Key".into())); }
    Ok(RuntimeSettings { value, reference, authorization_digest })
}
fn status_for(settings: &RuntimeSettings, running: bool, error: Option<String>) -> McpStatus { McpStatus { running, endpoint: mcp_endpoint(settings.port), error } }
fn protocol_error(error: String) -> Error { Error::Task(error.into()) }
