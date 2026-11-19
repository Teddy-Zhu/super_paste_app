use std::{future::Future, sync::{Arc, Mutex as AdmissionMutex, atomic::{AtomicBool, Ordering}}};
use async_trait::async_trait;
use sha2::{Digest as _, Sha256};
use super_paste_format::*;
use tokio::{runtime::Handle, sync::{Mutex, Semaphore, OwnedSemaphorePermit, watch}};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use crate::{Error, Result, configuration::ConfigurationService, events::{AppEvent, EventBus, RecordChange}, images::{AlphaMode, ImageLimits, ImageService, ImageSource, PixelLayout, PixelSpec, ResizeFilter}, storage::{PreparedRecord, Storage, UpsertResult}};
use crate::ocr::{CapturedImage, OcrService};

#[derive(Clone, Default)]
pub struct ClipboardSnapshot { pub content: TextContent, pub image: Option<ImageSource> }
impl ClipboardSnapshot {
    pub fn is_empty(&self) -> bool { self.content.formats(self.image.is_some()).is_empty() }
}

#[async_trait]
pub trait ClipboardBackend: Send + Sync {
    async fn read(&self, cancel: &CancellationToken) -> Result<ClipboardSnapshot>;
    async fn write(&self, snapshot: ClipboardSnapshot, cancel: &CancellationToken) -> Result<()>;
}
#[derive(Clone, Debug)]
pub struct ClipboardChange {
    pub cursor: uuid::Uuid, pub timestamp: i64, pub content: TextContent, pub image_bytes: Option<u64>,
}
#[derive(Clone, Copy, Debug)]
pub enum RestoreMode { Original, PlainText, Ocr }
#[derive(Clone, Copy, Debug)]
pub enum QrAction { Generate, Parse }
#[derive(Clone, Debug)]
pub enum CaptureOutcome { Duplicate, ObservedOnly, Stored { record: UpsertResult, removed: Vec<RecordId> } }

