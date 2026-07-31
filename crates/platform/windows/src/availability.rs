//! Runtime checks for WGC capabilities that may be newer than the deployment
//! minimum eventually selected by G-001.

use std::sync::OnceLock;

use mado_pilot_capture::CaptureFault;
use mado_pilot_core::Result;
use windows::Graphics::Capture::{GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{
    RO_INIT_MULTITHREADED, RoGetActivationFactory, RoInitialize, RoUninitialize,
};
use windows::core::{HSTRING, RuntimeName};

thread_local! {
    /// WinRT initialization belongs to a thread, not a provider. This value
    /// pairs an initialization owned by the package at thread teardown.
    static WINRT_APARTMENT: Apartment = Apartment::initialize();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApartmentState {
    Owned,
    Borrowed,
    Failed,
}

struct Apartment {
    state: ApartmentState,
}

impl Apartment {
    fn initialize() -> Self {
        // SAFETY: this initializer runs once per thread.
        let state = match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => ApartmentState::Owned,
            Err(error) if error.code() == RPC_E_CHANGED_MODE => {
                // The caller already initialized another COM apartment.
                ApartmentState::Borrowed
            }
            Err(_) => ApartmentState::Failed,
        };
        Self { state }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.state == ApartmentState::Owned {
            // SAFETY: this thread successfully paired RoInitialize with this
            // guard, which is dropped on the same thread. The process-wide MTA
            // usage below keeps windows-rs factory caches valid independently.
            unsafe { RoUninitialize() };
        }
    }
}

/// Verifies that WGC and the picker-free capture-item interop factory exist.
///
/// This performs no authorization request and cannot show a picker. Absence is
/// reported as a typed unsupported capture outcome at operation time.
pub(crate) fn ensure_capture_available() -> Result<()> {
    ensure_winrt_apartment()?;
    if !GraphicsCaptureSession::IsSupported().map_err(|_| CaptureFault::UnsupportedOption)? {
        return Err(CaptureFault::UnsupportedOption.into());
    }

    capture_item_factory()?;
    Ok(())
}

/// Returns the picker-free desktop capture-item factory after initializing the
/// calling thread's WinRT apartment.
pub(crate) fn capture_item_factory() -> Result<IGraphicsCaptureItemInterop> {
    ensure_winrt_apartment()?;
    let class_name = HSTRING::from(GraphicsCaptureItem::NAME);
    // SAFETY: the requested interface is the documented desktop interop factory
    // for GraphicsCaptureItem. Failure is kept at the operation boundary.
    let factory: windows::core::Result<IGraphicsCaptureItemInterop> =
        unsafe { RoGetActivationFactory(&class_name) };
    factory.map_err(|_| CaptureFault::UnsupportedOption.into())
}

fn ensure_winrt_apartment() -> Result<()> {
    static PROCESS_MTA: OnceLock<bool> = OnceLock::new();
    let process_mta = *PROCESS_MTA.get_or_init(|| {
        // SAFETY: the returned opaque usage token is intentionally held by the
        // process for its remaining lifetime. This keeps windows-rs' process
        // factory caches backed by an MTA even when a short-lived calling thread
        // exits; the operating system releases the process-scoped reference at
        // process teardown.
        unsafe { CoIncrementMTAUsage() }.is_ok()
    });
    if !process_mta {
        return Err(CaptureFault::UnsupportedOption.into());
    }
    WINRT_APARTMENT.with(|apartment| match apartment.state {
        ApartmentState::Owned | ApartmentState::Borrowed => Ok(()),
        ApartmentState::Failed => Err(CaptureFault::UnsupportedOption.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::ensure_capture_available;

    #[test]
    fn availability_check_never_requires_a_picker() {
        // The host may legitimately report unsupported, but the check must be
        // callable without a window, UI thread, or permission interaction.
        let _outcome = ensure_capture_available();
    }
}
