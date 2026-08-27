//! Deterministic contracts for the closed G-005 runtime policy seam.

use std::mem::{needs_drop, size_of};

use mado_pilot_capture::{CpuMapping, Frame, FrameDescriptor, PixelFormat};
use mado_pilot_core::{
    ClipPolicy, CoordinateSpace, GeometryRevision, IdentityIssuer, MonotonicInstant,
    OperationContext, PixelExtent, Rect, StreamCursor, TransformSnapshot,
};
use mado_pilot_vision::{
    ANALYSIS_ALWAYS_POLICY_CODE, ChangeDecision, ChangeDetectionPolicy, ChangeDetector,
    DEFAULT_CHANGE_DETECTION_DESCRIPTOR, EXACT_RGBA_POLICY_CODE,
};

const EXTENT: PixelExtent = PixelExtent::new(4, 4);

fn mapped(stamp: mado_pilot_core::FrameStamp, format: PixelFormat, bytes: Vec<u8>) -> CpuMapping {
    let descriptor = FrameDescriptor::packed(EXTENT, format).expect("packed descriptor");
    Frame::new(
        stamp,
        MonotonicInstant::ORIGIN,
        descriptor,
        TransformSnapshot::frame_only(stamp.geometry(), EXTENT),
        bytes.into_boxed_slice(),
    )
    .expect("consistent frame")
    .map(format, &OperationContext::new())
    .expect("CPU mapping")
}

fn continuous_pair(
    previous_bytes: Vec<u8>,
    current_bytes: Vec<u8>,
    format: PixelFormat,
    skipped: u64,
) -> (CpuMapping, CpuMapping) {
    let issuer = IdentityIssuer::new();
    let mut cursor = StreamCursor::new(issuer.issue_stream().expect("stream identity"));
    let previous = cursor
        .publish(GeometryRevision::FIRST)
        .expect("previous stamp");
    cursor.skip(skipped).expect("bounded sequence gap");
    let current = cursor
        .publish(GeometryRevision::FIRST)
        .expect("current stamp");
    (
        mapped(previous, format, previous_bytes),
        mapped(current, format, current_bytes),
    )
}

#[test]
fn selected_default_is_exact_rgba_and_analysis_always_remains_available() {
    let descriptor = DEFAULT_CHANGE_DETECTION_DESCRIPTOR;

    assert_eq!(
        ChangeDetectionPolicy::default(),
        ChangeDetectionPolicy::ExactRgba
    );
    assert_eq!(
        ChangeDetector::default(),
        ChangeDetector::selected_default()
    );
    assert_eq!(descriptor.policy(), ChangeDetectionPolicy::ExactRgba);
    assert_eq!(descriptor.policy_id(), "exact-rgba-v1");
    assert!(descriptor.unchanged_may_skip_routine_analysis());
    assert!(!descriptor.unchanged_confirms_presence());
    assert!(!descriptor.unchanged_advances_consecutive_stability());
    assert!(!descriptor.unchanged_creates_duration_stability());
    assert!(!descriptor.unchanged_crosses_incompatible_identity_or_geometry());

    let bytes = vec![
        7;
        FrameDescriptor::packed(EXTENT, PixelFormat::Rgba8)
            .expect("descriptor")
            .byte_len()
    ];
    let (previous, current) = continuous_pair(bytes.clone(), bytes, PixelFormat::Rgba8, 0);
    assert_eq!(
        ChangeDetector::new(ChangeDetectionPolicy::AnalysisAlways).compare(&previous, &current),
        ChangeDecision::AnalysisRequired
    );
}

#[test]
fn exact_rgba_skips_identical_pixels_and_admits_any_changed_pixel() {
    let byte_len = FrameDescriptor::packed(EXTENT, PixelFormat::Rgba8)
        .expect("descriptor")
        .byte_len();
    let previous_bytes = vec![11; byte_len];
    let identical = previous_bytes.clone();
    let mut changed = previous_bytes.clone();
    changed[13] ^= 0xff;

    let (previous, current) =
        continuous_pair(previous_bytes.clone(), identical, PixelFormat::Rgba8, 0);
    assert_eq!(
        ChangeDetector::selected_default().compare(&previous, &current),
        ChangeDecision::Unchanged
    );

    let (previous, current) = continuous_pair(previous_bytes, changed, PixelFormat::Rgba8, 0);
    assert_eq!(
        ChangeDetector::selected_default().compare(&previous, &current),
        ChangeDecision::AnalysisRequired
    );
}

#[test]
fn sequence_gaps_do_not_invent_a_discontinuity() {
    let bytes = vec![
        29;
        FrameDescriptor::packed(EXTENT, PixelFormat::Rgba8)
            .expect("descriptor")
            .byte_len()
    ];
    let (previous, current) = continuous_pair(bytes.clone(), bytes, PixelFormat::Rgba8, 4);

    assert_eq!(
        ChangeDetector::selected_default().compare(&previous, &current),
        ChangeDecision::Unchanged
    );
}

