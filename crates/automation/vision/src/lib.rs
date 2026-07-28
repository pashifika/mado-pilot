//! MadoPilot vision contracts.
//!
//! # Responsibility
//!
//! This package owns the template-matching backend contract, template source
//! descriptors, preprocessing descriptors, and matching requests and results
//! including their correlation with a source frame identity.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core and capture packages. The
//! capture dependency exists because public matching operations consume
//! capture-owned frame views; it is a contract-to-contract dependency and
//! exposes no adapter type. OpenCV and other vision backends implement these
//! contracts; this package never depends on them.
//!
//! Nothing here names an asset package, a manifest, or a package entry. A
//! template source is built from plain values, so a caller may supply one from
//! bytes it read itself and never adopt the asset manifest at all.
//!
//! # What a backend does not decide
//!
//! A [`MatchBackend`] compiles a template and finds candidates. Region
//! resolution, score validation, thresholding, canonical ordering, overlap
//! suppression, and the result limit are applied by [`Matcher`] afterwards, so
//! two backends cannot disagree about what a match *is*. This is the same
//! division the capture package makes between an adapter that supplies pixels
//! and a stream that assigns identity.
//!
//! Three outcomes that look like failures are successes with no matches:
//! nothing scored high enough, the template is larger than the searched region,
//! and a clip-permitted region that misses the frame entirely.
//!
//! # Implementation status
//!
//! Phase 1, complete. Template sources, prepared-template ownership, typed
//! requests and options, backend descriptors, immutable source-correlated
//! results, and the deterministic rules in [`Matcher`] are implemented and
//! tested against two behaviourally distinct backends: the controlled double in
//! `mado-pilot-testkit` and the OpenCV CPU adapter in
//! `mado-pilot-backend-opencv`, which both pass the same contract suite.
//!
//! Preprocessing descriptors are not implemented and are not reserved as an
//! empty seam. See `docs/architecture.md`.
//!
//! **The public names here are reviewed, not yet stable.**
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md` records the
//! review that settled them and the policy that now applies: renaming or
//! removing one is a breaking change needing an ADR and a version bump, while
//! adding is free. The stability promise itself begins at 1.0.
//!
//! # Where to start
//!
//! ```
//! use std::sync::Arc;
//!
//! use mado_pilot_core::{CoordinateSpace, PixelExtent};
//! use mado_pilot_vision::{
//!     MatchDefaults, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
//! };
//!
//! # let png_bytes: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00];
//! // Content is identified by its own bytes, never by a declared extension.
//! let encoding = TemplateEncoding::identify(png_bytes).expect("a PNG signature");
//!
//! let template = TemplateSource::new(TemplateSourceRequest {
//!     id: TemplateId::new("start-button")?,
//!     encoding,
//!     extent: PixelExtent::new(24, 24),
//!     space: CoordinateSpace::CapturePixels,
//!     defaults: MatchDefaults::new(0.9, 8)?,
//!     content: Arc::from(png_bytes),
//! })?;
//!
//! assert_eq!(template.id().as_str(), "start-button");
//! # Ok::<(), mado_pilot_vision::VisionFault>(())
//! ```

pub mod backend;
pub mod fault;
pub mod matcher;
pub mod prepared;
pub mod request;
pub mod result;
pub mod template;

pub use backend::{BackendDescriptor, BackendRequest, Candidate, MatchBackend, TemplatePayload};
pub use fault::VisionFault;
pub use matcher::Matcher;
pub use prepared::{BackendId, PreparedTemplate};
pub use request::{MatchOptions, MatchRequest, RegionSelection, Suppression};
pub use result::{Match, MatchResult};
pub use template::{
    MatchDefaults, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
};
