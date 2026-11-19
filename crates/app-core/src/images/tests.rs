use std::path::PathBuf;
use image::ImageEncoder;
use super::*;

mod pq;
mod hlg;
mod png_color_fallback;
mod jpeg_color;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("super-paste-rs-images-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).expect("isolated image database directory");
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).expect("remove isolated image database"); }
}

fn png() -> Vec<u8> {
    let mut data = Vec::new();
    image::codecs::png::PngEncoder::new(&mut data)
        .write_image(&[255, 0, 0, 128], 1, 1, image::ExtendedColorType::Rgba8).expect("fixture PNG");
    data
}

#[tokio::test]
async fn display_metadata_does_not_change_canonical_pixels_or_normalized_pngs() {
    fn source_png(width: u32, height: u32, depth: ::png::BitDepth, pixels: &[u8], orientation: Option<u8>, linear: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = ::png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(::png::ColorType::Rgba);
            encoder.set_depth(depth);
            if linear { encoder.set_source_gamma(::png::ScaledFloat::new(1.0)); }
            let mut writer = encoder.write_header().expect("PNG header");
            if let Some(orientation) = orientation {
                // A complete little-endian TIFF IFD with one Orientation tag.
                let exif = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, orientation, 0, 0, 0, 0, 0, 0, 0];
                writer.write_chunk(::png::chunk::ChunkType(*b"eXIf"), &exif).expect("orientation metadata");
            }
            writer.write_image_data(pixels).expect("PNG pixels");
        }
        bytes
    }
    fn srgb(value: f64) -> f64 {
        if value <= 0.003_130_8 { value * 12.92 } else { 1.055 * value.powf(1.0 / 2.4) - 0.055 }
    }

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let directory = Directory::new();
        let storage = Storage::open(&directory.0.join("display-metadata.sqlite3")).await.expect("isolated database");
        let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).expect("image service");
        let cancel = CancellationToken::new();
        let spec = PixelSpec { max_width: 8, max_height: 8, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
        let source = [1_u8, 2, 3, 4, 5, 6].into_iter().flat_map(|value| [value, 0, 0, 255]).collect::<Vec<_>>();
        let orientations = [
            (1, (2, 3), [1, 2, 3, 4, 5, 6]),
            (2, (2, 3), [2, 1, 4, 3, 6, 5]),
            (3, (2, 3), [6, 5, 4, 3, 2, 1]),
            (4, (2, 3), [5, 6, 3, 4, 1, 2]),
            (5, (3, 2), [1, 3, 5, 2, 4, 6]),
            (6, (3, 2), [5, 3, 1, 6, 4, 2]),
            (7, (3, 2), [6, 4, 2, 5, 3, 1]),
            (8, (3, 2), [2, 4, 6, 1, 3, 5]),
        ];
        for (orientation, dimensions, order) in orientations {
            let bytes = source_png(2, 3, ::png::BitDepth::Eight, &source, Some(orientation), false);
            let original_id = Digest(Sha256::digest(&bytes).into());
            let encoded = service.import_encoded(bytes, &cancel).await.expect("oriented PNG");
            let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.expect("canonical data");
            let displayed = service.acquire_display_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), spec, &cancel).await.expect("oriented display");
            assert_eq!(displayed.natural_dimensions, dimensions);
            assert_eq!((displayed.pixels.width(), displayed.pixels.height()), dimensions);
            assert_eq!(displayed.pixels.bytes().chunks_exact(4).map(|pixel| pixel[0]).collect::<Vec<_>>(), order);
            assert_eq!(raw.bytes(), source);
            assert_eq!(raw.alpha(), AlphaMode::Straight);
            let reread = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.expect("shared canonical data");
            assert_eq!(raw.bytes().as_ptr(), reread.bytes().as_ptr(), "canonical readers share the pixel allocation");
            assert!(Arc::ptr_eq(&raw._reservation, &reread._reservation), "canonical readers share one budget lease");
            let normalized = service.encode_png(raw, &cancel).await.expect("protocol PNG");
            let normalized_display = service.acquire_display_pixels(ImageSource::Encoded(normalized), ImageLimits::default(), spec, &cancel).await.expect("normalized display");
            assert_eq!(normalized_display.natural_dimensions, (2, 3), "normalization must not inherit original EXIF");
            assert_eq!(normalized_display.pixels.bytes(), source);
            assert_eq!(encoded.id(), original_id);
            assert_eq!(Digest(Sha256::digest(encoded.bytes()).into()), original_id);
        }

        for (depth, samples, maximum) in [
            (::png::BitDepth::Eight, [128_u16, 64, 32, 128], 255_u32),
            // The first 16-bit sample distinguishes conversion before/after
            // 8-bit quantization: its linear-light value must survive decode.
            (::png::BitDepth::Sixteen, [129, 26_000, 50_000, 32_768], 65_535),
        ] {
            let pixels = if depth == ::png::BitDepth::Eight { samples.iter().map(|value| *value as u8).collect::<Vec<_>>() }
                else { samples.iter().flat_map(|value| value.to_be_bytes()).collect() };
            let encoded = service.import_encoded(source_png(1, 1, depth, &pixels, None, true), &cancel).await.expect("linear-light PNG");
            let raw = service.acquire_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), &cancel).await.expect("canonical linear samples");
            let canonical = samples.map(|value| ((u32::from(value) * 255 + maximum / 2) / maximum) as u8);
            assert_eq!(raw.bytes(), canonical);
            let displayed = service.acquire_display_pixels(ImageSource::Encoded(encoded.clone()), ImageLimits::default(), spec, &cancel).await.expect("sRGB display");
            assert_eq!(displayed.pixels.alpha(), AlphaMode::Premultiplied);
            assert_eq!(displayed.pixels.bytes()[3], canonical[3]);
            for (channel, sample) in samples[..3].iter().enumerate() {
                let color = (srgb(f64::from(*sample) / f64::from(maximum)) * 255.0).round() as u16;
                let expected = ((color * u16::from(canonical[3]) + 127) / 255) as u8;
                assert!(displayed.pixels.bytes()[channel].abs_diff(expected) <= 1, "{depth:?}: channel {channel}, expected {expected}, got {:?}", displayed.pixels.bytes());
            }
            assert_eq!(raw.bytes(), canonical, "display must not mutate OCR/protocol data");
            let normalized = service.encode_png(raw, &cancel).await.expect("normalized color PNG");
            let normalized_display = service.acquire_display_pixels(ImageSource::Encoded(normalized), ImageLimits::default(), spec, &cancel).await.expect("normalized sRGB display");
            let mut expected = canonical;
            for component in &mut expected[..3] {
                *component = ((u16::from(*component) * u16::from(canonical[3]) + 127) / 255) as u8;
            }
            assert_eq!(normalized_display.pixels.bytes(), expected, "normalization must not reapply the original gamma");
        }
        {
            // Native macOS icons and SVG rasters arrive premultiplied. Data
            // normalization must restore their color before PNG persistence,
            // while native display can reuse the original allocation.
            let source = vec![128, 64, 32, 128, 0, 0, 0, 0];
            let native = service.import_pixels(2, 1, 8, PixelLayout::Rgba8, AlphaMode::Premultiplied, source.clone(), &cancel)
                .await.expect("native premultiplied pixels");
            let display = service.derive_display_pixels(native.clone(), spec, &cancel).await.expect("native display");
            assert!(Arc::ptr_eq(&native, &display), "premultiplied display does not need another conversion");
            let normalized = service.derive_pixels(native.clone(), spec, &cancel).await.expect("straight-alpha icon data");
            let expected = [255, 128, 64, 128, 0, 0, 0, 0];
            assert_eq!(normalized.alpha(), AlphaMode::Straight);
            assert_eq!(normalized.bytes(), expected);
            let png = service.encode_png(normalized, &cancel).await.expect("normalized native icon PNG");
            assert_eq!(image::load_from_memory(png.bytes()).expect("decode persisted PNG").to_rgba8().as_raw(), &expected);
            assert_eq!(native.bytes(), source, "persistence must not mutate native display pixels");
        }
        assert_eq!(service.memory().scratch, 0);
        service.shutdown().await.expect("shutdown images");
        assert_eq!(service.memory().encoded, 0);
        assert_eq!(service.memory().pixels, 0);
        storage.close_for_test().await.expect("close database");
    }).await.expect("metadata and canonical ownership finish within ten seconds");
}

