use std::{borrow::Cow, sync::Arc};

use bytes::Bytes;
use image::ImageDecoder;
use moxcms::{CicpColorPrimaries, CicpProfile, ColorProfile, DataColorSpace, Layout,
    MatrixCoefficients, ParsingOptions, RenderingIntent, TransferCharacteristics, TransformOptions};

use crate::{Error, Result, resources::{ByteBudget, Reservation}};

const MAX_PROFILE_BYTES: usize = 4 * 1024 * 1024;
// Admission allowance for bounded profile/LUT parsing, transform tables and
// row work; it does not allocate this amount or replace the library's limits.
const WORKING_BYTES: u64 = 64 * 1024 * 1024;

mod png;
mod hdr;

enum Description {
    Icc(ProfileBytes),
    PngCicp { codes: [u8; 4], hdr: Option<hdr::Input> },
    PngSdr(png::Sdr),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceChannels { Rgb, Gray, Cmyk }

impl SourceChannels {
    fn from_image(color: image::ColorType) -> Self {
        if matches!(color, image::ColorType::L8 | image::ColorType::La8 | image::ColorType::L16 | image::ColorType::La16) {
            Self::Gray
        } else { Self::Rgb }
    }
}

struct ProfileBytes { bytes: Bytes, reservation: Reservation }

impl ProfileBytes {
    fn read(bytes: image::ImageResult<Option<Cow<'_, [u8]>>>, budget: &ByteBudget) -> Result<Result<Option<Self>>> {
        match bytes {
            Ok(Some(bytes)) => Self::retain(bytes, budget).map(|profile| profile.map(Some)),
            Ok(None) => Ok(Ok(None)),
            Err(error) => Ok(Err(error.into())),
        }
    }

    fn retain(bytes: Cow<'_, [u8]>, budget: &ByteBudget) -> Result<Result<Self>> {
        if bytes.len() > MAX_PROFILE_BYTES { return Ok(Err(profile_limit())); }
        let allocation = match &bytes { Cow::Borrowed(bytes) => bytes.len(), Cow::Owned(bytes) => bytes.capacity() };
        let reservation = budget.reserve(allocation as u64)?;
        let bytes = match bytes { Cow::Borrowed(bytes) => Bytes::copy_from_slice(bytes), Cow::Owned(bytes) => Bytes::from(bytes) };
        Ok(Ok(Self { bytes, reservation }))
    }

    fn from_chunks<'a>(chunks: impl Iterator<Item = &'a [u8]> + Clone, budget: &ByteBudget) -> Result<Result<Self>> {
        let Some(length) = chunks.clone().try_fold(0_usize, |total, chunk| {
            total.checked_add(chunk.len()).filter(|length| *length <= MAX_PROFILE_BYTES)
        }) else { return Ok(Err(profile_limit())); };
        let mut reservation = budget.reserve(length as u64)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| Error::ResourceLimit("无法分配图片色彩配置".into()))?;
        // Account for the actual capacity if the allocator provides more.
        if bytes.capacity() > length { reservation.grow((bytes.capacity() - length) as u64)?; }
        for chunk in chunks { bytes.extend_from_slice(chunk); }
        Ok(Ok(Self { bytes: Bytes::from(bytes), reservation }))
    }
}

fn profile_limit() -> Error { Error::ResourceLimit("图片色彩配置超出显示限制".into()) }

pub(super) struct Input {
    description: Description,
    source: SourceChannels,
}

impl Input {
    pub fn weight(&self) -> u64 {
        match &self.description { Description::Icc(profile) => profile.reservation.bytes(),
            Description::PngCicp { .. } | Description::PngSdr(_) => 0 }
    }

    pub fn working_bytes(&self, width: u32) -> u64 {
        // Narrow-range RGB needs separate float input/output scanlines, so that
        // undershoot/overshoot survive until the final sRGB quantization.
        WORKING_BYTES + u64::from(width) * if self.hdr().is_some() { 48 } else if self.narrow_range() { 32 } else { 4 }
    }

    fn narrow_range(&self) -> bool { matches!(&self.description, Description::PngCicp { codes: [_, _, _, 0], .. }) }

    fn hdr(&self) -> Option<hdr::Input> {
        match &self.description { Description::PngCicp { hdr, .. } => *hdr, _ => None }
    }

