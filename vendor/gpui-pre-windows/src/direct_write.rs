use std::{
    borrow::Cow,
    cell::RefCell,
    ffi::{c_uint, c_void},
    mem::ManuallyDrop,
};

use anyhow::{Context, Result};
use collections::HashMap;
use gpui_util::{ResultExt, maybe};
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use smallvec::SmallVec;
use windows::{
    Win32::{
        Foundation::*,
        Globalization::GetUserDefaultLocaleName,
        Graphics::{
            Direct2D::Common::*,
            Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, Direct3D11::*, DirectWrite::*,
            Dxgi::Common::*, Gdi::LOGFONTW,
        },
        System::SystemServices::LOCALE_NAME_MAX_LENGTH,
        UI::WindowsAndMessaging::*,
    },
    core::*,
};
use windows_numerics::Vector2;

use crate::*;
use gpui::*;

#[derive(Debug)]
struct FontInfo {
    font_family_h: HSTRING,
    font_face: IDWriteFontFace3,
    features: IDWriteTypography,
    fallbacks: Option<IDWriteFontFallback>,
    font_collection: IDWriteFontCollection1,
    axes: SmallVec<[DWRITE_FONT_AXIS_VALUE; 5]>,
    style: FontStyle,
    raster_face: Option<IDWriteFontFace3>,
    shear: f32,
}

impl FontInfo {
    fn drawing_face(&self) -> &IDWriteFontFace3 { self.raster_face.as_ref().unwrap_or(&self.font_face) }

    fn set_axes(&self, layout: &IDWriteTextLayout, range: DWRITE_TEXT_RANGE) -> windows::core::Result<()> {
        let Some(angle) = oblique_degrees(self.style) else {
            if !self.axes.is_empty() {
                unsafe { layout.cast::<IDWriteTextLayout4>()?.SetFontAxisValues(&self.axes, range)?; }
            }
            return Ok(());
        };
        let layout = match layout.cast::<IDWriteTextLayout4>() {
            Ok(layout) => layout,
            // Older systems cannot shape variable fonts. Their static faces
            // still use the same authored outline/raster shear below.
            Err(error) if error.code() == E_NOINTERFACE && self.axes.is_empty() => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut axes = self.axes.clone();
        for (tag, value) in [(DWRITE_FONT_AXIS_TAG_SLANT, -angle), (DWRITE_FONT_AXIS_TAG_ITALIC, 0.)] {
            if let Some(axis) = axes.iter_mut().find(|axis| axis.axisTag == tag) { axis.value = value; }
            else { axes.push(DWRITE_FONT_AXIS_VALUE { axisTag: tag, value }); }
        }
        // Keep the requested angle for fallback selection, even when the
        // primary font is static or clamps to an endpoint of its slant axis.
        unsafe { layout.SetFontAxisValues(&axes, range) }
    }
}

enum MatchingFonts {
    Axes(IDWriteFontSet1),
    Legacy(IDWriteFontSet),
}

impl MatchingFonts {
    fn new(set: &IDWriteFontSet, family: &HSTRING, weight: FontWeight, style: FontStyle) -> windows::core::Result<Self> {
        match set.cast::<IDWriteFontSet1>() {
            Ok(set) => {
                let property = DWRITE_FONT_PROPERTY {
                    propertyId: DWRITE_FONT_PROPERTY_ID_WEIGHT_STRETCH_STYLE_FAMILY_NAME,
                    propertyValue: PCWSTR(family.as_ptr()), localeName: PCWSTR::null(),
                };
                let (italic, slant) = match style {
                    FontStyle::Normal => (0., 0.),
                    FontStyle::Italic => (1., 0.),
                    FontStyle::Oblique => (0., -14.),
                    FontStyle::ObliqueAngle(angle) => (0., -angle.degrees()),
                };
                let axes = [
                    DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT, value: weight.0 },
                    DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WIDTH, value: 100. },
                    DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_ITALIC, value: italic },
                    DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_SLANT, value: slant },
                ];
                // This overload creates unnamed variable instances and keeps
                // fractional weights. Keep those actual instances rather than
                // rebuilding a face from integer weight/stretch/style values.
                Ok(Self::Axes(unsafe { set.GetMatchingFonts(Some(&property), &axes)? }))
            }
            Err(error) if error.code() == E_NOINTERFACE => Ok(Self::Legacy(unsafe {
                set.GetMatchingFonts(family, font_weight_to_dwrite(weight),
                    DWRITE_FONT_STRETCH_NORMAL, font_style_to_dwrite(style))?
            })),
            Err(error) => Err(error),
        }
    }

    fn len(&self) -> u32 {
        unsafe { match self { Self::Axes(set) => set.GetFontCount(), Self::Legacy(set) => set.GetFontCount() } }
    }

    fn face(&self, index: u32) -> windows::core::Result<IDWriteFontFace3> {
        unsafe { match self {
            Self::Axes(set) => set.CreateFontFace(index)?.cast(),
            Self::Legacy(set) => set.GetFontFaceReference(index)?.CreateFontFace(),
        } }
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct NativeFontKey { face: usize, features: usize, fallbacks: usize, style: FontStyle }

impl NativeFontKey {
    fn new(face: &IDWriteFontFace3, features: &IDWriteTypography, fallbacks: Option<&IDWriteFontFallback>, style: FontStyle) -> windows::core::Result<Self> {
        Ok(Self {
            face: face.cast::<IUnknown>()?.as_raw().addr(),
            features: features.as_raw().addr(),
            fallbacks: fallbacks.map_or(0, |fallback| fallback.as_raw().addr()),
            style,
        })
    }
}

pub(crate) struct DirectWriteTextSystem {
    components: DirectWriteComponents,
    state: RwLock<DirectWriteState>,
}

struct DirectWriteComponents {
    locale: HSTRING,
    factory: IDWriteFactory5,
    in_memory_loader: IDWriteInMemoryFontFileLoader,
    builder: IDWriteFontSetBuilder1,
    text_renderer: TextRendererWrapper,
    system_ui_font_name: SharedString,
    system_subpixel_rendering: bool,
}

impl Drop for DirectWriteComponents {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .factory
                .UnregisterFontFileLoader(&self.in_memory_loader);
        }
    }
}

struct GPUState {
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,
    sampler: Option<ID3D11SamplerState>,
    blend_state: ID3D11BlendState,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
}

struct DirectWriteState {
    /// Used to rasterize color glyphs. `None` while the platform is headless: shaping and
    /// layout don't need a GPU, and nothing is drawn.
    gpu_state: Option<GPUState>,
    system_font_collection: IDWriteFontCollection1,
    custom_font_collection: IDWriteFontCollection1,
    fonts: Vec<FontInfo>,
    font_to_font_id: HashMap<Font, FontId>,
    font_info_cache: HashMap<NativeFontKey, FontId>,
    layout_line_scratch: Vec<u16>,
}

impl GPUState {
    fn new(directx_devices: &DirectXDevices) -> Result<Self> {
        let device = directx_devices.device.clone();
        let device_context = directx_devices.device_context.clone();

        let blend_state = {
            let mut blend_state = None;
            let desc = D3D11_BLEND_DESC {
                AlphaToCoverageEnable: false.into(),
                IndependentBlendEnable: false.into(),
                RenderTarget: [
                    D3D11_RENDER_TARGET_BLEND_DESC {
                        BlendEnable: true.into(),
                        SrcBlend: D3D11_BLEND_ONE,
                        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                        BlendOp: D3D11_BLEND_OP_ADD,
                        SrcBlendAlpha: D3D11_BLEND_ONE,
                        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                        BlendOpAlpha: D3D11_BLEND_OP_ADD,
                        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
                    },
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    Default::default(),
                ],
            };
            unsafe { device.CreateBlendState(&desc, Some(&mut blend_state)) }?;
            blend_state.unwrap()
        };

        let sampler = {
            let mut sampler = None;
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_BORDER,
                AddressV: D3D11_TEXTURE_ADDRESS_BORDER,
                AddressW: D3D11_TEXTURE_ADDRESS_BORDER,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                BorderColor: [0.0, 0.0, 0.0, 0.0],
                MinLOD: 0.0,
                MaxLOD: 0.0,
            };
            unsafe { device.CreateSamplerState(&desc, Some(&mut sampler)) }?;
            sampler
        };

        let vertex_shader = {
            let source = shader_resources::RawShaderBytes::new(
                shader_resources::ShaderModule::EmojiRasterization,
                shader_resources::ShaderTarget::Vertex,
            )?;
            let mut shader = None;
            unsafe { device.CreateVertexShader(source.as_bytes(), None, Some(&mut shader)) }?;
            shader.unwrap()
        };

        let pixel_shader = {
            let source = shader_resources::RawShaderBytes::new(
                shader_resources::ShaderModule::EmojiRasterization,
                shader_resources::ShaderTarget::Fragment,
            )?;
            let mut shader = None;
            unsafe { device.CreatePixelShader(source.as_bytes(), None, Some(&mut shader)) }?;
            shader.unwrap()
        };

        Ok(Self {
            device,
            device_context,
            sampler,
            blend_state,
            vertex_shader,
            pixel_shader,
        })
    }
}

impl DirectWriteTextSystem {
    /// Creates the text system, able to rasterize color glyphs once it has `directx_devices`.
    pub(crate) fn new(directx_devices: Option<&DirectXDevices>) -> Result<Self> {
        let factory: IDWriteFactory5 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        // The `IDWriteInMemoryFontFileLoader` here is supported starting from
        // Windows 10 Creators Update, which consequently requires the entire
        // `DirectWriteTextSystem` to run on `win10 1703`+.
        let in_memory_loader = unsafe { factory.CreateInMemoryFontFileLoader()? };
        unsafe { factory.RegisterFontFileLoader(&in_memory_loader)? };
        let builder = unsafe { factory.CreateFontSetBuilder()? };
        let mut locale = [0u16; LOCALE_NAME_MAX_LENGTH as usize];
        unsafe { GetUserDefaultLocaleName(&mut locale) };
        let locale = HSTRING::from_wide(&locale);
        let text_renderer = TextRendererWrapper::new();

        let gpu_state = directx_devices.map(GPUState::new).transpose()?;

        let system_subpixel_rendering = get_system_subpixel_rendering();
        let system_ui_font_name = get_system_ui_font_name();
        let components = DirectWriteComponents {
            locale,
            factory,
            in_memory_loader,
            builder,
            text_renderer,
            system_ui_font_name,
            system_subpixel_rendering,
        };

        let system_font_collection = unsafe {
            let mut result = None;
            components
                .factory
                .GetSystemFontCollection(false, &mut result, true)?;
            result.context("Failed to get system font collection")?
        };
        let custom_font_set = unsafe { components.builder.CreateFontSet()? };
        let custom_font_collection = unsafe {
            components
                .factory
                .CreateFontCollectionFromFontSet(&custom_font_set)?
        };

        Ok(Self {
            components,
            state: RwLock::new(DirectWriteState {
                gpu_state,
                system_font_collection,
                custom_font_collection,
                fonts: Vec::new(),
                font_to_font_id: HashMap::default(),
                font_info_cache: HashMap::default(),
                layout_line_scratch: Vec::new(),
            }),
        })
    }

    /// Uses `directx_devices` to rasterize color glyphs, replacing any previous devices, for
    /// example after the device was lost.
    pub(crate) fn handle_gpu_lost(&self, directx_devices: &DirectXDevices) -> Result<()> {
        self.state.write().handle_gpu_lost(directx_devices)
    }

    /// Releases the GPU state, when the platform switches to headless mode.
    pub(crate) fn release_gpu(&self) {
        self.state.write().gpu_state = None;
    }
}

