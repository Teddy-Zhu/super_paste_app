//! Wayland data-control backend extracted from node-clipboard-rs' native core.
//! The watcher reads every MIME representation from the changed offer itself,
//! so one snapshot cannot accidentally combine two clipboard selections.

use crate::{ChangeCallback, ClipboardImage, ClipboardResult, ClipboardSnapshot, ReadCancellation, start_wayland_watch};
use std::io::Read;
use std::os::{fd::AsFd, unix::net::UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use url::Url;
use wl_clipboard_rs::copy::{MimeSource, MimeType as CopyMimeType, Options, Source};
use wl_clipboard_rs::paste::{ClipboardType, Seat as PasteSeat};
use wl_clipboard_rs::watch::{CancelHandle, ClipboardEvent, Watcher};

pub(super) struct ReadControl {
    ready: UnixStream,
    signal: UnixStream,
    state: Mutex<ControlState>,
}

#[derive(Default)]
struct ControlState { initializing: Option<UnixStream>, watcher: Option<CancelHandle> }

impl ReadControl {
    pub fn new() -> ClipboardResult<Self> {
        let (ready, signal) = UnixStream::pair()?;
        Ok(Self { ready, signal, state: Mutex::new(ControlState::default()) })
    }

    pub fn cancel(&self) {
        // EOF is persistent and needs no write, timer or polling thread.
        if let Err(error) = self.signal.shutdown(std::net::Shutdown::Write) {
            tracing::error!(%error, "could not signal clipboard pipe cancellation");
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(socket) = state.initializing.take() {
            if let Err(error) = socket.shutdown(std::net::Shutdown::Both) {
                tracing::error!(%error, "could not cancel clipboard protocol initialization");
            }
        }
        if let Some(watcher) = state.watcher.take() { watcher.cancel(); }
    }

    fn register_initialization(&self, socket: UnixStream, cancel: &ReadCancellation) -> ClipboardResult<()> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        cancel.check()?;
        state.initializing = Some(socket);
        Ok(())
    }

    fn register(&self, watcher: &Watcher, cancel: &ReadCancellation) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let handle = watcher.cancel_handle();
        // The watcher event pipe takes over only after both initial roundtrips.
        // Dropping our duplicate leaves the exclusively owned connection alive.
        state.initializing = None;
        if cancel.is_cancelled() { handle.cancel(); } else { state.watcher = Some(handle); }
    }
}

fn new_watcher(cancel: &ReadCancellation) -> ClipboardResult<Watcher> {
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with};
    use wayland_client::{ConnectError, Connection};
    cancel.check()?;
    // Preserve wayland-client's ownership/CLOEXEC rules for an inherited fd.
    let connection = if std::env::var("WAYLAND_SOCKET").is_ok() {
        Connection::connect_to_env()?
    } else {
        let name = std::env::var_os("WAYLAND_DISPLAY").map(PathBuf::from).ok_or(ConnectError::NoCompositor)?;
        let path = if name.is_absolute() { name } else {
            let directory = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).ok_or(ConnectError::NoCompositor)?;
            if !directory.is_absolute() { return Err(ConnectError::NoCompositor.into()); }
            directory.join(name)
        };
        let socket = socket_with(AddressFamily::UNIX, SocketType::STREAM, SocketFlags::NONBLOCK | SocketFlags::CLOEXEC, None)?;
        // Linux AF_UNIX reports EAGAIN when it cannot connect immediately;
        // unlike EINPROGRESS for TCP, this is not a pending connection to poll.
        // Close/report it rather than blocking on a full listen backlog.
        connect(&socket, &SocketAddrUnix::new(path)?)?;
        cancel.check()?;
        Connection::from_socket(UnixStream::from(socket))?
    };
    watcher_from_connection(connection, cancel)
}

fn watcher_from_connection(connection: wayland_client::Connection, cancel: &ReadCancellation) -> ClipboardResult<Watcher> {
    let control = cancel.0.wayland.as_ref().expect("Wayland initialization control");
    let socket = UnixStream::from(connection.backend().poll_fd().try_clone_to_owned()?);
    control.register_initialization(socket, cancel)?;
    let watcher = match Watcher::from_connection(ClipboardType::Regular.into(), PasteSeat::Unspecified, connection) {
        Ok(watcher) => watcher,
        Err(error) => {
            control.state.lock().unwrap_or_else(|error| error.into_inner()).initializing = None;
            return Err(error.into());
        },
    };
    control.register(&watcher, cancel);
    cancel.check()?;
    Ok(watcher)
}

const TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "text/plain",
    "utf8_string",
    "text",
    "string",
];
const HTML_MIMES: &[&str] = &["text/html"];
const RTF_MIMES: &[&str] = &[
    "text/rtf",
    "application/rtf",
    "application/x-rtf",
    "text/richtext",
];
const IMAGE_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/bmp",
    "image/gif",
    "application/x-qt-image",
];
const FILE_MIMES: &[&str] = &[
    "text/uri-list",
    "x-special/gnome-copied-files",
    "x-special/nautilus-clipboard",
];

pub(super) fn check_available() -> ClipboardResult<()> {
    drop(Watcher::new(
        ClipboardType::Regular.into(),
        PasteSeat::Unspecified,
    )?);
    Ok(())
}

pub(super) fn read(cancel: &ReadCancellation) -> ClipboardResult<ClipboardSnapshot> {
    // The first event reports the current selection on one resolved seat.
    // Retain its offer across all representations instead of re-fetching the
    // current clipboard (and resolving a seat) for every MIME separately.
    let mut watcher = new_watcher(cancel)?;
    let Some(event) = watcher.next_event()? else { return Err("clipboard read cancelled before the initial selection".into()); };
    read_event(event, cancel)
}

pub(super) fn watch(callback: ChangeCallback, cancel: ReadCancellation) -> ClipboardResult<crate::WatchHandle> {
    let watcher = new_watcher(&cancel)?;
    start_wayland_watch(watcher, callback, cancel)
}

pub(super) fn read_event(event: ClipboardEvent<'_>, cancel: &ReadCancellation) -> ClipboardResult<ClipboardSnapshot> {
    cancel.check()?;
    match event {
        ClipboardEvent::Changed { mime_types, mut offer, .. } => read_offer(mime_types, cancel,
            |mime| read_pipe(offer.receive(mime)?, cancel)),
        ClipboardEvent::Cleared { .. } => Ok(ClipboardSnapshot::default()),
    }
}

fn read_pipe(pipe: impl Read + AsFd, cancel: &ReadCancellation) -> ClipboardResult<Vec<u8>> {
    let mut bytes = Vec::new();
    // Keep std's direct Vec growth/read path; cancellation adds no staging copy.
    CancellablePipe { pipe, cancel }.read_to_end(&mut bytes)?;
    Ok(bytes)
}

struct CancellablePipe<'a, R> { pipe: R, cancel: &'a ReadCancellation }

impl<R: Read + AsFd> Read for CancellablePipe<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use rustix::event::{poll, PollFd, PollFlags};
        let interrupted = || std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "clipboard pipe cancelled");
        if buffer.is_empty() { return Ok(0); }
        let control = self.cancel.0.wayland.as_ref().expect("Wayland pipe control");
        loop {
            if self.cancel.is_cancelled() { return Err(interrupted()); }
            let mut fds = [PollFd::new(&self.pipe, PollFlags::IN), PollFd::new(&control.ready, PollFlags::IN)];
            match poll(&mut fds, None) {
                Ok(_) => {},
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            // ConnectionAborted is terminal: read_to_end retries Interrupted.
            if self.cancel.is_cancelled() || !fds[1].revents().is_empty() { return Err(interrupted()); }
            if fds[0].revents().contains(PollFlags::NVAL) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid clipboard pipe"));
            }
            break;
        }
        let count = self.pipe.read(buffer)?;
        if self.cancel.is_cancelled() { return Err(interrupted()); }
        Ok(count)
    }
}

fn read_offer(
    offered_mimes: Vec<String>,
    cancel: &ReadCancellation,
    mut read: impl FnMut(&str) -> ClipboardResult<Vec<u8>>,
) -> ClipboardResult<ClipboardSnapshot> {
    cancel.check()?;
    let mut snapshot = ClipboardSnapshot::default();
    let mut formats = crate::FormatReader::new();
    if let Some(mime) = find_mime(&offered_mimes, TEXT_MIMES, is_text_mime) {
        let result = read(mime);
        cancel.check()?;
        snapshot.text = formats.read("text", result).map(decode_utf8);
    }
    if let Some(mime) = find_mime(&offered_mimes, HTML_MIMES, is_html_mime) {
        let result = read(mime);
        cancel.check()?;
        snapshot.html = formats.read("html", result).map(decode_utf8);
    }
    if let Some(mime) = find_mime(&offered_mimes, RTF_MIMES, is_rtf_mime) {
        let result = read(mime);
        cancel.check()?;
        snapshot.rtf = formats.read("rtf", result).map(decode_utf8);
    }
    if let Some(mime) = find_mime(&offered_mimes, IMAGE_MIMES, is_image_mime) {
        let result = read(mime);
        cancel.check()?;
        snapshot.image = formats.read("image", result).map(ClipboardImage::Encoded);
    }
    if let Some(mime) = find_mime(&offered_mimes, FILE_MIMES, is_files_mime) {
        let result = read(mime);
        cancel.check()?;
        snapshot.files = formats
            .read("files", result.and_then(|bytes| decode_files(&bytes)))
            .unwrap_or_default();
    }
    formats.finish(snapshot)
}

