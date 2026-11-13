mod clipboard;
mod config;
mod mcp;
mod ocr;
mod policy;
mod storage;
mod sync;
mod system;
mod windows;

use crate::{
    config::{StoreMutationMetadata, WindowAccess},
    events,
    runtime::{Runtime, RuntimeHolder},
};
use futures_util::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;

// Every command resolves the caller supplied by Tauri. A renderer cannot claim a
// different window by adding a label to its request payload.
fn caller(webview: &tauri::Webview) -> Result<WindowAccess, String> {
    let window = webview.window();
    match (webview.label(), window.label()) {
        ("main", "main") => Ok(WindowAccess::Main),
        ("overlay", "overlay") => Ok(WindowAccess::Overlay),
        _ => Err("未授权的窗口".into()),
    }
}
fn main_only(access: WindowAccess) -> Result<(), String> {
    if matches!(access, WindowAccess::Main) {
        Ok(())
    } else {
        Err("此操作仅限设置窗口".into())
    }
}
fn overlay_only(access: WindowAccess) -> Result<(), String> {
    if matches!(access, WindowAccess::Overlay) {
        Ok(())
    } else {
        Err("此操作仅限粘贴面板".into())
    }
}
fn argument<T: DeserializeOwned>(args: &[Value], index: usize) -> Result<T, String> {
    serde_json::from_value(
        args.get(index)
            .cloned()
            .ok_or_else(|| format!("缺少第 {} 个参数", index + 1))?,
    )
    .map_err(|e| format!("参数格式无效: {e}"))
}
fn record_id(args: &[Value]) -> Result<String, String> {
    let id: String = argument(args, 0)?;
    if id.trim().is_empty() || id.len() > 256 {
        Err("记录标识无效".into())
    } else {
        Ok(id)
    }
}
fn transition(args: &[Value]) -> Result<i64, String> {
    argument(args, 0)
}
fn ok() -> Value {
    json!({"success":true})
}
fn data(value: impl serde::Serialize) -> Result<Value, String> {
    Ok(json!({"success":true,"data":serde_json::to_value(value).map_err(|e|e.to_string())?}))
}
fn emit(runtime: &Runtime, channel: &str, payload: Value) {
    (events::sink(runtime.app.clone()))(channel, payload);
}

async fn invoke(
    channel: &str,
    webview: tauri::Webview,
    holder: tauri::State<'_, RuntimeHolder>,
    args: Option<Vec<Value>>,
) -> Result<Value, String> {
    let access = caller(&webview)?;
    let runtime = holder.get()?;
    if runtime.is_quitting() {
        return Err("应用正在退出".into());
    }
    let result = dispatch(channel, &webview, runtime, args.unwrap_or_default(), access).await;
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            tracing::warn!(%channel,%error,"desktop command failed");
            if matches!(
                channel,
                "desktop_bootstrap" | "store:get-snapshot" | "store:patch"
            ) {
                Err(error)
            } else {
                Ok(json!({"success":false,"error":error,"message":error}))
            }
        }
    }
}

// Keep routing boxed so generated IPC wrappers do not grow Tokio worker stacks.
fn dispatch<'a>(
    channel: &'a str,
    webview: &'a tauri::Webview,
    runtime: Arc<Runtime>,
    args: Vec<Value>,
    access: WindowAccess,
) -> BoxFuture<'a, Result<Value, String>> {
    Box::pin(async move {
        policy::authorize(channel, access)?;
        match command_domain(channel) {
            Some(CommandDomain::Config) => {
                config::dispatch(channel, webview, &runtime, &args, access).await
            }
            Some(CommandDomain::Clipboard) => {
                clipboard::dispatch(channel, &runtime, &args, access).await
            }
            Some(CommandDomain::Ocr) => ocr::dispatch(channel, &runtime, &args).await,
            Some(CommandDomain::Mcp) => mcp::dispatch(channel, &runtime, &args).await,
            Some(CommandDomain::Sync) => sync::dispatch(channel, &runtime, &args).await,
            Some(CommandDomain::System) => system::dispatch(channel, &runtime, &args).await,
            Some(CommandDomain::Windows) => {
                windows::dispatch(channel, webview, &runtime, &args, access).await
            }
            Some(CommandDomain::Storage) => storage::dispatch(channel, &runtime, &args).await,
            None => Err(format!("未注册的命令: {channel}")),
        }
    })
}