impl PlatformTextSystem for DirectWriteTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.state.write().add_fonts(&self.components, fonts)
    }

    fn all_font_names(&self) -> Vec<String> {
        self.state.read().all_font_names(&self.components)
    }

    fn font_id(&self, font: &Font) -> Result<FontId> {
        let lock = self.state.upgradable_read();
        if let Some(font_id) = lock.font_to_font_id.get(font) {
            Ok(*font_id)
        } else {
            RwLockUpgradableReadGuard::upgrade(lock)
                .select_and_cache_font(&self.components, font)
                .with_context(|| format!("Failed to select font: {:?}", font))
        }
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.state.read().font_metrics(font_id)
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        self.state.read().get_typographic_bounds(font_id, glyph_id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> anyhow::Result<Size<f32>> {
        self.state.read().get_advance(font_id, glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.state.read().glyph_for_char(font_id, ch)
    }

    fn intersect_glyph_ink(
        &self, font_id: FontId, glyph_id: GlyphId, font_size: Pixels, ink: &mut GlyphInk,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(font_size.as_f32().is_finite() && font_size > Pixels::ZERO,
            "invalid glyph outline size");
        let glyph = u16::try_from(glyph_id.0)?;
        let state = self.state.read();
        let font = &state.fonts[font_id.0];
        let sink = ComObject::new(GlyphInkSink { ink: RefCell::new(ink.clone()), shear: font.shear });
        let interface = sink.to_interface::<ID2D1SimplifiedGeometrySink>();
        // DirectWrite supplies baseline-relative pixel outlines for the already
        // selected face, including its simulations and variation instance.
        unsafe {
            font.drawing_face().GetGlyphRunOutline(
                font_size.as_f32(), &glyph, None, None, 1, false, false, &interface,
            )?;
        }
        *ink = sink.ink.borrow().clone();
        Ok(())
    }

    fn glyph_raster_bounds(
        &self,
        params: &RenderGlyphParams,
    ) -> anyhow::Result<Bounds<DevicePixels>> {
        self.state.read().raster_bounds(&self.components, params)
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> anyhow::Result<(Size<DevicePixels>, Vec<u8>)> {
        self.state
            .read()
            .rasterize_glyph(&self.components, params, raster_bounds)
    }

    fn layout_line_with_direction(
        &self,
        text: &str,
        font_size: Pixels,
        runs: &[FontRun],
        direction: TextDirection,
    ) -> LineLayout {
        self.state
            .write()
            .layout_line(&self.components, text, font_size, runs, direction)
            .log_err()
            .unwrap_or(LineLayout {
                font_size,
                ..Default::default()
            })
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        if self.components.system_subpixel_rendering {
            TextRenderingMode::Subpixel
        } else {
            TextRenderingMode::Grayscale
        }
    }
}

impl DirectWriteState {
    fn select_and_cache_font(
        &mut self,
        components: &DirectWriteComponents,
        font: &Font,
    ) -> Option<FontId> {
        let select_font = |this: &mut DirectWriteState, font: &Font| -> Option<FontId> {
            let info = [&this.custom_font_collection, &this.system_font_collection]
                .into_iter()
                .find_map(|font_collection| unsafe {
                    DirectWriteState::make_font_from_font_collection(
                        font,
                        font_collection,
                        &components.factory,
                        &this.system_font_collection,
                        &components.system_ui_font_name,
                    )
                })?;

            let font_id = FontId(this.fonts.len());
            let font_face_key = NativeFontKey::new(&info.font_face, &info.features, info.fallbacks.as_ref(), info.style).log_err()?;
            this.fonts.push(info);
            this.font_info_cache.insert(font_face_key, font_id);
            Some(font_id)
        };

        let mut font_id = select_font(self, font);
        if font_id.is_none() {
            // try updating system fonts and reselect
            let mut collection = None;
            let font_collection_updated = unsafe {
                components
                    .factory
                    .GetSystemFontCollection(false, &mut collection, true)
            }
            .log_err()
            .is_some();
            if font_collection_updated && let Some(collection) = collection {
                self.system_font_collection = collection;
            }
            font_id = select_font(self, font);
        };
        let font_id = font_id?;
        self.font_to_font_id.insert(font.clone(), font_id);
        Some(font_id)
    }

    fn font_id_for_native_face(
        &mut self, components: &DirectWriteComponents, face: &IDWriteFontFace3, source: FontId,
    ) -> windows::core::Result<FontId> {
        let source = self.fonts.get(source.0)
            .ok_or_else(|| Error::new(E_INVALIDARG, "Invalid source font in DirectWrite glyph run"))?;
        let key = NativeFontKey::new(face, &source.features, source.fallbacks.as_ref(), source.style)?;
        if let Some(id) = self.font_info_cache.get(&key) { return Ok(*id); }

        let (raster_face, shear) = authored_oblique_rendering(face, source.style)?;
        let axes = native_font_axes(raster_face.as_ref().unwrap_or(face))?;
        // A singleton collection keeps later shaping on this native face, even
        // when the source collection did not contain the system fallback.
        let (family, collection) = unsafe {
            let family = get_name(face.GetFamilyNames()?, &components.locale)
                .map_err(|error| Error::new(DWRITE_E_NOFONT, error.to_string()))?;
            let native_axes = native_font_axes(face)?;
            let reference = if native_axes.is_empty() {
                face.GetFontFaceReference()?
            } else {
                face.cast::<IDWriteFontFace5>()?.GetFontResource()?
                    .CreateFontFaceReference(face.GetSimulations(), &native_axes)?.cast()?
            };
            let builder = components.factory.CreateFontSetBuilder()?;
            builder.AddFontFaceReference2(&reference)?;
            let set = builder.CreateFontSet()?;
            (family, components.factory.CreateFontCollectionFromFontSet(&set)?)
        };
        let info = FontInfo {
            font_family_h: HSTRING::from(family), font_face: face.clone(),
            features: source.features.clone(), fallbacks: source.fallbacks.clone(),
            font_collection: collection, axes, style: source.style, raster_face, shear,
        };
        // Retaining the exact face also keeps the cached COM identity alive.
        // Never alias an unowned native address to a newly selected face.
        let id = FontId(self.fonts.len());
        self.fonts.push(info);
        self.font_info_cache.insert(key, id);
        Ok(id)
    }

    fn add_fonts(
        &mut self,
        components: &DirectWriteComponents,
        fonts: Vec<Cow<'static, [u8]>>,
    ) -> Result<()> {
        for font_data in fonts {
            match font_data {
                Cow::Borrowed(data) => unsafe {
                    let font_file = components
                        .in_memory_loader
                        .CreateInMemoryFontFileReference(
                            &components.factory,
                            data.as_ptr().cast(),
                            data.len() as _,
                            None,
                        )?;
                    components.builder.AddFontFile(&font_file)?;
                },
                Cow::Owned(data) => unsafe {
                    let font_file = components
                        .in_memory_loader
                        .CreateInMemoryFontFileReference(
                            &components.factory,
                            data.as_ptr().cast(),
                            data.len() as _,
                            None,
                        )?;
                    components.builder.AddFontFile(&font_file)?;
                },
            }
        }
        let set = unsafe { components.builder.CreateFontSet()? };
        let collection = unsafe { components.factory.CreateFontCollectionFromFontSet(&set)? };
        self.custom_font_collection = collection;

        Ok(())
    }

    fn generate_font_fallbacks(
        fallbacks: &FontFallbacks,
        factory: &IDWriteFactory5,
        system_font_collection: &IDWriteFontCollection1,
    ) -> Result<Option<IDWriteFontFallback>> {
        let fallback_list = fallbacks.fallback_list();
        if fallback_list.is_empty() {
            return Ok(None);
        }
        unsafe {
            let builder = factory.CreateFontFallbackBuilder()?;
            let font_set = &system_font_collection.GetFontSet()?;
            let mut unicode_ranges = Vec::new();
            for family_name in fallback_list {
                let family_name = HSTRING::from(family_name);
                let Some(fonts) = font_set
                    .GetMatchingFonts(
                        &family_name,
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                    )
                    .log_err()
                else {
                    continue;
                };
                let Ok(font_face) = fonts.GetFontFaceReference(0) else {
                    continue;
                };
                let font = font_face.CreateFontFace()?;
                let mut count = 0;
                font.GetUnicodeRanges(None, &mut count).ok();
                if count == 0 {
                    continue;
                }
                unicode_ranges.clear();
                unicode_ranges.resize_with(count as usize, DWRITE_UNICODE_RANGE::default);
                let Some(_) = font
                    .GetUnicodeRanges(Some(&mut unicode_ranges), &mut count)
                    .log_err()
                else {
                    continue;
                };
                builder.AddMapping(
                    &unicode_ranges,
                    &[family_name.as_ptr()],
                    None,
                    None,
                    None,
                    1.0,
                )?;
            }
            let system_fallbacks = factory.GetSystemFontFallback()?;
            builder.AddMappings(&system_fallbacks)?;
            Ok(Some(builder.CreateFontFallback()?))
        }
    }

    unsafe fn generate_font_features(
        factory: &IDWriteFactory5,
        font_features: &FontFeatures,
    ) -> Result<IDWriteTypography> {
        let direct_write_features = unsafe { factory.CreateTypography()? };
        apply_font_features(&direct_write_features, font_features)?;
        Ok(direct_write_features)
    }

    unsafe fn make_font_from_font_collection(
        &Font {
            ref family,
            ref features,
            ref fallbacks,
            weight,
            style,
        }: &Font,
        collection: &IDWriteFontCollection1,
        factory: &IDWriteFactory5,
        system_font_collection: &IDWriteFontCollection1,
        system_ui_font_name: &SharedString,
    ) -> Option<FontInfo> {
        const SYSTEM_UI_FONT_NAME: &str = ".SystemUIFont";
        let family = if family == SYSTEM_UI_FONT_NAME {
            system_ui_font_name
        } else {
            gpui::font_name_with_fallbacks_shared(&family, &system_ui_font_name)
        };
        let fontset = unsafe { collection.GetFontSet().log_err()? };
        let font_family_h = HSTRING::from(family.as_str());
        let font = MatchingFonts::new(&fontset, &font_family_h, weight, style).log_err()?;
        let total_number = font.len();
        for index in 0..total_number {
            let res = maybe!({
                let font_face = font.face(index).log_err()?;
                let (raster_face, shear) = authored_oblique_rendering(&font_face, style).log_err()?;
                let axes = native_font_axes(raster_face.as_ref().unwrap_or(&font_face)).log_err()?;
                let direct_write_features =
                    unsafe { Self::generate_font_features(factory, features).log_err()? };
                let fallbacks = match fallbacks {
                    Some(fallbacks) => Self::generate_font_fallbacks(fallbacks, factory, system_font_collection).log_err()?,
                    None => None,
                };
                let font_info = FontInfo {
                    font_family_h: font_family_h.clone(),
                    font_face,
                    features: direct_write_features,
                    fallbacks,
                    font_collection: collection.clone(),
                    axes,
                    style, raster_face, shear,
                };
                Some(font_info)
            });
            if res.is_some() {
                return res;
            }
        }
        None
    }

    fn layout_line(
        &mut self,
        components: &DirectWriteComponents,
        text: &str,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
    ) -> Result<LineLayout> {
        if !text.contains(is_line_separator) {
            return self.layout_line_without_separators(components, text, font_size, font_runs, direction);
        }

        // DirectWrite honors mandatory breaks even without wrapping. Keep each
        // separator at a segment's end so no subsequent text moves to a new baseline.
        let mut layout = LineLayout {
            font_size,
            len: text.len(),
            ..Default::default()
        };
        let mut remaining_runs = font_runs;
        let mut run_offset = 0;
        let mut text_offset = 0;
        let mut segment_runs = SmallVec::<[FontRun; 4]>::new();
        for segment in text.split_inclusive(is_line_separator) {
            segment_runs.clear();
            let mut remaining_bytes = segment.len();
            while remaining_bytes > 0 {
                let Some(run) = remaining_runs.first() else {
                    break;
                };
                let len = remaining_bytes.min(run.len - run_offset);
                if len > 0 {
                    segment_runs.push(FontRun { font_size: run.font_size,
                        len,
                        font_id: run.font_id,
                    });
                }
                remaining_bytes -= len;
                run_offset += len;
                if run_offset == run.len {
                    remaining_runs = &remaining_runs[1..];
                    run_offset = 0;
                }
            }

            let segment_layout =
                self.layout_line_without_separators(components, segment, font_size, &segment_runs, direction)?;
            layout.interior_carets.extend(segment_layout.interior_carets.into_iter().map(|mut caret| {
                caret.index += text_offset;
                caret.cluster_start += text_offset;
                caret.x += layout.width;
                caret
            }));
            for mut run in segment_layout.runs {
                for glyph in &mut run.glyphs {
                    glyph.index += text_offset;
                    glyph.position.x += layout.width;
                }
                if let Some(last) = layout.runs.last_mut()
                    && last.font_id == run.font_id
                    && last.font_size == run.font_size
                {
                    last.glyphs.append(&mut run.glyphs);
                } else {
                    layout.runs.push(run);
                }
            }
            layout.width += segment_layout.width;
            layout.ascent = layout.ascent.max(segment_layout.ascent);
            layout.descent = layout.descent.max(segment_layout.descent);
            text_offset += segment.len();
        }
        Ok(layout)
    }

    fn layout_line_without_separators(
        &mut self,
        components: &DirectWriteComponents,
        text: &str,
        font_size: Pixels,
        font_runs: &[FontRun],
        direction: TextDirection,
    ) -> Result<LineLayout> {
        if font_runs.is_empty() {
            return Ok(LineLayout {
                font_size,
                ..Default::default()
            });
        }
        unsafe {
            self.layout_line_scratch.clear();
            self.layout_line_scratch.extend(text.encode_utf16());
            let text_wide = &*self.layout_line_scratch;

            let mut utf8_offset = 0usize;
            let mut utf16_offset = 0u32;
            let text_layout = {
                let first_run = &font_runs[0];
                let font_info = &self.fonts[first_run.font_id.0];
                let collection = &font_info.font_collection;
                let format: IDWriteTextFormat1 = components
                    .factory
                    .CreateTextFormat(
                        &font_info.font_family_h,
                        collection,
                        font_info.font_face.GetWeight(),
                        font_info.font_face.GetStyle(),
                        DWRITE_FONT_STRETCH_NORMAL,
                        first_run.font_size.unwrap_or(font_size).as_f32(),
                        &components.locale,
                    )?
                    .cast()?;
                if let Some(ref fallbacks) = font_info.fallbacks {
                    format.SetFontFallback(fallbacks)?;
                }
                let is_rtl = direction.resolve(text) == TextDirection::Rtl;
                format.SetReadingDirection(if is_rtl {
                    DWRITE_READING_DIRECTION_RIGHT_TO_LEFT
                } else {
                    DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
                })?;
                if is_rtl {
                    // Measure at the physical left edge first. Leading RTL
                    // alignment against an unbounded box has no finite origin.
                    format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
                }
                // GPUI owns wrapping. Keep this native layout on one line even
                // after reducing its alignment box to the measured width.
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;

                let layout = components.factory.CreateTextLayout(
                    text_wide,
                    &format,
                    f32::INFINITY,
                    f32::INFINITY,
                )?;
                let current_text = &text[utf8_offset..(utf8_offset + first_run.len)];
                utf8_offset += first_run.len;
                let current_text_utf16_length = current_text.encode_utf16().count() as u32;
                let text_range = DWRITE_TEXT_RANGE {
                    startPosition: utf16_offset,
                    length: current_text_utf16_length,
                };
                layout.SetTypography(&font_info.features, text_range)?;
                font_info.set_axes(&layout, text_range)?;
                utf16_offset += current_text_utf16_length;

                layout
            };

            let mut break_ligatures = true;
            for run in &font_runs[1..] {
                let font_info = &self.fonts[run.font_id.0];
                let current_text = &text[utf8_offset..(utf8_offset + run.len)];
                utf8_offset += run.len;
                let current_text_utf16_length = current_text.encode_utf16().count() as u32;

                let collection = &font_info.font_collection;
                let text_range = DWRITE_TEXT_RANGE {
                    startPosition: utf16_offset,
                    length: current_text_utf16_length,
                };
                utf16_offset += current_text_utf16_length;
                text_layout.SetFontCollection(collection, text_range)?;
                text_layout.SetFontFamilyName(&font_info.font_family_h, text_range)?;
                let run_size = run.font_size.unwrap_or(font_size).as_f32();
                let font_size = if break_ligatures {
                    run_size.next_up()
                } else {
                    run_size
                };
                text_layout.SetFontSize(font_size, text_range)?;
                text_layout.SetFontStyle(font_info.font_face.GetStyle(), text_range)?;
                text_layout.SetFontWeight(font_info.font_face.GetWeight(), text_range)?;
                text_layout.SetTypography(&font_info.features, text_range)?;
                font_info.set_axes(&text_layout, text_range)?;

                break_ligatures = !break_ligatures;
            }

            let mut metrics = DWRITE_TEXT_METRICS::default();
            text_layout.GetMetrics(&mut metrics)?;
            let width = px(metrics.widthIncludingTrailingWhitespace);
            // RTL leading alignment against infinity would produce invalid
            // glyph origins. Use the actual extent, including trailing spaces;
            // source positions and the shaper's bidi runs remain untouched.
            text_layout.SetMaxWidth(metrics.widthIncludingTrailingWhitespace.max(f32::MIN_POSITIVE))?;
            text_layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
            let (ascent, descent) = {
                let mut first_metrics = [DWRITE_LINE_METRICS::default(); 4];
                let mut line_count = 0u32;
                text_layout.GetLineMetrics(Some(&mut first_metrics), &mut line_count)?;
                (
                    px(first_metrics[0].baseline),
                    px(first_metrics[0].height - first_metrics[0].baseline),
                )
            };
            let mut runs = Vec::new();
            let mut renderer_context = RendererContext {
                text_system: self,
                components,
                index_converter: StringIndexConverter::new(text),
                runs: &mut runs,
                baseline: ascent.as_f32(),
                sources: SourceFontRuns::new(font_runs), font_size,
            };
            text_layout.Draw(
                Some((&raw mut renderer_context).cast::<c_void>().cast_const()),
                &components.text_renderer.0,
                0.0,
                0.0,
            )?;
            if runs.windows(2).any(|pair| pair[0].glyphs[0].position.x > pair[1].glyphs[0].position.x) {
                runs.sort_by(|left, right| left.glyphs[0].position.x.as_f32().total_cmp(&right.glyphs[0].position.x.as_f32()));
            }
            let mut layout = LineLayout {
                font_size,
                width,
                ascent,
                descent,
                runs,
                len: text.len(),
                interior_carets: Vec::new(),
                isolation: None,
                ink_clip: None,
            };
            let mut carets = layout.interior_caret_candidates(text);
            let mut converter = StringIndexConverter::new(text);
            for caret in &mut carets {
                converter.advance_to_utf8_ix(caret.index);
                let mut x = 0.0;
                let mut y = 0.0;
                let mut hit = DWRITE_HIT_TEST_METRICS::default();
                text_layout.HitTestTextPosition(
                    converter.utf16_ix as u32,
                    false,
                    &mut x,
                    &mut y,
                    &mut hit,
                )?;
                anyhow::ensure!(x.is_finite(), "DirectWrite returned a non-finite ligature caret");
                caret.x = px(x);
            }
            layout.interior_carets = carets;
            Ok(layout)
        }
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        unsafe {
            let font_info = &self.fonts[font_id.0];
            let mut metrics = std::mem::zeroed();
            font_info.drawing_face().GetMetrics(&mut metrics);

            FontMetrics {
                units_per_em: metrics.Base.designUnitsPerEm as _,
                ascent: metrics.Base.ascent as _,
                descent: -(metrics.Base.descent as f32),
                line_gap: metrics.Base.lineGap as _,
                underline_position: metrics.Base.underlinePosition as _,
                underline_thickness: metrics.Base.underlineThickness as _,
                cap_height: metrics.Base.capHeight as _,
                x_height: metrics.Base.xHeight as _,
                bounding_box: shear_typographic_bounds(Bounds {
                    origin: Point {
                        x: metrics.glyphBoxLeft as _,
                        y: metrics.glyphBoxBottom as _,
                    },
                    size: Size {
                        width: (metrics.glyphBoxRight - metrics.glyphBoxLeft) as _,
                        height: (metrics.glyphBoxTop - metrics.glyphBoxBottom) as _,
                    },
                }, font_info.shear),
            }
        }
    }

    fn create_glyph_run_analysis(
        &self,
        components: &DirectWriteComponents,
        params: &RenderGlyphParams,
    ) -> Result<IDWriteGlyphRunAnalysis> {
        let font = &self.fonts[params.font_id.0];
        anyhow::ensure!(font.shear == 0. || oblique_degrees(font.style).is_none_or(|angle| angle.abs() != 90.),
            "singular synthetic oblique angle has no finite glyph raster");
        let glyph_id = [params.glyph_id.0 as u16];
        let advance = [0.0];
        let offset = [DWRITE_GLYPH_OFFSET::default()];
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&****font.drawing_face()) })),
            fontEmSize: params.font_size.as_f32(),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advance.as_ptr(),
            glyphOffsets: offset.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        let transform = DWRITE_MATRIX {
            m11: params.scale_factor,
            m12: 0.0,
            m21: -font.shear * params.scale_factor,
            m22: params.scale_factor,
            dx: 0.0,
            dy: 0.0,
        };
        let baseline_origin_y = params.subpixel_variant.y as f32
            / gpui::SUBPIXEL_VARIANTS_Y as f32
            / params.scale_factor;
        let baseline_origin_x = params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32
            / params.scale_factor + font.shear * baseline_origin_y;

        let mut rendering_mode = DWRITE_RENDERING_MODE1::default();
        let mut grid_fit_mode = DWRITE_GRID_FIT_MODE::default();
        unsafe {
            font.drawing_face().GetRecommendedRenderingMode(
                params.font_size.as_f32(),
                // Using 96 as scale is applied by the transform
                96.0,
                96.0,
                Some(&transform),
                false,
                DWRITE_OUTLINE_THRESHOLD_ANTIALIASED,
                DWRITE_MEASURING_MODE_NATURAL,
                None,
                &mut rendering_mode,
                &mut grid_fit_mode,
            )?;
        }
        let rendering_mode = match rendering_mode {
            DWRITE_RENDERING_MODE1_OUTLINE => DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
            m => m,
        };

        let antialias_mode = if params.subpixel_rendering {
            DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE
        } else {
            DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE
        };

        let glyph_analysis = unsafe {
            components.factory.CreateGlyphRunAnalysis(
                &glyph_run,
                Some(&transform),
                rendering_mode,
                DWRITE_MEASURING_MODE_NATURAL,
                grid_fit_mode,
                antialias_mode,
                baseline_origin_x,
                baseline_origin_y,
            )
        }?;
        Ok(glyph_analysis)
    }

    fn raster_bounds(
        &self,
        components: &DirectWriteComponents,
        params: &RenderGlyphParams,
    ) -> Result<Bounds<DevicePixels>> {
        let glyph_analysis = self.create_glyph_run_analysis(components, params)?;

        let texture_type = if params.subpixel_rendering {
            DWRITE_TEXTURE_CLEARTYPE_3x1
        } else {
            DWRITE_TEXTURE_ALIASED_1x1
        };

        let bounds = unsafe { glyph_analysis.GetAlphaTextureBounds(texture_type)? };

        if bounds.right < bounds.left || bounds.bottom < bounds.top {
            Ok(Bounds {
                origin: point(0.into(), 0.into()),
                size: size(0.into(), 0.into()),
            })
        } else {
            let width = i64::from(bounds.right) - i64::from(bounds.left);
            let height = i64::from(bounds.bottom) - i64::from(bounds.top);
            check_raster_size(width, height)?;
            Ok(Bounds {
                origin: point(bounds.left.into(), bounds.top.into()),
                size: size(
                    (width as i32).into(),
                    (height as i32).into(),
                ),
            })
        }
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        let font_info = &self.fonts[font_id.0];
        let codepoints = ch as u32;
        let mut glyph_indices = 0u16;
        unsafe {
            font_info
                .font_face
                .GetGlyphIndices(&raw const codepoints, 1, &raw mut glyph_indices)
                .log_err()
        }
        .map(|_| GlyphId(glyph_indices as u32))
    }

    fn rasterize_glyph(
        &self,
        components: &DirectWriteComponents,
        params: &RenderGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        }
        check_raster_size(i64::from(glyph_bounds.size.width.0), i64::from(glyph_bounds.size.height.0))?;
        anyhow::ensure!(glyph_bounds.origin.x.0.checked_add(glyph_bounds.size.width.0).is_some()
            && glyph_bounds.origin.y.0.checked_add(glyph_bounds.size.height.0).is_some(), "glyph raster bounds overflow");

        let bitmap_data = if params.is_emoji {
            if let Ok(color) = self.rasterize_color(components, params, glyph_bounds) {
                color
            } else {
                let monochrome = self.rasterize_monochrome(components, params, glyph_bounds)?;
                monochrome
                    .into_iter()
                    .flat_map(|pixel| [0, 0, 0, pixel])
                    .collect::<Vec<_>>()
            }
        } else {
            self.rasterize_monochrome(components, params, glyph_bounds)?
        };

        Ok((glyph_bounds.size, bitmap_data))
    }

    fn rasterize_monochrome(
        &self,
        components: &DirectWriteComponents,
        params: &RenderGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<Vec<u8>> {
        let glyph_analysis = self.create_glyph_run_analysis(components, params)?;
        if !params.subpixel_rendering {
            let mut bitmap_data =
                vec![0u8; glyph_bounds.size.width.0 as usize * glyph_bounds.size.height.0 as usize];
            unsafe {
                glyph_analysis.CreateAlphaTexture(
                    DWRITE_TEXTURE_ALIASED_1x1,
                    &RECT {
                        left: glyph_bounds.origin.x.0,
                        top: glyph_bounds.origin.y.0,
                        right: glyph_bounds.size.width.0 + glyph_bounds.origin.x.0,
                        bottom: glyph_bounds.size.height.0 + glyph_bounds.origin.y.0,
                    },
                    &mut bitmap_data,
                )?;
            }

            return Ok(bitmap_data);
        }

        let width = glyph_bounds.size.width.0 as usize;
        let height = glyph_bounds.size.height.0 as usize;
        let pixel_count = width * height;

        let mut bitmap_data = vec![0u8; pixel_count * 4];

        unsafe {
            glyph_analysis.CreateAlphaTexture(
                DWRITE_TEXTURE_CLEARTYPE_3x1,
                &RECT {
                    left: glyph_bounds.origin.x.0,
                    top: glyph_bounds.origin.y.0,
                    right: glyph_bounds.size.width.0 + glyph_bounds.origin.x.0,
                    bottom: glyph_bounds.size.height.0 + glyph_bounds.origin.y.0,
                },
                &mut bitmap_data[..pixel_count * 3],
            )?;
        }

        // The output buffer expects RGBA data, so pad the alpha channel with zeros.
        for pixel_ix in (0..pixel_count).rev() {
            let src = pixel_ix * 3;
            let dst = pixel_ix * 4;
            (
                bitmap_data[dst],
                bitmap_data[dst + 1],
                bitmap_data[dst + 2],
                bitmap_data[dst + 3],
            ) = (
                bitmap_data[src],
                bitmap_data[src + 1],
                bitmap_data[src + 2],
                0,
            );
        }

        Ok(bitmap_data)
    }

    fn rasterize_color(
        &self,
        components: &DirectWriteComponents,
        params: &RenderGlyphParams,
        glyph_bounds: Bounds<DevicePixels>,
    ) -> Result<Vec<u8>> {
        // INVARIANT: the code below drives the *shared* D3D11 immediate context
        // (`Map`/`Unmap`/`Draw`/`CopyResource`), which `DirectXRenderer` and `DirectXAtlas` also
        // touch. An immediate `ID3D11DeviceContext` is not thread-safe, so this must only run on
        // the main UI thread (which it always is; text rasterization never leaves that thread).
        let gpu_state = self
            .gpu_state
            .as_ref()
            .context("rasterizing color glyphs needs a GPU, which headless mode has none of")?;
        let bitmap_size = glyph_bounds.size;
        let subpixel_shift = params
            .subpixel_variant
            .map(|v| v as f32 / SUBPIXEL_VARIANTS_X as f32);
        let font = &self.fonts[params.font_id.0];
        let baseline_origin_y = subpixel_shift.y / params.scale_factor;
        let baseline_origin_x = subpixel_shift.x / params.scale_factor + font.shear * baseline_origin_y;

        let transform = DWRITE_MATRIX {
            m11: params.scale_factor,
            m12: 0.0,
            m21: -font.shear * params.scale_factor,
            m22: params.scale_factor,
            dx: 0.0,
            dy: 0.0,
        };

        let glyph_id = [params.glyph_id.0 as u16];
        let advance = [glyph_bounds.size.width.0 as f32];
        let offset = [DWRITE_GLYPH_OFFSET {
            advanceOffset: (-glyph_bounds.origin.x.0 as f32 - font.shear * glyph_bounds.origin.y.0 as f32) / params.scale_factor,
            ascenderOffset: glyph_bounds.origin.y.0 as f32 / params.scale_factor,
        }];
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&****font.drawing_face()) })),
            fontEmSize: params.font_size.as_f32(),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advance.as_ptr(),
            glyphOffsets: offset.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };

        // todo: support formats other than COLR
        let color_enumerator = unsafe {
            components.factory.TranslateColorGlyphRun(
                Vector2::new(baseline_origin_x, baseline_origin_y),
                &glyph_run,
                None,
                DWRITE_GLYPH_IMAGE_FORMATS_COLR,
                DWRITE_MEASURING_MODE_NATURAL,
                Some(&transform),
                0,
            )
        }?;

        let mut glyph_layers = Vec::new();
        let mut alpha_data = Vec::new();
        loop {
            let color_run = unsafe { color_enumerator.GetCurrentRun() }?;
            let color_run = unsafe { &*color_run };
            let image_format = color_run.glyphImageFormat & !DWRITE_GLYPH_IMAGE_FORMATS_TRUETYPE;
            if image_format == DWRITE_GLYPH_IMAGE_FORMATS_COLR {
                let color_analysis = unsafe {
                    components.factory.CreateGlyphRunAnalysis(
                        &color_run.Base.glyphRun as *const _,
                        Some(&transform),
                        DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                        DWRITE_MEASURING_MODE_NATURAL,
                        DWRITE_GRID_FIT_MODE_DEFAULT,
                        DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                        baseline_origin_x,
                        baseline_origin_y,
                    )
                }?;

                let color_bounds =
                    unsafe { color_analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1) }?;

                let color_size = size(
                    color_bounds.right - color_bounds.left,
                    color_bounds.bottom - color_bounds.top,
                );
                if color_size.width > 0 && color_size.height > 0 {
                    alpha_data.clear();
                    alpha_data.resize((color_size.width * color_size.height) as usize, 0);
                    unsafe {
                        color_analysis.CreateAlphaTexture(
                            DWRITE_TEXTURE_ALIASED_1x1,
                            &color_bounds,
                            &mut alpha_data,
                        )
                    }?;

                    let run_color = {
                        let run_color = color_run.Base.runColor;
                        Rgba {
                            r: run_color.r,
                            g: run_color.g,
                            b: run_color.b,
                            a: run_color.a,
                        }
                    };
                    let bounds = bounds(point(color_bounds.left, color_bounds.top), color_size);
                    glyph_layers.push(GlyphLayerTexture::new(
                        gpu_state,
                        run_color,
                        bounds,
                        &alpha_data,
                    )?);
                }
            }

            let has_next = unsafe { color_enumerator.MoveNext() }
                .map(|e| e.as_bool())
                .unwrap_or(false);
            if !has_next {
                break;
            }
        }

        let render_target_texture = {
            let mut texture = None;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: bitmap_size.width.0 as u32,
                Height: bitmap_size.height.0 as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            unsafe {
                gpu_state
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
            }?;
            texture.unwrap()
        };

        let render_target_view = {
            let desc = D3D11_RENDER_TARGET_VIEW_DESC {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
                },
            };
            let mut rtv = None;
            unsafe {
                gpu_state.device.CreateRenderTargetView(
                    &render_target_texture,
                    Some(&desc),
                    Some(&mut rtv),
                )
            }?;
            rtv
        };

        Self::composite_color_layers(
            gpu_state,
            &glyph_layers,
            bitmap_size,
            &render_target_texture,
            &render_target_view,
        )
    }

    fn composite_color_layers(
        gpu_state: &GPUState,
        glyph_layers: &[GlyphLayerTexture],
        bitmap_size: Size<DevicePixels>,
        render_target_texture: &ID3D11Texture2D,
        render_target_view: &Option<ID3D11RenderTargetView>,
    ) -> Result<Vec<u8>> {
        let params_buffer = {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: std::mem::size_of::<GlyphLayerTextureParams>() as u32,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
                StructureByteStride: 0,
            };

            let mut buffer = None;
            unsafe {
                gpu_state
                    .device
                    .CreateBuffer(&desc, None, Some(&mut buffer))
            }?;
            buffer
        };

        let staging_texture = {
            let mut texture = None;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: bitmap_size.width.0 as u32,
                Height: bitmap_size.height.0 as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            unsafe {
                gpu_state
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
            }?;
            texture.unwrap()
        };

        let device_context = &gpu_state.device_context;
        unsafe { device_context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP) };
        unsafe { device_context.VSSetShader(&gpu_state.vertex_shader, None) };
        unsafe { device_context.PSSetShader(&gpu_state.pixel_shader, None) };
        unsafe {
            device_context.VSSetConstantBuffers(0, Some(std::slice::from_ref(&params_buffer)))
        };
        unsafe {
            device_context.PSSetConstantBuffers(0, Some(std::slice::from_ref(&params_buffer)))
        };
        unsafe {
            device_context.OMSetRenderTargets(Some(std::slice::from_ref(render_target_view)), None)
        };
        unsafe {
            if let Some(render_target_view) = render_target_view.as_ref() {
                device_context.ClearRenderTargetView(render_target_view, &[0.0, 0.0, 0.0, 0.0]);
            }
        }
        unsafe { device_context.PSSetSamplers(0, Some(std::slice::from_ref(&gpu_state.sampler))) };
        unsafe { device_context.OMSetBlendState(&gpu_state.blend_state, None, 0xffffffff) };

        let crate::FontInfo {
            gamma_ratios,
            grayscale_enhanced_contrast,
            ..
        } = DirectXRenderer::get_font_info();

        for layer in glyph_layers {
            let params = GlyphLayerTextureParams {
                run_color: layer.run_color,
                bounds: layer.bounds,
                gamma_ratios: *gamma_ratios,
                grayscale_enhanced_contrast: *grayscale_enhanced_contrast,
                _pad: [0f32; 3],
            };
            unsafe {
                let mut dest = std::mem::zeroed();
                gpu_state.device_context.Map(
                    params_buffer.as_ref().unwrap(),
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut dest),
                )?;
                std::ptr::copy_nonoverlapping(&params as *const _, dest.pData as *mut _, 1);
                gpu_state
                    .device_context
                    .Unmap(params_buffer.as_ref().unwrap(), 0);
            };

            let texture = [Some(layer.texture_view.clone())];
            unsafe { device_context.PSSetShaderResources(0, Some(&texture)) };

            let viewport = [D3D11_VIEWPORT {
                TopLeftX: layer.bounds.origin.x as f32,
                TopLeftY: layer.bounds.origin.y as f32,
                Width: layer.bounds.size.width as f32,
                Height: layer.bounds.size.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }];
            unsafe { device_context.RSSetViewports(Some(&viewport)) };

            unsafe { device_context.Draw(4, 0) };
        }

        unsafe { device_context.CopyResource(&staging_texture, render_target_texture) };

        let mapped_data = {
            let mut mapped_data = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                device_context.Map(
                    &staging_texture,
                    0,
                    D3D11_MAP_READ,
                    0,
                    Some(&mut mapped_data),
                )
            }?;
            mapped_data
        };
        let mut rasterized =
            vec![0u8; (bitmap_size.width.0 as u32 * bitmap_size.height.0 as u32 * 4) as usize];

        for y in 0..bitmap_size.height.0 as usize {
            let width = bitmap_size.width.0 as usize;
            unsafe {
                std::ptr::copy_nonoverlapping::<u8>(
                    (mapped_data.pData as *const u8).byte_add(mapped_data.RowPitch as usize * y),
                    rasterized
                        .as_mut_ptr()
                        .byte_add(width * y * std::mem::size_of::<u32>()),
                    width * std::mem::size_of::<u32>(),
                )
            };
        }

        // Release the mapping now that the rows have been copied out; leaving `staging_texture`
        // mapped would leak the mapping and keep the resource pinned for later reuse.
        unsafe { device_context.Unmap(&staging_texture, 0) };

        // Convert from premultiplied to straight alpha
        for chunk in rasterized.chunks_exact_mut(4) {
            let b = chunk[0] as f32;
            let g = chunk[1] as f32;
            let r = chunk[2] as f32;
            let a = chunk[3] as f32;
            if a > 0.0 {
                let inv_a = 255.0 / a;
                chunk[0] = (b * inv_a).clamp(0.0, 255.0) as u8;
                chunk[1] = (g * inv_a).clamp(0.0, 255.0) as u8;
                chunk[2] = (r * inv_a).clamp(0.0, 255.0) as u8;
            }
        }

        Ok(rasterized)
    }

    fn get_typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        unsafe {
            let info = &self.fonts[font_id.0];
            let font = info.drawing_face();
            let glyph_indices = [glyph_id.0 as u16];
            let mut metrics = [DWRITE_GLYPH_METRICS::default()];
            font.GetDesignGlyphMetrics(glyph_indices.as_ptr(), 1, metrics.as_mut_ptr(), false)?;

            let metrics = &metrics[0];
            let advance_width = metrics.advanceWidth as i32;
            let advance_height = metrics.advanceHeight as i32;
            let left_side_bearing = metrics.leftSideBearing;
            let right_side_bearing = metrics.rightSideBearing;
            let top_side_bearing = metrics.topSideBearing;
            let bottom_side_bearing = metrics.bottomSideBearing;
            let vertical_origin_y = metrics.verticalOriginY;

            let y_offset = vertical_origin_y + bottom_side_bearing - advance_height;
            let width = advance_width - (left_side_bearing + right_side_bearing);
            let height = advance_height - (top_side_bearing + bottom_side_bearing);

            Ok(shear_typographic_bounds(Bounds {
                origin: Point {
                    x: left_side_bearing as f32,
                    y: y_offset as f32,
                },
                size: Size {
                    width: width as f32,
                    height: height as f32,
                },
            }, info.shear))
        }
    }

    fn get_advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        unsafe {
            let font = self.fonts[font_id.0].drawing_face();
            let glyph_indices = [glyph_id.0 as u16];
            let mut metrics = [DWRITE_GLYPH_METRICS::default()];
            font.GetDesignGlyphMetrics(glyph_indices.as_ptr(), 1, metrics.as_mut_ptr(), false)?;

            let metrics = &metrics[0];

            Ok(Size {
                width: metrics.advanceWidth as f32,
                height: 0.0,
            })
        }
    }

    fn all_font_names(&self, components: &DirectWriteComponents) -> Vec<String> {
        let mut result =
            get_font_names_from_collection(&self.system_font_collection, &components.locale);
        result.extend(get_font_names_from_collection(
            &self.custom_font_collection,
            &components.locale,
        ));
        result
    }

    fn handle_gpu_lost(&mut self, directx_devices: &DirectXDevices) -> Result<()> {
        try_to_recover_from_device_lost(|| {
            GPUState::new(directx_devices).context("Recreating GPU state for DirectWrite")
        })
        .map(|gpu_state| self.gpu_state = Some(gpu_state))
    }
}

