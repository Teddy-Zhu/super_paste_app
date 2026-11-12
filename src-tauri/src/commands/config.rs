use super::{Runtime, StoreMutationMetadata, WindowAccess, argument};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) async fn dispatch(
    channel: &str,
    webview: &tauri::Webview,
    runtime: &Arc<Runtime>,
    args: &[Value],
    access: WindowAccess,
) -> Result<Value, String> {
    match channel {
        "desktop_bootstrap" => {
            let config = runtime.config.get_bootstrap(access).await?;
            tracing::debug!(window = webview.label(), "desktop bootstrap ready");
            let platform = if cfg!(target_os = "windows") {
                "win32"
            } else if cfg!(target_os = "macos") {
                "darwin"
            } else {
                "linux"
            };
            Ok(
                json!({"config":config,"platform":{"platform":platform,"wayland":clipboard_core::is_wayland_session(),"version":runtime.app.package_info().version.to_string(),"appName":"Super Paste"}}),
            )
        }
        "store:get-snapshot" => serde_json::to_value(
            runtime
                .config
                .get_snapshot(&argument::<String>(args, 0)?, access)
                .await?,
        )
        .map_err(|e| e.to_string()),
        "store:patch" => {
            let store: String = argument(args, 0)?;
            let patch: Value = argument(args, 1)?;
            let metadata: StoreMutationMetadata = argument(args, 2)?;
            serde_json::to_value(runtime.config.patch(&store, patch, metadata, access).await)
                .map_err(|e| e.to_string())
        }
        _ => Err(format!("未由 config 处理的命令: {channel}")),
    }
}
