use std::{fmt, sync::{Arc, LazyLock}};
use bitflags::bitflags;
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name { pub fn new() -> Self { Self(Uuid::new_v4()) } }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse().map(Self) }
        }
    };
}
identifier!(RecordId);
identifier!(TagId);
identifier!(SourceId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Digest(pub [u8; 32]);

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Formats: u8 {
        const TEXT = 1;
        const HTML = 2;
        const IMAGE = 4;
        const RTF = 8;
        const FILES = 16;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrimaryType { Text, Image, Html, Url, Files, Email, Phone, Code, Rtf, Color, Empty }
impl PrimaryType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text", Self::Image => "image", Self::Html => "html",
            Self::Url => "url", Self::Files => "files", Self::Email => "email",
            Self::Phone => "phone", Self::Code => "code", Self::Rtf => "rtf",
            Self::Color => "color", Self::Empty => "empty",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "text" => Self::Text, "image" => Self::Image, "html" => Self::Html,
            "url" => Self::Url, "files" => Self::Files, "email" => Self::Email,
            "phone" => Self::Phone, "code" => Self::Code, "rtf" => Self::Rtf,
            "color" => Self::Color, "empty" => Self::Empty, _ => return None,
        })
    }
}

/// Original formats; derived OCR never participates in clipboard identity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextContent {
    pub text: Option<Arc<str>>,
    pub html: Option<Arc<str>>,
    pub rtf: Option<Arc<str>>,
    pub files: Arc<[String]>,
}
impl TextContent {
    pub fn formats(&self, has_image: bool) -> Formats {
        let mut result = Formats::empty();
        for (value, format) in [(&self.text, Formats::TEXT), (&self.html, Formats::HTML), (&self.rtf, Formats::RTF)] {
            if value.as_deref().is_some_and(|v| !v.is_empty()) { result.insert(format); }
        }
        result.set(Formats::IMAGE, has_image);
        result.set(Formats::FILES, !self.files.is_empty());
        result
    }
    pub fn primary_type(&self, has_image: bool) -> PrimaryType {
        if !self.files.is_empty() { return if self.files.len() == 1 && has_image { PrimaryType::Image } else { PrimaryType::Files }; }
        if has_image { return PrimaryType::Image; }
        let raw = self.text.as_deref().unwrap_or("");
        if is_color(raw) { return PrimaryType::Color; }
        if self.rtf.as_deref().is_some_and(|v| !v.is_empty()) { return PrimaryType::Rtf; }
        if self.html.as_deref().is_some_and(|v| !v.is_empty()) { return PrimaryType::Html; }
        if raw.is_empty() { return PrimaryType::Empty; }
        let text = raw.trim();
        static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^https?://[^\s]+$").expect("valid URL expression"));
        static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").expect("valid email expression"));
        if URL.is_match(text) { return PrimaryType::Url; }
        if EMAIL.is_match(text) { return PrimaryType::Email; }
        if is_phone(text) { return PrimaryType::Phone; }
        if (text.contains('{') && text.contains('}')) || ["function", "class ", "import ", "export "].iter().any(|v| text.contains(v)) { return PrimaryType::Code; }
        PrimaryType::Text
    }
    /// Text restoration intentionally follows existing product rules, not preview parsing.
    pub fn plain_text(&self) -> Result<Arc<str>, crate::ValidationError> {
        if let Some(text) = self.original_plain_text() { return Ok(text.clone()); }
        if let Some(html) = self.html.as_deref() {
            static SCRIPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("valid expression"));
            static STYLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("valid expression"));
            static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").expect("valid expression"));
            let scriptless = SCRIPT.replace_all(html, "");
            let styleless = STYLE.replace_all(&scriptless, "");
            let tagless = TAG.replace_all(&styleless, "");
            let text = html_plain_text(&tagless);
            if !text.is_empty() { return Ok(text.into()); }
        }
        if let Some(rtf) = self.rtf.as_deref() {
            static CONTROL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\[a-z]+[0-9]*\s?").expect("valid expression"));
            let mut text = CollapsedText::default();
            let mut slashes = 0_usize;
            for character in CONTROL.split(rtf).flat_map(str::chars).filter(|character| !matches!(character, '{' | '}')) {
                if character == '\\' { slashes += 1; continue; }
                // Brace/control removal precedes escape replacement, including
                // slash pairs joined across removed fragments.
                let retained = slashes.div_ceil(2);
                let escaped_quote = character == '\'' && retained > 0;
                for _ in 0..retained - usize::from(escaped_quote) { text.push('\\'); }
                text.push(character);
                slashes = 0;
            }
            for _ in 0..slashes.div_ceil(2) { text.push('\\'); }
            let text = text.value;
            if !text.is_empty() { return Ok(text.into()); }
        }
        if !self.files.is_empty() { return Ok(self.files.join("\n").into()); }
        Err(crate::ValidationError("该记录没有可恢复的纯文本".into()))
    }

    /// Whitespace-only raw text falls through to the other representations.
    /// A usable value retains its exact original bytes, including edge spaces.
    pub fn original_plain_text(&self) -> Option<&Arc<str>> {
        self.text.as_ref().filter(|text| !text.trim().is_empty())
    }
    pub fn byte_len(&self) -> u64 {
        [&self.text, &self.html, &self.rtf].iter().filter_map(|v| v.as_deref()).map(|v| v.len() as u64).sum::<u64>()
            + self.files.iter().map(|v| v.len() as u64).sum::<u64>()
    }
}

