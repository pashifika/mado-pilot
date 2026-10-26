//! Deterministic replay capture for MadoPilot.
//!
//! # Responsibility
//!
//! This package implements the capture contracts against a configured replay
//! source rather than a live desktop. It owns reading a source from memory or
//! from a directory, validating what that source declares, and driving the
//! capture package's stream state so replay frames get the same identity rules
//! as any other frames.
//!
//! Replay is a platform-neutral concrete adapter: test support must not ship,
//! capture contracts must not depend on adapters, and the facade only wires them.
//!
//! # What it deliberately does not do
//!
//! No desktop enumeration, no permission probe, no host DPI lookup, no network
//! access, and no image decoding. A replay source stores raw pixels, so what a
//! fixture contains is exactly what a test observes, and no codec sits between
//! the two.
//!
//! Advancing a session moves the exact source frame into a short-lived
//! reservation. Successful publication consumes it once; interruption or
//! stream refusal restores the same allocation to the source head. The queue
//! lock is held only for constant-time removal or restoration and is never
//! nested with capture stream publication.
//!
//! # What a replay source is trusted to be
//!
//! A replay source is a caller's own configuration, and this package applies no
//! size ceiling to one. A directory source is read with an ordinary file read
//! and a memory source is bytes the caller already holds, so the manifest and
//! the pixel files are as large as the caller made them.
//!
//! Unlike caller-owned replay input, asset packages may contain untrusted data.
//! `mado-pilot-assets` therefore bounds manifest bytes, entry count, per-entry
//! bytes, source bytes, total expanded bytes, and expansion ratio.
//!
//! The distinction matters when the same reasoning is applied to a later
//! adapter. Ask whether the bytes are the host's own or something the host
//! accepted; only the second needs ceilings.
//!
//! # Allowed seam
//!
//! Depends on `mado-pilot-core` and `mado-pilot-capture` only. Nothing depends
//! on this package except the facade, which wires it as the Phase 1 default.
//!
//! # Implementation status
//!
//! Phase 1, complete. Discovery, session open, publication, latest-frame
//! selection, and close are implemented.
//!
//! **The public names here are reviewed, not yet stable.**
//! Renaming or removing a public name is a breaking change requiring a version
//! bump; additions are allowed. The stability promise begins at 1.0.
//! Current naming and compatibility policy lives in `docs/architecture.md`.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//!
//! use mado_pilot_adapter_replay::{ReplayFrame, ReplayProvider, ReplaySource, ReplayTarget};
//! use mado_pilot_capture::{
//!     CaptureProvider, Continuity, FrameDescriptor, FrameRequest, OpenRequest, PixelFormat,
//! };
//! use mado_pilot_core::{IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent};
//!
//! let descriptor = FrameDescriptor::packed(PixelExtent::new(4, 4), PixelFormat::Rgba8)?;
//! let frame = ReplayFrame::new(
//!     descriptor,
//!     MonotonicInstant::ORIGIN,
//!     Continuity::Continuous,
//!     None,
//!     vec![0x20; descriptor.byte_len()].into_boxed_slice(),
//! )?;
//! let source = ReplaySource::from_targets(vec![ReplayTarget::new("panel", vec![frame])?])?;
//!
//! let provider = ReplayProvider::new(Arc::new(IdentityIssuer::new()), source)?;
//! let operation = OperationContext::new();
//! let targets = provider.discover(&operation)?;
//! let session = provider.open(targets[0].id(), &OpenRequest::new(), &operation)?;
//!
//! let published = session.frame(&FrameRequest::latest(), &operation)?;
//! assert_eq!(published.stamp().sequence().value(), 0);
//! session.close(&operation)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod fault;
pub mod provider;
pub mod source;

pub use fault::ReplayFault;
pub use provider::{PROVIDER, ReplayProvider};
pub use source::{MANIFEST_NAME, ReplayFrame, ReplaySource, ReplayTarget, SCHEMA_VERSION};
