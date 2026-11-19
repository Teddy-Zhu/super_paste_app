use std::{collections::{HashMap, VecDeque}, sync::{Arc, Mutex}, time::Instant};
use gpui_kit::{prelude::*, *};
use super_paste_core::{Result, images::{AnimationCheckpoint, DisplayFrame, ImageAnimation, EncodedImageInput, ImageLimits, ImagePriority, ImageService, PixelLayout, PixelSpec, ResizeFilter}};
use super_paste_format::{Digest, ImageMetadata};
use tokio::{runtime::Handle, sync::{mpsc, Notify}, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use crate::ui::rasters::RasterPin;
use super::super::OverlayView;
use super::super::details::DetailsView;

#[derive(Clone)]
pub(in crate::ui::overlay) enum RasterOwner { Overlay(WeakEntity<OverlayView>), Details(WeakEntity<DetailsView>) }
impl From<WeakEntity<OverlayView>> for RasterOwner { fn from(owner: WeakEntity<OverlayView>) -> Self { Self::Overlay(owner) } }
impl From<WeakEntity<DetailsView>> for RasterOwner { fn from(owner: WeakEntity<DetailsView>) -> Self { Self::Details(owner) } }
impl RasterOwner {
    fn prune(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Overlay(owner) => { let _ = owner.update(cx, |owner, _| owner.prune_rasters(window)); },
            Self::Details(owner) => { let _ = owner.update(cx, |owner, _| owner.prune_rasters(window)); },
        }
    }
}

