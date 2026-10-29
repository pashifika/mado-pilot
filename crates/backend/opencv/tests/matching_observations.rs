//! Real-search observations in an isolated test process.

#![cfg(feature = "benchmark-instrumentation")]

use std::sync::Arc;
use std::time::Duration;

use mado_pilot_backend_opencv::OpenCvBackend;
use mado_pilot_backend_opencv::benchmark_instrumentation::{snapshot, work_snapshot};
use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{CancellationToken, Clock, MonotonicInstant, OperationContext, Status};
use mado_pilot_testkit::match_fixtures;
use mado_pilot_vision::{BackendRequest, MatchBackend, MatchDefaults, MatchOptions};

#[derive(Debug)]
struct ExpireAfterConversion(u64);

impl Clock for ExpireAfterConversion {
    fn now(&self) -> MonotonicInstant {
        if work_snapshot().bgr_conversions > self.0 {
            MonotonicInstant::from_origin(Duration::from_secs(1))
        } else {
            MonotonicInstant::ORIGIN
        }
    }
}

#[test]
fn real_searches_report_logical_work_separately_from_reused_preparation() {
    let backend = OpenCvBackend::new().expect("backend");
    let frame = match_fixtures::scene_frame(PixelFormat::Rgba8);
    let context = OperationContext::new();
    let pixels = frame.map(PixelFormat::Bgra8, &context).expect("mapping");
    let present = backend
        .prepare(&match_fixtures::planted_template("present"), &context)
        .expect("template");
    let absent = backend
        .prepare(&match_fixtures::absent_template("absent"), &context)
        .expect("template");
    let options = MatchOptions::from_defaults(MatchDefaults::new(0.9, 8).expect("options"));
    let request = |template| BackendRequest {
        template,
        pixels: &pixels,
        region: pixels.region(),
        options,
    };
    let before = snapshot();
    let work_before = work_snapshot();

    let found = backend
        .find(&request(&present), &context)
        .expect("present search");
    let mut origins: Vec<_> = found.iter().map(|candidate| candidate.origin()).collect();
    origins.sort_unstable();
    assert_eq!(origins, [(20, 12), (60, 40)]);
    assert!(
        backend
            .find(&request(&absent), &context)
            .expect("absent search")
            .is_empty()
    );
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        backend
            .find(
                &request(&present),
                &OperationContext::new().with_cancellation(token)
            )
            .expect_err("cancelled search")
            .status(),
        Status::Cancelled
    );

    let observed = snapshot().checked_delta(before).expect("quiescent process");
    assert_eq!(observed.find_calls, 3);
    assert_eq!(observed.find_completions, 2);
    assert_eq!(observed.find_failures, 1);
    assert_eq!(
        observed.mapped_bytes,
        3 * u64::try_from(pixels.bytes().len()).expect("mapped bytes")
    );
    let work = work_snapshot();
    assert_eq!(work.bgr_conversions - work_before.bgr_conversions, 1);
    assert_eq!(
        work.bgr_converted_bytes - work_before.bgr_converted_bytes,
        96 * 64 * 3
    );
    assert_eq!(work.score_copied_bytes - work_before.score_copied_bytes, 0);
    assert_eq!(
        work.bgr_retained_bytes - work_before.bgr_retained_bytes,
        96 * 64 * 3
    );

    let independent = OpenCvBackend::new().expect("independent backend");
    let other = independent
        .find(&request(&present), &context)
        .expect("independent search");
    let mut other_origins: Vec<_> = other.iter().map(|candidate| candidate.origin()).collect();
    other_origins.sort_unstable();
    assert_eq!(other_origins, origins);
    assert_eq!(
        work_snapshot().bgr_retained_bytes - work_before.bgr_retained_bytes,
        2 * 96 * 64 * 3
    );
    drop(backend);
    assert_eq!(
        work_snapshot().bgr_retained_bytes - work_before.bgr_retained_bytes,
        96 * 64 * 3
    );
    drop(independent);
    let released = work_snapshot();
    assert_eq!(released.bgr_retained_bytes, work_before.bgr_retained_bytes);
    assert_eq!(released.bgr_evicted_bytes, work_before.bgr_evicted_bytes);

    let backend = OpenCvBackend::new().expect("fresh admission");
    let before_rejection = work_snapshot();
    let expired = OperationContext::new()
        .with_clock(Arc::new(ExpireAfterConversion(
            before_rejection.bgr_conversions,
        )))
        .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
    assert_eq!(
        backend
            .find(&request(&present), &expired)
            .expect_err("preparation expired")
            .status(),
        Status::DeadlineExceeded
    );
    let rejected = work_snapshot();
    assert_eq!(
        rejected.bgr_conversions - before_rejection.bgr_conversions,
        1
    );
    assert_eq!(
        rejected.bgr_retained_bytes,
        before_rejection.bgr_retained_bytes
    );
    let found = backend
        .find(&request(&present), &context)
        .expect("fresh authority");
    let mut retried_origins: Vec<_> = found.iter().map(|candidate| candidate.origin()).collect();
    retried_origins.sort_unstable();
    assert_eq!(retried_origins, origins);
    assert_eq!(
        work_snapshot().bgr_conversions - before_rejection.bgr_conversions,
        2
    );
    drop(backend);
    assert_eq!(
        work_snapshot().bgr_retained_bytes,
        work_before.bgr_retained_bytes
    );
    assert_eq!(
        work_snapshot().bgr_evicted_bytes,
        work_before.bgr_evicted_bytes
    );
}
