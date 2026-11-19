//! Fixed-size PNG color metadata missing from the common ImageDecoder interface.
//! Pixel inflation, palette expansion and image validation remain in image/png.

use moxcms::{Chromaticity, ColorPrimaries, ColorProfile, ToneReprCurve, XyY};

use crate::{Error, Result};

#[derive(Default)]
struct Field<const N: usize>(Option<Result<[u8; N]>>);
impl<const N: usize> Field<N> {
    fn capture(&mut self, bytes: &[u8]) {
        self.0 = Some(if self.0.is_some() { Err(invalid()) } else { bytes.try_into().map_err(|_| invalid()) });
    }
    fn value(&self) -> Result<Option<[u8; N]>> { self.0.clone().transpose() }
}

#[derive(Default)]
pub(super) struct Metadata {
    cicp: Field<4>,
    srgb: Field<1>,
    gamma: Field<4>,
    chromaticities: Field<32>,
    content_light: Field<8>,
    mastering: Field<24>,
}

impl Metadata {
    pub fn cicp(&self) -> Result<Option<[u8; 4]>> { validate_cicp(self.cicp.value()?) }

    /// HDR mapping peak in nits. Zero denotes unknown; prefer content over
    /// mastering metadata, then the transfer function's reference peak.
    /// Mastering primaries never replace the actual cICP color primaries.
    pub fn hdr_peak(&self, reference_peak: f32) -> Result<f32> {
        if let Some(bytes) = self.content_light.value()? {
            let peak = u32::from_be_bytes(bytes[..4].try_into().map_err(|_| invalid())?);
            let average = u32::from_be_bytes(bytes[4..].try_into().map_err(|_| invalid())?);
            if peak > 100_000_000 || average > 100_000_000 || (peak != 0 && average > peak) { return Err(invalid()); }
            if peak != 0 { return Ok(peak as f32 / 10_000.); }
        }
        if let Some(bytes) = self.mastering.value()? {
            let peak = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| invalid())?);
            let minimum = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| invalid())?);
            if peak > 100_000_000 || minimum > peak { return Err(invalid()); }
            if peak != 0 { return Ok(peak as f32 / 10_000.); }
        }
        Ok(reference_peak)
    }

    /// Called only after recognized CICP and an embedded ICC were excluded.
    /// Lower-priority malformed fields cannot invalidate a chosen color space.
    pub fn fallback(&self) -> Result<Option<Sdr>> {
        if let Some([intent]) = self.srgb.value()? {
            if intent > 3 { return Err(invalid()); }
            return Ok(None);
        }
        let gamma = self.gamma.value()?.map(u32::from_be_bytes);
        if gamma == Some(0) { return Err(invalid()); }
        let chromaticities = self.chromaticities.value()?.map(|bytes| {
            std::array::from_fn(|index| u32::from_be_bytes([
                bytes[index * 4], bytes[index * 4 + 1], bytes[index * 4 + 2], bytes[index * 4 + 3],
            ]))
        });
        if gamma.is_none() && chromaticities.is_none() { return Ok(None); }
        Ok(Some(Sdr { gamma, chromaticities }))
    }
}

pub(super) struct Sdr {
    pub gamma: Option<u32>,
    pub chromaticities: Option<[u32; 8]>,
}

