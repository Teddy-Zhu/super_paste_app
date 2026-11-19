use std::sync::Arc;
use async_trait::async_trait;
use clipboard_core::{ClipboardImage, ImageRepresentation, WatchHandle};
use super_paste_core::{Error, Result, clipboard::{ClipboardBackend, ClipboardService, ClipboardSnapshot}, events::{AppEvent, EventBus}, images::{AlphaMode, EncodedImage, ImageLimits, ImageService, ImageSource, PixelBuffer, PixelLayout, PixelSpec, ResizeFilter}};
use super_paste_format::{SourceApp, TextContent};
use tokio::{runtime::Handle, sync::{Mutex, mpsc}, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub type SourceReader = Arc<dyn Fn(bool) -> Result<Option<Arc<SourceApp>>> + Send + Sync>;

pub struct NativeClipboard { runtime: Handle, images: Arc<ImageService>, reads: Arc<Mutex<()>> }
impl NativeClipboard {
    pub fn new(runtime: Handle, images: Arc<ImageService>) -> Arc<Self> { Arc::new(Self { runtime, images, reads: Arc::new(Mutex::new(())) }) }
    pub fn prefers_encoded_images(&self) -> bool { clipboard_core::write_image_representation() == ImageRepresentation::Encoded }

    pub async fn check_available(&self) -> Result<()> {
        self.runtime.spawn_blocking(|| clipboard_core::check_available().map_err(platform_error)).await?
    }

    pub async fn start_watch(self: &Arc<Self>, service: Arc<ClipboardService>, source: SourceReader, events: EventBus,
        initialization: clipboard_core::ReadCancellation,
    ) -> Result<ClipboardWatch> {
        let (sender, mut receiver) = mpsc::channel::<Result<Capture>>(2);
        let port = self.clone();
        let cancel_on_drop = CancelNativeRead(Some(initialization.clone()));
        let control = initialization.clone();
        let watch = self.runtime.spawn_blocking(move || {
            clipboard_core::watch_with_cancel(move |result| {
                let result = result.map_err(platform_error).and_then(|raw| {
                    let timestamp = super_paste_core::now_ms()?;
                    let source = source(raw.image.is_some())?;
                    // Watch callbacks run on the backend thread. Admit buffers before queuing them.
                    let snapshot = port.runtime.block_on(port.adapt_read(prepare_read(raw), &CancellationToken::new()))?;
                    Ok(Capture { snapshot, source, timestamp })
                });
                // Backpressure preserves accepted copy order instead of dropping arbitrary notifications.
                let _ = sender.blocking_send(result);
            }, control).map_err(platform_error)
        }).await?;
        let watch = match watch {
            Err(_) if initialization.is_cancelled() => return Err(Error::Cancelled),
            result => result?,
        };
        if initialization.is_cancelled() {
            // Release bounded callback backpressure before joining a watcher
            // that completed initialization concurrently with cancellation.
            drop(receiver);
            let (completed, thread) = watch.stop_tracked();
            finish_native_watch(&self.runtime, completed, thread).await?;
            return Err(Error::Cancelled);
        }
        cancel_on_drop.disarm();
        let shutdown = CancellationToken::new();
        service.set_native_listening(watch.is_running());
        let owner = service.clone();
        let worker_shutdown = shutdown.clone();
        let worker = self.runtime.spawn(async move {
            let _listening = Listening(service.clone());
            let mut stopping = false;
            let work_cancel = CancellationToken::new();
            loop {
                let next = tokio::select! {
                    _ = worker_shutdown.cancelled(), if !stopping => {
                        stopping = true;
                        receiver.close();
                        continue;
                    }
                    next = receiver.recv() => next,
                };
                let Some(capture) = next else { break };
                let result = match capture {
                    Ok(capture) => service.capture(capture.snapshot, capture.source, capture.timestamp, &work_cancel).await.map(|_| ()),
                    Err(error) => Err(error),
                };
                if let Err(error) = result { events.publish(AppEvent::ClipboardFailed(error)); }
            }
        });
        Ok(ClipboardWatch { runtime: self.runtime.clone(), watch: Some(watch), shutdown, worker: Some(worker), service: owner })
    }

    async fn adapt_read(&self, (content, image): (TextContent, Option<ClipboardImage>), cancel: &CancellationToken) -> Result<ClipboardSnapshot> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let image = match image {
            Some(ClipboardImage::Encoded(bytes)) => self.images.import_encoded(bytes, cancel).await.map(|image| Some(ImageSource::Encoded(image))),
            Some(ClipboardImage::Rgba8 { width, height, pixels }) => self.images.import_pixels(width, height, width as usize * 4, PixelLayout::Rgba8, AlphaMode::Straight, pixels, cancel).await.map(|image| Some(ImageSource::Pixels(image))),
            None => Ok(None),
        };
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let image = match image {
            // Image admission is still a representation read. Preserve other
            // successful formats just as the native FormatReader does; global
            // cancellation and infrastructure failures remain whole-read errors.
            Err(error @ (Error::Invalid(_) | Error::Image(_) | Error::ResourceLimit(_)))
                if !content.formats(false).is_empty() => {
                tracing::warn!(%error, "剪贴板图片接入失败，已保留其他可用格式");
                None
            },
            result => result?,
        };
        Ok(ClipboardSnapshot { content, image })
    }
}

