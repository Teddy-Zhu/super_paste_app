use super::{Runtime, WindowAccess, argument, ok, transition};
use serde_json::Value;
use std::sync::Arc;
use tauri_plugin_opener::OpenerExt;

pub(super) async fn dispatch(
    channel: &str,
    webview: &tauri::Webview,
    runtime: &Arc<Runtime>,
    args: &[Value],
    access: WindowAccess,
) -> Result<Value, String> {
    match channel {
        "app:window-minimize" => {
            webview.window().minimize().map_err(|e| e.to_string())?;
            Ok(ok())
        }
        "app:window-close" => {
            match access {
                WindowAccess::Main => runtime.windows.hide_main().await?,
                WindowAccess::Overlay => runtime.windows.request_close(true).await?,
            }
            Ok(ok())
        }
        "open-overlay-window" => {
            runtime.windows.toggle(None).await?;
            Ok(ok())
        }
        "open-main-window" => {
            runtime.windows.open_main().await?;
            Ok(ok())
        }
        "open-external-url" => {
            let url: String = argument(args, 0)?;
            let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err("仅允许打开 HTTP 或 HTTPS 网页".into());
            }
            runtime
                .app
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|e| e.to_string())?;
            Ok(ok())
        }
        "overlay-ready" => {
            runtime.windows.ready().await?;
            Ok(ok())
        }
        "overlay-open-prepared" => {
            runtime.windows.prepared(transition(args)?).await?;
            Ok(ok())
        }
        "overlay-close-started" => {
            runtime.windows.close_started(transition(args)?).await?;
            Ok(ok())
        }
        "open-overlay-finished" => {
            runtime.windows.opened(transition(args)?).await?;
            Ok(ok())
        }
        "close-overlay-window" => {
            runtime.windows.close_ack(transition(args)?).await?;
            Ok(ok())
        }
        "overlay-trace" => {
            // Only lifecycle metadata is logged; arbitrary renderer objects may
            // contain clipboard contents and are deliberately not formatted.
            if let Some(trace) = args.first() {
                tracing::debug!(
                    phase = trace["event"].as_str().unwrap_or("unknown"),
                    transition = trace["transitionId"].as_u64(),
                    "overlay transition"
                );
            }
            Ok(ok())
        }
        _ => Err(format!("未由 windows 处理的命令: {channel}")),
    }
}
