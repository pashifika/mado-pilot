//! The replay adapter against the shared capture contract, and against the
//! tracked fixture's oracle.

use std::path::PathBuf;
use std::sync::Arc;

use mado_pilot_adapter_replay::{ReplayFrame, ReplayProvider, ReplaySource, ReplayTarget};
use mado_pilot_capture::{
    CaptureProvider, Continuity, FrameDescriptor, FrameRequest, OpenRequest, PixelFormat,
};
use mado_pilot_core::{
    ClipPolicy, CoordinateSpace, FrameOrder, IdentityIssuer, MonotonicInstant, OperationContext,
    PixelExtent, Point, Rect, Status,
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
fn target_conversions_are_available_only_where_the_source_declares_a_placement() {
    let provider = fixture_provider();
    let operation = OperationContext::new();
    let targets = provider.discover(&operation).expect("discovered");

    assert!(
        !targets[0]
            .coordinates()
            .supports(CoordinateSpace::TargetLogical)
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

    let converted = placed
        .transform()
        .convert_point(logical, CoordinateSpace::CapturePixels)
        .expect("the placement makes this authoritative");
    assert_eq!((converted.x(), converted.y()), (2.0, 2.0));
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
