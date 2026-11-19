//! N-API-free clipboard core extracted from the project's clipboard bindings.
//!
//! Windows, macOS and X11 use `clipboard-rs`; Wayland uses the upstream
//! `wl-clipboard-rs` data-control path so a changed selection is read from one
//! offer and all representations can be restored together.

use clipboard_rs::common::RustImage;
use clipboard_rs::{
    Clipboard, ClipboardContent, ClipboardContext, ClipboardHandler, ClipboardWatcher,
    ClipboardWatcherContext, ContentFormat, RustImageData,
};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use url::Url;

#[cfg(target_os = "linux")]
mod wayland;

pub type ClipboardResult<T> = Result<T, Box<dyn Error + Send + Sync + 'static>>;

/// One operation's cancellation state. Wayland additionally interrupts native
/// initialization, the event wait and offer data pipes; other native APIs retain
/// their drain policy.
#[derive(Clone)]
pub struct ReadCancellation(std::sync::Arc<ReadCancellationState>);

struct ReadCancellationState {
    cancelled: AtomicBool,
    #[cfg(target_os = "linux")]
    wayland: Option<wayland::ReadControl>,
}

impl ReadCancellation {
    pub fn new() -> ClipboardResult<Self> {
        #[cfg(target_os = "linux")]
        if is_wayland_session() { return Self::for_wayland(); }
        Ok(Self(std::sync::Arc::new(ReadCancellationState {
            cancelled: AtomicBool::new(false),
            #[cfg(target_os = "linux")]
            wayland: None,
        })))
    }

    #[cfg(target_os = "linux")]
    fn for_wayland() -> ClipboardResult<Self> {
        Ok(Self(std::sync::Arc::new(ReadCancellationState {
            cancelled: AtomicBool::new(false), wayland: Some(wayland::ReadControl::new()?),
        })))
    }

    pub fn is_cancelled(&self) -> bool { self.0.cancelled.load(Ordering::Acquire) }

    pub fn cancel(&self) {
        if self.0.cancelled.swap(true, Ordering::AcqRel) { return; }
        #[cfg(target_os = "linux")]
        if let Some(control) = &self.0.wayland { control.cancel(); }
    }

    fn check(&self) -> ClipboardResult<()> {
        if self.is_cancelled() {
            Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "clipboard read cancelled").into())
        } else { Ok(()) }
    }
}

// clipboard-rs keeps its X11 owner thread alive after a context is dropped.
// Reuse one process-wide context and serialize requests over its read connection.
#[cfg(target_os = "linux")]
static X11_CONTEXT: std::sync::Mutex<Option<ClipboardContext>> = std::sync::Mutex::new(None);

