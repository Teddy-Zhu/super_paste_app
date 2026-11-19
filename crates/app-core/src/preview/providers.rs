use std::{collections::HashMap, sync::{Arc, LazyLock}};
use regex::Regex;
use super_paste_format::{PrimaryType, is_color};
use super::{ColorPreview, FilePreview, Input, LinkPreview, PreviewBody, PreviewKind, PreviewModel, PreviewNotice, PreviewScope, rich, structured, text_whitespace};
use crate::Result;

pub(super) struct Prepared {
    pub input: Input, pub needs_rich: bool,
    files: Vec<FilePreview>, files_total: u64, kind: PreviewKind, body: PreviewBody, display: Option<Arc<str>>,
}
pub(super) struct RichResolution {
    pub kind: PreviewKind, pub document: Option<Arc<rich::Document>>, pub notice: Option<PreviewNotice>,
}

/// Cheap recognition and bounded parsing run on the preview worker, not the
/// Tokio I/O threads. Rich documents have their own shared content identity.
pub(super) fn select(input: Input) -> Prepared {
    let content = &input.content; let primary_type = input.primary_type; let image = &input.image;
    let html_omitted = input.html_omitted; let rtf_omitted = input.rtf_omitted;
    let text = content.text.clone().unwrap_or_default(); let trimmed = text.trim_matches(text_whitespace);
    // A card excerpt must not turn a partial final line into a file target.
    let file_text = if input.text_clipped { text.rfind(['\r', '\n']).map_or("", |end| &text[..end]) } else { &text };
    let (files, files_total) = files(&content.files, primary_type, file_text, if input.scope == PreviewScope::Card { 4 } else { usize::MAX });
    let mut kind = PreviewKind::Empty; let mut body = PreviewBody::Empty;
    let mut display: Option<Arc<str>> = None; let mut needs_rich = false;
    if !files.is_empty() && (primary_type == PrimaryType::Files || image.is_none()) { kind = PreviewKind::Files; body = PreviewBody::Files; }
    else if image.is_some() { kind = PreviewKind::Image; body = PreviewBody::Image; }
    else if let Some(color) = color(trimmed) { kind = PreviewKind::Color; body = PreviewBody::Color(color); }
    else if content.html.as_deref().is_some_and(|value| !value.is_empty()) || content.rtf.as_deref().is_some_and(|value| !value.is_empty()) || html_omitted || rtf_omitted {
        kind = if content.html.is_some() || html_omitted { PreviewKind::Html } else { PreviewKind::Rtf };
        needs_rich = true;
    } else if let Some(link) = (!input.text_clipped).then(|| link(trimmed)).flatten() { kind = PreviewKind::Url; body = PreviewBody::Link(link); }
    else if !input.text_clipped && email(trimmed) { kind = PreviewKind::Email; body = PreviewBody::Contact; }
    else if !input.text_clipped && phone(trimmed, primary_type) { kind = PreviewKind::Phone; body = PreviewBody::Contact; }
    else if let Some(formatted) = (!input.text_clipped).then(|| structured::json(trimmed)).flatten() { kind = PreviewKind::Json; body = PreviewBody::Code; display = Some(formatted); }
    else if code(&text) { kind = PreviewKind::Code; body = PreviewBody::Code; }
    else if let Some(table) = (!input.text_clipped).then(|| structured::table(&text)).flatten() { kind = PreviewKind::Table; body = PreviewBody::Table(table); }
    else if !text.is_empty() { kind = if primary_type == PrimaryType::Code { PreviewKind::Code } else { PreviewKind::Text }; body = if kind == PreviewKind::Code { PreviewBody::Code } else { PreviewBody::Text }; }

    Prepared { input, needs_rich, files, files_total, kind, body, display }
}