struct GlyphLayerTexture {
    run_color: Rgba,
    bounds: Bounds<i32>,
    texture_view: ID3D11ShaderResourceView,
    // holding on to the texture to not RAII drop it
    _texture: ID3D11Texture2D,
}

impl GlyphLayerTexture {
    fn new(
        gpu_state: &GPUState,
        run_color: Rgba,
        bounds: Bounds<i32>,
        alpha_data: &[u8],
    ) -> Result<Self> {
        let texture_size = bounds.size;

        let desc = D3D11_TEXTURE2D_DESC {
            Width: texture_size.width as u32,
            Height: texture_size.height as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        let texture = {
            let mut texture: Option<ID3D11Texture2D> = None;
            unsafe {
                gpu_state
                    .device
                    .CreateTexture2D(&desc, None, Some(&mut texture))?
            };
            texture.unwrap()
        };
        let texture_view = {
            let mut view: Option<ID3D11ShaderResourceView> = None;
            unsafe {
                gpu_state
                    .device
                    .CreateShaderResourceView(&texture, None, Some(&mut view))?
            };
            view.unwrap()
        };

        unsafe {
            gpu_state.device_context.UpdateSubresource(
                &texture,
                0,
                None,
                alpha_data.as_ptr() as _,
                texture_size.width as u32,
                0,
            )
        };

        Ok(GlyphLayerTexture {
            run_color,
            bounds,
            texture_view,
            _texture: texture,
        })
    }
}

#[repr(C)]
struct GlyphLayerTextureParams {
    bounds: Bounds<i32>,
    run_color: Rgba,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    _pad: [f32; 3],
}

#[implement(ID2D1SimplifiedGeometrySink)]
struct GlyphInkSink { ink: RefCell<GlyphInk>, shear: f32 }

impl GlyphInkSink {
    fn position(&self, p: &Vector2) -> Point<f32> { point(p.X - self.shear * p.Y, p.Y) }
}

impl ID2D1SimplifiedGeometrySink_Impl for GlyphInkSink_Impl {
    fn SetFillMode(&self, _: D2D1_FILL_MODE) {}
    fn SetSegmentFlags(&self, _: D2D1_PATH_SEGMENT) {}
    fn BeginFigure(&self, start: &Vector2, _: D2D1_FIGURE_BEGIN) {
        self.ink.borrow_mut().move_to(self.position(start));
    }
    fn AddLines(&self, points: *const Vector2, count: u32) {
        if count == 0 { return; }
        let mut ink = self.ink.borrow_mut();
        // DirectWrite owns this nonempty array for the synchronous callback.
        for p in unsafe { std::slice::from_raw_parts(points, count as usize) } {
            ink.line_to(self.position(p));
        }
    }
    fn AddBeziers(&self, segments: *const D2D1_BEZIER_SEGMENT, count: u32) {
        if count == 0 { return; }
        let mut ink = self.ink.borrow_mut();
        // DirectWrite owns this nonempty array for the synchronous callback.
        for p in unsafe { std::slice::from_raw_parts(segments, count as usize) } {
            ink.curve_to(self.position(&p.point1), self.position(&p.point2), self.position(&p.point3));
        }
    }
    fn EndFigure(&self, _: D2D1_FIGURE_END) { self.ink.borrow_mut().close(); }
    fn Close(&self) -> windows::core::Result<()> { Ok(()) }
}

struct TextRendererWrapper(IDWriteTextRenderer);

impl TextRendererWrapper {
    fn new() -> Self {
        TextRendererWrapper(TextRenderer {}.into())
    }
}

#[implement(IDWriteTextRenderer)]
struct TextRenderer {}

struct SourceFontRuns<'a> { runs: &'a [FontRun], ends: SmallVec<[usize; 4]> }

