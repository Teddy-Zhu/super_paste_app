//! Borrow the shared JPEG source through one decoder, including its metadata.
//! Ordinary output follows image 0.25.10's JPEG adapter. CMYK/YCCK additionally
//! expose retained source samples for ICC, without another JPEG decode.

use image::{ColorType, ImageDecoder, ImageError, ImageFormat, ImageResult, Limits,
    error::{DecodingError, LimitError, LimitErrorKind, UnsupportedError, UnsupportedErrorKind},
    metadata::Orientation};
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};

pub(super) struct Decoder<'a> {
    inner: zune_jpeg::JpegDecoder<ZCursor<&'a [u8]>>,
    dimensions: (u32, u32),
    color: ColorType,
}

impl<'a> Decoder<'a> {
    pub(super) fn new(source: &'a [u8], limits: &Limits) -> ImageResult<Self> {
        limits.check_support(&image::LimitSupport::default())?;
        let options = DecoderOptions::default().set_strict_mode(false)
            .set_max_width(limits.max_image_width.map_or(usize::MAX, |v| v as usize))
            .set_max_height(limits.max_image_height.map_or(usize::MAX, |v| v as usize));
        let mut inner = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(source), options);
        inner.decode_headers().map_err(decode_error)?;
        let (width, height) = inner.dimensions().ok_or_else(|| invalid("JPEG 头部缺少尺寸"))?;
        let dimensions = (
            u32::try_from(width).map_err(|_| dimension_error())?,
            u32::try_from(height).map_err(|_| dimension_error())?,
        );
        limits.check_dimensions(dimensions.0, dimensions.1)?;
        let (output, color) = match inner.input_colorspace().ok_or_else(|| invalid("JPEG 头部缺少色彩信息"))? {
            ColorSpace::Luma => (ColorSpace::Luma, ColorType::L8),
            ColorSpace::LumaA => (ColorSpace::LumaA, ColorType::La8),
            ColorSpace::RGBA => (ColorSpace::RGBA, ColorType::Rgba8),
            _ => (ColorSpace::RGB, ColorType::Rgb8),
        };
        inner.set_options(inner.options().jpeg_set_out_colorspace(output));
        Ok(Self { inner, dimensions, color })
    }

    pub(super) fn is_cmyk(&self) -> bool {
        matches!(self.inner.input_colorspace(), Some(ColorSpace::CMYK | ColorSpace::YCCK))
    }

    pub(super) fn icc_profile_chunks(&self) -> Option<impl Iterator<Item = &[u8]> + Clone + '_> {
        self.inner.icc_profile_chunks()
    }

    pub(super) fn read_cmyk(mut self, output: &mut [u8]) -> ImageResult<()> {
        let required = u64::from(self.dimensions.0) * u64::from(self.dimensions.1) * 4;
        if output.len() as u64 != required {
            return Err(invalid("JPEG CMYK 像素缓冲区尺寸不匹配"));
        }
        self.inner.decode_into_cmyk(output).map_err(decode_error)
    }
}

impl ImageDecoder for Decoder<'_> {
    fn dimensions(&self) -> (u32, u32) { self.dimensions }
    fn color_type(&self) -> ColorType { self.color }

    fn icc_profile(&mut self) -> ImageResult<Option<Vec<u8>>> { Ok(self.inner.icc_profile()) }
    fn exif_metadata(&mut self) -> ImageResult<Option<Vec<u8>>> { Ok(self.inner.exif().cloned()) }
    fn xmp_metadata(&mut self) -> ImageResult<Option<Vec<u8>>> { Ok(self.inner.xmp().cloned()) }
    fn iptc_metadata(&mut self) -> ImageResult<Option<Vec<u8>>> { Ok(self.inner.iptc().cloned()) }

    fn orientation(&mut self) -> ImageResult<Orientation> {
        // Read the decoder's existing metadata in place; the pixel consumer
        // does not need another owned EXIF vector or a second header pass.
        Ok(self.inner.exif().and_then(|exif| Orientation::from_exif_chunk(exif))
            .unwrap_or(Orientation::NoTransforms))
    }

    fn set_limits(&mut self, mut limits: Limits) -> ImageResult<()> {
        limits.check_support(&image::LimitSupport::default())?;
        limits.check_dimensions(self.dimensions.0, self.dimensions.1)?;
        // The caller has admitted both its retained output and any coexisting
        // conversion buffer before supplying this allocation allowance.
        limits.reserve(self.total_bytes())?;
        let options = self.inner.options()
            .set_max_width(limits.max_image_width.map_or(usize::MAX, |v| v as usize))
            .set_max_height(limits.max_image_height.map_or(usize::MAX, |v| v as usize));
        self.inner.set_options(options);
        Ok(())
    }

    fn read_image(mut self, output: &mut [u8]) -> ImageResult<()> {
        if output.len() as u64 != self.total_bytes() {
            return Err(invalid("JPEG 像素缓冲区尺寸不匹配"));
        }
        self.inner.decode_into(output).map_err(decode_error)
    }

    fn read_image_boxed(self: Box<Self>, output: &mut [u8]) -> ImageResult<()> {
        (*self).read_image(output)
    }
}

fn invalid(message: &'static str) -> ImageError {
    ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), message))
}

fn dimension_error() -> ImageError {
    ImageError::Limits(LimitError::from_kind(LimitErrorKind::DimensionError))
}

fn decode_error(error: zune_jpeg::errors::DecodeErrors) -> ImageError {
    match error {
        zune_jpeg::errors::DecodeErrors::LargeDimensions(_) => dimension_error(),
        zune_jpeg::errors::DecodeErrors::Unsupported(description) => {
            ImageError::Unsupported(UnsupportedError::from_format_and_kind(ImageFormat::Jpeg.into(),
                UnsupportedErrorKind::GenericFeature(format!("{description:?}"))))
        },
        error => ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), error)),
    }
}
