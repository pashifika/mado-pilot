//! The replay adapter against the shared capture contract, and against the
//! tracked fixture's oracle.

use std::path::PathBuf;
use std::sync::Arc;

use mado_pilot_adapter_replay::{ReplayFrame, ReplayProvider, ReplaySource, ReplayTarget};
use mado_pilot_capture::{
    CaptureProvider, Continuity, FrameDescriptor, FrameRequest, OpenRequest, PixelFormat,
};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, FrameOrder, IdentityIssuer, MonotonicInstant,
    OperationContext, PixelExtent, Point, Rect, Status,
};
use mado_pilot_testkit::capture_contract;

fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/capture/replay-basic")
}

fn memory_provider() -> ReplayProvider {
    let descriptor =
        FrameDescriptor::packed(PixelExtent::new(8, 6), PixelFormat::Rgba8).expect("valid");
    let frames: Vec<ReplayFrame> = (0..3)
        .map(|index| {
            ReplayFrame::new(
                descriptor,
                MonotonicInstant::ORIGIN,
                Continuity::Continuous,
                None,
                vec![index; descriptor.byte_len()].into_boxed_slice(),
            )
            .expect("valid")
        })
        .collect();
    let source =
        ReplaySource::from_targets(vec![ReplayTarget::new("memory", frames).expect("valid")])
            .expect("valid");
    ReplayProvider::new(Arc::new(IdentityIssuer::new()), source).expect("built")
}

fn fixture_provider() -> ReplayProvider {
    let source = ReplaySource::from_directory(fixture_directory()).expect("fixture loads");
    ReplayProvider::new(Arc::new(IdentityIssuer::new()), source).expect("built")
}

#[test]
fn a_memory_source_satisfies_the_capture_contract() {
    capture_contract::run(&memory_provider());
}

#[test]
fn a_directory_source_satisfies_the_capture_contract() {
    capture_contract::run(&fixture_provider());
}

#[test]
fn the_fixture_declares_the_targets_and_extents_its_oracle_expects() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");

    let names: Vec<&str> = targets.iter().map(|target| target.name()).collect();
    assert_eq!(
        names,
        vec!["panel", "placed"],
        "declaration order is preserved"
    );
    assert_eq!(targets[0].extent(), PixelExtent::new(8, 6));
    assert_eq!(targets[1].extent(), PixelExtent::new(4, 4));
    assert_eq!(targets[0].format(), PixelFormat::Rgba8);
}

#[test]
fn a_repeated_frame_is_still_a_new_frame() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");
    let session = provider
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");

    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    let second = session
        .frame(&FrameRequest::newer_than(first.stamp()), &operation)
        .expect("second frame");

    let a = first.map(PixelFormat::Rgba8, &operation).expect("mapped");
    let b = second.map(PixelFormat::Rgba8, &operation).expect("mapped");

    assert_eq!(
        a.bytes(),
        b.bytes(),
        "the fixture's first two frames are byte-identical on purpose"
    );
    assert_ne!(
        first.stamp(),
        second.stamp(),
        "identical pixels are still a distinct observation"
    );
    assert_eq!(first.stamp().epoch(), second.stamp().epoch());
    assert_eq!(second.stamp().sequence().value(), 1);
    assert_eq!(first.stamp().order(&second.stamp()), Ok(FrameOrder::Before));
}

#[test]
fn an_already_cancelled_newer_than_request_does_not_advance_replay() {
    let provider = memory_provider();
    let operation = OperationContext::new();
    let target = provider.discover(&operation).expect("discovered").remove(0);
    let session = provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("opened");
    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = OperationContext::new().with_cancellation(cancellation);

    let error = session
        .frame(&FrameRequest::newer_than(first.stamp()), &cancelled)
        .expect_err("cancelled before admission");

    assert_eq!(error.status(), Status::Cancelled);
    assert_eq!(
        session
            .frame(&FrameRequest::latest(), &operation)
            .expect("current frame")
            .stamp(),
        first.stamp(),
        "a rejected request must not publish the next replay frame"
    );
    assert_eq!(
        session
            .frame(&FrameRequest::newer_than(first.stamp()), &operation)
            .expect("next frame remains available")
            .stamp()
            .sequence()
            .value(),
        1,
        "a rejected request must not consume the next replay frame"
    );
}