#[test]
fn epoch_geometry_stream_order_region_and_format_boundaries_fail_safe() {
    let byte_len = FrameDescriptor::packed(EXTENT, PixelFormat::Rgba8)
        .expect("descriptor")
        .byte_len();
    let bytes = vec![41; byte_len];
    let detector = ChangeDetector::selected_default();
    let issuer = IdentityIssuer::new();

    let mut geometry_cursor =
        StreamCursor::new(issuer.issue_stream().expect("geometry stream identity"));
    let previous_stamp = geometry_cursor
        .publish(GeometryRevision::FIRST)
        .expect("previous geometry stamp");
    let next_geometry = GeometryRevision::FIRST.next().expect("next geometry");
    let geometry_stamp = geometry_cursor
        .publish(next_geometry)
        .expect("changed geometry stamp");
    assert_eq!(
        detector.compare(
            &mapped(previous_stamp, PixelFormat::Rgba8, bytes.clone()),
            &mapped(geometry_stamp, PixelFormat::Rgba8, bytes.clone()),
        ),
        ChangeDecision::AnalysisRequired
    );

    let mut epoch_cursor = StreamCursor::new(issuer.issue_stream().expect("epoch stream identity"));
    let previous_stamp = epoch_cursor
        .publish(GeometryRevision::FIRST)
        .expect("previous epoch stamp");
    epoch_cursor.begin_epoch().expect("next epoch");
    let epoch_stamp = epoch_cursor
        .publish(GeometryRevision::FIRST)
        .expect("new epoch stamp");
    assert_eq!(
        detector.compare(
            &mapped(previous_stamp, PixelFormat::Rgba8, bytes.clone()),
            &mapped(epoch_stamp, PixelFormat::Rgba8, bytes.clone()),
        ),
        ChangeDecision::AnalysisRequired
    );

    let mut first = StreamCursor::new(issuer.issue_stream().expect("first stream identity"));
    let mut second = StreamCursor::new(issuer.issue_stream().expect("second stream identity"));
    assert_eq!(
        detector.compare(
            &mapped(
                first
                    .publish(GeometryRevision::FIRST)
                    .expect("first stream stamp"),
                PixelFormat::Rgba8,
                bytes.clone(),
            ),
            &mapped(
                second
                    .publish(GeometryRevision::FIRST)
                    .expect("second stream stamp"),
                PixelFormat::Rgba8,
                bytes.clone(),
            ),
        ),
        ChangeDecision::AnalysisRequired
    );

    let (later, earlier) = continuous_pair(bytes.clone(), bytes.clone(), PixelFormat::Rgba8, 0);
    assert_eq!(
        detector.compare(&earlier, &later),
        ChangeDecision::AnalysisRequired
    );

    let mut region_cursor =
        StreamCursor::new(issuer.issue_stream().expect("region stream identity"));
    let previous_stamp = region_cursor
        .publish(GeometryRevision::FIRST)
        .expect("previous region stamp");
    let current_stamp = region_cursor
        .publish(GeometryRevision::FIRST)
        .expect("current region stamp");
    let descriptor =
        FrameDescriptor::packed(EXTENT, PixelFormat::Rgba8).expect("region descriptor");
    let previous_frame = Frame::new(
        previous_stamp,
        MonotonicInstant::ORIGIN,
        descriptor,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, EXTENT),
        bytes.clone().into_boxed_slice(),
    )
    .expect("previous region frame");
    let current_frame = Frame::new(
        current_stamp,
        MonotonicInstant::ORIGIN,
        descriptor,
        TransformSnapshot::frame_only(GeometryRevision::FIRST, EXTENT),
        bytes.clone().into_boxed_slice(),
    )
    .expect("current region frame");
    let previous_region = previous_frame
        .view(
            Rect::from_origin_size(CoordinateSpace::CapturePixels, 0.0, 0.0, 2.0, 2.0)
                .expect("previous region"),
            ClipPolicy::Reject,
        )
        .expect("previous view")
        .map(PixelFormat::Rgba8, &OperationContext::new())
        .expect("previous region mapping");
    let current_region = current_frame
        .view(
            Rect::from_origin_size(CoordinateSpace::CapturePixels, 1.0, 0.0, 2.0, 2.0)
                .expect("current region"),
            ClipPolicy::Reject,
        )
        .expect("current view")
        .map(PixelFormat::Rgba8, &OperationContext::new())
        .expect("current region mapping");
    assert_eq!(
        detector.compare(&previous_region, &current_region),
        ChangeDecision::AnalysisRequired
    );

    let bgra_bytes = vec![
        41;
        FrameDescriptor::packed(EXTENT, PixelFormat::Bgra8)
            .expect("descriptor")
            .byte_len()
    ];
    let (previous, current) =
        continuous_pair(bgra_bytes.clone(), bgra_bytes, PixelFormat::Bgra8, 0);
    assert_eq!(
        detector.compare(&previous, &current),
        ChangeDecision::AnalysisRequired
    );
}

#[test]
fn unsupported_codes_are_rejected_and_policy_values_are_stable() {
    assert_eq!(
        ChangeDetectionPolicy::try_from(ANALYSIS_ALWAYS_POLICY_CODE),
        Ok(ChangeDetectionPolicy::AnalysisAlways)
    );
    assert_eq!(
        ChangeDetectionPolicy::try_from(EXACT_RGBA_POLICY_CODE),
        Ok(ChangeDetectionPolicy::ExactRgba)
    );
    let error = ChangeDetectionPolicy::try_from(u32::MAX).expect_err("unsupported code");
    assert_eq!(error.code(), u32::MAX);
}

#[test]
fn detector_is_copy_send_sync_and_contains_no_worker_state() {
    fn assert_traits<T: Copy + Send + Sync + 'static>() {}

    assert_traits::<ChangeDetectionPolicy>();
    assert_traits::<mado_pilot_vision::ChangeDetectionDescriptor>();
    assert_traits::<ChangeDetector>();
    assert_eq!(
        size_of::<ChangeDetector>(),
        size_of::<ChangeDetectionPolicy>()
    );
    assert!(!needs_drop::<ChangeDetector>());
}
