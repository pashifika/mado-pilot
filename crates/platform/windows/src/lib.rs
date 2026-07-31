//! Native Windows target discovery and capture.
//!
//! This package is the only workspace package that names Win32, Windows
//! Graphics Capture (WGC), WinRT, D3D11, or DXGI. The platform-neutral capture
//! contracts depend on none of them.
//!
//! The implementation is target-gated: on non-Windows targets this crate keeps
//! an empty, documented seam and resolves no Windows dependency. Windows builds
//! expose [`WindowsCaptureProvider`], which performs picker-free discovery and
//! creates free-threaded WGC sessions.
//!
//! # Ownership
//!
//! The capture path implements
//! [ADR 0013](../../../docs/adr/0013-windows-capture-frame-detachment.md):
//! the WGC producer pool contains exactly two frames, and a callback copies a
//! publishable surface into finite, Adapter-owned D3D11 storage before releasing
//! the WGC frame. CPU mapping, consumer work, and host callbacks never run in the
//! WGC callback.
//!
//! Close stops admission, removes both native handlers, drains admitted
//! callbacks, closes the WGC objects, and releases the pool before the session.
//! An already terminal native session is not closed twice; a bounded event-queue
//! quiescence prevents a queued agile sender release from deadlocking the final
//! WinRT session release.

#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod availability;
#[cfg(windows)]
mod discovery;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod provider;
#[cfg(windows)]
mod storage;

#[cfg(windows)]
pub use provider::{PROVIDER, WindowsCaptureProvider};
