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
//! It exists as its own package because none of the alternatives held up: the
//! test-support package must never ship, the capture package must not contain a
//! concrete adapter, the facade is a composition root rather than an
//! implementation, and replay is platform-neutral so it does not belong under
//! `crates/platform`. See
//! `docs/adr/0002-replay-capture-adapter-package.md`.
//!
//! # What it deliberately does not do
//!
//! No desktop enumeration, no permission probe, no host DPI lookup, no network
//! access, and no image decoding. A replay source stores raw pixels, so what a
//! fixture contains is exactly what a test observes, and no codec sits between
//! the two.
//!
//! # Allowed seam
//!
//! Depends on `mado-pilot-core` and `mado-pilot-capture` only. Nothing depends
//! on this package except the facade, which wires it as the Phase 1 default.
//!
//! # Implementation status
//!
//! Phase 1 stage 2. Discovery, session open, publication, latest-frame
//! selection, and close are implemented. **Every public name is provisional**
//! until gate `G-009` is resolved.
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
