//! Runtime checks for capture capabilities newer than the deployment minimum.
//!
//! This implementation's declared deployment floor is macOS 26.5.2. It requires the
//! frame-attached `SCStreamFrameInfoScreenRect` key and is qualified only on the
//! Apple Silicon 26.5.2 (25F84) development host. The shim still loads the
//! framework under a runtime 26.5.2 gate and reports absence as typed unsupported.

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
/// Returns an unsupported outcome when the framework or required frame key is
/// absent, when the host predates macOS 26.5.2, or when the linked shim disagrees with
/// the declarations this build mirrors.
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

/// Reports whether the linked shim's version, structure sizes, and process-field
/// offsets are the ones this build mirrors.
///
/// A mismatch means the compiled shim and these declarations disagree about the
/// boundary's layout, which is a build defect rather than a host limitation.
/// Reporting it as unsupported keeps it from being read as capture data.
pub(crate) fn linked_shim_agrees() -> bool {
    static AGREES: OnceLock<bool> = OnceLock::new();
    *AGREES.get_or_init(|| {
        let (version, sizes, offsets) = shim::linked_layout();
        let base_agrees = version == shim::ABI_VERSION
            && sizes == shim::declared_layout()
            && offsets == shim::declared_process_offsets();
        #[cfg(feature = "sck-suspension-diagnostics")]
        {
            base_agrees && sck_diagnostics_layout_agrees(shim::linked_sck_diagnostics_layout())
        }
        #[cfg(not(feature = "sck-suspension-diagnostics"))]
        {
            base_agrees
        }
    })
}

#[cfg(feature = "sck-suspension-diagnostics")]
fn sck_diagnostics_layout_agrees(linked: [u32; 10]) -> bool {
    linked == shim::declared_sck_diagnostics_layout()
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::Status;

    #[cfg(feature = "sck-suspension-diagnostics")]
    use super::sck_diagnostics_layout_agrees;
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

    #[cfg(feature = "sck-suspension-diagnostics")]
    #[test]
    fn the_diagnostic_layout_gate_rejects_every_size_and_offset_mismatch() {
        let declared = crate::shim::declared_sck_diagnostics_layout();
        assert!(sck_diagnostics_layout_agrees(declared));
        for index in 0..declared.len() {
            let mut mismatched = declared;
            mismatched[index] ^= 1;
            assert!(
                !sck_diagnostics_layout_agrees(mismatched),
                "layout field {index} was not load-bearing"
            );
        }
    }
}