impl<'a> SourceFontRuns<'a> {
    fn new(runs: &'a [FontRun]) -> Self {
        let mut ends = SmallVec::new();
        if runs.len() > 1 {
            let mut end = 0;
            for run in runs { end += run.len; ends.push(end); }
        }
        Self { runs, ends }
    }
    fn at(&self, index: usize) -> Option<&FontRun> {
        if self.ends.is_empty() { return self.runs.first().filter(|run| index < run.len); }
        self.runs.get(self.ends.partition_point(|end| *end <= index))
    }
}

struct RendererContext<'t, 'a, 'b> {
    text_system: &'t mut DirectWriteState,
    components: &'a DirectWriteComponents,
    index_converter: StringIndexConverter<'a>,
    runs: &'b mut Vec<ShapedRun>,
    baseline: f32,
    sources: SourceFontRuns<'a>,
    font_size: Pixels,
}

#[derive(Debug)]
struct ClusterAnalyzer<'t> {
    utf16_idx: usize,
    previous_cluster_start: usize,
    glyph_count: usize,
    cluster_map: &'t [u16],
    descending: bool,
}

impl<'t> ClusterAnalyzer<'t> {
    fn new(cluster_map: &'t [u16], glyph_count: usize) -> Self {
        ClusterAnalyzer {
            utf16_idx: 0,
            previous_cluster_start: glyph_count,
            glyph_count,
            cluster_map,
            descending: cluster_map.first().zip(cluster_map.last()).is_some_and(|(first, last)| first > last),
        }
    }
}

