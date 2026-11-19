mod header;
mod apng;

use std::{collections::HashMap, io::Cursor, sync::{Arc, Weak, mpsc, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant}};
use image::{AnimationDecoder, ImageDecoder, ImageFormat, metadata::Orientation};
use sha2::{Digest as _, Sha256};
use super_paste_format::Digest;
use tokio::sync::{Mutex, oneshot};
use tokio_util::{sync::CancellationToken, task::task_tracker::TaskTrackerToken};
use crate::{Error, Result, resources::{ByteBudget, Reservation, Weighted, WorkDemand}};
use super::{AlphaMode, DecodedImage, DecodedPixels, DerivedKey, DisplayPixels, EncodedBytes, EncodedImage, ImageLimits, ImageService, ImageSource,
    PixelBuffer, PixelLayout, PixelSpec, PixelUse, RenderSource, WorkerPermit, color, make_pixels, validate_spec};

#[derive(Clone, Copy)]
pub(super) enum AnimationInfo { Static, Animated { single_frame: bool, gif_plays: Option<u32> } }

/// Logical playback state only: retaining it keeps no pixels or codec alive.
#[derive(Clone, Copy)]
pub struct AnimationCheckpoint {
    source: super_paste_format::Digest, frame_index: u32, plays_left: Option<u32>,
    remaining: Duration, deadline: Option<Instant>, ended: bool,
}
impl AnimationCheckpoint {
    pub fn paused(self) -> Self {
        Self { remaining: self.deadline.map_or(self.remaining, |deadline| deadline.saturating_duration_since(Instant::now())),
            deadline: None, ..self }
    }
}

pub struct DisplayFrame {
    pub pixels: Arc<PixelBuffer>,
    pub natural_dimensions: (u32, u32),
    /// Shared decode position and clock, not a vector containing every frame.
    pub animation: Option<Arc<ImageAnimation>>,
    /// None with an animation means playback has not started yet.
    pub next_frame: Option<Instant>,
    pub checkpoint: Option<AnimationCheckpoint>,
}
impl From<DisplayPixels> for DisplayFrame {
    fn from(display: DisplayPixels) -> Self {
        Self { pixels: display.pixels, natural_dimensions: display.natural_dimensions, animation: None, next_frame: None, checkpoint: None }
    }
}

pub struct ImageAnimation {
    service: Weak<ImageService>, source: super_paste_format::Digest,
    session: Arc<Session>, orientation: Orientation, color: Option<Arc<color::Input>>,
    clock: std::sync::Mutex<Clock>, stepping: Mutex<()>,
    advance_demand: std::sync::Mutex<Weak<WorkDemand>>,
}
struct Clock {
    frame: RawFrame, deadline: Option<Instant>, start_delay: Option<Duration>,
    ended: bool, failed: Option<Error>, next: Option<PendingFrame>,
}
struct PendingFrame { response: oneshot::Receiver<Result<Result<Read>>>, needed: CancellationToken, demand: Arc<WorkDemand> }
enum Playback { Read, Advance { deadline: Option<Instant> } }
struct RawFrame { pixels: FramePixels, duration: Duration, index: u32, plays_left: Option<u32> }
#[derive(Clone)]
enum FramePixels { Eight(Arc<PixelBuffer>), Native { decoded: Arc<DecodedImage>, id: Digest } }
impl FramePixels {
    fn id(&self) -> Digest { match self { Self::Eight(pixels) => pixels.id(), Self::Native { id, .. } => *id } }
    fn dimensions(&self) -> (u32, u32) { match self { Self::Eight(pixels) => (pixels.width, pixels.height), Self::Native { decoded, .. } => (decoded.width, decoded.height) } }
    fn render_source(self) -> RenderSource { match self { Self::Eight(pixels) => RenderSource::Pixels(pixels), Self::Native { decoded, .. } => RenderSource::Native(decoded) } }
}
struct Sample { pixels: Option<FrameData>, duration: Duration }
enum FrameData { Eight(Vec<u8>), Sixteen(Vec<u16>) }
impl Weighted for ImageAnimation { fn weight(&self) -> u64 { 0 } }

