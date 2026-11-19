use std::sync::Arc;
use super::TablePreview;

/// Truncate like the original UTF-16 string slice without splitting a surrogate pair.
pub fn excerpt(text: &str, limit: usize) -> &str {
    let mut used = 0;
    for (index, character) in text.char_indices() {
        used += character.len_utf16();
        if used > limit { return &text[..index]; }
    }
    text
}

#[derive(Clone, Copy)]
enum Frame { ObjectKeyOrEnd, ObjectKey, Colon, ObjectValue, ObjectComma, ArrayValueOrEnd, ArrayValue, ArrayComma }

/// Validate iteratively without constructing a JSON value. Number lexemes,
/// duplicate object keys and escaped strings remain untouched.
pub(super) fn json(text: &str) -> Option<Arc<str>> {
    if text.encode_utf16().count() > 20_000 || !matches!(text.as_bytes().first(), Some(b'{' | b'[')) { return None; }
    let bytes = text.as_bytes(); let mut at = 0; let mut stack = Vec::new();
    value(bytes, &mut at, &mut stack)?;
    while let Some(frame) = stack.last_mut() {
        whitespace(bytes, &mut at);
        let current = *bytes.get(at)?;
        match *frame {
            Frame::ObjectKeyOrEnd if current == b'}' => { at += 1; stack.pop(); },
            Frame::ObjectKeyOrEnd | Frame::ObjectKey => { string(bytes, &mut at)?; *frame = Frame::Colon; },
            Frame::Colon => { if current != b':' { return None; } at += 1; *frame = Frame::ObjectValue; },
            Frame::ObjectValue => { *frame = Frame::ObjectComma; value(bytes, &mut at, &mut stack)?; },
            Frame::ObjectComma => match current { b',' => { at += 1; *frame = Frame::ObjectKey; }, b'}' => { at += 1; stack.pop(); }, _ => return None },
            Frame::ArrayValueOrEnd if current == b']' => { at += 1; stack.pop(); },
            Frame::ArrayValueOrEnd | Frame::ArrayValue => { *frame = Frame::ArrayComma; value(bytes, &mut at, &mut stack)?; },
            Frame::ArrayComma => match current { b',' => { at += 1; *frame = Frame::ArrayValue; }, b']' => { at += 1; stack.pop(); }, _ => return None },
        }
    }
    whitespace(bytes, &mut at);
    if at != bytes.len() { return None; }
    Some(format_json(text))
}
fn whitespace(bytes: &[u8], at: &mut usize) { while bytes.get(*at).is_some_and(|byte| matches!(byte, b' ' | b'\r' | b'\n' | b'\t')) { *at += 1; } }
fn string(bytes: &[u8], at: &mut usize) -> Option<()> {
    if bytes.get(*at) != Some(&b'"') { return None; } *at += 1;
    loop {
        match *bytes.get(*at)? {
            b'"' => { *at += 1; return Some(()); },
            b'\\' => {
                *at += 1;
                match *bytes.get(*at)? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => *at += 1,
                    b'u' => { *at += 1; for _ in 0..4 { if !bytes.get(*at)?.is_ascii_hexdigit() { return None; } *at += 1; } },
                    _ => return None,
                }
            },
            0..=31 => return None,
            _ => *at += 1,
        }
    }
}
fn value(bytes: &[u8], at: &mut usize, stack: &mut Vec<Frame>) -> Option<()> {
    whitespace(bytes, at);
    match *bytes.get(*at)? {
        b'{' => { *at += 1; stack.push(Frame::ObjectKeyOrEnd); },
        b'[' => { *at += 1; stack.push(Frame::ArrayValueOrEnd); },
        b'"' => string(bytes, at)?,
        b't' | b'f' | b'n' => {
            let word: &[u8] = match bytes[*at] { b't' => b"true", b'f' => b"false", _ => b"null" };
            if !bytes.get(*at..)?.starts_with(word) { return None; } *at += word.len();
        },
        b'-' | b'0'..=b'9' => {
            if bytes[*at] == b'-' { *at += 1; }
            match *bytes.get(*at)? {
                b'0' => *at += 1,
                b'1'..=b'9' => { *at += 1; while bytes.get(*at).is_some_and(u8::is_ascii_digit) { *at += 1; } },
                _ => return None,
            }
            if bytes.get(*at) == Some(&b'.') { *at += 1; digits(bytes, at)?; }
            if bytes.get(*at).is_some_and(|byte| matches!(byte, b'e' | b'E')) {
                *at += 1; if bytes.get(*at).is_some_and(|byte| matches!(byte, b'+' | b'-')) { *at += 1; } digits(bytes, at)?;
            }
        },
        _ => return None,
    }
    Some(())
}
fn digits(bytes: &[u8], at: &mut usize) -> Option<()> {
    let start = *at; while bytes.get(*at).is_some_and(u8::is_ascii_digit) { *at += 1; }
    (*at != start).then_some(())
}
fn format_json(text: &str) -> Arc<str> {
    let mut result = String::new(); let mut depth = 0; let mut quoted = false; let mut escaped = false;
    // Input is at most 20k UTF-16 units; output is capped at the original 200k limit.
    let mut units = 0usize;
    for character in text.chars() {
        let before = result.len();
        if quoted {
            result.push(character);
            if escaped { escaped = false; } else if character == '\\' { escaped = true; } else if character == '"' { quoted = false; }
            units += character.len_utf16();
        } else {
            match character {
                '"' => { quoted = true; result.push(character); },
                '{' | '[' => { depth += 1; if depth > 40 { return text.into(); } result.push(character); result.push('\n'); result.push_str(&"  ".repeat(depth)); },
                '}' | ']' => {
                    depth -= 1;
                    let end = result.trim_end().len(); result.truncate(end);
                    if !result.ends_with(['{', '[']) { result.push('\n'); result.push_str(&"  ".repeat(depth)); } result.push(character);
                },
                ',' => { result.push_str(",\n"); result.push_str(&"  ".repeat(depth)); },
                ':' => result.push_str(": "),
                value if !value.is_whitespace() => result.push(value),
                _ => {},
            }
            // Outside strings all emitted characters are ASCII; a close may trim.
            if result.len() >= before { units += result.len() - before; }
            else { units -= before - result.len(); }
        }
        if units > 200_000 { return text.into(); }
    }
    result.into()
}

