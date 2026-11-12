use clipboard_core::ClipboardSnapshot;
use serde_json::{Value, json};
use std::sync::LazyLock;

pub fn textual_content(snapshot: &ClipboardSnapshot) -> Value {
    let mut value = serde_json::Map::new();
    for (name, text) in [
        ("text", &snapshot.text),
        ("html", &snapshot.html),
        ("rtf", &snapshot.rtf),
    ] {
        if let Some(text) = text {
            value.insert(name.into(), json!(text));
        }
    }
    if !snapshot.files.is_empty() {
        value.insert("files".into(), json!(snapshot.files));
    }
    Value::Object(value)
}

pub fn from_content(value: &Value) -> Result<ClipboardSnapshot, String> {
    let image = value
        .get("image")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .transpose()?;
    let files = value
        .get("files")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or_default();
    Ok(ClipboardSnapshot {
        text: value.get("text").and_then(Value::as_str).map(str::to_owned),
        html: value.get("html").and_then(Value::as_str).map(str::to_owned),
        rtf: value.get("rtf").and_then(Value::as_str).map(str::to_owned),
        image,
        files,
    })
}

pub(super) fn formats(snapshot: &ClipboardSnapshot) -> Vec<&'static str> {
    let mut formats = Vec::new();
    if snapshot.text.as_ref().is_some_and(|v| !v.is_empty()) {
        formats.push("text");
    }
    if snapshot.html.as_ref().is_some_and(|v| !v.is_empty()) {
        formats.push("html");
    }
    if snapshot.image.is_some() {
        formats.push("image");
    }
    if snapshot.rtf.as_ref().is_some_and(|v| !v.is_empty()) {
        formats.push("rtf");
    }
    if !snapshot.files.is_empty() {
        formats.push("files");
    }
    formats
}

pub fn is_color(text: &str) -> bool {
    static COLOR: LazyLock<regex::Regex> = LazyLock::new(|| {
        let pattern: String =
            serde_json::from_str(include_str!("../../../src/shared/color-pattern.json"))
                .expect("constant color pattern JSON");
        regex::Regex::new(&format!("(?i){pattern}")).expect("constant color regex")
    });
    COLOR.is_match(text.trim())
}

pub(super) fn primary_type(snapshot: &ClipboardSnapshot) -> &'static str {
    static URL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)^https?://[^\s]+$").expect("constant URL regex"));
    static EMAIL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").expect("constant email regex")
    });
    static PHONE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^1[3-9][0-9]{9}$").expect("constant phone regex"));
    if !snapshot.files.is_empty() {
        return if snapshot.files.len() == 1 && snapshot.image.is_some() {
            "image"
        } else {
            "files"
        };
    }
    if snapshot.image.is_some() {
        return "image";
    }
    // 网页复制通常同时提供纯文本与富文本，独立颜色值优先用于分类。
    if is_color(snapshot.text.as_deref().unwrap_or("")) {
        return "color";
    }
    if snapshot.rtf.as_ref().is_some_and(|v| !v.is_empty()) {
        return "rtf";
    }
    if snapshot.html.as_ref().is_some_and(|v| !v.is_empty()) {
        return "html";
    }
    let raw_text = snapshot.text.as_deref().unwrap_or("");
    if raw_text.is_empty() {
        return "empty";
    }
    let text = raw_text.trim();
    if URL.is_match(text) {
        return "url";
    }
    if EMAIL.is_match(text) {
        return "email";
    }
    let phone: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if PHONE.is_match(&phone) {
        return "phone";
    }
    if (text.contains('{') && text.contains('}'))
        || ["function", "class ", "import ", "export "]
            .iter()
            .any(|value| text.contains(value))
    {
        return "code";
    }
    "text"
}

pub fn plain_text(content: &Value) -> Result<String, String> {
    if let Some(text) = content
        .get("text")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        return Ok(text.to_owned());
    }
    if let Some(html) = content.get("html").and_then(Value::as_str) {
        let html = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>")
            .expect("constant script regex")
            .replace_all(html, "");
        let html = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>")
            .expect("constant style regex")
            .replace_all(&html, "");
        let html = regex::Regex::new(r"<[^>]*>")
            .expect("constant tag regex")
            .replace_all(&html, "");
        let html = html
            .replace("&nbsp;", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'");
        let text = html.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            return Ok(text);
        }
    }
    if let Some(rtf) = content.get("rtf").and_then(Value::as_str) {
        let rtf = regex::Regex::new(r"\\[a-z]+[0-9]*\s?")
            .expect("constant RTF regex")
            .replace_all(rtf, "");
        let text = rtf
            .replace(['{', '}'], "")
            .replace("\\\\", "\\")
            .replace("\\'", "'")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            return Ok(text);
        }
    }
    if let Some(files) = content
        .get("files")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        return files
            .iter()
            .map(|v| v.as_str().ok_or("无效文件路径"))
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join("\n"))
            .map_err(str::to_owned);
    }
    Err("该记录没有可恢复的纯文本".into())
}

pub(super) fn content_bytes(snapshot: &ClipboardSnapshot) -> u64 {
    snapshot.text.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.html.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.rtf.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.image.as_ref().map_or(0, |v| v.len() as u64)
        + snapshot.files.iter().map(|v| v.len() as u64).sum::<u64>()
}
