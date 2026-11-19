use std::{fmt, ops::{Deref, Range}};
use crate::SharedString;

/// UTF-8 text owned by a shaped paragraph or a view into that same owner.
/// Slicing changes offsets, never copies the paragraph or builds owner chains.
#[derive(Clone, Default)]
pub struct ShapedText {
    owner: SharedString,
    range: Range<usize>,
}

impl ShapedText {
    /// Borrow a source interval. Endpoints must be UTF-8 character boundaries.
    pub fn slice(&self, range: Range<usize>) -> Self {
        self.get(range.clone()).expect("shaped text slice must be valid UTF-8");
        if range.is_empty() { return Self::default(); }
        Self { owner: self.owner.clone(), range: self.range.start + range.start..self.range.start + range.end }
    }

    /// Materialize only when an API actually requires a standalone string.
    pub fn to_shared_string(&self) -> SharedString {
        if self.range == (0..self.owner.len()) { self.owner.clone() }
        else { SharedString::new(self.as_ref()) }
    }
}
impl From<SharedString> for ShapedText {
    fn from(owner: SharedString) -> Self { let end = owner.len(); Self { owner, range: 0..end } }
}
impl Deref for ShapedText {
    type Target = str;
    fn deref(&self) -> &str { &self.owner[self.range.clone()] }
}
impl AsRef<str> for ShapedText { fn as_ref(&self) -> &str { self } }
impl From<&str> for ShapedText { fn from(text: &str) -> Self { SharedString::new(text).into() } }
impl From<String> for ShapedText { fn from(text: String) -> Self { SharedString::from(text).into() } }
impl PartialEq for ShapedText { fn eq(&self, other: &Self) -> bool { self.as_ref() == other.as_ref() } }
impl Eq for ShapedText {}
impl PartialEq<SharedString> for ShapedText { fn eq(&self, other: &SharedString) -> bool { self.as_ref() == other.as_ref() } }
impl PartialEq<ShapedText> for SharedString { fn eq(&self, other: &ShapedText) -> bool { self.as_ref() == other.as_ref() } }
impl PartialEq<&str> for ShapedText { fn eq(&self, other: &&str) -> bool { self.as_ref() == *other } }
impl std::hash::Hash for ShapedText { fn hash<H: std::hash::Hasher>(&self, state: &mut H) { std::hash::Hash::hash(self.as_ref(), state); } }
impl fmt::Debug for ShapedText { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Debug::fmt(self.as_ref(), f) } }
impl fmt::Display for ShapedText { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self) } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_unicode_views_retain_one_owner_and_relative_offsets() {
        let source = ShapedText::from("prefix 这是共享文本😀 and a sufficiently long suffix");
        let start = source.find('这').unwrap();
        let end = source.find(" and").unwrap();
        let view = source.slice(start..end);
        assert_eq!(view, "这是共享文本😀");
        assert_eq!(view.as_ptr(), source[start..].as_ptr());
        let nested = view.slice(3..view.len());
        assert_eq!(nested, "是共享文本😀");
        assert_eq!(nested.as_ptr(), source[start + 3..].as_ptr());
        drop(source); drop(view);
        assert_eq!(nested, "是共享文本😀");
        assert_eq!(nested.to_shared_string().as_ref(), nested.as_ref());
        assert_eq!(nested.slice(0..0), "");
    }
    #[test]
    #[should_panic(expected = "valid UTF-8")]
    fn rejects_a_split_inside_a_codepoint() { let _ = ShapedText::from("😀").slice(0..1); }
}