impl Iterator for ClusterAnalyzer<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        if self.utf16_idx >= self.cluster_map.len() {
            return None; // No more clusters
        }
        let start_utf16_idx = self.utf16_idx;
        let current_glyph = self.cluster_map[start_utf16_idx] as usize;

        // Find the end of current cluster (where glyph index changes)
        let mut end_utf16_idx = start_utf16_idx + 1;
        while end_utf16_idx < self.cluster_map.len()
            && self.cluster_map[end_utf16_idx] as usize == current_glyph
        {
            end_utf16_idx += 1;
        }

        let utf16_len = end_utf16_idx - start_utf16_idx;

        // Calculate glyph count for this cluster
        let next_glyph = if self.descending {
            self.previous_cluster_start
        } else if end_utf16_idx < self.cluster_map.len() {
            self.cluster_map[end_utf16_idx] as usize
        } else {
            self.glyph_count
        };

        let glyph_count = next_glyph.checked_sub(current_glyph)?;

        // Update state for next call
        self.utf16_idx = end_utf16_idx;
        self.previous_cluster_start = current_glyph;

        Some((utf16_len, glyph_count))
    }
}

#[allow(non_snake_case)]
impl IDWritePixelSnapping_Impl for TextRenderer_Impl {
    fn IsPixelSnappingDisabled(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
    ) -> windows::core::Result<BOOL> {
        Ok(BOOL(0))
    }

    fn GetCurrentTransform(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
        transform: *mut DWRITE_MATRIX,
    ) -> windows::core::Result<()> {
        unsafe {
            *transform = DWRITE_MATRIX {
                m11: 1.0,
                m12: 0.0,
                m21: 0.0,
                m22: 1.0,
                dx: 0.0,
                dy: 0.0,
            };
        }
        Ok(())
    }

    fn GetPixelsPerDip(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
    ) -> windows::core::Result<f32> {
        Ok(1.0)
    }
}

#[allow(non_snake_case)]
impl IDWriteTextRenderer_Impl for TextRenderer_Impl {
    fn DrawGlyphRun(
        &self,
        clientdrawingcontext: *const ::core::ffi::c_void,
        baselineoriginx: f32,
        baselineoriginy: f32,
        _measuringmode: DWRITE_MEASURING_MODE,
        glyphrun: *const DWRITE_GLYPH_RUN,
        glyphrundescription: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _clientdrawingeffect: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        let glyphrun = unsafe { &*glyphrun };
        let glyph_count = glyphrun.glyphCount as usize;
        if glyph_count == 0 {
            return Ok(());
        }
        let desc = unsafe { &*glyphrundescription };
        let context = unsafe { &mut *(clientdrawingcontext.cast::<RendererContext>().cast_mut()) };
        let Some(font_face) = glyphrun.fontFace.as_ref() else {
            return Err(Error::new(DWRITE_E_NOFONT, "DirectWrite returned a glyph run without a font face"));
        };
        // This `cast()` action here should never fail since we are running on Win10+, and
        // `IDWriteFontFace3` requires Win10
        let Ok(font_face) = &font_face.cast::<IDWriteFontFace3>() else {
            return Err(Error::new(
                DWRITE_E_UNSUPPORTEDOPERATION,
                "Failed to cast font face",
            ));
        };

        context.index_converter.advance_to_utf16_ix(desc.textPosition as usize);
        let source = context.sources.at(context.index_converter.utf8_ix)
            .ok_or_else(|| Error::new(E_INVALIDARG, "DirectWrite glyph run has no source font"))?;
        let requested_size = source.font_size.unwrap_or(context.font_size);
        let font_id = context.text_system.font_id_for_native_face(context.components, font_face, source.font_id)?;
        // DirectWrite can scale a fallback independently. Only remove the
        // one-ULP perturbation introduced above to split authored ligatures.
        let effective_size = if glyphrun.fontEmSize == requested_size.as_f32().next_up() {
            requested_size
        } else {
            px(glyphrun.fontEmSize)
        };
        let run_size = if effective_size == requested_size { source.font_size } else { Some(effective_size) };

        let color_font = unsafe { font_face.IsColorFont().as_bool() };

        let glyph_ids = unsafe {
            slice_from_nullable(
                glyphrun.glyphIndices,
                glyph_count,
                "DirectWrite returned a null glyph indices array",
            )?
        };
        let cluster_map = unsafe {
            slice_from_nullable(
                desc.clusterMap,
                desc.stringLength as usize,
                "DirectWrite returned a null cluster map",
            )?
        };

        let glyph_advances = unsafe {
            slice_from_nullable(glyphrun.glyphAdvances, glyph_count,
                "DirectWrite returned a null glyph advances array")?
        };
        let cluster_analyzer = ClusterAnalyzer::new(cluster_map, glyph_count);
        let mut origins = vec![Vector2::zero(); glyph_count];
        let factory: &IDWriteFactory4 = &context.components.factory;
        // The generated windows binding returns a single Vector2, but this COM
        // method writes glyphCount elements. Supply the actual output array via
        // the inherited vtable; DirectWrite resolves offsets and bidi direction.
        unsafe {
            (Interface::vtable(factory).ComputeGlyphOrigins)(
                Interface::as_raw(factory),
                glyphrun,
                Vector2::new(baselineoriginx, baselineoriginy),
                origins.as_mut_ptr(),
            ).ok()?;
        }
        let mut utf16_idx = desc.textPosition as usize;
        let mut source_offset = 0;
        let mut glyphs = Vec::with_capacity(glyph_count);
        for (cluster_utf16_len, cluster_glyph_count) in cluster_analyzer {
            let glyph_idx = cluster_map[source_offset] as usize;
            source_offset += cluster_utf16_len;
            let cluster_end = glyph_idx.checked_add(cluster_glyph_count)
                .filter(|end| *end <= glyph_count)
                .ok_or_else(|| Error::new(E_INVALIDARG, "DirectWrite returned an invalid glyph cluster"))?;
            context.index_converter.advance_to_utf16_ix(utf16_idx);
            utf16_idx += cluster_utf16_len;
            for (cluster_glyph_idx, glyph_id) in glyph_ids
                [glyph_idx..cluster_end]
                .iter()
                .enumerate()
            {
                let id = GlyphId(*glyph_id as u32);
                let is_emoji =
                    color_font && is_color_glyph(font_face, id, &context.components.factory);
                let this_glyph_idx = glyph_idx + cluster_glyph_idx;
                glyphs.push(ShapedGlyph {
                    id,
                    position: point(
                        px(origins[this_glyph_idx].X),
                        px(origins[this_glyph_idx].Y - context.baseline),
                    ),
                    index: context.index_converter.utf8_ix,
                    is_emoji,
                    advance: px(glyph_advances[this_glyph_idx]),
                    is_rtl: glyphrun.bidiLevel % 2 == 1,
                });
            }
        }
        if source_offset != cluster_map.len() || glyphs.len() != glyph_count {
            return Err(Error::new(E_INVALIDARG, "DirectWrite returned an incomplete glyph cluster map"));
        }
        if glyphrun.bidiLevel % 2 == 1 {
            glyphs.reverse();
        }
        context.runs.push(ShapedRun { font_size: run_size, font_id, glyphs });
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
        _baselineoriginx: f32,
        _baselineoriginy: f32,
        _underline: *const DWRITE_UNDERLINE,
        _clientdrawingeffect: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::new(
            E_NOTIMPL,
            "DrawUnderline unimplemented",
        ))
    }

    fn DrawStrikethrough(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
        _baselineoriginx: f32,
        _baselineoriginy: f32,
        _strikethrough: *const DWRITE_STRIKETHROUGH,
        _clientdrawingeffect: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::new(
            E_NOTIMPL,
            "DrawStrikethrough unimplemented",
        ))
    }

    fn DrawInlineObject(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
        _originx: f32,
        _originy: f32,
        _inlineobject: windows::core::Ref<IDWriteInlineObject>,
        _issideways: BOOL,
        _isrighttoleft: BOOL,
        _clientdrawingeffect: windows::core::Ref<windows::core::IUnknown>,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::new(
            E_NOTIMPL,
            "DrawInlineObject unimplemented",
        ))
    }
}

