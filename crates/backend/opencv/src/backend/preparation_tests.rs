use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use mado_pilot_capture::{
    CpuMapping, CpuPixels, Frame, FrameDescriptor, FrameStorage, PixelFormat,
};
use mado_pilot_core::{
    CancellationToken, Clock, CoordinateSpace, MonotonicInstant, OperationContext, PixelExtent,
    Result, Status,
};
use mado_pilot_testkit::{ControlledProducer, match_fixtures, png, vision_contract};
use mado_pilot_vision::{
    BackendDescriptor, BackendRequest, Candidate, MatchBackend, MatchDefaults, MatchOptions,
    MatchRequest, MatchResult, Matcher, PreparedTemplate, RegionSelection, TemplateEncoding,
    TemplateId, TemplateSource, TemplateSourceRequest,
};
use opencv::core::MatTraitConstManual;

use super::OpenCvBackend;
use crate::prepared_images::{Lookup, PreparedRegion};

/// Source and mapped layouts: both direct inputs, then an explicit conversion.
const LAYOUTS: [(PixelFormat, PixelFormat); 3] = [
    (PixelFormat::Rgba8, PixelFormat::Rgba8),
    (PixelFormat::Bgra8, PixelFormat::Bgra8),
    (PixelFormat::Rgba8, PixelFormat::Bgra8),
];

fn fixture(
    source: PixelFormat,
    mapped: PixelFormat,
) -> (OpenCvBackend, Frame, CpuMapping, PreparedTemplate) {
    let backend = OpenCvBackend::new().expect("backend");
    let frame = match_fixtures::scene_frame(source);
    let operation = OperationContext::new();
    let pixels = frame
        .full_view()
        .expect("view")
        .map(mapped, &operation)
        .expect("mapping");
    let template = backend
        .prepare(&match_fixtures::planted_template("patch"), &operation)
        .expect("template");
    (backend, frame, pixels, template)
}

fn options() -> MatchOptions {
    MatchOptions::from_defaults(MatchDefaults::new(0.9, 8).expect("options"))
}

fn request<'a>(pixels: &'a CpuMapping, template: &'a PreparedTemplate) -> BackendRequest<'a> {
    BackendRequest {
        template,
        pixels,
        region: pixels.region(),
        options: options(),
    }
}

fn assert_planted(found: &[Candidate]) {
    let mut origins: Vec<_> = found.iter().map(|candidate| candidate.origin()).collect();
    origins.sort_unstable();
    assert_eq!(origins, [(20, 12), (60, 40)]);
    assert!(
        found
            .iter()
            .all(|candidate| (candidate.score() - 1.0).abs() < 1e-5)
    );
}

fn assert_planted_result(result: &MatchResult) {
    let mut origins: Vec<_> = result
        .matches()
        .iter()
        .map(|found| (found.bounds().left(), found.bounds().top()))
        .collect();
    origins.sort_unstable();
    assert_eq!(origins, [(20, 12), (60, 40)]);
    assert!(
        result
            .matches()
            .iter()
            .all(|found| (found.score() - 1.0).abs() < 1e-5)
    );
}

#[test]
fn clones_share_preparation_until_the_last_backend_owner_ends() {
    for (source, mapped) in LAYOUTS {
        let (backend, _frame, pixels, template) = fixture(source, mapped);
        assert_planted(
            &backend
                .find(&request(&pixels, &template), &OperationContext::new())
                .expect("search"),
        );
        let owner = match backend.prepared.lookup(&pixels) {
            Lookup::Hit(image) => Arc::downgrade(&image),
            _ => panic!("successful preparation retained"),
        };
        let clone = backend.clone();
        let independent = OpenCvBackend::new().expect("independent backend");
        assert!(matches!(
            independent.prepared.lookup(&pixels),
            Lookup::Vacant
        ));
        drop(backend);
        assert_planted(
            &clone
                .find(&request(&pixels, &template), &OperationContext::new())
                .expect("clone search"),
        );
        assert!(owner.upgrade().is_some());
        assert_planted(
            &independent
                .find(&request(&pixels, &template), &OperationContext::new())
                .expect("independent search"),
        );
        drop(clone);
        assert!(owner.upgrade().is_none());
        assert_planted(
            &independent
                .find(&request(&pixels, &template), &OperationContext::new())
                .expect("remaining backend"),
        );
    }
}

