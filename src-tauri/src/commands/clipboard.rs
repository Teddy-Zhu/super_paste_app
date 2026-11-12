use super::{Runtime, StoreMutationMetadata, WindowAccess, argument, data, ok, record_id};
use crate::ocr::OcrTrigger;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) async fn dispatch(
    channel: &str,
    runtime: &Arc<Runtime>,
    args: &[Value],
    access: WindowAccess,
) -> Result<Value, String> {
    match channel {
        "clipboard-get-status" => data(runtime.clipboard.status().await?),
        "clipboard-start-listening" | "clipboard-stop-listening" => {
            let start = channel == "clipboard-start-listening";
            runtime.set_listening(start).await?;
            Ok(ok())
        }
        "clipboard-update-config" => update_clipboard_config(runtime, args, access).await,
        "clipboard-restore" | "clipboard-restore-as-text" | "clipboard-restore-ocr-text" => {
            runtime
                .clipboard
                .restore(
                    &record_id(args)?,
                    channel == "clipboard-restore-as-text",
                    channel == "clipboard-restore-ocr-text",
                )
                .await?;
            Ok(ok())
        }
        "clipboard-parse-ocr" => {
            runtime
                .ocr
                .enqueue_record(&record_id(args)?, OcrTrigger::Manual)
                .await?;
            Ok(ok())
        }
        "clipboard-copy-color" => {
            let text: String = argument(args, 0)?;
            if !crate::core::clipboard_content::is_color(&text) {
                return Err("颜色格式无效".into());
            }
            runtime.clipboard.write_text(text).await?;
            Ok(ok())
        }
        "clipboard-parse-qrcode" => {
            let image = runtime
                .storage
                .get_clipboard_image(&record_id(args)?)
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
                .generate_qr_for_record(&record_id(args)?)
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
        _ => Err(format!("未由 clipboard 处理的命令: {channel}")),
    }
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
