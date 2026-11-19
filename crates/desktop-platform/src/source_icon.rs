//! Source icons are encoded once for persistence; native pixels do not travel through data URLs.
use std::{path::Path, sync::Arc};
use bytes::Bytes;
use super_paste_core::{Error, Result, images::{AlphaMode, ImageLimits, ImageService, ImageSource, PixelLayout, PixelSpec, ResizeFilter}};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use crate::source::ActiveWindow;

const ICON_SIZE: u32 = 96;
const MAX_ICON_FILE: u64 = 8 * 1024 * 1024;
pub(crate) const ICON_CACHE_BYTES: usize = 4 * 1024 * 1024;

pub struct SourceIcons {
    images: Arc<ImageService>, runtime: Handle, cancel: CancellationToken,
    #[cfg(target_os = "linux")]
    fonts: std::sync::OnceLock<Arc<resvg::usvg::fontdb::Database>>,
}
impl SourceIcons {
    pub fn new(images: Arc<ImageService>, runtime: Handle) -> Arc<Self> {
        Arc::new(Self { images, runtime, cancel: CancellationToken::new(), #[cfg(target_os = "linux")] fonts: std::sync::OnceLock::new() })
    }
    pub(crate) fn cancel(&self) { self.cancel.cancel(); }

    pub(crate) fn load(&self, window: &ActiveWindow) -> Result<Bytes> {
        if self.cancel.is_cancelled() { return Err(Error::Cancelled); }
        #[cfg(target_os = "linux")]
        let window_icon = if !clipboard_core::is_wayland_session() { window.id.parse::<u32>().ok().and_then(|id| {
            x11_icon(id).and_then(|icon| self.encode_pixels(icon.width, icon.height, icon.pixels, AlphaMode::Straight))
                .inspect_err(|error| tracing::debug!(%error, "X11 window icon unavailable; trying application icon")).ok()
        }) } else { None };
        #[cfg(not(target_os = "linux"))]
        let window_icon: Option<Bytes> = None;
        if let Some(icon) = &window_icon {
            if !needs_refresh(icon) { return Ok(icon.clone()); }
        }
        match self.application_icon(window) {
            Ok(icon) => {
                if let Some(previous) = window_icon {
                    if resolution(&previous) > resolution(&icon) { return Ok(previous); }
                }
                Ok(icon)
            }
            Err(error) => {
                if let Some(icon) = window_icon { return Ok(icon); }
                #[cfg(not(target_os = "linux"))]
                if let Some(native) = &window.native {
                    // x-win exposes only a PNG data URL here. Decode its envelope at this library
                    // boundary; the ordinary file/system icon path uses owned native RGBA.
                    use base64::{Engine, engine::general_purpose::STANDARD};
                    let icon = x_win::get_window_icon(native).map_err(|failure| Error::Platform(failure.to_string().into()))?;
                    let payload = icon.data.strip_prefix("data:image/png;base64,").ok_or_else(|| Error::Image("来源窗口图标格式无效".into()))?;
                    if payload.len() as u64 > MAX_ICON_FILE * 4 / 3 + 4 { return Err(Error::ResourceLimit("来源窗口图标过大".into())); }
                    return self.normalize_encoded(STANDARD.decode(payload).map_err(|failure| Error::Image(failure.to_string().into()))?);
                }
                Err(error)
            }
        }
    }

    fn normalize_encoded(&self, data: Vec<u8>) -> Result<Bytes> {
        if data.len() as u64 > MAX_ICON_FILE { return Err(Error::ResourceLimit("来源应用图标过大".into())); }
        self.runtime.block_on(async {
            let encoded = self.images.import_encoded(data, &self.cancel).await?;
            if encoded.format() == Some(image::ImageFormat::Png)
                && encoded.dimensions().is_some_and(|(width, height)| width <= ICON_SIZE && height <= ICON_SIZE)
                && encoded.bytes().len() <= ICON_CACHE_BYTES
            {
                // Validate the pixels, retaining metadata without a codec round trip.
                self.images.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &self.cancel).await?;
                return Ok(encoded.shared_bytes());
            }
            // Normalizing strips metadata, so apply its color/orientation first.
            // Go straight from the shared native decode to bounded display pixels,
            // avoiding an unmanaged RGBA8 projection before color conversion.
            let display = self.images.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), PixelSpec {
                max_width: ICON_SIZE, max_height: ICON_SIZE, layout: PixelLayout::Rgba8, filter: ResizeFilter::Lanczos3,
            }, &self.cancel).await?;
            let png = self.images.encode_png(display.pixels, &self.cancel).await?;
            Ok(png.shared_bytes())
        })
    }
    fn encode_pixels(&self, width: u32, height: u32, data: Vec<u8>, alpha: AlphaMode) -> Result<Bytes> {
        self.runtime.block_on(async {
            let pixels = self.images.import_pixels(width, height, width as usize * 4, PixelLayout::Rgba8, alpha, data, &self.cancel).await?;
            let pixels = self.images.derive_pixels(pixels, PixelSpec { max_width: ICON_SIZE, max_height: ICON_SIZE, layout: PixelLayout::Rgba8, filter: ResizeFilter::Lanczos3 }, &self.cancel).await?;
            Ok(self.images.encode_png(pixels, &self.cancel).await?.shared_bytes())
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn application_icon(&self, window: &ActiveWindow) -> Result<Bytes> {
        let path = window.path.as_deref().filter(|path| !path.is_empty()).ok_or_else(|| Error::Platform("来源应用路径为空".into()))?;
        let icon = file_icon_provider::get_file_icon(path, ICON_SIZE as u16).map_err(|error| Error::Platform(format!("无法读取来源应用图标: {error}").into()))?;
        // file_icon_provider 1.0.2 draws into AppKit's default premultiplied
        // NSBitmapImageRep. Let the shared image service normalize it for PNG.
        let alpha = if cfg!(target_os = "macos") { AlphaMode::Premultiplied } else { AlphaMode::Straight };
        self.encode_pixels(icon.width, icon.height, icon.pixels, alpha)
    }

    #[cfg(target_os = "linux")]
    fn application_icon(&self, window: &ActiveWindow) -> Result<Bytes> {
        use std::io::Read;
        use freedesktop_desktop_entry::{desktop_entries, find_app_by_id, unicase::Ascii};
        let locales = std::env::var("LANG").ok().into_iter().collect::<Vec<_>>();
        let entries = desktop_entries(&locales);
        let application = window.name.trim();
        let path = window.path.as_deref().map(|path| path.strip_suffix(" (deleted)").unwrap_or(path));
        let executable = path.and_then(|path| Path::new(path).file_name()?.to_str());
        let entry = find_app_by_id(&entries, Ascii::new(application))
            .or_else(|| application.strip_suffix(".desktop").and_then(|name| find_app_by_id(&entries, Ascii::new(name))))
            .or_else(|| path.and_then(|path| find_app_by_id(&entries, Ascii::new(path))))
            .or_else(|| executable.and_then(|name| find_app_by_id(&entries, Ascii::new(name))))
            .ok_or_else(|| Error::Platform("未找到来源应用的桌面条目".into()))?;
        let name = entry.icon().filter(|name| !name.trim().is_empty()).ok_or_else(|| Error::Platform("来源应用未声明图标".into()))?;
        let theme = icon_theme();
        let file = if Path::new(name).is_absolute() { Path::new(name).to_owned() }
        else { freedesktop_icons::lookup(name).with_size(32).with_scale(3).with_theme(&theme).find().ok_or_else(|| Error::Platform("系统主题中没有来源应用图标".into()))? };
        let input = std::fs::File::open(&file)?;
        if input.metadata()?.len() > MAX_ICON_FILE { return Err(Error::ResourceLimit("来源应用图标文件过大".into())); }
        // Bound the actual read, including files that grow after metadata was read.
        let mut data = Vec::new();
        input.take(MAX_ICON_FILE + 1).read_to_end(&mut data)?;
        if data.len() as u64 > MAX_ICON_FILE { return Err(Error::ResourceLimit("来源应用图标文件过大".into())); }
        if file.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("svg")) { self.render_svg(&data, &file) }
        else { self.normalize_encoded(data) }
    }

    #[cfg(target_os = "linux")]
    fn render_svg(&self, data: &[u8], file: &Path) -> Result<Bytes> {
        use resvg::{tiny_skia, usvg};
        let mut options = usvg::Options { resources_dir: file.canonicalize()?.parent().map(Path::to_owned), ..usvg::Options::default() };
        if let Some(fonts) = self.fonts.get() { options.fontdb = fonts.clone(); }
        let select_font = usvg::FontResolver::default_font_selector();
        options.font_resolver.select_font = Box::new(move |font, database| {
            if self.cancel.is_cancelled() { return None; }
            // Most application icons contain only paths. Initialize the shared
            // fonts only when usvg actually resolves text, retaining its native
            // font selection and fallback. Never replace a populated database.
            if database.faces().next().is_none() {
                *database = self.fonts.get_or_init(|| {
                    let mut fonts = usvg::fontdb::Database::new();
                    fonts.load_system_fonts();
                    Arc::new(fonts)
                }).clone();
            }
            select_font(font, database)
        });
        let tree = usvg::Tree::from_data(data, &options).map_err(|error| Error::Image(format!("SVG 应用图标无效: {error}").into()))?;
        let size = tree.size();
        let scale = (ICON_SIZE as f32 / size.width()).min(ICON_SIZE as f32 / size.height()).min(1.0);
        let width = (size.width() * scale).round().max(1.0) as u32;
        let height = (size.height() * scale).round().max(1.0) as u32;
        let mut pixels = tiny_skia::Pixmap::new(width, height).ok_or_else(|| Error::Image("SVG 应用图标尺寸无效".into()))?;
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixels.as_mut());
        self.encode_pixels(width, height, pixels.take(), AlphaMode::Premultiplied)
    }
}