/// A packed `rgb` template of `extent`.
fn rgb_template(id: &str, extent: PixelExtent, rgb: &[u8]) -> TemplateSource {
    TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new(id).expect("id"),
        encoding: TemplateEncoding::Png,
        extent,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.9, 1).expect("defaults"),
        content: Arc::from(png::encode_rgb(extent.width(), extent.height(), rgb)),
    })
    .expect("source")
}

#[test]
fn small_region_diagnostics_exclude_native_matrix_contents() {
    let rgb = [[233, 11, 5], [17, 91, 61], [47, 139, 181], [197, 29, 83]];
    let bgr: Vec<u8> = rgb.iter().flat_map(|&[r, g, b]| [b, g, r]).collect();
    for format in [PixelFormat::Bgra8, PixelFormat::Rgba8] {
        let frame_pixels = rgb
            .iter()
            .flat_map(|&[r, g, b]| {
                if format == PixelFormat::Bgra8 {
                    [b, g, r, 255]
                } else {
                    [r, g, b, 255]
                }
            })
            .collect();
        let backend = OpenCvBackend::new().expect("backend");
        let extent = PixelExtent::new(2, 2);
        let frame = vision_contract::frame_with_pixels(extent, format, frame_pixels);
        let operation = OperationContext::new();
        let pixels = frame
            .full_view()
            .expect("view")
            .map(format, &operation)
            .expect("mapping");
        let source = rgb_template("privacy", extent, rgb.as_flattened());
        let template = backend.prepare(&source, &operation).expect("template");
        let found = backend
            .find(&request(&pixels, &template), &operation)
            .expect("search");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].origin(), (0, 0));
        let region = match backend.prepared.lookup(&pixels) {
            Lookup::Hit(image) => PreparedRegion::Shared(image),
            _ => panic!("cached small region"),
        };
        assert_eq!(region.matrix().data_bytes().expect("BGR"), bgr, "{format}");
        let native_debug = format!("{:?}", region.matrix());
        let diagnostic = format!("{backend:?}");
        assert!(!diagnostic.contains(&native_debug));
        assert!(!diagnostic.contains("5, 11, 233"));
    }
}

#[derive(Debug)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::ORIGIN
    }
}

#[derive(Debug)]
struct ExpireAfterAdmission(AtomicBool);

impl Clock for ExpireAfterAdmission {
    fn now(&self) -> MonotonicInstant {
        if self.0.swap(true, Ordering::AcqRel) {
            MonotonicInstant::from_origin(Duration::from_secs(1))
        } else {
            MonotonicInstant::ORIGIN
        }
    }
}

#[test]
fn preparation_requires_live_authority_before_decode_and_publication() {
    let backend = OpenCvBackend::new().expect("backend");
    let source = match_fixtures::planted_template("authority");
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);
    let expired = OperationContext::new()
        .with_clock(Arc::new(FixedClock))
        .with_deadline(MonotonicInstant::ORIGIN);
    let expires_after_decode = OperationContext::new()
        .with_clock(Arc::new(ExpireAfterAdmission(AtomicBool::new(false))))
        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
    for (operation, expected) in [
        (&cancelled, Status::Cancelled),
        (&expired, Status::DeadlineExceeded),
        (&expires_after_decode, Status::DeadlineExceeded),
    ] {
        assert_eq!(
            backend
                .prepare(&source, operation)
                .expect_err("interrupted preparation publishes nothing")
                .status(),
            expected
        );
    }
    backend
        .prepare(&source, &OperationContext::new())
        .expect("independent live preparation");
}

