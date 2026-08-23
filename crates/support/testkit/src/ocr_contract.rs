//! Backend-independent OCR conformance checks.

use std::sync::Arc;

use mado_pilot_capture::PixelFormat;
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, Rect, Status,
};
use mado_pilot_ocr::{OcrBackend, OcrRecognizer, OcrRegion, OcrRequest};

pub use crate::vision_contract::frame;

fn source_frame() -> mado_pilot_capture::Frame {
    frame(PixelExtent::new(32, 24), PixelFormat::Bgra8, 0)
}

/// Proves a backend descriptor is stable and complete.
///
/// # Panics
///
/// Panics when identity changes between reads.
pub fn descriptor_is_stable(backend: Arc<dyn OcrBackend>) {
    let first = backend.descriptor();
    let second = backend.descriptor();
    assert_eq!(first, second);
    assert!(!first.id().as_str().is_empty());
    assert!(!first.version().as_str().is_empty());
    assert!(!first.model().as_str().is_empty());
    assert!(!first.profile().as_str().is_empty());
}

/// Proves cancellation before admission never publishes a result.
///
/// # Panics
///
/// Panics when the request succeeds or reports another status.
pub fn an_already_cancelled_request_is_refused(backend: Arc<dyn OcrBackend>) {
    let recognizer = OcrRecognizer::new(backend);
    let descriptor = recognizer.descriptor();
    let frame = source_frame();
    let token = CancellationToken::new();
    token.cancel();
    let context = OperationContext::new().with_cancellation(token);
    let request = OcrRequest::new(
        &frame,
        descriptor.id(),
        descriptor.model(),
        descriptor.profile(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        &context,
    );

    assert_eq!(
        recognizer.recognize(request).unwrap_err().status(),
        Status::Cancelled
    );
}

/// Proves a region that clips to nothing is an invalid request.
///
/// # Panics
///
/// Panics when backend work produces a result or another status.
pub fn a_clipped_empty_region_is_refused(backend: Arc<dyn OcrBackend>) {
    let recognizer = OcrRecognizer::new(backend);
    let descriptor = recognizer.descriptor();
    let frame = source_frame();
    let context = OperationContext::new();
    let outside = Rect::new(CoordinateSpace::CapturePixels, 100.0, 100.0, 110.0, 110.0)
        .expect("valid outside rectangle");
    let request = OcrRequest::new(
        &frame,
        descriptor.id(),
        descriptor.model(),
        descriptor.profile(),
        OcrRegion::Region {
            rect: outside,
            policy: ClipPolicy::Clip,
        },
        CoordinateSpace::CapturePixels,
        &context,
    );

    assert_eq!(
        recognizer.recognize(request).unwrap_err().status(),
        Status::InvalidArgument
    );
}

/// Proves a successful empty result owns exact source correlation.
///
/// # Panics
///
/// Panics when recognition fails or substitutes identity, geometry, or backend.
pub fn an_empty_result_correlates_with_its_exact_source(backend: Arc<dyn OcrBackend>) {
    let recognizer = OcrRecognizer::new(backend);
    let descriptor = recognizer.descriptor();
    let frame = source_frame();
    let context = OperationContext::new();
    let request = OcrRequest::new(
        &frame,
        descriptor.id(),
        descriptor.model(),
        descriptor.profile(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        &context,
    );
    let result = recognizer.recognize(request).expect("recognition succeeds");

    assert!(result.is_empty());
    assert_eq!(result.stamp(), frame.stamp());
    assert_eq!(result.transform(), frame.transform());
    assert_eq!(
        result.effective_region(),
        frame.transform().frame_bounds().unwrap()
    );
    assert_eq!(result.backend(), &descriptor);
}

/// Proves close is idempotent under a live context.
///
/// # Panics
///
/// Panics when either close fails.
pub fn close_is_idempotent(backend: Arc<dyn OcrBackend>) {
    let recognizer = OcrRecognizer::new(backend);
    let context = OperationContext::new();
    recognizer.close(&context).expect("first close succeeds");
    recognizer.close(&context).expect("second close succeeds");
}

/// Runs every backend-independent OCR conformance check.
///
/// # Panics
///
/// Panics on the first failed check.
pub fn run(backend: &Arc<dyn OcrBackend>) {
    descriptor_is_stable(Arc::clone(backend));
    an_already_cancelled_request_is_refused(Arc::clone(backend));
    a_clipped_empty_region_is_refused(Arc::clone(backend));
    an_empty_result_correlates_with_its_exact_source(Arc::clone(backend));
    close_is_idempotent(Arc::clone(backend));
}