    /// The outer result propagates temporary budget failures, which must not
    /// become retained metadata errors. The inner result belongs to display.
    pub fn read(decoder: &mut dyn ImageDecoder, encoded: &super::EncodedImage, budget: &ByteBudget) -> Result<Result<Option<Arc<Self>>>> {
        let source = SourceChannels::from_image(decoder.color_type());
        Self::read_metadata(source, encoded, || ProfileBytes::read(decoder.icc_profile().map(|profile| profile.map(Cow::Owned)), budget))
    }

    pub(super) fn read_jpeg(decoder: &mut super::jpeg::Decoder<'_>, encoded: &super::EncodedImage, budget: &ByteBudget) -> Result<Result<Option<Arc<Self>>>> {
        let source = if decoder.is_cmyk() { SourceChannels::Cmyk } else { SourceChannels::from_image(decoder.color_type()) };
        Self::read_metadata(source, encoded, || match decoder.icc_profile_chunks() {
            Some(chunks) => ProfileBytes::from_chunks(chunks, budget).map(|profile| profile.map(Some)),
            None => Ok(Ok(None)),
        })
    }

    /// Native APNG already owns a PNG reader; do not reopen the same stream or
    /// inflate its metadata through a second image decoder.
    pub(super) fn read_png(info: &::png::Info<'_>, encoded: &super::EncodedImage, budget: &ByteBudget) -> Result<Result<Option<Arc<Self>>>> {
        let grayscale = matches!(info.color_type, ::png::ColorType::Grayscale | ::png::ColorType::GrayscaleAlpha);
        let source = if grayscale { SourceChannels::Gray } else { SourceChannels::Rgb };
        Self::read_metadata(source, encoded, || ProfileBytes::read(Ok(info.icc_profile.as_deref().map(Cow::Borrowed)), budget))
    }

