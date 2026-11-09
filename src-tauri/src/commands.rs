use crate::{
    config::{StoreMutationMetadata, WindowAccess},
    events,
    ocr::OcrTrigger,
    runtime::{Runtime, RuntimeHolder},
    storage::Storage,
};
use futures_util::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use tauri_plugin_opener::OpenerExt;

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

// The routing state must not be embedded in every generated IPC wrapper. Its
// combined async branches otherwise exhaust Tokio's worker stack in dev builds.
fn dispatch<'a>(
    channel: &'a str,
    webview: &'a tauri::Webview,
    runtime: Arc<Runtime>,
    args: Vec<Value>,
    access: WindowAccess,
) -> BoxFuture<'a, Result<Value, String>> {
    Box::pin(async move {
        if matches!(
            channel,
            "statistics:get-copy" | "statistics:clear-copy" | "ocr:get-usage" | "ocr:clear-usage"
        ) || channel.starts_with("mcp:")
            || channel.starts_with("scheduled-sync:")
            || matches!(
                channel,
                "ocr:download-model"
                    | "ocr:cancel-download"
                    | "ocr:delete-model"
                    | "ocr:redownload-model"
                    | "ocr:ai-list-models"
                    | "ocr:ai-test-connection"
                    | "export-diagnostics"
                    | "system-access-retry"
                    | "app:restart"
            )
        {
            main_only(access)?;
        }
        if Storage::handles(channel) {
            let result = runtime.storage.invoke(channel, &args).await?;
            if result["success"].as_bool() == Some(true) {
                match channel {
                    "clipboard-delete-item" | "clipboard-clear-history" => {
                        let ids = result["deletedIds"]
                            .as_array()
                            .ok_or("删除操作未返回记录清单")?
                            .iter()
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_owned)
                                    .ok_or_else(|| "删除清单格式无效".to_string())
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        runtime.ocr.cancel_records(&ids).await?;
                        emit(
                            &runtime,
                            "clipboard-history-updated",
                            json!({"reason":"delete","ids":ids}),
                        );
                    }
                    "create-tag"
                    | "update-tag"
                    | "delete-tag"
                    | "bind-tag-to-record"
                    | "unbind-tag-from-record" => {
                        emit(&runtime, "tags-updated", Value::Null);
                        emit(
                            &runtime,
                            "clipboard-history-updated",
                            json!({"reason":"tags"}),
                        );
                    }
                    "statistics:clear-copy" => {
                        emit(&runtime, "statistics:copy-updated", Value::Null)
                    }
                    "ocr:clear-usage" => emit(&runtime, "ocr:usage-updated", Value::Null),
                    _ => {}
                }
            }
            return Ok(result);
        }
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
                    .get_snapshot(&argument::<String>(&args, 0)?, access)
                    .await?,
            )
            .map_err(|e| e.to_string()),
            "store:patch" => {
                let store: String = argument(&args, 0)?;
                let patch: Value = argument(&args, 1)?;
                let metadata: StoreMutationMetadata = argument(&args, 2)?;
                serde_json::to_value(runtime.config.patch(&store, patch, metadata, access).await)
                    .map_err(|e| e.to_string())
            }
            "clipboard-get-status" => data(runtime.clipboard.status().await?),
            "clipboard-start-listening" | "clipboard-stop-listening" => {
                let start = channel == "clipboard-start-listening";
                runtime.set_listening(start).await?;
                Ok(ok())
            }
            "clipboard-update-config" => update_clipboard_config(&runtime, &args, access).await,
            "clipboard-restore" | "clipboard-restore-as-text" | "clipboard-restore-ocr-text" => {
                runtime
                    .clipboard
                    .restore(
                        &record_id(&args)?,
                        channel == "clipboard-restore-as-text",
                        channel == "clipboard-restore-ocr-text",
                    )
                    .await?;
                Ok(ok())
            }
            "clipboard-parse-ocr" => {
                runtime
                    .ocr
                    .enqueue_record(&record_id(&args)?, OcrTrigger::Manual)
                    .await?;
                Ok(ok())
            }
            "clipboard-parse-qrcode" => {
                let image = runtime
                    .storage
                    .get_clipboard_image(&record_id(&args)?)
                    .await?
                    .ok_or("记录不包含图片")?;
                let ocr = runtime.ocr.clone();
                let text = tokio::task::spawn_blocking(move || ocr.parse_qr(&image))
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or("未识别到二维码")?;
                runtime.clipboard.write_text(text).await?;
                Ok(json!({"success":true,"message":"二维码内容已复制到剪贴板"}))
            }
            "clipboard-generate-qrcode" => {
                let qr = runtime
                    .ocr
                    .generate_qr_for_record(&record_id(&args)?)
                    .await?;
                runtime
                    .clipboard
                    .write(clipboard_core::ClipboardSnapshot {
                        image: Some(qr.png_bytes),
                        ..Default::default()
                    })
                    .await?;
                Ok(
                    json!({"success":true,"message":"二维码已生成并复制到剪贴板","data":{"qrCodeDataUrl":qr.data_url}}),
                )
            }
            "ocr:list-models" => data(runtime.ocr.list_models().await?),
            "ocr:download-model" => {
                runtime.ocr.download_model(&record_id(&args)?).await?;
                Ok(ok())
            }
            "ocr:cancel-download" => {
                runtime.ocr.cancel_download(&record_id(&args)?).await?;
                Ok(ok())
            }
            "ocr:delete-model" => {
                runtime.ocr.delete_model(&record_id(&args)?).await?;
                Ok(ok())
            }
            "ocr:redownload-model" => {
                runtime.ocr.redownload_model(&record_id(&args)?).await?;
                Ok(ok())
            }
            "ocr:ai-list-models" | "ocr:ai-test-connection" => {
                let request: Value = argument(&args, 0)?;
                let provider = request["provider"].as_str().ok_or("缺少 AI 提供商")?;
                let candidate = request["config"].as_object().ok_or("AI 配置格式无效")?;
                if candidate
                    .keys()
                    .any(|key| !matches!(key.as_str(), "baseUrl" | "model" | "api"))
                {
                    return Err("AI 测试仅接受非敏感连接参数，密钥须先通过配置保存".into());
                }
                let mut config = runtime.config.runtime_ai_config(provider).await?;
                let target = config.as_object_mut().ok_or("已保存的 AI 配置格式无效")?;
                for (key, value) in candidate {
                    target.insert(key.clone(), value.clone());
                }
                if channel == "ocr:ai-list-models" {
                    data(runtime.ocr.ai_list_models(provider, &config).await?)
                } else {
                    data(runtime.ocr.ai_test_connection(provider, &config).await?)
                }
            }
            "mcp:get-state" => data(runtime.mcp_config.state().await?),
            "mcp:save-settings" => {
                let settings = argument(&args, 0)?;
                data(runtime.mcp_config.save(settings).await?)
            }
            "mcp:generate-key" => data(crate::core::mcp_config::generate_key()),
            "scheduled-sync:test-connection" => {
                let result = runtime.sync.test_connection(args.first().cloned()).await;
                if result.success {
                    Ok(json!(true))
                } else {
                    Ok(json!({"success":false,"error":result.message}))
                }
            }
            "scheduled-sync:manual-sync" => {
                serde_json::to_value(runtime.sync.manual_sync().await).map_err(|e| e.to_string())
            }
            "scheduled-sync:manual-sync-with-config" => serde_json::to_value(
                runtime
                    .sync
                    .manual_sync_with_config(argument(&args, 0)?)
                    .await,
            )
            .map_err(|e| e.to_string()),
            "scheduled-sync:get-status" => data(runtime.sync.status().await),
            "system-access-status" => data(runtime.system_access().await?),
            "system-access-retry" => data(runtime.retry_access().await?),
            "app:restart" => {
                let runtime = runtime.clone();
                tauri::async_runtime::spawn(async move {
                    runtime.shutdown(true).await;
                });
                Ok(ok())
            }
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
                let url: String = argument(&args, 0)?;
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
                overlay_only(access)?;
                runtime.windows.ready().await?;
                Ok(ok())
            }
            "overlay-open-prepared" => {
                overlay_only(access)?;
                runtime.windows.prepared(transition(&args)?).await?;
                Ok(ok())
            }
            "overlay-close-started" => {
                overlay_only(access)?;
                runtime.windows.close_started(transition(&args)?).await?;
                Ok(ok())
            }
            "open-overlay-finished" => {
                overlay_only(access)?;
                runtime.windows.opened(transition(&args)?).await?;
                Ok(ok())
            }
            "close-overlay-window" => {
                overlay_only(access)?;
                runtime.windows.close_ack(transition(&args)?).await?;
                Ok(ok())
            }
            "overlay-trace" => {
                overlay_only(access)?;
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
            "export-diagnostics" => save_diagnostics(&runtime, &args).await,
            _ => Err(format!("未注册的命令: {channel}")),
        }
    })
}

