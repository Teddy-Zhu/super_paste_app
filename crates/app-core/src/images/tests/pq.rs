use super::*;

// Encode physical luminance into fixtures; display expectations below use
// independent SDR luminance, not the tone-mapper's own output as an oracle.
pub(super) fn signal(nits: f64, maximum: f64, narrow: bool) -> u16 {
    let power = (nits / 10_000.).powf(2610. / 16384.);
    let pq = ((3424. / 4096. + 2413. / 128. * power) / (1. + 2392. / 128. * power)).powf(2523. / 32.);
    (if narrow {
        if maximum > 255. { 4096. + 56064. * pq } else { 16. + 219. * pq }
    } else { pq * maximum }).round() as u16
}

fn source(depth: ::png::BitDepth, narrow: bool, lights: &[f64], chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = ::png::Encoder::new(&mut bytes, lights.len() as u32, 1);
        encoder.set_color(::png::ColorType::Rgba);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        writer.write_chunk(::png::chunk::ChunkType(*b"cICP"), &[9, 16, 0, u8::from(!narrow)]).unwrap();
        for (kind, data) in chunks { writer.write_chunk(::png::chunk::ChunkType(**kind), data).unwrap(); }
        let mut pixels = Vec::new();
        for (index, nits) in lights.iter().enumerate() {
            let maximum = if depth == ::png::BitDepth::Sixteen { 65535. } else { 255. };
            let sample = signal(*nits, maximum, narrow);
            let alpha = if index == 1 { (128 * maximum as u32 / 255) as u16 } else { maximum as u16 };
            for value in [sample, sample, sample, alpha] {
                if depth == ::png::BitDepth::Sixteen { pixels.extend_from_slice(&value.to_be_bytes()); } else { pixels.push(value as u8); }
            }
        }
        writer.write_image_data(&pixels).unwrap();
    }
    bytes
}

fn clli(peak: u32, average: u32) -> Vec<u8> { [peak.to_be_bytes(), average.to_be_bytes()].concat() }
fn mastering(peak: u32) -> Vec<u8> {
    let mut bytes = vec![0; 24];
    bytes[16..20].copy_from_slice(&peak.to_be_bytes());
    bytes
}

fn spec() -> PixelSpec { PixelSpec { max_width: 16, max_height: 1, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest } }

#[tokio::test]
async fn pq_display_preserves_physical_luminance_alpha_and_canonical_pixels() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("pq.sqlite3")).await.unwrap();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).unwrap();
    let cancel = CancellationToken::new();
    for depth in [::png::BitDepth::Eight, ::png::BitDepth::Sixteen] {
        for narrow in [false, true] {
            let lights = [0., 10., 100.];
            let bytes = source(depth, narrow, &lights, &[(b"cLLI", clli(1_000_000, 500_000))]);
            let expected_raw = image::load_from_memory(&bytes).unwrap().to_rgba8().into_raw();
            let encoded = service.import_encoded(bytes, &cancel).await.unwrap();
            let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.unwrap();
            assert_eq!(raw.bytes(), expected_raw);
            let display = service.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), spec(), &cancel).await.unwrap();
            assert_eq!(display.pixels.alpha(), AlphaMode::Premultiplied);
            for (index, pixel) in display.pixels.bytes().chunks_exact(4).enumerate() {
                let linear = lights[index] / 203.;
                let gamma = if linear <= 0.0031308 { linear * 12.92 } else { 1.055 * linear.powf(1. / 2.4) - 0.055 };
                let alpha = if index == 1 { 128 } else { 255 };
                let expected = (gamma * f64::from(alpha)).round() as u8;
                assert_eq!(pixel[3], alpha);
                for value in &pixel[..3] { assert!(value.abs_diff(expected) <= 3, "{depth:?}, narrow={narrow}, {lights:?}: {pixel:?} != {expected}"); }
            }
            assert_eq!(raw.bytes(), expected_raw, "display mapping must not alter OCR, QR or protocol samples");
            assert_eq!(service.memory().scratch, 0);
        }
    }
    service.shutdown().await.unwrap();
    storage.close_for_test().await.unwrap();
}

#[tokio::test]
async fn pq_peak_metadata_selects_mapping_and_invalid_display_keeps_original_data() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("pq-metadata.sqlite3")).await.unwrap();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).unwrap();
    let cancel = CancellationToken::new();
    let lights = [0., 10., 100., 400., 1000.];
    let mut outputs = Vec::new();
    for chunks in [
        vec![(b"cLLI", clli(10_000_000, 2_000_000))],
        vec![(b"mDCV", mastering(10_000_000))],
        vec![(b"cLLI", clli(0, 0)), (b"mDCV", mastering(10_000_000))],
        vec![(b"cLLI", clli(10_000_000, 2_000_000)), (b"mDCV", vec![0])],
        vec![],
    ] {
        let encoded = service.import_encoded(source(::png::BitDepth::Sixteen, false, &lights, &chunks), &cancel).await.unwrap();
        let display = service.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), spec(), &cancel).await.unwrap();
        let pixels = display.pixels.bytes().to_vec();
        assert_eq!(&pixels[..4], &[0, 0, 0, 255]);
        assert!(pixels.chunks_exact(4).enumerate().all(|(index, p)| p[0] == p[1] && p[1] == p[2] && p[3] == if index == 1 { 128 } else { 255 }));
        assert!(pixels[8] < pixels[12] && pixels[12] <= pixels[16], "highlights remain ordered: {pixels:?}");
        outputs.push(pixels);
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[0], outputs[2]);
    assert_eq!(outputs[0], outputs[3], "chosen content peak makes malformed lower-priority mastering metadata irrelevant");
    assert_ne!(outputs[0], outputs[4], "unknown peak uses the PQ ceiling, not a guessed content brightness");
    for invalid in [vec![0], clli(1_000_000, 2_000_000), clli(100_000_001, 0)] {
        let bytes = source(::png::BitDepth::Sixteen, false, &lights, &[(b"cLLI", invalid)]);
        let encoded = service.import_encoded(bytes, &cancel).await.unwrap();
        let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.unwrap();
        assert_eq!(raw.width(), lights.len() as u32);
        assert!(service.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), spec(), &cancel).await.is_err());
    }
    assert_eq!(service.memory().scratch, 0);
    service.shutdown().await.unwrap();
    storage.close_for_test().await.unwrap();
}
