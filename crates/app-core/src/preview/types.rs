use std::sync::Arc;
use super_paste_format::{ImageMetadata, PrimaryType, TextContent};
use crate::resources::{Reservation, Weighted};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewScope { Card, Details }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind { Text, Code, Json, Table, Html, Rtf, Image, Files, Url, Email, Phone, Color, Empty }
impl PreviewKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "文本", Self::Code => "代码", Self::Json => "JSON", Self::Table => "表格",
            Self::Html | Self::Rtf => "富文本", Self::Image => "图片", Self::Files => "文件",
            Self::Url => "链接", Self::Email => "邮箱", Self::Phone => "电话", Self::Color => "颜色", Self::Empty => "其他",
        }
    }
}
#[derive(Clone, Debug)]
pub struct FilePreview { pub path: Arc<str>, pub name: Arc<str>, pub directory: Arc<str>, pub extension: Arc<str> }
#[derive(Clone, Debug)]
pub struct LinkPreview { pub href: Arc<str>, pub host: Arc<str>, pub detail: Arc<str> }
#[derive(Clone, Debug)]
pub struct ColorPreview {
    pub original: Arc<str>, pub hex: Arc<str>, pub rgb: Arc<str>,
    /// Unquantized CSS channels for compositing. HEX/RGB serialization has its
    /// own byte rounding and must not lower the swatch's display precision.
    pub rgba: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct TablePreview { pub rows: Arc<[Arc<[Arc<str>]>]>, pub columns: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewNotice { Excerpt, RichTooLarge, RichUnavailable }
impl PreviewNotice {
    pub fn text(self) -> &'static str {
        match self { Self::Excerpt => "预览已截取", Self::RichTooLarge => "富文本较大，已省略排版预览", Self::RichUnavailable => "富文本预览不可用" }
    }
}
#[derive(Clone, Debug)]
pub enum PreviewBody { Text, Code, Image, Files, Color(ColorPreview), Rich(Arc<super::rich::Document>), Link(LinkPreview), Contact, Table(TablePreview), Empty }

pub struct PreviewModel {
    pub kind: PreviewKind, pub body: PreviewBody, pub scope: PreviewScope,
    pub text: Arc<str>, pub display_text: Arc<str>, pub summary: Arc<str>, pub notice: Option<PreviewNotice>,
    pub image: Option<ImageMetadata>, pub files: Arc<[FilePreview]>, pub files_total: u64,
    pub(crate) lease: Reservation,
}
impl Weighted for PreviewModel {
    fn weight(&self) -> u64 { self.lease.bytes() + match &self.body { PreviewBody::Rich(document) => document.weight(), _ => 0 } }
}

/// Non-omitted rich sources fit the shared UTF-16 source limit. Storage checks
/// borrowed rows before allocating; hot excerpts are already smaller than it.
pub(crate) struct Input {
    pub content: TextContent, pub primary_type: PrimaryType, pub image: Option<ImageMetadata>,
    pub text_characters: u64, pub text_clipped: bool,
    pub html_omitted: bool, pub rtf_omitted: bool, pub scope: PreviewScope, pub lease: Reservation,
}
