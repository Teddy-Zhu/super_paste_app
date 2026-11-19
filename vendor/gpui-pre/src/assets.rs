use crate::{DevicePixels, Pixels, Result, SharedString, Size, size};
use smallvec::SmallVec;

use image::{Delay, Frame};
use std::{
    borrow::Cow,
    fmt,
    hash::Hash,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering::SeqCst},
};

/// A source of assets for this app to use.
pub trait AssetSource: 'static + Send + Sync {
    /// Load the given asset from the source path.
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>>;

    /// List the assets at the given path.
    fn list(&self, path: &str) -> Result<Vec<SharedString>>;
}

impl AssetSource for () {
    fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

/// A unique identifier for the image cache
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ImageId(pub usize);

#[derive(PartialEq, Eq, Hash, Clone)]
#[expect(missing_docs)]
pub struct RenderImageParams {
    pub image_id: ImageId,
    pub frame_index: usize,
}

/// A cached and processed image, in BGRA format
pub struct RenderImage {
    /// The ID associated with this image
    pub id: ImageId,
    /// The scale factor of this image on render.
    pub(crate) scale_factor: f32,
    data: SmallVec<[Frame; 1]>,
    shared: Option<SharedBgra>,
}

struct SharedBgra { width: u32, height: u32, owner: Arc<dyn AsRef<[u8]> + Send + Sync>, premultiplied: bool }

impl PartialEq for RenderImage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RenderImage {}

impl RenderImage {
    /// Create a new image from the given data.
    pub fn new(data: impl Into<SmallVec<[Frame; 1]>>) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

        Self {
            id: ImageId(NEXT_ID.fetch_add(1, SeqCst)),
            scale_factor: 1.0,
            data: data.into(),
            shared: None,
        }
    }

    /// Wrap tightly packed, straight-alpha BGRA8 pixels without a CPU copy.
    /// The owner must keep its bytes immutable for the image's lifetime.
    /// GPU upload and driver storage remain separate allocations.
    pub fn from_shared_bgra(width: u32, height: u32, owner: Arc<dyn AsRef<[u8]> + Send + Sync>) -> Option<Self> {
        Self::shared_bgra(width, height, owner, false)
    }

    /// Wrap immutable premultiplied BGRA8 pixels without a CPU copy. The atlas
    /// filters associated colors, then the shader adapts to the target blend mode.
    pub fn from_shared_premultiplied_bgra(width: u32, height: u32, owner: Arc<dyn AsRef<[u8]> + Send + Sync>) -> Option<Self> {
        Self::shared_bgra(width, height, owner, true)
    }

    fn shared_bgra(width: u32, height: u32, owner: Arc<dyn AsRef<[u8]> + Send + Sync>, premultiplied: bool) -> Option<Self> {
        let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        if width == 0 || height == 0 || owner.as_ref().as_ref().len() != expected { return None; }
        let mut image = Self::new(SmallVec::new());
        image.shared = Some(SharedBgra { width, height, owner, premultiplied }); Some(image)
    }

    // Bit 0 is shared with PolychromeSprite in the native shaders. Frame-backed
    // images and color glyphs retain their existing straight-alpha path.
    pub(crate) fn image_flags(&self) -> u32 { u32::from(self.shared.as_ref().is_some_and(|shared| shared.premultiplied)) }

    /// Convert this image into a byte slice.
    pub fn as_bytes(&self, frame_index: usize) -> Option<&[u8]> {
        if let Some(shared) = &self.shared { return (frame_index == 0).then(|| shared.owner.as_ref().as_ref()); }
        self.data
            .get(frame_index)
            .map(|frame| frame.buffer().as_raw().as_slice())
    }

    /// Get the size of this image, in pixels.
    pub fn size(&self, frame_index: usize) -> Size<DevicePixels> {
        if let Some(shared) = &self.shared { return if frame_index == 0 { size(shared.width.into(), shared.height.into()) } else { Size::default() }; }
        self.data
            .get(frame_index)
            .map(|frame| {
                let (width, height) = frame.buffer().dimensions();
                size(width.into(), height.into())
            })
            .unwrap_or_default()
    }

    /// Get the size of this image, in pixels for display, adjusted for the scale factor.
    pub(crate) fn render_size(&self, frame_index: usize) -> Size<Pixels> {
        self.size(frame_index)
            .map(|v| (v.0 as f32 / self.scale_factor).into())
    }

    /// Get the delay of this frame from the previous
    pub fn delay(&self, frame_index: usize) -> Delay {
        self.data
            .get(frame_index)
            .map(|frame| frame.delay())
            .unwrap_or(Delay::from_numer_denom_ms(100, 1))
    }

    /// Get the number of frames for this image.
    pub fn frame_count(&self) -> usize {
        if self.shared.is_some() { 1 } else { self.data.len() }
    }
}

impl fmt::Debug for RenderImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("id", &self.id)
            .field("size", &self.size(0))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;

    #[test]
    fn empty_render_image_does_not_panic() {
        let image = RenderImage::new(SmallVec::new());
        assert_eq!(image.frame_count(), 0);
        assert_eq!(image.size(0), Size::default());
        assert_eq!(image.as_bytes(0), None);
        assert_eq!(image.render_size(0), Size::default());
        assert_eq!(image.delay(0), Delay::from_numer_denom_ms(100, 1));
        let _ = format!("{image:?}");
    }
}
