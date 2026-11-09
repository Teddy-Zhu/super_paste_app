use serde_json::Value;
use std::sync::Arc;
use tauri::{Emitter, Manager};

pub type EventSink = Arc<dyn Fn(&str, Value) + Send + Sync>;
pub type JsonFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>>;
pub type SettingsProvider = Arc<dyn Fn() -> JsonFuture + Send + Sync>;

pub fn sink(app: tauri::AppHandle) -> EventSink {
    Arc::new(move |channel, payload| {
        let committed_key = if channel == "store-value-changed" {
            payload
                .get("key")
                .and_then(Value::as_str)
                .map(str::to_owned)
        } else {
            None
        };
        if let Err(error) = app.emit(channel, vec![payload]) {
            tracing::error!(channel, %error, "desktop event delivery failed");
        }
        if let Some(key) = committed_key
            && let Some(holder) = app.try_state::<crate::runtime::RuntimeHolder>()
            && let Ok(runtime) = holder.get()
        {
            runtime.configuration_committed(key);
        }
    })
}

pub fn emit_window(
    app: &tauri::AppHandle,
    label: &str,
    channel: &str,
    args: Vec<Value>,
) -> Result<(), String> {
    let view = app
        .get_webview(label)
        .ok_or_else(|| format!("window is not available: {label}"))?;
    view.emit(channel, args).map_err(|e| e.to_string())
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