fn is_phone(text: &str) -> bool {
    // Match the product's ASCII 1[3-9][0-9]{9} rule after ignoring Unicode
    // whitespace, without allocating a copy of an unrelated clipboard payload.
    let mut characters = text.chars().filter(|character| !character.is_whitespace());
    if characters.next() != Some('1') || !matches!(characters.next(), Some('3'..='9')) { return false; }
    for _ in 0..9 {
        if !characters.next().is_some_and(|character| character.is_ascii_digit()) { return false; }
    }
    characters.next().is_none()
}

#[derive(Default)]
struct CollapsedText { value: String, whitespace: bool }
impl CollapsedText {
    fn push(&mut self, character: char) {
        if character.is_whitespace() { self.whitespace = true; return; }
        if self.whitespace && !self.value.is_empty() { self.value.push(' '); }
        self.whitespace = false;
        self.value.push(character);
    }
    fn extend(&mut self, text: &str) { for character in text.chars() { self.push(character); } }
}
fn html_plain_text(text: &str) -> String {
    // Preserve the ordered legacy replacements. amp runs after lt/gt/nbsp,
    // but before quot/#39, so only the latter are decoded after amp expansion.
    static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(?:nbsp|lt|gt|amp;(?:quot|#39)|amp|quot|#39);").expect("valid entity expression"));
    let mut result = CollapsedText::default();
    let mut offset = 0;
    for entity in ENTITY.find_iter(text) {
        result.extend(&text[offset..entity.start()]);
        result.push(match entity.as_str() {
            "&nbsp;" => ' ', "&lt;" => '<', "&gt;" => '>', "&amp;" => '&',
            "&quot;" | "&amp;quot;" => '"', "&#39;" | "&amp;#39;" => '\'',
            _ => unreachable!("entity expression contains only the supported replacements"),
        });
        offset = entity.end();
    }
    result.extend(&text[offset..]);
    result.value
}

pub fn is_color(text: &str) -> bool {
    static COLOR: LazyLock<Regex> = LazyLock::new(|| {
        let pattern: String = serde_json::from_str(include_str!("../resources/color-pattern.json")).expect("bundled color expression");
        Regex::new(&format!("(?i){pattern}")).expect("valid color expression")
    });
    COLOR.is_match(text.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_restoration_preserves_legacy_rules() {
        let original: Arc<str> = " \t原文\r\n".into();
        let content = TextContent { text: Some(original.clone()), html: Some("<b>other</b>".into()), ..TextContent::default() };
        assert!(Arc::ptr_eq(&content.plain_text().expect("original text"), &original));

        for (html, expected) in [
            ("<div> A&nbsp;B\u{2003}&lt;x&gt; &amp; &quot; &#39; </div>", "A B <x> & \" '"),
            ("<SCRIPT>hidden</SCRIPT><STYLE>hidden</STYLE><p>一</p><p>二</p>", "一二"),
            ("&amp;lt; &amp;gt; &amp;nbsp; &amp;quot; &amp;#39; &amp;amp;quot;", "&lt; &gt; &nbsp; \" ' &amp;quot;"),
            ("&am<b>p;</b>quot; &#x41; &QUOT;\u{2003}tail", "\" &#x41; &QUOT; tail"),
            ("&l<script>ignored</script>t;b&gt;", "<b>"),
            ("<sty<script>x</script>le>hidden</style>tail", "tail"),
        ] {
            let content = TextContent { html: Some(html.into()), ..TextContent::default() };
            assert_eq!(content.plain_text().expect("HTML extraction").as_ref(), expected, "{html}");
        }
        for (rtf, expected) in [
            (r"{\rtf1\ansi hello\par world}", "helloworld"),
            (r"{{\\}}'", "'"),
            (r"{{\\\\}}'", r"\'"),
            (r"{{\\}}", "\\"),
            (r"a\{word", r"a\word"),
            (r"a\\word b", r"a\b"),
            ("\u{2003}{  中\t文  }\r\n", "中 文"),
        ] {
            let content = TextContent { rtf: Some(rtf.into()), ..TextContent::default() };
            assert_eq!(content.plain_text().expect("RTF extraction").as_ref(), expected, "{rtf}");
        }
        let mut fallback = TextContent {
            text: Some("\u{2003}".into()), html: Some("<script>hidden</script>".into()), rtf: Some(r"{\rtf1 fallback}".into()),
            files: vec!["/资料/a b.txt".into(), "/资料/a b.txt".into()].into(),
        };
        assert_eq!(fallback.plain_text().expect("RTF fallback").as_ref(), "fallback");
        fallback.rtf = Some(r"{\rtf1 }".into());
        assert_eq!(fallback.plain_text().expect("file fallback").as_ref(), "/资料/a b.txt\n/资料/a b.txt");
        assert_eq!(TextContent::default().plain_text().expect_err("empty content").0, "该记录没有可恢复的纯文本");
    }
}

