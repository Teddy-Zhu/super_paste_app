mod color;
mod cmyk;
mod jpeg;
mod sampling;
mod animation;
mod workers;
#[cfg(test)]
mod tests;
pub use animation::{AnimationCheckpoint, DisplayFrame, ImageAnimation};

use std::{borrow::Cow, future::Future, hash::Hash, io::Cursor, sync::{Arc, OnceLock}};
use bytes::Bytes;
use image::{ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Pixel, imageops::{self, FilterType}};
use image::metadata::Orientation;
use sha2::{Digest as _, Sha256};
use super_paste_format::{Digest, ImageMetadata};
use tokio::runtime::Handle;
use tokio_util::{sync::CancellationToken, task::{TaskTracker, task_tracker::TaskTrackerToken}};
use crate::{Error, Result, resources::{BoundedWriter, ByteBudget, Reservation, Weighted, WorkCache, WorkDemand}, storage::Storage};
use workers::{WorkerPermit, WorkerPool};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelLayout { Rgba8, Bgra8, Rgb8, Gray8 }
impl PixelLayout {
    pub const fn channels(self) -> u32 { match self { Self::Rgba8 | Self::Bgra8 => 4, Self::Rgb8 => 3, Self::Gray8 => 1 } }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AlphaMode { Straight, Premultiplied, Opaque }

/// A shared immutable encoded input. Structured documents retain its header and
/// one lazily computed digest across card/detail requests, never another data URL.
pub struct EncodedImageInput {
    data: Bytes, format: Option<ImageFormat>, dimensions: Option<(u32, u32)>, digest: OnceLock<Digest>,
}
impl EncodedImageInput {
    /// Call on a parsing/ingress worker; header inspection does not decode pixels.
    pub fn new(data: Bytes) -> Arc<Self> {
        let (format, dimensions) = inspect_header(&data);
        Arc::new(Self { data, format, dimensions, digest: OnceLock::new() })
    }
    pub fn bytes(&self) -> &[u8] { &self.data }
    pub fn dimensions(&self) -> Option<(u32, u32)> { self.dimensions }
}
impl std::fmt::Debug for EncodedImageInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodedImageInput").field("bytes", &self.data.len()).field("dimensions", &self.dimensions).finish_non_exhaustive()
    }
}

pub struct EncodedImage {
    id: Digest,
    data: Bytes,
    format: Option<ImageFormat>,
    dimensions: Option<(u32, u32)>,
    animation: OnceLock<Result<animation::AnimationInfo>>,
    _reservation: Reservation,
}
impl EncodedImage {
    pub fn id(&self) -> Digest { self.id }
    pub fn bytes(&self) -> &[u8] { &self.data }
    /// The HTTP body retains the allocation's budget lease until its last byte owner drops.
    pub fn shared_bytes(self: &Arc<Self>) -> Bytes { Bytes::from_owner(EncodedBytes(self.clone())) }
    pub fn format(&self) -> Option<ImageFormat> { self.format }
    pub fn dimensions(&self) -> Option<(u32, u32)> { self.dimensions }
    pub fn metadata(&self) -> ImageMetadata {
        ImageMetadata { id: self.id, encoded_bytes: self.data.len() as u64, width: self.dimensions.map(|v| v.0), height: self.dimensions.map(|v| v.1) }
    }
    pub fn format_name(&self) -> &'static str {
        match self.format {
            Some(ImageFormat::Png) => "png", Some(ImageFormat::Jpeg) => "jpeg", Some(ImageFormat::Gif) => "gif",
            Some(ImageFormat::WebP) => "webp", Some(ImageFormat::Bmp) => "bmp", Some(ImageFormat::Tiff) => "tiff", _ => "unknown",
        }
    }
}
struct EncodedBytes(Arc<EncodedImage>);
impl AsRef<[u8]> for EncodedBytes { fn as_ref(&self) -> &[u8] { self.0.bytes() } }
impl Weighted for EncodedImage { fn weight(&self) -> u64 { self._reservation.bytes() } }

#[derive(Clone)]
pub struct PixelBuffer {
    id: Digest, width: u32, height: u32, stride: usize, layout: PixelLayout,
    alpha: AlphaMode, data: Bytes, _reservation: Arc<Reservation>,
    // A projected 8-bit view keeps its decode reusable while consumers hold it.
    // The base buffer inside DecodedImage never points back to that owner.
    _decoded_owner: Option<Arc<DecodedImage>>,
    // Display extent belongs to this result, not to canonical pixel identity.
    display_source_dimensions: (u32, u32),
}
struct DecodedImage {
    width: u32, height: u32,
    pixels: DecodedPixels,
    orientation: Result<Orientation>,
    color: Result<Option<Arc<color::Input>>>,
}
enum DecodedPixels {
    Rgba8(Arc<PixelBuffer>),
    Native { image: image::DynamicImage, reservation: Reservation },
    Cmyk { samples: Bytes, reservation: Reservation },
}
impl Weighted for DecodedImage {
    fn weight(&self) -> u64 {
        let pixels = match &self.pixels { DecodedPixels::Rgba8(pixels) => pixels.weight(),
            DecodedPixels::Native { reservation, .. } | DecodedPixels::Cmyk { reservation, .. } => reservation.bytes() };
        pixels + self.color.as_ref().ok().and_then(Option::as_ref).map_or(0, |profile| profile.weight())
    }
}
impl DecodedImage {
    fn canonical_png(pixels: &PixelBuffer) -> Self {
        let mut alias = pixels.clone();
        alias._decoded_owner = None;
        alias.display_source_dimensions = (alias.width, alias.height);
        Self { width: alias.width, height: alias.height, pixels: DecodedPixels::Rgba8(Arc::new(alias)),
            orientation: Ok(Orientation::NoTransforms), color: Ok(None) }
    }
    fn rgba8_view(self: &Arc<Self>) -> Option<Arc<PixelBuffer>> {
        let DecodedPixels::Rgba8(pixels) = &self.pixels else { return None };
        let mut view = pixels.as_ref().clone();
        view._decoded_owner = Some(self.clone());
        Some(Arc::new(view))
    }
}
enum RenderSource { Pixels(Arc<PixelBuffer>), Native(Arc<DecodedImage>), Card(Arc<DecodedImage>) }
impl RenderSource {
    fn dimensions(&self) -> (u32, u32) {
        match self { Self::Pixels(pixels) => (pixels.width, pixels.height), Self::Native(decoded) => (decoded.width, decoded.height),
            Self::Card(decoded) => card_dimensions(decoded.width, decoded.height) }
    }
    fn pixels(&self) -> Option<&Arc<PixelBuffer>> { match self { Self::Pixels(pixels) => Some(pixels), Self::Native(_) | Self::Card(_) => None } }
    fn intermediate_bytes(&self) -> u64 {
        if let Self::Card(decoded) = self {
            let channels = match &decoded.pixels {
                DecodedPixels::Native { image, .. } => image.color().bytes_per_pixel(),
                DecodedPixels::Cmyk { .. } => 3,
                DecodedPixels::Rgba8(_) => return 0,
            };
            let (width, height) = self.dimensions();
            return u64::from(width) * u64::from(height) * u64::from(channels);
        }
        0
    }
}
impl PixelBuffer {
    pub fn id(&self) -> Digest { self.id }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn stride(&self) -> usize { self.stride }
    pub fn layout(&self) -> PixelLayout { self.layout }
    pub fn alpha(&self) -> AlphaMode { self.alpha }
    pub fn bytes(&self) -> &[u8] { &self.data }
}
impl Weighted for PixelBuffer {
    fn weight(&self) -> u64 { self._reservation.bytes() }
}