#[derive(Clone)]
enum Source { Inline(Arc<EncodedImageInput>), Stored(ImageMetadata) }
impl Source {
    fn dimensions(&self) -> Option<(u32, u32)> {
        match self { Self::Inline(input) => input.dimensions(), Self::Stored(image) => image.width.zip(image.height) }
    }
}
enum Work { Load { source: Source, checkpoint: Option<AnimationCheckpoint> }, Poll { animation: Arc<ImageAnimation>, deadline: Option<Instant> } }
struct Request { key: SharedString, generation: u64, spec: PixelSpec, work: Work }
pub(in crate::ui::overlay) struct Response { pub key: SharedString, pub generation: u64, pub pixels: Result<DisplayFrame> }
pub(in crate::ui::overlay) struct LoadedFrame {
    pub id: Digest, pub image: Arc<RenderImage>, pub pin: Option<RasterPin>, pub natural_dimensions: (u32, u32),
    pub animation: Option<Arc<ImageAnimation>>, pub next_frame: Option<Instant>,
    pub checkpoint: Option<AnimationCheckpoint>,
}
enum Frame { Pending, Queued, Ready { id: Digest, image: Arc<RenderImage>, _pin: Option<RasterPin>, animation: Option<Arc<ImageAnimation>>, deadline: Option<Instant>, polling: bool }, Failed(SharedString) }
struct Slot { source: Source, visible: bool, frame: Frame, display_dimensions: Option<(u32, u32)>, generation: u64, checkpoint: Option<AnimationCheckpoint>, refresh: bool }
#[derive(Default)]
enum Queue { #[default] Waiting, Running(mpsc::Sender<Request>), Failed(SharedString) }
#[derive(Default)]
struct State {
    slots: HashMap<SharedString, Slot>, queue: Queue, spec: Option<PixelSpec>,
    visible_order: VecDeque<SharedString>,
    owner: Option<RasterOwner>, reclaim_scheduled: bool,
    playing: bool, notify: Arc<Notify>,
}
#[derive(Clone, Default)]
pub(in crate::ui::overlay) struct Sources(Arc<Mutex<State>>);
/// Element/renderer closures may retain Sources after the displayed document
/// disappears. The last view owner releases payloads independently.
pub(super) struct Owner(Sources);
impl Drop for Owner { fn drop(&mut self) { self.0.close(); } }

/// Original-image details use the same bounded producer, visibility lifecycle
/// and playback clock as inline media, without a text document or second cache.
pub(in crate::ui::overlay) struct StandaloneImage { sources: Sources, key: SharedString, _owner: Arc<Owner> }
impl StandaloneImage {
    pub fn new(source: ImageMetadata, frame: LoadedFrame, spec: PixelSpec) -> Self {
        let sources = Sources::default(); let owner = sources.owner(); let key: SharedString = "superpaste-rs-original-image".into();
        let mut state = sources.0.lock().unwrap_or_else(|poison| poison.into_inner());
        state.spec = Some(spec);
        state.slots.insert(key.clone(), Slot {
            source: Source::Stored(source), visible: true, display_dimensions: Some(frame.natural_dimensions),
            generation: 0, checkpoint: frame.checkpoint, refresh: false,
            frame: Frame::Ready { id: frame.id, image: frame.image, _pin: frame.pin, animation: frame.animation, deadline: frame.next_frame, polling: false },
        });
        state.visible_order.push_back(key.clone());
        drop(state);
        Self { sources, key, _owner: owner }
    }
    pub fn sources(&self) -> Sources { self.sources.clone() }
    pub fn render(&self, p: crate::ui::palette::Palette) -> Div {
        self.sources.set_playing(true);
        let sources = self.sources.clone(); let key = self.key.clone();
        let source = self.sources.resolve(&SharedUri::from(self.key.clone()));
        div().relative().size_full()
            .child(canvas(move |bounds, window, cx| {
                let clipped = bounds.intersect(&window.content_mask().bounds);
                let visible = clipped.size.width > Pixels::ZERO && clipped.size.height > Pixels::ZERO;
                if visible { sources.set_spec(display_spec(bounds.size.width, window.scale_factor())); }
                // Img asks for its source during layout, before this clipped
                // visibility is known. Wake it once when it re-enters view.
                if sources.visible(&key, visible, window, cx) && visible { window.request_animation_frame(); }
            }, |_, _, _, _| {}).absolute().inset_0().size_full())
            .child(img(source).id("details-original-image").absolute().inset_0().size_full().object_fit(ObjectFit::Contain)
                .with_loading(move || div().id("details-image-loading").size_full().flex().items_center().justify_center().role(Role::Status)
                    .text_color(p.text_secondary).text_size(px(12.0)).child("加载中…").into_any_element())
                .with_fallback(move || super::super::preview_content::image_failure("图片无法预览".into(), p).into_any_element()))
    }
}

/// Physical-width bands avoid deriving another raster for every resize pixel.
pub(in crate::ui::overlay) fn display_spec(width: Pixels, scale: f32) -> PixelSpec {
    let width = (f32::from(width).max(1.0) * scale).ceil() as u32;
    PixelSpec { max_width: width.div_ceil(64).clamp(1, 128) * 64, max_height: 8192,
        layout: PixelLayout::Bgra8, filter: ResizeFilter::Triangle }
}
impl Sources {
    pub(super) fn owner(&self) -> Arc<Owner> { Arc::new(Owner(self.clone())) }
    fn close(&self) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        state.queue = Queue::Waiting; state.slots.clear(); state.visible_order.clear(); state.playing = false; state.notify.notify_one();
    }
    pub fn release_visible(&self) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        for slot in state.slots.values_mut() {
            slot.checkpoint = slot.checkpoint.map(AnimationCheckpoint::paused);
            slot.visible = false;
            slot.generation = slot.generation.wrapping_add(1);
            slot.refresh = false;
            if !matches!(slot.frame, Frame::Failed(_)) { slot.frame = Frame::Pending; }
        }
        state.visible_order.clear(); state.playing = false; state.notify.notify_one();
    }
    pub fn set_playing(&self, playing: bool) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if state.playing != playing { state.playing = playing; state.notify.notify_one(); }
    }
    pub fn fail_pending(&self, message: SharedString) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        state.queue = Queue::Failed(message.clone());
        for slot in state.slots.values_mut() {
            if matches!(&slot.frame, Frame::Pending | Frame::Queued | Frame::Ready { animation: Some(_), .. }) { slot.frame = Frame::Failed(message.clone()); }
        }
        state.notify.notify_one();
    }
    pub fn insert(&self, index: usize, source: Arc<EncodedImageInput>, failed: bool) -> Option<SharedString> {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let key = SharedString::from(format!("superpaste-rs-image:{index}"));
        if let Some(slot) = state.slots.get(&key) {
            return matches!(&slot.source, Source::Inline(previous) if Arc::ptr_eq(previous, &source)).then_some(key);
        }
        let frame = if failed { Frame::Failed("无法读取图片尺寸".into()) } else { Frame::Pending };
        state.slots.insert(key.clone(), Slot { source: Source::Inline(source), visible: false, frame, display_dimensions: None, generation: 0, checkpoint: None, refresh: false }); Some(key)
    }
    pub fn natural_size(&self, key: &SharedString) -> Option<Size<Pixels>> {
        let state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let (width, height) = state.slots.get(key)?.display_dimensions?;
        Some(size(px(width as f32), px(height as f32)))
    }
    pub fn has_images(&self) -> bool { self.0.lock().unwrap_or_else(|poison| poison.into_inner()).slots.values().any(|slot| !matches!(slot.frame, Frame::Failed(_))) }
    pub fn visible_pending(&self) -> bool {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner()).slots.values()
            .any(|slot| slot.visible && matches!(slot.frame, Frame::Pending | Frame::Queued))
    }
    /// The producer remains asleep until a clipped native image enters view.
    fn attach(&self, owner: RasterOwner, spec: PixelSpec) -> mpsc::Receiver<Request> {
        let (sender, receiver) = mpsc::channel(1);
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        state.queue = Queue::Running(sender); state.owner = Some(owner);
        drop(state); self.set_spec(spec); receiver
    }
    pub fn set_spec(&self, spec: PixelSpec) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        Self::update_spec(&mut state, spec);
    }
    /// A finished worker cannot consume another size request. Let its owner
    /// restart the failed preview instead of leaving a refresh pending forever.
    pub fn resize_if_running(&self, spec: PixelSpec) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if !matches!(&state.queue, Queue::Running(sender) if !sender.is_closed()) { return false; }
        Self::update_spec(&mut state, spec);
        true
    }
    fn update_spec(state: &mut State, spec: PixelSpec) {
        if state.spec == Some(spec) { return; }
        if state.spec.replace(spec).is_some() {
            for slot in state.slots.values_mut() {
                slot.generation = slot.generation.wrapping_add(1);
                match &mut slot.frame {
                    // Keep the current raster pinned until its replacement is
                    // accepted. Old-size responses cannot overwrite this slot.
                    Frame::Ready { polling, .. } => { *polling = false; slot.refresh = true; },
                    Frame::Queued => { slot.frame = Frame::Pending; slot.refresh = slot.visible; },
                    Frame::Pending => { slot.refresh = slot.visible; },
                    _ => {},
                }
            }
        }
        state.notify.notify_one();
    }
    pub fn visible(&self, key: &SharedString, visible: bool, window: &mut Window, cx: &mut App) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(slot) = state.slots.get_mut(key) else { return false; };
        let changed = slot.visible != visible;
        let released = slot.visible && !visible;
        slot.visible = visible;
        if released {
            slot.checkpoint = slot.checkpoint.map(AnimationCheckpoint::paused);
            slot.generation = slot.generation.wrapping_add(1);
            slot.refresh = false;
            if !matches!(slot.frame, Frame::Failed(_)) { slot.frame = Frame::Pending; }
        }
        if changed {
            if visible { state.visible_order.push_back(key.clone()); }
            else { state.visible_order.retain(|scheduled| scheduled != key); }
            state.notify.notify_one();
        }
        if released && !state.reclaim_scheduled {
            state.reclaim_scheduled = true;
            let sources = self.clone(); let owner = state.owner.clone();
            window.defer(cx, move |window, cx| {
                sources.0.lock().unwrap_or_else(|poison| poison.into_inner()).reclaim_scheduled = false;
                if let Some(owner) = owner { owner.prune(window, cx); }
            });
        }
        changed
    }
    /// A queued decode may finish after scrolling away. Do not admit its raster.
    pub fn wants(&self, key: &SharedString, generation: u64) -> bool {
        let state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(slot) = state.slots.get(key) else { return false; };
        slot.visible && slot.generation == generation
            && matches!(slot.frame, Frame::Queued | Frame::Ready { polling: true, .. })
    }
    pub fn accept(&self, key: SharedString, generation: u64, result: Result<LoadedFrame>) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(slot) = state.slots.get_mut(&key) else { return false; };
        if !slot.visible || slot.generation != generation { return false; }
        slot.refresh = false;
        let mut dimensions_changed = false;
        slot.frame = match result {
            Ok(frame) => {
                dimensions_changed = slot.display_dimensions.or_else(|| slot.source.dimensions()) != Some(frame.natural_dimensions);
                slot.display_dimensions = Some(frame.natural_dimensions);
                slot.checkpoint = frame.checkpoint;
                Frame::Ready { id: frame.id, image: frame.image, _pin: frame.pin, animation: frame.animation, deadline: frame.next_frame, polling: false }
            },
            Err(error) => Frame::Failed(error.to_string().into()),
        };
        state.notify.notify_one();
        dimensions_changed
    }
    fn next_request(&self) -> (Option<Request>, Option<Instant>) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(spec) = state.spec else { return (None, None); };
        let playing = state.playing;
        let State { slots, visible_order, .. } = &mut *state;
        // Missing or resized visible content takes precedence over advancing
        // an existing image. Rotate after each dispatch so a fast/overdue
        // animation cannot monopolize this document's bounded worker.
        for _ in 0..visible_order.len() {
            let key = visible_order.pop_front().expect("visible queue length");
            let Some(slot) = slots.get_mut(&key).filter(|slot| slot.visible) else { continue; };
            visible_order.push_back(key.clone());
            if slot.refresh || matches!(slot.frame, Frame::Pending) {
                slot.refresh = false;
                slot.generation = slot.generation.wrapping_add(1);
                match &mut slot.frame {
                    Frame::Ready { polling, .. } => *polling = true,
                    _ => slot.frame = Frame::Queued,
                }
                return (Some(Request { key, generation: slot.generation, spec,
                    work: Work::Load { source: slot.source.clone(), checkpoint: slot.checkpoint } }), None);
            }
        }
        if !playing { return (None, None); }
        let now = Instant::now(); let mut earliest: Option<Instant> = None;
        for _ in 0..visible_order.len() {
            let key = visible_order.pop_front().expect("visible queue length");
            let Some(slot) = slots.get_mut(&key).filter(|slot| slot.visible) else { continue; };
            visible_order.push_back(key.clone());
            let Frame::Ready { animation: Some(animation), deadline, polling, .. } = &mut slot.frame else { continue; };
            if *polling { continue; }
            if deadline.is_none_or(|deadline| deadline <= now) {
                *polling = true; slot.generation = slot.generation.wrapping_add(1);
                return (Some(Request { key, generation: slot.generation, spec,
                    work: Work::Poll { animation: animation.clone(), deadline: *deadline } }), None);
            }
            if let Some(deadline) = *deadline { earliest = Some(earliest.map_or(deadline, |earliest| earliest.min(deadline))); }
        }
        (None, earliest)
    }
    pub fn resolve(&self, uri: &SharedUri) -> ImageSource {
        let key = (**uri).clone(); let sources = self.clone();
        ImageSource::Custom(Arc::new(move |_, _| {
            let mut state = sources.0.lock().unwrap_or_else(|poison| poison.into_inner());
            let State { slots, queue, spec, .. } = &mut *state;
            let Some(slot) = slots.get_mut(&key) else { return Some(Err(ImageCacheError::Asset("未授权的富文本图片资源".into()))); };
            match &slot.frame {
                Frame::Ready { image, .. } => Some(Ok(image.clone())),
                Frame::Failed(message) => Some(Err(ImageCacheError::Asset(message.clone()))),
                Frame::Pending if slot.visible => {
                    if let Queue::Failed(message) = queue {
                        let message = message.clone(); slot.frame = Frame::Failed(message.clone());
                        return Some(Err(ImageCacheError::Asset(message)));
                    }
                    if let (Queue::Running(queue), Some(spec)) = (queue, *spec) {
                        let generation = slot.generation.wrapping_add(1);
                        match queue.try_send(Request { key: key.clone(), generation, spec, work: Work::Load { source: slot.source.clone(), checkpoint: slot.checkpoint } }) {
                            Ok(()) => { slot.generation = generation; slot.frame = Frame::Queued; slot.refresh = false; },
                            Err(mpsc::error::TrySendError::Full(_)) => {},
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                let message: SharedString = "图片加载任务已停止".into(); slot.frame = Frame::Failed(message.clone());
                                return Some(Err(ImageCacheError::Asset(message)));
                            },
                        }
                    }
                    None
                },
                _ => None,
            }
        }))
    }
    pub fn retain_pixels(&self, alive: &mut std::collections::HashSet<Digest>) {
        alive.extend(self.0.lock().unwrap_or_else(|poison| poison.into_inner()).slots.values().filter_map(|slot| {
            if let Frame::Ready { id, .. } = &slot.frame { Some(*id) } else { None }
        }));
    }
    pub fn failed(&self) -> bool { self.0.lock().unwrap_or_else(|poison| poison.into_inner()).slots.values().any(|slot| matches!(slot.frame, Frame::Failed(_))) }
}