pub(super) fn table(text: &str) -> Option<TablePreview> {
    if text.encode_utf16().count() > 20_000 || (!text.contains('\n') && !text.contains('\t')) { return None; }
    let delimiter = if text.contains('\t') { '\t' } else { ',' };
    if !text.contains(delimiter) { return None; }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let input = normalized.strip_suffix('\n').unwrap_or(&normalized); let mut characters = input.chars().peekable();
    let mut rows = Vec::new(); let mut row = Vec::new(); let mut cell = String::new(); let mut quoted = false; let mut closed = false;
    while let Some(character) = characters.next() {
        if character == '"' {
            if quoted && characters.peek() == Some(&'"') { cell.push('"'); characters.next(); }
            else if quoted || cell.is_empty() { quoted = !quoted; closed = !quoted; }
            else { if delimiter == ',' { return None; } cell.push(character); }
        } else if !quoted && (character == delimiter || character == '\n') {
            row.push(Arc::<str>::from(std::mem::take(&mut cell))); closed = false;
            if character == '\n' { rows.push(Arc::<[Arc<str>]>::from(std::mem::take(&mut row))); }
        } else { if closed && !quoted { return None; } cell.push(character); }
    }
    row.push(cell.into()); rows.push(row.into());
    let columns = rows.first()?.len();
    (!quoted && (rows.len() > 1 || delimiter == '\t') && columns > 1 && rows.iter().all(|row| row.len() == columns)).then(|| TablePreview { rows: rows.into(), columns })
}
