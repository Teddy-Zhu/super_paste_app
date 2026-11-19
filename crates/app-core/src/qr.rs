use std::sync::Arc;
use image::GrayImage;
use qrcode::{Color, EcLevel, QrCode};
use tokio_util::sync::CancellationToken;
use crate::{Error, Result, images::{AlphaMode, ImageLimits, ImageService, ImageSource, PixelBuffer, PixelLayout, PixelSpec, ResizeFilter}};

pub(crate) const MAX_TEXT_BYTES: usize = 7089;
const IMAGE_SIZE: usize = 256;
const QUIET_ZONE: usize = 2;
// Admission allowance, not a preallocation. The bounded source permits at
// most one encoder segment per byte. qrcode 0.14.1 holds two segment vectors
// during optimization and up to three version-40 canvases during masking;
// its codeword buffers and the fixed output also fit within this allowance.
const GENERATION_WORK_BYTES: u64 = 1024 * 1024;

pub struct QrService { images: Arc<ImageService> }
impl QrService {
    pub fn new(images: Arc<ImageService>) -> Self { Self { images } }

    /// Keep generated QR as pixels; encode only at a consumer boundary.
    pub async fn generate(&self, text: impl AsRef<str> + Send + 'static, cancel: &CancellationToken) -> Result<Arc<PixelBuffer>> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        if text.as_ref().len() > MAX_TEXT_BYTES { return Err(text_limit()); }
        let data = self.images.run_cpu(GENERATION_WORK_BYTES, cancel, move || render(text.as_ref())).await?;
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        self.images.import_pixels(IMAGE_SIZE as u32, IMAGE_SIZE as u32, IMAGE_SIZE, PixelLayout::Gray8, AlphaMode::Opaque, data, cancel).await
    }

    pub async fn parse(&self, source: ImageSource, cancel: &CancellationToken) -> Result<Option<String>> {
        let pixels = self.images.acquire_pixels(source, ImageLimits::default(), cancel).await?;
        let gray = self.images.derive_pixels(pixels.clone(), PixelSpec { max_width: pixels.width(), max_height: pixels.height(), layout: PixelLayout::Gray8, filter: ResizeFilter::Nearest }, cancel).await?;
        let pixel_bytes = u64::from(gray.width()) * u64::from(gray.height());
        // rqrr binarization also needs a usize per column. An area-only
        // allowance undercounts this phase for shallow, wide inputs.
        let threshold_bytes = u64::from(gray.width()) * std::mem::size_of::<usize>() as u64;
        let scratch = (pixel_bytes * 4).max(pixel_bytes + threshold_bytes);
        self.images.run_cpu(scratch, cancel, move || {
            // rqrr owns and mutates this input; retain the shared gray buffer for other consumers.
            let image = GrayImage::from_raw(gray.width(), gray.height(), gray.bytes().to_vec()).ok_or_else(|| Error::Image("二维码像素输入无效".into()))?;
            let mut prepared = rqrr::PreparedImage::prepare(image);
            Ok(prepared.detect_grids().into_iter().find_map(|grid| grid.decode().ok().map(|(_, text)| text)))
        }).await
    }
}

/// Called on the SQLite worker with borrowed UTF-8: only the bounded extracted
/// text leaves the row, never a second owned copy of the full HTML payload.
/// Preserve the product's literal stripping (including encoded entities).
pub(crate) fn text_from_html(html: &str, cancel: &CancellationToken) -> Result<Arc<str>> {
    if cancel.is_cancelled() { return Err(Error::Cancelled); }
    let mut text = String::with_capacity(html.len().min(MAX_TEXT_BYTES)); let mut in_tag = false;
    let mut next_check = 4096;
    for (offset, character) in html.char_indices() {
        // Long markup can yield almost no text. Do not monopolize the SQLite
        // worker after the consumer leaves, even while scanning inside a tag.
        if offset >= next_check {
            if cancel.is_cancelled() { return Err(Error::Cancelled); }
            next_check = offset.saturating_add(4096);
        }
        match character {
            '<' if !in_tag => in_tag = true, '>' if in_tag => in_tag = false,
            _ if !in_tag => {
                if text.len() + character.len_utf8() > MAX_TEXT_BYTES { return Err(text_limit()); }
                text.push(character);
            },
            _ => {},
        }
    }
    if cancel.is_cancelled() { return Err(Error::Cancelled); }
    Ok(text.into())
}

fn text_limit() -> Error { Error::Invalid("文本过长，无法生成二维码".into()) }

fn render(text: &str) -> Result<Vec<u8>> {
    if text.trim().is_empty() { return Err(Error::Invalid("文本内容为空，无法生成二维码".into())); }
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|error| Error::Invalid(format!("生成二维码失败: {error}").into()))?;
    let modules = code.width();
    let canvas_width = modules + 2 * QUIET_ZONE;
    // Preserve image 0.25.10's nearest-neighbor pixel-center sampling. For a
    // 256px output and standard QR versions (at most 177 modules), its f32
    // coordinates are exact binary fractions, so this integer floor selects
    // the same source pixel without a gray canvas or floating RGBA scratch.
    let samples: [usize; IMAGE_SIZE] = std::array::from_fn(|index|
        ((2 * index + 1) * canvas_width) / (2 * IMAGE_SIZE));
    let content = QUIET_ZONE..QUIET_ZONE + modules;
    let mut pixels = vec![255; IMAGE_SIZE * IMAGE_SIZE];
    for (row, &source_y) in pixels.chunks_exact_mut(IMAGE_SIZE).zip(&samples) {
        if !content.contains(&source_y) { continue; }
        for (pixel, &source_x) in row.iter_mut().zip(&samples) {
            if content.contains(&source_x)
                && code[(source_x - QUIET_ZONE, source_y - QUIET_ZONE)] == Color::Dark {
                *pixel = 0;
            }
        }
    }
    Ok(pixels)
}
