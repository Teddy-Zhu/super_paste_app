//! Inverted JPEG CMYK is retained once; K is an ink component, never alpha.
use crate::{Error, Result};
use image::{GenericImageView, Rgb};

pub(super) struct RgbView<'a> {
    samples: &'a [u8],
    width: u32,
    height: u32,
}

impl<'a> RgbView<'a> {
    pub(super) fn new(samples: &'a [u8], width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || samples.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err(Error::Image("CMYK 像素尺寸不一致".into()));
        }
        Ok(Self { samples, width, height })
    }

    pub(super) fn rgba(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(self.samples.len());
        for sample in self.samples.chunks_exact(4) {
            let [r, g, b] = rgb(sample);
            output.extend_from_slice(&[r, g, b, 255]);
        }
        output
    }
}

impl GenericImageView for RgbView<'_> {
    type Pixel = Rgb<u8>;

    fn dimensions(&self) -> (u32, u32) { (self.width, self.height) }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        assert!(x < self.width && y < self.height, "CMYK pixel coordinates out of bounds");
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        Rgb(rgb(&self.samples[offset..offset + 4]))
    }
}

fn rgb(sample: &[u8]) -> [u8; 3] {
    // Match zune-jpeg 0.5.15's existing unprofiled/canonical RGB exactly.
    // Keeping the rounded multiply matters for content identity and OCR.
    let multiply = |component: u8| {
        let product = u32::from(component) * u32::from(sample[3]) + 128;
        ((product + (product >> 8)) >> 8) as u8
    };
    [multiply(sample[0]), multiply(sample[1]), multiply(sample[2])]
}
