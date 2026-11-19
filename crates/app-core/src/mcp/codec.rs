//! JSON and Base64 exist only here and in the MCP transport, after typed service calls.
use std::{borrow::Cow, sync::Arc};
use base64::{Engine as _, engine::general_purpose::{STANDARD, STANDARD_NO_PAD}};
use image::ImageFormat;
use rmcp::model::ContentBlock;
use serde::Serialize;
use serde_json::{Value, json};
use super_paste_format::{Formats, TextContent, MCP_IMAGE_PIXEL_LIMIT};
use tokio_util::sync::CancellationToken;
use crate::images::{EncodedImage, ImageLimits, ImageService, ImageSource, PixelBuffer};
use super::types::{MCP_MAX_IMAGE_BYTES, McpChange, McpClipboardData};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageInfo {
    bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")] mime_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")] width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")] height: Option<u32>,
}
#[derive(Serialize)]
struct Content<'a> {
    #[serde(skip_serializing_if = "Option::is_none")] text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")] html: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")] rtf: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")] files: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")] ocr: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")] image: Option<ImageInfo>,
    #[serde(skip_serializing_if = "Option::is_none")] cursor: Option<&'a str>,
}
impl<'a> Content<'a> {
    fn new(value: &'a TextContent, ocr: Option<&'a str>, image: Option<ImageInfo>, cursor: Option<&'a str>) -> Self {
        Self { text: value.text.as_deref(), html: value.html.as_deref(), rtf: value.rtf.as_deref(),
            files: (!value.files.is_empty()).then_some(value.files.as_ref()), ocr, image, cursor }
    }
}
pub(super) struct ValidatedImage { pub original: Arc<EncodedImage>, pub pixels: Arc<PixelBuffer> }
pub(super) async fn validate_image(images: &Arc<ImageService>, source: ImageSource, cancel: &CancellationToken) -> Result<ValidatedImage, String> {
    let original = images.acquire_encoded(source.clone(), cancel).await.map_err(error)?;
    if original.bytes().is_empty() || original.bytes().len() > MCP_MAX_IMAGE_BYTES { return Err("图片须为 1 字节至 10 MiB".into()); }
    if !matches!(original.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) { return Err("图片格式无效或不受支持".into()); }
    let (width, height) = original.dimensions().ok_or("图片格式无效或不受支持")?;
    if u64::from(width) * u64::from(height) > MCP_IMAGE_PIXEL_LIMIT { return Err("图片超过 4000 万像素，请缩小后重试".into()); }
    let pixels = images.acquire_pixels(source, ImageLimits { max_encoded_bytes: MCP_MAX_IMAGE_BYTES as u64, max_pixels: MCP_IMAGE_PIXEL_LIMIT, max_dimension: u32::MAX }, cancel).await.map_err(error)?;
    Ok(ValidatedImage { original, pixels })
}
pub(super) async fn clipboard(value: McpClipboardData, include_image: bool, cursor: Option<&str>, images: &Arc<ImageService>, cancel: &CancellationToken) -> Result<(Value, Vec<ContentBlock>), String> {
    if cancel.is_cancelled() { return Err("请求已取消".into()); }
    let mut blocks = Vec::new();
    let image = match value.image {
        Some(source) if include_image => {
            let validated = validate_image(images, source, cancel).await?;
            let png = images.encode_png(validated.pixels.clone(), cancel).await.map_err(error)?;
            if png.bytes().len() > MCP_MAX_IMAGE_BYTES { return Err("PNG 图片超过 10 MiB，请关闭 includeImage".into()); }
            let bytes = png.bytes().len() as u64;
            let encoded_bytes = bytes.div_ceil(3) * 4;
            let encoded = images.run_cpu(encoded_bytes, cancel, move || Ok(STANDARD.encode(png.bytes()))).await.map_err(error)?;
            blocks.push(ContentBlock::image(encoded, "image/png"));
            Some(ImageInfo { bytes, mime_type: Some("image/png"), width: Some(validated.pixels.width()), height: Some(validated.pixels.height()) })
        },
        Some(source) => {
            let bytes = match value.image_bytes { Some(bytes) => bytes, None => images.acquire_encoded(source, cancel).await.map_err(error)?.bytes().len() as u64 };
            Some(ImageInfo { bytes, mime_type: None, width: None, height: None })
        },
        None => None,
    };
    if cancel.is_cancelled() { return Err("请求已取消".into()); }
    let content = serde_json::to_value(Content::new(&value.content, value.ocr.as_deref(), image, cursor)).map_err(json_error)?;
    if cancel.is_cancelled() { return Err("请求已取消".into()); }
    Ok((content, blocks))
}
pub(super) fn history(item: super_paste_format::HistoryEntry, content: Value) -> Result<Value, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Tag<'a> { id: super_paste_format::TagId, name: &'a str, color: &'a str, description: Option<&'a str>, created_at: i64, updated_at: i64 }
    let formats = [(Formats::TEXT, "text"), (Formats::HTML, "html"), (Formats::IMAGE, "image"), (Formats::RTF, "rtf"), (Formats::FILES, "files")]
        .into_iter().filter_map(|(format, name)| item.formats.contains(format).then_some(name)).collect::<Vec<_>>();
    let tags = item.tags.iter().map(|tag| Tag { id: tag.id, name: &tag.name, color: &tag.color, description: tag.description.as_deref(), created_at: tag.created_at, updated_at: tag.updated_at }).collect::<Vec<_>>();
    let mut fields = json!({ "id": item.id, "primaryType": item.primary_type, "availableFormats": formats, "timestamp": item.timestamp, "lastUsed": item.last_used, "tags": tags });
    let fields = fields.as_object_mut().ok_or("历史记录序列化格式错误")?;
    if let Some(source) = item.source { fields.insert("source_app".into(), Value::String(source.application.to_string())); }
    fields.insert("content".into(), content);
    Ok(Value::Object(std::mem::take(fields)))
}
pub(super) fn change(value: &McpChange) -> Result<Value, String> {
    #[derive(Serialize)] struct Event<'a> { cursor: &'a str, timestamp: f64, content: Content<'a> }
    let image = value.change.image_bytes.map(|bytes| ImageInfo { bytes, mime_type: None, width: None, height: None });
    serde_json::to_value(Event { cursor: &value.cursor, timestamp: value.change.timestamp as f64,
        content: Content::new(&value.change.content, None, image, None) }).map_err(json_error)
}
pub(super) async fn import_image_base64(images: &Arc<ImageService>, input: String, cancel: &CancellationToken) -> Result<Arc<EncodedImage>, String> {
    // Keep the owned request string, optional whitespace normalization and
    // decoder output admitted while queued and until their physical work ends.
    // The output lease transfers to image ingress instead of charging it twice.
    let scratch_bytes = input.capacity() as u64 + input.len() as u64 + (input.len() as u64).div_ceil(4) * 3;
    let mut scratch = images.reserve_consumer_copy(scratch_bytes).map_err(error)?;
    let (bytes, scratch) = images.run_cpu(0, cancel, move || {
        let bytes = decode_image_base64(&input).map_err(|message| crate::Error::Invalid(message.into()))?;
        drop(input);
        scratch.shrink(bytes.capacity() as u64);
        Ok((bytes, scratch))
    }).await.map_err(error)?;
    images.import_reserved_encoded(bytes, scratch, cancel).await.map_err(error)
}

