//! Runtime checks for capture capabilities newer than the deployment minimum.
//!
//! The minimum supported macOS version is still gate `G-001`, and ScreenCaptureKit
//! arrived in 12.3. Rather than let the linker decide what happens below that,
//! the shim loads the framework at runtime and this module reports its absence as
//! a typed unsupported outcome at operation time. An application can therefore
//! include this package without making an unresolved minimum-macOS claim, and a
//! host that cannot capture still loads the library.

use std::sync::OnceLock;

use mado_pilot_capture::CaptureFault;
use mado_pilot_core::Result;

use crate::shim::{self, ShimStatus};

/// Verifies that the capture framework is loadable and its surface is the one
/// this build was written against.
///
/// Performs no authorization request and cannot present UI.
///
/// # Errors
///
/// Returns an unsupported outcome when the framework is absent, when the host
/// predates it, or when the linked shim disagrees with the declarations this
/// build mirrors.
pub(crate) fn ensure_capture_available() -> Result<()> {
    if !linked_shim_agrees() {
        return Err(CaptureFault::UnsupportedOption.into());
    }
    shim::capture_available().map_err(|status| match status {
        // Absence of the framework and a host below its minimum are the same
        // answer to the caller: this host cannot capture, and nothing failed.
        ShimStatus::Unsupported => CaptureFault::UnsupportedOption.into(),
        other => mado_pilot_core::Error::from(other),
    })
}

/// Reports whether the linked shim's surface version and structure sizes are the
/// ones this build mirrors.
///
/// A mismatch means the compiled shim and these declarations disagree about the
/// boundary's layout, which is a build defect rather than a host limitation.
/// Reporting it as unsupported keeps it from being read as capture data.
pub(crate) fn linked_shim_agrees() -> bool {
    static AGREES: OnceLock<bool> = OnceLock::new();
    *AGREES.get_or_init(|| {
        let (version, sizes) = shim::linked_layout();
        version == shim::ABI_VERSION && sizes == shim::declared_layout()
    })
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::Status;

    use super::{ensure_capture_available, linked_shim_agrees};

    #[test]
    fn the_availability_check_needs_no_window_authorization_or_ui() {
        // A host may legitimately report unsupported. What matters is that the
        // check is callable with no window, no run loop, and no authorization.
        match ensure_capture_available() {
            Ok(()) => {}
            Err(error) => assert_eq!(error.status(), Status::Unsupported),
        }
    }

    #[test]
    fn the_linked_shim_agrees_with_the_declarations_this_build_mirrors() {
        assert!(linked_shim_agrees());
    }
}
