use super::{Runtime, data, ok};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) async fn dispatch(
    channel: &str,
    runtime: &Arc<Runtime>,
    args: &[Value],
) -> Result<Value, String> {
    match channel {
        "system-access-status" => data(runtime.system_access().await?),
        "system-access-retry" => data(runtime.retry_access().await?),
        "app:restart" => {
            let runtime = runtime.clone();
            tauri::async_runtime::spawn(async move {
                runtime.shutdown(true).await;
            });
            Ok(ok())
        }
        "export-diagnostics" => save_diagnostics(runtime, args).await,
        _ => Err(format!("未由 system 处理的命令: {channel}")),
    }
}

async fn save_diagnostics(runtime: &Arc<Runtime>, args: &[Value]) -> Result<Value, String> {
    let user_agent = match args.first() {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value.len() <= 4096 => Some(value.clone()),
        _ => return Err("浏览器版本信息格式无效".into()),
    };
    let mcp = runtime.mcp_config.diagnostic_state().await?;
    let settings = runtime.config.runtime_settings().await?;
    let payload = json!({
        "format":"super-paste/diagnostics/v1","createdAt":chrono::Utc::now().to_rfc3339(),
        "app":{"name":"Super Paste","version":runtime.app.package_info().version.to_string(),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"webviewUserAgent":user_agent},
        "process":crate::diagnostics::Diagnostics::memory()?,"systemAccess":runtime.system_access().await?,
        "clipboard":{"running":runtime.clipboard.is_running(),"listening":runtime.clipboard.is_listening(),"historyCount":runtime.storage.history_count().await?,"lastError":runtime.clipboard.error()},
        "ocr":runtime.ocr.diagnostics().await,
        "mcp":mcp,
        "sync":runtime.sync.status().await,
        "preferences":{"colorMode":settings["colorMode"],"autoStart":settings["autoStart"],"clipboardListening":settings["clipboardListening"],"enableLocalOCR":settings["enableLocalOCR"],"ocrMode":settings["ocrMode"]},
        "logs":runtime.diagnostics.recent_events()?
    });
    match crate::diagnostics::Diagnostics::export(runtime.app.clone(), payload).await? {
        Some(file) => Ok(json!({"success":true,"file":file})),
        None => Ok(json!({"success":false,"message":"已取消导出"})),
    }
}
