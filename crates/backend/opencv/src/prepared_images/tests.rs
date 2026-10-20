use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak, mpsc};
use std::time::Duration;

use mado_pilot_capture::{
    CpuMapping, CpuPixels, Frame, FrameDescriptor, FrameStorage, PixelFormat,
};
use mado_pilot_core::{OperationContext, PixelExtent, Result, Status};
use mado_pilot_testkit::{ControlledProducer, vision_contract};
use opencv::core::MatTraitConstManual;

use super::{BYTE_LIMIT, CachedImage, Lookup, PreparedImages, PreparedRegion};
use crate::image;

fn mapped(extent: PixelExtent, value: u8) -> (Frame, CpuMapping) {
    let frame = vision_contract::frame(extent, PixelFormat::Bgra8, value);
    let pixels = frame
        .full_view()
        .expect("view")
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("mapping");
    (frame, pixels)
}

fn converted(pixels: &CpuMapping) -> PreparedRegion {
    PreparedRegion::new(image::region_to_bgr(pixels).expect("BGR conversion"), true)
}

fn hit(cache: &PreparedImages, pixels: &CpuMapping) -> PreparedRegion {
    match cache.lookup(pixels) {
        Lookup::Hit(image) => PreparedRegion::Shared(image),
        _ => panic!("expected retained preparation"),
    }
}

fn weak(region: &PreparedRegion) -> Weak<CachedImage> {
    match region {
        PreparedRegion::Shared(image) => Arc::downgrade(image),
        PreparedRegion::Owned(_) => panic!("expected cache-sized preparation"),
    }
}

fn assert_pixels(region: &PreparedRegion, extent: PixelExtent, value: u8) {
    let bytes = region.matrix().data_bytes().expect("packed BGR");
    let expected = usize::try_from(extent.width()).expect("width")
        * usize::try_from(extent.height()).expect("height")
        * 3;
    assert_eq!(bytes.len(), expected);
    assert!(bytes.iter().all(|&byte| byte == value));
}

#[test]
fn eviction_keeps_active_pixels_valid_and_releases_the_last_reader() {
    let extent = PixelExtent::new(6, 4);
    let (_first, a) = mapped(extent, 17);
    let (_second, b) = mapped(extent, 53);
    let (_third, c) = mapped(extent, 149);
    let cache = PreparedImages::new();
    drop(cache.publish(&a, converted(&a)));
    let active_b = cache.publish(&b, converted(&b));
    let released_b = weak(&active_b);
    assert_pixels(&hit(&cache, &a), extent, 17);
    drop(cache.publish(&c, converted(&c)));

    assert!(matches!(cache.lookup(&b), Lookup::Vacant));
    assert_pixels(&active_b, extent, 53);
    assert_pixels(&hit(&cache, &a), extent, 17);
    assert_pixels(&hit(&cache, &c), extent, 149);
    drop(active_b);
    assert!(released_b.upgrade().is_none());
}

#[test]
fn racing_same_key_publications_do_not_displace_another_live_source() {
    let extent = PixelExtent::new(6, 4);
    let (_first, a) = mapped(extent, 17);
    let (_second, b) = mapped(extent, 53);
    let cache = PreparedImages::new();
    let b_image = cache.publish(&b, converted(&b));
    let b_owner = weak(&b_image);
    drop(b_image);
    let cold_a = converted(&a);
    let racing_a = converted(&a);
    drop(cache.publish(&a, cold_a));
    drop(cache.publish(&a, racing_a));

    assert_pixels(&hit(&cache, &a), extent, 17);
    assert_pixels(&hit(&cache, &b), extent, 53);
    assert!(b_owner.upgrade().is_some());
    drop(cache);
    assert!(b_owner.upgrade().is_none());
}