#[tokio::test]
async fn detail_admission_preserves_fifo_and_returns_cancelled_grants() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("detail-admission.sqlite3")).await.expect("isolated database");
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets { workers: 1, ..Default::default() }).expect("image service");
    let cancel = CancellationToken::new();
    let held = service.worker_permit(&cancel).await.expect("hold sole worker");
    let mut first_card = Box::pin(service.worker_permit(&cancel));
    let mut second_card = Box::pin(service.worker_permit(&cancel));
    assert!(futures_util::poll!(first_card.as_mut()).is_pending());
    assert!(futures_util::poll!(second_card.as_mut()).is_pending());
    let mut first_detail = Box::pin(service.with_priority(ImagePriority::Interactive, service.worker_permit(&cancel)));
    let mut cancelled_detail = Box::pin(service.with_priority(ImagePriority::Interactive, service.worker_permit(&cancel)));
    assert!(futures_util::poll!(first_detail.as_mut()).is_pending());
    assert!(futures_util::poll!(cancelled_detail.as_mut()).is_pending());
    drop(held);
    assert!(futures_util::poll!(first_card.as_mut()).is_pending());
    let detail = first_detail.await.expect("detail goes ahead of queued cards");
    assert!(futures_util::poll!(cancelled_detail.as_mut()).is_pending(), "interactive requests retain FIFO order");
    drop(detail);
    drop(cancelled_detail); // Capacity was granted, but the consumer never polled again.
    assert!(futures_util::poll!(second_card.as_mut()).is_pending());
    drop(first_card.await.expect("first card resumes after cancelled grant"));
    drop(second_card.await.expect("second card retains its place"));
    assert_eq!(service.workers.available_permits(), 1);
    service.shutdown().await.expect("shutdown");
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn detail_join_promotes_shared_dependencies_without_restarting() {
    use std::{future::Future, sync::atomic::{AtomicUsize, Ordering}, task::Poll};
    struct Value(u8);
    impl Weighted for Value { fn weight(&self) -> u64 { 1 } }
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("shared-priority.sqlite3")).await.expect("isolated database");
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets { workers: 1, ..Default::default() }).expect("image service");
    // Cover both producer-joining and the fast path that only joins existing
    // canonical work. A nested flight represents a shared decode dependency.
    for existing_only in [false, true] {
        let outer = WorkCache::new(8); let inner = WorkCache::new(8);
        let started = Arc::new(AtomicUsize::new(0));
        let live = CancellationToken::new(); let abandoned = CancellationToken::new();
        let held = service.worker_permit(&live).await.expect("hold worker");
        let mut earlier_card = Box::pin(service.worker_permit(&live));
        assert!(futures_util::poll!(earlier_card.as_mut()).is_pending());
        let (queued, waiting) = tokio::sync::oneshot::channel();
        let first = tokio::spawn({ let service = service.clone(); let outer = outer.clone(); let started = started.clone(); let cancel = abandoned.clone(); async move {
            service.cached(&outer, 1_u8, true, &cancel, move |service, cancel| async move {
                let value = service.cached(&inner, 2_u8, true, &cancel, move |service, cancel| async move {
                    started.fetch_add(1, Ordering::SeqCst);
                    let mut admission = Box::pin(service.worker_permit(&cancel));
                    std::future::poll_fn(|cx| {
                        assert!(admission.as_mut().poll(cx).is_pending());
                        Poll::Ready(())
                    }).await;
                    queued.send(()).expect("report queued dependency");
                    let _permit = admission.await?;
                    Ok(Value(7))
                }).await?;
                Ok(Value(value.0))
            }).await
        }});
        waiting.await.expect("shared dependency entered the worker queue");
        let detail_work = async {
            if existing_only { outer.get_existing(&1, &live).await?.ok_or_else(|| Error::Task("共享测试任务缺失".into())) }
            else { service.cached(&outer, 1_u8, true, &live, |_, _| async { panic!("joined work must not restart") }).await }
        };
        let mut detail = Box::pin(service.with_priority(ImagePriority::Interactive, detail_work));
        assert!(futures_util::poll!(detail.as_mut()).is_pending());
        abandoned.cancel();
        assert!(matches!(first.await.expect("original observer returns"), Err(Error::Cancelled)));
        drop(held);
        assert!(futures_util::poll!(earlier_card.as_mut()).is_pending(), "the promoted dependency overtakes earlier card work");
        assert_eq!(detail.await.expect("detail keeps shared work alive").0, 7);
        drop(earlier_card.await.expect("card resumes"));
        assert_eq!(started.load(Ordering::SeqCst), 1);
    }
    service.shutdown().await.expect("shutdown");
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn speculative_admission_yields_to_readers_and_promotes_existing_work() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("prefetch-admission.sqlite3")).await.expect("isolated database");
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets { workers: 1, ..Default::default() }).expect("image service");
    let cancel = CancellationToken::new(); let needed = CancellationToken::new();
    let held = service.worker_permit(&cancel).await.expect("hold sole worker");
    let mut speculative = Box::pin(service.idle_worker_permit(&cancel, &needed));
    assert!(futures_util::poll!(speculative.as_mut()).is_pending());
    let mut reader = Box::pin(service.worker_permit(&cancel));
    assert!(futures_util::poll!(reader.as_mut()).is_pending());
    drop(held);
    assert!(futures_util::poll!(speculative.as_mut()).is_pending(), "prefetch must yield even though it arrived first");
    let reading = reader.await.expect("visible reader gets released capacity");
    drop(reading);
    let prefetched = speculative.await.expect("idle work wakes when the reader finishes");
    drop(prefetched);

    let held = service.worker_permit(&cancel).await.expect("hold worker before promotion");
    let mut speculative = Box::pin(service.idle_worker_permit(&cancel, &needed));
    assert!(futures_util::poll!(speculative.as_mut()).is_pending());
    needed.cancel();
    assert!(futures_util::poll!(speculative.as_mut()).is_pending());
    let mut later_reader = Box::pin(service.worker_permit(&cancel));
    assert!(futures_util::poll!(later_reader.as_mut()).is_pending());
    drop(held);
    let demanded = speculative.await.expect("the same pending work now joins normal FIFO admission");
    assert!(futures_util::poll!(later_reader.as_mut()).is_pending());
    drop(demanded);
    drop(later_reader.await.expect("next reader follows promoted work"));

    let held = service.worker_permit(&cancel).await.expect("hold worker during cancellation");
    let cancelled = CancellationToken::new(); let idle = CancellationToken::new();
    let mut abandoned = Box::pin(service.idle_worker_permit(&cancelled, &idle));
    assert!(futures_util::poll!(abandoned.as_mut()).is_pending());
    cancelled.cancel();
    assert!(matches!(abandoned.await, Err(Error::Cancelled)));
    let mut stopped = Box::pin(service.idle_worker_permit(&cancel, &idle));
    assert!(futures_util::poll!(stopped.as_mut()).is_pending());
    service.shutdown().await.expect("stop idle admission");
    assert!(matches!(stopped.await, Err(Error::Cancelled)));
    drop(held);
    assert_eq!(service.memory().pixels, 0);
    assert_eq!(service.memory().scratch, 0);
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn shared_derivation_keeps_live_consumers_and_cancels_abandoned_admission() {
    use std::time::Duration;
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("shared-derive.sqlite3")).await.expect("isolated database");
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets { workers: 1, ..Default::default() }).expect("image service");
    let live = CancellationToken::new();
    let pixels = service.import_pixels(2, 2, 8, PixelLayout::Rgba8, AlphaMode::Straight, vec![255; 16], &live).await.expect("source pixels");
    let spec = PixelSpec { max_width: 1, max_height: 1, layout: PixelLayout::Gray8, filter: ResizeFilter::Nearest };
    let permit = service.workers.clone().acquire_owned().await.expect("hold sole worker");
    let cancel = CancellationToken::new();
    let mut first = Box::pin(service.derive_pixels(pixels.clone(), spec, &cancel));
    let mut second = Box::pin(service.derive_pixels(pixels.clone(), spec, &live));
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    assert!(futures_util::poll!(second.as_mut()).is_pending());
    cancel.cancel();
    assert!(matches!(first.await, Err(Error::Cancelled)));
    assert!(futures_util::poll!(second.as_mut()).is_pending(), "the live consumer still owns shared work");
    drop(permit);
    let derived = second.await.expect("remaining consumer finishes");
    assert_eq!(derived.bytes(), &[255]);
    assert!(Arc::ptr_eq(&derived, &service.derive_pixels(pixels.clone(), spec, &live).await.expect("reuse successful result")));

    let permit = service.workers.clone().acquire_owned().await.expect("hold worker for abandoned request");
    let cancel = CancellationToken::new();
    let mut abandoned = Box::pin(service.derive_pixels(pixels.clone(), PixelSpec { max_width: 2, max_height: 2, ..spec }, &cancel));
    assert!(futures_util::poll!(abandoned.as_mut()).is_pending());
    cancel.cancel();
    assert!(matches!(abandoned.await, Err(Error::Cancelled)));
    tokio::time::timeout(Duration::from_secs(5), service.shutdown()).await.expect("cancel queued work without a worker permit").expect("shutdown");
    assert_eq!(service.tasks.len(), 0);
    assert_eq!(service.memory().scratch, 0);
    assert!(matches!(service.acquire_pixels(ImageSource::Pixels(pixels.clone()), ImageLimits::default(), &live).await, Err(Error::Cancelled)));
    drop((permit, pixels, derived));
    assert_eq!(service.memory().pixels, 0);
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn abandoned_blocking_work_keeps_budget_permit_and_shutdown_ownership() {
    use std::{future::Future, task::Poll, time::Duration};
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("physical-work.sqlite3")).await.expect("isolated database");
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets { workers: 1, ..Default::default() }).expect("image service");
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let cancel = CancellationToken::new();
    let job = tokio::spawn({ let service = service.clone(); let cancel = cancel.clone(); async move {
        service.run_cpu(128, &cancel, move || {
            let _ = entered.send(());
            blocked.recv_timeout(Duration::from_secs(5)).expect("release physical work");
            Ok(())
        }).await
    }});
    ready.await.expect("physical work entered");
    cancel.cancel(); job.abort();
    assert!(job.await.expect_err("async observer aborted").is_cancelled());
    assert_eq!(service.workers.available_permits(), 0);
    assert_eq!(service.memory().scratch, 128);
    assert_eq!(service.tasks.len(), 1);
    let mut shutdown = Box::pin(service.shutdown());
    std::future::poll_fn(|cx| {
        assert!(shutdown.as_mut().poll(cx).is_pending(), "shutdown must await physical work");
        Poll::Ready(())
    }).await;
    release.send(()).expect("release blocking job");
    tokio::time::timeout(Duration::from_secs(5), shutdown).await.expect("physical work drains").expect("shutdown");
    assert_eq!(service.memory().scratch, 0);
    assert_eq!(service.tasks.len(), 0);
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn shutdown_releases_animation_codec_with_live_display_handles() {
    use std::time::Duration;
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("animation-stop.sqlite3")).await.expect("isolated database");
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets::default()).expect("image service");
    let mut data = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut data);
        encoder.set_repeat(image::codecs::gif::Repeat::Infinite).expect("repeat animation");
        for value in [0, 255] {
            encoder.encode_frame(image::Frame::from_parts(image::RgbaImage::from_pixel(2, 2, image::Rgba([value, value, value, 255])), 0, 0,
                image::Delay::from_numer_denom_ms(100, 1))).expect("encode tiny animation");
        }
    }
    let cancel = CancellationToken::new();
    let encoded = service.import_encoded(data, &cancel).await.expect("animation input");
    let spec = PixelSpec { max_width: 2, max_height: 2, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
    let frame = service.acquire_display_frame(encoded.clone(), None, ImageLimits::default(), spec, &cancel).await.expect("first animation frame");
    let animation = frame.animation.as_ref().expect("live animation").clone();
    let playing = animation.poll(frame.next_frame, spec, ImageLimits::default(), &cancel).await.expect("start shared frame prefetch");
    assert!(playing.next_frame.is_some());
    assert!(service.memory().scratch > 0, "codec owns decoder work");
    tokio::time::timeout(Duration::from_secs(5), service.shutdown()).await.expect("stop codec despite live handles").expect("shutdown");
    assert_eq!(service.tasks.len(), 0);
    assert_eq!(service.memory().scratch, 0);
    assert!(matches!(animation.poll(frame.next_frame, spec, ImageLimits::default(), &cancel).await, Err(Error::Cancelled)));
    drop((animation, frame, playing, encoded));
    assert_eq!(service.memory().pixels, 0);
    assert_eq!(service.memory().encoded, 0);
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn encoded_ingress_reuses_full_pool_without_copying_or_losing_capacity_accounting() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("images.sqlite3")).await.expect("isolated database");
    let png = png();
    let mut owned = Vec::with_capacity(png.len() * 4);
    owned.extend_from_slice(&png);
    let allocation = owned.capacity() as u64;
    let pointer = owned.as_ptr();
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets {
        encoded_bytes: allocation, retained_encoded_bytes: allocation,
        scratch_bytes: allocation * 2, workers: 1, ..Default::default()
    }).expect("bounded image service");
    let cancel = CancellationToken::new();
    let original = service.import_encoded(owned, &cancel).await.expect("admit original allocation");
    assert_eq!(original.bytes(), png);
    assert_eq!(original.bytes().as_ptr(), pointer);
    assert_eq!(original.dimensions(), Some((1, 1)));
    assert_eq!(original.format(), Some(ImageFormat::Png));
    assert_eq!(service.memory().encoded, allocation);
    assert_eq!(service.memory().scratch, 0);

    // The persistent pool is full. Both public byte entry points must still
    // converge on the live original without reserving another encoded copy.
    let (owned_again, shared_again) = tokio::join!(
        service.import_encoded(png.clone(), &cancel),
        service.import_shared_encoded(Bytes::from(png.clone()), &cancel),
    );
    let owned_again = owned_again.expect("duplicate owned input");
    let shared_again = shared_again.expect("duplicate shared input");
    assert!(Arc::ptr_eq(&original, &owned_again));
    assert!(Arc::ptr_eq(&original, &shared_again));
    let prepared = service.import_shared_input(EncodedImageInput::new(Bytes::from(png.clone())), &cancel).await.expect("prepared input");
    assert!(Arc::ptr_eq(&original, &prepared));

    // A protocol decoder transfers its lease, including while the scratch pool
    // is full. A second ingress reservation would reject this same allocation.
    let mut transferred = Vec::with_capacity((allocation * 2) as usize);
    transferred.extend_from_slice(&png);
    let ingress = service.reserve_consumer_copy(transferred.capacity() as u64).expect("full scratch pool");
    let transferred = service.import_reserved_encoded(transferred, ingress, &cancel).await.expect("transfer existing ingress lease");
    assert!(Arc::ptr_eq(&original, &transferred));
    assert_eq!(service.memory().scratch, 0);

    let mut different = png;
    different[0] ^= 1;
    assert!(matches!(service.import_encoded(different, &cancel).await, Err(Error::ResourceLimit(_))));
    assert_eq!(service.memory().encoded, allocation);
    assert_eq!(service.memory().scratch, 0);

    cancel.cancel();
    assert!(matches!(service.acquire_encoded(ImageSource::Encoded(original.clone()), &cancel).await, Err(Error::Cancelled)));
    drop((original, owned_again, shared_again, prepared, transferred));
    service.trim().expect("release retained bytes");
    assert_eq!(service.memory().encoded, 0);
    storage.close_for_test().await.expect("close isolated database");
}