/// One producer and one queued request/response per document. Global image
/// workers and work caches coalesce decoding across visible cards and details.
pub(in crate::ui::overlay) fn load(runtime: &Handle, images: Arc<ImageService>, sources: Sources, owner: impl Into<RasterOwner>, spec: PixelSpec, cancel: CancellationToken, priority: ImagePriority)
    -> (mpsc::Receiver<Response>, JoinHandle<()>)
{
    let mut requests = sources.attach(owner.into(), spec);
    let (sender, receiver) = mpsc::channel(1);
    let notify = sources.0.lock().unwrap_or_else(|poison| poison.into_inner()).notify.clone();
    let task = runtime.spawn(async move {
        images.with_priority(priority, async {
            loop {
                if cancel.is_cancelled() { break; }
                // notify_one keeps a permit if a state change arrives before this
                // document's sole waiter is polled.
                let notified = notify.notified();
                let request = match requests.try_recv() {
                    Ok(request) => request,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                    Err(mpsc::error::TryRecvError::Empty) => {
                        let (request, deadline) = sources.next_request();
                        if let Some(request) = request { request } else {
                            let timer = async move {
                                match deadline { Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                                    None => std::future::pending::<()>().await }
                            };
                            tokio::select! { biased;
                                _ = cancel.cancelled() => break,
                                request = requests.recv() => match request { Some(request) => request, None => break },
                                _ = notified => continue,
                                _ = timer => continue,
                            }
                        }
                    }
                };
                if !sources.wants(&request.key, request.generation) { continue; }
                let spec = request.spec;
                let pixels = match request.work {
                    Work::Load { source, checkpoint } => async {
                        let encoded = match source {
                            Source::Inline(source) => images.import_shared_input(source, &cancel).await?,
                            Source::Stored(source) => images.acquire_encoded(super_paste_core::images::ImageSource::Stored(source), &cancel).await?,
                        };
                        images.acquire_display_frame(encoded, checkpoint, ImageLimits::default(), spec, &cancel).await
                    }.await,
                    Work::Poll { animation, deadline } => animation.poll(deadline, spec, ImageLimits::default(), &cancel).await,
                };
                if cancel.is_cancelled() { break; }
                let response = Response { key: request.key, generation: request.generation, pixels };
                tokio::select! { biased;
                    _ = cancel.cancelled() => break,
                    sent = sender.send(response) => if sent.is_err() { break; },
                }
            }
        }).await
    });
    (receiver, task)
}
