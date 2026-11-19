use super::*;

fn segment(output: &mut Vec<u8>, marker: u8, payload: &[u8]) {
    output.extend_from_slice(&[0xff, marker]);
    output.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
    output.extend_from_slice(payload);
}

fn with_metadata(jpeg: &[u8], profile: &[u8], orientation: Option<u8>) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    let mut output = jpeg[..2].to_vec();
    if let Some(orientation) = orientation {
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&[b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0,
            1, 0, 0, 0, orientation, 0, 0, 0, 0, 0, 0, 0]);
        segment(&mut output, 0xe1, &payload);
    }
    // Physical APP2 order need not match the profile's sequence numbers.
    let split = profile.len().div_ceil(3);
    let chunks = profile.chunks(split).collect::<Vec<_>>();
    for (index, chunk) in chunks.iter().enumerate().rev() {
        let mut payload = b"ICC_PROFILE\0".to_vec();
        payload.extend_from_slice(&[u8::try_from(index + 1).unwrap(), u8::try_from(chunks.len()).unwrap()]);
        payload.extend_from_slice(chunk);
        segment(&mut output, 0xe2, &payload);
    }
    output.extend_from_slice(&jpeg[2..]);
    output
}

fn linear_srgb() -> moxcms::ColorProfile {
    let mut profile = moxcms::ColorProfile::new_srgb();
    profile.cicp = None;
    let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
    profile.red_trc = Some(curve.clone());
    profile.green_trc = Some(curve.clone());
    profile.blue_trc = Some(curve);
    profile
}

fn srgb(sample: u8) -> u8 {
    let linear = f64::from(sample) / 255.;
    let value = if linear <= 0.003_130_8 { linear * 12.92 }
        else { 1.055 * linear.powf(1. / 2.4) - 0.055 };
    (value * 255.).round() as u8
}

// A small four-input ICC LUT maps conventional ink to linear sRGB's D50
// XYZ colorants. Its vertices have independent analytic black/white/primary
// expectations; it is not a printer profile or another production converter.
fn cmyk_profile() -> Vec<u8> {
    use moxcms::{ColorProfile, DataColorSpace, LutDataType, LutStore, LutType, LutWarehouse, Matrix3d, ProfileClass};
    let mut profile = ColorProfile::new_srgb();
    let mut clut = Vec::new();
    for c in [0., 1.] { for m in [0., 1.] { for y in [0., 1.] { for k in [0., 1.] {
        let rgb = [(1. - c) * (1. - k), (1. - m) * (1. - k), (1. - y) * (1. - k)];
        for xyz in [
            [profile.red_colorant.x, profile.green_colorant.x, profile.blue_colorant.x],
            [profile.red_colorant.y, profile.green_colorant.y, profile.blue_colorant.y],
            [profile.red_colorant.z, profile.green_colorant.z, profile.blue_colorant.z],
        ] {
            let value = xyz.into_iter().zip(rgb).map(|(coefficient, sample)| coefficient * sample).sum::<f64>();
            // ICC lut16 XYZ uses unsigned 1.15 fixed-point storage.
            clut.push((value * 32768.).round().clamp(0., 65535.) as u16);
        }
    } } } }
    profile.color_space = DataColorSpace::Cmyk;
    profile.profile_class = ProfileClass::OutputDevice;
    profile.cicp = None;
    profile.red_trc = None;
    profile.green_trc = None;
    profile.blue_trc = None;
    profile.lut_a_to_b_perceptual = Some(LutWarehouse::Lut(LutDataType {
        num_input_channels: 4, num_output_channels: 3, num_clut_grid_points: 2,
        matrix: Matrix3d::IDENTITY, num_input_table_entries: 2, num_output_table_entries: 2,
        input_table: LutStore::Store16([0, 65535].repeat(4)),
        clut_table: LutStore::Store16(clut), output_table: LutStore::Store16([0, 65535].repeat(3)),
        lut_type: LutType::Lut16,
    }));
    profile.encode().unwrap()
}

