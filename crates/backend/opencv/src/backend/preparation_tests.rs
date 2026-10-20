use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use mado_pilot_capture::{CpuMapping, Frame, PixelFormat};
use mado_pilot_core::{
    CancellationToken, Clock, CoordinateSpace, MonotonicInstant, OperationContext, PixelExtent,
    Status,
};
use mado_pilot_testkit::{match_fixtures, png, vision_contract};
use mado_pilot_vision::{
    BackendRequest, Candidate, MatchBackend, MatchDefaults, MatchOptions, PreparedTemplate,
    TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
};
use opencv::core::MatTraitConstManual;

use super::OpenCvBackend;
use crate::prepared_images::{Lookup, PreparedRegion};

fn fixture() -> (OpenCvBackend, Frame, CpuMapping, PreparedTemplate) {
    let backend = OpenCvBackend::new().expect("backend");
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let operation = OperationContext::new();
    let pixels = frame
        .full_view()
        .expect("view")
        .map(PixelFormat::Bgra8, &operation)
        .expect("mapping");
    let template = backend
        .prepare(&match_fixtures::planted_template("patch"), &operation)
        .expect("template");
    (backend, frame, pixels, template)
}

fn request<'a>(pixels: &'a CpuMapping, template: &'a PreparedTemplate) -> BackendRequest<'a> {
    BackendRequest {
        template,
        pixels,
        region: pixels.region(),
        options: MatchOptions::from_defaults(MatchDefaults::new(0.9, 8).expect("options")),
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

#[test]
fn clones_share_preparation_until_the_last_backend_owner_ends() {
    let (backend, _frame, pixels, template) = fixture();
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

#[test]
fn small_region_diagnostics_exclude_native_matrix_contents() {
    let backend = OpenCvBackend::new().expect("backend");
    let extent = PixelExtent::new(2, 2);
    let frame =
        vision_contract::frame_with_pixels(extent, PixelFormat::Bgra8, [5, 11, 233, 255].repeat(4));
    let operation = OperationContext::new();
    let pixels = frame
        .full_view()
        .expect("view")
        .map(PixelFormat::Bgra8, &operation)
        .expect("mapping");
    let source = TemplateSource::new(TemplateSourceRequest {
        id: TemplateId::new("privacy").expect("id"),
        encoding: TemplateEncoding::Png,
        extent,
        space: CoordinateSpace::CapturePixels,
        defaults: MatchDefaults::new(0.9, 1).expect("defaults"),
        content: Arc::from(png::solid_rgb(2, 2, [233, 11, 5])),
    })
    .expect("source");
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
    assert_eq!(
        region.matrix().data_bytes().expect("BGR"),
        [5, 11, 233].repeat(4)
    );
    let native_debug = format!("{:?}", region.matrix());
    let diagnostic = format!("{backend:?}");
    assert!(!diagnostic.contains(&native_debug));
    assert!(!diagnostic.contains("5, 11, 233"));
}

#[derive(Debug)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant::ORIGIN
    }
}

#[test]
fn cancelled_and_expired_hits_and_misses_cannot_return_matches() {
    for warm in [false, true] {
        let (backend, _frame, pixels, template) = fixture();
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
    for expire in [false, true] {
        let (backend, _frame, pixels, template) = fixture();
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
