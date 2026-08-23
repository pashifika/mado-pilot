//! Explicit source-correlated OCR requests.

use mado_pilot_capture::Frame;
use mado_pilot_core::{ClipPolicy, CoordinateSpace, OperationContext, Rect};

use crate::model::{BackendId, ModelId, ProfileId};

/// Which part of one exact source frame to recognize.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum OcrRegion {
    /// Recognize the complete frame.
    FullFrame,
    /// Resolve a region through the frame's immutable transform snapshot.
    Region {
        /// Region in its declared coordinate space.
        rect: Rect,
        /// Whether to reject or clip geometry outside the frame.
        policy: ClipPolicy,
    },
}

/// One recognition request against one exact immutable frame.
///
/// The request borrows its frame, identities, and operation context for one
/// synchronous call. A committed result owns every value it exposes.
#[derive(Debug)]
pub struct OcrRequest<'a> {
    frame: &'a Frame,
    backend: &'a BackendId,
    model: &'a ModelId,
    profile: &'a ProfileId,
    source_region: OcrRegion,
    output_space: CoordinateSpace,
    operation: &'a OperationContext,
}

impl<'a> OcrRequest<'a> {
    /// Builds a fully explicit recognition request.
    #[must_use]
    pub const fn new(
        frame: &'a Frame,
        backend: &'a BackendId,
        model: &'a ModelId,
        profile: &'a ProfileId,
        source_region: OcrRegion,
        output_space: CoordinateSpace,
        operation: &'a OperationContext,
    ) -> Self {
        Self {
            frame,
            backend,
            model,
            profile,
            source_region,
            output_space,
            operation,
        }
    }

    /// Returns the exact immutable source frame.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        self.frame
    }

    /// Returns the explicitly selected backend.
    #[must_use]
    pub const fn backend(&self) -> &BackendId {
        self.backend
    }

    /// Returns the explicitly selected model.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        self.model
    }

    /// Returns the explicitly selected profile.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        self.profile
    }

    /// Returns the requested source region and clipping policy.
    #[must_use]
    pub const fn source_region(&self) -> OcrRegion {
        self.source_region
    }

    /// Returns the coordinate space requested for recognized geometry.
    #[must_use]
    pub const fn output_space(&self) -> CoordinateSpace {
        self.output_space
    }

    /// Returns the one operation context shared by every blocking stage.
    #[must_use]
    pub const fn operation(&self) -> &OperationContext {
        self.operation
    }
}
