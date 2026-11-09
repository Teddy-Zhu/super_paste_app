use std::sync::{Arc, Mutex};

use serde_json::to_value;
use tokio::{sync::Mutex as AsyncMutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    events::EventSink,
    mcp::{
        http::HttpRuntime,
        server::McpServerContext,
        types::{
            MCP_HOST, MCP_PATH, McpBackend, McpClipboardChange, McpSettings, McpState, McpStatus,
        },
    },
};

struct RunningServer {
    cancellation: CancellationToken,
    server_task: JoinHandle<()>,
    notification_task: JoinHandle<()>,
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct ServiceInner {
    backend: Arc<dyn McpBackend>,
    version: String,
    events: EventSink,
    lifecycle: AsyncMutex<()>,
    settings: Mutex<McpSettings>,
    status: Mutex<McpStatus>,
    context: Mutex<Option<Arc<McpServerContext>>>,
    runtime: AsyncMutex<Option<RunningServer>>,
}

#[derive(Clone)]
pub struct McpService {
    inner: Arc<ServiceInner>,
}

impl McpService {
    pub fn new(
        backend: Arc<dyn McpBackend>,
        version: impl Into<String>,
        events: EventSink,
    ) -> Self {
        let settings = McpSettings::default();
        let status = status_for(&settings, false, None);
        Self {
            inner: Arc::new(ServiceInner {
                backend,
                version: version.into(),
                events,
                lifecycle: AsyncMutex::new(()),
                settings: Mutex::new(settings),
                status: Mutex::new(status),
                context: Mutex::new(None),
                runtime: AsyncMutex::new(None),
            }),
        }
    }

    pub fn state(&self) -> McpState {
        McpState {
            settings: self
                .inner
                .settings
                .lock()
                .expect("MCP settings lock poisoned")
                .clone(),
            status: self
                .inner
                .status
                .lock()
                .expect("MCP status lock poisoned")
                .clone(),
        }
    }

    pub fn report_error(&self, error: String) {
        self.inner.report_error(error);
    }

    pub async fn apply_settings(&self, settings: McpSettings) -> Result<McpStatus, String> {
        validate_settings(&settings)?;
        let _lifecycle = self.inner.lifecycle.lock().await;
        let current = self.state();
        if current.settings == settings && current.status.running {
            return Ok(current.status);
        }

        self.stop_locked().await?;
        *self
            .inner
            .settings
            .lock()
            .expect("MCP settings lock poisoned") = settings.clone();
        self.set_status(status_for(&settings, false, None));
        if !settings.enabled {
            return Ok(self.state().status);
        }

        match self.start_locked(settings.clone()).await {
            Ok(()) => Ok(self.state().status),
            Err(error) => {
                self.set_status(status_for(&settings, false, Some(error.clone())));
                Err(error)
            }
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.stop_locked().await
    }

    pub fn record_clipboard_change(&self, change: McpClipboardChange) {
        let context = self
            .inner
            .context
            .lock()
            .expect("MCP server context lock poisoned")
            .clone();
        if let Some(context) = context {
            context.changes.record(change);
        }
    }

    async fn start_locked(&self, settings: McpSettings) -> Result<(), String> {
        let context = McpServerContext::new(self.inner.backend.clone(), self.inner.version.clone());
        let cancellation = CancellationToken::new();
        *self
            .inner
            .context
            .lock()
            .expect("MCP server context lock poisoned") = Some(context.clone());
        let runtime = match HttpRuntime::bind(
            settings.port,
            settings.clone(),
            context.clone(),
            cancellation.clone(),
        )
        .await
        {
            Ok(runtime) => runtime,
            Err(error) => {
                *self
                    .inner
                    .context
                    .lock()
                    .expect("MCP server context lock poisoned") = None;
                return Err(error);
            }
        };
        let notification_context = context.clone();
        let notification_cancellation = cancellation.clone();
        let notification_task = tokio::spawn(async move {
            notification_context
                .notification_worker(notification_cancellation)
                .await;
        });
        let task_cancellation = cancellation.clone();
        let task_context = Arc::downgrade(&self.inner);
        let task_settings = settings.clone();
        let server_task = tokio::spawn(async move {
            let _cancel_on_exit = CancelOnDrop(task_cancellation.clone());
            let result = runtime.serve().await;
            if task_cancellation.is_cancelled() {
                return;
            }
            let error = result
                .err()
                .map(|error| format!("MCP 服务异常停止：{error}"))
                .or_else(|| Some("MCP 服务已停止".to_owned()));
            if let Some(inner) = task_context.upgrade() {
                inner.set_status(status_for(&task_settings, false, error));
            }
        });

        *self.inner.runtime.lock().await = Some(RunningServer {
            cancellation,
            server_task,
            notification_task,
        });
        self.set_status(status_for(&settings, true, None));
        Ok(())
    }

    async fn stop_locked(&self) -> Result<(), String> {
        let running = self.inner.runtime.lock().await.take();
        *self
            .inner
            .context
            .lock()
            .expect("MCP server context lock poisoned") = None;
        let result = if let Some(running) = running {
            running.cancellation.cancel();
            let (server_result, notification_result) =
                tokio::join!(running.server_task, running.notification_task);
            server_result
                .map_err(|error| format!("MCP 服务关闭失败：{error}"))
                .and(notification_result.map_err(|error| format!("MCP 通知任务关闭失败：{error}")))
        } else {
            Ok(())
        };
        let settings = self
            .inner
            .settings
            .lock()
            .expect("MCP settings lock poisoned")
            .clone();
        self.set_status(status_for(&settings, false, result.as_ref().err().cloned()));
        result
    }

    fn set_status(&self, status: McpStatus) {
        self.inner.set_status(status);
    }
}

impl ServiceInner {
    fn set_status(&self, status: McpStatus) {
        *self.status.lock().expect("MCP status lock poisoned") = status.clone();
        let payload = to_value(status).expect("MCP status serializes");
        (self.events)("mcp:status", payload);
    }

    fn report_error(&self, error: String) {
        let status = {
            let mut status = self.status.lock().expect("MCP status lock poisoned");
            status.error = Some(error);
            status.clone()
        };
        let payload = to_value(status).expect("MCP status serializes");
        (self.events)("mcp:status", payload);
    }
}

fn validate_settings(settings: &McpSettings) -> Result<(), String> {
    if !(1024..=u16::MAX).contains(&settings.port) {
        return Err("MCP 端口范围须为 1024–65535".to_owned());
    }
    if settings.enabled && settings.auth_enabled {
        let key = settings.api_key.as_bytes();
        if !(8..=256).contains(&key.len()) || !key.iter().all(|byte| (0x21..=0x7e).contains(byte)) {
            return Err("MCP API Key 须为 8–256 位可见 ASCII 字符且不含空格".to_owned());
        }
    }
    Ok(())
}

fn status_for(settings: &McpSettings, running: bool, error: Option<String>) -> McpStatus {
    McpStatus {
        running,
        endpoint: endpoint(settings.port),
        error,
    }
}

fn endpoint(port: u16) -> String {
    format!("http://{MCP_HOST}:{port}{MCP_PATH}")
}
