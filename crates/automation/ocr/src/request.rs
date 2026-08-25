//! Explicit source-correlated OCR requests.

use std::fmt;

use mado_pilot_capture::Frame;
use mado_pilot_core::{ClipPolicy, CoordinateSpace, OperationContext, Rect, Result};

use crate::backend::OcrBackendIdentity;
use crate::fault::OcrFault;
use crate::model::{ModelId, OcrModelIdentity, ProfileId};

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
/// Maximum caller-order zones admitted by one grouped scan.
pub const MAX_OCR_ZONES: usize = 8;

/// One source-relative zone and its clipping policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcrZone {
    rect: Rect,
    policy: ClipPolicy,
}

impl OcrZone {
    /// Builds one zone in its declared coordinate space.
    #[must_use]
    pub const fn new(rect: Rect, policy: ClipPolicy) -> Self {
        Self { rect, policy }
    }

    /// Returns the requested source rectangle.
    #[must_use]
    pub const fn rect(self) -> Rect {
        self.rect
    }

    /// Returns whether geometry outside the frame is rejected or clipped.
    #[must_use]
    pub const fn policy(self) -> ClipPolicy {
        self.policy
    }
}

/// One grouped scan request against one exact immutable frame.
///
/// The request borrows its frame, identities, zones, and operation context for
/// one synchronous call. A committed result owns every value it exposes.
pub struct OcrZoneScanRequest<'a> {
    frame: &'a Frame,
    backend: &'a OcrBackendIdentity,
    model: &'a OcrModelIdentity,
    zones: &'a [OcrZone],
    output_space: CoordinateSpace,
    operation: &'a OperationContext,
}

impl fmt::Debug for OcrZoneScanRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrZoneScanRequest")
            .field("stamp", &self.frame.stamp())
            .field("backend", self.backend)
            .field("model", &self.model())
            .field("profile", &self.profile())
            .field("zones", &self.zones)
            .field("output_space", &self.output_space)
            .finish()
    }
}

impl<'a> OcrZoneScanRequest<'a> {
    /// Builds a fully explicit grouped scan request.
    ///
    /// # Errors
    ///
    /// Returns [`OcrFault::ZoneCountOutOfRange`] unless `zones` contains one
    /// through eight caller-order entries.
    pub fn new(
        frame: &'a Frame,
        backend: &'a OcrBackendIdentity,
        model: &'a OcrModelIdentity,
        zones: &'a [OcrZone],
        output_space: CoordinateSpace,
        operation: &'a OperationContext,
    ) -> Result<Self> {
        if !(1..=MAX_OCR_ZONES).contains(&zones.len()) {
            return Err(OcrFault::ZoneCountOutOfRange.into());
        }
        Ok(Self {
            frame,
            backend,
            model,
            zones,
            output_space,
            operation,
        })
    }

    /// Returns the exact immutable source frame.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        self.frame
    }

    /// Returns the explicitly selected backend implementation.
    #[must_use]
    pub const fn backend(&self) -> &OcrBackendIdentity {
        self.backend
    }

    /// Returns complete explicitly selected model/profile identity.
    #[must_use]
    pub const fn model_identity(&self) -> &OcrModelIdentity {
        self.model
    }

    /// Returns the explicitly selected model identifier.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        self.model.model()
    }

    /// Returns the explicitly selected profile identifier.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        self.model.profile()
    }

    /// Returns caller-order zones.
    #[must_use]
    pub const fn zones(&self) -> &'a [OcrZone] {
        self.zones
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

/// One recognition request against one exact immutable frame.
///
/// The request borrows its frame, identities, and operation context for one
/// synchronous call. A committed result owns every value it exposes.
#[derive(Debug)]
pub struct OcrRequest<'a> {
    frame: &'a Frame,
    backend: &'a OcrBackendIdentity,
    model: &'a OcrModelIdentity,
    source_region: OcrRegion,
    output_space: CoordinateSpace,
    operation: &'a OperationContext,
}

impl<'a> OcrRequest<'a> {
    /// Builds a fully explicit recognition request.
    #[must_use]
    pub const fn new(
        frame: &'a Frame,
        backend: &'a OcrBackendIdentity,
        model: &'a OcrModelIdentity,
        source_region: OcrRegion,
        output_space: CoordinateSpace,
        operation: &'a OperationContext,
    ) -> Self {
        Self {
            frame,
            backend,
            model,
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

    /// Returns the explicitly selected backend implementation.
    #[must_use]
    pub const fn backend(&self) -> &OcrBackendIdentity {
        self.backend
    }

    /// Returns complete explicitly selected model/profile identity.
    #[must_use]
    pub const fn model_identity(&self) -> &OcrModelIdentity {
        self.model
    }

    /// Returns the explicitly selected model identifier.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        self.model.model()
    }

    /// Returns the explicitly selected profile identifier.
    #[must_use]
    pub const fn profile(&self) -> &ProfileId {
        self.model.profile()
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
