//! MadoPilot capture, frame, and mapping contracts.
//!
//! # Responsibility
//!
//! This package defines what a capture adapter must provide and what a caller
//! receives: target discovery and session opening, the authoritative stream
//! state that assigns frame identity, immutable frames and the views taken over
//! them, and CPU mappings with explicit ownership.
//!
//! The stream state is the part worth understanding first. Every adapter drives
//! [`StreamState`], so epochs, sequences, geometry revisions, latest-frame
//! semantics, and close behavior are the same whether frames come from a replay
//! file or from a compositor. An adapter supplies pixels and says how a frame
//! relates to the last one; it does not get to invent identity, and a claim of
//! continuity that the pixels contradict is overruled.
//!
//! # Ownership
//!
//! A [`Frame`] outlives its session, a [`FrameView`] retains its exact source
//! frame, and a [`CpuMapping`] outlives both. Releasing a frame, publishing a
//! later one, or closing the session cannot invalidate anything a caller already
//! holds. Mapped bytes borrow from the mapping, so the compiler enforces the
//! lifetime rule the C ABI will later have to state in prose.
//! [`StreamState::publish_recoverable`] additionally lets an Adapter recover
//! the complete owned [`Publication`] when the stream refuses it, without
//! changing the existing [`StreamState::publish`] contract.
//!
//! Frame storage is private and there is no public storage enum. Phase 1 frames
//! are always CPU bytes, but Windows frames will be GPU textures, and a caller
//! that had learned to match on storage would break. What a caller gets instead
//! is [`Frame::map`], which is the same call either way.
//!
//! # Allowed seam
//!
//! This package depends only on `mado-pilot-core`. It contains no Windows,
//! macOS, OpenCV, or executor type, and no concrete adapter: an adapter
//! implements [`CaptureProvider`] and [`CaptureSession`] from its own package.
//!
//! # Implementation status
//!
//! Phase 1, complete. Discovery, session lifecycle, publication, frames, views,
//! and CPU mapping are implemented. Watchers, queue policy, change detection,
//! native frame storage, and one-shot capture are not, and none of them is
//! reserved here as an empty seam.
//!
//! **The public names here are reviewed, not yet stable.**
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md` records the
//! review that settled them and the policy that now applies: renaming or
//! removing one is a breaking change needing an ADR and a version bump, while
//! adding is free. The stability promise itself begins at 1.0.

pub mod descriptor;
pub mod discovery;
pub mod fault;
pub mod frame;
pub mod mapping;
pub mod session;
pub mod storage;
pub mod stream;

pub use descriptor::{
    CoordinateSupport, FrameDescriptor, OverflowPolicy, PixelFormat, QueuePolicy,
    SessionDescription, TargetDescription,
};
pub use discovery::DiscoveryRequest;
pub use fault::CaptureFault;
pub use frame::{Frame, FrameView};
pub use mapping::CpuMapping;
pub use session::{CaptureProvider, CaptureSession, OpenRequest};
pub use storage::{CpuFrameStorage, CpuPixels, FrameStorage};
pub use stream::{
    Continuity, FrameRequest, FrameSelection, Lifecycle, Publication, RefusedPublication,
    RefusedStorage, StoragePublication, StreamState,
};