    fn read_metadata(source: SourceChannels, encoded: &super::EncodedImage,
        icc: impl FnOnce() -> Result<Result<Option<ProfileBytes>>>) -> Result<Result<Option<Arc<Self>>>> {
        let png = if encoded.format == Some(image::ImageFormat::Png) {
            match png::read(encoded.bytes()) { Ok(metadata) => Some(metadata), Err(error) => return Ok(Err(error)) }
        } else { None };
        // PNG's recognized cICP takes precedence over iCCP. The same header
        // pass also retains lower-priority fields, without inflating pixels.
        if let Some(metadata) = png.as_ref() {
            match metadata.cicp() {
                Ok(Some(cicp)) => {
                    if cicp == [1, 13, 0, 1] { return Ok(Ok(None)); }
                    let hdr = if matches!(cicp[1], 16 | 18) {
                        match hdr::Input::read(cicp, metadata) { Ok(input) => Some(input), Err(error) => return Ok(Err(error)) }
                    } else { None };
                    return Ok(Ok(Some(Self::new(Description::PngCicp { codes: cicp, hdr }, source))));
                },
                Ok(None) => {},
                Err(error) => return Ok(Err(error)),
            }
        }
        let profile = match icc()? {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                let fallback = match png.as_ref() { Some(metadata) => metadata.fallback(), None => Ok(None) };
                return Ok(fallback.map(|sdr| sdr.map(|sdr| Self::new(Description::PngSdr(sdr), source))));
            },
            Err(error) => return Ok(Err(error)),
        };
        Ok(Ok(Some(Self::new(Description::Icc(profile), source))))
    }

    fn new(description: Description, source: SourceChannels) -> Arc<Self> {
        Arc::new(Self { description, source })
    }

    pub fn working_high_bytes(&self, width: u32) -> u64 {
        WORKING_BYTES + u64::from(width) * if self.hdr().is_some() { 48 } else { 32 }
    }

    fn profile(&self) -> Result<(ColorProfile, Layout, usize, RenderingIntent)> {
        let (profile, expanded_rgb) = match &self.description {
            Description::Icc(profile) => (ColorProfile::new_from_slice_with_options(&profile.bytes, ParsingOptions {
                max_profile_size: MAX_PROFILE_BYTES + 1,
                max_allowed_clut_size: MAX_PROFILE_BYTES,
                max_allowed_trc_size: 16_384,
            }).map_err(color_error)?, false),
            Description::PngCicp { codes: cicp, .. } => {
                let mut profile = ColorProfile::new_srgb();
                profile.cicp = None;
                // In 0.8.1 this setter returns false even after a successful
                // update; code points were validated at metadata ingress.
                profile.update_rgb_colorimetry_from_cicp(CicpProfile {
                    color_primaries: CicpColorPrimaries::try_from(cicp[0]).map_err(color_error)?,
                    transfer_characteristics: TransferCharacteristics::try_from(cicp[1]).map_err(color_error)?,
                    matrix_coefficients: MatrixCoefficients::Identity, full_range: cicp[3] == 1,
                });
                if profile.cicp.is_none() { return Err(Error::Image("无法转换图片的 CICP 色彩信息".into())); }
                // PNG gray/palette samples use RGB colorimetry. The caller
                // expands native gray samples in its bounded row buffer.
                (profile, true)
            },
            Description::PngSdr(sdr) => (sdr.profile(self.source == SourceChannels::Gray)?, true),
        };
        let (layout, channels) = match profile.color_space {
            DataColorSpace::Rgb if expanded_rgb || self.source == SourceChannels::Rgb => (Layout::Rgba, 4),
            DataColorSpace::Gray if self.source == SourceChannels::Gray => (Layout::GrayAlpha, 2),
            DataColorSpace::Cmyk if self.source == SourceChannels::Cmyk => (Layout::Rgba, 4),
            DataColorSpace::Rgb | DataColorSpace::Gray | DataColorSpace::Cmyk => {
                return Err(Error::Image("图片色彩配置与像素通道不一致".into()));
            },
            // Lab/other generic decoders have already discarded their source
            // channels; those profiles must not be applied to converted RGB.
            _ => return Err(Error::Image("无法转换图片的原始颜色通道".into())),
        };
        // Match SkCMS's A2B0 -> A2B1 priority rather than rejecting a profile
        // whose valid lookup table only supplies the colorimetric intent.
        // https://skia.googlesource.com/skia/+/main/modules/skcms/src/skcms_public.h
        let rendering_intent = if profile.lut_a_to_b_perceptual.is_none() && profile.lut_a_to_b_colorimetric.is_some() {
            RenderingIntent::RelativeColorimetric
        } else { RenderingIntent::Perceptual };
        Ok((profile, layout, channels, rendering_intent))
    }

    /// JPEG samples are inverted CMYK; ICC expects conventional ink coverage.
    /// The single scanline scratch is admitted by working_bytes. The owned
    /// display output is admitted by the caller, just like the RGBA path.
    pub(super) fn apply_cmyk(&self, samples: &[u8], width: u32) -> Result<Vec<u8>> {
        if self.source != SourceChannels::Cmyk { return Err(Error::Image("CMYK 色彩转换通道不一致".into())); }
        let (profile, layout, _, rendering_intent) = self.profile()?;
        let row_bytes = (width as usize).checked_mul(4).ok_or_else(|| Error::ResourceLimit("图片行尺寸超出限制".into()))?;
        if row_bytes == 0 || samples.len() % row_bytes != 0 { return Err(Error::Image("CMYK 色彩转换像素输入无效".into())); }
        let transform = profile.create_transform_8bit(layout, &ColorProfile::new_srgb(), Layout::Rgba,
            TransformOptions { rendering_intent, ..Default::default() }).map_err(color_error)?;
        let mut source = vec![0_u8; row_bytes];
        let mut output = vec![0_u8; samples.len()];
        for (sample_row, output_row) in samples.chunks_exact(row_bytes).zip(output.chunks_exact_mut(row_bytes)) {
            for (ink, inverted) in source.iter_mut().zip(sample_row) { *ink = 255 - *inverted; }
            transform.transform(&source, output_row).map_err(color_error)?;
            // Layout::Rgba names CMYK's four-channel storage; K is not alpha.
            for pixel in output_row.chunks_exact_mut(4) { pixel[3] = 255; }
        }
        Ok(output)
    }

    /// Convert the display's owned, straight RGBA to sRGB before resize/orientation.
    pub fn apply(&self, rgba: &mut [u8], width: u32) -> Result<()> {
        let (profile, layout, channels, rendering_intent) = self.profile()?;
        let width = width as usize;
        let row_bytes = width.checked_mul(4).ok_or_else(|| Error::ResourceLimit("图片行尺寸超出限制".into()))?;
        if row_bytes == 0 || rgba.len() % row_bytes != 0 {
            return Err(Error::Image("色彩转换像素输入无效".into()));
        }
        if self.narrow_range() || self.hdr().is_some() {
            let (transform, mapping) = self.float_transform(&profile, layout, rendering_intent)?;
            let range = if self.narrow_range() { (16., 219.) } else { (0., 255.) };
            return apply_float(transform.as_ref(), mapping.as_ref(), rgba, row_bytes, range);
        }
        let transform = profile.create_transform_8bit(
            layout, &ColorProfile::new_srgb(), Layout::Rgba, TransformOptions { rendering_intent, ..Default::default() },
        ).map_err(color_error)?;
        let mut source = vec![0_u8; width * channels];
        for row in rgba.chunks_exact_mut(row_bytes) {
            if channels == 4 {
                source.copy_from_slice(row);
            } else {
                for (pixel, gray) in row.chunks_exact(4).zip(source.chunks_exact_mut(2)) {
                    gray[0] = pixel[0];
                    gray[1] = pixel[3];
                }
            }
            transform.transform(&source, row).map_err(color_error)?;
            // Alpha is coverage, not a profile color channel.
            for (pixel, original) in row.chunks_exact_mut(4).zip(source.chunks_exact(channels)) {
                pixel[3] = original[channels - 1];
            }
        }
        Ok(())
    }

    /// Normalize one native scanline at a time. RGB is quantized only after
    /// profile conversion; alpha uses the canonical image crate rounding rule.
    pub fn apply_high(&self, image: &image::DynamicImage) -> Result<Vec<u8>> {
        let (profile, layout, channels, rendering_intent) = self.profile()?;
        let (transform, mapping) = self.float_transform(&profile, layout, rendering_intent)?;
        let narrow = self.narrow_range();
        macro_rules! convert_u16 {
            ($image:expr, $channels:expr) => {
                convert_high($image.as_raw(), $image.width(), $channels, channels, transform.as_ref(), mapping.as_ref(),
                    if narrow { (4096.0, 56064.0) } else { (0.0, 65535.0) },
                    |value: u16| f32::from(value), |value: u16| ((u32::from(value) + 128) / 257) as u8)
            };
        }
        macro_rules! convert_f32 {
            ($image:expr, $channels:expr) => {
                if narrow { Err(Error::Image("PNG CICP 像素精度无效".into())) } else {
                    convert_high($image.as_raw(), $image.width(), $channels, channels, transform.as_ref(), mapping.as_ref(), (0.0, 1.0),
                        |value: f32| value, |value: f32| {
                            // image::FromPrimitive<f32> maps NaN to the maximum.
                            let value = if !(value < 1.0) { 1.0 } else { value.max(0.0) };
                            (value * 255.0).round() as u8
                        })
                }
            };
        }
        match image {
            image::DynamicImage::ImageLuma16(image) => convert_u16!(image, 1),
            image::DynamicImage::ImageLumaA16(image) => convert_u16!(image, 2),
            image::DynamicImage::ImageRgb16(image) => convert_u16!(image, 3),
            image::DynamicImage::ImageRgba16(image) => convert_u16!(image, 4),
            image::DynamicImage::ImageRgb32F(image) => convert_f32!(image, 3),
            image::DynamicImage::ImageRgba32F(image) => convert_f32!(image, 4),
            _ => Err(Error::Task("高精度色彩转换表示不一致".into())),
        }
    }

    fn float_transform(&self, profile: &ColorProfile, layout: Layout, rendering_intent: RenderingIntent)
        -> Result<(Arc<moxcms::TransformF32Executor>, Option<hdr::Mapping>)> {
        let (target, mapping) = if let Some(input) = self.hdr() {
            let linear = hdr::linear_profile(profile);
            let mapping = hdr::Mapping::new(&linear, input)?;
            (linear, Some(mapping))
        } else { (ColorProfile::new_srgb(), None) };
        // MoxCMS 0.8.1's HLG curve applies gamma per channel. The HDR
        // stage instead decodes scene light and applies the luminance OOTF.
        let source = if self.hdr().is_some_and(|input| input.is_hlg()) { &target } else { profile };
        let transform = source.create_transform_f32(layout, &target, Layout::Rgba,
            TransformOptions { rendering_intent, allow_extended_range_rgb_xyz: true, ..Default::default() }).map_err(color_error)?;
        Ok((transform, mapping))
    }
}