#[test]
fn byte_admission_evicts_before_the_second_image_exceeds_the_limit() {
    let extent = PixelExtent::new(2048, 3072);
    let (_first, a) = mapped(extent, 17);
    let (_second, b) = mapped(extent, 53);
    let cache = PreparedImages::new();
    let first = cache.publish(&a, converted(&a));
    let first_owner = weak(&first);
    drop(first);
    let second = cache.publish(&b, converted(&b));

    assert_pixels(&second, extent, 53);
    assert!(first_owner.upgrade().is_none());
    assert!(matches!(cache.lookup(&a), Lookup::Vacant));
    assert_pixels(&hit(&cache, &b), extent, 53);
}

#[test]
fn largest_admissible_bgr_image_is_retained_but_the_next_pixel_is_not() {
    let below = PixelExtent::new(u32::try_from(BYTE_LIMIT / 3).expect("width"), 1);
    let above = PixelExtent::new(below.width() + 1, 1);
    let (_first, a) = mapped(below, 17);
    let (_second, b) = mapped(above, 53);
    let cache = PreparedImages::new();
    let retained = cache.publish(&a, converted(&a));
    let retained_owner = weak(&retained);
    drop(retained);
    let uncached = cache.publish(&b, converted(&b));

    assert!(matches!(uncached, PreparedRegion::Owned(_)));
    assert_pixels(&uncached, above, 53);
    assert_pixels(&hit(&cache, &a), below, 17);
    assert!(matches!(cache.lookup(&b), Lookup::Vacant));
    drop(cache);
    assert!(retained_owner.upgrade().is_none());
}

#[test]
fn expired_sources_are_pruned_without_invalidating_an_active_image() {
    let extent = PixelExtent::new(6, 4);
    let (first, a) = mapped(extent, 17);
    let (_second, b) = mapped(extent, 53);
    let cache = PreparedImages::new();
    let active = cache.publish(&a, converted(&a));
    let owner = weak(&active);
    drop(first);
    drop(a);

    assert!(matches!(cache.lookup(&b), Lookup::Vacant));
    assert_pixels(&active, extent, 17);
    drop(active);
    assert!(owner.upgrade().is_none());
}

#[test]
fn contention_uses_uncached_pixels_without_waiting_for_the_cache() {
    let extent = PixelExtent::new(6, 4);
    let (_frame, pixels) = mapped(extent, 73);
    let cache = PreparedImages::new();
    std::thread::scope(|scope| {
        let held = cache.state.lock().expect("cache guard");
        let (sent, received) = mpsc::channel();
        let cache = &cache;
        let pixels = &pixels;
        let worker = scope.spawn(move || {
            let bypassed = matches!(cache.lookup(pixels), Lookup::Bypass);
            let region =
                PreparedRegion::new(image::region_to_bgr(pixels).expect("conversion"), false);
            sent.send(bypassed).expect("receiver");
            region
        });
        let completed = received.recv_timeout(Duration::from_secs(5));
        drop(held);
        let region = worker.join().expect("worker");
        assert!(completed.expect("no cache wait"));
        assert_pixels(&region, extent, 73);
    });
}

#[test]
fn a_poisoned_optional_cache_preserves_uncached_execution_and_owner_release() {
    let extent = PixelExtent::new(6, 4);
    let (_frame, pixels) = mapped(extent, 73);
    let cache = PreparedImages::new();
    let retained = cache.publish(&pixels, converted(&pixels));
    let owner = weak(&retained);
    drop(retained);
    let poisoned = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _guard = cache.state.lock().expect("cache guard");
                panic!("poison fixture");
            })
            .join()
    });
    assert!(poisoned.is_err());
    assert!(matches!(cache.lookup(&pixels), Lookup::Bypass));
    let uncached = PreparedRegion::new(image::region_to_bgr(&pixels).expect("conversion"), false);
    assert_pixels(&uncached, extent, 73);
    drop(cache);
    assert!(owner.upgrade().is_none());
    assert_pixels(&uncached, extent, 73);
}

