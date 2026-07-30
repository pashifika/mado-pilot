//! Platform-neutral MadoPilot core contracts.
//!
//! # Responsibility
//!
//! This package owns the vocabulary every other MadoPilot package shares: target
//! and stream identities, frame identity and ordering, coordinate spaces and
//! validated geometry, frame-time coordinate transforms, the monotonic clock
//! domain, operation deadlines and cancellation, and the shared status and error
//! types.
//!
//! The rules that make those values trustworthy live here once rather than in
//! every adapter. A capture adapter does not decide how a frame sequence
//! advances, and a vision backend does not decide how a fractional region rounds
//! to pixels; both call into this package, so two adapters cannot disagree.
//!
//! # Allowed seam
//!
//! This package depends on no other MadoPilot package, and on no Windows, macOS,
//! OpenCV, ONNX Runtime, GUI, or async-executor type. Platform-native handles are
//! never added here. It has no external dependency at all.
//!
//! # Implementation status
//!
//! Phase 1, complete. The identity, geometry, transform, time, operation-context,
//! and status contracts below are implemented and tested. Capture, mapping,
//! assets, matching, input, and OCR are not: this package describes what they
//! will agree on, not behavior that exists yet.
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
//! use std::time::Duration;
//!
//! use mado_pilot_core::geometry::{ClipPolicy, CoordinateSpace, PixelExtent, Rect};
//! use mado_pilot_core::identity::{GeometryRevision, IdentityIssuer, StreamCursor};
//! use mado_pilot_core::operation::{Operation, OperationContext};
//! use mado_pilot_core::transform::TransformSnapshot;
//!
//! // An engine issues identities that only it accepts.
//! let issuer = IdentityIssuer::new();
//! let mut cursor = StreamCursor::new(issuer.issue_stream()?);
//!
//! // Each published frame gets a complete identity.
//! let stamp = cursor.publish(GeometryRevision::FIRST)?;
//! assert_eq!(stamp.sequence().value(), 0);
//!
//! // Geometry is resolved against the frame's own transform snapshot.
//! let snapshot = TransformSnapshot::frame_only(stamp.geometry(), PixelExtent::new(800, 600));
//! let region = Rect::new(CoordinateSpace::FrameNormalized, 0.0, 0.0, 0.5, 0.5)?;
//! let pixels = snapshot.resolve_capture_pixels(region, ClipPolicy::Reject)?;
//! assert_eq!(pixels.extent(), PixelExtent::new(400, 300));
//!
//! // Blocking work carries a deadline and commits exactly one outcome. A real
//! // caller sizes the deadline to the work; this one is far larger than the
//! // three lines under it need, because the clock here is the host's and a
//! // documentation example that a scheduling stall can fail reports a defect
//! // that is not in the library.
//! let context = OperationContext::new().with_timeout(Duration::from_secs(60))?;
//! let operation = Operation::admit(&context).expect("not yet expired");
//! let value = operation.commit(pixels).expect("committed in time");
//! assert_eq!(value, pixels);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod geometry;
pub mod identity;
pub mod operation;
pub mod status;
pub mod time;
pub mod transform;

pub use geometry::{
    ClipPolicy, CoordinateSpace, GeometryFault, PixelExtent, PixelRect, Point, Rect,
};
pub use identity::{
    EngineId, FrameOrder, FrameSequence, FrameStamp, GeometryRevision, IdentityFault,
    IdentityIssuer, ProviderId, StreamCursor, StreamEpoch, StreamId, TargetId,
};
pub use operation::{CancellationToken, Interruption, Operation, OperationContext};
pub use status::{Error, Result, Status};
pub use time::{Clock, MonotonicInstant, SystemClock};
pub use transform::{Scale, TargetPlacement, TransformSnapshot};