fn convert_high<T: Copy>(samples: &[T], width: u32, source_channels: usize, channels: usize,
    transform: &moxcms::TransformF32Executor, mapping: Option<&hdr::Mapping>, range: (f32, f32),
    sample: impl Fn(T) -> f32, coverage: impl Fn(T) -> u8) -> Result<Vec<u8>> {
    let width = width as usize;
    let source_row = width.checked_mul(source_channels).filter(|bytes| *bytes > 0)
        .ok_or_else(|| Error::Image("高精度图片行尺寸无效".into()))?;
    if samples.len() % source_row != 0 { return Err(Error::Image("高精度图片输入无效".into())); }
    let output_len = (samples.len() / source_channels).checked_mul(4)
        .ok_or_else(|| Error::ResourceLimit("高精度图片输出尺寸超出限制".into()))?;
    let mut data = vec![0_u8; output_len];
    let mut input = vec![0_f32; width * channels];
    let mut output = vec![0_f32; width * 4];
    let mut scratch = if mapping.is_some() { vec![0_f32; width * 4] } else { Vec::new() };
    let gray = source_channels <= 2;
    let has_alpha = matches!(source_channels, 2 | 4);
    for (original, row) in samples.chunks_exact(source_row).zip(data.chunks_exact_mut(width * 4)) {
        for (pixel, input) in original.chunks_exact(source_channels).zip(input.chunks_exact_mut(channels)) {
            let colors = if channels == 2 { 1 } else { 3 };
            for channel in 0..colors {
                let value = sample(pixel[if gray { 0 } else { channel }]);
                if !value.is_finite() { return Err(Error::Image("高精度图片颜色数值无效".into())); }
                input[channel] = (value - range.0) / range.1;
            }
            // Alpha is independent coverage. Quantizing it here cannot affect
            // straight RGB; the final byte is copied from the source separately.
            input[channels - 1] = if has_alpha { f32::from(coverage(pixel[source_channels - 1])) / 255.0 } else { 1.0 };
        }
        transform.transform(&input, &mut output).map_err(color_error)?;
        if let Some(mapping) = mapping { mapping.apply(&mut output, &mut scratch)?; }
        for ((original, pixel), converted) in original.chunks_exact(source_channels).zip(row.chunks_exact_mut(4)).zip(output.chunks_exact(4)) {
            for channel in 0..3 {
                if !converted[channel].is_finite() { return Err(Error::Image("图片色彩转换结果无效".into())); }
                pixel[channel] = (converted[channel] * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            pixel[3] = if has_alpha { coverage(original[source_channels - 1]) } else { 255 };
        }
    }
    Ok(data)
}

fn apply_float(transform: &moxcms::TransformF32Executor, mapping: Option<&hdr::Mapping>, rgba: &mut [u8], row_bytes: usize, range: (f32, f32)) -> Result<()> {
    let mut source = vec![0_f32; row_bytes];
    let mut output = vec![0_f32; row_bytes];
    let mut scratch = if mapping.is_some() { vec![0_f32; row_bytes] } else { Vec::new() };
    for row in rgba.chunks_exact_mut(row_bytes) {
        for (pixel, input) in row.chunks_exact(4).zip(source.chunks_exact_mut(4)) {
            for channel in 0..3 { input[channel] = (f32::from(pixel[channel]) - range.0) / range.1; }
            input[3] = f32::from(pixel[3]) / 255.0;
        }
        transform.transform(&source, &mut output).map_err(color_error)?;
        if let Some(mapping) = mapping { mapping.apply(&mut output, &mut scratch)?; }
        for (pixel, converted) in row.chunks_exact_mut(4).zip(output.chunks_exact(4)) {
            for channel in 0..3 {
                let value = converted[channel];
                if !value.is_finite() { return Err(Error::Image("图片色彩转换结果无效".into())); }
                pixel[channel] = (value * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            // Alpha remains the original full-range coverage byte.
        }
    }
    Ok(())
}

fn color_error(error: moxcms::CmsError) -> Error {
    Error::Image(format!("图片色彩转换失败：{error}").into())
}
