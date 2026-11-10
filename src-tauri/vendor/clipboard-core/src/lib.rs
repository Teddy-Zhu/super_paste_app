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
    /// Encoded clipboard image bytes, retained at source resolution.
    pub image: Option<Vec<u8>>,
    pub files: Vec<String>,
}

impl ClipboardSnapshot {
    pub fn is_empty(&self) -> bool {
        self.text.as_ref().is_none_or(String::is_empty)
            && self.html.as_ref().is_none_or(String::is_empty)
            && self.rtf.as_ref().is_none_or(String::is_empty)
            && self.image.as_ref().is_none_or(Vec::is_empty)
            && self.files.is_empty()
    }
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
    if is_wayland_session() {
        #[cfg(target_os = "linux")]
        return wayland::read();
    }
    read_native()
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
                    .and_then(|image| image.to_png().map(|png| png.get_bytes().to_vec())),
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
    if snapshot.is_empty() {
        return Err("clipboard snapshot contains no restorable formats".into());
    }
    if is_wayland_session() {
        #[cfg(target_os = "linux")]
        return wayland::write(snapshot);
    }
    write_native(snapshot)
}

fn write_native(snapshot: &ClipboardSnapshot) -> ClipboardResult<()> {
    let mut contents = Vec::new();

    // Windows' clipboard-rs backend clears the clipboard while installing an
    // image. Install it before the other representations so they remain
    // available as part of the same clipboard operation.
    if let Some(image) = &snapshot.image {
        contents.push(ClipboardContent::Image(RustImageData::from_bytes(image)?));
    }
    if let Some(text) = &snapshot.text {
        contents.push(ClipboardContent::Text(text.clone()));
    }
    if let Some(html) = &snapshot.html {
        contents.push(ClipboardContent::Html(html.clone()));
    }
    if let Some(rtf) = &snapshot.rtf {
        contents.push(ClipboardContent::Rtf(rtf.clone()));
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
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
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

pub fn watch(
    callback: impl FnMut(ClipboardResult<ClipboardSnapshot>) + Send + 'static,
) -> ClipboardResult<WatchHandle> {
    let callback: ChangeCallback = Box::new(callback);
    if is_wayland_session() {
        #[cfg(target_os = "linux")]
        return wayland::watch(callback);
    }
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
    let thread_state = running.clone();
    let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(0);
    thread::Builder::new()
        .name("clipboard-watch-native".into())
        .spawn(move || {
            let _thread_state = WatchThreadState::start(thread_state);
            let _ = ready_sender.send(());
            watcher.start_watch();
        })?;
    ready_receiver.recv()?;

    Ok(WatchHandle {
        cancel: Some(Box::new(move || drop(shutdown))),
        running,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn start_wayland_watch(
    mut watcher: wl_clipboard_rs::watch::Watcher,
    mut callback: ChangeCallback,
    running: std::sync::Arc<AtomicBool>,
) -> ClipboardResult<WatchHandle> {
    let cancel_handle = watcher.cancel_handle();
    let cancel_running = running.clone();
    let thread_running = std::sync::Arc::new(AtomicBool::new(false));
    let thread_state = thread_running.clone();
    let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(0);
    thread::Builder::new()
        .name("clipboard-watch-wayland".into())
        .spawn(move || {
            let _thread_state = WatchThreadState::start(thread_state);
            let _ = ready_sender.send(());
            while running.load(Ordering::Acquire) {
                let event = match watcher.next_event() {
                    Ok(Some(event)) => event,
                    Ok(None) => break,
                    Err(error) => {
                        callback(Err(Box::new(error)));
                        break;
                    }
                };
                match event {
                    wl_clipboard_rs::watch::ClipboardEvent::Changed {
                        mime_types,
                        mut offer,
                        ..
                    } => callback(wayland::read_offer(mime_types, |mime| {
                        let mut pipe = offer.receive(mime)?;
                        let mut payload = Vec::new();
                        std::io::Read::read_to_end(&mut pipe, &mut payload)?;
                        Ok(payload)
                    })),
                    wl_clipboard_rs::watch::ClipboardEvent::Cleared { .. } => {
                        callback(Ok(ClipboardSnapshot::default()));
                    }
                }
            }
        })?;
    ready_receiver.recv()?;

    Ok(WatchHandle {
        cancel: Some(Box::new(move || {
            cancel_running.store(false, Ordering::Release);
            cancel_handle.cancel();
        })),
        running: thread_running,
    })
}
