mod executor;
use executor::{process_history_task, recognize_with_snapshot};
mod ai;
pub mod catalog;
mod layout;
mod local;
mod models;
mod qr;
mod remote;

use crate::{
    events::{EventSink, JsonFuture, SettingsProvider},
    storage::Storage,
};
use ai::{AIConfig, AIProvider};
use catalog::{OcrModelStatus, default_model_id, get_model};
use futures_util::future::BoxFuture;
use local::LocalWorker;
use models::ModelManager;
use remote::RemoteConfig;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, RwLock, Semaphore, watch};
use tokio_util::sync::CancellationToken;

const MAX_BACKGROUND_TASKS: usize = 3;
const MAX_PENDING_TASKS: usize = 2_000;
const MAX_DIRECT_TASKS: usize = 1;
const MAX_DIRECT_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const LOCAL_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub type OcrFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
pub type DisableAutoOcr = Arc<dyn Fn() -> OcrFuture + Send + Sync>;
pub type AIConfigProvider = Arc<dyn Fn(&str) -> JsonFuture + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrTrigger {
    Automatic,
    Manual,
}

fn merge_trigger(existing: OcrTrigger, incoming: OcrTrigger) -> OcrTrigger {
    if existing == OcrTrigger::Automatic && incoming == OcrTrigger::Automatic {
        OcrTrigger::Automatic
    } else {
        OcrTrigger::Manual
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrPurpose {
    Mcp,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrRecognition {
    pub text: String,
    pub confidence: f64,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecognitionMode {
    Local,
    Remote,
    AI(AIProvider),
}

impl RecognitionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::AI(provider) => provider.as_str(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ConfigSnapshot {
    generation: u64,
    automatic_enabled: bool,
    mode: RecognitionMode,
    model_id: String,
    remote: Option<Result<RemoteConfig, String>>,
    ai: Option<Result<AIConfig, String>>,
    credential_version: Option<Value>,
}

impl ConfigSnapshot {
    fn from_settings(settings: &Value) -> Result<Self, String> {
        let mode_name = settings
            .get("ocrMode")
            .and_then(Value::as_str)
            .unwrap_or("local");
        let mode = match mode_name {
            "local" => RecognitionMode::Local,
            "remote" => RecognitionMode::Remote,
            provider => RecognitionMode::AI(AIProvider::parse(provider)?),
        };
        let model_id = settings
            .get("ocrLocalModel")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_model_id())
            .to_owned();
        if mode == RecognitionMode::Local {
            get_model(&model_id)?;
        }
        let remote = if mode == RecognitionMode::Remote {
            Some(RemoteConfig::from_settings(settings))
        } else {
            None
        };
        let ai = match mode {
            RecognitionMode::AI(provider) => Some(AIConfig::from_settings(provider, settings)),
            _ => None,
        };
        Ok(Self {
            generation: 0,
            automatic_enabled: settings
                .get("enableLocalOCR")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            mode,
            model_id,
            remote,
            ai,
            credential_version: settings.get("_ocrCredentialVersion").cloned(),
        })
    }

    fn same_configuration(&self, other: &Self) -> bool {
        self.automatic_enabled == other.automatic_enabled
            && self.mode == other.mode
            && self.model_id == other.model_id
            && self.remote == other.remote
            && self.ai == other.ai
            && self.credential_version == other.credential_version
    }
}

#[derive(Clone)]
struct QueuedTask {
    task_id: String,
    image_hash: String,
    records: HashMap<String, OcrTrigger>,
    generation: u64,
    cancel: CancellationToken,
}

struct ActiveTask {
    image_hash: String,
    records: HashMap<String, OcrTrigger>,
    generation: u64,
    visible: bool,
    accepting: bool,
    finished: bool,
    outcome: Option<Result<OcrRecognition, String>>,
    cancel: CancellationToken,
    done: watch::Receiver<bool>,
}

#[derive(Default)]
struct Scheduler {
    queue: VecDeque<QueuedTask>,
    active: HashMap<String, ActiveTask>,
    record_tasks: HashMap<String, String>,
    image_tasks: HashMap<String, String>,
}

struct OcrInner {
    storage: Arc<Storage>,
    settings: SettingsProvider,
    ai_config: AIConfigProvider,
    disable_auto_ocr: DisableAutoOcr,
    events: EventSink,
    models: ModelManager,
    local: LocalWorker,
    http: reqwest::Client,
    current: RwLock<Option<ConfigSnapshot>>,
    lifecycle: Mutex<()>,
    history_writeback: Mutex<()>,
    scheduler: Mutex<Scheduler>,
    maintenance: Mutex<HashSet<String>>,
    auto_preparations: Mutex<HashMap<String, watch::Receiver<Option<Result<(), String>>>>>,
    direct_limit: Arc<Semaphore>,
    generation: AtomicU64,
    idle_generation: AtomicU64,
    paused: AtomicBool,
    shutting_down: AtomicBool,
}

#[derive(Clone)]
pub struct OcrService {
    inner: Arc<OcrInner>,
}

impl OcrService {
    pub fn new(
        model_root: PathBuf,
        ort_runtime: Result<PathBuf, String>,
        storage: Arc<Storage>,
        settings: SettingsProvider,
        ai_config: AIConfigProvider,
        disable_auto_ocr: DisableAutoOcr,
        events: EventSink,
    ) -> Result<Self, String> {
        let models = ModelManager::new(model_root, events.clone())?;
        let local = LocalWorker::new(ort_runtime)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|error| format!("无法初始化 OCR 网络客户端: {error}"))?;
        Ok(Self {
            inner: Arc::new(OcrInner {
                storage,
                settings,
                ai_config,
                disable_auto_ocr,
                events,
                models,
                local,
                http,
                current: RwLock::new(None),
                lifecycle: Mutex::new(()),
                history_writeback: Mutex::new(()),
                scheduler: Mutex::new(Scheduler::default()),
                maintenance: Mutex::new(HashSet::new()),
                auto_preparations: Mutex::new(HashMap::new()),
                direct_limit: Arc::new(Semaphore::new(MAX_DIRECT_TASKS)),
                generation: AtomicU64::new(0),
                idle_generation: AtomicU64::new(0),
                paused: AtomicBool::new(true),
                shutting_down: AtomicBool::new(false),
            }),
        })
    }

    pub async fn initialize(&self) -> Result<(), String> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.current.read().await.is_none() {
            self.inner.models.cleanup_staging().await?;
        }
        self.refresh_configuration_locked().await?;
        self.inner.paused.store(false, Ordering::Release);
        Ok(())
    }

    pub async fn resume(&self) -> Result<(), String> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err("OCR 服务正在关闭".to_owned());
        }
        self.refresh_configuration_locked().await?;
        self.inner.paused.store(false, Ordering::Release);
        self.pump().await;
        Ok(())
    }

    pub async fn suspend(&self) -> Result<(), String> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.inner.paused.store(true, Ordering::Release);
        Ok(())
    }

    pub async fn configuration_changed(&self) -> Result<(), String> {
        let lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err("OCR 服务正在关闭".to_owned());
        }
        let result = self.refresh_configuration_locked().await;
        drop(lifecycle);
        if result.is_ok() {
            self.pump().await;
        }
        result
    }

    pub async fn list_models(&self) -> Result<Vec<OcrModelStatus>, String> {
        self.inner.models.list().await
    }

    pub async fn download_model(&self, id: &str) -> Result<(), String> {
        self.inner.models.download(id).await.map(|_| ())
    }

    pub async fn cancel_download(&self, id: &str) -> Result<(), String> {
        self.inner.models.cancel_download(id).await
    }

    pub async fn delete_model(&self, id: &str) -> Result<(), String> {
        self.maintain_model(id, true).await
    }

    pub async fn redownload_model(&self, id: &str) -> Result<(), String> {
        self.maintain_model(id, false).await
    }

    pub async fn enqueue_record(
        &self,
        content_id: &str,
        trigger: OcrTrigger,
    ) -> Result<(), String> {
        let content_id = content_id.trim();
        if content_id.is_empty() {
            return Err("剪贴板记录 ID 不能为空".to_owned());
        }
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.paused.load(Ordering::Acquire) {
            return Err("OCR 服务已暂停".to_owned());
        }
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err("OCR 服务正在关闭".to_owned());
        }
        let snapshot = self.current_snapshot().await?;
        if trigger == OcrTrigger::Automatic && !snapshot.automatic_enabled {
            return Ok(());
        }
        if snapshot.mode == RecognitionMode::Local
            && self
                .inner
                .maintenance
                .lock()
                .await
                .contains(&snapshot.model_id)
        {
            return Err("本地模型正在维护，请稍后重试".to_owned());
        }
        let _writeback = self.inner.history_writeback.lock().await;
        {
            let mut scheduler = self.inner.scheduler.lock().await;
            if let Some(task_id) = scheduler.record_tasks.get(content_id).cloned() {
                if let Some(task) = scheduler.active.get_mut(&task_id) {
                    if task.accepting {
                        if let Some(existing) = task.records.get_mut(content_id) {
                            *existing = merge_trigger(*existing, trigger);
                        }
                    }
                } else if let Some(task) = scheduler
                    .queue
                    .iter_mut()
                    .find(|task| task.task_id == task_id)
                {
                    if let Some(existing) = task.records.get_mut(content_id) {
                        *existing = merge_trigger(*existing, trigger);
                    }
                }
                return Ok(());
            }
        }
        let image = self
            .inner
            .storage
            .get_clipboard_image(content_id)
            .await?
            .ok_or_else(|| "剪贴板记录不包含图片".to_owned())?;
        let image_hash = digest_hex(&Sha256::digest(&image));
        drop(image);
        let mut scheduler = self.inner.scheduler.lock().await;
        if let Some(task_id) = scheduler.image_tasks.get(&image_hash).cloned() {
            if let Some(task) = scheduler.active.get_mut(&task_id) {
                if !task.cancel.is_cancelled() && task.generation == snapshot.generation {
                    if task.finished {
                        let outcome = task.outcome.clone();
                        drop(scheduler);
                        let result = match outcome {
                            Some(Ok(recognition)) => {
                                if self
                                    .inner
                                    .storage
                                    .update_ocr(content_id, &recognition.text)
                                    .await?
                                {
                                    (self.inner.events)(
                                        "record-ocr-updated",
                                        json!({
                                            "contentId":content_id,
                                            "ocrText":recognition.text,
                                            "timestamp":crate::events::now_ms(),
                                        }),
                                    );
                                }
                                emit_status(&self.inner, content_id, "completed", None);
                                Ok(())
                            }
                            Some(Err(error)) => {
                                emit_status(&self.inner, content_id, "error", Some(&error));
                                Ok(())
                            }
                            None => {
                                emit_status(&self.inner, content_id, "cancelled", None);
                                Ok(())
                            }
                        };
                        drop(_writeback);
                        return result;
                    }
                    if task.accepting {
                        task.records.insert(content_id.to_owned(), trigger);
                        scheduler
                            .record_tasks
                            .insert(content_id.to_owned(), task_id);
                        drop(scheduler);
                        drop(_writeback);
                        emit_status(&self.inner, content_id, "running", None);
                        return Ok(());
                    }
                }
            } else if let Some(task) = scheduler
                .queue
                .iter_mut()
                .find(|task| task.task_id == task_id && task.generation == snapshot.generation)
            {
                task.records.insert(content_id.to_owned(), trigger);
                scheduler
                    .record_tasks
                    .insert(content_id.to_owned(), task_id);
                drop(scheduler);
                drop(_writeback);
                emit_status(&self.inner, content_id, "queued", None);
                return Ok(());
            }
        }
        if scheduler.queue.len() >= MAX_PENDING_TASKS {
            return Err("OCR 等待队列已满，请稍后重试".to_owned());
        }
        let task_id = uuid::Uuid::new_v4().to_string();
        let mut records = HashMap::new();
        records.insert(content_id.to_owned(), trigger);
        let task = QueuedTask {
            task_id: task_id.clone(),
            image_hash: image_hash.clone(),
            records,
            generation: snapshot.generation,
            cancel: CancellationToken::new(),
        };
        scheduler
            .record_tasks
            .insert(content_id.to_owned(), task_id.clone());
        scheduler.image_tasks.insert(image_hash, task_id);
        scheduler.queue.push_back(task);
        drop(scheduler);
        drop(_writeback);
        emit_status(&self.inner, content_id, "queued", None);
        self.pump().await;
        Ok(())
    }

    pub async fn cancel_records(&self, ids: &[String]) -> Result<(), String> {
        let selected = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        if selected.is_empty() {
            return Ok(());
        }
        let mut completions = Vec::new();
        let mut queued = Vec::new();
        let mut active_canceled = Vec::new();
        {
            let _writeback = self.inner.history_writeback.lock().await;
            let mut scheduler = self.inner.scheduler.lock().await;
            let mut retained = VecDeque::with_capacity(scheduler.queue.len());
            while let Some(mut task) = scheduler.queue.pop_front() {
                let removed = task
                    .records
                    .keys()
                    .filter(|content_id| selected.contains(content_id.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                for content_id in removed {
                    task.records.remove(&content_id);
                    if scheduler.record_tasks.get(&content_id) == Some(&task.task_id) {
                        scheduler.record_tasks.remove(&content_id);
                    }
                    queued.push(content_id);
                }
                if task.records.is_empty() {
                    task.cancel.cancel();
                    if scheduler.image_tasks.get(&task.image_hash) == Some(&task.task_id) {
                        scheduler.image_tasks.remove(&task.image_hash);
                    }
                } else {
                    retained.push_back(task);
                }
            }
            scheduler.queue = retained;
            let mut canceled_active = Vec::new();
            let mut canceled_records = Vec::new();
            for (task_id, task) in scheduler.active.iter_mut() {
                let removed = task
                    .records
                    .keys()
                    .filter(|content_id| selected.contains(content_id.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                let removed_any = !removed.is_empty();
                if removed_any {
                    for content_id in removed {
                        task.records.remove(&content_id);
                        canceled_records.push((task_id.clone(), content_id.clone()));
                        active_canceled.push(content_id);
                    }
                }
                if removed_any && task.records.is_empty() {
                    task.cancel.cancel();
                    completions.push(task.done.clone());
                    canceled_active.push((task_id.clone(), task.image_hash.clone()));
                }
            }
            for (task_id, content_id) in canceled_records {
                if scheduler.record_tasks.get(&content_id) == Some(&task_id) {
                    scheduler.record_tasks.remove(&content_id);
                }
            }
            for (task_id, image_hash) in canceled_active {
                if scheduler.image_tasks.get(&image_hash) == Some(&task_id) {
                    scheduler.image_tasks.remove(&image_hash);
                }
            }
        }
        for content_id in queued {
            emit_status(&self.inner, &content_id, "cancelled", None);
        }
        for content_id in active_canceled {
            emit_status(&self.inner, &content_id, "cancelled", None);
        }
        for completion in completions {
            wait_for_completion(completion).await;
        }
        Ok(())
    }

    pub async fn recognize_image(
        &self,
        image: Vec<u8>,
        purpose: OcrPurpose,
        caller_cancel: CancellationToken,
    ) -> Result<OcrRecognition, String> {
        if image.is_empty() || image.len() > MAX_DIRECT_IMAGE_BYTES {
            return Err("OCR 图片大小无效或超过 10 MiB 上限".to_owned());
        }
        let permit = self
            .inner
            .direct_limit
            .clone()
            .try_acquire_owned()
            .map_err(|_| "已有即时 OCR 请求正在处理".to_owned())?;
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.paused.load(Ordering::Acquire) {
            return Err("OCR 服务已暂停".to_owned());
        }
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err("OCR 服务正在关闭".to_owned());
        }
        let snapshot = self.current_snapshot().await?;
        if snapshot.mode == RecognitionMode::Local
            && self
                .inner
                .maintenance
                .lock()
                .await
                .contains(&snapshot.model_id)
        {
            return Err("本地模型正在维护，请稍后重试".to_owned());
        }
        let internal_id = format!("@mcp:{}", uuid::Uuid::new_v4());
        let cancel = CancellationToken::new();
        let (done_tx, done_rx) = watch::channel(false);
        self.inner.scheduler.lock().await.active.insert(
            internal_id.clone(),
            ActiveTask {
                image_hash: String::new(),
                records: HashMap::new(),
                generation: snapshot.generation,
                visible: false,
                accepting: false,
                finished: false,
                outcome: None,
                cancel: cancel.clone(),
                done: done_rx,
            },
        );
        drop(_lifecycle);
        let urgent = matches!(purpose, OcrPurpose::Mcp);
        let cancel_for_watcher = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = caller_cancel.cancelled() => cancel_for_watcher.cancel(),
                _ = cancel_for_watcher.cancelled() => {},
            }
        });
        let service = self.inner.clone();
        let idle_snapshot = snapshot.clone();
        let result = tokio::spawn(async move {
            let _permit = permit;
            let result =
                recognize_with_snapshot(&service, &snapshot, image, &cancel, None, urgent).await;
            let current_generation = service.generation.load(Ordering::Acquire);
            let result = if cancel.is_cancelled() || current_generation != snapshot.generation {
                Err("OCR 请求已取消".to_owned())
            } else {
                result
            };
            service.scheduler.lock().await.active.remove(&internal_id);
            let _ = done_tx.send(true);
            cancel.cancel();
            result
        })
        .await
        .map_err(|error| format!("即时 OCR 任务异常退出: {error}"))??;
        self.schedule_idle_unload(idle_snapshot).await;
        Ok(result)
    }

    pub async fn ai_list_models(
        &self,
        provider: &str,
        config: &Value,
    ) -> Result<Vec<AIModelOption>, String> {
        let provider = AIProvider::parse(provider)?;
        let config = AIConfig::from_value(provider, config, false)?;
        let client = self.ai_management_client()?;
        ai::list_models(&client, provider, &config, &CancellationToken::new()).await
    }

    pub async fn ai_test_connection(
        &self,
        provider: &str,
        config: &Value,
    ) -> Result<AIConnectionTest, String> {
        let provider = AIProvider::parse(provider)?;
        let config = AIConfig::from_value(provider, config, true)?;
        let client = self.ai_management_client()?;
        ai::test_connection(
            &self.inner.storage,
            &client,
            provider,
            &config,
            &CancellationToken::new(),
            &self.inner.events,
        )
        .await
    }

    pub async fn generate_qr_for_record(&self, content_id: &str) -> Result<QRCodeData, String> {
        let item = self
            .inner
            .storage
            .get_history_metadata(content_id)
            .await?
            .ok_or_else(|| "未找到指定的剪贴板项目".to_owned())?;
        let primary_type = item
            .fields
            .get("primaryType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(primary_type, "text" | "html" | "url") {
            return Err("只有文本类型的内容才能生成二维码".to_owned());
        }
        let content = &item.content;
        let mut text = content
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if text.is_empty() {
            if let Some(html) = content.get("html").and_then(Value::as_str) {
                text = strip_html_tags(html);
            }
        }
        if text.trim().is_empty() {
            return Err("文本内容为空，无法生成二维码".to_owned());
        }
        qr::generate(&text)
    }

    pub fn parse_qr(&self, image: &[u8]) -> Option<String> {
        qr::parse(image)
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.shutting_down.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.inner.paused.store(true, Ordering::Release);
        self.cancel_generation(None).await;
        self.inner.models.cancel_all().await;
        self.inner.local.shutdown().await
    }

    pub async fn diagnostics(&self) -> Value {
        let (pending, running, direct) = {
            let scheduler = self.inner.scheduler.lock().await;
            let running = scheduler
                .active
                .values()
                .filter(|task| task.visible)
                .count();
            let direct = scheduler.active.len().saturating_sub(running);
            (scheduler.queue.len(), running, direct)
        };
        let maintenance = self.inner.maintenance.lock().await.len();
        let model_status = self.inner.models.diagnostics().await;
        json!({
            "generation":self.inner.generation.load(Ordering::Acquire),
            "paused":self.inner.paused.load(Ordering::Acquire),
            "shuttingDown":self.inner.shutting_down.load(Ordering::Acquire),
            "initialized":self.inner.current.read().await.is_some(),
            "pendingTasks":pending,
            "runningTasks":running,
            "directTasks":direct,
            "modelMaintenance":maintenance,
            "modelDownloads":model_status.get("downloads").cloned().unwrap_or(Value::Null),
            "models":model_status.get("models").cloned().unwrap_or_else(|| json!([])),
        })
    }

    fn ai_management_client(&self) -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|error| format!("无法初始化 AI OCR 客户端: {error}"))
    }

    async fn current_snapshot(&self) -> Result<ConfigSnapshot, String> {
        self.inner
            .current
            .read()
            .await
            .clone()
            .ok_or_else(|| "OCR 服务未初始化".to_owned())
    }

    async fn refresh_configuration_locked(&self) -> Result<(), String> {
        let settings = (self.inner.settings)().await;
        let settings = match settings {
            Ok(settings) => settings,
            Err(error) => {
                self.cancel_generation(None).await;
                let old = self.inner.current.write().await.take();
                self.inner.generation.fetch_add(1, Ordering::AcqRel);
                if old.is_some_and(|snapshot| snapshot.mode == RecognitionMode::Local) {
                    let _ = self.inner.local.unload(None).await;
                }
                return Err(error);
            }
        };
        let mut candidate = match ConfigSnapshot::from_settings(&settings) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.cancel_generation(None).await;
                let old = self.inner.current.write().await.take();
                self.inner.generation.fetch_add(1, Ordering::AcqRel);
                if old.is_some_and(|snapshot| snapshot.mode == RecognitionMode::Local) {
                    let _ = self.inner.local.unload(None).await;
                }
                return Err(error);
            }
        };
        let old = self.inner.current.read().await.clone();
        if old
            .as_ref()
            .is_some_and(|current| current.same_configuration(&candidate))
        {
            return Ok(());
        }
        let was_paused = self.inner.paused.swap(true, Ordering::AcqRel);
        candidate.generation = self
            .advance_configuration_generation(candidate.automatic_enabled)
            .await;
        if old
            .as_ref()
            .is_some_and(|snapshot| snapshot.mode == RecognitionMode::Local)
        {
            let _ = self.inner.local.unload(None).await;
        }
        *self.inner.current.write().await = Some(candidate.clone());
        let should_prepare = candidate.automatic_enabled
            && candidate.mode == RecognitionMode::Local
            && old.as_ref().is_none_or(|previous| {
                !previous.automatic_enabled
                    || previous.mode != RecognitionMode::Local
                    || previous.model_id != candidate.model_id
            });
        if should_prepare {
            self.start_auto_preparation(candidate.model_id.clone())
                .await;
        }
        self.inner.paused.store(was_paused, Ordering::Release);
        Ok(())
    }

    async fn start_auto_preparation(&self, model_id: String) {
        let inner = self.inner.clone();
        let (sender, receiver) = watch::channel(None);
        self.inner
            .auto_preparations
            .lock()
            .await
            .insert(model_id.clone(), receiver);
        tokio::spawn(async move {
            let result = async {
                let model = get_model(&model_id)?;
                if inner.models.installed(model, true).await?.is_some() {
                    return Ok(());
                }
                inner.models.download(&model_id).await.map(|_| ())
            }
            .await;
            let _ = sender.send(Some(result));
        });
    }

    fn pump(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if self.inner.paused.load(Ordering::Acquire)
                || self.inner.shutting_down.load(Ordering::Acquire)
            {
                return;
            }
            loop {
                let snapshot = match self.current_snapshot().await {
                    Ok(snapshot) => snapshot,
                    Err(_) => return,
                };
                let maintenance = self.inner.maintenance.lock().await;
                if snapshot.mode == RecognitionMode::Local
                    && maintenance.contains(&snapshot.model_id)
                {
                    return;
                }
                {
                    let mut scheduler = self.inner.scheduler.lock().await;
                    if self.inner.paused.load(Ordering::Acquire)
                        || self.inner.shutting_down.load(Ordering::Acquire)
                    {
                        return;
                    }
                    if snapshot.generation != self.inner.generation.load(Ordering::Acquire) {
                        continue;
                    }
                    let running = scheduler
                        .active
                        .values()
                        .filter(|task| task.visible)
                        .count();
                    if running >= MAX_BACKGROUND_TASKS {
                        return;
                    }
                    let Some(task) = scheduler.queue.pop_front() else {
                        return;
                    };
                    if task.generation != snapshot.generation {
                        for content_id in task.records.keys() {
                            if scheduler.record_tasks.get(content_id) == Some(&task.task_id) {
                                scheduler.record_tasks.remove(content_id);
                            }
                        }
                        if scheduler.image_tasks.get(&task.image_hash) == Some(&task.task_id) {
                            scheduler.image_tasks.remove(&task.image_hash);
                        }
                        let content_ids = task.records.keys().cloned().collect::<Vec<_>>();
                        drop(scheduler);
                        for content_id in content_ids {
                            emit_status(&self.inner, &content_id, "cancelled", None);
                        }
                        continue;
                    }
                    let (done_tx, done_rx) = watch::channel(false);
                    scheduler.active.insert(
                        task.task_id.clone(),
                        ActiveTask {
                            image_hash: task.image_hash.clone(),
                            records: task.records.clone(),
                            generation: task.generation,
                            visible: true,
                            accepting: true,
                            finished: false,
                            outcome: None,
                            cancel: task.cancel.clone(),
                            done: done_rx,
                        },
                    );
                    let content_ids = task.records.keys().cloned().collect::<Vec<_>>();
                    drop(scheduler);
                    for content_id in content_ids {
                        emit_status(&self.inner, &content_id, "running", None);
                    }
                    let service = self.clone();
                    let snapshot = snapshot.clone();
                    tokio::spawn(async move {
                        service.run_history_task(task, snapshot, done_tx).await;
                    });
                }
                drop(maintenance);
            }
        })
    }

    async fn run_history_task(
        &self,
        task: QueuedTask,
        snapshot: ConfigSnapshot,
        done: watch::Sender<bool>,
    ) {
        let result = process_history_task(&self.inner, &task, &snapshot).await;
        let writeback = self.inner.history_writeback.lock().await;
        let (content_ids, state, error) = {
            let mut scheduler = self.inner.scheduler.lock().await;
            let process_error = result.err();
            let mut active = scheduler.active.remove(&task.task_id);
            if let Some(active) = active.as_mut() {
                if !active.finished {
                    active.accepting = false;
                    active.finished = true;
                    active.outcome = process_error.clone().map(Err);
                }
            }
            let finished = active.as_ref().is_some_and(|active| active.finished);
            let content_ids = if let Some(active) = active {
                for content_id in active.records.keys() {
                    if scheduler.record_tasks.get(content_id) == Some(&task.task_id) {
                        scheduler.record_tasks.remove(content_id);
                    }
                }
                active.records.into_keys().collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if scheduler.image_tasks.get(&task.image_hash) == Some(&task.task_id) {
                scheduler.image_tasks.remove(&task.image_hash);
            }
            let _ = done.send(true);
            let generation_changed =
                snapshot.generation != self.inner.generation.load(Ordering::Acquire);
            let (state, error) = match process_error {
                Some(_error) if task.cancel.is_cancelled() || generation_changed => {
                    ("cancelled", None)
                }
                Some(error) => ("error", Some(error)),
                None if (task.cancel.is_cancelled() || generation_changed) && !finished => {
                    ("cancelled", None)
                }
                None => ("completed", None),
            };
            (content_ids, state, error)
        };
        for content_id in content_ids {
            emit_status(&self.inner, &content_id, state, error.as_deref());
        }
        drop(writeback);
        self.pump().await;
        self.schedule_idle_unload(snapshot).await;
    }

    async fn schedule_idle_unload(&self, snapshot: ConfigSnapshot) {
        if snapshot.mode != RecognitionMode::Local {
            return;
        }
        let idle_generation = self.inner.idle_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let service = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(LOCAL_IDLE_TIMEOUT).await;
            if service.inner.idle_generation.load(Ordering::Acquire) != idle_generation {
                return;
            }
            let current = service.current_snapshot().await.ok();
            if current
                .as_ref()
                .is_none_or(|current| current.generation != snapshot.generation)
            {
                return;
            }
            let scheduler = service.inner.scheduler.lock().await;
            if !scheduler.active.is_empty() || !scheduler.queue.is_empty() {
                return;
            }
            drop(scheduler);
            let _ = service.inner.local.unload(Some(snapshot.model_id)).await;
        });
    }

    async fn cancel_generation(&self, generation: Option<u64>) {
        let mut queued = Vec::new();
        let mut completions = Vec::new();
        {
            let _writeback = self.inner.history_writeback.lock().await;
            let mut scheduler = self.inner.scheduler.lock().await;
            let mut retained = VecDeque::with_capacity(scheduler.queue.len());
            while let Some(task) = scheduler.queue.pop_front() {
                if generation.is_none_or(|generation| generation == task.generation) {
                    task.cancel.cancel();
                    for content_id in task.records.keys() {
                        if scheduler.record_tasks.get(content_id) == Some(&task.task_id) {
                            scheduler.record_tasks.remove(content_id);
                        }
                        queued.push(content_id.clone());
                    }
                    if scheduler.image_tasks.get(&task.image_hash) == Some(&task.task_id) {
                        scheduler.image_tasks.remove(&task.image_hash);
                    }
                } else {
                    retained.push_back(task);
                }
            }
            scheduler.queue = retained;
            let mut canceled_active = Vec::new();
            for (task_id, task) in scheduler.active.iter_mut() {
                if generation.is_none_or(|generation| generation == task.generation) {
                    task.cancel.cancel();
                    completions.push(task.done.clone());
                    canceled_active.push((
                        task_id.clone(),
                        task.records.keys().cloned().collect::<Vec<_>>(),
                        task.image_hash.clone(),
                    ));
                }
            }
            for (task_id, content_ids, image_hash) in canceled_active {
                for content_id in content_ids {
                    if scheduler.record_tasks.get(&content_id) == Some(&task_id) {
                        scheduler.record_tasks.remove(&content_id);
                    }
                }
                if scheduler.image_tasks.get(&image_hash) == Some(&task_id) {
                    scheduler.image_tasks.remove(&image_hash);
                }
            }
        }
        for content_id in queued {
            emit_status(&self.inner, &content_id, "cancelled", None);
        }
        for completion in completions {
            wait_for_completion(completion).await;
        }
    }

    async fn wait_for_generation(&self, generation: u64) {
        loop {
            let completions = self
                .inner
                .scheduler
                .lock()
                .await
                .active
                .values()
                .filter(|task| task.generation == generation)
                .map(|task| task.done.clone())
                .collect::<Vec<_>>();
            if completions.is_empty() {
                return;
            }
            for completion in completions {
                wait_for_completion(completion).await;
            }
        }
    }

    async fn advance_configuration_generation(&self, automatic_enabled: bool) -> u64 {
        let writeback = self.inner.history_writeback.lock().await;
        let generation = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let mut canceled_queued = Vec::new();
        let mut completions = Vec::new();
        {
            let mut scheduler = self.inner.scheduler.lock().await;
            let mut retained = VecDeque::with_capacity(scheduler.queue.len());
            while let Some(mut task) = scheduler.queue.pop_front() {
                if !automatic_enabled {
                    let automatic_ids = task
                        .records
                        .iter()
                        .filter_map(|(id, trigger)| {
                            (*trigger == OcrTrigger::Automatic).then_some(id.clone())
                        })
                        .collect::<Vec<_>>();
                    for content_id in automatic_ids {
                        task.records.remove(&content_id);
                        if scheduler.record_tasks.get(&content_id) == Some(&task.task_id) {
                            scheduler.record_tasks.remove(&content_id);
                        }
                        canceled_queued.push(content_id);
                    }
                }
                if task.records.is_empty() {
                    task.cancel.cancel();
                    if scheduler.image_tasks.get(&task.image_hash) == Some(&task.task_id) {
                        scheduler.image_tasks.remove(&task.image_hash);
                    }
                } else {
                    task.generation = generation;
                    retained.push_back(task);
                }
            }
            scheduler.queue = retained;

            let mut canceled_active = Vec::new();
            for (task_id, task) in scheduler.active.iter_mut() {
                completions.push(task.done.clone());
                if task.finished {
                    continue;
                }
                task.cancel.cancel();
                canceled_active.push((
                    task_id.clone(),
                    task.image_hash.clone(),
                    task.records.keys().cloned().collect::<Vec<_>>(),
                ));
            }
            for (task_id, image_hash, content_ids) in canceled_active {
                for content_id in content_ids {
                    if scheduler.record_tasks.get(&content_id) == Some(&task_id) {
                        scheduler.record_tasks.remove(&content_id);
                    }
                }
                if scheduler.image_tasks.get(&image_hash) == Some(&task_id) {
                    scheduler.image_tasks.remove(&image_hash);
                }
            }
        }
        drop(writeback);
        for content_id in canceled_queued {
            emit_status(&self.inner, &content_id, "cancelled", None);
        }
        for completion in completions {
            wait_for_completion(completion).await;
        }
        generation
    }

    async fn maintain_model(&self, id: &str, delete: bool) -> Result<(), String> {
        let model_id = get_model(id)?.id.clone();
        let mut lifecycle = Some(self.inner.lifecycle.lock().await);
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err("OCR 服务正在关闭".to_owned());
        }
        {
            let mut maintenance = self.inner.maintenance.lock().await;
            if !maintenance.insert(model_id.clone()) {
                return Err("模型正在维护，请稍后重试".to_owned());
            }
            // Synchronize with a pump that passed its maintenance check before this marker.
            let scheduler = self.inner.scheduler.lock().await;
            drop(scheduler);
        }
        let current = self.inner.current.read().await.clone();
        let selected = current.as_ref().is_some_and(|snapshot| {
            snapshot.mode == RecognitionMode::Local && snapshot.model_id == model_id
        });
        let selected_generation = current.as_ref().map(|snapshot| snapshot.generation);
        let must_disable_auto = delete
            && selected
            && current
                .as_ref()
                .is_some_and(|snapshot| snapshot.automatic_enabled);
        let mut result = Ok(());
        if selected {
            if delete {
                self.cancel_generation(selected_generation).await;
            } else if let Some(generation) = selected_generation {
                self.wait_for_generation(generation).await;
            }
            result = self.inner.local.unload(Some(model_id.clone())).await;
        }
        if result.is_ok() && must_disable_auto {
            drop(lifecycle.take());
            result = (self.inner.disable_auto_ocr)().await;
            lifecycle = Some(self.inner.lifecycle.lock().await);
            if result.is_ok() {
                let latest = self.inner.current.read().await.clone();
                let still_enabled = latest.as_ref().is_some_and(|snapshot| {
                    snapshot.mode == RecognitionMode::Local
                        && snapshot.model_id == model_id
                        && snapshot.automatic_enabled
                });
                if still_enabled {
                    result = Err("关闭自动 OCR 后配置仍在使用该模型，无法删除".to_owned());
                }
            }
        }
        if result.is_ok() {
            result = if delete {
                self.inner.models.remove(&model_id).await
            } else {
                self.inner.models.redownload(&model_id).await.map(|_| ())
            };
        }
        drop(lifecycle.take());
        self.inner.maintenance.lock().await.remove(&model_id);
        if result.is_ok() || (selected && !delete) {
            self.pump().await;
        }
        result
    }
}

async fn wait_for_completion(mut receiver: watch::Receiver<bool>) {
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

fn emit_status(inner: &OcrInner, content_id: &str, state: &str, error: Option<&str>) {
    let mut payload = serde_json::Map::new();
    payload.insert("contentId".to_owned(), Value::String(content_id.to_owned()));
    payload.insert("state".to_owned(), Value::String(state.to_owned()));
    if let Some(error) = error {
        payload.insert("error".to_owned(), Value::String(error.to_owned()));
    }
    (inner.events)("ocr:task-status", Value::Object(payload));
}

fn digest_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

fn strip_html_tags(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' if !in_tag => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output
}

pub use ai::{AIConnectionTest, AIModelOption};
pub use qr::QRCodeData;
