use std::{collections::{HashMap, HashSet}, sync::{Arc, Weak}};
use gpui_kit::{RenderImage, Window};
use super_paste_core::{Error, Result, images::{AlphaMode, PixelBuffer, PixelLayout}};
use super_paste_format::Digest;

const WINDOW_RASTER_BYTES: u64 = 128 * 1024 * 1024;

/// One window owns its atlas entries. Visible cards, inline images and dialogs
/// share this pool; CPU data remains owned and budgeted by ImageService.
#[derive(Default)]
pub(crate) struct WindowRasters { entries: HashMap<Digest, Raster>, bytes: u64 }
struct Raster { image: Arc<RenderImage>, bytes: u64, pin: Weak<()> }
pub(crate) struct RasterPin { _owner: Arc<()> }
struct PixelOwner(Arc<PixelBuffer>);
impl AsRef<[u8]> for PixelOwner { fn as_ref(&self) -> &[u8] { self.0.bytes() } }

impl WindowRasters {
    pub fn acquire(&mut self, pixels: Arc<PixelBuffer>) -> Result<Arc<RenderImage>> {
        if let Some(raster) = self.entries.get(&pixels.id()) { return Ok(raster.image.clone()); }
        if pixels.layout() != PixelLayout::Bgra8 || pixels.alpha() != AlphaMode::Premultiplied
            || pixels.stride() != pixels.width() as usize * 4 {
            return Err(Error::Invalid("绘制图片需要紧密排列的预乘 Alpha BGRA8 像素".into()));
        }
        let bytes = pixels.bytes().len() as u64;
        if self.bytes.checked_add(bytes).is_none_or(|total| total > WINDOW_RASTER_BYTES) { return Err(Error::ResourceLimit("当前窗口图片绘制预算不足".into())); }
        let id = pixels.id();
        let image = RenderImage::from_shared_premultiplied_bgra(pixels.width(), pixels.height(), Arc::new(PixelOwner(pixels)))
            .ok_or_else(|| Error::Invalid("绘制图片缓冲区尺寸无效".into()))?;
        let image = Arc::new(image); self.entries.insert(id, Raster { image: image.clone(), bytes, pin: Weak::new() }); self.bytes += bytes;
        Ok(image)
    }
    pub fn pin(&mut self, id: Digest) -> Result<RasterPin> {
        let raster = self.entries.get_mut(&id).ok_or(Error::NotFound)?;
        let owner = raster.pin.upgrade().unwrap_or_else(|| Arc::new(()));
        raster.pin = Arc::downgrade(&owner); Ok(RasterPin { _owner: owner })
    }
    /// Reconcile all consumers together. A dialog can pin a card's image even
    /// after its row scrolls out; pruning must include both sets of owners.
    pub fn retain(&mut self, visible: &HashSet<Digest>, window: &mut Window) -> Result<()> {
        self.prune(visible, false, window)
    }
    fn prune(&mut self, visible: &HashSet<Digest>, force: bool, window: &mut Window) -> Result<()> {
        let expired: Vec<_> = self.entries.iter().filter(|(id, raster)| !visible.contains(*id) && (force || raster.pin.strong_count() == 0)).map(|(id, _)| *id).collect();
        let mut failure = None;
        for id in expired {
            let Some(raster) = self.entries.get(&id) else { continue; };
            if let Err(error) = window.drop_image(raster.image.clone()) { failure.get_or_insert_with(|| Error::Platform(error.to_string().into())); continue; }
            if let Some(raster) = self.entries.remove(&id) { self.bytes -= raster.bytes; }
        }
        failure.map_or(Ok(()), Err)
    }
    pub fn clear(&mut self, window: &mut Window) -> Result<()> { self.prune(&HashSet::new(), true, window) }
    pub fn estimated_atlas_bytes(&self) -> u64 { self.bytes }
}