async fn update_clipboard_config(
    runtime: &Arc<Runtime>,
    args: &[Value],
    access: WindowAccess,
) -> Result<Value, String> {
    let mut config: serde_json::Map<String, Value> = argument(args, 0)?;
    let auto_save = config.remove("enableAutoSave");
    if auto_save.as_ref().is_some_and(|v| !v.is_boolean()) {
        return Err("enableAutoSave 必须为布尔值".into());
    }
    if config.keys().any(|key| {
        !matches!(
            key.as_str(),
            "maxHistorySize"
                | "enableLocalOCR"
                | "ocrMode"
                | "ocrAIConfigs"
                | "ocrLocalModel"
                | "ocrRemoteConfig"
        )
    }) {
        return Err("剪贴板配置包含未知字段".into());
    }
    if let Some(remote) = config.remove("ocrRemoteConfig") {
        let remote = remote.as_object().ok_or("远程 OCR 配置必须为对象")?;
        let fields = [
            ("url", "ocrRemoteUrl"),
            ("extraParams", "ocrRemoteExtraParams"),
            ("imageFieldName", "ocrRemoteImageFieldName"),
            ("requestFormat", "ocrRemoteRequestFormat"),
            ("resultPath", "ocrRemoteResultPath"),
        ];
        if remote
            .keys()
            .any(|key| !fields.iter().any(|(source, _)| source == key))
        {
            return Err("远程 OCR 配置包含未知字段".into());
        }
        for (source, target) in fields {
            if let Some(value) = remote.get(source) {
                config.insert(target.to_owned(), value.clone());
            }
        }
    }
    if !config.is_empty() {
        let current = runtime.config.get_snapshot("settings", access).await?;
        let metadata = StoreMutationMetadata {
            client_id: "host:clipboard-config".into(),
            mutation_id: uuid::Uuid::new_v4().to_string(),
            base_versions: current.field_versions,
            credential_changes: Default::default(),
        };
        let result = runtime
            .config
            .patch("settings", Value::Object(config), metadata, access)
            .await;
        if !result.success {
            return Err(result.error.unwrap_or_else(|| "配置提交失败".into()));
        }
    }
    if let Some(enabled) = auto_save {
        runtime
            .clipboard
            .set_auto_save(enabled.as_bool().ok_or("enableAutoSave 无效")?);
    }
    Ok(ok())
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

macro_rules! desktop_commands{
    ($($name:ident=>$channel:literal),*$(,)?)=>{
        $(#[tauri::command]
        pub async fn $name(webview:tauri::Webview,state:tauri::State<'_,RuntimeHolder>,args:Option<Vec<Value>>)->Result<Value,String>{invoke($channel,webview,state,args).await})*
        pub fn handler()->impl Fn(tauri::ipc::Invoke<tauri::Wry>)->bool+Send+Sync+'static{tauri::generate_handler![$($name),*]}
    }
}
desktop_commands! {
    desktop_bootstrap=>"desktop_bootstrap",store_get_snapshot=>"store:get-snapshot",store_patch=>"store:patch",
    clipboard_get_history=>"clipboard-get-history",clipboard_restore=>"clipboard-restore",clipboard_restore_as_text=>"clipboard-restore-as-text",clipboard_restore_ocr_text=>"clipboard-restore-ocr-text",clipboard_parse_ocr=>"clipboard-parse-ocr",clipboard_parse_qrcode=>"clipboard-parse-qrcode",clipboard_generate_qrcode=>"clipboard-generate-qrcode",clipboard_delete_item=>"clipboard-delete-item",clipboard_clear_history=>"clipboard-clear-history",clipboard_get_status=>"clipboard-get-status",clipboard_start_listening=>"clipboard-start-listening",clipboard_stop_listening=>"clipboard-stop-listening",clipboard_update_config=>"clipboard-update-config",
    get_all_tags=>"get-all-tags",create_tag=>"create-tag",update_tag=>"update-tag",delete_tag=>"delete-tag",get_tag_stats=>"get-tag-stats",bind_tag_to_record=>"bind-tag-to-record",unbind_tag_from_record=>"unbind-tag-from-record",
    statistics_get_copy=>"statistics:get-copy",statistics_clear_copy=>"statistics:clear-copy",ocr_list_models=>"ocr:list-models",ocr_download_model=>"ocr:download-model",ocr_cancel_download=>"ocr:cancel-download",ocr_delete_model=>"ocr:delete-model",ocr_redownload_model=>"ocr:redownload-model",ocr_ai_list_models=>"ocr:ai-list-models",ocr_ai_test_connection=>"ocr:ai-test-connection",ocr_get_usage=>"ocr:get-usage",ocr_clear_usage=>"ocr:clear-usage",
    mcp_get_state=>"mcp:get-state",mcp_save_settings=>"mcp:save-settings",mcp_generate_key=>"mcp:generate-key",scheduled_sync_test_connection=>"scheduled-sync:test-connection",scheduled_sync_manual_sync=>"scheduled-sync:manual-sync",scheduled_sync_manual_sync_with_config=>"scheduled-sync:manual-sync-with-config",scheduled_sync_get_status=>"scheduled-sync:get-status",
    system_access_status=>"system-access-status",system_access_retry=>"system-access-retry",app_restart=>"app:restart",app_window_minimize=>"app:window-minimize",app_window_close=>"app:window-close",export_diagnostics=>"export-diagnostics",open_overlay_window=>"open-overlay-window",close_overlay_window=>"close-overlay-window",open_main_window=>"open-main-window",open_external_url=>"open-external-url",overlay_ready=>"overlay-ready",overlay_open_prepared=>"overlay-open-prepared",overlay_close_started=>"overlay-close-started",open_overlay_finished=>"open-overlay-finished",overlay_trace=>"overlay-trace",
}
