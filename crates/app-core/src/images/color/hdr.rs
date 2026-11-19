//! HDR display mapping operates on the already-decoded, straight RGB.
//! Source pixels remain canonical; only the shared SDR display is mapped.
use std::sync::Arc;
use gainforge::{GainHdrMetadata, MappingColorSpace, RgbToneMapperParameters, SyncToneMapper8Bit, ToneMappingMethod};
use moxcms::{Chromaticity, CicpColorPrimaries, ColorPrimaries, ColorProfile, Layout, ToneReprCurve, TransformF32Executor, TransformOptions};
use crate::{Error, Result};

const SDR_WHITE: f32 = 203.;
const PQ_CEILING: f32 = 10_000.;
const HLG_REFERENCE: f32 = 1000.;

#[derive(Clone, Copy)]
enum Transfer { Pq, Hlg { luma: [f32; 3] } }

#[derive(Clone, Copy)]
pub(super) struct Input { transfer: Transfer, peak: f32 }

impl Input {
    pub fn read(codes: [u8; 4], metadata: &super::png::Metadata) -> Result<Self> {
        let (transfer, reference) = if codes[1] == 18 {
            let primaries = CicpColorPrimaries::try_from(codes[0]).map_err(super::color_error)?;
            let white = match codes[0] {
                4 | 8 => Chromaticity { x: 0.310, y: 0.316 },
                11 => Chromaticity { x: 0.314, y: 0.351 },
                _ => Chromaticity::D65,
            };
            let matrix = ColorPrimaries::try_from(primaries).map_err(super::color_error)?.transform_to_xyz_d(white);
            (Transfer::Hlg { luma: matrix.v[1].map(|value| value as f32) }, HLG_REFERENCE)
        } else if codes[1] == 16 { (Transfer::Pq, PQ_CEILING) }
        else { return Err(Error::Image("无法显示图片的 HDR 传递函数".into())); };
        Ok(Self { transfer, peak: metadata.hdr_peak(reference)? })
    }

    pub fn is_hlg(self) -> bool { matches!(self.transfer, Transfer::Hlg { .. }) }
}

pub(super) fn linear_profile(profile: &ColorProfile) -> ColorProfile {
    let mut linear = profile.clone();
    linear.cicp = None;
    let curve = ToneReprCurve::Parametric(vec![1.]);
    linear.red_trc = Some(curve.clone());
    linear.green_trc = Some(curve.clone());
    linear.blue_trc = Some(curve);
    linear
}

pub(super) struct Mapping {
    mapper: Option<Arc<SyncToneMapper8Bit>>,
    display: Arc<TransformF32Executor>,
    scale: f32,
    transfer: Transfer,
}

impl Mapping {
    pub fn new(linear: &ColorProfile, input: Input) -> Result<Self> {
        let peak = input.peak;
        if !peak.is_finite() || peak <= 0. || peak > PQ_CEILING { return Err(Error::Image("PNG HDR 亮度元数据无效".into())); }
        let mapper = if peak > SDR_WHITE {
            // This entry point's linearized method only applies the mapper;
            // conversion/encoding stay in the existing MoxCMS float path.
            Some(gainforge::create_tone_mapper_rgba(linear, &ColorProfile::new_srgb(),
                ToneMappingMethod::Itu2408(GainHdrMetadata::new(peak, SDR_WHITE)),
                MappingColorSpace::Rgb(RgbToneMapperParameters::default())).map_err(error)?)
        } else { None };
        let display = linear.create_transform_f32(Layout::Rgba, &ColorProfile::new_srgb(), Layout::Rgba,
            TransformOptions { allow_extended_range_rgb_xyz: true, ..Default::default() }).map_err(super::color_error)?;
        let reference = if input.is_hlg() { HLG_REFERENCE } else { PQ_CEILING };
        Ok(Self { scale: reference / if mapper.is_some() { peak } else { SDR_WHITE }, mapper, display, transfer: input.transfer })
    }

    pub fn apply(&self, pixels: &mut Vec<f32>, scratch: &mut Vec<f32>) -> Result<()> {
        for pixel in pixels.chunks_exact_mut(4) {
            for value in &mut pixel[..3] {
                if !value.is_finite() { return Err(Error::Image("HDR 图片色彩转换结果无效".into())); }
            }
            let gain = match self.transfer {
                Transfer::Pq => self.scale,
                Transfer::Hlg { luma } => {
                    for value in &mut pixel[..3] { *value = hlg_scene(*value); }
                    let luminance = pixel[..3].iter().zip(luma).map(|(sample, weight)| sample * weight).sum::<f32>();
                    // BT.2100 reference OOTF at a 1000-nit reference display:
                    // one luminance gain preserves RGB ratios (gamma=1.2).
                    luminance.max(0.).powf(0.2) * self.scale
                },
            };
            for value in &mut pixel[..3] {
                *value *= gain;
            }
        }
        if let Some(mapper) = &self.mapper { mapper.tonemap_linearized_lane(pixels).map_err(error)?; }
        self.display.transform(pixels, scratch).map_err(super::color_error)?;
        std::mem::swap(pixels, scratch);
        Ok(())
    }
}

// BT.2100 inverse OETF. Keep super-whites; clipping belongs after mapping.
fn hlg_scene(signal: f32) -> f32 {
    let signal = signal.max(0.);
    if signal <= 0.5 { signal * signal / 3. }
    else { (((signal - 0.55991073) / 0.17883277).exp() + 0.28466892) / 12. }
}

fn error(error: gainforge::ForgeError) -> Error { Error::Image(format!("图片亮度映射失败：{error}").into()) }