#[test]
fn an_already_expired_newer_than_request_does_not_advance_replay() {
    let provider = memory_provider();
    let operation = OperationContext::new();
    let target = provider.discover(&operation).expect("discovered").remove(0);
    let session = provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("opened");
    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    let expired = OperationContext::new().with_deadline(MonotonicInstant::ORIGIN);

    let error = session
        .frame(&FrameRequest::newer_than(first.stamp()), &expired)
        .expect_err("expired before admission");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    assert_eq!(
        session
            .frame(&FrameRequest::latest(), &operation)
            .expect("current frame")
            .stamp(),
        first.stamp(),
        "a rejected request must not publish the next replay frame"
    );
    assert_eq!(
        session
            .frame(&FrameRequest::newer_than(first.stamp()), &operation)
            .expect("next frame remains available")
            .stamp()
            .sequence()
            .value(),
        1,
        "a rejected request must not consume the next replay frame"
    );
}

#[test]
fn a_satisfiable_newer_than_request_is_rejected_after_closing_begins() {
    let provider = memory_provider();
    let operation = OperationContext::new();
    let target = provider.discover(&operation).expect("discovered").remove(0);
    let session = provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("opened");
    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    session
        .frame(&FrameRequest::newer_than(first.stamp()), &operation)
        .expect("second frame");
    let expired = OperationContext::new().with_deadline(MonotonicInstant::ORIGIN);
    assert_eq!(
        session.close(&expired).expect_err("close expires").status(),
        Status::DeadlineExceeded
    );

    let error = session
        .frame(&FrameRequest::newer_than(first.stamp()), &operation)
        .expect_err("closing rejects cached fast path");

    assert_eq!(error.status(), Status::Closed);
}

#[test]
fn a_foreign_stream_request_does_not_advance_replay() {
    let provider = memory_provider();
    let operation = OperationContext::new();
    let target = provider.discover(&operation).expect("discovered").remove(0);
    let session = provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("opened");
    let other = provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("opened another stream");
    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    let foreign = other
        .frame(&FrameRequest::latest(), &operation)
        .expect("foreign frame");

    let error = session
        .frame(&FrameRequest::newer_than(foreign.stamp()), &operation)
        .expect_err("foreign stream");

    assert_eq!(error.status(), Status::InvalidArgument);
    assert_eq!(
        session
            .frame(&FrameRequest::latest(), &operation)
            .expect("current frame")
            .stamp(),
        first.stamp()
    );
}

#[test]
fn an_extent_change_starts_a_later_epoch() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");
    let session = provider
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");

    let mut current = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    for _ in 0..2 {
        current = session
            .frame(&FrameRequest::newer_than(current.stamp()), &operation)
            .expect("next frame");
    }

    assert_eq!(current.descriptor().extent(), PixelExtent::new(12, 6));
    assert_eq!(current.stamp().epoch().value(), 1);
    assert_eq!(current.stamp().sequence().value(), 0);
    assert!(
        current.stamp().geometry().value() > 0,
        "an extent change advances the geometry revision"
    );
}

#[test]
fn an_older_frame_keeps_its_geometry_and_pixels_after_an_extent_change() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let target = provider.discover(&operation).expect("discovered").remove(0);
    let session = provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("opened");
    let original = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    let original_stamp = original.stamp();
    let original_transform = *original.transform();

    let repeated = session
        .frame(&FrameRequest::newer_than(original_stamp), &operation)
        .expect("repeated frame");
    let changed = session
        .frame(&FrameRequest::newer_than(repeated.stamp()), &operation)
        .expect("extent change");

    let mapping = original
        .map(PixelFormat::Rgba8, &operation)
        .expect("old frame still maps");
    assert_eq!(mapping.stamp(), original_stamp);
    assert_eq!(*mapping.transform(), original_transform);
    assert_eq!(mapping.descriptor().extent(), PixelExtent::new(8, 6));
    assert_ne!(mapping.stamp().geometry(), changed.stamp().geometry());
    assert_ne!(mapping.descriptor().extent(), changed.descriptor().extent());
}

#[test]
fn an_exhausted_sequence_is_reported_rather_than_waited_out() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");
    let session = provider
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened");

    let mut current = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("first frame");
    for _ in 0..2 {
        current = session
            .frame(&FrameRequest::newer_than(current.stamp()), &operation)
            .expect("next frame");
    }

    let error = session
        .frame(&FrameRequest::newer_than(current.stamp()), &operation)
        .expect_err("the sequence is finite");

    assert_eq!(
        error.status(),
        Status::Closed,
        "waiting for a frame that can never arrive is not a useful answer"
    );
}