/// Interprets an optional DirectWrite array pointer as a slice, treating a
/// null pointer with a zero length as an empty slice. A null pointer with a
/// nonzero length fails with `null_error_message`.
///
/// # Safety
///
/// When `ptr` is non-null, the caller must guarantee that it points to a valid
/// array of at least `len` elements that outlives the returned slice.
unsafe fn slice_from_nullable<'a, T>(
    ptr: *const T,
    len: usize,
    null_error_message: &str,
) -> windows::core::Result<&'a [T]> {
    if ptr.is_null() {
        if len != 0 {
            return Err(Error::new(E_INVALIDARG, null_error_message));
        }
        Ok(&[])
    } else {
        Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
    }
}

struct StringIndexConverter<'a> {
    text: &'a str,
    utf8_ix: usize,
    utf16_ix: usize,
    reversed_lookup: Option<Vec<usize>>,
}

impl<'a> StringIndexConverter<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            utf8_ix: 0,
            utf16_ix: 0,
            reversed_lookup: None,
        }
    }

    #[allow(dead_code)]
    fn advance_to_utf8_ix(&mut self, utf8_target: usize) {
        for (ix, c) in self.text[self.utf8_ix..].char_indices() {
            if self.utf8_ix + ix >= utf8_target {
                self.utf8_ix += ix;
                return;
            }
            self.utf16_ix += c.len_utf16();
        }
        self.utf8_ix = self.text.len();
    }

    fn advance_to_utf16_ix(&mut self, utf16_target: usize) {
        if self.reversed_lookup.is_none() && utf16_target < self.utf16_ix {
            // Visual runs can arrive out of source order. Build the index once
            // on that path instead of rescanning the prefix for every RTL run.
            let mut indices = Vec::with_capacity(self.text.encode_utf16().count() + 1);
            for (index, character) in self.text.char_indices() {
                indices.extend(std::iter::repeat_n(index, character.len_utf16()));
            }
            indices.push(self.text.len());
            self.reversed_lookup = Some(indices);
        }
        if let Some(indices) = &self.reversed_lookup {
            self.utf16_ix = utf16_target.min(indices.len() - 1);
            self.utf8_ix = indices[self.utf16_ix];
            return;
        }
        for (ix, c) in self.text[self.utf8_ix..].char_indices() {
            if self.utf16_ix >= utf16_target {
                self.utf8_ix += ix;
                return;
            }
            self.utf16_ix += c.len_utf16();
        }
        self.utf8_ix = self.text.len();
    }
}

fn font_style_to_dwrite(style: FontStyle) -> DWRITE_FONT_STYLE {
    match style {
        FontStyle::Normal => DWRITE_FONT_STYLE_NORMAL,
        FontStyle::Italic => DWRITE_FONT_STYLE_ITALIC,
        FontStyle::Oblique | FontStyle::ObliqueAngle(_) => DWRITE_FONT_STYLE_OBLIQUE,
    }
}

fn font_weight_to_dwrite(weight: FontWeight) -> DWRITE_FONT_WEIGHT {
    DWRITE_FONT_WEIGHT(weight.0 as i32)
}

fn get_font_names_from_collection(
    collection: &IDWriteFontCollection1,
    locale: &HSTRING,
) -> Vec<String> {
    unsafe {
        let mut result = Vec::new();
        let family_count = collection.GetFontFamilyCount();
        for index in 0..family_count {
            let Some(font_family) = collection.GetFontFamily(index).log_err() else {
                continue;
            };
            let Some(localized_family_name) = font_family.GetFamilyNames().log_err() else {
                continue;
            };
            let Some(family_name) = get_name(localized_family_name, locale).log_err() else {
                continue;
            };
            result.push(family_name);
        }

        result
    }
}

fn oblique_degrees(style: FontStyle) -> Option<f32> {
    match style { FontStyle::Oblique => Some(14.), FontStyle::ObliqueAngle(angle) => Some(angle.degrees()), _ => None }
}

fn authored_oblique_rendering(face: &IDWriteFontFace3, style: FontStyle) -> windows::core::Result<(Option<IDWriteFontFace3>, f32)> {
    let Some(angle) = oblique_degrees(style) else { return Ok((None, 0.)); };
    unsafe {
        let simulations = face.GetSimulations();
        let has_simulation = simulations.0 & DWRITE_FONT_SIMULATIONS_OBLIQUE.0 != 0;
        let remaining = DWRITE_FONT_SIMULATIONS(simulations.0 & !DWRITE_FONT_SIMULATIONS_OBLIQUE.0);
        let mut variable_slant = false;
        let raster_face = match face.cast::<IDWriteFontFace5>() {
            Ok(face5) => {
                let resource = face5.GetFontResource()?;
                let mut ranges = SmallVec::<[DWRITE_FONT_AXIS_RANGE; 5]>::new();
                ranges.resize(resource.GetFontAxisCount() as usize, DWRITE_FONT_AXIS_RANGE::default());
                if !ranges.is_empty() { resource.GetFontAxisRanges(&mut ranges)?; }
                variable_slant = ranges.iter().any(|range| range.axisTag == DWRITE_FONT_AXIS_TAG_SLANT && range.minValue != range.maxValue);
                if has_simulation { Some(resource.CreateFontFace(remaining, &native_font_axes(face)?)?.cast()?) } else { None }
            }
            Err(error) if error.code() == E_NOINTERFACE => {
                if has_simulation { Some(face.GetFontFaceReference()?.CreateFontFaceWithSimulations(remaining)?) } else { None }
            }
            Err(error) => return Err(error),
        };
        let drawing_face = raster_face.as_ref().unwrap_or(face);
        // A native slant range or a real italic/oblique face is already the
        // selected nearest face. Synthesize only an otherwise upright face.
        // Never stack an authored shear over DirectWrite's fixed simulation.
        let shear = if !variable_slant && drawing_face.GetStyle() == DWRITE_FONT_STYLE_NORMAL {
            f64::from(angle).to_radians().tan() as f32
        } else { 0. };
        Ok((raster_face, shear))
    }
}

fn shear_typographic_bounds(mut bounds: Bounds<f32>, shear: f32) -> Bounds<f32> {
    if shear != 0. {
        // Typographic bounds use y-up; DirectWrite outlines/raster use y-down.
        let lower = bounds.origin.y * shear;
        let upper = (bounds.origin.y + bounds.size.height) * shear;
        bounds.origin.x += lower.min(upper);
        bounds.size.width += (upper - lower).abs();
    }
    bounds
}

fn check_raster_size(width: i64, height: i64) -> Result<()> {
    anyhow::ensure!(width >= 0 && height >= 0 && width <= 8192 && height <= 8192
        && width * height <= 16 * 1024 * 1024 / 4, "glyph raster exceeds the 8192px / 16MiB limit");
    Ok(())
}

fn native_font_axes(face: &IDWriteFontFace3) -> windows::core::Result<SmallVec<[DWRITE_FONT_AXIS_VALUE; 5]>> {
    let face = match face.cast::<IDWriteFontFace5>() {
        Ok(face) => face,
        Err(error) if error.code() == E_NOINTERFACE => return Ok(SmallVec::new()),
        Err(error) => return Err(error),
    };
    let mut axes = SmallVec::new();
    unsafe {
        axes.resize(face.GetFontAxisValueCount() as usize, DWRITE_FONT_AXIS_VALUE::default());
        if !axes.is_empty() { face.GetFontAxisValues(&mut axes)?; }
    }
    Ok(axes)
}

// https://learn.microsoft.com/en-us/windows/win32/api/dwrite/ne-dwrite-dwrite_font_feature_tag
fn apply_font_features(
    direct_write_features: &IDWriteTypography,
    features: &FontFeatures,
) -> Result<()> {
    let tag_values = features.tag_value_list();

    // All of these features are enabled by default by DirectWrite.
    // If you want to (and can) peek into the source of DirectWrite
    let mut feature_liga = make_direct_write_feature("liga", 1);
    let mut feature_clig = make_direct_write_feature("clig", 1);
    let mut feature_calt = make_direct_write_feature("calt", 1);

    for (tag, value) in tag_values {
        if tag.as_str() == "liga" && *value == 0 {
            feature_liga.parameter = 0;
            continue;
        }
        if tag.as_str() == "clig" && *value == 0 {
            feature_clig.parameter = 0;
            continue;
        }
        if tag.as_str() == "calt" && *value == 0 {
            feature_calt.parameter = 0;
            continue;
        }

        unsafe {
            direct_write_features.AddFontFeature(make_direct_write_feature(tag, *value))?;
        }
    }
    unsafe {
        direct_write_features.AddFontFeature(feature_liga)?;
        direct_write_features.AddFontFeature(feature_clig)?;
        direct_write_features.AddFontFeature(feature_calt)?;
    }

    Ok(())
}

#[inline]
const fn make_direct_write_feature(feature_name: &str, parameter: u32) -> DWRITE_FONT_FEATURE {
    let tag = make_direct_write_tag(feature_name);
    DWRITE_FONT_FEATURE {
        nameTag: tag,
        parameter,
    }
}