impl ImageService {
    /// Inline/original media may animate. Persistent history cards deliberately
    /// keep their independent static-640 projection.
    pub async fn acquire_display_frame(self: &Arc<Self>, encoded: Arc<EncodedImage>, checkpoint: Option<AnimationCheckpoint>, limits: ImageLimits, spec: PixelSpec, cancel: &CancellationToken) -> Result<DisplayFrame> {
        validate_spec(spec)?;
        limits.encoded(encoded.bytes().len() as u64, encoded.dimensions)?;
        self.check_running(cancel)?;
        if checkpoint.is_some_and(|checkpoint| checkpoint.source != encoded.id) { return Err(Error::Invalid("动图播放位置与图片来源不匹配".into())); }
        let checkpoint = checkpoint.map(AnimationCheckpoint::paused);
        let info = match encoded.animation.get() {
            Some(info) => info.clone()?,
            None => { let input = encoded.clone();
                self.run_cpu(0, cancel, move || input.animation.get_or_init(|| header::inspect(&input)).clone()).await?
            }
        };
        if matches!(info, AnimationInfo::Static) {
            return self.acquire_display_pixels(ImageSource::Encoded(encoded), limits, spec, cancel).await.map(Into::into);
        }
        let id = encoded.id;
        let animation = self.cached(&self.animations, id, false, cancel, move |service, cancel| async move {
            let (width, height) = encoded.dimensions.ok_or_else(|| Error::Image("无法读取动图尺寸".into()))?;
            limits.dimensions(width, height)?;
            let bytes_per_pixel = if encoded.format == Some(ImageFormat::Png) && encoded.bytes().get(24) == Some(&16) { 8 } else { 4 };
            let canvas = u64::from(width) * u64::from(height) * bytes_per_pixel;
            let permit = service.worker_permit(&cancel).await?;
            // Native APNG admits Previous regions only when needed. GIF owns
            // one canvas; WebP additionally owns native restore state.
            let resident = service.reserve_pixels(canvas * if encoded.format == Some(ImageFormat::WebP) { 2 } else { 1 })?;
            let decoder_work = service.scratch_budget.reserve(encoded.bytes().len() as u64 + u64::from(width) * 64 + 8 * 1024 * 1024)?;
            let skip_snapshot = encoded.format == Some(ImageFormat::Png) && checkpoint.is_some_and(|checkpoint| checkpoint.frame_index > 0);
            let output = service.reserve_pixels(if skip_snapshot { 0 } else { canvas })?;
            let frame_work = service.scratch_budget.reserve(frame_work_bytes(encoded.format, canvas))?;
            let worker = {
                let mut slot = service.animation_worker.lock().map_err(|_| Error::Task("动图工作线程状态不可用".into()))?;
                // Shutdown cancels before taking this same lock. It must see
                // every codec thread that was admitted before cancellation.
                service.check_running(&cancel)?;
                match slot.upgrade() {
                    Some(worker) => worker,
                    None => { let worker = DecoderWorker::new(service.work_token()?)?; *slot = Arc::downgrade(&worker); worker }
                }
            };
            let session = Arc::new(Session { worker, encoded_bytes: encoded.bytes().len() as u64, id: NEXT_SESSION.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| Error::ResourceLimit("动图会话标识已耗尽".into()))?, canvas, dimensions: (width, height), format: encoded.format });
            let (reply, response) = oneshot::channel();
            session.worker.sender.send(Command::Open(Open {
                id: session.id, encoded, limits, checkpoint, resident, decoder_work, output, frame_work,
                pixel_budget: service.pixel_budget.clone(), permit, reply,
            })).map_err(|_| Error::Task("动图工作线程已停止".into()))?;
            let opened = response.await.map_err(|_| Error::Task("动图解码未返回结果".into()))??;
            let mut frame = opened.frame;
            if let Some(checkpoint) = checkpoint {
                // One command/permit per frame allows other image work to run
                // between replay steps. Skipped frames never enter a cache.
                for index in 1..=checkpoint.frame_index {
                    let last = index == checkpoint.frame_index;
                    match service.animation_step(&session, last, last && checkpoint.ended, None).await?? {
                        Read::Frame(restored) => frame = Some(restored),
                        Read::Skipped => {},
                        Read::End => return Err(Error::Image("动图播放位置超出帧范围".into())),
                    }
                }
            }
            Ok(ImageAnimation { service: Arc::downgrade(&service), source: id, session,
                orientation: opened.orientation, color: opened.color,
                clock: std::sync::Mutex::new(Clock { frame: frame.ok_or_else(|| Error::Task("动图恢复未产生显示帧".into()))?, deadline: None,
                    start_delay: checkpoint.map(|checkpoint| checkpoint.remaining),
                    ended: opened.single_frame || checkpoint.is_some_and(|checkpoint| checkpoint.ended), failed: None, next: None }), stepping: Mutex::new(()),
                advance_demand: std::sync::Mutex::new(Weak::new()) })
        }).await?;
        // Current limits still apply when another view opened the shared stream.
        animation.display(spec, limits, cancel, Playback::Read).await
    }
    // Admission errors leave the codec unchanged and use the outer result.
    // The inner result describes the session: decoder failure, a stopped
    // worker or a lost reply is terminal, even if the UI waiter went away.
    async fn animation_step(&self, session: &Session, emit: bool, finish: bool, needed: Option<&CancellationToken>) -> Result<Result<Read>> {
        self.check_running(&self.stopped)?;
        let permit = match needed {
            Some(needed) => self.idle_worker_permit(&self.stopped, needed).await?,
            None => self.worker_permit(&self.stopped).await?,
        };
        let skip_snapshot = session.format == Some(ImageFormat::Png) && !emit;
        let output = self.reserve_pixels(if skip_snapshot { 0 } else { session.canvas })?;
        let work = self.scratch_budget.reserve(frame_work_bytes(session.format, session.canvas))?;
        let (reply, response) = oneshot::channel();
        if session.worker.sender.send(Command::Next { id: session.id, output, work, permit, emit, finish, reply }).is_err() {
            return Ok(Err(Error::Task("动图工作线程已停止".into())));
        }
        Ok(response.await.unwrap_or_else(|_| Err(Error::Task("动图解码未返回结果".into()))))
    }

    // One receiver belongs to the source clock, never one per view. Its closed
    // signal cancels admission when the last animation owner disappears. A
    // dispatched command still owns its codec allocations until it finishes.
    fn prefetch_animation_frame(self: &Arc<Self>, session: &Arc<Session>) -> PendingFrame {
        let (mut reply, response) = oneshot::channel();
        let needed = CancellationToken::new(); let demand = needed.clone();
        // A speculative read has no interactive demand until a reader needs
        // its result. The same context is promoted when the deadline arrives.
        let priority = WorkDemand::new(false); let work_priority = priority.clone();
        let service = self.clone(); let session = session.clone();
        let task = self.work_token();
        self.runtime.spawn(WorkDemand::scope(Some(work_priority), async move {
            let _task = match task { Ok(task) => task, Err(error) => { let _ = reply.send(Err(error)); return; } };
            tokio::select! { biased;
                _ = reply.closed() => {},
                _ = service.stopped.cancelled() => { let _ = reply.send(Err(Error::Cancelled)); },
                result = service.animation_step(&session, true, false, Some(&demand)) => { let _ = reply.send(result); },
            }
        }));
        PendingFrame { response, needed, demand: priority }
    }
}