#[tokio::test]
async fn rgb_and_gray_jpeg_profiles_and_exif_only_change_display() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("jpeg-color.sqlite3")).await.unwrap();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).unwrap();
    let cancel = CancellationToken::new();
    let spec = PixelSpec { max_width: 8, max_height: 8, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
    for gray in [false, true] {
        let levels = [40, 75, 110, 145, 180, 215];
        let samples = if gray { levels.to_vec() } else { levels.into_iter().flat_map(|v| [v, 255 - v, v / 2]).collect() };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
            .encode(&samples, 2, 3, if gray { image::ExtendedColorType::L8 } else { image::ExtendedColorType::Rgb8 }).unwrap();
        let reference = image::load_from_memory(&jpeg).unwrap().into_rgba8();
        let profile = if gray { moxcms::ColorProfile::new_gray_with_gamma(1.) } else { linear_srgb() }.encode().unwrap();
        let bytes = with_metadata(&jpeg, &profile, Some(6));
        let identity = Digest(Sha256::digest(&bytes).into());
        let encoded = service.import_encoded(bytes.clone(), &cancel).await.unwrap();
        let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.unwrap();
        let display = service.acquire_display_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), spec, &cancel).await.unwrap();
        assert_eq!((raw.width(), raw.height()), (2, 3));
        assert_eq!(raw.bytes(), reference.as_raw());
        assert_eq!(display.natural_dimensions, (3, 2));
        assert_eq!((display.pixels.width(), display.pixels.height()), (3, 2));
        for (pixel, source) in display.pixels.bytes().chunks_exact(4).zip([4, 2, 0, 5, 3, 1]) {
            assert_eq!(pixel[3], 255);
            for channel in 0..3 {
                let expected = srgb(reference.as_raw()[source * 4 + channel]);
                assert!(pixel[channel].abs_diff(expected) <= 3,
                    "gray={gray}, source={source}, channel={channel}, expected={expected}, pixel={pixel:?}");
            }
        }
        let repeated = service.acquire_display_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), spec, &cancel).await.unwrap();
        assert!(Arc::ptr_eq(&display.pixels, &repeated.pixels));
        assert_eq!(encoded.id(), identity);
        assert_eq!(encoded.bytes(), bytes);
        let png = service.encode_png(raw.clone(), &cancel).await.unwrap();
        let normalized = service.acquire_display_pixels(ImageSource::Encoded(png), ImageLimits::default(), spec, &cancel).await.unwrap();
        assert_eq!(normalized.natural_dimensions, (2, 3));
        assert_eq!(normalized.pixels.bytes(), raw.bytes(), "normalization does not carry JPEG display metadata");
    }
    assert_eq!(service.memory().scratch, 0);
    service.shutdown().await.unwrap();
    assert_eq!(service.memory().pixels, 0);
    assert_eq!(service.memory().encoded, 0);
    storage.close_for_test().await.unwrap();
}

#[tokio::test]
async fn baseline_and_progressive_cmyk_keep_ink_and_profile_out_of_canonical_pixels() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("cmyk-color.sqlite3")).await.unwrap();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).unwrap();
    let cancel = CancellationToken::new();
    let spec = PixelSpec { max_width: 8, max_height: 8, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
    let profile = cmyk_profile();
    for jpeg in [include_bytes!("fixtures/cmyk-baseline.jpg").as_slice(), include_bytes!("fixtures/cmyk-progressive.jpg").as_slice()] {
        let reference = image::load_from_memory(jpeg).unwrap().into_rgba8();
        for converted in [false, true] {
            let bytes = if converted { with_metadata(jpeg, &profile, None) } else { jpeg.to_vec() };
            let encoded = service.import_encoded(bytes.clone(), &cancel).await.unwrap();
            let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.unwrap();
            let display = service.acquire_display_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), spec, &cancel).await.unwrap();
            assert_eq!(raw.bytes(), reference.as_raw(), "retained CMYK matches the codec's canonical RGB rounding");
            assert_eq!(display.natural_dimensions, (3, 2));
            let expected = [[255, 255, 255], [0, 0, 0], [0, 255, 255], [255, 0, 255], [255, 255, 0],
                if converted { [187, 187, 187] } else { [127, 127, 127] }];
            for (pixel, expected) in display.pixels.bytes().chunks_exact(4).zip(expected) {
                assert_eq!(pixel[3], 255, "K is ink, never alpha");
                assert!(pixel[..3].iter().zip(expected).all(|(value, expected)| value.abs_diff(expected) <= 5),
                    "ICC={converted}, expected={expected:?}, pixel={pixel:?}");
            }
            assert_eq!(encoded.bytes(), bytes);
        }
    }
    assert_eq!(service.memory().scratch, 0);
    service.shutdown().await.unwrap();
    assert_eq!(service.memory().pixels, 0);
    assert_eq!(service.memory().encoded, 0);
    storage.close_for_test().await.unwrap();
}
