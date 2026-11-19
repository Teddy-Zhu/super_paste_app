use gpui_kit::SharedString;
use super_paste_core::preview::text_whitespace;

/// HTML normal-flow labels collapse ASCII whitespace, retaining NBSP and
/// other Unicode spaces. Unchanged shared strings keep their allocation.
pub(crate) fn normal_flow_text(value: impl Into<SharedString>) -> SharedString {
    normal_flow(value.into(), false)
}

/// Contact values apply the original JavaScript trim before CSS whitespace.
pub(crate) fn trimmed_normal_flow_text(value: impl Into<SharedString>) -> SharedString {
    normal_flow(value.into(), true)
}

fn normal_flow(value: SharedString, trim: bool) -> SharedString {
    let source: &str = if trim { value.trim_matches(text_whitespace) } else { &value };
    let mut space = true; let mut normalize = false;
    for byte in source.as_bytes() {
        match byte {
            b' ' => { normalize |= space; space = true; }
            b'\t' | b'\r' | b'\n' | 0x0c => { normalize = true; space = true; }
            _ => space = false,
        }
    }
    if !normalize && !(space && !source.is_empty()) {
        return if source.len() == value.len() { value } else { SharedString::new(source) };
    }
    let mut output = String::with_capacity(source.len()); let mut pending = false;
    for character in source.chars() {
        if matches!(character, ' ' | '\t' | '\r' | '\n' | '\u{000c}') { pending = !output.is_empty(); }
        else { if pending { output.push(' '); pending = false; } output.push(character); }
    }
    output.into()
}

/// CSS treats literal CR as a space. Line feeds and all other source bytes
/// stay intact; the original payload is never rewritten.
pub(crate) fn preformatted_text(value: impl Into<SharedString>) -> SharedString {
    let value = value.into();
    if value.contains('\r') { SharedString::from(value.replace('\r', " ")) } else { value }
}
