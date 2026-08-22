//! Native Windows target discovery, capture, and input.
//!
//! This package is the only workspace package that names Win32, Windows
//! Graphics Capture (WGC), WinRT, D3D11, or DXGI. The platform-neutral capture
//! contracts depend on none of them.
//!
//! The implementation is target-gated: on non-Windows targets this crate keeps
//! an empty, documented seam and resolves no Windows dependency. Windows builds
//! expose `WindowsCaptureProvider`, which performs picker-free discovery and
//! creates free-threaded WGC sessions and explicitly requested input
//! controllers.
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
//! Discovery stages a complete snapshot before final operation arbitration,
//! mints fresh identities, and keeps only the current and previous retained-item
//! generations openable. Capture surfaces are checked against D3D11's axis limit
//! and a 128 MiB byte ceiling; producer, detached, staging, and mapped ownership
//! also shares 2 GiB per-session and 4 GiB process retained-byte ceilings. The
//! reported retained count is an extent-derived session-local maximum that
//! leaves headroom for the two producer surfaces plus one staging-and-output
//! mapping. Its public policy reports that the backing 4 GiB budget is shared
//! across Windows sessions, so process contention may produce pressure before
//! the local count is reached.
//!
//! Close stops admission, removes both native handlers, drains admitted
//! callbacks, and moves the WGC objects to an apartment-initialized teardown
//! worker. Explicit close polls that worker under the caller's operation
//! deadline; implicit destruction leaves the worker to finish its
//! uninterruptible callback drain and ordered native release.

#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod availability;
#[cfg(windows)]
mod benchmark_metrics;
#[cfg(windows)]
mod discovery;
#[cfg(windows)]
#[doc(hidden)]
#[allow(missing_docs)]
pub mod fixture_protocol;
#[cfg(all(windows, feature = "benchmark-instrumentation"))]
#[doc(hidden)]
pub mod benchmark {
    pub use crate::benchmark_metrics::{
        CallbackCopyObservation, CallbackMetricBaseline, CallbackObservationError,
        CaptureMetricsSnapshot, callback_copied_bytes_between, callback_metric_baseline,
        callback_observation_after, capture_metrics, dual_display_fixture_marker_points,
        dual_display_seam_x, reset_capture_metrics,
    };
}
#[cfg(windows)]
mod input;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod native_input;
#[cfg(windows)]
mod optional_api;
#[cfg(windows)]
mod provider;
#[cfg(windows)]
mod storage;
#[cfg(windows)]
mod window_authority;
#[cfg(windows)]
mod window_message;

#[cfg(windows)]
pub use provider::{PROVIDER, WindowsCaptureProvider};