#[inline]
const fn make_open_type_tag(tag_name: &str) -> u32 {
    let bytes = tag_name.as_bytes();
    debug_assert!(bytes.len() == 4);
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[inline]
const fn make_direct_write_tag(tag_name: &str) -> DWRITE_FONT_FEATURE_TAG {
    DWRITE_FONT_FEATURE_TAG(make_open_type_tag(tag_name))
}

#[inline]
fn get_name(string: IDWriteLocalizedStrings, locale: &HSTRING) -> Result<String> {
    let mut locale_name_index = 0u32;
    let mut exists = BOOL(0);
    unsafe { string.FindLocaleName(locale, &mut locale_name_index, &mut exists as _)? };
    if !exists.as_bool() {
        unsafe {
            string.FindLocaleName(
                DEFAULT_LOCALE_NAME,
                &mut locale_name_index as _,
                &mut exists as _,
            )?
        };
        if !exists.as_bool() {
            anyhow::ensure!(unsafe { string.GetCount() } > 0, "Font has no localised names");
            locale_name_index = 0;
        }
    }

    let name_length = unsafe { string.GetStringLength(locale_name_index) }? as usize;
    let mut name_vec = vec![0u16; name_length + 1];
    unsafe {
        string.GetString(locale_name_index, &mut name_vec)?;
    }

    Ok(String::from_utf16_lossy(&name_vec[..name_length]))
}

fn get_system_subpixel_rendering() -> bool {
    let mut value = c_uint::default();
    let result = unsafe {
        SystemParametersInfoW(
            SPI_GETFONTSMOOTHINGTYPE,
            0,
            Some((&mut value) as *mut c_uint as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        )
    };
    if result.log_err().is_some() {
        value == FE_FONTSMOOTHINGCLEARTYPE
    } else {
        true
    }
}

fn get_system_ui_font_name() -> SharedString {
    unsafe {
        let mut info: LOGFONTW = std::mem::zeroed();
        let font_family = if SystemParametersInfoW(
            SPI_GETICONTITLELOGFONT,
            std::mem::size_of::<LOGFONTW>() as u32,
            Some(&mut info as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .log_err()
        .is_none()
        {
            // https://learn.microsoft.com/en-us/windows/win32/uxguide/vis-fonts
            // Segoe UI is the Windows font intended for user interface text strings.
            "Segoe UI".into()
        } else {
            let font_name = String::from_utf16_lossy(&info.lfFaceName);
            font_name.trim_matches(char::from(0)).to_owned().into()
        };
        log::info!("Use {} as UI font.", font_family);
        font_family
    }
}

// One would think that with newer DirectWrite method: IDWriteFontFace4::GetGlyphImageFormats
// but that doesn't seem to work for some glyphs, say ❤
fn is_color_glyph(
    font_face: &IDWriteFontFace3,
    glyph_id: GlyphId,
    factory: &IDWriteFactory5,
) -> bool {
    let glyph_run = DWRITE_GLYPH_RUN {
        fontFace: ManuallyDrop::new(Some(unsafe { std::ptr::read(&****font_face) })),
        fontEmSize: 14.0,
        glyphCount: 1,
        glyphIndices: &(glyph_id.0 as u16),
        glyphAdvances: &0.0,
        glyphOffsets: &DWRITE_GLYPH_OFFSET {
            advanceOffset: 0.0,
            ascenderOffset: 0.0,
        },
        isSideways: BOOL(0),
        bidiLevel: 0,
    };
    unsafe {
        factory.TranslateColorGlyphRun(
            Vector2::default(),
            &glyph_run as _,
            None,
            DWRITE_GLYPH_IMAGE_FORMATS_COLR
                | DWRITE_GLYPH_IMAGE_FORMATS_SVG
                | DWRITE_GLYPH_IMAGE_FORMATS_PNG
                | DWRITE_GLYPH_IMAGE_FORMATS_JPEG
                | DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8,
            DWRITE_MEASURING_MODE_NATURAL,
            None,
            0,
        )
    }
    .is_ok()
}

const DEFAULT_LOCALE_NAME: PCWSTR = windows::core::w!("en-US");

fn is_line_separator(character: char) -> bool {
    matches!(character, '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use super::{DirectWriteState, DirectWriteTextSystem, GPUState, GlyphLayerTexture};
    use crate::direct_write::ClusterAnalyzer;
    use crate::directx_devices::DirectXDevices;
    use anyhow::Result;
    use gpui::{
        DevicePixels, Font, FontWeight, PlatformTextSystem, RenderGlyphParams, Rgba, TextRun,
        TextSystem, WindowTextSystem, bounds, point, px, size,
    };
    use std::{ffi::c_void, sync::Arc};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_RENDER_TARGET, D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RENDER_TARGET_VIEW_DESC_0,
        D3D11_RTV_DIMENSION_TEXTURE2D, D3D11_SUBRESOURCE_DATA, D3D11_TEX2D_RTV,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

    #[test]
    fn authored_oblique_keeps_angle_through_shaping_outline_and_raster() -> Result<()> {
        use gpui::{FontRun, FontStyle, GlyphInk, TextDirection};
        use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_SIMULATIONS_OBLIQUE;
        let fonts = DirectWriteTextSystem::new(None)?;
        fonts.add_fonts(vec![include_bytes!("../../cosmic-text/fonts/FiraMono-Medium.ttf").as_slice().into()])?;
        let family = {
            let state = fonts.state.read();
            let face = unsafe { state.custom_font_collection.GetFontSet()?.GetFontFaceReference(0)?.CreateFontFace()? };
            super::get_name(unsafe { face.GetFamilyNames()? }, &fonts.components.locale)?
        };
        let base = gpui::font(family);
        let base_id = fonts.font_id(&base)?;
        let glyph = fonts.glyph_for_char(base_id, 'A').unwrap();
        let mut params = RenderGlyphParams { font_id: base_id, glyph_id: glyph, font_size: px(32.),
            subpixel_variant: point(1u8, 1u8), scale_factor: 1.5, is_emoji: false, subpixel_rendering: false, dilation: 0 };
        let base_bounds = fonts.glyph_raster_bounds(&params)?;
        let (_, bitmap) = fonts.rasterize_glyph(&params, base_bounds)?;
        let centroid = |data: &[u8], rect: gpui::Bounds<DevicePixels>| {
            let mut total = 0u64;
            let mut weighted = 0f64;
            for (index, &alpha) in data.iter().enumerate() {
                total += u64::from(alpha);
                weighted += f64::from(alpha) * (rect.origin.x.0 as f64 + (index % rect.size.width.0 as usize) as f64);
            }
            assert!(total > 0);
            weighted / total as f64
        };
        let base_center = centroid(&bitmap, base_bounds);
        let metrics = fonts.font_metrics(base_id);
        let cap = metrics.cap_height / metrics.units_per_em as f32 * 32.;
        let band = px(-cap * 0.85)..px(-cap * 0.7);
        let mut ink = GlyphInk::new(&[band.clone()])?;
        fonts.intersect_glyph_ink(base_id, glyph, px(32.), &mut ink)?;
        let base_ink = ink.intersections()?[0].clone().unwrap();
        for angle in [-30f32, -5., 14., 30.] {
            let mut requested = base.clone();
            requested.style = FontStyle::with_oblique_angle(angle).unwrap();
            let id = fonts.font_id(&requested)?;
            assert_eq!(fonts.advance(id, glyph)?, fonts.advance(base_id, glyph)?);
            {
                let mut state = fonts.state.write();
                let layout = state.layout_line(&fonts.components, "Af", px(32.),
                    &[FontRun { len: 2, font_id: id, font_size: None }], TextDirection::Ltr)?;
                assert_eq!(layout.len, 2);
                assert!(!layout.runs.is_empty());
                for run in &layout.runs {
                    let actual = &state.fonts[run.font_id.0];
                    assert_eq!(actual.style, requested.style);
                    assert!((actual.shear - angle.to_radians().tan()).abs() < 1e-6);
                    assert_eq!(unsafe { actual.drawing_face().GetSimulations().0 } & DWRITE_FONT_SIMULATIONS_OBLIQUE.0, 0);
                }
                let raw = state.fonts[base_id.0].font_face.clone();
                let retained = state.font_id_for_native_face(&fonts.components, &raw, id)?;
                assert_eq!(state.fonts[retained.0].style, requested.style);
                assert_eq!(state.font_id_for_native_face(&fonts.components, &raw, id)?, retained);
            }
            let mut ink = GlyphInk::new(&[band.clone()])?;
            fonts.intersect_glyph_ink(id, glyph, px(32.), &mut ink)?;
            let ink = ink.intersections()?[0].clone().unwrap();
            if angle > 0. { assert!(ink.start > base_ink.start); }
            else { assert!(ink.end < base_ink.end); }
            params.font_id = id;
            let raster = fonts.glyph_raster_bounds(&params)?;
            let (_, bitmap) = fonts.rasterize_glyph(&params, raster)?;
            assert_eq!(bitmap.len(), raster.size.width.0 as usize * raster.size.height.0 as usize);
            let center = centroid(&bitmap, raster);
            if angle > 0. { assert!(center > base_center); }
            else { assert!(center < base_center); }
        }
        for angle in [89.99, 90., -90.] {
            let mut requested = base.clone();
            requested.style = FontStyle::with_oblique_angle(angle).unwrap();
            params.font_id = fonts.font_id(&requested)?;
            assert!(fonts.glyph_raster_bounds(&params).is_err());
        }
        Ok(())
    }

    #[test]
    fn authored_oblique_does_not_stack_on_a_real_italic_face() -> Result<()> {
        use gpui::FontStyle;
        use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_STYLE_ITALIC;
        let fonts = DirectWriteTextSystem::new(None)?;
        fonts.add_fonts(vec![include_bytes!("../../cosmic-text/fonts/InterVariable-Italic.ttf").as_slice().into()])?;
        let family = {
            let state = fonts.state.read();
            let face = unsafe { state.custom_font_collection.GetFontSet()?.GetFontFaceReference(0)?.CreateFontFace()? };
            super::get_name(unsafe { face.GetFamilyNames()? }, &fonts.components.locale)?
        };
        let mut requested = gpui::font(family);
        requested.style = FontStyle::with_oblique_angle(30.).unwrap();
        let id = fonts.font_id(&requested)?;
        let state = fonts.state.read();
        let actual = &state.fonts[id.0];
        assert_eq!(unsafe { actual.drawing_face().GetStyle() }, DWRITE_FONT_STYLE_ITALIC);
        assert_eq!(actual.shear, 0.);
        Ok(())
    }

    #[test]
    fn requested_variable_weights_reach_shaping_without_integer_quantization() -> Result<()> {
        use gpui::{FontRun, TextDirection};
        use windows::Win32::Graphics::DirectWrite::DWRITE_FONT_AXIS_TAG_WEIGHT;

        let fonts = DirectWriteTextSystem::new(None)?;
        fonts.add_fonts(vec![include_bytes!("../../cosmic-text/fonts/InterVariable.ttf").as_slice().into()])?;
        let family = {
            let state = fonts.state.read();
            let face = unsafe { state.custom_font_collection.GetFontSet()?.GetFontFaceReference(0)?.CreateFontFace()? };
            super::get_name(unsafe { face.GetFamilyNames()? }, &fonts.components.locale)?
        };
        let mut widths = Vec::new();
        for weight in [100., 350.5, 650.5, 900.] {
            let mut requested = gpui::font(family.clone());
            requested.weight = FontWeight(weight);
            let id = fonts.font_id(&requested)?;
            assert_eq!(fonts.font_id(&requested)?, id);
            let mut state = fonts.state.write();
            let actual = |font: &super::FontInfo| font.axes.iter()
                .find(|axis| axis.axisTag == DWRITE_FONT_AXIS_TAG_WEIGHT).unwrap().value;
            assert_eq!(actual(&state.fonts[id.0]), weight);
            let layout = state.layout_line(&fonts.components, "Wi", px(24.),
                &[FontRun { len: 2, font_id: id, font_size: None }], TextDirection::Ltr)?;
            assert_eq!(layout.len, 2);
            assert!(!layout.runs.is_empty());
            for run in &layout.runs {
                assert_eq!(actual(&state.fonts[run.font_id.0]), weight);
            }
            widths.push(layout.width);
        }
        assert_ne!(widths[1], widths[2]);
        Ok(())
    }

    #[test]
    fn native_fallback_preserves_fractional_axes_and_source_context() -> Result<()> {
        use anyhow::Context;
        use gpui::{FontFallbacks, FontFeatures, FontRun, TextDirection};
        use windows::{core::{Interface, IUnknown}, Win32::Graphics::DirectWrite::*};

        let fonts = DirectWriteTextSystem::new(None)?;
        fonts.add_fonts(vec![include_bytes!("../../cosmic-text/fonts/InterVariable.ttf").as_slice().into()])?;
        let mut source = gpui::font("Segoe UI");
        source.features = FontFeatures::disable_ligatures();
        source.fallbacks = Some(FontFallbacks(Arc::new(vec!["Arial".into()])));
        let source_id = fonts.font_id(&source)?;
        source.features = FontFeatures::default();
        let other_source = fonts.font_id(&source)?;
        let mut state = fonts.state.write();
        let face = unsafe {
            let set = state.custom_font_collection.GetFontSet()?;
            let initial = set.GetFontFaceReference(0)?.CreateFontFace()?;
            let mut axes = super::native_font_axes(&initial)?;
            axes.iter_mut().find(|axis| axis.axisTag == DWRITE_FONT_AXIS_TAG_WEIGHT)
                .context("Inter variable font has no weight axis")?.value = 650.5;
            initial.cast::<IDWriteFontFace5>()?.GetFontResource()?
                .CreateFontFace(initial.GetSimulations(), &axes)?.cast::<IDWriteFontFace3>()?
        };
        let id = state.font_id_for_native_face(&fonts.components, &face, source_id)?;
        assert_eq!(state.fonts[id.0].font_face.cast::<IUnknown>()?, face.cast::<IUnknown>()?);
        assert_eq!(state.fonts[id.0].features, state.fonts[source_id.0].features);
        assert_eq!(state.fonts[id.0].fallbacks, state.fonts[source_id.0].fallbacks);
        assert_eq!(state.fonts[id.0].axes.iter().find(|axis| axis.axisTag == DWRITE_FONT_AXIS_TAG_WEIGHT).unwrap().value, 650.5);
        let count = state.fonts.len();
        assert_eq!(state.font_id_for_native_face(&fonts.components, &face, source_id)?, id);
        assert_eq!(state.fonts.len(), count);
        let other_id = state.font_id_for_native_face(&fonts.components, &face, other_source)?;
        assert_ne!(other_id, id);
        assert_eq!(state.fonts[other_id.0].features, state.fonts[other_source.0].features);

        let layout = state.layout_line(&fonts.components, "Wi", px(13.),
            &[FontRun { len: 2, font_id: id, font_size: None }], TextDirection::Ltr)?;
        assert_eq!(layout.len, 2);
        assert!(layout.width > px(0.));
        assert!(!layout.runs.is_empty());
        for run in &layout.runs {
            let retained = &state.fonts[run.font_id.0];
            assert!(unsafe { retained.font_face.cast::<IDWriteFontFace5>()?.Equals(&face).as_bool() });
            assert_eq!(retained.features, state.fonts[source_id.0].features);
            assert_eq!(run.font_size.unwrap_or(layout.font_size), px(13.));
        }
        Ok(())
    }

    #[test]
    fn unicode_line_separators_preserve_shaped_line_cursor() -> Result<()> {
        check_unicode_separator_cursor('\u{2028}')
    }

    #[test]
    fn unicode_paragraph_separators_preserve_shaped_line_cursor() -> Result<()> {
        check_unicode_separator_cursor('\u{2029}')
    }

    fn check_unicode_separator_cursor(separator: char) -> Result<()> {
        let platform_text_system = Arc::new(DirectWriteTextSystem::new(None)?);
        let font = Font {
            family: "Segoe UI".into(),
            ..Default::default()
        };
        platform_text_system.font_id(&font)?;
        let text_system = WindowTextSystem::new(Arc::new(TextSystem::new(platform_text_system)));

        let separated_letters = ["a", "b", "c", "d", "e", "f"].join(&separator.to_string());
        for text in [
            separated_letters,
            format!("{separator}ab{separator}{separator}cd{separator}"),
            format!("{separator}{separator}{separator}{separator}{separator}"),
            format!("é{separator}𐐀{separator}z"),
        ] {
            for multiple_fonts in [false, true] {
                let mut runs = vec![TextRun {
                    len: text.len(),
                    font: font.clone(),
                    ..Default::default()
                }];
                if multiple_fonts {
                    let first_len = text.chars().next().map_or(0, char::len_utf8);
                    runs[0].len = first_len;
                    let mut bold_font = font.clone();
                    bold_font.weight = FontWeight::BOLD;
                    runs.push(TextRun {
                        len: text.len() - first_len,
                        font: bold_font,
                        ..Default::default()
                    });
                }
                let shaped = text_system.shape_line(text.clone().into(), px(14.0), &runs, None);
                let mut reference_runs = runs.clone();
                let mut offset = 0;
                for run in &mut reference_runs {
                    let end = offset + run.len;
                    run.len = text[offset..end]
                        .chars()
                        .filter(|character| *character != separator)
                        .map(char::len_utf8)
                        .sum();
                    offset = end;
                }
                let reference_text: String = text
                    .chars()
                    .filter(|character| *character != separator)
                    .collect();
                let reference =
                    text_system.shape_line(reference_text.into(), px(14.0), &reference_runs, None);
                assert!(
                    (shaped.width - reference.width).abs() < px(0.001),
                    "{text:?}, multiple_fonts={multiple_fonts}"
                );

                let visible_glyphs: Vec<_> = shaped
                    .runs
                    .iter()
                    .flat_map(|run| run.glyphs.iter().map(|glyph| (run.font_id, glyph)))
                    .filter(|(_, glyph)| !text[glyph.index..].starts_with(separator))
                    .collect();
                let reference_glyphs: Vec<_> = reference
                    .runs
                    .iter()
                    .flat_map(|run| run.glyphs.iter().map(|glyph| (run.font_id, glyph)))
                    .collect();
                assert_eq!(visible_glyphs.len(), reference_glyphs.len(), "{text:?}");
                for ((actual_font, actual), (expected_font, expected)) in
                    visible_glyphs.into_iter().zip(reference_glyphs)
                {
                    assert_eq!(actual_font, expected_font, "{text:?}");
                    assert_eq!(actual.id, expected.id, "{text:?}");
                    assert!((actual.position.x - expected.position.x).abs() < px(0.001));
                    assert!((actual.position.y - expected.position.y).abs() < px(0.001));
                    let reference_index: usize = text[..actual.index]
                        .chars()
                        .filter(|character| *character != separator)
                        .map(char::len_utf8)
                        .sum();
                    assert_eq!(reference_index, expected.index, "{text:?}");
                }

                let mut cursor = shaped.cursor();
                for (index, character) in text.char_indices() {
                    let piece = cursor.take_until(index + character.len_utf8());
                    assert_eq!(piece.text.as_ref(), character.to_string());
                    assert_eq!(piece.len(), character.len_utf8());
                    if character == separator {
                        assert_eq!(piece.width, px(0.0));
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_cluster_map() {
        let cluster_map = [0];
        let mut analyzer = ClusterAnalyzer::new(&cluster_map, 1);
        let next = analyzer.next();
        assert_eq!(next, Some((1, 1)));
        let next = analyzer.next();
        assert_eq!(next, None);

        let cluster_map = [0, 1, 2];
        let mut analyzer = ClusterAnalyzer::new(&cluster_map, 3);
        let next = analyzer.next();
        assert_eq!(next, Some((1, 1)));
        let next = analyzer.next();
        assert_eq!(next, Some((1, 1)));
        let next = analyzer.next();
        assert_eq!(next, Some((1, 1)));
        let next = analyzer.next();
        assert_eq!(next, None);
        // 👨‍👩‍👧‍👦👩‍💻
        let cluster_map = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 4, 4, 4];
        let mut analyzer = ClusterAnalyzer::new(&cluster_map, 5);
        let next = analyzer.next();
        assert_eq!(next, Some((11, 4)));
        let next = analyzer.next();
        assert_eq!(next, Some((5, 1)));
        let next = analyzer.next();
        assert_eq!(next, None);
        // 👩‍💻
        let cluster_map = [0, 0, 0, 0, 0];
        let mut analyzer = ClusterAnalyzer::new(&cluster_map, 1);
        let next = analyzer.next();
        assert_eq!(next, Some((5, 1)));
        let next = analyzer.next();
        assert_eq!(next, None);
    }

    #[test]
    fn color_emoji_rasterization_is_stable_across_batches() -> Result<()> {
        let devices = DirectXDevices::new()?;
        let text_system = DirectWriteTextSystem::new(Some(&devices))?;

        let font = Font {
            family: "Segoe UI Emoji".into(),
            ..Default::default()
        };
        let font_id = text_system.font_id(&font)?;

        let mut params_list = Vec::new();
        for ch in ['🫠', '🥹', '🧗', '🏋', '🚀', '🥺'] {
            let Some(glyph_id) = text_system.glyph_for_char(font_id, ch) else {
                log::info!("no glyph found for {ch}");
                continue;
            };
            let params = RenderGlyphParams {
                font_id,
                glyph_id,
                font_size: px(48.0),
                subpixel_variant: point(0u8, 0u8),
                scale_factor: 1.0,
                is_emoji: true,
                subpixel_rendering: false,
                dilation: 0,
            };
            let raster_bounds = text_system.glyph_raster_bounds(&params)?;
            if raster_bounds.size.width.0 == 0 || raster_bounds.size.height.0 == 0 {
                log::info!("raster bounds are empty for {ch}");
                continue;
            }
            params_list.push((params, raster_bounds));
        }
        assert!(!params_list.is_empty());

        let first: Vec<_> = params_list
            .iter()
            .map(|(params, bounds)| text_system.rasterize_glyph(params, *bounds))
            .collect::<Result<_>>()?;

        // Churn the texture heap with further rasterization passes. If the color
        // compositing leaks leftover texture data (the render target is not cleared),
        // the second batch can pick up different contents and differ from the first.
        // With an explicit clear both batches are deterministic and identical.
        for _ in 0..3 {
            for (params, bounds) in &params_list {
                text_system.rasterize_glyph(params, *bounds)?;
            }
        }
        let second: Vec<_> = params_list
            .iter()
            .map(|(params, bounds)| text_system.rasterize_glyph(params, *bounds))
            .collect::<Result<_>>()?;

        assert_eq!(
            first, second,
            "color glyph rasterization changed between batches; \
             render target contents are leaking into the glyph bitmaps"
        );
        Ok(())
    }

    #[test]
    fn color_emoji_composites_over_cleared_texture() -> Result<()> {
        let devices = DirectXDevices::new()?;
        let gpu_state = GPUState::new(&devices)?;

        const SIZE: u32 = 32;
        // Seed the render target with solid red so that any texel which the
        // compositing pass fails to clear/overwrite remains identifiable after
        // the readback.
        let poison = {
            let mut v = vec![0u8; (SIZE * SIZE * 4) as usize];
            for pixel in v.chunks_exact_mut(4) {
                pixel[2] = 255;
                pixel[3] = 255;
            }
            v
        };
        let desc = D3D11_TEXTURE2D_DESC {
            Width: SIZE,
            Height: SIZE,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let initial_data = D3D11_SUBRESOURCE_DATA {
            pSysMem: poison.as_ptr() as *const c_void,
            SysMemPitch: SIZE * 4,
            SysMemSlicePitch: 0,
        };
        let texture = unsafe {
            let mut texture = None;
            gpu_state
                .device
                .CreateTexture2D(&desc, Some(&initial_data), Some(&mut texture))?;
            texture.unwrap()
        };
        let render_target_view = unsafe {
            let desc = D3D11_RENDER_TARGET_VIEW_DESC {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
                },
            };
            let mut rtv = None;
            gpu_state
                .device
                .CreateRenderTargetView(&texture, Some(&desc), Some(&mut rtv))?;
            rtv.unwrap()
        };

        // A single opaque layer in the top-left corner; the bottom-right corner
        // of the texture is covered by no layer at all.
        let layer_alpha = vec![255u8; 4 * 4];
        let layer = GlyphLayerTexture::new(
            &gpu_state,
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            bounds(point(0, 0), size(4, 4)),
            &layer_alpha,
        )?;

        let rasterized = DirectWriteState::composite_color_layers(
            &gpu_state,
            std::slice::from_ref(&layer),
            size(DevicePixels(SIZE as i32), DevicePixels(SIZE as i32)),
            &texture,
            &Some(render_target_view),
        )?;

        let corner = (SIZE as usize - 1 + (SIZE as usize - 1) * SIZE as usize) * 4;
        assert_eq!(
            &rasterized[corner..corner + 4],
            &[0, 0, 0, 0],
            "uncovered texel retained the poison from the uninitialized render target"
        );
        Ok(())
    }
}