#[async_trait]
impl ClipboardBackend for NativeClipboard {
    async fn read(&self, cancel: &CancellationToken) -> Result<ClipboardSnapshot> {
        let control = clipboard_core::ReadCancellation::new().map_err(platform_error)?;
        let snapshot = cancellable_native_read(&self.runtime, self.reads.clone(), cancel, control,
            |control| clipboard_core::read_with_cancel(&control).map_err(platform_error)).await?;
        self.adapt_read(snapshot, cancel).await
    }
    async fn write(&self, snapshot: ClipboardSnapshot, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let image = match snapshot.image {
            Some(source) if self.prefers_encoded_images() => {
                let encoded = match source {
                    ImageSource::Encoded(encoded) => encoded,
                    ImageSource::Stored(metadata) => self.images.load_original(metadata, cancel).await?,
                    ImageSource::Pixels(pixels) => self.images.encode_png(pixels, cancel).await?,
                };
                Some(PreparedImage::Encoded(encoded))
            }
            Some(source) => {
                let pixels = self.images.acquire_pixels(source, ImageLimits::default(), cancel).await?;
                let spec = PixelSpec { max_width: pixels.width(), max_height: pixels.height(), layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
                Some(PreparedImage::Rgba(self.images.derive_pixels(pixels, spec, cancel).await?))
            }
            None => None,
        };
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let content = snapshot.content; let cancel = cancel.clone();
        self.runtime.spawn_blocking(move || native_write(content, image, &cancel,
            |raw| clipboard_core::write_owned(raw).map_err(platform_error))).await?
    }
}

enum PreparedImage { Encoded(Arc<EncodedImage>), Rgba(Arc<PixelBuffer>) }

struct CancelNativeRead(Option<clipboard_core::ReadCancellation>);
impl CancelNativeRead { fn disarm(mut self) { self.0 = None; } }
impl Drop for CancelNativeRead { fn drop(&mut self) { if let Some(control) = &self.0 { control.cancel(); } } }

async fn cancellable_native_read(runtime: &Handle, reads: Arc<Mutex<()>>, cancel: &CancellationToken,
    control: clipboard_core::ReadCancellation,
    read: impl FnOnce(clipboard_core::ReadCancellation) -> Result<clipboard_core::ClipboardSnapshot> + Send + 'static,
) -> Result<(TextContent, Option<ClipboardImage>)> {
    // Dropping a waiter also wakes a Wayland pipe already owned by the blocking
    // worker. Cancellation still waits for physical completion when awaited.
    let _cancel_on_drop = CancelNativeRead(Some(control.clone()));
    let native_control = control.clone();
    let pending = native_read(runtime, reads, cancel, move || read(native_control));
    tokio::pin!(pending);
    tokio::select! { biased;
        _ = cancel.cancelled() => {
            control.cancel();
            pending.await
        },
        result = &mut pending => result,
    }
}

async fn native_read(runtime: &Handle, reads: Arc<Mutex<()>>, cancel: &CancellationToken,
    read: impl FnOnce() -> Result<clipboard_core::ClipboardSnapshot> + Send + 'static,
) -> Result<(TextContent, Option<ClipboardImage>)> {
    let token = cancel.child_token();
    let _cancel_on_drop = token.clone().drop_guard();
    let permit = tokio::select! { biased;
        _ = token.cancelled() => return Err(Error::Cancelled),
        permit = reads.lock_owned() => permit,
    };
    if token.is_cancelled() { return Err(Error::Cancelled); }
    runtime.spawn_blocking(move || {
        // Do not occupy blocking threads while waiting for another direct
        // read. The physical operation owns admission even if its waiter dies.
        let _permit = permit;
        if token.is_cancelled() { return Err(Error::Cancelled); }
        let snapshot = read();
        if token.is_cancelled() { return Err(Error::Cancelled); }
        Ok(prepare_read(snapshot?))
    }).await?
}

// String -> Arc<str> also allocates: prepare the shared text on the native read
// worker (or watch callback), not on the asynchronous service executor.
fn prepare_read(raw: clipboard_core::ClipboardSnapshot) -> (TextContent, Option<ClipboardImage>) {
    (TextContent { text: raw.text.map(Arc::from), html: raw.html.map(Arc::from), rtf: raw.rtf.map(Arc::from), files: raw.files.into() }, raw.image)
}

fn native_write(content: TextContent, image: Option<PreparedImage>, cancel: &CancellationToken,
    write: impl FnOnce(clipboard_core::ClipboardSnapshot) -> Result<()>) -> Result<()>
{
    // Queueing and native buffer copies are still cancellable. Once the native
    // call starts, return its actual result so the service can commit metadata.
    if cancel.is_cancelled() { return Err(Error::Cancelled); }
    let image = image.map(|image| match image {
        PreparedImage::Encoded(image) => ClipboardImage::Encoded(image.bytes().to_vec()),
        PreparedImage::Rgba(pixels) => ClipboardImage::Rgba8 { width: pixels.width(), height: pixels.height(), pixels: pixels.bytes().to_vec() },
    });
    // Native APIs require owned buffers. Materialize them only on this worker,
    // retaining shared source buffers while waiting without another payload copy.
    let raw = clipboard_core::ClipboardSnapshot { text: content.text.as_deref().map(str::to_owned), html: content.html.as_deref().map(str::to_owned), rtf: content.rtf.as_deref().map(str::to_owned), files: content.files.to_vec(), image };
    drop(content);
    if cancel.is_cancelled() { return Err(Error::Cancelled); }
    write(raw)
}

struct Capture { snapshot: ClipboardSnapshot, source: Option<Arc<SourceApp>>, timestamp: i64 }
struct Listening(Arc<ClipboardService>);
impl Drop for Listening { fn drop(&mut self) { self.0.set_native_listening(false); } }
pub struct ClipboardWatch { runtime: Handle, watch: Option<WatchHandle>, shutdown: CancellationToken, worker: Option<JoinHandle<()>>, service: Arc<ClipboardService> }
impl ClipboardWatch {
    pub fn is_running(&self) -> bool { self.watch.as_ref().is_some_and(WatchHandle::is_running) }
    pub async fn stop(mut self) -> Result<()> {
        self.service.set_native_listening(false);
        let stopping = self.watch.take().map(WatchHandle::stop_tracked);
        self.shutdown.cancel();
        let worker = self.worker.take();
        let native = async { match stopping {
            Some((completed, thread)) => finish_native_watch(&self.runtime, completed, thread).await,
            None => Ok(()),
        } };
        let consumer = async { if let Some(worker) = worker { worker.await?; } Ok(()) };
        let (native, consumer): (Result<()>, Result<()>) = tokio::join!(native, consumer);
        let errors = [native, consumer].into_iter().filter_map(Result::err).map(|error| error.to_string()).collect::<Vec<_>>();
        if errors.is_empty() { Ok(()) } else { Err(Error::Platform(errors.join("；").into())) }
    }
}
impl Drop for ClipboardWatch {
    fn drop(&mut self) {
        self.service.set_native_listening(false);
        self.watch.take();
        self.shutdown.cancel();
    }
}
async fn finish_native_watch(runtime: &Handle,
    completed: impl std::future::Future<Output = clipboard_core::ClipboardResult<()>>,
    thread: std::thread::JoinHandle<()>,
) -> Result<()> {
    let completed = completed.await.map_err(platform_error);
    // Only queue the physical join once callback resources no longer need the
    // blocking pool. This also works when that pool has only one worker.
    runtime.spawn_blocking(move || thread.join().map_err(|_| Error::Task("原生剪贴板监听线程异常退出".into()))).await??;
    completed
}
fn platform_error(error: Box<dyn std::error::Error + Send + Sync>) -> Error { Error::Platform(error.to_string().into()) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_shutdown_leaves_pool_available_until_callbacks_finish_and_joins_native_exit() {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).max_blocking_threads(1)
            .enable_all().build().expect("isolated runtime");
        let worker = runtime.handle().clone();
        runtime.block_on(async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), async move {
                let (entered, started) = tokio::sync::oneshot::channel();
                let (completed, completion) = tokio::sync::oneshot::channel();
                let (announced, ready) = tokio::sync::oneshot::channel();
                let (release_callback, callback) = std::sync::mpsc::channel();
                let (release_thread, exit) = std::sync::mpsc::channel();
                let native = std::thread::spawn({ let worker = worker.clone(); move || {
                    callback.recv().expect("release callback to use the runtime");
                    let value = worker.block_on(worker.spawn_blocking(|| 42)).expect("callback blocking work");
                    entered.send(value).expect("callback reached final phase");
                    completed.send(()).expect("announce callback completion");
                    announced.send(()).expect("completion is ready to observe");
                    exit.recv().expect("release native thread teardown");
                } });
                let mut stopping = Box::pin(finish_native_watch(&worker, async move {
                    completion.await.map_err(Into::into)
                }, native));
                assert!(futures_util::poll!(stopping.as_mut()).is_pending());
                assert_eq!(worker.spawn_blocking(|| 17).await.expect("shutdown must leave the only blocking worker available"), 17);
                release_callback.send(()).expect("callback can use the runtime before completing");
                assert_eq!(started.await.expect("callback can still use the runtime"), 42);
                ready.await.expect("native completion announced");
                assert!(futures_util::poll!(stopping.as_mut()).is_pending(), "completion must still join physical thread teardown");
                release_thread.send(()).expect("native thread can exit");
                stopping.await.expect("native watcher joined");

                let native = std::thread::spawn(|| panic!("native watcher panic for ownership regression"));
                assert!(matches!(finish_native_watch(&worker, async { Ok(()) }, native).await, Err(Error::Task(_))),
                    "native thread panic must be reported as a failed stop");
            }).await.expect("watch callback and native shutdown finish within five seconds");
        });
    }

    #[test]
    fn native_reads_cancel_admission_and_hold_gate_until_physical_return() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).max_blocking_threads(1)
            .enable_all().build().expect("isolated runtime");
        let worker = runtime.handle().clone();
        runtime.block_on(async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), async move {
                let reads = Arc::new(Mutex::new(()));
                let held = reads.clone().lock_owned().await;
                let cancel = CancellationToken::new();
                let queued = native_read(&worker, reads.clone(), &cancel, || panic!("cancelled admission reached native read"));
                tokio::pin!(queued);
                assert!(futures_util::poll!(queued.as_mut()).is_pending());
                cancel.cancel();
                assert!(matches!(queued.await, Err(Error::Cancelled)));
                drop(held);

                let (started, entered) = tokio::sync::oneshot::channel();
                let (release, waiting) = std::sync::mpsc::channel();
                let first = tokio::spawn({ let worker = worker.clone(); let reads = reads.clone(); async move {
                    native_read(&worker, reads, &CancellationToken::new(), move || {
                        started.send(()).expect("physical read entered");
                        waiting.recv().expect("release physical read");
                        Ok(clipboard_core::ClipboardSnapshot::default())
                    }).await
                } });
                entered.await.expect("physical read started");
                first.abort();
                assert!(matches!(first.await, Err(error) if error.is_cancelled()));
                assert!(reads.try_lock().is_err(), "caller destruction must not release physical admission");
                let cancel = CancellationToken::new();
                let replacement = native_read(&worker, reads.clone(), &cancel, || Ok(clipboard_core::ClipboardSnapshot {
                    text: Some("新鲜\0数据 😀".into()), ..Default::default()
                }));
                tokio::pin!(replacement);
                assert!(futures_util::poll!(replacement.as_mut()).is_pending());
                release.send(()).expect("finish abandoned physical read");
                let (content, _) = replacement.await.expect("replacement reads actual fresh data");
                assert_eq!(content.text.as_deref(), Some("新鲜\0数据 😀"));

                let (started, entered) = tokio::sync::oneshot::channel();
                let (release, waiting) = std::sync::mpsc::channel();
                let blocker = worker.spawn_blocking(move || {
                    started.send(()).expect("blocking pool occupied");
                    waiting.recv().expect("release blocking pool");
                });
                entered.await.expect("sole blocking thread occupied");
                let called = Arc::new(AtomicUsize::new(0)); let observed = called.clone();
                let cancel = CancellationToken::new();
                let mut queued = Box::pin(native_read(&worker, reads.clone(), &cancel, move || {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok(clipboard_core::ClipboardSnapshot::default())
                }));
                assert!(futures_util::poll!(queued.as_mut()).is_pending());
                assert!(reads.try_lock().is_err(), "admission transfers into the queued blocking task");
                drop(queued);
                assert!(reads.try_lock().is_err());
                release.send(()).expect("release blocking task");
                blocker.await.expect("blocking pool gate joined");
                let drained = reads.lock().await;
                assert_eq!(called.load(Ordering::SeqCst), 0, "abandoned queued work must skip the native call");
                drop(drained);

                for abandon in [false, true] {
                    let control = clipboard_core::ReadCancellation::new().expect("cancellation state");
                    let observed = control.clone();
                    let cancel = CancellationToken::new();
                    let (started, entered) = tokio::sync::oneshot::channel();
                    let (release, waiting) = std::sync::mpsc::channel();
                    let mut request = Box::pin(cancellable_native_read(&worker, reads.clone(), &cancel, control, move |control| {
                        started.send(()).expect("cancellable native read started");
                        waiting.recv().expect("release cancellable physical operation");
                        assert!(control.is_cancelled(), "waiter cancellation reaches native control");
                        Err(Error::Platform("native operation interrupted".into()))
                    }));
                    assert!(futures_util::poll!(request.as_mut()).is_pending());
                    entered.await.expect("native operation entered");
                    if abandon {
                        drop(request);
                        assert!(observed.is_cancelled());
                        assert!(reads.try_lock().is_err(), "native admission survives waiter destruction");
                        release.send(()).expect("release abandoned native read");
                    } else {
                        cancel.cancel();
                        assert!(futures_util::poll!(request.as_mut()).is_pending(), "cancellation waits for physical drain");
                        assert!(observed.is_cancelled());
                        assert!(reads.try_lock().is_err());
                        release.send(()).expect("release cancelled native read");
                        assert!(matches!(request.await, Err(Error::Cancelled)), "native interruption is reported as cancellation");
                    }
                    let _drained = reads.lock().await;
                }
            }).await.expect("native read admission and drain finish within five seconds");
        });
    }

    // Exercise the real handoff on a deliberately occupied blocking pool. The
    // writer is local: neither reads nor writes touch the system clipboard.
    #[test]
    fn queued_writes_cancel_before_handoff_and_started_writes_keep_their_result() {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).max_blocking_threads(1)
            .enable_all().build().expect("isolated runtime");
        let (ready, started) = tokio::sync::oneshot::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let blocker = runtime.spawn_blocking(move || {
            ready.send(()).expect("announce occupied worker");
            waiting.recv().expect("release occupied worker");
        });
        let worker = runtime.handle().clone();
        runtime.block_on(async move {
            started.await.expect("worker occupied");
            let content = TextContent {
                text: Some("  中文👩🏽‍💻e\u{301}\r\n\0尾部\t ".into()),
                html: Some("<p data-original=\"yes\"> &amp; &#x1f600; </p>\r\n".into()),
                rtf: Some("{\\rtf1\\ansi \\b preserved\\b0 \\u20013?}".into()),
                files: vec!["/资料/含 空格.txt".into(), "C:\\资料\\原件.png".into(), "/资料/含 空格.txt".into()].into(),
            };
            let cancel = CancellationToken::new(); let queued_cancel = cancel.clone(); let queued_content = content.clone();
            let queued = worker.spawn_blocking(move || native_write(queued_content, None, &queued_cancel,
                |_| panic!("cancelled queued write reached native handoff")));
            cancel.cancel();
            release.send(()).expect("release queue");
            blocker.await.expect("blocking gate joined");
            assert!(matches!(queued.await.expect("queued job joined"), Err(Error::Cancelled)));

            let expected = content.clone(); let failure_content = content.clone();
            let cancel = CancellationToken::new(); let inside_native = cancel.clone(); let observed_cancel = cancel.clone();
            let written = worker.spawn_blocking(move || native_write(content, None, &cancel, |raw| {
                assert_eq!(raw.text.as_deref(), expected.text.as_deref());
                assert_eq!(raw.html.as_deref(), expected.html.as_deref());
                assert_eq!(raw.rtf.as_deref(), expected.rtf.as_deref());
                assert_eq!(raw.files.as_slice(), expected.files.as_ref());
                assert!(raw.image.is_none());
                let (read, image) = prepare_read(raw);
                assert_eq!(read, expected);
                assert!(image.is_none());
                inside_native.cancel();
                Ok(())
            }));
            written.await.expect("native success joined").expect("retain completed write success");
            assert!(observed_cancel.is_cancelled());

            let cancel = CancellationToken::new(); let inside_native = cancel.clone();
            let failed = worker.spawn_blocking(move || native_write(failure_content, None, &cancel, |_| {
                inside_native.cancel();
                Err(Error::Platform("native write failed".into()))
            }));
            assert!(matches!(failed.await.expect("native failure joined"), Err(Error::Platform(error)) if error.as_ref() == "native write failed"));
        });
    }
}