fn resolution(data: &[u8]) -> u32 { imagesize::blob_size(data).map(|size| size.width.min(size.height) as u32).unwrap_or(0) }
pub(crate) fn needs_refresh(data: &[u8]) -> bool { resolution(data) < ICON_SIZE }

#[cfg(target_os = "linux")]
fn icon_theme() -> String {
    let Some(config) = dirs::config_dir() else { return "hicolor".into(); };
    for (file, section, key) in [("kdeglobals", "Icons", "Theme"), ("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name")] {
        let Ok(data) = std::fs::read_to_string(config.join(file)) else { continue; };
        let mut active = false;
        for line in data.lines().map(str::trim) {
            if line.starts_with('[') { active = line == format!("[{section}]"); }
            else if active { if let Some((name, value)) = line.split_once('=') { if name.trim() == key && !value.trim().is_empty() { return value.trim().trim_matches('"').to_owned(); } } }
        }
    }
    "hicolor".into()
}

#[cfg(target_os = "linux")]
fn x11_icon(window_id: u32) -> Result<NativeIcon> {
    use xcb::{Xid, XidNew, x};
    let (connection, _) = xcb::Connection::connect(None).map_err(|error| Error::Platform(error.to_string().into()))?;
    let atom = connection.wait_for_reply(connection.send_request(&x::InternAtom { only_if_exists: true, name: b"_NET_WM_ICON" }))
        .map_err(|error| Error::Platform(error.to_string().into()))?.atom();
    if atom.is_none() { return Err(Error::NotFound); }
    let reply = connection.wait_for_reply(connection.send_request(&x::GetProperty { delete: false, window: x::Window::new(window_id), property: atom, r#type: x::ATOM_CARDINAL, long_offset: 0, long_length: 2 * 1024 * 1024 }))
        .map_err(|error| Error::Platform(error.to_string().into()))?;
    if reply.bytes_after() != 0 { return Err(Error::ResourceLimit("X11 图标属性过大".into())); }
    if reply.format() != 32 || reply.r#type() != x::ATOM_CARDINAL { return Err(Error::Image("X11 图标属性格式无效".into())); }
    let values = reply.value::<u32>();
    let mut offset = 0usize;
    let mut selected: Option<(u32, u32, &[u32])> = None;
    while offset + 2 <= values.len() {
        let width = values[offset]; let height = values[offset + 1]; offset += 2;
        let length = (width as usize).checked_mul(height as usize).ok_or_else(|| Error::Image("X11 图标尺寸溢出".into()))?;
        if width == 0 || height == 0 || length > values.len() - offset { return Err(Error::Image("X11 图标属性截断".into())); }
        let candidate = (width, height, &values[offset..offset + length]);
        let score = |width: u32, height: u32| { let size = width.min(height); (size >= ICON_SIZE, if size >= ICON_SIZE { u32::MAX - size } else { size }) };
        if selected.as_ref().is_none_or(|previous| score(width, height) > score(previous.0, previous.1)) { selected = Some(candidate); }
        offset += length;
    }
    if offset != values.len() { return Err(Error::Image("X11 图标属性尾部截断".into())); }
    let (width, height, pixels) = selected.ok_or(Error::NotFound)?;
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels { rgba.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8, (pixel >> 24) as u8]); }
    Ok(NativeIcon { width, height, pixels: rgba })
}

#[cfg(target_os = "linux")]
struct NativeIcon { width: u32, height: u32, pixels: Vec<u8> }

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use super_paste_core::{images::ImageBudgets, storage::Storage};

    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) { std::fs::remove_dir_all(&self.0).expect("remove isolated source icon database"); }
    }

    fn with_icons(check: impl FnOnce(&SourceIcons)) {
        let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-icons-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&directory.0).unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let storage = runtime.block_on(Storage::open(directory.0.join("icons.sqlite3"))).unwrap();
        let images = ImageService::new(runtime.handle().clone(), storage, ImageBudgets::default()).unwrap();
        let icons = SourceIcons::new(images.clone(), runtime.handle().clone());
        check(&icons);
        icons.cancel();
        drop(icons);
        runtime.block_on(images.shutdown()).unwrap();
    }

    fn exif(orientation: u8) -> Vec<u8> {
        vec![b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0,
            1, 0, 0, 0, orientation, 0, 0, 0, 0, 0, 0, 0]
    }

    fn png(width: u32, height: u32, pixels: &[u8], exif: Vec<u8>) -> Vec<u8> {
        let mut output = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut output);
        encoder.set_exif_metadata(exif).unwrap();
        encoder.write_image(pixels, width, height, image::ExtendedColorType::Rgba8).unwrap();
        output
    }

    #[test]
    fn small_png_is_validated_and_shared_without_reencoding() {
        with_icons(|icons| {
            let input = png(1, 1, &[17, 91, 203, 128], exif(2));
            let expected = input.clone();
            let pointer = input.as_ptr();
            let output = icons.normalize_encoded(input).unwrap();
            assert_eq!(output.as_ref(), expected);
            assert_eq!(output.as_ptr(), pointer, "retain the input allocation and metadata");

            let mut truncated = expected.clone();
            truncated.truncate(33); // Complete IHDR, but no image data.
            assert!(icons.normalize_encoded(truncated).is_err());
            icons.cancel();
            assert!(matches!(icons.normalize_encoded(expected), Err(Error::Cancelled)));
        });
    }

    #[test]
    fn metadata_heavy_png_is_oriented_and_compacted_before_caching() {
        with_icons(|icons| {
            let source = [1_u8, 2, 3, 4, 5, 6].into_iter().flat_map(|value| [value, 0, 0, 255]).collect::<Vec<_>>();
            let mut metadata = exif(6);
            metadata.resize(ICON_CACHE_BYTES + 1, 0);
            let input = png(2, 3, &source, metadata);
            assert!(input.len() > ICON_CACHE_BYTES && input.len() as u64 <= MAX_ICON_FILE);
            let output = icons.normalize_encoded(input).unwrap();
            assert!(output.len() < 1024, "do not retain oversized metadata in the icon cache or database");
            let pixels = image::load_from_memory(&output).unwrap().into_rgba8();
            assert_eq!(pixels.dimensions(), (3, 2));
            let expected = [5_u8, 3, 1, 6, 4, 2].into_iter().flat_map(|value| [value, 0, 0, 255]).collect::<Vec<_>>();
            assert_eq!(pixels.as_raw(), &expected);
        });
    }

    #[test]
    fn jpeg_orientation_and_resized_png_alpha_survive_normalization() {
        with_icons(|icons| {
            let mut input = Vec::new();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut input, 100);
            encoder.set_exif_metadata(exif(6)).unwrap();
            encoder.encode(&[30, 80, 130, 180, 210, 240], 2, 3, image::ExtendedColorType::L8).unwrap();
            let expected = image::imageops::rotate90(&image::load_from_memory(&input).unwrap().into_rgba8());
            let output = icons.normalize_encoded(input).unwrap();
            assert_eq!(image::guess_format(&output).unwrap(), image::ImageFormat::Png);
            assert_eq!(image::load_from_memory(&output).unwrap().into_rgba8(), expected);

            let input = png(128, 64, &[128, 0, 0, 128].repeat(128 * 64), Vec::new());
            let output = icons.normalize_encoded(input).unwrap();
            let pixels = image::load_from_memory(&output).unwrap().into_rgba8();
            assert_eq!(pixels.dimensions(), (96, 48));
            for pixel in pixels.pixels() {
                assert!(pixel[0].abs_diff(128) <= 1 && pixel[3].abs_diff(128) <= 1, "{pixel:?}");
                assert_eq!([pixel[1], pixel[2]], [0, 0]);
            }
        });
    }
}
