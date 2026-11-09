use crate::{
    events::{EventSink, SettingsProvider, now_ms},
    platform::active_window::SourceTracker,
    storage::Storage,
};
use clipboard_core::{ClipboardSnapshot, WatchHandle};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, LazyLock, Mutex, RwLock,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub type RecordOcr = Arc<dyn Fn(String, bool) + Send + Sync>;
pub type ChangeObserver = Arc<dyn Fn(Value) + Send + Sync>;
pub type RecordsRemoved = Arc<dyn Fn(Vec<String>) + Send + Sync>;

pub struct ClipboardService {
    storage: Arc<Storage>,
    settings: SettingsProvider,
    events: EventSink,
    source: Arc<SourceTracker>,
    watcher: Mutex<Option<WatchHandle>>,
    worker: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    sender: mpsc::Sender<(ClipboardSnapshot, Option<Value>, i64)>,
    last_digest: Mutex<Option<String>>,
    error: RwLock<Option<String>>,
    enabled: AtomicBool,
    auto_save: AtomicBool,
    shutdown: CancellationToken,
    ocr: RwLock<Option<RecordOcr>>,
    observer: RwLock<Option<ChangeObserver>>,
    removed: RwLock<Option<RecordsRemoved>>,
}

impl ClipboardService {
    pub fn new(
        storage: Arc<Storage>,
        settings: SettingsProvider,
        events: EventSink,
        source: Arc<SourceTracker>,
    ) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::channel(4);
        let service = Arc::new(Self {
            storage,
            settings,
            events,
            source,
            watcher: Mutex::new(None),
            worker: Mutex::new(None),
            sender,
            last_digest: Mutex::new(None),
            error: RwLock::new(None),
            enabled: AtomicBool::new(false),
            auto_save: AtomicBool::new(true),
            shutdown: CancellationToken::new(),
            ocr: RwLock::new(None),
            observer: RwLock::new(None),
            removed: RwLock::new(None),
        });
        let weak = Arc::downgrade(&service);
        let stop = service.shutdown.clone();
        let worker = tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => {
                        receiver.close();
                        while let Some((snapshot, source, timestamp)) = receiver.recv().await {
                            let Some(service) = weak.upgrade() else { break };
                            if let Err(error) = service.capture(snapshot, source, timestamp).await { service.report_error(error); }
                        }
                        break;
                    },
                    next = receiver.recv() => {
                        let Some((snapshot, source, timestamp)) = next else { break };
                        let Some(service) = weak.upgrade() else { break };
                        if let Err(error) = service.capture(snapshot, source, timestamp).await { service.report_error(error); }
                    }
                }
            }
        });
        *service.worker.lock().expect("new clipboard worker lock") = Some(worker);
        service
    }

    pub fn attach_ocr(&self, callback: RecordOcr) -> Result<(), String> {
        *self.ocr.write().map_err(|_| "OCR hook lock poisoned")? = Some(callback);
        Ok(())
    }
    pub fn attach_observer(&self, callback: ChangeObserver) -> Result<(), String> {
        *self
            .observer
            .write()
            .map_err(|_| "clipboard observer lock poisoned")? = Some(callback);
        Ok(())
    }
    pub fn attach_removed(&self, callback: RecordsRemoved) -> Result<(), String> {
        *self
            .removed
            .write()
            .map_err(|_| "clipboard removal hook lock poisoned")? = Some(callback);
        Ok(())
    }
    pub fn set_auto_save(&self, enabled: bool) {
        self.auto_save.store(enabled, Ordering::Release);
    }
    pub fn is_running(&self) -> bool {
        !self.shutdown.is_cancelled()
    }
    pub async fn read_current(&self) -> Result<ClipboardSnapshot, String> {
        tokio::task::spawn_blocking(|| clipboard_core::read().map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())?
    }
    pub fn is_listening(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
            && self
                .watcher
                .lock()
                .is_ok_and(|guard| guard.as_ref().is_some_and(WatchHandle::is_running))
    }
    pub fn error(&self) -> Option<String> {
        self.error
            .read()
            .map(|v| v.clone())
            .unwrap_or_else(|_| Some("clipboard state lock poisoned".into()))
    }

    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        if self.shutdown.is_cancelled() {
            return Err("剪贴板服务已关闭".into());
        }
        let mut watcher = self
            .watcher
            .lock()
            .map_err(|_| "clipboard watcher lock poisoned")?;
        if watcher.as_ref().is_some_and(WatchHandle::is_running) {
            return Ok(());
        }
        watcher.take();
        self.enabled.store(true, Ordering::Release);
        let weak = Arc::downgrade(self);
        let sender = self.sender.clone();
        let source = self.source.clone();
        let watch = clipboard_core::watch(move |result| {
            let Some(service) = weak.upgrade() else {
                return;
            };
            if !service.enabled.load(Ordering::Acquire) {
                return;
            }
            match result {
                Ok(snapshot) => match source.snapshot(snapshot.image.is_some()) {
                    Ok(source) => {
                        if sender.blocking_send((snapshot, source, now_ms())).is_err() {
                            service.report_error("剪贴板处理队列已关闭".into());
                        }
                    }
                    Err(error) => service.report_error(error),
                },
                Err(error) => service.report_error(error.to_string()),
            }
        });
        let watch = match watch {
            Ok(watch) => watch,
            Err(error) => {
                self.enabled.store(false, Ordering::Release);
                self.report_error(error.to_string());
                return Err(error.to_string());
            }
        };
        *watcher = Some(watch);
        *self
            .error
            .write()
            .map_err(|_| "clipboard error lock poisoned")? = None;
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        let mut watcher = self
            .watcher
            .lock()
            .map_err(|_| "clipboard watcher lock poisoned")?;
        self.enabled.store(false, Ordering::Release);
        watcher.take();
        Ok(())
    }

    pub async fn shutdown(self: &Arc<Self>) -> Result<(), String> {
        self.enabled.store(false, Ordering::Release);
        self.shutdown.cancel();
        let service = self.clone();
        let stopped = tokio::task::spawn_blocking(move || service.stop())
            .await
            .map_err(|error| error.to_string())?;
        let worker = self
            .worker
            .lock()
            .map_err(|_| "clipboard worker lock poisoned")?
            .take();
        if let Some(worker) = worker {
            worker.await.map_err(|error| error.to_string())?;
        }
        stopped
    }

    pub async fn restore(
        self: &Arc<Self>,
        id: &str,
        text_only: bool,
        ocr_only: bool,
    ) -> Result<(), String> {
        let item = self
            .storage
            .get_clipboard(id)
            .await?
            .ok_or("历史记录不存在")?;
        let mut snapshot = from_content(&item["content"])?;
        if ocr_only {
            let text = item["content"]["ocr"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or("该记录没有 OCR 文本")?;
            snapshot = ClipboardSnapshot {
                text: Some(text.into()),
                ..Default::default()
            };
        } else if text_only {
            let text = plain_text(&item["content"])?;
            snapshot = ClipboardSnapshot {
                text: Some(text),
                ..Default::default()
            };
        }
        self.write(snapshot).await?;
        self.storage.touch_clipboard(id).await?;
        (self.events)("clipboard-history-updated", Value::Null);
        Ok(())
    }

    pub async fn write(self: &Arc<Self>, snapshot: ClipboardSnapshot) -> Result<(), String> {
        tokio::task::spawn_blocking(move || {
            clipboard_core::write(&snapshot).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn write_text(self: &Arc<Self>, text: String) -> Result<(), String> {
        self.write(ClipboardSnapshot {
            text: Some(text),
            ..Default::default()
        })
        .await
    }

    pub async fn status(&self) -> Result<Value, String> {
        let settings = (self.settings)().await?;
        let history_count = self.storage.history_count().await?;
        let mut config = manager_config(&settings);
        config["enableAutoSave"] = json!(self.auto_save.load(Ordering::Acquire));
        Ok(json!({"isListening":self.is_listening(),"config":config,"historyCount":history_count}))
    }

    async fn capture(
        &self,
        snapshot: ClipboardSnapshot,
        source: Option<Value>,
        now: i64,
    ) -> Result<(), String> {
        if snapshot.is_empty() {
            return Ok(());
        }
        let (snapshot, content, digest) = tokio::task::spawn_blocking(move || {
            let content = textual_content(&snapshot);
            let mut hash = Sha256::new();
            hash.update(serde_json::to_vec(&content).map_err(|e| e.to_string())?);
            if let Some(image) = &snapshot.image {
                hash.update(b"\0image\0");
                hash.update((image.len() as u64).to_be_bytes());
                hash.update(image);
            }
            // Raw-offer equality filters duplicate native notifications. Storage defines canonical pixel identity.
            let digest = hex::encode(hash.finalize());
            Ok::<_, String>((snapshot, content, digest))
        })
        .await
        .map_err(|e| e.to_string())??;
        {
            let mut last = self
                .last_digest
                .lock()
                .map_err(|_| "clipboard digest lock poisoned")?;
            if last.as_ref() == Some(&digest) {
                return Ok(());
            }
            *last = Some(digest);
        }
        let kind = primary_type(&snapshot);
        let mut item = json!({"content":content,"primaryType":kind,"availableFormats":formats(&snapshot),"timestamp":now,"lastUsed":now});
        let source_name = source
            .as_ref()
            .and_then(|s| s["application"].as_str())
            .unwrap_or("")
            .trim()
            .chars()
            .take(200)
            .collect::<String>();
        let observer = self
            .observer
            .read()
            .map_err(|_| "clipboard observer lock poisoned")?
            .clone();
        if let Some(observer) = observer {
            let mut change = item.clone();
            if let Some(image) = &snapshot.image {
                change["content"]["image"] = json!({"bytes":image.len()});
            }
            observer(change);
        }
        let bytes = content_bytes(&snapshot);
        self.storage.record_copy(&json!({"timestamp":now,"type":kind,"source":source_name,"bytes":bytes,"files":snapshot.files.len()})).await?;
        (self.events)("statistics:copy-updated", Value::Null);
        if !self.auto_save.load(Ordering::Acquire) {
            return Ok(());
        }
        let settings = (self.settings)().await?;
        if let Some(source) = source {
            let id = self.storage.upsert_app(&source).await?;
            item["source_app"] = json!(id);
        }
        let inserted = self
            .storage
            .upsert_clipboard_with_image(&item, snapshot.image)
            .await?;
        let removed = self
            .storage
            .cleanup_history(
                settings["maxHistorySize"]
                    .as_i64()
                    .ok_or("历史数量限制无效")?,
                settings["maxKeepTime"].as_i64().ok_or("历史保留时间无效")?,
            )
            .await?;
        let retained = !removed.contains(&inserted.id);
        if !removed.is_empty() {
            let callback = self
                .removed
                .read()
                .map_err(|_| "clipboard removal hook lock poisoned")?
                .clone();
            if let Some(callback) = callback {
                callback(removed);
            }
        }
        (self.events)("clipboard-history-updated", Value::Null);
        if retained
            && !self.shutdown.is_cancelled()
            && inserted.is_new
            && kind == "image"
            && settings["enableLocalOCR"].as_bool() == Some(true)
        {
            let callback = self
                .ocr
                .read()
                .map_err(|_| "OCR hook lock poisoned")?
                .clone();
            if let Some(callback) = callback {
                callback(inserted.id, true);
            }
        }
        Ok(())
    }

    fn report_error(&self, message: String) {
        if let Ok(mut error) = self.error.write() {
            *error = Some(message.clone());
        }
        tracing::error!(error = %message, "clipboard service failure");
        (self.events)("clipboard-service-error", json!({"error":message}));
    }
}

pub fn textual_content(snapshot: &ClipboardSnapshot) -> Value {
    let mut value = serde_json::Map::new();
    for (name, text) in [
        ("text", &snapshot.text),
        ("html", &snapshot.html),
        ("rtf", &snapshot.rtf),
    ] {
        if let Some(text) = text {
            value.insert(name.into(), json!(text));
        }
    }
    if !snapshot.files.is_empty() {
        value.insert("files".into(), json!(snapshot.files));
    }
    Value::Object(value)
}

pub fn from_content(value: &Value) -> Result<ClipboardSnapshot, String> {
    let image = value
        .get("image")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .transpose()?;
    let files = value
        .get("files")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or_default();
    Ok(ClipboardSnapshot {
        text: value.get("text").and_then(Value::as_str).map(str::to_owned),
        html: value.get("html").and_then(Value::as_str).map(str::to_owned),
        rtf: value.get("rtf").and_then(Value::as_str).map(str::to_owned),
        image,
        files,
    })
}

fn formats(snapshot: &ClipboardSnapshot) -> Vec<&'static str> {
    let mut formats = Vec::new();
    if snapshot.text.as_ref().is_some_and(|v| !v.is_empty()) {
        formats.push("text");
    }
    if snapshot.html.as_ref().is_some_and(|v| !v.is_empty()) {
        formats.push("html");
    }
    if snapshot.image.is_some() {
        formats.push("image");
    }
    if snapshot.rtf.as_ref().is_some_and(|v| !v.is_empty()) {
        formats.push("rtf");
    }
    if !snapshot.files.is_empty() {
        formats.push("files");
    }
    formats
}

fn primary_type(snapshot: &ClipboardSnapshot) -> &'static str {
    static COLOR: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^#(?:[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$").expect("constant color regex")
    });
    static URL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)^https?://[^\s]+$").expect("constant URL regex"));
    static EMAIL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").expect("constant email regex")
    });
    static PHONE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^1[3-9][0-9]{9}$").expect("constant phone regex"));
    if !snapshot.files.is_empty() {
        return if snapshot.files.len() == 1 && snapshot.image.is_some() {
            "image"
        } else {
            "files"
        };
    }
    if snapshot.image.is_some() {
        return "image";
    }
    if snapshot.rtf.as_ref().is_some_and(|v| !v.is_empty()) {
        return "rtf";
    }
    if snapshot.html.as_ref().is_some_and(|v| !v.is_empty()) {
        return "html";
    }
    let raw_text = snapshot.text.as_deref().unwrap_or("");
    if raw_text.is_empty() {
        return "empty";
    }
    let text = raw_text.trim();
    if COLOR.is_match(text) {
        return "color";
    }
    if URL.is_match(text) {
        return "url";
    }
    if EMAIL.is_match(text) {
        return "email";
    }
    let phone: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if PHONE.is_match(&phone) {
        return "phone";
    }
    if (text.contains('{') && text.contains('}'))
        || ["function", "class ", "import ", "export "]
            .iter()
            .any(|value| text.contains(value))
    {
        return "code";
    }
    "text"
}