impl ImageAnimation {
    /// Called at the visible view's next deadline. Multiple views share one
    /// source clock. Supply the deadline returned with the displayed frame,
    /// including None before playback starts. A queued request for an older
    /// deadline only reads the current frame. No frame work runs merely because
    /// a stream is retained.
    pub async fn poll(self: &Arc<Self>, deadline: Option<Instant>, spec: PixelSpec, limits: ImageLimits, cancel: &CancellationToken) -> Result<DisplayFrame> {
        self.display(spec, limits, cancel, Playback::Advance { deadline }).await
    }
    async fn display(self: &Arc<Self>, spec: PixelSpec, limits: ImageLimits, cancel: &CancellationToken, playback: Playback) -> Result<DisplayFrame> {
        validate_spec(spec)?;
        let service = self.service.upgrade().ok_or(Error::Cancelled)?;
        limits.encoded(self.session.encoded_bytes, Some(self.session.dimensions))?;
        service.check_running(cancel)?;
        if let Playback::Advance { deadline } = playback {
            let animation = self.clone(); let producer = service.clone();
            let task = service.work_token()?;
            let demand = {
                let mut pending = self.advance_demand.lock().map_err(|_| Error::Task("动图任务优先级状态不可用".into()))?;
                let demand = match pending.upgrade() {
                    Some(demand) => demand,
                    None => { let demand = WorkDemand::new(false); *pending = Arc::downgrade(&demand); demand },
                };
                // Overlapping clock readers share demand before waiting on
                // stepping. A detail can promote a card's already-waiting
                // successor even while that card owns the stepping mutex.
                if let Some(parent) = WorkDemand::current() { parent.link(&demand)?; }
                demand
            };
            // A detached producer commits state even if the view's task is
            // aborted while the thread-local iterator is decoding a frame.
            let step = service.runtime.spawn(WorkDemand::scope(Some(demand), async move { let _task = task; animation.advance(&producer, deadline).await }));
            tokio::select! { biased; _ = cancel.cancelled() => return Err(Error::Cancelled),
                result = step => result?? }
        }
        let (pixels, next_frame, ended, checkpoint) = {
            let clock = self.clock.lock().map_err(|_| Error::Task("动图时钟状态不可用".into()))?;
            if let Some(error) = &clock.failed { return Err(error.clone()); }
            limits.encoded(self.session.encoded_bytes, Some(clock.frame.pixels.dimensions()))?;
            let checkpoint = AnimationCheckpoint { source: self.source, frame_index: clock.frame.index, plays_left: clock.frame.plays_left,
                remaining: clock.start_delay.unwrap_or(clock.frame.duration), deadline: clock.deadline, ended: clock.ended };
            (clock.frame.pixels.clone(), clock.deadline, clock.ended, checkpoint)
        };
        let frame = pixels.id();
        service.check_running(cancel)?;
        let orientation = self.orientation; let color = self.color.clone();
        let pixels = service.cached(&service.derived, DerivedKey::Animation { source: self.source, frame, spec }, false, cancel, move |service, cancel| async move {
            service.render_pixels(pixels.render_source(), spec, orientation, color, PixelUse::Display, &cancel).await
        }).await?;
        Ok(DisplayFrame { natural_dimensions: pixels.display_source_dimensions, pixels,
            animation: (!ended).then(|| self.clone()), next_frame, checkpoint: Some(checkpoint) })
    }
    async fn advance(self: &Arc<Self>, service: &Arc<ImageService>, deadline: Option<Instant>) -> Result<()> {
        let _step = self.stepping.lock().await;
        // Fix the catch-up target before awaiting decoding. Slow codecs cannot
        // extend this request's target indefinitely while it is doing work.
        let tick = Instant::now();
        let mut next_deadline = {
            let mut clock = self.clock.lock().map_err(|_| Error::Task("动图时钟状态不可用".into()))?;
            if let Some(error) = &clock.failed { return Err(error.clone()); }
            if clock.ended || clock.deadline != deadline { return Ok(()); }
            match clock.deadline {
                None => {
                    let delay = clock.start_delay.take().unwrap_or(clock.frame.duration);
                    clock.deadline = Some(tick + delay);
                    clock.next = Some(service.prefetch_animation_frame(&self.session));
                    return Ok(());
                }
                Some(deadline) if tick < deadline => return Ok(()),
                Some(deadline) => deadline,
            }
        };
        loop {
            // Readers can continue showing the current frame while its sole
            // successor is decoded. No std mutex guard crosses this await.
            let next = {
                let mut clock = self.clock.lock().map_err(|_| Error::Task("动图时钟状态不可用".into()))?;
                clock.next.take().unwrap_or_else(|| service.prefetch_animation_frame(&self.session))
            };
            // The successor is now needed for display. Promote this same
            // pending read into normal admission; do not spawn another decode.
            if let Some(parent) = WorkDemand::current() { parent.link(&next.demand)?; }
            next.needed.cancel();
            let next = next.response.await.unwrap_or_else(|_| Ok(Err(Error::Task("动图预读任务未返回结果".into()))))?;
            let mut clock = self.clock.lock().map_err(|_| Error::Task("动图时钟状态不可用".into()))?;
            match next {
                Ok(Read::Frame(frame)) => {
                    next_deadline += frame.duration;
                    clock.deadline = Some(next_deadline); clock.frame = frame;
                }
                Ok(Read::End) => { clock.ended = true; clock.deadline = None; return Ok(()); }
                Ok(Read::Skipped) => {
                    let error = Error::Task("动图显示帧被意外跳过".into());
                    clock.failed = Some(error.clone()); clock.deadline = None;
                    return Err(error);
                }
                Err(error) => {
                    clock.failed = Some(error.clone()); clock.deadline = None;
                    return Err(error);
                }
            }
            // The detached producer may outlive its UI waiter. Do not start
            // more work if it has become the only animation owner.
            if Arc::strong_count(self) == 1 { return Ok(()); }
            clock.next = Some(service.prefetch_animation_frame(&self.session));
            if next_deadline > tick { return Ok(()); }
            // Intermediate catch-up frames do not reach color conversion,
            // scaling or GPU upload. The compositor is advanced one frame at
            // a time under the existing worker and pixel budgets.
        }
    }
}

