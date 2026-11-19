use std::{io::Cursor, sync::Arc, time::Duration};
use image::metadata::Orientation;
use png::{BitDepth, BlendOp, ColorType, DisposeOp, FrameControl};
use crate::{Error, Result, resources::{ByteBudget, Reservation}};
use super::{FrameData, Sample};
use super::super::{EncodedBytes, EncodedImage};

/// The PNG reader expands palette/sub-byte/transparency samples but never
/// strips 16-bit channels. Compositing needs no encode/decode round trip.
/// Disposal/blending: https://www.w3.org/TR/png-3/#fcTL-chunk
pub(super) struct Decoder {
    reader: png::Reader<Cursor<EncodedBytes>>,
    canvas: Canvas,
    dimensions: (u32, u32),
    remaining: u32,
    plays: Option<u32>,
    first: bool,
    poster: bool,
    previous: Option<FrameControl>,
    pixel_budget: ByteBudget,
}
enum Canvas { Eight(Pixels<u8>), Sixteen(Pixels<u16>) }
struct Pixels<T> { current: Vec<T>, restore: Option<Restore<T>> }
struct Restore<T> { pixels: Vec<T>, _reservation: Reservation }

impl Decoder {
    pub(super) fn new(encoded: &Arc<EncodedImage>, work_limit: u64, pixel_budget: &ByteBudget) -> Result<Self> {
        let bytes = usize::try_from(work_limit).map_err(|_| Error::ResourceLimit("PNG 工作区无法寻址".into()))?;
        let mut decoder = png::Decoder::new_with_limits(Cursor::new(EncodedBytes(encoded.clone())), png::Limits { bytes });
        decoder.set_transformations(png::Transformations::EXPAND);
        // Descriptive text is not used by image presentation. Keep the color
        // and orientation metadata while avoiding unrelated text allocations.
        decoder.set_ignore_text_chunk(true);
        let reader = decoder.read_info()?;
        let info = reader.info();
        let dimensions = (info.width, info.height);
        if Some(dimensions) != encoded.dimensions { return Err(Error::Image("APNG 尺寸校验失败".into())); }
        let animation = info.animation_control.ok_or_else(|| Error::Image("PNG 缺少动画控制信息".into()))?;
        if animation.num_frames == 0 { return Err(Error::Image("APNG 没有动画帧".into())); }
        let (_, depth) = reader.output_color_type();
        // Admission used IHDR's bit-depth byte. Check it before allocating a
        // canvas so malformed metadata cannot exceed that reservation.
        if (depth == BitDepth::Sixteen) != (encoded.bytes().get(24) == Some(&16)) {
            return Err(Error::Image("APNG 像素精度校验失败".into()));
        }
        let channels = u64::from(info.width) * u64::from(info.height) * 4;
        let count = usize::try_from(channels).map_err(|_| Error::ResourceLimit("APNG 画布无法寻址".into()))?;
        let canvas = match depth {
            BitDepth::Eight => Canvas::Eight(Pixels { current: vec![0; count], restore: None }),
            BitDepth::Sixteen => Canvas::Sixteen(Pixels { current: vec![0; count], restore: None }),
            _ => return Err(Error::Image("APNG 未展开到可显示的像素精度".into())),
        };
        let remaining = animation.num_frames;
        let plays = (animation.num_plays != 0).then_some(animation.num_plays);
        let poster = info.frame_control.is_none();
        Ok(Self { reader, canvas, dimensions, remaining, plays, first: true, poster, previous: None, pixel_budget: pixel_budget.clone() })
    }

    pub(super) fn info(&self) -> &png::Info<'static> { self.reader.info() }
    pub(super) fn plays(&self) -> Option<u32> { self.plays }
    pub(super) fn orientation(&self) -> Orientation {
        self.info().exif_metadata.as_ref().and_then(|bytes| Orientation::from_exif_chunk(bytes)).unwrap_or(Orientation::NoTransforms)
    }