#[derive(Clone)]
pub enum ImageSource { Encoded(Arc<EncodedImage>), Pixels(Arc<PixelBuffer>), Stored(ImageMetadata) }

enum EncodedSource { Ready(Arc<EncodedImage>), Stored(ImageMetadata) }

pub struct DisplayPixels {
    pub pixels: Arc<PixelBuffer>,
    pub natural_dimensions: (u32, u32),
}

#[derive(Clone, Copy, Debug)]
pub struct ImageLimits { pub max_encoded_bytes: u64, pub max_pixels: u64, pub max_dimension: u32 }
impl Default for ImageLimits {
    fn default() -> Self { Self { max_encoded_bytes: 64 * 1024 * 1024, max_pixels: 40_000_000, max_dimension: 65_536 } }
}
impl ImageLimits {
    fn dimensions(self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || width > self.max_dimension || height > self.max_dimension || u64::from(width) * u64::from(height) > self.max_pixels {
            return Err(Error::ResourceLimit("图片尺寸超出当前操作的限制".into()));
        }
        Ok(())
    }
    fn encoded(self, bytes: u64, dimensions: Option<(u32, u32)>) -> Result<()> {
        if bytes > self.max_encoded_bytes { return Err(Error::ResourceLimit("图片编码大小超出当前操作的限制".into())); }
        if let Some((width, height)) = dimensions { self.dimensions(width, height)?; }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ImageBudgets {
    pub encoded_bytes: u64, pub pixel_bytes: u64, pub scratch_bytes: u64,
    pub retained_encoded_bytes: u64, pub retained_pixel_bytes: u64, pub workers: usize,
}
impl Default for ImageBudgets {
    fn default() -> Self {
        // Caps are admission limits, not eagerly allocated memory or measured performance claims.
        Self { encoded_bytes: 96 * 1024 * 1024, pixel_bytes: 256 * 1024 * 1024, scratch_bytes: 256 * 1024 * 1024,
            retained_encoded_bytes: 8 * 1024 * 1024, retained_pixel_bytes: 32 * 1024 * 1024, workers: 2 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ImageMemory { pub encoded: u64, pub pixels: u64, pub scratch: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResizeFilter { Triangle, Nearest, Lanczos3 }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PixelSpec { pub max_width: u32, pub max_height: u32, pub layout: PixelLayout, pub filter: ResizeFilter }
#[derive(Clone, Copy, Debug)]
pub enum ImagePriority { Visible, Interactive }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PixelUse { Data, Display }
impl PixelUse {
    fn alpha(self, layout: PixelLayout) -> AlphaMode {
        match layout {
            PixelLayout::Rgb8 | PixelLayout::Gray8 => AlphaMode::Opaque,
            _ if self == Self::Display => AlphaMode::Premultiplied,
            _ => AlphaMode::Straight,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DerivedKey {
    Canonical(Digest),
    NormalizedPng(Digest),
    CardBase(Digest),
    Card { source: Digest, spec: PixelSpec },
    Display { source: Digest, spec: PixelSpec },
    Animation { source: Digest, frame: Digest, spec: PixelSpec },
    Pixels { source: Digest, spec: PixelSpec, usage: PixelUse },
}

pub struct ImageService {
    runtime: Handle, storage: Arc<Storage>, encoded_budget: ByteBudget,
    pixel_budget: ByteBudget, scratch_budget: ByteBudget, workers: Arc<WorkerPool>,
    originals: Arc<WorkCache<Digest, EncodedImage>>,
    decoded: Arc<WorkCache<Digest, DecodedImage>>,
    derived: Arc<WorkCache<DerivedKey, PixelBuffer>>,
    png: Arc<WorkCache<Digest, EncodedImage>>,
    animations: Arc<WorkCache<Digest, ImageAnimation>>,
    animation_worker: std::sync::Mutex<std::sync::Weak<animation::DecoderWorker>>,
    stopped: CancellationToken, tasks: TaskTracker,
}
impl ImageService {
    pub fn new(runtime: Handle, storage: Arc<Storage>, budgets: ImageBudgets) -> Result<Arc<Self>> {
        if budgets.workers == 0 || budgets.retained_encoded_bytes > budgets.encoded_bytes || budgets.retained_pixel_bytes > budgets.pixel_bytes {
            return Err(Error::Invalid("图片资源预算无效".into()));
        }
        Ok(Arc::new(Self {
            runtime, storage, encoded_budget: ByteBudget::new(budgets.encoded_bytes, "图片编码"),
            pixel_budget: ByteBudget::new(budgets.pixel_bytes, "解码像素"), scratch_budget: ByteBudget::new(budgets.scratch_bytes, "图片工作内存"),
            workers: WorkerPool::new(budgets.workers), originals: WorkCache::new(budgets.retained_encoded_bytes),
            decoded: WorkCache::new(budgets.retained_pixel_bytes / 4), derived: WorkCache::new(budgets.retained_pixel_bytes - budgets.retained_pixel_bytes / 4), png: WorkCache::new(0),
            animations: WorkCache::new(0), animation_worker: std::sync::Mutex::new(std::sync::Weak::new()),
            stopped: CancellationToken::new(), tasks: TaskTracker::new(),
        }))
    }
    pub fn memory(&self) -> ImageMemory { ImageMemory { encoded: self.encoded_budget.used(), pixels: self.pixel_budget.used(), scratch: self.scratch_budget.used() } }
    pub(crate) fn reserve_consumer_copy(&self, bytes: u64) -> Result<Reservation> { self.scratch_budget.reserve(bytes) }
    pub(crate) fn scratch_writer(&self) -> Result<BoundedWriter> { BoundedWriter::new(self.scratch_budget.reserve(0)?, self.scratch_budget.limit()) }
    pub fn trim(&self) -> Result<()> { self.originals.trim()?; self.decoded.trim()?; self.derived.trim()?; self.animations.trim()?; self.png.trim() }

    fn check_running(&self, cancel: &CancellationToken) -> Result<()> {
        if self.stopped.is_cancelled() || cancel.is_cancelled() { Err(Error::Cancelled) } else { Ok(()) }
    }
    fn work_token(&self) -> Result<TaskTrackerToken> {
        self.check_running(&self.stopped)?;
        let task = self.tasks.token();
        // Closing a TaskTracker does not reject new tokens. Recheck after
        // registration so shutdown cannot miss newly admitted physical work.
        self.check_running(&self.stopped)?;
        Ok(task)
    }
    pub async fn with_priority<F: Future>(&self, priority: ImagePriority, work: F) -> F::Output {
        WorkDemand::scope(Some(WorkDemand::new(matches!(priority, ImagePriority::Interactive))), work).await
    }
    async fn worker_permit(&self, cancel: &CancellationToken) -> Result<WorkerPermit> {
        self.check_running(cancel)?;
        let permit = tokio::select! {
            _ = self.stopped.cancelled() => return Err(Error::Cancelled),
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            permit = self.workers.clone().acquire_owned() => permit?,
        };
        self.check_running(cancel)?;
        Ok(permit)
    }
    async fn idle_worker_permit(&self, cancel: &CancellationToken, needed: &CancellationToken) -> Result<WorkerPermit> {
        loop {
            self.check_running(cancel)?;
            // Register before checking capacity: a physical worker can return
            // its permit concurrently. Never enqueue speculative work ahead
            // of readers already waiting in the priority queue.
            let available = self.workers.idle_notification();
            tokio::pin!(available);
            available.as_mut().enable();
            if needed.is_cancelled() { return self.worker_permit(cancel).await; }
            match self.workers.try_idle()? {
                Some(permit) => {
                    self.check_running(cancel)?;
                    return Ok(permit);
                }
                None => {},
            }
            tokio::select! { biased;
                _ = self.stopped.cancelled() => return Err(Error::Cancelled),
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                _ = needed.cancelled() => return self.worker_permit(cancel).await,
                _ = &mut available => {},
            }
        }
    }
    async fn spawn_cpu<T, F>(&self, cancel: &CancellationToken, operation: F) -> Result<T>
    where T: Send + 'static, F: FnOnce() -> Result<T> + Send + 'static {
        self.check_running(cancel)?;
        let task = self.work_token()?;
        let cancel = cancel.clone(); let stopped = self.stopped.clone();
        self.runtime.spawn_blocking(move || {
            let _task = task;
            let result = {
                // Drop queued input/permit captures before publishing physical
                // completion, including when cancellation skips the operation.
                let operation = operation;
                if cancel.is_cancelled() || stopped.is_cancelled() { Err(Error::Cancelled) }
                else { operation() }
            };
            if cancel.is_cancelled() || stopped.is_cancelled() { return Err(Error::Cancelled); }
            result
        }).await?
    }
    async fn cached<K, V, F, Fut>(self: &Arc<Self>, cache: &Arc<WorkCache<K, V>>, key: K, retain: bool, cancel: &CancellationToken, load: F) -> Result<Arc<V>>
    where K: Eq + Hash + Clone + Send + Sync + 'static, V: Weighted + Send + Sync + 'static,
        F: FnOnce(Arc<Self>, CancellationToken) -> Fut + Send + 'static, Fut: Future<Output = Result<V>> + Send + 'static {
        self.check_running(cancel)?;
        // Register before the cache can dispatch a producer. A queued loader
        // is owned even if the executor has not polled its first instruction.
        let task = self.work_token()?;
        let demand = WorkDemand::new(false);
        if let Some(parent) = WorkDemand::current() { parent.link(&demand)?; }
        let service = self.clone();
        let load = move |observer: CancellationToken| async move {
            let _task = task;
            let work = service.stopped.child_token();
            let _cancel_on_drop = work.clone().drop_guard();
            tokio::select! { biased;
                _ = observer.cancelled() => Err(Error::Cancelled),
                _ = work.cancelled() => Err(Error::Cancelled),
                result = load(service, work.clone()) => result,
            }
        };
        WorkDemand::scope(Some(demand), async {
            if retain { cache.get_or_load_observed(key, &self.runtime, cancel, load).await }
            // Weak flights already drop their loader when unobserved. Its guard
            // cancels the same downstream work without retaining a finished frame.
            else { cache.get_or_load_weak(key, &self.runtime, cancel, move || load(CancellationToken::new())).await }
        }).await
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.stopped.cancel();
        let worker_close = self.workers.close();
        if let Some(worker) = self.animation_worker.lock().map_err(|_| Error::Task("动图工作线程状态不可用".into()))?.upgrade() {
            worker.stop();
        }
        self.tasks.close();
        self.tasks.wait().await;
        // Cancelled loaders may have left a read on SQLite's serial worker.
        // Their closures still own BLOB admission. Drain that queue before
        // declaring resource teardown complete; no producer can enqueue more.
        self.storage.call(|_| Ok(())).await?;
        worker_close?;
        self.trim()
    }

    pub(crate) async fn run_cpu<T, F>(&self, scratch_bytes: u64, cancel: &CancellationToken, operation: F) -> Result<T>
    where T: Send + 'static, F: FnOnce() -> Result<T> + Send + 'static {
        let permit = self.worker_permit(cancel).await?;
        let scratch = self.scratch_budget.reserve(scratch_bytes)?;
        self.spawn_cpu(cancel, move || {
            let _permit = permit;
            let _scratch = scratch;
            operation()
        }).await
    }

    pub async fn import_encoded(self: &Arc<Self>, data: Vec<u8>, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        let allocated = data.capacity() as u64;
        self.import_bytes(Bytes::from(data), allocated, None, cancel).await
    }

    /// Import a complete shared payload without copying it into another Vec.
    /// Callers pass the full allocation, not a small slice of a larger buffer.
    pub async fn import_shared_encoded(self: &Arc<Self>, data: Bytes, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        let allocated = data.len() as u64;
        self.import_bytes(data, allocated, None, cancel).await
    }

    /// Transfer a protocol decoder's existing working-memory lease with its allocation.
    pub(crate) async fn import_reserved_encoded(self: &Arc<Self>, data: Vec<u8>, ingress: Reservation, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        let allocated = data.capacity() as u64;
        self.import_bytes(Bytes::from(data), allocated, Some(ingress), cancel).await
    }

    async fn import_bytes(self: &Arc<Self>, data: Bytes, allocated: u64, ingress: Option<Reservation>, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        self.check_running(cancel)?;
        if data.is_empty() { return Err(Error::Invalid("图片内容为空".into())); }
        // Bound the ingress allocation while hashing/queuing. Only a new
        // identity enters the encoded pool; a duplicate must not evict cached
        // content or require a second persistent lease. Keep Vec capacity,
        // which Bytes::from(Vec) can retain beyond its visible length.
        let ingress = match ingress {
            Some(mut ingress) => {
                if ingress.bytes() < allocated { ingress.grow(allocated - ingress.bytes())?; }
                ingress.shrink(allocated);
                ingress
            },
            None => self.scratch_budget.reserve(allocated)?,
        };
        let (id, data, ingress) = self.run_cpu(0, cancel, move ||
            Ok((Digest(Sha256::digest(&data).into()), data, ingress))).await?;
        self.cached(&self.originals, id, true, cancel, move |service, cancel| async move {
            let _ingress = ingress;
            let reservation = service.reserve_encoded(allocated)?;
            // Header inspection stays off the async executor and runs once per
            // admitted identity, with the original allocation and no copying.
            service.run_cpu(0, &cancel, move ||
                Ok(inspect_hashed_encoded(data, id, reservation))).await
        }).await
    }

    pub async fn import_shared_input(self: &Arc<Self>, input: Arc<EncodedImageInput>, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        self.check_running(cancel)?;
        if input.data.is_empty() { return Err(Error::Invalid("图片内容为空".into())); }
        let id = match input.digest.get().copied() {
            Some(id) => id,
            None => {
                let source = input.clone();
                self.run_cpu(0, cancel, move || {
                    Ok(*source.digest.get_or_init(|| Digest(Sha256::digest(&source.data).into())))
                }).await?
            }
        };
        self.check_running(cancel)?;
        self.cached(&self.originals, id, true, cancel, move |service, _cancel| async move {
            let reservation = service.reserve_encoded(input.data.len() as u64)?;
            Ok(EncodedImage { id, data: input.data.clone(), format: input.format, dimensions: input.dimensions, animation: OnceLock::new(), _reservation: reservation })
        }).await
    }

    pub async fn load_original(self: &Arc<Self>, metadata: ImageMetadata, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        self.cached(&self.originals, metadata.id, true, cancel, move |service, cancel| async move {
            let reservation = service.reserve_encoded(metadata.encoded_bytes)?;
            let (data, reservation) = service.storage.read_image(metadata.id, metadata.encoded_bytes, reservation, &cancel).await?;
            let expected = metadata.id;
            service.run_cpu(0, &cancel, move || {
                let image = inspect_encoded(Bytes::from(data), reservation)?;
                if image.id != expected { return Err(Error::Image("图片原始载荷校验失败".into())); }
                Ok(image)
            }).await
        }).await
    }

    pub async fn acquire_encoded(self: &Arc<Self>, source: ImageSource, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        self.check_running(cancel)?;
        match source {
            ImageSource::Encoded(image) => Ok(image),
            ImageSource::Pixels(pixels) => self.encode_png(pixels, cancel).await,
            ImageSource::Stored(metadata) => self.load_original(metadata, cancel).await,
        }
    }

    pub async fn import_pixels(self: &Arc<Self>, width: u32, height: u32, stride: usize, layout: PixelLayout, alpha: AlphaMode, data: Vec<u8>, cancel: &CancellationToken) -> Result<Arc<PixelBuffer>> {
        self.check_running(cancel)?;
        validate_buffer(width, height, stride, layout, data.len())?;
        let reservation = self.reserve_pixels(data.capacity() as u64)?;
        self.run_cpu(0, cancel, move || Ok(Arc::new(make_pixels(width, height, stride, layout, alpha, data, reservation)))).await
    }

    pub async fn acquire_pixels(self: &Arc<Self>, source: ImageSource, limits: ImageLimits, cancel: &CancellationToken) -> Result<Arc<PixelBuffer>> {
        self.check_running(cancel)?;
        let source = match source {
            ImageSource::Pixels(pixels) => { limits.dimensions(pixels.width, pixels.height)?; return Ok(pixels); }
            source => source,
        };
        let (id, source) = encoded_source(source, limits)?;
        self.check_running(cancel)?;
        let key = DerivedKey::Canonical(id);
        // A materialized canonical result can outlive the larger native decode.
        // Reuse it before loading either the original BLOB or native pixels.
        if let Some(pixels) = self.derived.get_existing(&key, cancel).await? {
            limits.dimensions(pixels.width, pixels.height)?;
            return Ok(pixels);
        }
        let decoded = self.acquire_decoded(id, source, limits, cancel).await?;
        if let Some(pixels) = decoded.rgba8_view() { return Ok(pixels); }
        self.cached(&self.derived, key, true, cancel, move |service, cancel| async move {
            let permit = service.worker_permit(&cancel).await?;
            let reservation = service.reserve_pixels(u64::from(decoded.width) * u64::from(decoded.height) * 4)?;
            service.spawn_cpu(&cancel, move || {
                let _permit = permit;
                // Keep the same image crate conversion used by canonical
                // identity/OCR before native display precision was retained.
                let rgba = match &decoded.pixels {
                    DecodedPixels::Native { image, .. } => image.to_rgba8().into_raw(),
                    DecodedPixels::Cmyk { samples, .. } => cmyk::RgbView::new(samples, decoded.width, decoded.height)?.rgba(),
                    DecodedPixels::Rgba8(_) => return Err(Error::Task("图片解码表示不一致".into())),
                };
                Ok(make_pixels(decoded.width, decoded.height, decoded.width as usize * 4,
                    PixelLayout::Rgba8, AlphaMode::Straight, rgba, reservation))
            }).await
        }).await
    }

    async fn acquire_decoded(self: &Arc<Self>, id: Digest, source: EncodedSource, limits: ImageLimits, cancel: &CancellationToken) -> Result<Arc<DecodedImage>> {
        let decoded = self.cached(&self.decoded, id, true, cancel, move |service, cancel| async move {
            // The normalized encoding may be larger than the decode retention
            // cap. A weak association still reuses its live original pixels,
            // without making the encoded payload retain a full-resolution image.
            if let Some(pixels) = service.derived.get_existing(&DerivedKey::NormalizedPng(id), &cancel).await? {
                limits.dimensions(pixels.width, pixels.height)?;
                return Ok(DecodedImage::canonical_png(&pixels));
            }
            // A retained native decode needs no original BLOB. Shared work is
            // independent of cancellation of an individual waiting view.
            let encoded = match source {
                EncodedSource::Ready(image) => image,
                EncodedSource::Stored(metadata) => service.load_original(metadata, &cancel).await?,
            };
            limits.encoded(encoded.data.len() as u64, encoded.dimensions)?;
            let permit = service.worker_permit(&cancel).await?;
            let (width, height) = encoded.dimensions.ok_or_else(|| Error::Image("无法读取图片尺寸".into()))?;
            limits.dimensions(width, height)?;
            let mut scratch = service.scratch_budget.reserve(encoded.bytes().len() as u64 + u64::from(width) * 64 + 8 * 1024 * 1024)?;
            let decode_service = service.clone();
            service.spawn_cpu(&cancel, move || {
                let _permit = permit;
                let format = encoded.format.ok_or_else(|| Error::Image("不支持的图片格式".into()))?;
                let mut decoder_limits = image::Limits::default();
                decoder_limits.max_image_width = Some(width);
                decoder_limits.max_image_height = Some(height);
                decoder_limits.max_alloc = Some(scratch.bytes());
                if format == ImageFormat::Jpeg {
                    let decoder = jpeg::Decoder::new(encoded.bytes(), &decoder_limits)?;
                    if decoder.is_cmyk() {
                        decode_service.decode_cmyk(decoder, &encoded, (width, height), decoder_limits, &scratch)
                    } else {
                        decode_service.decode_retained(decoder, &encoded, (width, height), decoder_limits, &mut scratch,
                            color::Input::read_jpeg)
                    }
                } else {
                    let mut reader = ImageReader::with_format(Cursor::new(encoded.bytes()), format);
                    reader.limits(decoder_limits.clone());
                    let decoder = reader.into_decoder()?;
                    decode_service.decode_retained(decoder, &encoded, (width, height), decoder_limits, &mut scratch,
                        |decoder, encoded, budget| color::Input::read(decoder, encoded, budget))
                }
            }).await
        }).await?;
        limits.dimensions(decoded.width, decoded.height)?;
        Ok(decoded)
    }

    /// Match the reference's static 640px PNG card pixels without encoding and
    /// decoding that PNG. Original orientation/color metadata is not inherited.
    pub async fn acquire_card_pixels(self: &Arc<Self>, metadata: ImageMetadata, limits: ImageLimits, spec: PixelSpec, cancel: &CancellationToken) -> Result<Arc<PixelBuffer>> {
        validate_spec(spec)?;
        limits.encoded(metadata.encoded_bytes, metadata.width.zip(metadata.height))?;
        let pixels = self.cached(&self.derived, DerivedKey::Card { source: metadata.id, spec }, true, cancel, move |service, cancel| async move {
            let base = service.cached(&service.derived, DerivedKey::CardBase(metadata.id), true, &cancel, move |service, cancel| async move {
                let decoded = service.acquire_decoded(metadata.id, EncodedSource::Stored(metadata), limits, &cancel).await?;
                if let DecodedPixels::Native { image, .. } = &decoded.pixels {
                    if matches!(image.color(), image::ColorType::Rgb32F | image::ColorType::Rgba32F) {
                        // The reference PNG encoder rejects these channels.
                        return Err(Error::Image("图片卡片不支持此颜色格式".into()));
                    }
                }
                let extent = (decoded.width, decoded.height);
                let base_spec = PixelSpec { max_width: 640, max_height: 640, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
                let mut base = service.render_pixels(RenderSource::Card(decoded), base_spec, Orientation::NoTransforms, None, PixelUse::Data, &cancel).await?;
                base.display_source_dimensions = extent;
                Ok(base)
            }).await?;
            let extent = base.display_source_dimensions;
            limits.dimensions(extent.0, extent.1)?;
            let mut result = service.render_pixels(RenderSource::Pixels(base), spec, Orientation::NoTransforms, None, PixelUse::Display, &cancel).await?;
            // Limits concern the original input, even if only the bounded
            // card projection survives in the cache.
            result.display_source_dimensions = extent;
            Ok(result)
        }).await?;
        limits.dimensions(pixels.display_source_dimensions.0, pixels.display_source_dimensions.1)?;
        Ok(pixels)
    }

    /// Display uses the native decode before quantization. Canonical identity,
    /// OCR and protocol normalization materialize unmanaged RGBA8 separately.
    pub async fn acquire_display_pixels(self: &Arc<Self>, source: ImageSource, limits: ImageLimits, spec: PixelSpec, cancel: &CancellationToken) -> Result<DisplayPixels> {
        validate_spec(spec)?;
        let source = match source {
            ImageSource::Pixels(pixels) => {
                limits.dimensions(pixels.width, pixels.height)?;
                let natural_dimensions = (pixels.width, pixels.height);
                let pixels = self.derive_display_pixels(pixels, spec, cancel).await?;
                return Ok(DisplayPixels { pixels, natural_dimensions });
            }
            source => source,
        };
        let (id, source) = encoded_source(source, limits)?;
        // Encoded identity includes orientation, color metadata, sample depth
        // and pixels. Domain-separated keys cannot collide with canonical data.
        let pixels = self.cached(&self.derived, DerivedKey::Display { source: id, spec }, true, cancel, move |service, cancel| async move {
            let decoded = service.acquire_decoded(id, source, limits, &cancel).await?;
            let orientation = decoded.orientation.clone()?;
            let color = decoded.color.clone()?;
            let source = match decoded.rgba8_view() {
                Some(pixels) => RenderSource::Pixels(pixels),
                None => RenderSource::Native(decoded),
            };
            service.render_pixels(source, spec, orientation, color, PixelUse::Display, &cancel).await
        }).await?;
        let natural_dimensions = pixels.display_source_dimensions;
        limits.dimensions(natural_dimensions.0, natural_dimensions.1)?;
        Ok(DisplayPixels { pixels, natural_dimensions })
    }

    pub async fn derive_pixels(self: &Arc<Self>, pixels: Arc<PixelBuffer>, spec: PixelSpec, cancel: &CancellationToken) -> Result<Arc<PixelBuffer>> {
        self.derive_for(pixels, spec, cancel, PixelUse::Data).await
    }

    /// Already normalized/native pixels need no encoded round trip or metadata
    /// pass. Alpha-bearing display output is premultiplied for GPU filtering.
    pub async fn derive_display_pixels(self: &Arc<Self>, pixels: Arc<PixelBuffer>, spec: PixelSpec, cancel: &CancellationToken) -> Result<Arc<PixelBuffer>> {
        self.derive_for(pixels, spec, cancel, PixelUse::Display).await
    }

    async fn derive_for(self: &Arc<Self>, pixels: Arc<PixelBuffer>, spec: PixelSpec, cancel: &CancellationToken, usage: PixelUse) -> Result<Arc<PixelBuffer>> {
        self.check_running(cancel)?;
        validate_spec(spec)?;
        if unchanged_pixels(&pixels, spec, usage) { return Ok(pixels); }
        let key = DerivedKey::Pixels { source: pixels.id, spec, usage };
        self.cached(&self.derived, key, true, cancel, move |service, cancel| async move {
            service.render_pixels(RenderSource::Pixels(pixels), spec, Orientation::NoTransforms, None, usage, &cancel).await
        }).await
    }

    async fn render_pixels(self: &Arc<Self>, source: RenderSource, spec: PixelSpec, orientation: Orientation, color: Option<Arc<color::Input>>, usage: PixelUse, cancel: &CancellationToken) -> Result<PixelBuffer> {
        self.check_running(cancel)?;
        validate_spec(spec)?;
        if color.is_none() && orientation == Orientation::NoTransforms {
            if let Some(pixels) = source.pixels().filter(|pixels| unchanged_pixels(pixels, spec, usage)) {
                // A separate result may carry its own display extent while
                // sharing both immutable bytes and their single budget lease.
                let mut result = pixels.as_ref().clone();
                result.display_source_dimensions = (pixels.width, pixels.height);
                return Ok(result);
            }
        }
        let (source_width, source_height) = source.dimensions();
        let permit = self.worker_permit(cancel).await?;
        let (display_width, display_height) = oriented_dimensions(source_width, source_height, orientation);
        let ratio = (f64::from(spec.max_width) / f64::from(display_width)).min(f64::from(spec.max_height) / f64::from(display_height)).min(1.0);
        let resize_width = (f64::from(source_width) * ratio).round().max(1.0) as u32;
        let resize_height = (f64::from(source_height) * ratio).round().max(1.0) as u32;
        let (width, height) = oriented_dimensions(resize_width, resize_height, orientation);
        let reservation = self.reserve_pixels(u64::from(width) * u64::from(height) * u64::from(spec.layout.channels()))?;
        let resized = resize_width != source_width || resize_height != source_height;
        let native = source.pixels().is_none();
        let rgba_output = matches!(spec.layout, PixelLayout::Rgba8 | PixelLayout::Bgra8);
        let premultiplied_output = usage.alpha(spec.layout) == AlphaMode::Premultiplied;
        let preserve_premultiplied = premultiplied_output && color.is_none()
            && source.pixels().is_some_and(|pixels| pixels.alpha == AlphaMode::Premultiplied);
        let coverage_resize = premultiplied_output && resized && spec.filter == ResizeFilter::Triangle;
        let uses_rgba = resized || orientation != Orientation::NoTransforms || color.is_some() || native || preserve_premultiplied;
        let borrowed_rgba = color.is_none() && source.pixels().is_some_and(|pixels| {
            pixels.layout == PixelLayout::Rgba8 && (pixels.alpha == AlphaMode::Straight || preserve_premultiplied)
                && pixels.stride == source_width as usize * 4
        });
        let moves_into_output = !resized && orientation == Orientation::NoTransforms && rgba_output;
        let source_conversion = if uses_rgba && !borrowed_rgba && !moves_into_output { u64::from(source_width) * u64::from(source_height) * 4 } else { 0 };
        let resize_work = if resized {
            // image::resize's vertical RGBA32F pass coexists with the source.
            u64::from(source_width) * u64::from(resize_height) * 16
                + if coverage_resize { u64::from(width) * u64::from(height) * 8 }
                    else if orientation != Orientation::NoTransforms || !rgba_output { u64::from(width) * u64::from(height) * 4 } else { 0 }
                // Sampling weights grow along either source axis, separately
                // from the image buffers (including for the nearest filter).
                + u64::from(source_width.max(source_height)) * 8
        } else { 0 };
        let color_work = color.as_ref().map_or(0, |profile| {
            if matches!(&source, RenderSource::Native(decoded) if matches!(&decoded.pixels, DecodedPixels::Native { .. })) {
                profile.working_high_bytes(source_width)
            } else { profile.working_bytes(source_width) }
        });
        let scratch = self.scratch_budget.reserve(source_conversion + resize_work + color_work + source.intermediate_bytes())?;
        self.spawn_cpu(cancel, move || {
            let _permit = permit;
            let _scratch = scratch;
            let mut data = if uses_rgba {
                let source_bytes = match &source {
                    RenderSource::Pixels(pixels) => {
                        let mut bytes = if preserve_premultiplied {
                            if borrowed_rgba { Cow::Borrowed(pixels.bytes()) }
                            else { Cow::Owned(convert_pixels_inner(pixels, PixelLayout::Rgba8, AlphaMode::Premultiplied)) }
                        } else { rgba_bytes(pixels)? };
                        if let Some(profile) = color.as_ref() { profile.apply(bytes.to_mut(), source_width)?; }
                        bytes
                    }
                    RenderSource::Native(decoded) => {
                        Cow::Owned(match &decoded.pixels {
                            DecodedPixels::Native { image, .. } => match color.as_ref() {
                                Some(profile) => profile.apply_high(image)?,
                                None => image.to_rgba8().into_raw(),
                            },
                            DecodedPixels::Cmyk { samples, .. } => match color.as_ref() {
                                Some(profile) => profile.apply_cmyk(samples, source_width)?,
                                None => cmyk::RgbView::new(samples, source_width, source_height)?.rgba(),
                            },
                            DecodedPixels::Rgba8(_) => return Err(Error::Task("图片解码表示不一致".into())),
                        })
                    }
                    RenderSource::Card(decoded) => Cow::Owned(match &decoded.pixels {
                        DecodedPixels::Rgba8(pixels) => {
                            let rgba = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(decoded.width, decoded.height, pixels.bytes())
                                .ok_or_else(|| Error::Image("卡片图片输入无效".into()))?;
                            imageops::thumbnail(&rgba, source_width, source_height).into_raw()
                        }
                        DecodedPixels::Native { image, .. } => image.thumbnail_exact(source_width, source_height).into_rgba8().into_raw(),
                        DecodedPixels::Cmyk { samples, .. } => {
                            // A borrowed RGB view supplies the reference's
                            // thumbnail pixels without a full RGB allocation.
                            let rgb = cmyk::RgbView::new(samples, decoded.width, decoded.height)?;
                            image::DynamicImage::ImageRgb8(imageops::thumbnail(&rgb, source_width, source_height)).into_rgba8().into_raw()
                        }
                    }),
                };
                let rgba = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(source_width, source_height, source_bytes.as_ref())
                    .ok_or_else(|| Error::Image("显示像素输入无效".into()))?;
                if coverage_resize {
                    sampling::resize_display(&rgba, resize_width, resize_height, spec.layout, orientation, preserve_premultiplied)
                } else if resized {
                    let rgba = imageops::resize(&rgba, resize_width, resize_height, match spec.filter {
                        ResizeFilter::Triangle => FilterType::Triangle,
                        ResizeFilter::Nearest => FilterType::Nearest,
                        ResizeFilter::Lanczos3 => FilterType::Lanczos3,
                    });
                    if orientation != Orientation::NoTransforms { orient_rgba(&rgba, spec.layout, orientation) } else { match spec.layout {
                        PixelLayout::Rgba8 => rgba.into_raw(),
                        PixelLayout::Bgra8 => { let mut data = rgba.into_raw(); for pixel in data.chunks_exact_mut(4) { pixel.swap(0, 2); } data }
                        PixelLayout::Rgb8 => image::DynamicImage::ImageRgba8(rgba).into_rgb8().into_raw(),
                        PixelLayout::Gray8 => image::DynamicImage::ImageRgba8(rgba).into_luma8().into_raw(),
                    } }
                } else if orientation != Orientation::NoTransforms { orient_rgba(&rgba, spec.layout, orientation) }
                else { match spec.layout {
                    PixelLayout::Rgba8 => source_bytes.into_owned(),
                    PixelLayout::Bgra8 => { let mut data = source_bytes.into_owned(); for pixel in data.chunks_exact_mut(4) { pixel.swap(0, 2); } data }
                    PixelLayout::Rgb8 | PixelLayout::Gray8 => orient_rgba(&rgba, spec.layout, Orientation::NoTransforms),
                } }
            } else {
                let RenderSource::Pixels(pixels) = &source else { return Err(Error::Task("图片派生表示不一致".into())); };
                convert_pixels_inner(pixels, spec.layout, usage.alpha(spec.layout))
            };
            if premultiplied_output && uses_rgba && !coverage_resize && !preserve_premultiplied {
                for pixel in data.chunks_exact_mut(4) {
                    let alpha = u16::from(pixel[3]);
                    for component in &mut pixel[..3] { *component = ((u16::from(*component) * alpha + 127) / 255) as u8; }
                }
            }
            let alpha = usage.alpha(spec.layout);
            let mut result = make_pixels(width, height, width as usize * spec.layout.channels() as usize, spec.layout, alpha, data, reservation);
            result.display_source_dimensions = (display_width, display_height);
            Ok(result)
        }).await
    }

    /// Canonical RGBA8 PNG, shared by persistence and protocol adapters that require normalization.
    pub async fn encode_png(self: &Arc<Self>, pixels: Arc<PixelBuffer>, cancel: &CancellationToken) -> Result<Arc<EncodedImage>> {
        self.check_running(cancel)?;
        // Keep the post-encoding identity associations inside shutdown's drain,
        // so they cannot reintroduce strong retention after the final trim.
        let _task = self.work_token()?;
        let original_pixels = pixels.clone();
        let encoded = self.cached(&self.png, pixels.id, true, cancel, move |service, cancel| async move {
            let permit = service.worker_permit(&cancel).await?;
            // A bounded writer rejects unexpectedly large encodings before unbounded Vec growth.
            let encoded_limit = 64 * 1024 * 1024;
            let reservation = service.reserve_encoded(0)?;
            let conversion_bytes = if pixels.layout == PixelLayout::Rgba8 && pixels.alpha == AlphaMode::Straight && pixels.stride == pixels.width as usize * 4 { 0 }
                else { u64::from(pixels.width) * u64::from(pixels.height) * 4 };
            let scratch = service.scratch_budget.reserve(conversion_bytes + u64::from(pixels.width) * 4 * 8 + 1024 * 1024)?;
            let reclaim_service = service.clone();
            service.spawn_cpu(&cancel, move || {
                let _permit = permit;
                let _scratch = scratch;
                let rgba = rgba_bytes(&pixels)?;
                let mut writer = BoundedWriter::new(reservation, encoded_limit)?.with_reclaim(move || {
                    reclaim_service.originals.trim()?;
                    reclaim_service.png.trim()
                });
                image::codecs::png::PngEncoder::new(&mut writer).write_image(&rgba, pixels.width, pixels.height, image::ExtendedColorType::Rgba8)?;
                let (data, reservation) = writer.into_parts()?;
                inspect_encoded(Bytes::from(data), reservation)
            }).await
        }).await?;
        if original_pixels.layout == PixelLayout::Rgba8 && original_pixels.alpha == AlphaMode::Straight && original_pixels.stride == original_pixels.width as usize * 4 {
            // Associate native pixels with the new persisted encoding without decoding that encoding.
            self.derived.share_weak(DerivedKey::NormalizedPng(encoded.id), &original_pixels)?;
            self.decoded.share(encoded.id, Arc::new(DecodedImage::canonical_png(&original_pixels)))?;
        }
        self.originals.share(encoded.id, encoded)
    }

    fn decode_cmyk(
        &self,
        mut decoder: jpeg::Decoder<'_>,
        encoded: &EncodedImage,
        (width, height): (u32, u32),
        mut limits: image::Limits,
        scratch: &Reservation,
    ) -> Result<DecodedImage> {
        if decoder.dimensions() != (width, height) { return Err(Error::Image("图片尺寸校验失败".into())); }
        let retained_bytes = u64::from(width) * u64::from(height) * 4;
        let reservation = self.reserve_pixels(retained_bytes)?;
        limits.max_alloc = Some(scratch.bytes() + retained_bytes);
        decoder.set_limits(limits)?;
        let orientation = decoder.orientation().map_err(Error::from);
        let color = color::Input::read_jpeg(&mut decoder, encoded, &self.pixel_budget)?;
        let length = usize::try_from(retained_bytes).map_err(|_| Error::ResourceLimit("CMYK 像素大小超出限制".into()))?;
        let mut samples = vec![0_u8; length];
        decoder.read_cmyk(&mut samples)?;
        let pixels = DecodedPixels::Cmyk { samples: Bytes::from(samples), reservation };
        Ok(DecodedImage { width, height, pixels, orientation, color })
    }

    fn decode_retained<D: ImageDecoder>(
        &self,
        mut decoder: D,
        encoded: &EncodedImage,
        (width, height): (u32, u32),
        mut limits: image::Limits,
        scratch: &mut Reservation,
        read_color: impl FnOnce(&mut D, &EncodedImage, &ByteBudget) -> Result<Result<Option<Arc<color::Input>>>>,
    ) -> Result<DecodedImage> {
        if decoder.dimensions() != (width, height) { return Err(Error::Image("图片尺寸校验失败".into())); }
        let high_precision = matches!(decoder.color_type(), image::ColorType::L16 | image::ColorType::La16
            | image::ColorType::Rgb16 | image::ColorType::Rgba16 | image::ColorType::Rgb32F | image::ColorType::Rgba32F);
        let retained_bytes = if high_precision { decoder.total_bytes() } else { u64::from(width) * u64::from(height) * 4 };
        let reservation = self.reserve_pixels(retained_bytes)?;
        // High precision stays in its native allocation. Only an 8-bit layout
        // conversion needs a coexisting decoder output buffer. Both decoder
        // adapters consume this same admitted path, without an extra trait box.
        if !high_precision && decoder.color_type() != image::ColorType::Rgba8 { scratch.grow(decoder.total_bytes())?; }
        limits.max_alloc = Some(scratch.bytes() + retained_bytes);
        decoder.set_limits(limits)?;
        let orientation = decoder.orientation().map_err(Error::from);
        // Format-specific extraction uses this decoder after pixel admission;
        // JPEG can admit borrowed ICC chunks before allocating their assembly.
        let color = read_color(&mut decoder, encoded, &self.pixel_budget)?;
        let image = image::DynamicImage::from_decoder(decoder)?;
        if image.width() != width || image.height() != height { return Err(Error::Image("图片尺寸校验失败".into())); }
        let pixels = if high_precision { DecodedPixels::Native { image, reservation } } else {
            DecodedPixels::Rgba8(Arc::new(make_pixels(width, height, width as usize * 4,
                PixelLayout::Rgba8, AlphaMode::Straight, image.into_rgba8().into_raw(), reservation)))
        };
        Ok(DecodedImage { width, height, pixels, orientation, color })
    }

    fn reserve_encoded(&self, size: u64) -> Result<Reservation> {
        match self.encoded_budget.reserve(size) {
            Ok(reservation) => Ok(reservation), Err(_) => { self.originals.trim()?; self.png.trim()?; self.encoded_budget.reserve(size) }
        }
    }
    fn reserve_pixels(&self, size: u64) -> Result<Reservation> {
        match self.pixel_budget.reserve(size) {
            Ok(reservation) => Ok(reservation), Err(_) => { self.decoded.trim()?; self.derived.trim()?; self.pixel_budget.reserve(size) }
        }
    }
}

fn encoded_source(source: ImageSource, limits: ImageLimits) -> Result<(Digest, EncodedSource)> {
    match source {
        ImageSource::Encoded(image) => {
            limits.encoded(image.data.len() as u64, image.dimensions)?;
            Ok((image.id, EncodedSource::Ready(image)))
        }
        ImageSource::Stored(metadata) => {
            limits.encoded(metadata.encoded_bytes, metadata.width.zip(metadata.height))?;
            Ok((metadata.id, EncodedSource::Stored(metadata)))
        }
        ImageSource::Pixels(_) => Err(Error::Task("像素来源未经过直接分支".into())),
    }
}
fn validate_spec(spec: PixelSpec) -> Result<()> {
    if spec.max_width == 0 || spec.max_height == 0 { return Err(Error::Invalid("目标图片尺寸无效".into())); }
    Ok(())
}
fn unchanged_pixels(pixels: &PixelBuffer, spec: PixelSpec, usage: PixelUse) -> bool {
    let alpha_matches = pixels.alpha == usage.alpha(spec.layout);
    pixels.width <= spec.max_width && pixels.height <= spec.max_height && pixels.layout == spec.layout
        && alpha_matches && pixels.stride == pixels.width as usize * pixels.layout.channels() as usize
}

fn inspect_encoded(data: Bytes, reservation: Reservation) -> Result<EncodedImage> {
    if data.is_empty() { return Err(Error::Invalid("图片内容为空".into())); }
    let id = Digest(Sha256::digest(&data).into());
    Ok(inspect_hashed_encoded(data, id, reservation))
}
fn inspect_hashed_encoded(data: Bytes, id: Digest, reservation: Reservation) -> EncodedImage {
    let (format, dimensions) = inspect_header(&data);
    EncodedImage { id, data, format, dimensions, animation: OnceLock::new(), _reservation: reservation }
}
fn inspect_header(data: &[u8]) -> (Option<ImageFormat>, Option<(u32, u32)>) {
    let format = image::guess_format(data).ok();
    // Header inspection must not construct a JPEG decoder, which copies the whole encoded input.
    let dimensions = imagesize::blob_size(data).ok().and_then(|size| Some((u32::try_from(size.width).ok()?, u32::try_from(size.height).ok()?)));
    (format, dimensions)
}
fn validate_buffer(width: u32, height: u32, stride: usize, layout: PixelLayout, bytes: usize) -> Result<()> {
    ImageLimits::default().dimensions(width, height)?;
    let row_bytes = (width as usize).checked_mul(layout.channels() as usize).ok_or_else(|| Error::Invalid("像素行宽溢出".into()))?;
    if stride < row_bytes || stride.checked_mul(height as usize) != Some(bytes) { return Err(Error::Invalid("像素缓冲区的尺寸或步长无效".into())); }
    Ok(())
}
fn make_pixels(width: u32, height: u32, stride: usize, layout: PixelLayout, alpha: AlphaMode, data: Vec<u8>, reservation: Reservation) -> PixelBuffer {
    let mut hash = Sha256::new();
    hash.update(b"super-paste-rs-pixels-v1\0");
    hash.update(width.to_be_bytes()); hash.update(height.to_be_bytes()); hash.update((stride as u64).to_be_bytes());
    hash.update([layout as u8, alpha as u8]); hash.update(&data);
    PixelBuffer { id: Digest(hash.finalize().into()), width, height, stride, layout, alpha, data: Bytes::from(data), _reservation: Arc::new(reservation), _decoded_owner: None, display_source_dimensions: (width, height) }
}

fn oriented_dimensions(width: u32, height: u32, orientation: Orientation) -> (u32, u32) {
    match orientation {
        Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH => (height, width),
        _ => (width, height),
    }
}

fn card_dimensions(width: u32, height: u32) -> (u32, u32) {
    // image::DynamicImage::thumbnail(640, 640) also expands smaller images.
    let ratio = (640.0 / f64::from(width)).min(640.0 / f64::from(height));
    ((f64::from(width) * ratio).round().max(1.0) as u32, (f64::from(height) * ratio).round().max(1.0) as u32)
}

/// Write the requested channel layout directly while mapping EXIF coordinates.
/// Both a borrowed decoded buffer and a resized buffer use this path; no rotated
/// intermediate or image encoding is needed.
fn orient_rgba(source: &impl image::GenericImageView<Pixel = image::Rgba<u8>>, target: PixelLayout, orientation: Orientation) -> Vec<u8> {
    let (source_width, source_height) = source.dimensions();
    let (width, height) = oriented_dimensions(source_width, source_height, orientation);
    let mut data = Vec::with_capacity(width as usize * height as usize * target.channels() as usize);
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = match orientation {
                Orientation::NoTransforms => (x, y),
                Orientation::Rotate90 => (y, source_height - 1 - x),
                Orientation::Rotate180 => (source_width - 1 - x, source_height - 1 - y),
                Orientation::Rotate270 => (source_width - 1 - y, x),
                Orientation::FlipHorizontal => (source_width - 1 - x, y),
                Orientation::FlipVertical => (x, source_height - 1 - y),
                Orientation::Rotate90FlipH => (y, x),
                Orientation::Rotate270FlipH => (source_width - 1 - y, source_height - 1 - x),
            };
            let rgba = source.get_pixel(sx, sy).0;
            match target {
                PixelLayout::Rgba8 => data.extend_from_slice(&rgba),
                PixelLayout::Bgra8 => data.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]),
                PixelLayout::Rgb8 => data.extend_from_slice(&rgba[..3]),
                PixelLayout::Gray8 => data.push(image::Rgba(rgba).to_luma()[0]),
            }
        }
    }
    data
}
fn rgba_image(pixels: &PixelBuffer) -> Result<image::RgbaImage> {
    let data = convert_pixels(pixels, PixelLayout::Rgba8);
    image::RgbaImage::from_raw(pixels.width, pixels.height, data).ok_or_else(|| Error::Image("像素缓冲区大小无效".into()))
}
fn convert_pixels(pixels: &PixelBuffer, target: PixelLayout) -> Vec<u8> {
    convert_pixels_inner(pixels, target, AlphaMode::Straight)
}
fn convert_pixels_inner(pixels: &PixelBuffer, target: PixelLayout, alpha_mode: AlphaMode) -> Vec<u8> {
    let mut data = Vec::with_capacity(pixels.width as usize * pixels.height as usize * target.channels() as usize);
    for row in pixels.data.chunks_exact(pixels.stride) {
        for source in row[..pixels.width as usize * pixels.layout.channels() as usize].chunks_exact(pixels.layout.channels() as usize) {
            let mut rgba = match pixels.layout {
                PixelLayout::Rgba8 => [source[0], source[1], source[2], source[3]],
                PixelLayout::Bgra8 => [source[2], source[1], source[0], source[3]],
                PixelLayout::Rgb8 => [source[0], source[1], source[2], 255],
                PixelLayout::Gray8 => [source[0], source[0], source[0], 255],
            };
            if pixels.alpha == AlphaMode::Opaque { rgba[3] = 255; }
            if alpha_mode != AlphaMode::Premultiplied && pixels.alpha == AlphaMode::Premultiplied {
                let alpha = u32::from(rgba[3]);
                for component in &mut rgba[..3] { *component = if alpha == 0 { 0 } else { ((u32::from(*component) * 255 + alpha / 2) / alpha).min(255) as u8 }; }
            } else if alpha_mode == AlphaMode::Premultiplied && pixels.alpha != AlphaMode::Premultiplied {
                let alpha = u16::from(rgba[3]);
                for component in &mut rgba[..3] { *component = ((u16::from(*component) * alpha + 127) / 255) as u8; }
            }
            match target {
                PixelLayout::Rgba8 => data.extend_from_slice(&rgba),
                PixelLayout::Bgra8 => data.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]),
                PixelLayout::Rgb8 => data.extend_from_slice(&rgba[..3]),
                PixelLayout::Gray8 => data.push(image::Rgba(rgba).to_luma()[0]),
            }
        }
    }
    data
}
fn rgba_bytes(pixels: &PixelBuffer) -> Result<Cow<'_, [u8]>> {
    if pixels.layout == PixelLayout::Rgba8 && pixels.alpha == AlphaMode::Straight && pixels.stride == pixels.width as usize * 4 {
        return Ok(Cow::Borrowed(pixels.bytes()));
    }
    Ok(Cow::Owned(rgba_image(pixels)?.into_raw()))
}