#[tokio::test]
async fn cancelled_ingress_releases_queued_buffers_and_skips_cached_fast_paths() {
    let directory = Directory::new();
    let storage = Storage::open(&directory.0.join("cancel.sqlite3")).await.expect("isolated database");
    let service = ImageService::new(Handle::current(), storage.clone(), ImageBudgets { workers: 1, ..Default::default() })
        .expect("image service");
    let permit = service.workers.clone().acquire_owned().await.expect("hold sole worker");
    let cancel = CancellationToken::new();
    let encoded = service.import_encoded(png(), &cancel);
    tokio::pin!(encoded);
    assert!(futures_util::poll!(encoded.as_mut()).is_pending());
    assert!(service.memory().scratch > 0);
    assert_eq!(service.memory().encoded, 0);
    cancel.cancel();
    assert!(matches!(encoded.await, Err(Error::Cancelled)));
    assert_eq!(service.memory().scratch, 0);

    let cancel = CancellationToken::new();
    let input = EncodedImageInput::new(Bytes::from(png()));
    let prepared = service.import_shared_input(input.clone(), &cancel);
    tokio::pin!(prepared);
    assert!(futures_util::poll!(prepared.as_mut()).is_pending());
    cancel.cancel();
    assert!(matches!(prepared.await, Err(Error::Cancelled)));
    assert!(input.digest.get().is_none(), "cancelled queued input must not be hashed");

    let cancel = CancellationToken::new();
    let pixels = service.import_pixels(1, 1, 4, PixelLayout::Rgba8, AlphaMode::Straight, vec![0, 0, 0, 255], &cancel);
    tokio::pin!(pixels);
    assert!(futures_util::poll!(pixels.as_mut()).is_pending());
    assert_eq!(service.memory().pixels, 4);
    cancel.cancel();
    assert!(matches!(pixels.await, Err(Error::Cancelled)));
    assert_eq!(service.memory().pixels, 0);
    drop(permit);

    let active = CancellationToken::new();
    let pixels = service.import_pixels(1, 1, 4, PixelLayout::Rgba8, AlphaMode::Straight, vec![0, 0, 0, 255], &active).await.expect("live pixel input");
    let spec = PixelSpec { max_width: 1, max_height: 1, layout: PixelLayout::Rgba8, filter: ResizeFilter::Nearest };
    assert!(matches!(service.acquire_pixels(ImageSource::Pixels(pixels.clone()), ImageLimits::default(), &cancel).await, Err(Error::Cancelled)));
    assert!(matches!(service.derive_pixels(pixels, spec, &cancel).await, Err(Error::Cancelled)));
    storage.close_for_test().await.expect("close isolated database");
}