pub(super) fn write(snapshot: ClipboardSnapshot) -> ClipboardResult<()> {
    let mut sources = Vec::new();
    if let Some(text) = snapshot.text {
        sources.push(MimeSource {
            source: Source::Bytes(text.into_bytes().into_boxed_slice()),
            mime_type: CopyMimeType::Text,
        });
    }
    if let Some(html) = snapshot.html {
        sources.push(MimeSource { source: Source::Bytes(html.into_bytes().into_boxed_slice()), mime_type: CopyMimeType::Specific("text/html".into()) });
    }
    if let Some(rtf) = &snapshot.rtf {
        for mime in RTF_MIMES {
            push_source(&mut sources, mime, rtf.as_bytes());
        }
    }
    if let Some(image) = snapshot.image {
        let ClipboardImage::Encoded(image) = image else {
            return Err("Wayland clipboard images must be encoded by the shared image service".into());
        };
        let mime = detect_image_mime(&image);
        sources.push(MimeSource {
            source: Source::Bytes(image.into_boxed_slice()),
            mime_type: mime,
        });
    }
    if !snapshot.files.is_empty() {
        let files = snapshot
            .files
            .iter()
            .map(|file| file_uri(file))
            .collect::<ClipboardResult<Vec<_>>>()?;
        let uri_list = format!("{}\r\n", files.join("\r\n"));
        push_source(&mut sources, "text/uri-list", uri_list.as_bytes());

        let gnome_list = format!("copy\n{}\n", files.join("\n"));
        for mime in [
            "x-special/gnome-copied-files",
            "x-special/nautilus-clipboard",
        ] {
            push_source(&mut sources, mime, gnome_list.as_bytes());
        }
    }
    if sources.is_empty() {
        return Err("clipboard snapshot contains no restorable formats".into());
    }

    Options::new().copy_multi(sources)?;
    Ok(())
}

fn find_mime<'a>(
    offered: &'a [String],
    priority: &[&str],
    is_matching: impl Fn(&str) -> bool,
) -> Option<&'a str> {
    priority
        .iter()
        .find_map(|wanted| {
            offered
                .iter()
                .find(|mime| mime.eq_ignore_ascii_case(wanted))
        })
        .or_else(|| offered.iter().find(|mime| is_matching(mime)))
        .map(String::as_str)
}

fn is_text_mime(mime: &str) -> bool {
    matches!(
        mime.trim().to_ascii_lowercase().as_str(),
        "text/plain" | "text/plain;charset=utf-8" | "utf8_string" | "text" | "string"
    )
}

fn is_html_mime(mime: &str) -> bool {
    mime.trim().eq_ignore_ascii_case("text/html")
}

fn is_rtf_mime(mime: &str) -> bool {
    matches!(
        mime.trim().to_ascii_lowercase().as_str(),
        "text/rtf" | "text/richtext" | "application/rtf" | "application/x-rtf"
    )
}

fn is_image_mime(mime: &str) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    mime.starts_with("image/") || mime == "application/x-qt-image"
}

fn is_files_mime(mime: &str) -> bool {
    matches!(
        mime.trim().to_ascii_lowercase().as_str(),
        "text/uri-list" | "x-special/gnome-copied-files" | "x-special/nautilus-clipboard"
    )
}

fn decode_utf8(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    }
}

fn decode_files(bytes: &[u8]) -> ClipboardResult<Vec<String>> {
    let raw = String::from_utf8_lossy(bytes);
    let mut files = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.eq_ignore_ascii_case("copy")
            || line.eq_ignore_ascii_case("cut")
        {
            continue;
        }
        files.push(local_path(line)?);
    }
    Ok(files)
}

