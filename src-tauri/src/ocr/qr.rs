use image::{ImageFormat, ImageReader, Limits, Luma};
use qrcode::{Color, EcLevel, QrCode};
use serde::Serialize;
use std::io::Cursor;

const QR_IMAGE_SIZE: u32 = 256;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QRCodeData {
    pub png_bytes: Vec<u8>,
    pub data_url: String,
}

pub(super) fn generate(text: &str) -> Result<QRCodeData, String> {
    if text.trim().is_empty() {
        return Err("文本内容不能为空".to_owned());
    }
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|error| format!("生成二维码失败: {error}"))?;
    let modules = code.width() as u32;
    let canvas_width = modules.saturating_add(4);
    let mut canvas = image::GrayImage::from_pixel(canvas_width, canvas_width, Luma([255]));
    for y in 0..modules {
        for x in 0..modules {
            if code[(x as usize, y as usize)] == Color::Dark {
                canvas.put_pixel(x + 2, y + 2, Luma([0]));
            }
        }
    }
    let bitmap = image::imageops::resize(
        &canvas,
        QR_IMAGE_SIZE,
        QR_IMAGE_SIZE,
        image::imageops::FilterType::Nearest,
    );
    let mut cursor = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(bitmap)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|error| format!("生成二维码失败: {error}"))?;
    let png_bytes = cursor.into_inner();
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
    Ok(QRCodeData {
        png_bytes,
        data_url: format!("data:image/png;base64,{encoded}"),
    })
}

pub(super) fn parse(image: &[u8]) -> Option<String> {
    let gray = decode_gray_image(image).ok()?;
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    prepared
        .detect_grids()
        .into_iter()
        .find_map(|grid| grid.decode().ok().map(|(_, text)| text))
}

fn decode_gray_image(encoded: &[u8]) -> Result<image::GrayImage, String> {
    if encoded.is_empty() || encoded.len() > MAX_IMAGE_BYTES {
        return Err("二维码图片大小无效".to_owned());
    }
    let mut reader = ImageReader::new(Cursor::new(encoded))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(65_536);
    limits.max_image_height = Some(65_536);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| error.to_string())?;
    if u64::from(image.width()) * u64::from(image.height()) > MAX_IMAGE_PIXELS {
        return Err("二维码图片像素超过上限".to_owned());
    }
    Ok(image.to_luma8())
}