// Frame iterators erase Send in image::Frames. Keep them on one lazy codec
// thread, rather than asserting unsafe Send, collecting all frames, or holding
// an idle blocking-pool thread for each image. The last live session releases
// the sender, allowing this thread and all codec allocations to exit.
pub(super) struct DecoderWorker { sender: mpsc::Sender<Command> }
impl DecoderWorker {
    fn new(task: TaskTrackerToken) -> Result<Arc<Self>> {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new().name("superpaste-animation".into()).spawn(move || {
            let _task = task;
            decoder_loop(receiver);
        })?;
        Ok(Arc::new(Self { sender }))
    }
    pub(super) fn stop(&self) {
        // A disconnected sender already denotes an exited worker. Live
        // session handles may outlive shutdown without keeping its codec alive.
        let _ = self.sender.send(Command::Stop);
    }
}
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
struct Session { worker: Arc<DecoderWorker>, id: u64, canvas: u64, dimensions: (u32, u32), encoded_bytes: u64, format: Option<ImageFormat> }
impl Drop for Session { fn drop(&mut self) { let _ = self.worker.sender.send(Command::Close(self.id)); } }
enum Command {
    Open(Open),
    Next { id: u64, output: Reservation, work: Reservation, permit: WorkerPermit, emit: bool, finish: bool, reply: oneshot::Sender<Result<Read>> },
    Close(u64),
    Stop,
}
struct Open {
    id: u64, encoded: Arc<EncodedImage>, limits: ImageLimits,
    checkpoint: Option<AnimationCheckpoint>,
    resident: Reservation, decoder_work: Reservation, output: Reservation, frame_work: Reservation,
    pixel_budget: ByteBudget, permit: WorkerPermit, reply: oneshot::Sender<Result<Opened>>,
}
struct Opened { frame: Option<RawFrame>, orientation: Orientation, color: Option<Arc<color::Input>>, single_frame: bool }
enum Read { Frame(RawFrame), Skipped, End }
struct Decoder {
    frames: Option<Frames>, encoded: Arc<EncodedImage>, limits: image::Limits,
    plays_left: Option<u32>, next_index: u32, _resident: Reservation, _work: Reservation, pixel_budget: ByteBudget,
}
fn frame_work_bytes(format: Option<ImageFormat>, canvas: u64) -> u64 {
    // Native APNG's expanded subframe is at most one canvas, and is dropped
    // before its display snapshot. WebP additionally needs YUV/RGB workspace.
    canvas * if format == Some(ImageFormat::WebP) { 4 } else { 1 }
}
fn decoder_loop(receiver: mpsc::Receiver<Command>) {
    let mut sessions: HashMap<u64, Decoder> = HashMap::new();
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Open(open) => {
                let Open { id, encoded, limits, checkpoint, resident, decoder_work, output, frame_work, pixel_budget, permit, reply } = open;
                let _permit = permit; let _frame_work = frame_work;
                if reply.is_closed() { continue; }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let (width, height) = encoded.dimensions.ok_or_else(|| Error::Image("无法读取动图尺寸".into()))?;
                    let mut codec_limits = image::Limits::default();
                    codec_limits.max_image_width = Some(width); codec_limits.max_image_height = Some(height);
                    codec_limits.max_alloc = Some(resident.bytes() + decoder_work.bytes() + output.bytes() + _frame_work.bytes());
                    limits.dimensions(width, height)?;
                    let factory = frames(&encoded, codec_limits.clone(), decoder_work.bytes(), &pixel_budget, true)?;
                    let mut decoder = Decoder { frames: Some(factory.frames), plays_left: checkpoint.map_or(factory.plays, |checkpoint| checkpoint.plays_left), next_index: 0,
                        encoded, limits: codec_limits, _resident: resident, _work: decoder_work, pixel_budget };
                    let replay = checkpoint.is_some_and(|checkpoint| checkpoint.frame_index > 0);
                    let frame = match read_frame(&mut decoder, output, !replay)? {
                        Read::Frame(frame) => Some(frame), Read::Skipped => None,
                        Read::End => return Err(Error::Image("动图没有可显示的帧".into())),
                    };
                    if !factory.single_frame && (replay || !checkpoint.is_some_and(|checkpoint| checkpoint.ended)) { sessions.insert(id, decoder); }
                    Ok(Opened { frame, orientation: factory.orientation, color: factory.color, single_frame: factory.single_frame })
                })).map_err(|_| Error::Task("动图解码任务意外终止".into())).and_then(|result| result);
                drop(_frame_work); drop(_permit);
                if reply.send(result).is_err() { sessions.remove(&id); }
            }
            Command::Next { id, output, work, permit, emit, finish, reply } => {
                let _permit = permit; let _work = work;
                // An abandoned prefetch has no clock left to consume this
                // decoder position. Drop its session before spending CPU;
                // a frame already decoding still runs to completion.
                if reply.is_closed() { sessions.remove(&id); continue; }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match sessions.get_mut(&id) {
                    Some(decoder) => read_frame(decoder, output, emit),
                    None => Err(Error::Task("动图会话已释放".into())),
                })).map_err(|_| Error::Task("动图解码任务意外终止".into())).and_then(|result| result);
                if finish || result.as_ref().is_err() || matches!(&result, Ok(Read::End)) { sessions.remove(&id); }
                drop(_work); drop(_permit);
                let _ = reply.send(result);
            }
            Command::Close(id) => { sessions.remove(&id); }
            Command::Stop => break,
        }
    }
}
enum Frames { Image(image::Frames<'static>), Png(Box<apng::Decoder>) }
impl Frames {
    fn next(&mut self, emit: bool, encoded: &EncodedImage) -> Result<Option<Sample>> {
        match self {
            Self::Png(decoder) => decoder.next(emit),
            Self::Image(frames) => {
                let Some(frame) = frames.next() else { return Ok(None); };
                let frame = frame?;
                validate_frame(&frame, encoded)?;
                let duration = Duration::from(frame.delay());
                Ok(Some(Sample { duration, pixels: emit.then(|| FrameData::Eight(frame.into_buffer().into_raw())) }))
            }
        }
    }
}
struct Factory { frames: Frames, orientation: Orientation, color: Option<Arc<color::Input>>, plays: Option<u32>, single_frame: bool }
fn frames(encoded: &Arc<EncodedImage>, limits: image::Limits, work_limit: u64, pixel_budget: &ByteBudget, read_metadata: bool) -> Result<Factory> {
    let info = encoded.animation.get().ok_or_else(|| Error::Task("缺少动图头部信息".into()))?.clone()?;
    let AnimationInfo::Animated { single_frame, gif_plays } = info else { return Err(Error::Task("图片不是动画来源".into())); };
    let reader = Cursor::new(EncodedBytes(encoded.clone()));
    macro_rules! metadata {
        ($decoder:ident) => {{
            if $decoder.dimensions() != encoded.dimensions.ok_or_else(|| Error::Image("无法读取动图尺寸".into()))? {
                return Err(Error::Image("动图尺寸校验失败".into()));
            }
            $decoder.set_limits(limits)?;
            if read_metadata { ($decoder.orientation()?, color::Input::read(&mut $decoder, encoded, pixel_budget)??) }
            else { (Orientation::NoTransforms, None) }
        }};
    }
    match encoded.format {
        Some(ImageFormat::Gif) => {
            let mut decoder = image::codecs::gif::GifDecoder::new(reader)?;
            let (orientation, color) = metadata!(decoder);
            // GIF's NETSCAPE field counts repetitions after the first play;
            // an absent extension means one play, not an infinite loop.
            Ok(Factory { frames: Frames::Image(decoder.into_frames()), orientation, color, plays: gif_plays, single_frame })
        }
        Some(ImageFormat::Png) => {
            let decoder = apng::Decoder::new(encoded, work_limit, pixel_budget)?;
            let (orientation, color) = if read_metadata {
                (decoder.orientation(), color::Input::read_png(decoder.info(), encoded, pixel_budget)??)
            } else {
                (Orientation::NoTransforms, None)
            };
            let plays = decoder.plays();
            Ok(Factory { frames: Frames::Png(Box::new(decoder)), orientation, color, plays, single_frame })
        }
        Some(ImageFormat::WebP) => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(reader)?;
            let (orientation, color) = metadata!(decoder);
            let plays = total_plays(decoder.loop_count());
            Ok(Factory { frames: Frames::Image(decoder.into_frames()), orientation, color, plays, single_frame })
        }
        _ => Err(Error::Image("不支持的动图格式".into())),
    }
}
fn total_plays(count: image::metadata::LoopCount) -> Option<u32> {
    match count { image::metadata::LoopCount::Infinite => None, image::metadata::LoopCount::Finite(count) => Some(count.get()) }
}
fn read_frame(decoder: &mut Decoder, output: Reservation, emit: bool) -> Result<Read> {
    let frame = match decoder.frames.as_mut().ok_or_else(|| Error::Task("动图解码器未初始化".into()))?.next(emit, &decoder.encoded)? {
        Some(frame) => frame,
        None => {
            if decoder.plays_left == Some(1) { return Ok(Read::End); }
            if let Some(left) = decoder.plays_left.as_mut() { *left -= 1; }
            // Release the old compositor before constructing the next play;
            // evaluating a replacement first would briefly double its memory.
            decoder.frames = None;
            decoder.frames = Some(frames(&decoder.encoded, decoder.limits.clone(), decoder._work.bytes(), &decoder.pixel_budget, false)?.frames);
            decoder.next_index = 0;
            decoder.frames.as_mut().ok_or_else(|| Error::Task("动图解码器未初始化".into()))?.next(emit, &decoder.encoded)?
                .ok_or_else(|| Error::Image("动图没有可显示的帧".into()))?
        }
    };
    let (width, height) = decoder.encoded.dimensions.ok_or_else(|| Error::Image("无法读取动图尺寸".into()))?;
    let index = decoder.next_index;
    decoder.next_index = index.checked_add(1).ok_or_else(|| Error::ResourceLimit("动图帧索引已耗尽".into()))?;
    if !emit { return Ok(Read::Skipped); }
    // Match the old host: WebKit uses <11 ms, Chromium uses <=10 ms.
    // The distinction matters for fractional APNG delays, not just zero frames.
    // https://chromium.googlesource.com/chromium/src/+/HEAD/third_party/blink/renderer/platform/graphics/deferred_image_decoder.cc
    // https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/image-decoders/ScalableImageDecoder.cpp
    let duration = frame.duration;
    let short = if cfg!(any(target_os = "macos", target_os = "linux")) {
        duration < Duration::from_millis(11)
    } else { duration <= Duration::from_millis(10) };
    let duration = if short { Duration::from_millis(100) } else { duration };
    let pixels = match frame.pixels.ok_or_else(|| Error::Task("动图未生成显示像素".into()))? {
        FrameData::Eight(data) => FramePixels::Eight(Arc::new(make_pixels(width, height, width as usize * 4, PixelLayout::Rgba8, AlphaMode::Straight, data, output))),
        FrameData::Sixteen(data) => {
            let image = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(width, height, data)
                .ok_or_else(|| Error::Image("APNG 高精度像素长度无效".into()))?;
            let image = image::DynamicImage::ImageRgba16(image);
            let mut hash = Sha256::new();
            hash.update(b"super-paste-rs-animation-rgba16-v1\0");
            hash.update(width.to_be_bytes()); hash.update(height.to_be_bytes()); hash.update(image.as_bytes());
            let id = Digest(hash.finalize().into());
            let decoded = Arc::new(DecodedImage { width, height, pixels: DecodedPixels::Native { image, reservation: output },
                orientation: Ok(Orientation::NoTransforms), color: Ok(None) });
            FramePixels::Native { decoded, id }
        }
    };
    Ok(Read::Frame(RawFrame { pixels, duration, index, plays_left: decoder.plays_left }))
}
fn validate_frame(frame: &image::Frame, encoded: &EncodedImage) -> Result<()> {
    if Some(frame.buffer().dimensions()) != encoded.dimensions || frame.left() != 0 || frame.top() != 0 {
        return Err(Error::Image("动图合成帧尺寸无效".into()));
    }
    Ok(())
}