pub(super) fn finish(prepared: Prepared, resolution: Option<RichResolution>) -> Result<PreviewModel> {
    let Prepared { input, files, files_total, mut kind, mut body, display, .. } = prepared;
    let Input { content, image, text_characters, text_clipped, scope, mut lease, .. } = input;
    let text = content.text.clone().unwrap_or_default(); let mut rich_text = None; let mut notice = None;
    if let Some(resolution) = resolution {
        kind = resolution.kind; notice = resolution.notice;
        if let Some(document) = resolution.document { rich_text = Some(document.text.clone()); body = PreviewBody::Rich(document); }
        else { body = if text.is_empty() { PreviewBody::Empty } else { PreviewBody::Text }; }
    }
    let text = if text.is_empty() { rich_text.unwrap_or(text) } else { text };
    let mut display_text = display.unwrap_or_else(|| text.clone());
    let mut display_clipped = false;
    if scope == PreviewScope::Card {
        let clipped = structured::excerpt(&display_text, 12_000);
        if clipped.len() != display_text.len() { display_clipped = true; display_text = clipped.into(); }
    }
    let available = text.chars().count() as u64;
    let summary: Arc<str> = if matches!(body, PreviewBody::Files) { format!("{} 个文件", grouped(files_total)).into() }
        else if matches!(body, PreviewBody::Image) { "图片".into() }
        else if let PreviewBody::Table(table) = &body { format!("{} 行 × {} 列", grouped(table.rows.len() as u64), grouped(table.columns as u64)).into() }
        else if !text.is_empty() {
            let characters = if content.text.as_ref().is_some_and(|text| !text.is_empty()) { text_characters } else { available };
            let mut lines = 1; let mut previous = '\0'; for character in text.chars() { if character == '\r' || character == '\n' && previous != '\r' { lines += 1; } previous = character; }
            if characters <= available && lines > 1 { format!("{} 字符 · {} 行", grouped(characters), grouped(lines)).into() } else { format!("{} 字符", grouped(characters)).into() }
        } else { Arc::default() };
    if text_clipped || display_clipped || content.text.as_ref().is_some_and(|text| !text.is_empty()) && text_characters > available { notice = notice.or(Some(PreviewNotice::Excerpt)); }
    // Document allocations carry their own lease, shared across both scopes.
    let document_text = match &body { PreviewBody::Rich(document) => Some(&document.text), _ => None };
    let text_owned = !document_text.is_some_and(|value| Arc::ptr_eq(value, &text));
    let mut weight = std::mem::size_of::<PreviewModel>() as u64 + if text_owned { text.len() as u64 } else { 0 } + summary.len() as u64;
    if !Arc::ptr_eq(&text, &display_text) { weight += display_text.len() as u64; }
    weight += files.iter().map(|file| std::mem::size_of::<FilePreview>() as u64 + (file.path.len() + file.name.len() + file.directory.len() + file.extension.len()) as u64).sum::<u64>();
    weight += match &body {
        PreviewBody::Table(table) => table.rows.iter().map(|row| row.iter().map(|cell| cell.len() as u64 + 16).sum::<u64>() + 16).sum(),
        PreviewBody::Link(link) => (link.href.len() + link.host.len() + link.detail.len()) as u64,
        PreviewBody::Color(color) => (color.original.len() + color.hex.len() + color.rgb.len()) as u64,
        _ => 0,
    };
    // Release raw payloads and file inputs before admitting another worker
    // against the smaller retained-model charge. Shared text stays in `text`.
    drop(content);
    if weight > lease.bytes() { lease.grow(weight - lease.bytes())?; } else { lease.shrink(weight); }
    Ok(PreviewModel { kind, body, scope, text, display_text, summary, notice, image, files: files.into(), files_total, lease })
}
fn grouped(value: u64) -> String {
    let text = value.to_string(); let mut result = String::with_capacity(text.len() + text.len() / 3);
    for (index, character) in text.chars().enumerate() { if index > 0 && (text.len() - index) % 3 == 0 { result.push(','); } result.push(character); } result
}

fn color(text: &str) -> Option<ColorPreview> {
    if !is_color(text) { return None; }
    let value = csscolorparser::parse(text).ok()?.clamp(); let [r, g, b, a] = value.to_rgba8(); let alpha = value.a;
    let serialized_alpha = if text.starts_with('#') { legacy_byte_alpha(a) } else { alpha };
    Some(ColorPreview { original: text.into(), rgba: [value.r as f32, value.g as f32, value.b as f32, alpha as f32],
        hex: if alpha < 1.0 { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}") }.into(),
        rgb: if alpha < 1.0 { format!("rgba({r}, {g}, {b}, {serialized_alpha})") } else { format!("rgb({r}, {g}, {b})") }.into() })
}

