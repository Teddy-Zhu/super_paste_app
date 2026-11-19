use super::*;

fn source(depth: ::png::BitDepth, codes: [u8; 4], icc: Option<&[u8]>, gamma: Option<f32>) -> Vec<u8> {
    let mut info = ::png::Info::with_size(1, 1);
    info.color_type = ::png::ColorType::Rgba;
    info.bit_depth = depth;
    info.icc_profile = icc.map(std::borrow::Cow::Borrowed);
    let mut bytes = Vec::new();
    {
        let mut encoder = ::png::Encoder::with_info(&mut bytes, info).unwrap();
        if let Some(gamma) = gamma { encoder.set_source_gamma(::png::ScaledFloat::new(gamma)); }
        let mut writer = encoder.write_header().unwrap();
        writer.write_chunk(::png::chunk::ChunkType(*b"cICP"), &codes).unwrap();
        let pixels = if depth == ::png::BitDepth::Sixteen {
            [128_u16, 64, 32, 128].into_iter().flat_map(|value| (value * 257).to_be_bytes()).collect::<Vec<_>>()
        } else { vec![128, 64, 32, 128] };
        writer.write_image_data(&pixels).unwrap();
    }
    bytes
}

#[tokio::test]
async fn unsupported_png_color_uses_existing_fallback_priority_and_preserves_source() {
    let mut linear = moxcms::ColorProfile::new_srgb();
    linear.cicp = None;
    let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
    linear.red_trc = Some(curve.clone());
    linear.green_trc = Some(curve.clone());
    linear.blue_trc = Some(curve);
    let profile = linear.encode().unwrap();
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("png-color-fallback.sqlite3")).await.unwrap();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).unwrap();
    let cancel = CancellationToken::new();
    let spec = PixelSpec { max_width: 1, max_height: 1, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
    for depth in [::png::BitDepth::Eight, ::png::BitDepth::Sixteen] {
        for (codes, icc, gamma, uses_linear) in [
            ([10, 17, 0, 1], None, None, false),
            ([10, 17, 0, 1], None, Some(1.), true),
            // Chosen ICC overrides a conflicting fallback gamma.
            ([10, 17, 0, 1], Some(profile.as_slice()), Some(0.45455), true),
            ([0, 17, 0, 1], Some(profile.as_slice()), None, true),
            // Understood sRGB overrides even an invalid lower-priority ICC.
            ([1, 13, 0, 1], Some(b"invalid ICC".as_slice()), Some(1.), false),
        ] {
            let bytes = source(depth, codes, icc, gamma);
            let encoded = service.import_encoded(bytes.clone(), &cancel).await.unwrap();
            let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.unwrap();
            let display = service.acquire_display_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), spec, &cancel).await.unwrap();
            assert_eq!(raw.bytes(), &[128, 64, 32, 128]);
            assert_eq!(encoded.bytes(), bytes);
            assert_eq!(display.pixels.alpha(), AlphaMode::Premultiplied);
            assert_eq!(display.pixels.bytes()[3], 128);
            for (index, sample) in [128., 64., 32.].into_iter().enumerate() {
                let value: f64 = sample / 255.;
                let gamma = if uses_linear { 1.055 * value.powf(1. / 2.4) - 0.055 } else { value };
                let expected = (gamma * 128.).round() as u8;
                assert!(display.pixels.bytes()[index].abs_diff(expected) <= 1,
                    "{depth:?}, codes={codes:?}, linear={uses_linear}: {:?}", display.pixels.bytes());
            }
        }
        let encoded = service.import_encoded(source(depth, [10, 17, 0, 1], Some(b"invalid ICC"), None), &cancel).await.unwrap();
        assert!(service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.is_ok());
        assert!(service.acquire_display_pixels(ImageSource::Encoded(encoded), ImageLimits::default(), spec, &cancel).await.is_err(),
            "an eligible but malformed ICC remains an error; unsupported cICP must not mask it");
    }
    assert_eq!(service.memory().scratch, 0);
    service.shutdown().await.unwrap();
    storage.close_for_test().await.unwrap();
}
