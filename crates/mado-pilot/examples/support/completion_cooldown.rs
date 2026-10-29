//! Shared private caller policy; no public runner or timer.

use std::time::Duration;

use mado_pilot::{
    CoordinateSpace, Frame, OcrBackendDescriptor, OcrRegion, OcrRequest, OcrResult,
    OperationContext, Result, Session,
};

pub(super) const WAIT_SLICE: Duration = Duration::from_millis(2);

pub(super) fn recognize_exact(
    session: &Session,
    backend: &OcrBackendDescriptor,
    frame: &Frame,
    region: OcrRegion,
    operation: &OperationContext,
) -> Result<OcrResult> {
    session.recognize(OcrRequest::new(
        frame,
        backend.backend_identity(),
        backend.model_identity(),
        region,
        CoordinateSpace::CapturePixels,
        operation,
    ))
}

pub(super) fn cooldown(
    interval: Duration,
    operation: &OperationContext,
    sleep: &mut impl FnMut(Duration),
) -> Result<()> {
    checkpoint(operation)?;
    let started = operation.now();
    loop {
        checkpoint(operation)?;
        // Elapsed arithmetic cannot overflow near the end of the clock domain.
        let elapsed = operation.now().saturating_duration_since(started);
        if elapsed >= interval {
            return checkpoint(operation);
        }
        let remaining = interval.saturating_sub(elapsed);
        let slice = WAIT_SLICE
            .min(remaining)
            .min(operation.remaining().unwrap_or(remaining));
        if !slice.is_zero() {
            sleep(slice);
        }
        // Zero remainder reaches the checkpoint, never a zero-duration sleep.
    }
}

pub(super) fn checkpoint(operation: &OperationContext) -> Result<()> {
    match operation.interruption() {
        Some(interruption) => Err(interruption.into()),
        None => Ok(()),
    }
}