    pub(super) fn next(&mut self, emit: bool) -> Result<Option<Sample>> {
        if self.remaining == 0 { return Ok(None); }
        if !self.first || self.poster {
            // A default IDAT image without fcTL is a poster, not an animation
            // frame. Skip it through the reader without allocating its bitmap.
            self.reader.next_frame_info()?;
        }
        let mut control = self.info().frame_control.ok_or_else(|| Error::Image("APNG 帧缺少控制信息".into()))?;
        self.validate_region(control)?;
        if self.first && control.dispose_op == DisposeOp::Previous { control.dispose_op = DisposeOp::Background; }
        let size = self.reader.output_buffer_size().ok_or_else(|| Error::ResourceLimit("APNG 子帧无法寻址".into()))?;
        let mut data = vec![0; size];
        let output = self.reader.next_frame(&mut data)?;
        if (output.width, output.height) != (control.width, control.height) {
            return Err(Error::Image("APNG 子帧尺寸不一致".into()));
        }
        // png 0.18.1 expands Adam7 rows at the IHDR canvas stride; ordinary
        // subframes are packed at their own width. Preserve that distinction.
        let row_width = if self.info().interlaced { self.dimensions.0 } else { control.width };
        let stride = row_width as usize * output.color_type.samples() * match output.bit_depth {
            BitDepth::Eight => 1, BitDepth::Sixteen => 2,
            _ => return Err(Error::Image("APNG 子帧像素精度无效".into())),
        };
        match &mut self.canvas {
            Canvas::Eight(pixels) if output.bit_depth == BitDepth::Eight => pixels.compose(self.dimensions.0, control, self.previous, &data, stride, output.color_type, &self.pixel_budget)?,
            Canvas::Sixteen(pixels) if output.bit_depth == BitDepth::Sixteen => pixels.compose(self.dimensions.0, control, self.previous, &data, stride, output.color_type, &self.pixel_budget)?,
            _ => return Err(Error::Image("APNG 子帧像素精度发生变化".into())),
        }
        // The decoded subframe is dead before the shared display snapshot is
        // allocated. Replay steps do not allocate or hash that snapshot at all.
        drop(data);
        let pixels = emit.then(|| match &self.canvas {
            Canvas::Eight(pixels) => FrameData::Eight(pixels.current.clone()),
            Canvas::Sixteen(pixels) => FrameData::Sixteen(pixels.current.clone()),
        });
        self.previous = Some(control);
        self.remaining -= 1;
        self.first = false;
        self.poster = false;
        let denominator = if control.delay_den == 0 { 100 } else { control.delay_den };
        let duration = Duration::from(image::Delay::from_numer_denom_ms(u32::from(control.delay_num) * 1000, u32::from(denominator)));
        Ok(Some(Sample { pixels, duration }))
    }

    fn validate_region(&self, frame: FrameControl) -> Result<()> {
        let fits = frame.width != 0 && frame.height != 0
            && frame.x_offset.checked_add(frame.width).is_some_and(|end| end <= self.dimensions.0)
            && frame.y_offset.checked_add(frame.height).is_some_and(|end| end <= self.dimensions.1);
        if !fits { return Err(Error::Image("APNG 子帧区域超出画布".into())); }
        Ok(())
    }
}

trait Channel: Copy + Default {
    const MAX: u64;
    const BYTES: usize;
    fn read(bytes: &[u8]) -> Self;
    fn value(self) -> u64;
    fn from_value(value: u64) -> Self;
}
impl Channel for u8 {
    const MAX: u64 = 255;
    const BYTES: usize = 1;
    fn read(bytes: &[u8]) -> Self { bytes[0] }
    fn value(self) -> u64 { u64::from(self) }
    fn from_value(value: u64) -> Self { value as Self }
}
impl Channel for u16 {
    const MAX: u64 = 65535;
    const BYTES: usize = 2;
    fn read(bytes: &[u8]) -> Self { u16::from_be_bytes([bytes[0], bytes[1]]) }
    fn value(self) -> u64 { u64::from(self) }
    fn from_value(value: u64) -> Self { value as Self }
}