#[test]
fn formatting_reenters_preparation_without_holding_the_cache_guard() {
    struct Writer<'a> {
        cache: &'a PreparedImages,
        pixels: &'a CpuMapping,
        correct: Option<bool>,
    }
    impl Write for Writer<'_> {
        fn write_str(&mut self, _text: &str) -> std::fmt::Result {
            let correct = match self.cache.lookup(self.pixels) {
                Lookup::Hit(image) => image
                    .matrix
                    .data_bytes()
                    .expect("pixels")
                    .iter()
                    .all(|&v| v == 73),
                _ => false,
            };
            self.correct = Some(self.correct.unwrap_or(true) && correct);
            Ok(())
        }
    }
    let extent = PixelExtent::new(6, 4);
    let (_frame, pixels) = mapped(extent, 73);
    let cache = PreparedImages::new();
    drop(cache.publish(&pixels, converted(&pixels)));
    let mut writer = Writer {
        cache: &cache,
        pixels: &pixels,
        correct: None,
    };
    write!(&mut writer, "{cache:?}").expect("format");
    assert_eq!(writer.correct, Some(true));
}

#[derive(Debug)]
struct Release(Arc<AtomicBool>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Debug)]
struct LeasedPixels {
    descriptor: FrameDescriptor,
    pixels: Arc<CpuPixels>,
}
impl FrameStorage for LeasedPixels {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }
    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        Some(Arc::clone(&self.pixels))
    }
    fn read_cpu(&self, _operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        Ok(Arc::clone(&self.pixels))
    }
}

#[test]
fn prepared_pixels_do_not_retain_the_cpu_pixel_resource_lease() {
    let extent = PixelExtent::new(6, 4);
    let basis = vision_contract::frame(extent, PixelFormat::Bgra8, 0);
    let released = Arc::new(AtomicBool::new(false));
    let storage = Arc::new(LeasedPixels {
        descriptor: basis.descriptor(),
        pixels: Arc::new(CpuPixels::with_retainer(
            vec![73; basis.descriptor().byte_len()].into_boxed_slice(),
            Arc::new(Release(Arc::clone(&released))),
        )),
    });
    let frame = Frame::from_storage(
        basis.stamp(),
        basis.captured_at(),
        *basis.transform(),
        storage,
    )
    .expect("leased frame");
    let pixels = frame
        .full_view()
        .expect("view")
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("mapping");
    let cache = PreparedImages::new();
    let image = cache.publish(&pixels, converted(&pixels));
    drop(frame);
    assert!(!released.load(Ordering::Acquire));
    assert!(pixels.bytes().iter().all(|&v| v == 73));
    drop(pixels);
    assert!(released.load(Ordering::Acquire));
    assert_pixels(&image, extent, 73);
}

#[test]
fn prepared_pixels_do_not_hold_the_native_detached_frame_budget() {
    let extent = PixelExtent::new(6, 4);
    let basis = vision_contract::frame(extent, PixelFormat::Bgra8, 0);
    let producer = ControlledProducer::new(extent, PixelFormat::Bgra8, 1, 1).expect("producer");
    let frame = Frame::from_storage(
        basis.stamp(),
        basis.captured_at(),
        *basis.transform(),
        producer.capture(73).expect("native frame"),
    )
    .expect("frame");
    let pixels = frame
        .full_view()
        .expect("view")
        .map(PixelFormat::Bgra8, &OperationContext::new())
        .expect("readback");
    let cache = PreparedImages::new();
    let image = cache.publish(&pixels, converted(&pixels));
    assert_eq!(
        producer
            .capture(19)
            .expect_err("native slot occupied")
            .status(),
        Status::LimitExceeded
    );
    drop(pixels);
    drop(frame);
    let next = producer
        .capture(19)
        .expect("native slot released while BGR remains live");
    let next_pixels = next
        .read_cpu(&OperationContext::new())
        .expect("new readback");
    assert!(next_pixels.bytes().iter().all(|&v| v == 19));
    assert_pixels(&image, extent, 73);
}