pub fn plain_text(content: &Value) -> Result<String, String> {
    if let Some(text) = content
        .get("text")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        return Ok(text.to_owned());
    }
    if let Some(html) = content.get("html").and_then(Value::as_str) {
        let html = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>")
            .expect("constant script regex")
            .replace_all(html, "");
        let html = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>")
            .expect("constant style regex")
            .replace_all(&html, "");
        let html = regex::Regex::new(r"<[^>]*>")
            .expect("constant tag regex")
            .replace_all(&html, "");
        let html = html
            .replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'");
        let text = html.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            return Ok(text);
        }
    }
    if let Some(rtf) = content.get("rtf").and_then(Value::as_str) {
        let rtf = regex::Regex::new(r"\\[a-z]+[0-9]*\s?")
            .expect("constant RTF regex")
            .replace_all(rtf, "");
        let text = rtf
            .replace(['{', '}'], "")
            .replace("\\\\", "\\")
            .replace("\\'", "'")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            return Ok(text);
        }
    }
    if let Some(files) = content
        .get("files")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        return files
            .iter()
            .map(|v| v.as_str().ok_or("无效文件路径"))
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join("\n"))
            .map_err(str::to_owned);
    }
    Err("该记录没有可恢复的纯文本".into())
}

fn content_bytes(snapshot: &ClipboardSnapshot) -> u64 {
    snapshot.text.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.html.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.rtf.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.image.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.files.iter().map(|v| v.len() as u64).sum::<u64>()
}

fn manager_config(settings: &Value) -> Value {
    json!({"maxHistorySize":settings["maxHistorySize"],"enableAutoSave":true,"enableLocalOCR":settings["enableLocalOCR"],"ocrMode":settings["ocrMode"],"ocrLocalModel":settings["ocrLocalModel"],"ocrAIConfigs":settings["ocrAIConfigs"],"ocrRemoteConfig":{"url":settings["ocrRemoteUrl"],"extraParams":settings["ocrRemoteExtraParams"],"imageFieldName":settings["ocrRemoteImageFieldName"],"requestFormat":settings["ocrRemoteRequestFormat"],"resultPath":settings["ocrRemoteResultPath"]}})
}
