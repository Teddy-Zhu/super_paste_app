use image::ImageFormat;
use crate::{Error, Result};
use super::super::EncodedImage;
use super::AnimationInfo;

pub(super) fn inspect(encoded: &EncodedImage) -> Result<AnimationInfo> {
    let data = encoded.bytes();
    match encoded.format {
        Some(ImageFormat::Gif) => {
            let (gif_plays, frames) = gif_info(data)?;
            Ok(AnimationInfo::Animated { single_frame: frames == 1, gif_plays })
        }
        Some(ImageFormat::WebP) => webp_info(data),
        Some(ImageFormat::Png) => {
            let mut offset = 8usize;
            while let Some(chunk) = data.get(offset..).and_then(|tail| tail.get(..8)) {
                if &chunk[4..8] == b"acTL" {
                    if u32::from_be_bytes(chunk[..4].try_into().unwrap()) != 8 { return Err(Error::Image("PNG 动画控制块长度无效".into())); }
                    let control = data.get(offset..).and_then(|tail| tail.get(8..16))
                        .ok_or_else(|| Error::Image("PNG 动画控制块不完整".into()))?;
                    let frames = u32::from_be_bytes(control[..4].try_into().unwrap());
                    return Ok(AnimationInfo::Animated { single_frame: frames == 1, gif_plays: None });
                }
                if matches!(&chunk[4..8], b"IDAT" | b"IEND") { break; }
                let size = u32::from_be_bytes(chunk[..4].try_into().unwrap()) as usize;
                offset = offset.checked_add(12).and_then(|offset| offset.checked_add(size))
                    .ok_or_else(|| Error::Image("PNG 数据块长度无效".into()))?;
            }
            Ok(AnimationInfo::Static)
        }
        _ => Ok(AnimationInfo::Static),
    }
}
fn webp_info(data: &[u8]) -> Result<AnimationInfo> {
    if data.get(12..16) != Some(b"VP8X") || !data.get(20).is_some_and(|flags| flags & 2 != 0) { return Ok(AnimationInfo::Static); }
    let invalid = || Error::Image("WebP 数据块长度无效".into());
    let size = u32::from_le_bytes(data.get(4..8).ok_or_else(invalid)?.try_into().unwrap()) as usize;
    let end = size.checked_add(8).ok_or_else(invalid)?;
    let data = data.get(..end).ok_or_else(invalid)?;
    let mut offset = 12usize; let mut frames = 0;
    while offset < data.len() {
        let chunk = data.get(offset..).and_then(|tail| tail.get(..8)).ok_or_else(invalid)?;
        let size = u32::from_le_bytes(chunk[4..8].try_into().unwrap()) as usize;
        let end = offset.checked_add(8).and_then(|offset| offset.checked_add(size))
            .and_then(|end| end.checked_add(size & 1)).ok_or_else(invalid)?;
        if end > data.len() { return Err(invalid()); }
        if &chunk[..4] == b"ANMF" { frames += 1; if frames == 2 { break; } }
        offset = end;
    }
    Ok(AnimationInfo::Animated { single_frame: frames == 1, gif_plays: None })
}

/// Read only GIF block framing; compressed image data is skipped by sub-block
/// lengths, never decoded here. The loop extension can follow an image block.
fn gif_info(data: &[u8]) -> Result<(Option<u32>, u32)> {
    let header = data.get(..13).ok_or_else(invalid_gif)?;
    if !matches!(&header[..6], b"GIF87a" | b"GIF89a") { return Err(invalid_gif()); }
    let mut offset = 13 + palette_size(header[10]);
    let mut plays = Some(1); let mut frames = 0u32;
    loop {
        let tag = *data.get(offset).ok_or_else(invalid_gif)?;
        offset += 1;
        match tag {
            0x3b => return Ok((plays, frames)),
            0x2c => {
                frames = frames.checked_add(1).ok_or_else(invalid_gif)?;
                let descriptor = data.get(offset..offset + 9).ok_or_else(invalid_gif)?;
                offset += 9 + palette_size(descriptor[8]);
                data.get(offset).ok_or_else(invalid_gif)?; // LZW minimum code size
                offset += 1;
                while sub_block(data, &mut offset)?.is_some() {}
            }
            0x21 => {
                let application = *data.get(offset).ok_or_else(invalid_gif)? == 0xff;
                offset += 1;
                let first = sub_block(data, &mut offset)?;
                let looping = application && first.is_some_and(|name| matches!(name, b"NETSCAPE2.0" | b"ANIMEXTS1.0"));
                if first.is_none() { continue; }
                while let Some(block) = sub_block(data, &mut offset)? {
                    if looping && block.len() == 3 && block[0] == 1 {
                        let repeats = u16::from_le_bytes([block[1], block[2]]);
                        plays = (repeats != 0).then(|| u32::from(repeats) + 1);
                    }
                }
            }
            _ => return Err(invalid_gif()),
        }
    }
}
fn palette_size(flags: u8) -> usize { if flags & 0x80 != 0 { 3 * (1usize << ((flags & 7) + 1)) } else { 0 } }
fn sub_block<'a>(data: &'a [u8], offset: &mut usize) -> Result<Option<&'a [u8]>> {
    let size = usize::from(*data.get(*offset).ok_or_else(invalid_gif)?);
    *offset += 1;
    if size == 0 { return Ok(None); }
    let end = offset.checked_add(size).ok_or_else(invalid_gif)?;
    let block = data.get(*offset..end).ok_or_else(invalid_gif)?;
    *offset = end; Ok(Some(block))
}
fn invalid_gif() -> Error { Error::Image("GIF 数据块无效".into()) }
