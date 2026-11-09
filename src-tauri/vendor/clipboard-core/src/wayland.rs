//! Wayland data-control backend extracted from node-clipboard-rs' native core.
//! The watcher reads every MIME representation from the changed offer itself,
//! so one snapshot cannot accidentally combine two clipboard selections.

use crate::{ChangeCallback, ClipboardResult, ClipboardSnapshot, start_wayland_watch};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use url::Url;
use wl_clipboard_rs::copy::{MimeSource, MimeType as CopyMimeType, Options, Source};
use wl_clipboard_rs::paste::{
    self, ClipboardType, Error as PasteError, MimeType, Seat as PasteSeat,
};
use wl_clipboard_rs::watch::Watcher;

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

pub(super) fn read() -> ClipboardResult<ClipboardSnapshot> {
    let mimes = match get_mimes() {
        Ok(mimes) => mimes,
        Err(PasteError::ClipboardEmpty) => return Ok(ClipboardSnapshot::default()),
        Err(error) => return Err(error.into()),
    };
    read_offer(mimes, |mime| read_one(MimeType::Specific(mime)))
}

pub(super) fn watch(callback: ChangeCallback) -> ClipboardResult<crate::WatchHandle> {
    let watcher = Watcher::new(ClipboardType::Regular.into(), PasteSeat::Unspecified)?;
    let running = Arc::new(AtomicBool::new(true));
    start_wayland_watch(watcher, callback, running)
}

pub(super) fn read_offer(
    offered_mimes: Vec<String>,
    mut read: impl FnMut(&str) -> ClipboardResult<Vec<u8>>,
) -> ClipboardResult<ClipboardSnapshot> {
    let mut snapshot = ClipboardSnapshot::default();
    let mut formats = crate::FormatReader::new();
    if let Some(mime) = find_mime(&offered_mimes, TEXT_MIMES, is_text_mime) {
        snapshot.text = formats.read("text", read(mime)).map(decode_utf8);
    }
    if let Some(mime) = find_mime(&offered_mimes, HTML_MIMES, is_html_mime) {
        snapshot.html = formats.read("html", read(mime)).map(decode_utf8);
    }
    if let Some(mime) = find_mime(&offered_mimes, RTF_MIMES, is_rtf_mime) {
        snapshot.rtf = formats.read("rtf", read(mime)).map(decode_utf8);
    }
    if let Some(mime) = find_mime(&offered_mimes, IMAGE_MIMES, is_image_mime) {
        snapshot.image = formats.read("image", read(mime));
    }
    if let Some(mime) = find_mime(&offered_mimes, FILE_MIMES, is_files_mime) {
        snapshot.files = formats
            .read("files", read(mime).and_then(|bytes| decode_files(&bytes)))
            .unwrap_or_default();
    }
    formats.finish(snapshot)
}

pub(super) fn write(snapshot: &ClipboardSnapshot) -> ClipboardResult<()> {
    let mut sources = Vec::new();
    if let Some(text) = &snapshot.text {
        sources.push(MimeSource {
            source: Source::Bytes(text.as_bytes().to_vec().into_boxed_slice()),
            mime_type: CopyMimeType::Text,
        });
    }
    if let Some(html) = &snapshot.html {
        push_source(&mut sources, "text/html", html.as_bytes());
    }
    if let Some(rtf) = &snapshot.rtf {
        for mime in RTF_MIMES {
            push_source(&mut sources, mime, rtf.as_bytes());
        }
    }
    if let Some(image) = &snapshot.image {
        let mime = detect_image_mime(image);
        sources.push(MimeSource {
            source: Source::Bytes(image.clone().into_boxed_slice()),
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

fn get_mimes() -> Result<Vec<String>, PasteError> {
    paste::get_mime_types_ordered(ClipboardType::Regular, PasteSeat::Unspecified)
}

fn read_one(mime: MimeType<'_>) -> ClipboardResult<Vec<u8>> {
    let (mut pipe, _) = paste::get_contents(ClipboardType::Regular, PasteSeat::Unspecified, mime)?;
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
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
    String::from_utf8_lossy(&bytes).into_owned()
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
