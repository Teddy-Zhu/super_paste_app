use std::{ops::Range, sync::Arc};
use crate::{Error, Result, resources::Reservation};
use super::text_whitespace;

/// Validated original text, held with its input reservation until OS handoff.
/// A trimmed range avoids copying a long URL into another presentation model.
pub struct ExternalLink { text: Arc<str>, range: Range<usize>, _lease: Reservation }
impl ExternalLink {
    pub fn href(&self) -> &str { &self.text[self.range.clone()] }
    pub(super) fn new(text: Arc<str>, mut lease: Reservation) -> Result<Self> {
        let href = text.trim_matches(text_whitespace);
        // Native URL APIs may treat NUL as a terminator. Reject it rather
        // than handing off an address different from the validated original.
        if href.contains('\0') { return Err(Error::Invalid("链接内容无效".into())); }
        let parsed = parse(href).ok_or_else(|| Error::Invalid("链接内容无效".into()))?;
        if !parsed.username().is_empty() || parsed.password().is_some() { return Err(Error::Invalid("仅允许打开不含账号密码的 HTTP 或 HTTPS 网页".into())); }
        let start = text.len() - text.trim_start_matches(text_whitespace).len();
        let range = start..start + href.len();
        drop(parsed);
        lease.shrink(text.len() as u64 + 4096);
        Ok(Self { text, range, _lease: lease })
    }
}
pub(super) fn parse(text: &str) -> Option<url::Url> {
    if !(text.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://")) || text.get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))) || text.chars().any(text_whitespace) { return None; }
    let parsed = url::Url::parse(text).ok()?;
    parsed.host_str()?;
    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::ByteBudget;

    #[test]
    fn external_link_validation_preserves_original_href_and_rejects_unsafe_targets() {
        let memory = ByteBudget::new(1024 * 1024, "external link");
        let text: Arc<str> = "\u{feff}\tHTTPS://例子.测试:8443/路径?q=%2F%25#片段\r\n".into();
        let lease = memory.reserve(text.len() as u64 * 4 + 4096).expect("parsing reservation");
        let link = ExternalLink::new(text.clone(), lease).expect("valid original URL");
        assert_eq!(link.href(), "HTTPS://例子.测试:8443/路径?q=%2F%25#片段");
        assert!(Arc::ptr_eq(&text, &link.text));
        assert_eq!(memory.used(), text.len() as u64 + 4096);
        drop(link); assert_eq!(memory.used(), 0);
        for value in ["https://user:password@example.com/", "https://user@example.com/", "https://:password@example.com/", "file:///tmp/item", "javascript:alert(1)", "http:example.com", "https://", "https://example.com/a\nb", "https://example.com/a b", "https://example.com/a\0b", ""] {
            let lease = memory.reserve(value.len() as u64 * 4 + 4096).expect("parsing reservation");
            assert!(matches!(ExternalLink::new(value.into(), lease), Err(Error::Invalid(_))), "accepted {value:?}");
            assert_eq!(memory.used(), 0);
        }
    }
}
