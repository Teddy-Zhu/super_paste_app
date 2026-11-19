use image::{GenericImageView, Rgba, imageops::{self, FilterType}, metadata::Orientation};
use super::{PixelLayout, orient_rgba};

/// Associate before filtering, with exact 8-bit color × alpha products. The
/// borrowed view avoids a full-size premultiplied source allocation. image's
/// separable filter keeps float intermediates; its 16-bit result avoids early
/// 8-bit rounding of low-coverage colors and costs half a float output buffer.
pub(super) fn resize_display(
    source: &impl GenericImageView<Pixel = Rgba<u8>>, width: u32, height: u32,
    layout: PixelLayout, orientation: Orientation, associated: bool,
) -> Vec<u8> {
    let source = CoverageInput { source, associated };
    let resized = imageops::resize(&source, width, height, FilterType::Triangle);
    // Write layout and EXIF orientation directly into the final budgeted bytes.
    orient_rgba(&CoverageOutput(&resized), layout, orientation)
}

struct CoverageInput<'a, I> { source: &'a I, associated: bool }
impl<I: GenericImageView<Pixel = Rgba<u8>>> GenericImageView for CoverageInput<'_, I> {
    type Pixel = Rgba<u16>;
    fn dimensions(&self) -> (u32, u32) { self.source.dimensions() }
    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let [r, g, b, a] = self.source.get_pixel(x, y).0;
        let factor = if self.associated { 255 } else { u16::from(a) };
        Rgba([u16::from(r) * factor, u16::from(g) * factor, u16::from(b) * factor, u16::from(a) * 255])
    }
}

struct CoverageOutput<'a>(&'a image::ImageBuffer<Rgba<u16>, Vec<u16>>);
impl GenericImageView for CoverageOutput<'_> {
    type Pixel = Rgba<u8>;
    fn dimensions(&self) -> (u32, u32) { self.0.dimensions() }
    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        Rgba(self.0.get_pixel(x, y).0.map(|component| ((u32::from(component) + 127) / 255).min(255) as u8))
    }
}