enum CommandDomain {
    Config,
    Clipboard,
    Ocr,
    Mcp,
    Sync,
    System,
    Windows,
    Storage,
}

macro_rules! desktop_commands {
    ($($domain:ident { $($name:ident => $channel:literal),* $(,)? }),* $(,)?) => {
        $($(#[tauri::command]
        pub async fn $name(webview: tauri::Webview, state: tauri::State<'_, RuntimeHolder>, args: Option<Vec<Value>>) -> Result<Value, String> {
            invoke($channel, webview, state, args).await
        })*)*
        fn command_domain(channel: &str) -> Option<CommandDomain> {
            match channel {
                $($($channel)|* => Some(CommandDomain::$domain),)*
                _ => None,
            }
        }
        pub fn handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$($($name),*) ,*]
        }
    }
}
desktop_commands! {
    Config {
        desktop_bootstrap => "desktop_bootstrap",
        store_get_snapshot => "store:get-snapshot",
        store_patch => "store:patch",
    },
    Clipboard {
        clipboard_copy_color => "clipboard-copy-color",
        clipboard_restore => "clipboard-restore",
        clipboard_restore_as_text => "clipboard-restore-as-text",
        clipboard_restore_ocr_text => "clipboard-restore-ocr-text",
        clipboard_parse_ocr => "clipboard-parse-ocr",
        clipboard_parse_qrcode => "clipboard-parse-qrcode",
        clipboard_generate_qrcode => "clipboard-generate-qrcode",
        clipboard_get_status => "clipboard-get-status",
        clipboard_start_listening => "clipboard-start-listening",
        clipboard_stop_listening => "clipboard-stop-listening",
        clipboard_update_config => "clipboard-update-config",
    },
    Ocr {
        ocr_list_models => "ocr:list-models",
        ocr_download_model => "ocr:download-model",
        ocr_cancel_download => "ocr:cancel-download",
        ocr_delete_model => "ocr:delete-model",
        ocr_redownload_model => "ocr:redownload-model",
        ocr_ai_list_models => "ocr:ai-list-models",
        ocr_ai_test_connection => "ocr:ai-test-connection",
    },
    Mcp {
        mcp_get_state => "mcp:get-state",
        mcp_save_settings => "mcp:save-settings",
        mcp_generate_key => "mcp:generate-key",
    },
    Sync {
        scheduled_sync_test_connection => "scheduled-sync:test-connection",
        scheduled_sync_manual_sync => "scheduled-sync:manual-sync",
        scheduled_sync_manual_sync_with_config => "scheduled-sync:manual-sync-with-config",
        scheduled_sync_get_status => "scheduled-sync:get-status",
    },
    System {
        system_access_status => "system-access-status",
        system_access_retry => "system-access-retry",
        app_restart => "app:restart",
        export_diagnostics => "export-diagnostics",
    },
    Windows {
        app_window_minimize => "app:window-minimize",
        app_window_close => "app:window-close",
        open_overlay_window => "open-overlay-window",
        close_overlay_window => "close-overlay-window",
        open_main_window => "open-main-window",
        open_external_url => "open-external-url",
        overlay_ready => "overlay-ready",
        overlay_open_prepared => "overlay-open-prepared",
        overlay_close_started => "overlay-close-started",
        open_overlay_finished => "open-overlay-finished",
        overlay_trace => "overlay-trace",
    },
    Storage {
        clipboard_get_history => "clipboard-get-history",
        clipboard_get_item => "clipboard-get-item",
        clipboard_delete_item => "clipboard-delete-item",
        clipboard_clear_history => "clipboard-clear-history",
        get_all_tags => "get-all-tags",
        create_tag => "create-tag",
        update_tag => "update-tag",
        delete_tag => "delete-tag",
        get_tag_stats => "get-tag-stats",
        bind_tag_to_record => "bind-tag-to-record",
        unbind_tag_from_record => "unbind-tag-from-record",
        statistics_get_copy => "statistics:get-copy",
        statistics_clear_copy => "statistics:clear-copy",
        ocr_get_usage => "ocr:get-usage",
        ocr_clear_usage => "ocr:clear-usage",
    },
}
