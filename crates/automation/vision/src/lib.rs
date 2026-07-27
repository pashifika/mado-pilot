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
//! # Implementation status
//!
//! Phase 1 stage 3, partial. The template source and its matching defaults are
//! implemented and tested, because the asset package resolves validated
//! templates into them. Preprocessing descriptors, prepared templates, matching
//! requests, matching results, and the backend adapter contract are not: stage 4
//! adds them together with the OpenCV adapter. No template matching is
//! available. See `docs/architecture.md`.
//!
//! **Every public name here is provisional.** Naming is settled by gate `G-009`
//! before Phase 1 exits; see `docs/validation-gates.md`.
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

pub mod fault;
pub mod template;

pub use fault::VisionFault;
pub use template::{
    MatchDefaults, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
};
