//! What a matching backend provides, and what it is not allowed to decide.
//!
//! A backend compiles a template and finds candidates. It does not resolve
//! coordinates, threshold scores, order results, suppress overlaps, apply a
//! result limit, or build the result envelope: those are the rules that must be
//! identical whichever backend ran, so they live in [`crate::matcher`] once
//! rather than in every adapter. This is the same division the capture package
//! makes, where an adapter supplies pixels and the stream assigns identity.
//!
//! Candidates come back in coordinates relative to the searched region's own
//! origin, because that is what a backend handed a mapping of that region
//! naturally produces. Translating them into full-frame capture pixels is one
//! addition, done in one place, so no adapter can get the offset wrong.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use mado_pilot_capture::{CpuMapping, PixelFormat};
use mado_pilot_core::{OperationContext, PixelExtent, PixelRect, Result};

use crate::prepared::PreparedTemplate;
use crate::request::MatchOptions;
use crate::template::TemplateSource;

/// A backend's compiled template state.
///
/// Implemented by backends and opaque to callers: the trait exposes nothing but
/// a downcast hook, so an OpenCV matrix reaches its own adapter and nothing
/// else. A backend must confirm the [`BackendId`] before downcasting, because a
/// payload from another backend is a caller mistake rather than a bug to
/// discover through a failed cast.
///
/// [`BackendId`]: crate::prepared::BackendId
pub trait TemplatePayload: Any + Send + Sync + fmt::Debug {
    /// Returns `self` for a backend to downcast to its own payload type.
    fn as_any(&self) -> &dyn Any;
}

/// One candidate a backend found, before any public rule has been applied.
///
/// The extent is not carried here: it is the prepared template's, so a backend
/// cannot report a match of a size the template never had.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    left: i32,
    top: i32,
    score: f64,
}

impl Candidate {
    /// Builds a candidate at `(left, top)` relative to the searched region's
    /// origin, with the backend's score.
    #[must_use]
    pub const fn new(left: i32, top: i32, score: f64) -> Self {
        Self { left, top, score }
    }

    /// Returns the offset from the searched region's origin.
    #[must_use]
    pub const fn origin(self) -> (i32, i32) {
        (self.left, self.top)
    }

    /// Returns the backend's score, which is validated before it is published.
    #[must_use]
    pub const fn score(self) -> f64 {
        self.score
    }
}

/// Everything a backend needs to execute one search.
///
/// The region has already been resolved, clipped, and proven large enough to
/// hold the template, and the pixels have already been mapped into the format
/// the backend's descriptor asked for. A backend that receives this can search;
/// it never has to decide whether it should.
#[derive(Debug)]
pub struct BackendRequest<'a> {
    /// The prepared template to search for.
    pub template: &'a PreparedTemplate,
    /// The searched region's pixels, in the backend's declared format.
    pub pixels: &'a CpuMapping,
    /// The searched region, in full-frame capture pixels, for diagnostics.
    pub region: PixelRect,
    /// The validated options. A backend may use the threshold to stop early; it
    /// is applied publicly regardless.
    pub options: MatchOptions,
}

/// A backend's public identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendDescriptor {
    id: Arc<str>,
    version: Arc<str>,
    format: PixelFormat,
}

impl BackendDescriptor {
    /// Builds a descriptor.
    ///
    /// `format` is the pixel format the backend requires; the matcher maps the
    /// searched region into it, so a backend never converts pixels a second
    /// time to get the layout it wanted.
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>, version: impl Into<Arc<str>>, format: PixelFormat) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            format,
        }
    }

    /// Returns the backend's stable identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the backend's version, which a result carries so a score can be
    /// attributed to the implementation that produced it.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the pixel format the backend requires.
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }
}

impl fmt::Display for BackendDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.id, self.version)
    }
}

/// A template-matching implementation.
///
/// Two behaviourally distinct implementations are what make this a seam rather
/// than a description of one adapter: the controlled double in
/// `mado-pilot-testkit` produces scripted candidates, latency, and failures,
/// and the OpenCV adapter produces real ones.
pub trait MatchBackend: fmt::Debug + Send + Sync {
    /// Returns the backend's public identity and required pixel format.
    fn descriptor(&self) -> BackendDescriptor;

    /// Compiles `source` into backend-private state.
    ///
    /// # Errors
    ///
    /// Returns a vision error when the content cannot be decoded, the encoding
    /// is unsupported, or the operation is interrupted. A failed preparation
    /// must not publish partially compiled state.
    fn prepare(
        &self,
        source: &TemplateSource,
        operation: &OperationContext,
    ) -> Result<PreparedTemplate>;

    /// Searches `request.pixels` for `request.template`.
    ///
    /// Candidates may be returned in any order and with any scores; the public
    /// rules are applied afterwards. A search that finds nothing returns an
    /// empty vector rather than an error.
    ///
    /// # Errors
    ///
    /// Returns a vision error when the backend is unavailable or fails, and the
    /// operation's terminal outcome when it is interrupted.
    fn find(
        &self,
        request: &BackendRequest<'_>,
        operation: &OperationContext,
    ) -> Result<Vec<Candidate>>;
}

/// The extent a prepared template occupies, used to bound candidates.
pub(crate) fn candidate_bounds(
    region: PixelRect,
    candidate: Candidate,
    extent: PixelExtent,
) -> Option<PixelRect> {
    let (left, top) = candidate.origin();
    let left = region.left().checked_add(left)?;
    let top = region.top().checked_add(top)?;
    let right = left.checked_add(i32::try_from(extent.width()).ok()?)?;
    let bottom = top.checked_add(i32::try_from(extent.height()).ok()?)?;
    let bounds = PixelRect::new(left, top, right, bottom).ok()?;
    region.contains_rect(bounds).then_some(bounds)
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::{PixelExtent, PixelRect};

    use super::{Candidate, candidate_bounds};

    fn region() -> PixelRect {
        PixelRect::new(100, 50, 300, 250).expect("valid")
    }

    #[test]
    fn a_candidate_is_translated_by_the_regions_origin() {
        let bounds = candidate_bounds(
            region(),
            Candidate::new(4, 6, 0.9),
            PixelExtent::new(10, 20),
        )
        .expect("inside");

        assert_eq!(bounds, PixelRect::new(104, 56, 114, 76).expect("valid"));
    }

    #[test]
    fn a_candidate_flush_with_the_far_edge_is_inside() {
        let bounds = candidate_bounds(
            region(),
            Candidate::new(190, 180, 1.0),
            PixelExtent::new(10, 20),
        )
        .expect("inside");

        assert_eq!(bounds, PixelRect::new(290, 230, 300, 250).expect("valid"));
    }

    #[test]
    fn a_candidate_that_would_leave_the_region_is_refused() {
        assert_eq!(
            candidate_bounds(
                region(),
                Candidate::new(191, 180, 1.0),
                PixelExtent::new(10, 20)
            ),
            None
        );
        assert_eq!(
            candidate_bounds(
                region(),
                Candidate::new(-1, 0, 1.0),
                PixelExtent::new(10, 20)
            ),
            None
        );
    }

    #[test]
    fn an_offset_that_would_overflow_is_refused_rather_than_wrapped() {
        assert_eq!(
            candidate_bounds(
                region(),
                Candidate::new(i32::MAX, 0, 1.0),
                PixelExtent::new(10, 20)
            ),
            None
        );
    }
}