pub struct ClipboardService {
    runtime: Handle, storage: Arc<Storage>, configuration: Arc<ConfigurationService>, images: Arc<ImageService>,
    backend: Arc<dyn ClipboardBackend>, events: EventBus, gate: Mutex<Option<Digest>>,
    latest: watch::Sender<Option<Arc<ClipboardChange>>>,
    ocr: Arc<OcrService>,
    native_listening: AtomicBool,
    admission: AdmissionMutex<bool>, pending: Arc<Semaphore>, writes: Arc<Semaphore>, tasks: TaskTracker,
    read_shutdown: CancellationToken,
}
impl ClipboardService {
    pub fn new(runtime: Handle, storage: Arc<Storage>, configuration: Arc<ConfigurationService>, images: Arc<ImageService>, backend: Arc<dyn ClipboardBackend>, events: EventBus, ocr: Arc<OcrService>) -> Arc<Self> {
        let (latest, _) = watch::channel(None);
        Arc::new(Self { runtime, storage, configuration, images, backend, events, gate: Mutex::new(None), latest, ocr, native_listening: AtomicBool::new(false), admission: AdmissionMutex::new(true), pending: Arc::new(Semaphore::new(16)), writes: Arc::new(Semaphore::new(3)), tasks: TaskTracker::new(), read_shutdown: CancellationToken::new() })
    }
    pub fn subscribe_changes(&self) -> watch::Receiver<Option<Arc<ClipboardChange>>> { self.latest.subscribe() }
    pub fn set_native_listening(&self, listening: bool) { self.native_listening.store(listening, Ordering::Release); }
    pub fn is_native_listening(&self) -> bool { self.native_listening.load(Ordering::Acquire) }
    pub fn is_listening(&self) -> bool { self.is_native_listening() && self.configuration.snapshot().snapshot.value.settings.clipboard_listening }
    pub async fn read(&self, cancel: &CancellationToken) -> Result<ClipboardSnapshot> {
        if cancel.is_cancelled() || self.read_shutdown.is_cancelled() { return Err(Error::Cancelled); }
        let token = self.read_shutdown.child_token();
        let _cancel_on_drop = token.clone().drop_guard();
        let operation_cancel = token.clone();
        let backend = self.backend.clone();
        // Reuse the existing admission bound and task ownership. Shutdown must
        // wait for accepted native reads, including abandoned caller futures.
        let read = self.owned(async move {
            if operation_cancel.is_cancelled() { return Err(Error::Cancelled); }
            let result = backend.read(&operation_cancel).await;
            if operation_cancel.is_cancelled() { Err(Error::Cancelled) } else { result }
        });
        tokio::pin!(read);
        tokio::select! { biased;
            _ = cancel.cancelled() => {
                token.cancel();
                let _ = read.await;
                Err(Error::Cancelled)
            },
            result = &mut read => if token.is_cancelled() { Err(Error::Cancelled) } else { result },
        }
    }
    async fn owned<T: Send + 'static>(&self, operation: impl Future<Output = Result<T>> + Send + 'static) -> Result<T> {
        let task = {
            let accepting = self.admission.lock().map_err(|_| Error::Task("剪贴板接入锁损坏".into()))?;
            if !*accepting { return Err(Error::Cancelled); }
            let permit = self.pending.clone().try_acquire_owned().map_err(|error| match error {
                tokio::sync::TryAcquireError::Closed => Error::Cancelled,
                tokio::sync::TryAcquireError::NoPermits => Error::ResourceLimit("剪贴板操作队列已满".into()),
            })?;
            self.tasks.spawn_on(async move {
                let _permit = permit;
                let result = operation.await;
                if let Err(error) = &result { tracing::warn!(%error, "剪贴板操作失败"); }
                result
            }, &self.runtime)
        };
        task.await?
    }
    fn write_permit(&self) -> Result<OwnedSemaphorePermit> {
        self.writes.clone().try_acquire_owned().map_err(|error| match error {
            tokio::sync::TryAcquireError::Closed => Error::Cancelled,
            tokio::sync::TryAcquireError::NoPermits => Error::ResourceLimit("剪贴板写入队列已满".into()),
        })
    }
    pub async fn write(self: &Arc<Self>, snapshot: ClipboardSnapshot, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        if snapshot.is_empty() { return Err(Error::Invalid("剪贴板没有可还原的内容".into())); }
        let permit = self.write_permit()?;
        let service = self.clone(); let cancel = cancel.clone();
        self.owned(async move { let _permit = permit; service.write_inner(snapshot, &cancel).await }).await
    }
    async fn write_inner(&self, snapshot: ClipboardSnapshot, cancel: &CancellationToken) -> Result<()> {
        if snapshot.is_empty() { return Err(Error::Invalid("剪贴板没有可还原的内容".into())); }
        self.backend.write(snapshot, cancel).await
    }

    pub async fn capture(&self, snapshot: ClipboardSnapshot, source: Option<Arc<SourceApp>>, timestamp: i64, cancel: &CancellationToken) -> Result<CaptureOutcome> {
        if snapshot.is_empty() { return Ok(CaptureOutcome::Duplicate); }
        let mut previous = self.gate.lock().await;
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let content = snapshot.content.clone();
        let source_identity = match &snapshot.image {
            Some(ImageSource::Encoded(image)) => Some((0, image.id())),
            Some(ImageSource::Pixels(image)) => Some((1, image.id())),
            Some(ImageSource::Stored(image)) => Some((0, image.id)),
            None => None,
        };
        let (text_hasher, raw_digest) = self.runtime.spawn_blocking(move || {
            let hash = hash_text(&content);
            let mut raw = hash.clone();
            if let Some((kind, id)) = source_identity { raw.update([kind]); raw.update(id.0); }
            (hash, Digest(raw.finalize().into()))
        }).await?;
        if previous.as_ref() == Some(&raw_digest) { return Ok(CaptureOutcome::Duplicate); }
        let encoded = match snapshot.image.clone() {
            Some(ImageSource::Encoded(image)) => Some(image),
            Some(ImageSource::Stored(metadata)) => Some(self.images.load_original(metadata, cancel).await?),
            Some(ImageSource::Pixels(pixels)) => Some(self.images.encode_png(pixels, cancel).await?),
            None => None,
        };
        let kind = snapshot.content.primary_type(encoded.is_some());
        let bytes = snapshot.content.byte_len() + encoded.as_ref().map_or(0, |image| image.bytes().len() as u64);
        self.latest.send_replace(Some(Arc::new(ClipboardChange {
            cursor: uuid::Uuid::new_v4(), timestamp, content: snapshot.content.clone(), image_bytes: encoded.as_ref().map(|image| image.bytes().len() as u64),
        })));
        let source_name = source.as_ref().map_or("", |source| source.application.trim());
        let source_name = truncate_utf16(source_name, 200);
        if self.storage.record_copy(CopyEvent { timestamp, kind, source: source_name, bytes, files: snapshot.content.files.len() as u64 }).await? {
            self.events.publish(AppEvent::CopyStatisticsChanged);
        }
        let settings = self.configuration.snapshot().snapshot.value;
        if !settings.settings.clipboard_listening { *previous = Some(raw_digest); return Ok(CaptureOutcome::ObservedOnly); }
        let mut hash = text_hasher;
        let mut captured_pixels = None;
        if let Some(image_source) = snapshot.image {
            match self.images.acquire_pixels(image_source, ImageLimits::default(), cancel).await {
                Ok(pixels) => {
                    let pixels = if pixels.layout() == PixelLayout::Rgba8 && pixels.alpha() == AlphaMode::Straight && pixels.stride() == pixels.width() as usize * 4 { pixels }
                    else { self.images.derive_pixels(pixels.clone(), PixelSpec { max_width: pixels.width(), max_height: pixels.height(), layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest }, cancel).await? };
                    hash.update(b"\0rgba8\0"); hash.update(pixels.id().0);
                    captured_pixels = Some(pixels);
                }
                Err(Error::Image(_)) => {
                    // Existing behavior preserves undecodable source bytes and deduplicates them by raw identity.
                    hash.update(b"\0raw\0"); hash.update(encoded.as_ref().ok_or_else(|| Error::Image("图片原始载荷缺失".into()))?.id().0);
                }
                Err(error) => return Err(error),
            }
        }
        let digest = Digest(hash.finalize().into());
        let (inserted, removed) = self.storage.upsert_record(
            PreparedRecord { digest, content: snapshot.content, image: encoded.clone(), timestamp, source },
            HistoryLimits { max_records: settings.settings.max_history_size, keep_days: settings.settings.max_keep_time },
        ).await?;
        self.events.publish(AppEvent::HistoryChanged { ids: Arc::from([inserted.id]), change: if inserted.inserted { RecordChange::Inserted } else { RecordChange::Metadata } });
        if !removed.is_empty() { self.events.publish(AppEvent::HistoryChanged { ids: removed.clone().into(), change: RecordChange::Removed }); }
        *previous = Some(raw_digest);
        drop(previous);
        if !removed.is_empty() { if let Err(error) = self.ocr.forget_records(&removed).await { self.events.publish(AppEvent::OcrServiceFailed(error)); } }
        if let Some(original) = encoded {
            if !removed.contains(&inserted.id) {
                if let Err(error) = self.ocr.enqueue_captured(inserted.id, CapturedImage { original, pixels: captured_pixels }).await { self.events.publish(AppEvent::OcrServiceFailed(error)); }
            }
        }
        Ok(CaptureOutcome::Stored { record: inserted, removed })
    }

    pub async fn restore(self: &Arc<Self>, id: RecordId, mode: RestoreMode, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let permit = self.write_permit()?;
        let service = self.clone(); let cancel = cancel.clone();
        // Once native writing starts, retain ownership through the metadata commit,
        // even if the overlay is hidden or its waiting task is dropped.
        self.owned(async move { let _permit = permit; service.restore_inner(id, mode, &cancel).await }).await
    }
    async fn restore_inner(&self, id: RecordId, mode: RestoreMode, cancel: &CancellationToken) -> Result<()> {
        let input = self.storage.restore_input(id, mode, cancel).await.map_err(|error| match error {
            Error::NotFound => Error::Invalid("历史记录不存在".into()), other => other,
        })?;
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let snapshot = match mode {
            RestoreMode::Original => input,
            RestoreMode::PlainText => {
                let mut content = input.content;
                // Storage retains raw text only when it wins the shared restore
                // priority. Move that Arc directly; only format extraction or
                // joining file paths needs a CPU task.
                let text = if let Some(text) = content.text.take() { text } else {
                    let token = cancel.clone();
                    // The caller holds a write permit for this entire operation,
                    // bounding extraction to the existing three accepted writers.
                    // Queued cancelled work must not begin a large format scan.
                    self.runtime.spawn_blocking(move || {
                        if token.is_cancelled() { return Err(Error::Cancelled); }
                        let text = content.plain_text()?;
                        if token.is_cancelled() { return Err(Error::Cancelled); }
                        Ok::<_, Error>(text)
                    }).await??
                };
                ClipboardSnapshot { content: TextContent { text: Some(text), ..TextContent::default() }, image: None }
            },
            RestoreMode::Ocr => {
                let text = input.content.text.filter(|text| !text.is_empty()).ok_or_else(|| Error::Invalid("该记录没有 OCR 文本".into()))?;
                ClipboardSnapshot { content: TextContent { text: Some(text), ..TextContent::default() }, image: None }
            }
        };
        self.write_inner(snapshot, cancel).await?;
        if self.storage.touch(id, crate::now_ms()?).await? { self.events.publish(AppEvent::HistoryChanged { ids: Arc::from([id]), change: RecordChange::Metadata }); }
        Ok(())
    }
    pub async fn copy_qr(self: &Arc<Self>, id: RecordId, action: QrAction, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let permit = self.write_permit()?; let service = self.clone(); let cancel = cancel.clone();
        self.owned(async move {
            let _permit = permit; let qr = crate::qr::QrService::new(service.images.clone());
            let snapshot = match action {
                QrAction::Generate => {
                    let source = service.storage.qr_text_input(id, &cancel).await?;
                    let pixels = qr.generate(source, &cancel).await?;
                    ClipboardSnapshot { content: TextContent::default(), image: Some(ImageSource::Pixels(pixels)) }
                },
                QrAction::Parse => {
                    let image = match service.storage.image_metadata(id, &cancel).await {
                        Ok(Some(image)) => image,
                        Ok(None) | Err(Error::NotFound) => return Err(Error::Invalid("记录不包含图片".into())),
                        Err(error) => return Err(error),
                    };
                    let text = qr.parse(ImageSource::Stored(image), &cancel).await?.ok_or_else(|| Error::Invalid("未识别到二维码".into()))?;
                    ClipboardSnapshot { content: TextContent { text: Some(text.into()), ..TextContent::default() }, image: None }
                },
            };
            // Generated pixels go straight to the native adapter; no temporary
            // PNG/Base64/data URL is produced for an unused response field.
            service.write_inner(snapshot, &cancel).await
        }).await
    }
    pub async fn delete(self: &Arc<Self>, id: RecordId) -> Result<bool> {
        let service = self.clone();
        self.owned(async move { service.delete_inner(id).await }).await
    }
    async fn delete_inner(&self, id: RecordId) -> Result<bool> {
        let deleted = self.storage.delete_record(id).await?;
        if deleted {
            if let Err(error) = self.ocr.forget_records(&[id]).await { self.events.publish(AppEvent::OcrServiceFailed(error)); }
            self.events.publish(AppEvent::HistoryChanged { ids: Arc::from([id]), change: RecordChange::Removed });
        }
        Ok(deleted)
    }
    pub async fn clear_history(self: &Arc<Self>) -> Result<Vec<RecordId>> {
        let service = self.clone();
        self.owned(async move { service.clear_history_inner().await }).await
    }
    async fn clear_history_inner(&self) -> Result<Vec<RecordId>> {
        let ids = self.storage.clear_history().await?;
        if !ids.is_empty() {
            if let Err(error) = self.ocr.forget_records(&ids).await { self.events.publish(AppEvent::OcrServiceFailed(error)); }
            self.events.publish(AppEvent::HistoryChanged { ids: ids.clone().into(), change: RecordChange::Removed });
        }
        Ok(ids)
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.read_shutdown.cancel();
        let failure = match self.admission.lock() {
            Ok(mut accepting) => { *accepting = false; None }
            Err(poisoned) => { *poisoned.into_inner() = false; Some(Error::Task("剪贴板接入锁损坏".into())) }
        };
        self.pending.close(); self.writes.close(); self.tasks.close(); self.tasks.wait().await;
        failure.map_or(Ok(()), Err)
    }
}

fn hash_text(content: &TextContent) -> Sha256 {
    let mut hash = Sha256::new(); hash.update(b"super-paste-rs-content-v1\0");
    for field in [&content.text, &content.html, &content.rtf] {
        match field {
            Some(value) => { hash.update([1]); hash.update((value.len() as u64).to_be_bytes()); hash.update(value.as_bytes()); }
            None => hash.update([0]),
        }
    }
    hash.update((content.files.len() as u64).to_be_bytes());
    for path in content.files.iter() { hash.update((path.len() as u64).to_be_bytes()); hash.update(path.as_bytes()); }
    hash
}
fn truncate_utf16(value: &str, max: usize) -> String {
    let mut used = 0;
    value.chars().take_while(|character| { used += character.len_utf16(); used <= max }).collect()
}