#[test]
fn cancelled_and_expired_hits_and_misses_cannot_return_matches() {
    for (source, mapped) in LAYOUTS {
        for warm in [false, true] {
            let (backend, _frame, pixels, template) = fixture(source, mapped);
            let request = request(&pixels, &template);
            if warm {
                assert_planted(
                    &backend
                        .find(&request, &OperationContext::new())
                        .expect("warm search"),
                );
            }
            let token = CancellationToken::new();
            token.cancel();
            let cancelled = OperationContext::new().with_cancellation(token);
            let expired = OperationContext::new()
                .with_clock(Arc::new(FixedClock))
                .with_deadline(MonotonicInstant::ORIGIN);
            for (operation, expected) in [
                (&cancelled, Status::Cancelled),
                (&expired, Status::DeadlineExceeded),
            ] {
                assert_eq!(
                    backend
                        .find(&request, operation)
                        .expect_err("authority required")
                        .status(),
                    expected
                );
            }
            assert_planted(
                &backend
                    .find(&request, &OperationContext::new())
                    .expect("independent authority"),
            );
        }
    }
}

#[derive(Debug)]
struct StopWhenPrepared {
    backend: OpenCvBackend,
    pixels: CpuMapping,
    cancellation: CancellationToken,
    expire: bool,
    fired: AtomicBool,
    reached: mpsc::SyncSender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl Clock for StopWhenPrepared {
    fn now(&self) -> MonotonicInstant {
        if matches!(self.backend.prepared.lookup(&self.pixels), Lookup::Hit(_))
            && !self.fired.swap(true, Ordering::AcqRel)
        {
            if !self.expire {
                self.cancellation.cancel();
            }
            self.reached.send(()).expect("observer");
            self.resume
                .lock()
                .expect("gate")
                .recv_timeout(Duration::from_secs(5))
                .expect("resume");
        }
        if self.expire && self.fired.load(Ordering::Acquire) {
            MonotonicInstant::from_origin(Duration::from_secs(1))
        } else {
            MonotonicInstant::ORIGIN
        }
    }
}

#[test]
fn a_stopped_request_does_not_revoke_preparation_or_a_concurrent_callers_authority() {
    for (source, mapped) in LAYOUTS {
        for expire in [false, true] {
            let (backend, _frame, pixels, template) = fixture(source, mapped);
            let request = request(&pixels, &template);
            let token = CancellationToken::new();
            let (reached, observation) = mpsc::sync_channel(1);
            let (resume, gate) = mpsc::sync_channel(1);
            let operation = OperationContext::new()
                .with_cancellation(token.clone())
                .with_clock(Arc::new(StopWhenPrepared {
                    backend: backend.clone(),
                    pixels: pixels.clone(),
                    cancellation: token,
                    expire,
                    fired: AtomicBool::new(false),
                    reached,
                    resume: Mutex::new(gate),
                }))
                .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
            std::thread::scope(|scope| {
                let stopped = scope.spawn(|| backend.find(&request, &operation));
                let prepared = observation.recv_timeout(Duration::from_secs(5));
                let independent = prepared
                    .as_ref()
                    .ok()
                    .map(|()| backend.find(&request, &OperationContext::new()));
                let resumed = resume.send(());
                let terminal = stopped.join().expect("worker");
                prepared.expect("request reached retained preparation");
                resumed.expect("resume interrupted request");
                assert_planted(
                    &independent
                        .expect("concurrent search")
                        .expect("independent authority"),
                );
                assert_eq!(
                    terminal.expect_err("no late match").status(),
                    if expire {
                        Status::DeadlineExceeded
                    } else {
                        Status::Cancelled
                    }
                );
            });
        }
    }
}

/// How [`StopAfterBackend`] ends its armed caller's authority.
#[derive(Debug)]
enum Stop {
    Cancel(CancellationToken),
    Expire(Arc<AtomicBool>),
}

/// The real backend, ending one armed caller's authority once it has returned.
///
/// Conversion, publication and correlation have all completed by then, so only
/// the matcher's post-backend and commit checks stand between that work and a
/// result.
#[derive(Debug)]
struct StopAfterBackend {
    inner: OpenCvBackend,
    armed: Mutex<Option<Stop>>,
}

impl MatchBackend for StopAfterBackend {
    fn descriptor(&self) -> BackendDescriptor {
        self.inner.descriptor()
    }

