use super::*;

// BT.2100 1000-nit reference-display samples. The unequal RGB sample detects
// applying system gamma independently to channels instead of to luminance.
const HLG: [[f64; 3]; 5] = [[0.; 3], [0.5; 3], [0.75, 0.5, 0.25], [0.75; 3], [1.; 3]];
const NITS: [[f64; 3]; 5] = [[0.; 3], [50.697028; 3],
    [175.460037, 55.183909, 13.795977], [203.152145; 3], [1000.; 3]];

fn source(hlg: bool, depth: ::png::BitDepth, narrow: bool, clli: Option<&[u8]>, count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = ::png::Encoder::new(&mut bytes, count as u32, 1);
        encoder.set_color(::png::ColorType::Rgba);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        writer.write_chunk(::png::chunk::ChunkType(*b"cICP"), &[9, if hlg { 18 } else { 16 }, 0, u8::from(!narrow)]).unwrap();
        if let Some(clli) = clli { writer.write_chunk(::png::chunk::ChunkType(*b"cLLI"), clli).unwrap(); }
        let maximum = if depth == ::png::BitDepth::Sixteen { 65535. } else { 255. };
        let (offset, span) = if narrow { if maximum > 255. { (4096., 56064.) } else { (16., 219.) } } else { (0., maximum) };
        let mut pixels = Vec::new();
        for index in 0..count {
            let rgb = if hlg { HLG[index].map(|value| (offset + span * value).round() as u16) }
                else { NITS[index].map(|value| super::pq::signal(value, maximum, narrow)) };
            let alpha = if index == 1 { (128 * maximum as u32 / 255) as u16 } else { maximum as u16 };
            for value in rgb.into_iter().chain([alpha]) {
                if depth == ::png::BitDepth::Sixteen { pixels.extend_from_slice(&value.to_be_bytes()); } else { pixels.push(value as u8); }
            }
        }
        writer.write_image_data(&pixels).unwrap();
    }
    bytes
}

fn spec() -> PixelSpec { PixelSpec { max_width: 16, max_height: 1, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest } }
fn clli(peak: u32) -> Vec<u8> { [peak.to_be_bytes(), 0_u32.to_be_bytes()].concat() }

#[tokio::test]
async fn hlg_reference_display_matches_physical_pq_without_altering_raw_pixels() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("hlg.sqlite3")).await.unwrap();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).unwrap();
    let cancel = CancellationToken::new();
    for peak in [10_000_000, 20_000_000] {
        let light = clli(peak);
        let pq = service.import_encoded(source(false, ::png::BitDepth::Sixteen, false, Some(&light), HLG.len()), &cancel).await.unwrap();
        let expected = service.acquire_display_pixels(ImageSource::Encoded(pq), ImageLimits::default(), spec(), &cancel).await.unwrap();
        for depth in [::png::BitDepth::Eight, ::png::BitDepth::Sixteen] {
            for narrow in [false, true] {
                // Unknown HLG peak defaults to 1000 nit, not PQ's 10000 nit.
                let bytes = source(true, depth, narrow, if peak == 10_000_000 { None } else { Some(&light) }, HLG.len());
                let expected_raw = image::load_from_memory(&bytes).unwrap().to_rgba8().into_raw();
                let encoded = service.import_encoded(bytes, &cancel).await.unwrap();
                let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.unwrap();
                let display = service.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), spec(), &cancel).await.unwrap();
                assert_eq!(display.pixels.alpha(), AlphaMode::Premultiplied);
                for (index, (actual, expected)) in display.pixels.bytes().iter().zip(expected.pixels.bytes()).enumerate() {
                    assert!(actual.abs_diff(*expected) <= if index % 4 == 3 { 0 } else { 3 },
                        "{depth:?}, narrow={narrow}, peak={peak}, byte {index}: {actual} != {expected}");
                }
                assert_eq!(raw.bytes(), expected_raw, "display mapping must preserve canonical samples");
                assert_eq!(service.memory().scratch, 0);
            }
        }
    }
    service.shutdown().await.unwrap();
    storage.close_for_test().await.unwrap();
}

#[tokio::test]
async fn hlg_low_peak_preserves_reference_brightness_and_bad_metadata_only_blocks_display() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("hlg-metadata.sqlite3")).await.unwrap();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).unwrap();
    let cancel = CancellationToken::new();
    let light = clli(1_000_000);
    let encoded = service.import_encoded(source(true, ::png::BitDepth::Sixteen, false, Some(&light), 2), &cancel).await.unwrap();
    let display = service.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), spec(), &cancel).await.unwrap();
    let linear = NITS[1][0] / 203.;
    let expected = ((1.055 * linear.powf(1. / 2.4) - 0.055) * 128.).round() as u8;
    assert_eq!(display.pixels.bytes()[7], 128);
    for value in &display.pixels.bytes()[4..7] { assert!(value.abs_diff(expected) <= 1); }
    for invalid in [vec![0], clli(100_000_001)] {
        let encoded = service.import_encoded(source(true, ::png::BitDepth::Sixteen, false, Some(&invalid), HLG.len()), &cancel).await.unwrap();
        let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.unwrap();
        assert_eq!(raw.width(), HLG.len() as u32);
        assert!(service.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), spec(), &cancel).await.is_err());
    }
    assert_eq!(service.memory().scratch, 0);
    service.shutdown().await.unwrap();
    storage.close_for_test().await.unwrap();
}