impl<T: Channel> Pixels<T> {
    fn compose(&mut self, width: u32, frame: FrameControl, previous: Option<FrameControl>, data: &[u8], stride: usize, color: ColorType, budget: &ByteBudget) -> Result<()> {
        if color == ColorType::Indexed { return Err(Error::Image("APNG 调色板未展开".into())); }
        let samples = color.samples();
        let row_bytes = frame.width as usize * samples * T::BYTES;
        if stride < row_bytes || (frame.height as usize - 1).checked_mul(stride).and_then(|offset| offset.checked_add(row_bytes)).is_none_or(|end| end > data.len()) {
            return Err(Error::Image("APNG 子帧像素长度无效".into()));
        }
        if let Some(previous) = previous {
            let length = previous.width as usize * 4;
            match previous.dispose_op {
                DisposeOp::None => {},
                DisposeOp::Background => {
                    for y in 0..previous.height as usize {
                        let start = ((previous.y_offset as usize + y) * width as usize + previous.x_offset as usize) * 4;
                        self.current[start..start + length].fill(T::default());
                    }
                }
                DisposeOp::Previous => {
                    let restore = self.restore.as_ref().ok_or_else(|| Error::Task("APNG 缺少还原区域".into()))?;
                    if restore.pixels.len() != length * previous.height as usize { return Err(Error::Task("APNG 还原区域不一致".into())); }
                    for y in 0..previous.height as usize {
                        let start = ((previous.y_offset as usize + y) * width as usize + previous.x_offset as usize) * 4;
                        self.current[start..start + length].copy_from_slice(&restore.pixels[y * length..(y + 1) * length]);
                    }
                }
            }
        }
        if frame.dispose_op == DisposeOp::Previous {
            let length = frame.width as usize * 4;
            let count = length * frame.height as usize;
            if self.restore.as_ref().is_none_or(|restore| restore.pixels.capacity() < count || restore.pixels.capacity() > count.saturating_mul(2)) {
                // Release the old region before replacing it, and avoid Vec's
                // geometric growth or retaining a large region for tiny frames.
                // Only Previous pays for a restore region.
                self.restore = None;
                let reservation = budget.reserve(count as u64 * T::BYTES as u64)?;
                self.restore = Some(Restore { pixels: vec![T::default(); count], _reservation: reservation });
            }
            let restore = self.restore.as_mut().ok_or_else(|| Error::Task("APNG 未分配还原区域".into()))?;
            restore.pixels.resize(count, T::default());
            for y in 0..frame.height as usize {
                let start = ((frame.y_offset as usize + y) * width as usize + frame.x_offset as usize) * 4;
                restore.pixels[y * length..(y + 1) * length].copy_from_slice(&self.current[start..start + length]);
            }
        } else { self.restore = None; }
        for y in 0..frame.height as usize {
            let start = ((frame.y_offset as usize + y) * width as usize + frame.x_offset as usize) * 4;
            let row = &data[y * stride..y * stride + row_bytes];
            for (input, output) in row.chunks_exact(samples * T::BYTES).zip(self.current[start..start + frame.width as usize * 4].chunks_exact_mut(4)) {
                let read = |index: usize| T::read(&input[index * T::BYTES..]);
                let source = match color {
                    ColorType::Grayscale => { let value = read(0); [value, value, value, T::from_value(T::MAX)] },
                    ColorType::GrayscaleAlpha => { let value = read(0); [value, value, value, read(1)] },
                    ColorType::Rgb => [read(0), read(1), read(2), T::from_value(T::MAX)],
                    ColorType::Rgba => [read(0), read(1), read(2), read(3)],
                    ColorType::Indexed => return Err(Error::Image("APNG 调色板未展开".into())),
                };
                if frame.blend_op == BlendOp::Source { output.copy_from_slice(&source); }
                else { blend_over(output, source); }
            }
        }
        Ok(())
    }
}

/// Exact straight-alpha arithmetic at the source precision. Intermediates fit
/// u64 even for 16-bit channels; no premultiplied 8-bit temporary is introduced.
fn blend_over<T: Channel>(output: &mut [T], source: [T; 4]) {
    let alpha = source[3].value();
    if alpha == 0 { return; }
    let previous_alpha = output[3].value();
    if alpha == T::MAX || previous_alpha == 0 { output.copy_from_slice(&source); return; }
    let previous_weight = previous_alpha * (T::MAX - alpha);
    let source_weight = alpha * T::MAX;
    let coverage = source_weight + previous_weight;
    for channel in 0..3 {
        output[channel] = T::from_value((source[channel].value() * source_weight + output[channel].value() * previous_weight + coverage / 2) / coverage);
    }
    output[3] = T::from_value((coverage + T::MAX / 2) / T::MAX);
}