    fn prepare(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate> {
        self.inner.prepare(source, operation)
    }

    fn find(
        &self,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<Candidate>> {
        let found = self.inner.find(request, operation)?;
        match self.armed.lock().expect("arming").take() {
            Some(Stop::Cancel(token)) => token.cancel(),
            Some(Stop::Expire(expired)) => expired.store(true, Ordering::Release),
            None => {}
        }
        Ok(found)
    }
}

/// A clock that passes a one-second deadline once its flag is set.
#[derive(Debug)]
struct Switched(Arc<AtomicBool>);
impl Clock for Switched {
    fn now(&self) -> MonotonicInstant {
        if self.0.load(Ordering::Acquire) {
            MonotonicInstant::from_origin(Duration::from_secs(1))
        } else {
            MonotonicInstant::ORIGIN
        }
    }
}

#[test]
fn authority_ending_after_the_backend_returns_commits_nothing_on_cold_or_reused_input() {
    for source in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        for expire in [false, true] {
            let backend = Arc::new(StopAfterBackend {
                inner: OpenCvBackend::new().expect("backend"),
                armed: Mutex::new(None),
            });
            let matcher = Matcher::new(Arc::clone(&backend) as Arc<dyn MatchBackend>);
            let frame = match_fixtures::scene_frame(source);
            let template = matcher
                .prepare(
                    &match_fixtures::planted_template("patch"),
                    &OperationContext::new(),
                )
                .expect("template");
            let search =
                || MatchRequest::new(&frame, RegionSelection::FullFrame, &template, options());
            // The first stopped request converts cold input; the second reuses
            // the preparation the first was lawfully admitted to publish.
            for _ in 0..2 {
                let token = CancellationToken::new();
                let expired = Arc::new(AtomicBool::new(false));
                let operation = OperationContext::new()
                    .with_cancellation(token.clone())
                    .with_clock(Arc::new(Switched(Arc::clone(&expired))))
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
                *backend.armed.lock().expect("arming") = Some(if expire {
                    Stop::Expire(expired)
                } else {
                    Stop::Cancel(token)
                });
                assert_eq!(
                    matcher
                        .find(search(), &operation)
                        .expect_err("no result after authority ended")
                        .status(),
                    if expire {
                        Status::DeadlineExceeded
                    } else {
                        Status::Cancelled
                    }
                );
            }
            assert_planted_result(
                &matcher
                    .find(search(), &OperationContext::new())
                    .expect("independent authority"),
            );
        }
    }
}

#[derive(Debug)]
struct Release(Arc<AtomicBool>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// CPU pixels that report when their last owner ends.
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

/// The scene in `format` with `padding` bytes per row, in leased storage.
fn leased_scene(format: PixelFormat, padding: usize) -> (Frame, Arc<AtomicBool>) {
    let basis = match_fixtures::scene_frame_with_padded_rows(format, padding);
    let bytes = basis
        .map(format, &OperationContext::new())
        .expect("basis bytes")
        .bytes()
        .to_vec();
    let released = Arc::new(AtomicBool::new(false));
    let storage = Arc::new(LeasedPixels {
        descriptor: basis.descriptor(),
        pixels: Arc::new(CpuPixels::with_retainer(
            bytes.into_boxed_slice(),
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
    (frame, released)
}

#[test]
fn direct_preparation_does_not_extend_source_or_mapping_owners() {
    let backend = OpenCvBackend::new().expect("backend");
    let operation = OperationContext::new();
    let template = backend
        .prepare(&match_fixtures::planted_template("patch"), &operation)
        .expect("template");
    let mut retained = Vec::new();
    // Three sources exceed the two retained preparations, so later frames
    // turn earlier ones over.
    for (format, padding) in [
        (PixelFormat::Rgba8, 0),
        (PixelFormat::Bgra8, 64),
        (PixelFormat::Rgba8, 64),
    ] {
        let (frame, released) = leased_scene(format, padding);
        let pixels = frame.map(format, &operation).expect("direct mapping");
        assert!(pixels.is_shared(), "an accepted full frame is not copied");
        for _ in 0..2 {
            let found = backend
                .find(&request(&pixels, &template), &operation)
                .expect("cold search, then persistent reuse");
            assert_planted(&found);
            retained.push(found);
        }
        drop(frame);
        assert!(
            !released.load(Ordering::Acquire),
            "the mapping still owns the source pixels"
        );
        drop(pixels);
        assert!(
            released.load(Ordering::Acquire),
            "retained BGR does not extend the source lease ({format})"
        );
    }
    for found in &retained {
        assert_planted(found);
    }
    let frame = match_fixtures::scene_frame(PixelFormat::Bgra8);
    let pixels = frame.map(PixelFormat::Bgra8, &operation).expect("mapping");
    assert_planted(
        &backend
            .find(&request(&pixels, &template), &operation)
            .expect("the surviving backend searches a new source"),
    );
}

/// Adds spatial variation while preserving the controlled native storage lease.
#[derive(Debug)]
struct PatternedReadback(Arc<dyn FrameStorage>);

impl FrameStorage for PatternedReadback {
    fn descriptor(&self) -> FrameDescriptor {
        self.0.descriptor()
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        None
    }

    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        let mut bytes = self.0.read_cpu(operation)?.bytes().to_vec();
        bytes[..3].fill(74);
        Ok(Arc::new(CpuPixels::new(bytes.into_boxed_slice())))
    }
}

#[test]
fn native_readback_reservations_end_with_their_owners_while_the_backend_survives() {
    let backend = OpenCvBackend::new().expect("backend");
    let operation = OperationContext::new();
    // Matching the full frame leaves one exact offset and retains readback ownership.
    let extent = PixelExtent::new(8, 8);
    let mut rgb = [73; 3].repeat(8 * 8);
    rgb[..3].fill(74);
    let template = backend
        .prepare(&rgb_template("readback", extent, &rgb), &operation)
        .expect("nonconstant template");
    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        let basis = vision_contract::frame(extent, format, 0);
        let producer = ControlledProducer::new(extent, format, 1, 1).expect("producer");
        let frame = Frame::from_storage(
            basis.stamp(),
            basis.captured_at(),
            *basis.transform(),
            Arc::new(PatternedReadback(
                producer.capture(73).expect("native frame"),
            )),
        )
        .expect("frame");
        let pixels = frame.map(format, &operation).expect("readback");
        assert_eq!(pixels.descriptor().format(), format);
        for _ in 0..2 {
            let found = backend
                .find(&request(&pixels, &template), &operation)
                .expect("cold search, then persistent reuse");
            assert_eq!(
                found
                    .iter()
                    .map(|candidate| candidate.origin())
                    .collect::<Vec<_>>(),
                [(0, 0)]
            );
        }
        assert_eq!(
            producer
                .capture(19)
                .expect_err("detached slot occupied")
                .status(),
            Status::LimitExceeded
        );
        drop(pixels);
        drop(frame);
        let next = producer
            .capture(19)
            .expect("the slot returns while backend preparation remains");
        drop(next);
        drop(producer);
    }
    let template = backend
        .prepare(&match_fixtures::planted_template("patch"), &operation)
        .expect("template");
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let pixels = frame.map(PixelFormat::Rgba8, &operation).expect("mapping");
    assert_planted(
        &backend
            .find(&request(&pixels, &template), &operation)
            .expect("the backend outlives the closed producer"),
    );
}