fn decode_image_base64(input: &str) -> Result<Vec<u8>, String> {
    let input = if let Some(uri) = input.strip_prefix("data:image/") {
        let (mime, data) = uri.split_once(";base64,").ok_or("图片 Base64 无效")?;
        if mime.is_empty() || !mime.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte)) { return Err("图片 Base64 无效".into()); }
        data
    } else { input };
    let encoded = if input.chars().any(char::is_whitespace) {
        let mut normalized = String::with_capacity(input.len());
        normalized.extend(input.chars().filter(|character| !character.is_whitespace()));
        Cow::Owned(normalized)
    } else { Cow::Borrowed(input) };
    if encoded.is_empty() || !encoded.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"+/=".contains(&byte)) { return Err("图片 Base64 无效".into()); }
    let unpadded = encoded.trim_end_matches('=');
    if unpadded.is_empty() || encoded.len() - unpadded.len() > 2 { return Err("图片 Base64 无效".into()); }
    // The strict decoder checks unused trailing bits; re-encoding the whole image
    // to compare canonical text would allocate a second Base64 buffer needlessly.
    let bytes = STANDARD_NO_PAD.decode(unpadded).map_err(|_| "图片 Base64 无效")?;
    if bytes.is_empty() || bytes.len() > MCP_MAX_IMAGE_BYTES { return Err("图片须为 1 字节至 10 MiB".into()); }
    Ok(bytes)
}
fn error(error: crate::Error) -> String { error.to_string() }
fn json_error(error: serde_json::Error) -> String { format!("MCP 响应序列化失败：{error}") }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{images::ImageBudgets, storage::Storage};
    use image::ImageEncoder;

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("super-paste-rs-mcp-images-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).expect("isolated protocol image directory");
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) { std::fs::remove_dir_all(&self.0).expect("remove protocol image directory"); }
    }

    #[tokio::test]
    async fn base64_images_preserve_validation_shared_identity_and_png_protocol() {
        let directory = Directory::new();
        let storage = Storage::open(&directory.0.join("protocol.sqlite3")).await.expect("database");
        let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets::default()).expect("images");
        let cancel = CancellationToken::new();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[31, 127, 239, 63], 1, 1, image::ExtendedColorType::Rgba8).expect("PNG");
        let encoded = STANDARD.encode(&png);
        let original = import_image_base64(&images, encoded.clone(), &cancel).await.expect("Base64 image");
        assert_eq!(original.bytes(), png);
        for alias in [encoded.trim_end_matches('=').to_owned(), format!("data:image/png;base64,\n{}\u{2003}", encoded)] {
            let alias = import_image_base64(&images, alias, &cancel).await.expect("accepted alias");
            assert!(Arc::ptr_eq(&original, &alias));
            assert_eq!(images.memory().scratch, 0);
        }
        for malformed in ["", " ", "AB==", "A===", "AA===", "AA=A", "-w==", "data:image/png,AQ=="] {
            assert_eq!(import_image_base64(&images, malformed.into(), &cancel).await.err().as_deref(), Some("图片 Base64 无效"));
            assert_eq!(images.memory().scratch, 0);
        }
        let value = McpClipboardData { content: TextContent { text: Some("原文\0😀".into()), ..Default::default() }, ocr: None,
            image: Some(ImageSource::Encoded(original)), image_bytes: None };
        let (content, blocks) = clipboard(value, true, Some("cursor"), &images, &cancel).await.expect("image response");
        assert_eq!(content["text"], "原文\0😀");
        assert_eq!(content["cursor"], "cursor");
        assert_eq!(content["image"]["mimeType"], "image/png");
        assert_eq!(content["image"]["width"], 1);
        assert_eq!(content["image"]["height"], 1);
        let blocks = serde_json::to_value(blocks).expect("wire image blocks");
        assert_eq!(blocks.as_array().expect("blocks").len(), 1);
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!(blocks[0]["mimeType"], "image/png");
        let bytes = STANDARD.decode(blocks[0]["data"].as_str().expect("Base64 PNG")).expect("valid Base64");
        assert_eq!(content["image"]["bytes"].as_u64(), Some(bytes.len() as u64));
        assert_eq!(image::load_from_memory(&bytes).expect("PNG pixels").to_rgba8().as_raw(), &[31, 127, 239, 63]);
        assert_eq!(images.memory().scratch, 0);
        images.shutdown().await.expect("shutdown");
        storage.close_for_test().await.expect("close database");
    }

    #[tokio::test]
    async fn cancelled_base64_admission_releases_the_owned_request_and_lease() {
        let directory = Directory::new();
        let storage = Storage::open(&directory.0.join("cancel.sqlite3")).await.expect("database");
        let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets { workers: 1, ..Default::default() }).expect("images");
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, finished) = std::sync::mpsc::channel();
        let worker = images.clone();
        let held = tokio::spawn(async move { worker.run_cpu(0, &CancellationToken::new(), move || {
            entered.send(()).expect("signal occupied CPU worker");
            finished.recv().expect("release CPU worker");
            Ok(())
        }).await });
        started.await.expect("worker started");
        for abandon in [false, true] {
            let cancel = CancellationToken::new();
            let mut input = String::with_capacity(4096);
            input.push_str("AQ==");
            let capacity = input.capacity() as u64;
            let mut request = Box::pin(import_image_base64(&images, input, &cancel));
            assert!(futures_util::poll!(request.as_mut()).is_pending());
            assert!(images.memory().scratch >= capacity);
            if abandon { drop(request); }
            else {
                cancel.cancel();
                assert!(request.await.is_err());
            }
            assert_eq!(images.memory().scratch, 0);
            assert_eq!(images.memory().encoded, 0, "queued Base64 must not be imported");
        }
        release.send(()).expect("release worker");
        held.await.expect("worker task").expect("worker result");
        images.shutdown().await.expect("shutdown");
        storage.close_for_test().await.expect("close database");
    }
}