impl Sdr {
    pub fn profile(&self, grayscale: bool) -> Result<ColorProfile> {
        // When only one half is specified, assume sRGB for the unspecified
        // transfer function or primaries and retain the supplied property.
        let mut profile = ColorProfile::new_srgb();
        profile.cicp = None;
        if let Some(gamma) = self.gamma {
            if gamma == 0 { return Err(invalid()); }
            // PNG stores the encoding exponent; the input TRC is its inverse.
            // Avoid quantizing the exponent to ICC's u8Fixed8 gamma encoding.
            let curve = ToneReprCurve::Parametric(vec![(100_000.0 / f64::from(gamma)) as f32]);
            profile.red_trc = Some(curve.clone());
            profile.green_trc = Some(curve.clone());
            profile.blue_trc = Some(curve);
        }
        if let Some(values) = self.chromaticities {
            let (white_x, white_y) = coordinates(values[0], values[1])?;
            // Grayscale PNGs specify a white point, not RGB primaries. Their
            // samples expand to equal RGB values at the display boundary.
            let primaries = if grayscale { ColorPrimaries::BT_709 } else {
                ColorPrimaries { red: chromaticity(values[2], values[3])?,
                    green: chromaticity(values[4], values[5])?, blue: chromaticity(values[6], values[7])? }
            };
            profile.update_rgb_colorimetry(XyY { x: white_x, y: white_y, yb: 1.0 }, primaries);
            // The library's singular-matrix inverse falls back to identity;
            // reject that geometry rather than accepting a false color space.
            let matrix = profile.rgb_to_xyz_matrix();
            if !matrix.v.iter().flatten().all(|value| value.is_finite()) || matrix.determinant().is_none() { return Err(invalid()); }
        }
        Ok(profile)
    }
}

fn chromaticity(x: u32, y: u32) -> Result<Chromaticity> {
    let (x, y) = coordinates(x, y)?;
    Ok(Chromaticity { x: x as f32, y: y as f32 })
}

fn coordinates(x: u32, y: u32) -> Result<(f64, f64)> {
    if y == 0 { return Err(invalid()); }
    Ok((f64::from(x) / 100_000.0, f64::from(y) / 100_000.0))
}

// https://www.w3.org/TR/png-3/#cICP-chunk
// Color chunks precede PLTE/IDAT. Keep only fixed fields, in one header pass;
// validation is deferred until precedence selects a field for consumption.
pub(super) fn read(bytes: &[u8]) -> Result<Metadata> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Err(invalid()); }
    let mut remaining = &bytes[8..];
    let mut metadata = Metadata::default();
    loop {
        let header = remaining.get(..8).ok_or_else(invalid)?;
        let length = u32::from_be_bytes(header[..4].try_into().map_err(|_| invalid())?) as usize;
        let kind = &header[4..8];
        if matches!(kind, b"PLTE" | b"IDAT" | b"IEND") { return Ok(metadata); }
        let end = 8_usize.checked_add(length).ok_or_else(invalid)?;
        let next = end.checked_add(4).ok_or_else(invalid)?;
        let chunk = remaining.get(..next).ok_or_else(invalid)?;
        if matches!(kind, b"cICP" | b"sRGB" | b"gAMA" | b"cHRM" | b"cLLI" | b"mDCV") {
            let crc = u32::from_be_bytes(chunk[end..next].try_into().map_err(|_| invalid())?);
            // Invalid ancillary CRCs are discarded, never used as color input.
            if crc32fast::hash(&chunk[4..end]) == crc {
                let data = &chunk[8..end];
                match kind {
                    b"cICP" => metadata.cicp.capture(data),
                    b"sRGB" => metadata.srgb.capture(data),
                    b"gAMA" => metadata.gamma.capture(data),
                    b"cHRM" => metadata.chromaticities.capture(data),
                    b"cLLI" => metadata.content_light.capture(data),
                    b"mDCV" => metadata.mastering.capture(data),
                    _ => unreachable!(),
                }
            }
        }
        remaining = &remaining[next..];
    }
}

fn validate_cicp(cicp: Option<[u8; 4]>) -> Result<Option<[u8; 4]>> {
    let Some(cicp) = cicp else { return Ok(None); };
    if cicp[2] != 0 || cicp[3] > 1 { return Err(invalid()); }
    // cICP takes precedence only when understood (PNG 3, 11.3.2.6).
    // XYZ (10), like unknown/unspecified primaries, is not a supported RGB
    // profile. Follow the browser PNG path: leave iCCP and SDR metadata
    // eligible instead of letting unsupported metadata hide the image.
    if !matches!(cicp[0], 1 | 4..=9 | 11..=12 | 22) || !matches!(cicp[1], 1 | 4..=18) { return Ok(None); }
    Ok(Some(cicp))
}

fn invalid() -> Error { Error::Image("PNG 色彩元数据无效".into()) }
