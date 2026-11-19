use std::{collections::{BTreeMap, BTreeSet, HashMap, VecDeque}, future::Future, panic::AssertUnwindSafe, path::PathBuf, sync::{Arc, Weak, atomic::{AtomicBool, Ordering}}, time::Duration};
use futures_util::FutureExt;
use serde::Serialize;
use serde_json::Value;
use super_paste_format::{AiProvider, ConfigChange, ConfigField, CredentialId, Digest, ImageMetadata, OcrMode, RecordId};
use tokio::{runtime::Handle, sync::{Mutex, Notify, RwLock, Semaphore, watch}, time::Instant};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use uuid::Uuid;
use crate::{Error, Result, configuration::{ConfigurationEffects, ConfigurationMutation, ConfigurationService, ConfigurationState}, events::{AppEvent, EventBus, OcrTaskState, OcrTaskStatus, RecordChange}, images::{EncodedImage, PixelBuffer, ImageService, ImageSource}, storage::Storage};
use super::{AiRequestConfig, AiService, InstalledModel, ModelManager, ModelsDiagnostics, OcrAdmission, OcrTicket, catalog, local::LocalWorker, remote::{self, RemoteConfig}, statuses::TaskStatuses};

const WORKERS: usize = 3;
const MAX_PENDING: usize = 2000;
const LOCAL_IDLE: Duration = Duration::from_secs(5 * 60);
type Outcome = Result<Arc<OcrRecognition>>;
type Completion<T> = watch::Receiver<Option<Result<T>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrTrigger { Automatic, Manual }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrRecognition {
    pub text: Arc<str>, pub confidence: f64, pub mode: OcrMode,
    #[serde(skip_serializing_if = "Option::is_none")] pub details: Option<Arc<Value>>,
}
#[derive(Clone)]
pub struct OcrTaskHandle { pub record: RecordId, completion: watch::Receiver<Option<Outcome>> }
impl OcrTaskHandle {
    /// Detaches this waiter only; explicit record cancellation is a separate operation.
    pub async fn wait(self, cancel: &CancellationToken) -> Outcome { await_result(self.completion, cancel).await }
}
async fn await_result<T: Clone + Send + Sync>(mut completion: Completion<T>, cancel: &CancellationToken) -> Result<T> {
        loop {
            if let Some(result) = completion.borrow().clone() { return result; }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                changed = completion.changed() => changed.map_err(|_| Error::Task("OCR 任务异常退出".into()))?,
            }
        }
}
struct Snapshot { state: ConfigurationState, revision: u64, model: String, remote: Option<Result<Arc<RemoteConfig>>>, cancel: CancellationToken }
impl Snapshot {
    fn new(state: ConfigurationState, root: &CancellationToken) -> Result<Arc<Self>> {
        let settings = &state.snapshot.value.settings;
        let model = if settings.ocr_local_model.trim().is_empty() { catalog::default_model_id().map_err(platform_error)?.to_owned() } else { settings.ocr_local_model.clone() };
        let remote = (settings.ocr_mode == OcrMode::Remote).then(|| RemoteConfig::new(settings));
        Ok(Arc::new(Self { revision: state.ocr_revision(), state, model, remote, cancel: root.child_token() }))
    }
    fn local_model(&self) -> Option<&str> { (self.state.snapshot.value.settings.ocr_mode == OcrMode::Local).then_some(self.model.as_str()) }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Queued, Running, Finishing }
#[derive(Clone, Copy, PartialEq, Eq)]
enum LocalRequest { Automatic, Manual, Preparation }
struct RecordTask { ticket: OcrTicket, trigger: OcrTrigger, completion: watch::Sender<Option<Outcome>> }
pub(crate) struct CapturedImage { pub original: Arc<EncodedImage>, pub pixels: Option<Arc<PixelBuffer>> }
impl CapturedImage {
    fn weak(&self) -> WeakInput { WeakInput { original: Arc::downgrade(&self.original), pixels: self.pixels.as_ref().map_or_else(Weak::new, Arc::downgrade) } }
    fn source(self, mode: OcrMode) -> ImageSource {
        if mode == OcrMode::Local || (matches!(mode, OcrMode::Openai | OcrMode::Claude) && !matches!(self.original.format(), Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif | image::ImageFormat::WebP))) {
            if let Some(pixels) = self.pixels { return ImageSource::Pixels(pixels); }
        }
        ImageSource::Encoded(self.original)
    }
}
#[derive(Default)]
struct WeakInput { original: Weak<EncodedImage>, pixels: Weak<PixelBuffer> }
impl WeakInput {
    fn source(&self, mode: OcrMode, metadata: &ImageMetadata) -> ImageSource {
        if mode == OcrMode::Local { if let Some(pixels) = self.pixels.upgrade() { return ImageSource::Pixels(pixels); } }
        if let Some(original) = self.original.upgrade() { return CapturedImage { original, pixels: self.pixels.upgrade() }.source(mode); }
        ImageSource::Stored(metadata.clone())
    }
}
struct Job {
    image: Digest, metadata: ImageMetadata, input: WeakInput, handoff: Option<CapturedImage>, snapshot: Arc<Snapshot>, phase: Phase, cancel: CancellationToken,
    records: BTreeMap<RecordId, RecordTask>, done: watch::Sender<bool>,
}
struct Direct { snapshot: Arc<Snapshot>, cancel: CancellationToken, done: watch::Receiver<bool> }
#[derive(Default)]
struct Scheduler {
    queue: VecDeque<Uuid>, jobs: HashMap<Uuid, Job>, records: HashMap<RecordId, Uuid>,
    images: HashMap<(Digest, u64), Uuid>, direct: HashMap<Uuid, Direct>,
    statuses: TaskStatuses,
}
#[derive(Clone)]
struct Run { id: Uuid, snapshot: Arc<Snapshot>, source: ImageSource, cancel: CancellationToken }