#[test]
fn target_normalized_needs_only_content_extent_while_logical_spaces_need_placement() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");

    assert!(
        targets[0]
            .coordinates()
            .supports(CoordinateSpace::TargetNormalized),
        "the source declares the target content extent"
    );
    assert!(
        !targets[0]
            .coordinates()
            .supports(CoordinateSpace::TargetLogical)
    );
    assert!(
        !targets[0]
            .coordinates()
            .supports(CoordinateSpace::DesktopLogical)
    );
    assert!(
        targets[1]
            .coordinates()
            .supports(CoordinateSpace::TargetLogical)
    );

    let unplaced = provider
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened")
        .frame(&FrameRequest::latest(), &operation)
        .expect("frame");
    let placed = provider
        .open(targets[1].id(), &OpenRequest::new(), &operation)
        .expect("opened")
        .frame(&FrameRequest::latest(), &operation)
        .expect("frame");

    let normalized = Point::new(CoordinateSpace::TargetNormalized, 0.5, 0.5).expect("valid");
    let normalized_pixels = unplaced
        .transform()
        .convert_point(normalized, CoordinateSpace::CapturePixels)
        .expect("target content extent is authoritative");
    assert_eq!((normalized_pixels.x(), normalized_pixels.y()), (4.0, 3.0));

    let logical = Point::new(CoordinateSpace::TargetLogical, 1.0, 1.0).expect("valid");
    assert_eq!(
        unplaced
            .transform()
            .convert_point(logical, CoordinateSpace::CapturePixels)
            .err()
            .map(|fault| fault.status()),
        Some(Status::Unsupported),
        "capture pixels are never assumed to be logical units"
    );

    let desktop = Point::new(CoordinateSpace::DesktopLogical, 1.0, 1.0).expect("valid");
    assert_eq!(
        unplaced
            .transform()
            .convert_point(desktop, CoordinateSpace::CapturePixels)
            .err()
            .map(|fault| fault.status()),
        Some(Status::Unsupported),
        "desktop placement is never inferred"
    );

    let converted = placed
        .transform()
        .convert_point(logical, CoordinateSpace::CapturePixels)
        .expect("the placement makes this authoritative");
    assert_eq!((converted.x(), converted.y()), (2.0, 2.0));
}

#[test]
fn frame_view_preserves_unsupported_status_and_rejects_actual_outside_geometry() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let target = provider.discover(&operation).expect("discovered").remove(0);
    let frame = provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("opened")
        .frame(&FrameRequest::latest(), &operation)
        .expect("frame");
    let unsupported =
        Rect::new(CoordinateSpace::TargetLogical, 0.0, 0.0, 1.0, 1.0).expect("valid geometry");
    let outside =
        Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 9.0, 6.0).expect("valid geometry");

    assert_eq!(
        frame
            .view(unsupported, ClipPolicy::Reject)
            .expect_err("conversion is unsupported")
            .status(),
        Status::Unsupported
    );
    assert_eq!(
        frame
            .view(outside, ClipPolicy::Reject)
            .expect_err("region is outside")
            .status(),
        Status::InvalidArgument
    );
}

#[test]
fn a_region_of_a_replay_frame_maps_to_exactly_that_region() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");
    let frame = provider
        .open(targets[0].id(), &OpenRequest::new(), &operation)
        .expect("opened")
        .frame(&FrameRequest::latest(), &operation)
        .expect("frame");

    let half = Rect::new(CoordinateSpace::FrameNormalized, 0.0, 0.0, 0.5, 1.0).expect("valid");
    let view = frame.view(half, ClipPolicy::Reject).expect("inside");
    let mapping = view.map(PixelFormat::Rgba8, &operation).expect("mapped");

    assert_eq!(mapping.descriptor().extent(), PixelExtent::new(4, 6));
    assert_eq!(mapping.stamp(), frame.stamp());
    assert_eq!(mapping.bytes().len(), 4 * 4 * 6);
}

#[test]
fn a_required_format_the_source_cannot_provide_fails_the_open() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");

    let error = provider
        .open(
            targets[0].id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &operation,
        )
        .expect_err("the fixture is rgba8");

    assert_eq!(error.status(), Status::Unsupported);
}

#[test]
fn the_tracked_fixture_is_the_one_the_oracle_describes() {
    // Guards against a fixture edit that silently changes what every assertion
    // above is measuring.
    let source = ReplaySource::from_directory(fixture_directory()).expect("loads");
    let panel = &source.targets()[0];

    assert_eq!(panel.frames().len(), 3);
    assert_eq!(panel.frames()[0].pixels(), panel.frames()[1].pixels());
    assert_eq!(
        panel.frames()[2].descriptor().extent(),
        PixelExtent::new(12, 6)
    );
    assert!(!panel.declares_placement());
    assert!(source.targets()[1].declares_placement());
}

#[test]
fn every_tracked_fixture_still_hashes_to_its_recorded_checksum() {
    // The tracked frames are what make this suite's identity and geometry
    // assertions mean anything: one frame repeats the previous frame's bytes
    // exactly, and one changes extent. A silent edit to either would leave the
    // assertions passing while testing something else.
    capture_contract::verify_fixture_checksums(&fixture_directory());
}