fn with_native_context<T>(
    operation: impl FnOnce(&ClipboardContext) -> ClipboardResult<T>,
) -> ClipboardResult<T> {
    #[cfg(target_os = "linux")]
    {
        let mut context = X11_CONTEXT
            .lock()
            .map_err(|_| "X11 clipboard context lock poisoned")?;
        if let Some(clipboard) = context.as_ref() {
            return operation(clipboard);
        }
        // Only cache successful initialization so unavailable displays can be retried.
        operation(context.insert(ClipboardContext::new()?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        operation(&ClipboardContext::new()?)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClipboardSnapshot {
    pub text: Option<String>,
    pub html: Option<String>,
    pub rtf: Option<String>,
    /// Preserve the backend representation without a PNG encode/decode round trip.
    pub image: Option<ClipboardImage>,
    pub files: Vec<String>,
}

impl ClipboardSnapshot {
    pub fn is_empty(&self) -> bool {
        self.text.as_ref().is_none_or(String::is_empty)
            && self.html.as_ref().is_none_or(String::is_empty)
            && self.rtf.as_ref().is_none_or(String::is_empty)
            && self.image.as_ref().is_none_or(ClipboardImage::is_empty)
            && self.files.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardImage {
    Encoded(Vec<u8>),
    Rgba8 { width: u32, height: u32, pixels: Vec<u8> },
}
impl ClipboardImage {
    pub fn is_empty(&self) -> bool {
        match self { Self::Encoded(bytes) => bytes.is_empty(), Self::Rgba8 { pixels, .. } => pixels.is_empty() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRepresentation { Encoded, Rgba8 }

pub fn write_image_representation() -> ImageRepresentation {
    if is_wayland_session() { ImageRepresentation::Encoded } else { ImageRepresentation::Rgba8 }
}

pub fn is_wayland_session() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland")
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Reports whether the selected backend can be opened. This checks the actual
/// Wayland data-control protocol instead of treating environment variables as
/// proof that clipboard access is available.
pub fn check_available() -> ClipboardResult<()> {
    if is_wayland_session() {
        #[cfg(target_os = "linux")]
        return wayland::check_available();
    }
    with_native_context(|_| Ok(()))
}

pub fn read() -> ClipboardResult<ClipboardSnapshot> {
    read_with_cancel(&ReadCancellation::new()?)
}

pub fn read_with_cancel(cancel: &ReadCancellation) -> ClipboardResult<ClipboardSnapshot> {
    cancel.check()?;
    #[cfg(target_os = "linux")]
    if cancel.0.wayland.is_some() { return wayland::read(cancel); }
    let result = read_native();
    cancel.check()?;
    result
}

// Applications sometimes advertise a representation they cannot provide. Keep
// the readable formats, but report a failure when no usable representation remains.
pub(crate) struct FormatReader {
    first_error: Option<String>,
}

impl FormatReader {
    pub(crate) fn new() -> Self {
        Self { first_error: None }
    }

    pub(crate) fn read<T>(&mut self, format: &str, result: ClipboardResult<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(format, "clipboard representation could not be read");
                self.first_error
                    .get_or_insert_with(|| format!("{format}: {error}"));
                None
            }
        }
    }

    pub(crate) fn finish(self, snapshot: ClipboardSnapshot) -> ClipboardResult<ClipboardSnapshot> {
        if snapshot.is_empty()
            && let Some(error) = self.first_error
        {
            return Err(error.into());
        }
        Ok(snapshot)
    }
}

fn read_native() -> ClipboardResult<ClipboardSnapshot> {
    with_native_context(|clipboard| {
        let mut snapshot = ClipboardSnapshot::default();
        let mut formats = FormatReader::new();
        if clipboard.has(ContentFormat::Text) {
            snapshot.text = formats.read("text", clipboard.get_text());
        }
        if clipboard.has(ContentFormat::Html) {
            snapshot.html = formats.read("html", clipboard.get_html());
        }
        if clipboard.has(ContentFormat::Rtf) {
            snapshot.rtf = formats.read("rtf", clipboard.get_rich_text());
        }
        if clipboard.has(ContentFormat::Image) {
            snapshot.image = formats.read(
                "image",
                clipboard
                    .get_image()
                    .and_then(|image| image.to_rgba8().map(|rgba| ClipboardImage::Rgba8 {
                        width: rgba.width(), height: rgba.height(), pixels: rgba.into_raw(),
                    })),
            );
        }
        if clipboard.has(ContentFormat::Files) {
            snapshot.files = formats
                .read("files", clipboard.get_files())
                .unwrap_or_default();
        }
        formats.finish(snapshot)
    })
}

pub fn write(snapshot: &ClipboardSnapshot) -> ClipboardResult<()> {
    write_owned(snapshot.clone())
}

pub fn write_owned(snapshot: ClipboardSnapshot) -> ClipboardResult<()> {
    if snapshot.is_empty() {
        return Err("clipboard snapshot contains no restorable formats".into());
    }
    if is_wayland_session() {
        #[cfg(target_os = "linux")]
        return wayland::write(snapshot);
    }
    write_native(snapshot)
}

fn write_native(snapshot: ClipboardSnapshot) -> ClipboardResult<()> {
    let mut contents = Vec::new();

    // Windows' clipboard-rs backend clears the clipboard while installing an
    // image. Install it before the other representations so they remain
    // available as part of the same clipboard operation.
    if let Some(image) = snapshot.image {
        let image = match image {
            ClipboardImage::Encoded(bytes) => RustImageData::from_bytes(&bytes)?,
            ClipboardImage::Rgba8 { width, height, pixels } => {
                let rgba = image::RgbaImage::from_raw(width, height, pixels).ok_or("invalid RGBA clipboard image dimensions")?;
                RustImageData::from_dynamic_image(image::DynamicImage::ImageRgba8(rgba))
            }
        };
        contents.push(ClipboardContent::Image(image));
    }
    if let Some(text) = snapshot.text {
        contents.push(ClipboardContent::Text(text));
    }
    if let Some(html) = snapshot.html {
        contents.push(ClipboardContent::Html(html));
    }
    if let Some(rtf) = snapshot.rtf {
        contents.push(ClipboardContent::Rtf(rtf));
    }
    if !snapshot.files.is_empty() {
        contents.push(ClipboardContent::Files(native_file_paths(&snapshot.files)?));
    }

    with_native_context(|clipboard| clipboard.set(contents))
}

fn native_file_paths(files: &[String]) -> ClipboardResult<Vec<String>> {
    files.iter().map(|file| local_path(file)).collect()
}

fn local_path(file: &str) -> ClipboardResult<String> {
    let path = if Path::new(file).is_absolute() {
        PathBuf::from(file)
    } else {
        if file.chars().any(char::is_control) {
            return Err("control characters in file URIs must be percent-encoded".into());
        }
        let uri = Url::parse(file)?;
        if uri.scheme() != "file" || uri.query().is_some() || uri.fragment().is_some() {
            return Err(
                "files must be absolute paths or file:// URIs without queries or fragments".into(),
            );
        }
        uri.to_file_path()
            .map_err(|_| "file URI cannot be represented as a local path on this platform")?
    };

    let path = path
        .into_os_string()
        .into_string()
        .map_err(|_| "file paths must be valid UTF-8")?;
    if path.contains('\0') {
        return Err("file paths cannot contain NUL bytes".into());
    }
    Ok(path)
}

type ChangeCallback = Box<dyn FnMut(ClipboardResult<ClipboardSnapshot>) + Send + 'static>;

pub struct WatchHandle {
    cancel: Option<Box<dyn FnOnce() + Send>>,
    running: std::sync::Arc<AtomicBool>,
    completed: Option<futures_channel::oneshot::Receiver<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl WatchHandle {
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn stop(mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }

    /// Signal stop and retain ownership of the native thread. Await completion
    /// before joining off the async executor: callbacks can themselves need
    /// the runtime's blocking pool while their captured resources are live.
    pub fn stop_tracked(mut self) -> (
        impl std::future::Future<Output = ClipboardResult<()>> + Send + 'static,
        thread::JoinHandle<()>,
    ) {
        if let Some(cancel) = self.cancel.take() { cancel(); }
        let completed = self.completed.take().expect("watch completion belongs to this handle");
        let thread = self.thread.take().expect("watch thread belongs to this handle");
        (async move { completed.await.map_err(Into::into) }, thread)
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

struct WatchThreadCompletion(Option<futures_channel::oneshot::Sender<()>>);
impl Drop for WatchThreadCompletion {
    fn drop(&mut self) {
        if let Some(completed) = self.0.take() { let _ = completed.send(()); }
    }
}

struct WatchThreadState(std::sync::Arc<AtomicBool>);

impl WatchThreadState {
    fn start(running: std::sync::Arc<AtomicBool>) -> Self {
        running.store(true, Ordering::Release);
        Self(running)
    }
}

impl Drop for WatchThreadState {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn start_watch_thread(name: &str, running: std::sync::Arc<AtomicBool>,
    cancel: impl FnOnce() + Send + 'static, run: impl FnOnce() + Send + 'static,
) -> ClipboardResult<WatchHandle> {
    let thread_state = running.clone();
    let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(0);
    let (completed, completion) = futures_channel::oneshot::channel();
    let thread = thread::Builder::new().name(name.into()).spawn(move || {
        // The run closure owns watcher/callback resources. It is consumed and
        // released before completion, on both normal return and unwinding.
        let _completed = WatchThreadCompletion(Some(completed));
        let run = run;
        let _thread_state = WatchThreadState::start(thread_state);
        let _ = ready_sender.send(());
        run();
    })?;
    ready_receiver.recv()?;
    Ok(WatchHandle { cancel: Some(Box::new(cancel)), running,
        completed: Some(completion), thread: Some(thread) })
}

pub fn watch(
    callback: impl FnMut(ClipboardResult<ClipboardSnapshot>) + Send + 'static,
) -> ClipboardResult<WatchHandle> {
    watch_with_cancel(callback, ReadCancellation::new()?)
}

/// Cancellation is owned before initialization starts. On Wayland it also
/// interrupts protocol roundtrips, before an event-loop handle exists.
pub fn watch_with_cancel(
    callback: impl FnMut(ClipboardResult<ClipboardSnapshot>) + Send + 'static,
    cancel: ReadCancellation,
) -> ClipboardResult<WatchHandle> {
    cancel.check()?;
    let callback: ChangeCallback = Box::new(callback);
    #[cfg(target_os = "linux")]
    if cancel.0.wayland.is_some() { return wayland::watch(callback, cancel); }
    watch_native(callback)
}

struct NativeChangeHandler {
    callback: ChangeCallback,
}

impl ClipboardHandler for NativeChangeHandler {
    fn on_clipboard_change(&mut self) {
        (self.callback)(read_native());
    }
}

fn watch_native(callback: ChangeCallback) -> ClipboardResult<WatchHandle> {
    let mut watcher = ClipboardWatcherContext::new_with_interval(Duration::from_millis(100))?;
    watcher.add_handler(NativeChangeHandler { callback });
    let shutdown = watcher.get_shutdown_channel();
    let running = std::sync::Arc::new(AtomicBool::new(false));
    start_watch_thread("clipboard-watch-native", running, move || drop(shutdown), move || watcher.start_watch())
}

#[cfg(target_os = "linux")]
pub(crate) fn start_wayland_watch(
    watcher: wl_clipboard_rs::watch::Watcher,
    callback: ChangeCallback,
    cancel: ReadCancellation,
) -> ClipboardResult<WatchHandle> {
    let cancel_read = cancel.clone();
    let thread_running = std::sync::Arc::new(AtomicBool::new(false));
    start_watch_thread("clipboard-watch-wayland", thread_running, move || {
        cancel_read.cancel();
    }, move || {
        let mut watcher = watcher;
        let mut callback = callback;
        while !cancel.is_cancelled() {
            let event = match watcher.next_event() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => {
                    if !cancel.is_cancelled() { callback(Err(Box::new(error))); }
                    break;
                }
            };
            let result = wayland::read_event(event, &cancel);
            if cancel.is_cancelled() { break; }
            callback(result);
        }
    })
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use std::{future::Future, sync::{Arc, mpsc}, task::{Context, Poll, Waker}};

    struct CallbackResource(Arc<AtomicBool>);
    impl Drop for CallbackResource { fn drop(&mut self) { self.0.store(true, Ordering::Release); } }

    #[test]
    fn tracked_watch_stop_keeps_callback_resources_and_preserves_panics() {
        let running = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let resource = CallbackResource(dropped.clone());
        let cancelled = Arc::new(AtomicBool::new(false)); let stop = cancelled.clone();
        let (started, entered) = mpsc::sync_channel(0);
        let (release, blocked) = mpsc::channel();
        let watch = start_watch_thread("clipboard-watch-lifecycle", running.clone(), move || {
            assert!(!stop.swap(true, Ordering::AcqRel), "stop signal is sent once");
        }, move || {
            let _resource = resource;
            started.send(()).expect("callback started");
            blocked.recv().expect("release callback");
        }).expect("start tracked native worker");
        entered.recv_timeout(Duration::from_secs(5)).expect("native callback entered");
        assert!(watch.is_running());
        let (completed, native) = watch.stop_tracked();
        assert!(cancelled.load(Ordering::Acquire));
        let mut completed = Box::pin(completed);
        let mut context = Context::from_waker(Waker::noop());
        assert!(completed.as_mut().poll(&mut context).is_pending());
        assert!(!native.is_finished());
        assert!(!dropped.load(Ordering::Acquire));
        release.send(()).expect("finish native callback");
        native.join().expect("join native worker");
        assert!(matches!(completed.as_mut().poll(&mut context), Poll::Ready(Ok(()))));
        assert!(dropped.load(Ordering::Acquire));
        assert!(!running.load(Ordering::Acquire));

        let running = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false)); let resource = CallbackResource(dropped.clone());
        let watch = start_watch_thread("clipboard-watch-unwind", running.clone(), || {}, move || {
            let _resource = resource;
            panic!("native watch failure for ownership regression");
        }).expect("start failing native worker");
        let (completed, native) = watch.stop_tracked();
        assert!(native.join().is_err(), "native panic must remain visible to the owner");
        let mut completed = Box::pin(completed);
        assert!(matches!(completed.as_mut().poll(&mut context), Poll::Ready(Ok(()))));
        assert!(dropped.load(Ordering::Acquire));
        assert!(!running.load(Ordering::Acquire));
    }
}