fn local_path(file: &str) -> ClipboardResult<String> {
    let path = if Path::new(file).is_absolute() {
        PathBuf::from(file)
    } else {
        let uri = Url::parse(file)?;
        if uri.scheme() != "file" || uri.query().is_some() || uri.fragment().is_some() {
            return Err("clipboard file entries must be local file URIs".into());
        }
        uri.to_file_path()
            .map_err(|_| "clipboard file URI cannot be represented as a local path")?
    };
    let value = path
        .into_os_string()
        .into_string()
        .map_err(|_| "clipboard file path is not valid UTF-8")?;
    if value.contains('\0') {
        return Err("clipboard file path contains a NUL byte".into());
    }
    Ok(value)
}

fn file_uri(file: &str) -> ClipboardResult<String> {
    let path = local_path(file)?;
    Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| "clipboard file path cannot be represented as a local file URI".into())
}

fn push_source(sources: &mut Vec<MimeSource>, mime: &str, bytes: &[u8]) {
    sources.push(MimeSource {
        source: Source::Bytes(bytes.to_vec().into_boxed_slice()),
        mime_type: CopyMimeType::Specific(mime.to_owned()),
    });
}

fn detect_image_mime(bytes: &[u8]) -> CopyMimeType {
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    mime.map_or(CopyMimeType::Autodetect, |mime| {
        CopyMimeType::Specific(mime.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialization_cancellation_disconnects_a_stalled_protocol_roundtrip() {
        use std::{sync::mpsc, time::Duration};

        for cancelled_before_registration in [false, true] {
            let cancel = ReadCancellation::for_wayland().expect("local cancellation descriptor");
            let (client, mut server) = UnixStream::pair().expect("private protocol transport");
            server.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let connection = wayland_client::Connection::from_socket(client).expect("native Wayland connection");
            if cancelled_before_registration { cancel.cancel(); }
            let native_cancel = cancel.clone();
            let (completed, completion) = mpsc::channel();
            let native = std::thread::spawn(move || {
                let result = watcher_from_connection(connection, &native_cancel);
                completed.send(result.is_err()).unwrap();
            });
            if !cancelled_before_registration {
                // Read an actual registry/sync request, then deliberately send
                // no reply. This exercises the library's blocking roundtrip,
                // without a compositor, environment mutation or UI fixture.
                let mut request_header = [0; 8];
                server.read_exact(&mut request_header).expect("initial protocol request");
                cancel.cancel();
            }
            assert!(completion.recv_timeout(Duration::from_secs(5)).expect("cancelled initialization returned"));
            native.join().expect("native initialization did not panic");
            let mut remaining = Vec::new();
            server.read_to_end(&mut remaining).expect("private connection physically disconnected");
            if cancelled_before_registration { assert!(remaining.is_empty()); }
        }
    }

    #[test]
    fn pipe_cancellation_wakes_stalled_watch_and_discards_partial_snapshot() {
        use std::{io::Write, sync::{Arc, atomic::AtomicBool, mpsc}, time::Duration};

        let cancel = ReadCancellation::for_wayland().expect("local cancellation descriptor");
        let (reader, mut writer) = UnixStream::pair().expect("local offer pipe");
        let payload = "文字\0 😀".as_bytes();
        writer.write_all(payload).expect("write complete payload");
        writer.shutdown(std::net::Shutdown::Write).expect("offer completed");
        assert_eq!(read_pipe(reader, &cancel).expect("complete pipe read"), payload);

        let (reader, mut writer) = UnixStream::pair().expect("stalled offer pipe");
        writer.write_all(b"prefix").expect("publish partial format");
        let (started, entered) = mpsc::sync_channel(1);
        let (finished, completion) = mpsc::sync_channel(1);
        let stop = cancel.clone();
        let reading = cancel.clone();
        let watch = crate::start_watch_thread("clipboard-pipe-cancellation", Arc::new(AtomicBool::new(false)),
            move || stop.cancel(), move || {
                let mut pipe = CancellablePipe { pipe: reader, cancel: &reading };
                let mut prefix = [0; 6];
                pipe.read_exact(&mut prefix).expect("observe real partial data");
                assert_eq!(&prefix, b"prefix");
                started.send(()).expect("announce incomplete offer");
                let result = read_pipe(pipe.pipe, &reading);
                finished.send(result.map_err(|error| error.downcast::<std::io::Error>()
                    .expect("pipe I/O error").kind())).expect("report native completion");
            }).expect("start owned pipe reader");
        entered.recv_timeout(Duration::from_secs(2)).expect("partial offer read started");
        let (_completed, thread) = watch.stop_tracked();
        assert_eq!(completion.recv_timeout(Duration::from_secs(2)).expect("stop wakes incomplete offer"),
            Err(std::io::ErrorKind::ConnectionAborted));
        thread.join().expect("physical watch thread released");
        // Keep the writer open until after the stopped thread returns: EOF
        // from the clipboard source cannot be what released this read.
        drop(writer);
        cancel.cancel();

        let (reader, _writer) = UnixStream::pair().expect("pre-cancelled offer");
        assert!(read_pipe(reader, &cancel).is_err(), "cancellation persists without consuming its signal");

        let cancel = ReadCancellation::for_wayland().expect("format cancellation");
        let mut calls = Vec::new();
        let snapshot = read_offer(vec!["text/plain".into(), "text/html".into()], &cancel, |mime| {
            calls.push(mime.to_owned());
            if mime == "text/html" { cancel.cancel(); }
            Ok(b"partial snapshot".to_vec())
        });
        assert!(snapshot.is_err(), "cancelled HTML must not publish the previously read text alone");
        assert_eq!(calls, ["text/plain", "text/html"]);
    }

    #[test]
    fn offer_reading_keeps_format_policy_and_reuses_valid_utf8() {
        let cancel = ReadCancellation::for_wayland().expect("local cancellation descriptor");
        let mut text = Some("文字\0 😀".as_bytes().to_vec());
        let mut html = Some("<b>文字 😀</b>".as_bytes().to_vec());
        let mut rtf = Some(br"{\rtf1 text}".to_vec());
        let pointers = [text.as_ref().unwrap().as_ptr(), html.as_ref().unwrap().as_ptr(), rtf.as_ref().unwrap().as_ptr()];
        let mut calls = Vec::new();
        let snapshot = read_offer([
            "text/plain", "application/rtf", "image/png", "text/uri-list", "text/html", "text/plain;charset=utf-8",
        ].into_iter().map(str::to_owned).collect(), &cancel, |mime| {
            calls.push(mime.to_owned());
            Ok(match mime {
                "text/plain;charset=utf-8" => text.take().expect("read preferred text once"),
                "text/html" => html.take().expect("read HTML once"),
                "application/rtf" => rtf.take().expect("read RTF once"),
                "image/png" => b"\x89PNG\r\n\x1a\n".to_vec(),
                "text/uri-list" => b"# comment\r\nfile:///tmp/a%20b.txt\r\n".to_vec(),
                _ => panic!("unexpected representation: {mime}"),
            })
        }).expect("read all formats from one offer");
        assert_eq!(calls, ["text/plain;charset=utf-8", "text/html", "application/rtf", "image/png", "text/uri-list"]);
        assert_eq!(snapshot.text.as_deref(), Some("文字\0 😀"));
        assert_eq!(snapshot.html.as_deref(), Some("<b>文字 😀</b>"));
        assert_eq!(snapshot.rtf.as_deref(), Some(r"{\rtf1 text}"));
        for (value, original) in [snapshot.text.as_ref(), snapshot.html.as_ref(), snapshot.rtf.as_ref()].into_iter().zip(pointers) {
            assert_eq!(value.expect("decoded string").as_ptr(), original, "valid UTF-8 must reuse its native buffer");
        }
        assert!(matches!(&snapshot.image, Some(ClipboardImage::Encoded(bytes)) if bytes == b"\x89PNG\r\n\x1a\n"));
        assert_eq!(snapshot.files, ["/tmp/a b.txt"]);
        assert_eq!(decode_utf8(b"a\xff\0b".to_vec()), "a\u{fffd}\0b", "invalid UTF-8 retains the existing lossy policy");
        assert!(read_event(ClipboardEvent::Cleared { clipboard: ClipboardType::Regular }, &cancel).expect("empty selection").is_empty());

        let snapshot = read_offer(vec!["text/plain".into(), "image/png".into()], &cancel, |mime| match mime {
            "text/plain" => Err("advertised text is unavailable".into()),
            _ => Ok(vec![1, 2, 3]),
        }).expect("keep an available representation when another advertised MIME fails");
        assert!(snapshot.text.is_none());
        assert!(matches!(&snapshot.image, Some(ClipboardImage::Encoded(bytes)) if bytes == &[1, 2, 3]));
        assert!(read_offer(vec!["text/plain".into()], &cancel, |_| Err("representation unavailable".into())).is_err(),
            "failure of every offered representation must remain an error");
    }
}
