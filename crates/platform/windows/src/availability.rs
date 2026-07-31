//! Runtime checks for WGC capabilities that may be newer than the deployment
//! minimum eventually selected by G-001.

use std::sync::OnceLock;

use mado_pilot_capture::CaptureFault;
use mado_pilot_core::Result;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    IDirect3D11CaptureFramePoolStatics2, IGraphicsCaptureSessionStatics,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::RO_INIT_MULTITHREADED;
use windows::core::{Interface, RuntimeName, Type};

use crate::optional_api::{
    activation_factory, geometry_api_available, increment_mta_usage, initialize_winrt,
    uninitialize_winrt, winrt_loader_available,
};

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
        let state = match initialize_winrt(RO_INIT_MULTITHREADED) {
            Some(result) if result.is_ok() => ApartmentState::Owned,
            Some(result) if result == RPC_E_CHANGED_MODE => {
                // The caller already initialized another COM apartment.
                ApartmentState::Borrowed
            }
            _ => ApartmentState::Failed,
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
            uninitialize_winrt();
        }
    }
}

/// Verifies that WGC and the picker-free capture-item interop factory exist.
///
/// This performs no authorization request and cannot show a picker. Absence is
/// reported as a typed unsupported capture outcome at operation time.
pub(crate) fn ensure_capture_available() -> Result<()> {
    ensure_winrt_apartment()?;
    if !winrt_loader_available() {
        return Err(CaptureFault::UnsupportedOption.into());
    }
    let session_factory: IGraphicsCaptureSessionStatics =
        activation_factory(GraphicsCaptureSession::NAME)
            .map_err(|_| CaptureFault::UnsupportedOption)?;
    let mut supported = false;
    // SAFETY: supported is writable and session_factory is the documented
    // activation factory interface for GraphicsCaptureSession.
    unsafe {
        (Interface::vtable(&session_factory).IsSupported)(
            Interface::as_raw(&session_factory),
            &raw mut supported,
        )
    }
    .ok()
    .map_err(|_| CaptureFault::UnsupportedOption)?;
    if !supported {
        return Err(CaptureFault::UnsupportedOption.into());
    }
    if !geometry_api_available() {
        return Err(CaptureFault::UnsupportedOption.into());
    }

    capture_item_factory()?;
    Ok(())
}

/// Returns the picker-free desktop capture-item factory after initializing the
/// calling thread's WinRT apartment.
pub(crate) fn capture_item_factory() -> Result<IGraphicsCaptureItemInterop> {
    ensure_winrt_apartment()?;
    activation_factory(GraphicsCaptureItem::NAME)
        .map_err(|_| CaptureFault::UnsupportedOption.into())
}

pub(crate) fn create_free_threaded_frame_pool(
    device: &IDirect3DDevice,
    pixel_format: DirectXPixelFormat,
    buffers: i32,
    size: SizeInt32,
) -> windows::core::Result<Direct3D11CaptureFramePool> {
    let factory: IDirect3D11CaptureFramePoolStatics2 =
        activation_factory(Direct3D11CaptureFramePool::NAME)?;
    let mut frame_pool = std::ptr::null_mut();
    // SAFETY: device is a live WinRT D3D device, frame_pool is writable, and the
    // remaining values are validated by WGC.
    unsafe {
        (Interface::vtable(&factory).CreateFreeThreaded)(
            Interface::as_raw(&factory),
            Interface::as_raw(device),
            pixel_format,
            buffers,
            size,
            &raw mut frame_pool,
        )
    }
    .ok()?;
    // SAFETY: a successful factory call returned one owned frame-pool reference.
    unsafe { Type::from_abi(frame_pool) }
}

pub(crate) fn ensure_winrt_apartment() -> Result<()> {
    static PROCESS_MTA: OnceLock<bool> = OnceLock::new();
    let process_mta = *PROCESS_MTA.get_or_init(|| {
        // SAFETY: the returned opaque usage token is intentionally held by the
        // process for its remaining lifetime. This keeps windows-rs' process
        // factory caches backed by an MTA even when a short-lived calling thread
        // exits; the operating system releases the process-scoped reference at
        // process teardown.
        increment_mta_usage()
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
