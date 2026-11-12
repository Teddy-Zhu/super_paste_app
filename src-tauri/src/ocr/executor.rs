use super::{
    ConfigSnapshot, OcrInner, OcrRecognition, OcrTrigger, QueuedTask, RecognitionMode, ai,
    catalog::get_model, local::LocalRecognition, models, remote,
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use tokio_util::sync::CancellationToken;

pub(super) async fn process_history_task(
    inner: &OcrInner,
    task: &QueuedTask,
    snapshot: &ConfigSnapshot,
) -> Result<(), String> {
    if task.cancel.is_cancelled() {
        return Err("OCR 任务已取消".to_owned());
    }
    let records = {
        let mut scheduler = inner.scheduler.lock().await;
        let Some(active) = scheduler.active.get_mut(&task.task_id) else {
            return Err("OCR 任务已取消".to_owned());
        };
        active.records.clone()
    };
    let Some(image_id) = records.keys().next().cloned() else {
        return Err("OCR 任务已取消".to_owned());
    };
    let Some(image) = inner.storage.get_clipboard_image(&image_id).await? else {
        return Err("剪贴板记录不包含图片".to_owned());
    };
    if task.cancel.is_cancelled() {
        return Err("OCR 任务已取消".to_owned());
    }
    let trigger = if records
        .values()
        .any(|trigger| *trigger == OcrTrigger::Automatic)
    {
        OcrTrigger::Automatic
    } else {
        OcrTrigger::Manual
    };
    let recognized =
        recognize_with_snapshot(inner, snapshot, image, &task.cancel, Some(trigger), false).await?;
    let _writeback = inner.history_writeback.lock().await;
    if task.cancel.is_cancelled() || inner.generation.load(Ordering::Acquire) != snapshot.generation
    {
        return Err("OCR 任务已取消".to_owned());
    }
    let records = {
        let mut scheduler = inner.scheduler.lock().await;
        let Some(active) = scheduler.active.get_mut(&task.task_id) else {
            return Err("OCR 任务已取消".to_owned());
        };
        if active.records.is_empty() {
            return Err("OCR 任务已取消".to_owned());
        }
        active.records.keys().cloned().collect::<Vec<_>>()
    };
    for content_id in records {
        if inner
            .storage
            .update_ocr(&content_id, &recognized.text)
            .await?
        {
            (inner.events)(
                "record-ocr-updated",
                json!({
                    "contentId":content_id,
                    "ocrText":recognized.text,
                    "timestamp":crate::events::now_ms(),
                }),
            );
        }
    }
    if let Some(active) = inner.scheduler.lock().await.active.get_mut(&task.task_id) {
        active.accepting = false;
        active.finished = true;
        active.outcome = Some(Ok(recognized.clone()));
    }
    Ok(())
}

async fn await_auto_preparation(
    inner: &OcrInner,
    model_id: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let receiver = inner.auto_preparations.lock().await.get(model_id).cloned();
    let Some(mut receiver) = receiver else {
        return Err("本地 OCR 模型未下载或已损坏，请在 OCR 设置中下载模型".to_owned());
    };
    loop {
        if let Some(result) = receiver.borrow().clone() {
            return result;
        }
        tokio::select! {
            _ = cancel.cancelled() => return Err("OCR 任务已取消".to_owned()),
            changed = receiver.changed() => {
                if changed.is_err() {
                    return Err("自动准备 OCR 模型异常退出".to_owned());
                }
            }
        }
    }
}

pub(super) async fn recognize_with_snapshot(
    inner: &OcrInner,
    snapshot: &ConfigSnapshot,
    image: Vec<u8>,
    cancel: &CancellationToken,
    trigger: Option<OcrTrigger>,
    urgent: bool,
) -> Result<OcrRecognition, String> {
    if cancel.is_cancelled() {
        return Err("OCR 任务已取消".to_owned());
    }
    match snapshot.mode {
        RecognitionMode::Local => {
            let installed = resolve_local_model(inner, snapshot, trigger, cancel).await?;
            let LocalRecognition { text, confidence } = inner
                .local
                .recognize(installed, image, cancel.clone(), urgent)
                .await?;
            Ok(OcrRecognition {
                text,
                confidence,
                mode: snapshot.mode.as_str().to_owned(),
                details: None,
            })
        }
        RecognitionMode::Remote => {
            let config = snapshot
                .remote
                .as_ref()
                .ok_or_else(|| "远程 OCR 配置未初始化".to_owned())?
                .as_ref()
                .map_err(Clone::clone)?;
            let result = remote::recognize(&inner.http, config, image, cancel).await?;
            Ok(OcrRecognition {
                text: result.text,
                confidence: 1.0,
                mode: snapshot.mode.as_str().to_owned(),
                details: Some(result.details),
            })
        }
        RecognitionMode::AI(provider) => {
            let config = snapshot
                .ai
                .as_ref()
                .ok_or_else(|| "AI OCR 配置为空".to_owned())?
                .as_ref()
                .map_err(Clone::clone)?
                .clone();
            let credentials = tokio::select! {
                _ = cancel.cancelled() => return Err("AI OCR 请求已取消".to_owned()),
                result = (inner.ai_config)(provider.as_str()) => result?,
            };
            let api_key = credentials
                .get("apiKey")
                .and_then(Value::as_str)
                .ok_or_else(|| "AI OCR API Key 不可用".to_owned())?;
            let config = config.with_api_key(provider, api_key)?;
            let result = ai::recognize(
                &inner.storage,
                &inner.http,
                provider,
                &config,
                image,
                cancel,
                &inner.events,
            )
            .await?;
            Ok(OcrRecognition {
                text: result.text,
                confidence: 0.0,
                mode: snapshot.mode.as_str().to_owned(),
                details: None,
            })
        }
    }
}

async fn resolve_local_model(
    inner: &OcrInner,
    snapshot: &ConfigSnapshot,
    trigger: Option<OcrTrigger>,
    cancel: &CancellationToken,
) -> Result<models::InstalledModel, String> {
    let model = get_model(&snapshot.model_id)?;
    let loaded = inner.local.is_loaded(&snapshot.model_id);
    if let Some(installed) = inner.models.installed(model, !loaded).await? {
        return Ok(installed);
    }
    if trigger == Some(OcrTrigger::Automatic) {
        await_auto_preparation(inner, &snapshot.model_id, cancel).await?;
        return inner
            .models
            .installed(model, true)
            .await?
            .ok_or_else(|| "本地 OCR 模型未下载或已损坏，请在 OCR 设置中下载模型".to_owned());
    }
    Err("本地 OCR 模型未下载或已损坏，请在 OCR 设置中下载模型".to_owned())
}