pub struct OcrService {
    storage: Arc<Storage>, configuration: Arc<ConfigurationService>, images: Arc<ImageService>,
    models: ModelManager, local: LocalWorker, ai: Arc<AiService>, http: reqwest::Client, events: EventBus,
    current: RwLock<Arc<Snapshot>>, lifecycle: Mutex<()>, writeback: Mutex<()>, scheduler: Mutex<Scheduler>,
    maintenance: Mutex<BTreeSet<String>>, verified: Mutex<Option<InstalledModel>>, direct_limit: Arc<Semaphore>, ai_limits: [Arc<Semaphore>; 2],
    notify: Notify, activity: watch::Sender<Instant>, root: CancellationToken, tasks: TaskTracker, runtime: Handle,
    shutdown: Mutex<Option<Completion<()>>>, suspended: AtomicBool,
}
pub struct OcrDiagnostics { pub revision: u64, pub queued: usize, pub running: usize, pub direct: usize, pub maintenance: usize, pub local_model_loaded: bool, pub suspended: bool, pub closing: bool, pub models: ModelsDiagnostics }

impl OcrService {
    pub async fn new(storage: Arc<Storage>, configuration: Arc<ConfigurationService>, images: Arc<ImageService>, models: ModelManager, ort_runtime: std::result::Result<PathBuf, String>, events: EventBus, runtime: Handle) -> Result<Arc<Self>> {
        let root = CancellationToken::new();
        let current = Snapshot::new(configuration.snapshot(), &root)?;
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(20)).redirect(reqwest::redirect::Policy::limited(5))
            .build().map_err(|error| Error::Platform(format!("无法初始化 OCR HTTP 客户端: {}", error.without_url()).into()))?;
        let ai = AiService::new(storage.clone(), images.clone(), events.clone())?;
        let local = LocalWorker::new(ort_runtime, images.clone(), runtime.clone()).map_err(platform_error)?;
        let (activity, _) = watch::channel(Instant::now());
        let service = Arc::new(Self { storage, configuration, images, models, local, ai, http, events, current: RwLock::new(current.clone()),
            lifecycle: Mutex::new(()), writeback: Mutex::new(()), scheduler: Mutex::new(Scheduler::default()), maintenance: Mutex::new(BTreeSet::new()),
            verified: Mutex::new(None), direct_limit: Arc::new(Semaphore::new(1)), ai_limits: [Arc::new(Semaphore::new(1)), Arc::new(Semaphore::new(1))], notify: Notify::new(), activity, root, tasks: TaskTracker::new(), runtime, shutdown: Mutex::new(None), suspended: AtomicBool::new(false) });
        for _ in 0..WORKERS { let worker = service.clone(); service.tasks.spawn_on(async move { worker.worker().await }, &service.runtime); }
        let watcher = service.clone();
        service.tasks.spawn_on(async move { watcher.watch_configuration().await }, &service.runtime);
        let idle = service.clone();
        service.tasks.spawn_on(async move { idle.idle_loop().await }, &service.runtime);
        service.start_preparation(current);
        Ok(service)
    }
    pub fn ai(&self) -> Arc<AiService> { self.ai.clone() }
    /// Pause queue dispatch without discarding queued records or cancelling
    /// already running recognition. Explicit image requests remain independent.
    pub fn set_suspended(&self, suspended: bool) {
        self.suspended.store(suspended, Ordering::Release);
        if !suspended { self.notify.notify_waiters(); }
    }
    pub async fn list_ai_models(self: &Arc<Self>, provider: AiProvider, expected: ConfigurationState, cancel: CancellationToken) -> Result<Arc<Vec<super_paste_format::AiModelOption>>> {
        self.ai_operation(provider, expected, cancel, false, |ai, config, cancel| async move { ai.list_models(config, &cancel).await }).await
    }
    pub async fn test_ai_connection(self: &Arc<Self>, provider: AiProvider, expected: ConfigurationState, cancel: CancellationToken) -> Result<Arc<super_paste_format::AiConnectionTest>> {
        self.ai_operation(provider, expected, cancel, true, |ai, config, cancel| async move { ai.test_connection(config, &cancel).await }).await
    }
    async fn ai_operation<T: Send + Sync + 'static, F, Fut>(self: &Arc<Self>, provider: AiProvider, expected: ConfigurationState, caller: CancellationToken, test: bool, operation: F) -> Result<Arc<T>>
    where F: FnOnce(Arc<AiService>, Arc<AiRequestConfig>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        self.check_open()?;
        let permit = self.ai_limits[usize::from(test)].clone().try_acquire_owned().map_err(|_| Error::ResourceLimit(if test { "已有 AI 连通性测试正在处理" } else { "已有模型列表请求正在处理" }.into()))?;
        let _lifecycle = self.lifecycle.lock().await;
        self.check_open()?;
        if caller.is_cancelled() { return Err(Error::Cancelled); }
        let cancel = self.root.child_token();
        let (sender, receiver) = watch::channel(None);
        let service = self.clone();
        self.tasks.spawn_on(async move {
            let _permit = permit;
            let work = async {
                let (field, credential) = match provider {
                    AiProvider::Openai => (ConfigField::OpenaiConfig, CredentialId::OpenaiApiKey),
                    AiProvider::Claude => (ConfigField::ClaudeConfig, CredentialId::ClaudeApiKey),
                };
                let guarded_fields = [field, credential.field()];
                let prepared = service.configuration.guarded_credential(credential, &expected, &guarded_fields);
                let (configuration, key) = tokio::select! { biased; _ = cancel.cancelled() => return Err(Error::Cancelled), result = prepared => result? };
                let profile = match provider { AiProvider::Openai => &configuration.settings.ocr_ai_configs.openai, AiProvider::Claude => &configuration.settings.ocr_ai_configs.claude };
                let config = AiRequestConfig::new(provider, profile, key)?;
                if cancel.is_cancelled() { return Err(Error::Cancelled); }
                operation(service.ai.clone(), config, cancel.clone()).await.map(Arc::new)
            };
            tokio::pin!(work);
            // Cancellation reaches the HTTP request, then the same future drains
            // its usage write; dropping it here would lose billed test records.
            let outcome = async { tokio::select! { biased; _ = caller.cancelled() => { cancel.cancel(); work.await }, result = &mut work => result } };
            let result = AssertUnwindSafe(outcome).catch_unwind().await.unwrap_or_else(|_| Err(Error::Task("AI 接口任务异常退出".into())));
            if let Err(error) = &result { if !matches!(error, Error::Cancelled) { tracing::warn!(%error, "AI 接口操作失败"); } }
            sender.send_replace(Some(result));
        }, &self.runtime);
        drop(_lifecycle);
        await_result(receiver, &self.root).await
    }
    pub async fn list_models(&self, cancel: &CancellationToken) -> Result<Vec<catalog::OcrModelStatus>> {
        self.check_open()?;
        self.models.list(cancel).await
    }
    pub async fn download_model(&self, id: &str) -> Result<()> { self.models.download(id).await.map(|_| ()).map_err(platform_error) }
    pub async fn cancel_download(&self, id: &str) -> Result<()> { self.models.cancel_download(id).await.map_err(platform_error) }

    pub async fn enqueue_record(self: &Arc<Self>, record: RecordId, trigger: OcrTrigger) -> Result<Option<OcrTaskHandle>> {
        self.enqueue(record, trigger, None).await
    }
    /// Recover pending and recent terminal states without loading images or results.
    /// The same deadlines travel through events and snapshots, so reopening a view
    /// cannot extend a failure's lifetime.
    pub async fn task_statuses(&self, records: &[RecordId]) -> Vec<OcrTaskStatus> {
        self.scheduler.lock().await.statuses.snapshot(records, std::time::Instant::now())
    }
    pub(crate) async fn enqueue_captured(self: &Arc<Self>, record: RecordId, source: CapturedImage) -> Result<Option<OcrTaskHandle>> {
        self.enqueue(record, OcrTrigger::Automatic, Some(source)).await
    }
    async fn enqueue(self: &Arc<Self>, record: RecordId, trigger: OcrTrigger, mut source: Option<CapturedImage>) -> Result<Option<OcrTaskHandle>> {
        let mut updates = self.configuration.subscribe();
        loop {
            self.synchronize().await?;
            let lifecycle = self.lifecycle.lock().await;
            self.check_open()?;
            let snapshot = self.current.read().await.clone();
            if trigger == OcrTrigger::Automatic && !snapshot.state.snapshot.value.settings.enable_local_ocr { return Ok(None); }
            self.check_maintenance(&snapshot).await?;
            let mut scheduler = self.scheduler.lock().await;
            if let Some(task) = scheduler.records.get(&record).and_then(|id| scheduler.jobs.get(id)) {
                if task.phase != Phase::Finishing && !task.cancel.is_cancelled() {
                    let id = scheduler.records[&record];
                    let entry = scheduler.jobs.get_mut(&id).and_then(|job| job.records.get_mut(&record)).ok_or_else(|| Error::Task("OCR 调度索引不一致".into()))?;
                    if trigger == OcrTrigger::Manual { entry.trigger = trigger; }
                    return Ok(Some(OcrTaskHandle { record, completion: entry.completion.subscribe() }));
                }
            }
            if scheduler.records.len() >= MAX_PENDING { return Err(Error::ResourceLimit("OCR 等待队列已满".into())); }
            let (revision, image) = self.storage.ocr_input(record).await?.ok_or_else(|| Error::Invalid("剪贴板记录不包含图片".into()))?;
            let ticket = match self.storage.begin_ocr(record, revision, snapshot.revision).await? {
                OcrAdmission::Started(ticket) => ticket,
                OcrAdmission::Missing => return Err(Error::NotFound),
                OcrAdmission::ConfigurationChanged => {
                    drop(scheduler); drop(lifecycle);
                    if updates.borrow().ocr_revision() == snapshot.revision {
                        tokio::select! { _ = self.root.cancelled() => return Err(Error::Cancelled), changed = updates.changed() => changed.map_err(|_| Error::Task("配置服务已关闭".into()))? }
                    }
                    continue;
                }
            };
            let (completion, receiver) = watch::channel(None);
            let record_task = RecordTask { ticket, trigger, completion };
            let key = (image.id, snapshot.revision);
            let existing = scheduler.images.get(&key).copied().filter(|id| scheduler.jobs.get(id).is_some_and(|job| job.phase != Phase::Finishing && !job.cancel.is_cancelled()));
            let id = if let Some(id) = existing {
                let admitted = scheduler.jobs.len() <= WORKERS;
                let job = scheduler.jobs.get_mut(&id).ok_or_else(|| Error::Task("OCR 图片索引不一致".into()))?;
                if let Some(source) = source.take() {
                    job.input = source.weak();
                    if admitted && job.phase == Phase::Queued { job.handoff = Some(source); }
                }
                job.records.insert(record, record_task);
                let state = if job.phase == Phase::Queued { OcrTaskState::Queued } else { OcrTaskState::Running };
                self.status(&mut scheduler.statuses, ticket, state);
                id
            } else {
                let id = Uuid::new_v4();
                let (done, _) = watch::channel(false);
                let input = source.as_ref().map_or_else(WeakInput::default, CapturedImage::weak);
                let admitted = scheduler.jobs.len() < WORKERS;
                let job = Job { image: image.id, metadata: image, input, handoff: if admitted { source.take() } else { None }, snapshot: snapshot.clone(), phase: Phase::Queued,
                    cancel: snapshot.cancel.child_token(), records: BTreeMap::from([(record, record_task)]), done };
                scheduler.jobs.insert(id, job); scheduler.images.insert(key, id); scheduler.queue.push_back(id);
                self.status(&mut scheduler.statuses, ticket, OcrTaskState::Queued);
                id
            };
            scheduler.records.insert(record, id);
            drop(scheduler); drop(lifecycle);
            self.activity.send_replace(Instant::now()); self.notify.notify_one();
            return Ok(Some(OcrTaskHandle { record, completion: receiver }));
        }
    }

    async fn worker(self: Arc<Self>) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Register before inspecting work: a wake between the queue check
            // and awaiting must not leave all suspended workers asleep.
            notified.as_mut().enable();
            if self.root.is_cancelled() { return; }
            if let Err(error) = self.synchronize().await { self.events.publish(AppEvent::OcrServiceFailed(error)); }
            let run = {
                let _lifecycle = self.lifecycle.lock().await;
                let maintenance = self.maintenance.lock().await;
                let mut scheduler = self.scheduler.lock().await;
                let selected = if self.suspended.load(Ordering::Acquire) { None } else { scheduler.queue.iter().position(|id| scheduler.jobs.get(id).is_some_and(|job| job.snapshot.local_model().is_none_or(|id| !maintenance.contains(id)))) };
                selected.and_then(|position| scheduler.queue.remove(position)).and_then(|id| scheduler.jobs.get_mut(&id).map(|job| {
                    job.phase = Phase::Running;
                    let mode = job.snapshot.state.snapshot.value.settings.ocr_mode;
                    let source = job.handoff.take().map(|image| image.source(mode)).unwrap_or_else(|| job.input.source(mode, &job.metadata));
                    Run { id, snapshot: job.snapshot.clone(), source, cancel: job.cancel.clone() }
                }))
            };
            if let Some(run) = run {
                let result = AssertUnwindSafe(self.execute(&run)).catch_unwind().await.unwrap_or_else(|_| Err(Error::Task("OCR 工作任务异常退出".into())));
                self.finish(run, result).await;
            } else { tokio::select! { _ = self.root.cancelled() => return, _ = notified => {} } }
        }
    }
    async fn execute(&self, run: &Run) -> Outcome {
        let mut scheduler = self.scheduler.lock().await;
        let Scheduler { jobs, statuses, .. } = &mut *scheduler;
        let job = jobs.get_mut(&run.id).ok_or(Error::Cancelled)?;
        let mut automatic = false;
        for entry in job.records.values_mut() {
            if entry.ticket.configuration_revision != run.snapshot.revision {
                entry.ticket = match self.storage.begin_ocr(entry.ticket.record, entry.ticket.content_revision, run.snapshot.revision).await? {
                    OcrAdmission::Started(ticket) => ticket,
                    OcrAdmission::ConfigurationChanged => return Err(Error::Conflict("OCR 配置".into())),
                    OcrAdmission::Missing => continue,
                };
            }
            self.status(statuses, entry.ticket, OcrTaskState::Running);
            automatic |= entry.trigger == OcrTrigger::Automatic;
        }
        drop(scheduler);
        self.recognize(&run.snapshot, run.source.clone(), &run.cancel, automatic, false).await.map(Arc::new)
    }
    async fn finish(&self, run: Run, outcome: Outcome) {
        let _writeback = self.writeback.lock().await;
        let records = {
            let mut scheduler = self.scheduler.lock().await;
            let Some(job) = scheduler.jobs.get_mut(&run.id) else { return };
            job.phase = Phase::Finishing;
            let records = std::mem::take(&mut job.records);
            for record in records.keys() { if scheduler.records.get(record) == Some(&run.id) { scheduler.records.remove(record); } }
            records
        };
        for (_, entry) in records {
            let result = if run.cancel.is_cancelled() || self.configuration.snapshot().ocr_revision() != run.snapshot.revision { Err(Error::Cancelled) } else { outcome.clone() };
            let mut result = match result {
                Ok(recognized) => match self.storage.update_ocr(entry.ticket, recognized.text.clone()).await {
                    Ok(Some(revision)) => { self.events.publish(AppEvent::HistoryChanged { ids: Arc::from([entry.ticket.record]), change: RecordChange::Ocr { revision } }); Ok(recognized) }
                    Ok(None) => Err(Error::Cancelled), Err(error) => Err(error),
                },
                Err(error) => Err(error),
            };
            if result.is_err() {
                match self.storage.cancel_ocr(entry.ticket).await {
                    Ok(false) => result = Err(Error::Cancelled),
                    Ok(true) => {},
                    Err(error) => self.events.publish(AppEvent::OcrServiceFailed(error)),
                }
            }
            {
                let mut scheduler = self.scheduler.lock().await;
                self.status(&mut scheduler.statuses, entry.ticket, match &result { Ok(_) => OcrTaskState::Completed, Err(Error::Cancelled) => OcrTaskState::Cancelled, Err(error) => OcrTaskState::Failed(error.clone()) });
            }
            entry.completion.send_replace(Some(result));
        }
        let mut scheduler = self.scheduler.lock().await;
        if let Some(job) = scheduler.jobs.remove(&run.id) {
            let key = (job.image, job.snapshot.revision);
            if scheduler.images.get(&key) == Some(&run.id) { scheduler.images.remove(&key); }
            job.done.send_replace(true);
        }
        self.activity.send_replace(Instant::now());
    }

    pub async fn cancel_records(&self, records: &[RecordId]) -> Result<()> {
        self.cancel_records_inner(records, false).await
    }
    pub(crate) async fn forget_records(&self, records: &[RecordId]) -> Result<()> {
        self.cancel_records_inner(records, true).await
    }
    async fn cancel_records_inner(&self, records: &[RecordId], forget: bool) -> Result<()> {
        let _writeback = self.writeback.lock().await;
        let detached = {
            let mut scheduler = self.scheduler.lock().await;
            let mut detached = Vec::new();
            let mut empty = Vec::new();
            for record in records {
                if forget { scheduler.statuses.remove(*record); }
                if let Some(id) = scheduler.records.remove(record) {
                    if let Some(job) = scheduler.jobs.get_mut(&id) {
                        if let Some(entry) = job.records.remove(record) { detached.push(entry); }
                        if job.records.is_empty() { job.cancel.cancel(); if job.phase == Phase::Queued { empty.push(id); } }
                    }
                }
            }
            for id in empty {
                if let Some(job) = scheduler.jobs.remove(&id) {
                    let key = (job.image, job.snapshot.revision);
                    if scheduler.images.get(&key) == Some(&id) { scheduler.images.remove(&key); }
                    job.done.send_replace(true);
                }
            }
            let live: BTreeSet<_> = scheduler.jobs.keys().copied().collect();
            scheduler.queue.retain(|id| live.contains(id));
            detached
        };
        let mut failure = None;
        for entry in detached {
            if let Err(error) = self.storage.cancel_ocr(entry.ticket).await { failure.get_or_insert(error); }
            {
                let mut scheduler = self.scheduler.lock().await;
                self.status(&mut scheduler.statuses, entry.ticket, OcrTaskState::Cancelled);
                if forget { scheduler.statuses.remove_generation(entry.ticket.record, entry.ticket.generation); }
            }
            entry.completion.send_replace(Some(Err(Error::Cancelled)));
        }
        match failure { Some(error) => Err(error), None => Ok(()) }
    }

    pub async fn recognize_image(self: &Arc<Self>, image: Arc<EncodedImage>, caller: CancellationToken) -> Outcome {
        if image.bytes().is_empty() || image.bytes().len() > 10 * 1024 * 1024 { return Err(Error::Invalid("OCR 图片大小无效或超过 10 MiB 上限".into())); }
        self.recognize_source(ImageSource::Encoded(image), caller).await
    }
    /// Protocol adapters validate their encoded limits before passing shared pixels.
    /// Local OCR therefore consumes an already validated buffer without re-encoding it.
    pub async fn recognize_source(self: &Arc<Self>, source: ImageSource, caller: CancellationToken) -> Outcome {
        let permit = self.direct_limit.clone().try_acquire_owned().map_err(|_| Error::ResourceLimit("已有即时 OCR 请求正在处理".into()))?;
        self.synchronize().await?;
        let _lifecycle = self.lifecycle.lock().await;
        self.check_open()?;
        let snapshot = self.current.read().await.clone();
        self.check_maintenance(&snapshot).await?;
        let cancel = snapshot.cancel.child_token();
        let id = Uuid::new_v4();
        let (done, done_receiver) = watch::channel(false);
        self.scheduler.lock().await.direct.insert(id, Direct { snapshot: snapshot.clone(), cancel: cancel.clone(), done: done_receiver });
        let (result, receiver) = watch::channel(None);
        let service = self.clone();
        self.tasks.spawn_on(async move {
            let _permit = permit;
            service.activity.send_replace(Instant::now());
            let operation = async {
                let result = service.recognize(&snapshot, source, &cancel, false, true);
                tokio::pin!(result);
                let result = tokio::select! { biased; _ = caller.cancelled() => { cancel.cancel(); result.await }, result = &mut result => result };
                if cancel.is_cancelled() { Err(Error::Cancelled) } else { result.map(Arc::new) }
            };
            let outcome = AssertUnwindSafe(operation).catch_unwind().await.unwrap_or_else(|_| Err(Error::Task("即时 OCR 任务异常退出".into())));
            service.scheduler.lock().await.direct.remove(&id);
            done.send_replace(true); result.send_replace(Some(outcome));
            service.activity.send_replace(Instant::now());
        }, &self.runtime);
        drop(_lifecycle);
        await_result(receiver, &self.root).await
    }

    async fn recognize(&self, snapshot: &Snapshot, source: ImageSource, cancel: &CancellationToken, automatic: bool, urgent: bool) -> Result<OcrRecognition> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let mode = snapshot.state.snapshot.value.settings.ocr_mode;
        let (text, confidence, details) = match mode {
            OcrMode::Local => {
                let model = self.resolve_local(snapshot, if automatic { LocalRequest::Automatic } else { LocalRequest::Manual }, cancel).await?;
                let result = self.local.recognize(model, source, cancel.clone(), urgent).await
                    .map_err(|error| if cancel.is_cancelled() { Error::Cancelled } else { platform_error(error) })?;
                (result.text, result.confidence, None)
            }
            OcrMode::Remote => {
                let config = snapshot.remote.as_ref().ok_or_else(|| Error::Task("远程 OCR 配置未初始化".into()))?.clone()?;
                let image = self.images.acquire_encoded(source, cancel).await?;
                let result = remote::recognize(&self.http, &self.images, config, image, cancel).await?;
                (result.text, 1.0, Some(Arc::new(result.details)))
            }
            OcrMode::Openai | OcrMode::Claude => {
                let (provider, field, config) = if mode == OcrMode::Openai {
                    (AiProvider::Openai, CredentialId::OpenaiApiKey, &snapshot.state.snapshot.value.settings.ocr_ai_configs.openai)
                } else { (AiProvider::Claude, CredentialId::ClaudeApiKey, &snapshot.state.snapshot.value.settings.ocr_ai_configs.claude) };
                let key = tokio::select! { _ = cancel.cancelled() => return Err(Error::Cancelled), key = self.configuration.credential(field) => key? }
                    .ok_or_else(|| Error::Invalid("AI OCR API Key 不可用".into()))?;
                if self.configuration.snapshot().ocr_revision() != snapshot.revision { return Err(Error::Cancelled); }
                let config = AiRequestConfig::new(provider, config, key)?;
                (self.ai.recognize(config, source, cancel).await?, 0.0, None)
            }
        };
        if cancel.is_cancelled() || self.configuration.snapshot().ocr_revision() != snapshot.revision { return Err(Error::Cancelled); }
        Ok(OcrRecognition { text: text.into(), confidence, mode, details })
    }
    async fn resolve_local(&self, snapshot: &Snapshot, request: LocalRequest, cancel: &CancellationToken) -> Result<InstalledModel> {
        let mut verified = tokio::select! { _ = cancel.cancelled() => return Err(Error::Cancelled), lock = self.verified.lock() => lock };
        if request == LocalRequest::Preparation { self.check_maintenance(snapshot).await?; }
        if let Some(model) = verified.as_ref().filter(|model| model.info.id == snapshot.model) { return Ok(model.clone()); }
        catalog::get_model(&snapshot.model).map_err(platform_error)?;
        // installed propagates cancellation and drains accepted file I/O. Keep
        // the verification lock until then so a replacement cannot overlap it.
        let model = self.models.installed(&snapshot.model, cancel).await?;
        let model = match model {
            Some(model) => model,
            None if request != LocalRequest::Manual => tokio::select! { _ = cancel.cancelled() => return Err(Error::Cancelled), model = self.models.download(&snapshot.model) => model.map_err(platform_error)? },
            None => return Err(Error::Invalid("本地 OCR 模型未下载或已损坏，请在 OCR 设置中下载模型".into())),
        };
        *verified = Some(model.clone());
        Ok(model)
    }

    async fn synchronize(self: &Arc<Self>) -> Result<()> { self.reconfigure(self.configuration.snapshot()).await }
    async fn reconfigure(self: &Arc<Self>, state: ConfigurationState) -> Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        self.check_open()?;
        let old = self.current.read().await.clone();
        if old.revision >= state.ocr_revision() { return Ok(()); }
        let next = Snapshot::new(state, &self.root)?;
        old.cancel.cancel();
        let mut removed = Vec::new();
        {
            let mut scheduler = self.scheduler.lock().await;
            let queued: Vec<_> = scheduler.queue.iter().copied().collect();
            for id in queued {
                if let Some(job) = scheduler.jobs.get_mut(&id) {
                    if !next.state.snapshot.value.settings.enable_local_ocr {
                        let ids: Vec<_> = job.records.iter().filter(|(_, record)| record.trigger == OcrTrigger::Automatic).map(|(id, _)| *id).collect();
                        for record in ids { if let Some(entry) = job.records.remove(&record) { removed.push(entry); } }
                    }
                    job.snapshot = next.clone(); job.cancel = next.cancel.child_token();
                }
            }
            for entry in &removed { scheduler.records.remove(&entry.ticket.record); }
            let empty: Vec<_> = scheduler.jobs.iter().filter(|(_, job)| job.phase == Phase::Queued && job.records.is_empty()).map(|(id, _)| *id).collect();
            for id in empty { if let Some(job) = scheduler.jobs.remove(&id) { job.done.send_replace(true); } }
            scheduler.images.clear();
            let indexes: Vec<_> = scheduler.jobs.iter().filter(|(_, job)| job.phase == Phase::Queued).map(|(id, job)| ((job.image, next.revision), *id)).collect();
            scheduler.images.extend(indexes);
            let live: BTreeSet<_> = scheduler.jobs.keys().copied().collect(); scheduler.queue.retain(|id| live.contains(id));
        }
        for entry in removed {
            if let Err(error) = self.storage.cancel_ocr(entry.ticket).await { self.events.publish(AppEvent::OcrServiceFailed(error)); }
            {
                let mut scheduler = self.scheduler.lock().await;
                self.status(&mut scheduler.statuses, entry.ticket, OcrTaskState::Cancelled);
            }
            entry.completion.send_replace(Some(Err(Error::Cancelled)));
        }
        let unloaded = if old.local_model().is_some() {
            let result = self.local.unload(None).await.map_err(platform_error);
            self.verified.lock().await.take(); result
        } else { Ok(()) };
        *self.current.write().await = next.clone();
        self.activity.send_replace(Instant::now()); self.notify.notify_waiters();
        self.start_preparation(next);
        unloaded
    }
    async fn watch_configuration(self: Arc<Self>) {
        let mut updates = self.configuration.subscribe();
        loop {
            let state = updates.borrow_and_update().clone();
            if let Err(error) = self.reconfigure(state).await { if !self.root.is_cancelled() { self.events.publish(AppEvent::OcrServiceFailed(error)); } }
            tokio::select! { _ = self.root.cancelled() => return, changed = updates.changed() => if changed.is_err() { return } }
        }
    }
    fn start_preparation(self: &Arc<Self>, snapshot: Arc<Snapshot>) {
        if !snapshot.state.snapshot.value.settings.enable_local_ocr || snapshot.local_model().is_none() || snapshot.cancel.is_cancelled() { return; }
        let service = self.clone();
        self.tasks.spawn_on(async move {
            let result = service.resolve_local(&snapshot, LocalRequest::Preparation, &snapshot.cancel).await;
            if let Err(error) = result { if !snapshot.cancel.is_cancelled() && !service.maintenance.lock().await.contains(&snapshot.model) { service.events.publish(AppEvent::OcrServiceFailed(error)); } }
            service.activity.send_replace(Instant::now());
        }, &self.runtime);
    }
    async fn idle_loop(self: Arc<Self>) {
        let mut activity = self.activity.subscribe();
        loop {
            let deadline = *activity.borrow_and_update() + LOCAL_IDLE;
            tokio::select! {
                _ = self.root.cancelled() => return,
                changed = activity.changed() => { if changed.is_err() { return; } continue; },
                _ = tokio::time::sleep_until(deadline) => {},
            }
            let _lifecycle = self.lifecycle.lock().await;
            let scheduler = self.scheduler.lock().await;
            if *self.activity.borrow() + LOCAL_IDLE <= Instant::now() && scheduler.jobs.is_empty() && scheduler.direct.is_empty() {
                if let Ok(mut verified) = self.verified.try_lock() {
                    if let Err(error) = self.local.unload(None).await { self.events.publish(AppEvent::OcrServiceFailed(platform_error(error))); }
                    else { verified.take(); }
                }
            }
            drop(scheduler); drop(_lifecycle);
            tokio::select! { _ = self.root.cancelled() => return, changed = activity.changed() => if changed.is_err() { return } }
        }
    }
    async fn check_maintenance(&self, snapshot: &Snapshot) -> Result<()> {
        if let Some(id) = snapshot.local_model() {
            if self.maintenance.lock().await.contains(id) { return Err(Error::Conflict("本地模型正在维护，请稍后重试".into())); }
        }
        Ok(())
    }
    pub async fn remove_model<E: ConfigurationEffects + 'static>(self: &Arc<Self>, id: String, effects: Arc<E>) -> Result<()> {
        self.maintenance_job(id, move |service, id| async move {
            service.quiesce_model(&id, true).await?;
            let state = service.configuration.snapshot();
            let settings = &state.snapshot.value.settings;
            let selected = if settings.ocr_local_model.trim().is_empty() { catalog::default_model_id().map_err(platform_error)? } else { settings.ocr_local_model.as_str() };
            if settings.ocr_mode == OcrMode::Local && selected == id && settings.enable_local_ocr {
                let fields = [ConfigField::EnableLocalOcr, ConfigField::OcrMode, ConfigField::OcrLocalModel];
                let mutation = ConfigurationMutation { client_id: Uuid::new_v4(), mutation_id: Uuid::new_v4(),
                    base_versions: fields.into_iter().map(|field| (field, state.fields.get(&field).copied().unwrap_or(0))).collect(),
                    changes: vec![ConfigChange::EnableLocalOcr(false)], credentials: Vec::new() };
                service.configuration.commit(mutation, effects).await?;
            }
            service.check_open()?;
            service.models.remove(&id).await.map_err(platform_error)
        }).await
    }
    pub async fn redownload_model(self: &Arc<Self>, id: String) -> Result<()> {
        self.maintenance_job(id, |service, id| async move {
            service.quiesce_model(&id, false).await?;
            service.check_open()?;
            let installed = service.models.redownload(&id).await.map_err(platform_error)?;
            let _lifecycle = service.lifecycle.lock().await;
            if service.current.read().await.local_model() == Some(id.as_str()) { *service.verified.lock().await = Some(installed); }
            Ok(())
        }).await
    }
    async fn maintenance_job<F, Fut>(self: &Arc<Self>, id: String, operation: F) -> Result<()>
    where F: FnOnce(Arc<Self>, String) -> Fut + Send + 'static, Fut: std::future::Future<Output = Result<()>> + Send + 'static {
        let _lifecycle = self.lifecycle.lock().await;
        self.check_open()?;
        catalog::get_model(&id).map_err(platform_error)?;
        if !self.maintenance.lock().await.insert(id.clone()) { return Err(Error::Conflict("模型正在维护，请稍后重试".into())); }
        let (result, receiver) = watch::channel(None);
        let service = self.clone();
        self.tasks.spawn_on(async move {
            let outcome = AssertUnwindSafe(operation(service.clone(), id.clone())).catch_unwind().await
                .unwrap_or_else(|_| Err(Error::Task("OCR 模型维护任务异常退出".into())));
            service.maintenance.lock().await.remove(&id);
            service.notify.notify_waiters();
            service.activity.send_replace(Instant::now());
            if outcome.is_ok() && !service.root.is_cancelled() { let snapshot = service.current.read().await.clone(); service.start_preparation(snapshot); }
            result.send_replace(Some(outcome));
        }, &self.runtime);
        drop(_lifecycle);
        await_result(receiver, &self.root).await
    }
    async fn quiesce_model(&self, id: &str, delete: bool) -> Result<()> {
        let (records, direct, done) = {
            let mut scheduler = self.scheduler.lock().await;
            let mut records = Vec::new();
            let mut done = Vec::new();
            for job in scheduler.jobs.values_mut().filter(|job| job.snapshot.local_model() == Some(id)) {
                job.handoff.take();
                if delete { records.extend(job.records.keys().copied()); }
                if delete || job.phase != Phase::Queued { done.push(job.done.subscribe()); }
            }
            let direct: Vec<_> = scheduler.direct.values().filter(|job| job.snapshot.local_model() == Some(id)).map(|job| job.cancel.clone()).collect();
            done.extend(scheduler.direct.values().filter(|job| job.snapshot.local_model() == Some(id)).map(|job| job.done.clone()));
            (records, direct, done)
        };
        if delete { for cancel in direct { cancel.cancel(); } self.cancel_records(&records).await?; }
        for mut done in done {
            while !*done.borrow_and_update() {
                tokio::select! { _ = self.root.cancelled() => return Err(Error::Cancelled), changed = done.changed() => changed.map_err(|_| Error::Task("OCR 维护等待任务异常退出".into()))? }
            }
        }
        self.check_open()?;
        self.models.cancel_download(id).await.map_err(platform_error)?;
        self.local.unload(Some(id.to_owned())).await.map_err(platform_error)?;
        let mut verified = self.verified.lock().await;
        if verified.as_ref().is_some_and(|model| model.info.id == id) { verified.take(); }
        Ok(())
    }
    pub async fn diagnostics(&self) -> OcrDiagnostics {
        let snapshot = self.current.read().await.clone();
        let (queued, running, direct) = {
            let scheduler = self.scheduler.lock().await;
            (scheduler.jobs.values().filter(|job| job.phase == Phase::Queued).count(), scheduler.jobs.values().filter(|job| job.phase != Phase::Queued).count(), scheduler.direct.len())
        };
        OcrDiagnostics { revision: snapshot.revision, queued, running, direct, maintenance: self.maintenance.lock().await.len(), local_model_loaded: self.local.is_loaded(&snapshot.model), suspended: self.suspended.load(Ordering::Acquire), closing: self.root.is_cancelled(), models: self.models.diagnostics().await }
    }
    pub async fn validate_configuration(&self, settings: &super_paste_format::Preferences) -> Result<()> {
        if settings.ocr_mode != OcrMode::Local { return Ok(()); }
        let id = if settings.ocr_local_model.trim().is_empty() { catalog::default_model_id().map_err(platform_error)? } else { settings.ocr_local_model.as_str() };
        catalog::get_model(id).map_err(platform_error)?;
        if settings.enable_local_ocr && self.maintenance.lock().await.contains(id) { return Err(Error::Conflict("本地模型正在维护，请稍后重试".into())); }
        Ok(())
    }
    pub async fn shutdown(self: &Arc<Self>) -> Result<()> {
        let mut shutdown = self.shutdown.lock().await;
        let receiver = if let Some(receiver) = shutdown.as_ref() { receiver.clone() } else {
            self.root.cancel();
            let (sender, receiver) = watch::channel(None);
            *shutdown = Some(receiver.clone());
            let service = self.clone();
            self.runtime.spawn(async move {
                let result = AssertUnwindSafe(service.shutdown_inner()).catch_unwind().await
                    .unwrap_or_else(|_| Err(Error::Task("OCR 关闭任务异常退出".into())));
                sender.send_replace(Some(result));
            });
            receiver
        };
        drop(shutdown);
        await_result(receiver, &CancellationToken::new()).await
    }
    async fn shutdown_inner(&self) -> Result<()> {
        let records = {
            let _lifecycle = self.lifecycle.lock().await;
            self.direct_limit.close();
            let scheduler = self.scheduler.lock().await;
            for job in scheduler.jobs.values() { job.cancel.cancel(); }
            for direct in scheduler.direct.values() { direct.cancel.cancel(); }
            self.tasks.close();
            scheduler.records.keys().copied().collect::<Vec<_>>()
        };
        let cancelled = self.cancel_records(&records).await;
        self.models.shutdown().await;
        self.tasks.wait().await;
        self.scheduler.lock().await.statuses.clear();
        let stopped = self.local.shutdown().await.map_err(platform_error);
        let trimmed = self.images.trim();
        let failures: Vec<_> = [cancelled, stopped, trimmed].into_iter().filter_map(Result::err).collect();
        if failures.len() == 1 { return Err(failures.into_iter().next().ok_or_else(|| Error::Task("OCR 关闭状态不一致".into()))?); }
        if failures.is_empty() { Ok(()) } else { Err(Error::Task(failures.iter().map(ToString::to_string).collect::<Vec<_>>().join("；").into())) }
    }
    fn check_open(&self) -> Result<()> { if self.root.is_cancelled() { Err(Error::Cancelled) } else { Ok(()) } }
    fn status(&self, statuses: &mut TaskStatuses, ticket: OcrTicket, state: OcrTaskState) {
        if let Some(status) = statuses.update(ticket.record, ticket.generation, state, std::time::Instant::now()) { self.events.publish(AppEvent::OcrTask(status)); }
    }
}
fn platform_error(message: String) -> Error { Error::Platform(message.into()) }