fn legacy_byte_alpha(alpha: u8) -> f32 {
    // CSS Color 4 legacy alpha serialization, used by the original Canvas parser.
    // Integer arithmetic preserves the prescribed midpoint rounding (e.g. 128 -> 0.5).
    let alpha = u32::from(alpha);
    let percent = (alpha * 100 + 127) / 255;
    if (percent * 255 + 50) / 100 == alpha { percent as f32 / 100.0 }
    else { ((alpha * 1000 + 127) / 255) as f32 / 1000.0 }
}
fn code(text: &str) -> bool {
    static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(?:\s*(?:const|let|var|function|class|import|export|def|public|SELECT|INSERT|UPDATE)\b|\s*```)").expect("valid code preview expression"));
    CODE.is_match(text)
}
fn email(text: &str) -> bool {
    static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^@]+@[^@]+\.[^@]+$").expect("valid email preview expression"));
    !text.chars().any(text_whitespace) && EMAIL.is_match(text)
}
fn phone(text: &str, kind: PrimaryType) -> bool {
    static PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:\+[0-9 ()-]+|\([0-9]{2,4}\)[0-9 -]+|0[0-9]{2,3}[- ][0-9]{7,8})$").expect("valid phone preview expression"));
    kind == PrimaryType::Phone || PHONE.is_match(text) && (7..=15).contains(&text.chars().filter(char::is_ascii_digit).count())
}
fn link(text: &str) -> Option<LinkPreview> {
    let url = super::link::parse(text)?; let mut host = url.host()?.to_string();
    if let Some(port) = url.port() { host.push_str(&format!(":{port}")); }
    let mut detail = url.path().to_owned(); if let Some(query) = url.query() { detail.push('?'); detail.push_str(query); } if let Some(fragment) = url.fragment() { detail.push('#'); detail.push_str(fragment); }
    Some(LinkPreview { href: text.into(), host: host.into(), detail: decode_uri(&detail, true).into() })
}
fn decode_uri(value: &str, preserve_reserved: bool) -> String {
    let mut result = Vec::with_capacity(value.len()); let bytes = value.as_bytes(); let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(hex) = bytes.get(index + 1..index + 3) else { return value.to_owned(); };
            let Some(a) = (hex[0] as char).to_digit(16) else { return value.to_owned(); }; let Some(b) = (hex[1] as char).to_digit(16) else { return value.to_owned(); };
            let decoded = (a * 16 + b) as u8;
            if preserve_reserved && b";/?:@&=+$,#".contains(&decoded) { result.extend_from_slice(&bytes[index..index + 3]); } else { result.push(decoded); } index += 3;
        } else { result.push(bytes[index]); index += 1; }
    }
    String::from_utf8(result).unwrap_or_else(|_| value.to_owned())
}
fn files(paths: &[String], kind: PrimaryType, text: &str, limit: usize) -> (Vec<FilePreview>, u64) {
    let fallback;
    let paths: Vec<&str> = if !paths.is_empty() { paths.iter().map(String::as_str).collect() }
        else if kind == PrimaryType::Files {
            fallback = text.replace("\r\n", "\n").replace('\r', "\n"); fallback.split('\n').filter(|line| !line.starts_with('#') && *line != "copy" && *line != "cut").collect()
        } else { Vec::new() };
    let mut positions = HashMap::new(); let mut result = Vec::new();
    for raw in paths {
        if raw.trim_matches(text_whitespace).is_empty() { continue; }
        let mut path = raw.to_owned();
        if raw.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://")) {
            if let Ok(url) = url::Url::parse(raw) {
                path = decode_uri(url.path(), false);
                if let Some(host) = url.host_str().filter(|host| *host != "localhost" && !host.is_empty()) { path = format!("//{host}{path}"); }
                else if path.as_bytes().first() == Some(&b'/') && path.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) && path.as_bytes().get(2) == Some(&b':') && path.as_bytes().get(3).is_some_and(|byte| matches!(byte, b'/' | b'\\')) { path.remove(0); }
            }
        }
        let normalized = path.replace('\\', "/"); let bytes = normalized.as_bytes();
        let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':') && bytes.get(2) == Some(&b'/');
        let key = if drive || normalized.starts_with("//") { normalized.to_lowercase() } else { normalized.clone() };
        let index = if let Some(index) = positions.get(&key) { *index } else { let index = positions.len(); positions.insert(key, index); index };
        if index >= limit { continue; }
        let root = normalized == "/" || drive && normalized.len() == 3;
        let without_trailing = normalized.trim_end_matches('/'); let separator = without_trailing.rfind('/');
        let name = if root { path.as_str() } else { without_trailing.get(separator.map_or(0, |position| position + 1)..).filter(|name| !name.is_empty()).unwrap_or("未命名文件") };
        let directory = if root { "" } else { &normalized[..separator.map_or(0, |position| position + 1)] };
        let extension = if path.ends_with(['/', '\\']) { "文件夹".to_owned() } else { name.rfind('.').filter(|position| *position > 0).map(|position| name[position + 1..].to_uppercase()).unwrap_or_else(|| "文件".to_owned()) };
        let file = FilePreview { name: name.into(), directory: directory.into(), extension: extension.into(), path: path.into() };
        if let Some(slot) = result.get_mut(index) { *slot = file; } else { result.push(file); }
    }
    (result, positions.len() as u64)
}
