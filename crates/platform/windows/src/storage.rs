//! Adapter-owned D3D11 texture leases and lazy CPU mapping.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError, Weak};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{CaptureFault, CpuPixels, FrameDescriptor, FrameStorage, PixelFormat};
use mado_pilot_core::{Operation, OperationContext, PixelExtent, Result};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::{E_ACCESSDENIED, HMODULE, RO_E_CLOSED};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED,
    DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_SESSION_DISCONNECTED, IDXGIDevice,
};
use windows::core::Interface;

use crate::optional_api::create_direct3d_device;

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// The finite production detached-texture budget selected from the G-002
/// retained-frame workload.
pub(crate) const DETACHED_TEXTURE_BUDGET: NonZeroU32 = NonZeroU32::new(40).unwrap();

/// Orders D3D device termination against lazy mapping cache commits.
#[derive(Debug, Default)]
pub(crate) struct DeviceTerminal {
    fault: AtomicU8,
}

impl DeviceTerminal {
    pub(crate) fn record(&self, fault: CaptureFault) {
        let encoded = match fault {
            CaptureFault::DeviceRemoved => 1,
            CaptureFault::DeviceReset => 2,
            _ => return,
        };
        let _first = self
            .fault
            .compare_exchange(0, encoded, Ordering::AcqRel, Ordering::Acquire);
    }

    fn fault(&self) -> Option<CaptureFault> {
        match self.fault.load(Ordering::Acquire) {
            1 => Some(CaptureFault::DeviceRemoved),
            2 => Some(CaptureFault::DeviceReset),
            _ => None,
        }
    }
}

/// One D3D11 device/context lifetime domain shared by a session and every
/// detached frame it publishes.
pub(crate) struct DeviceDomain {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    context_gate: Mutex<()>,
}

impl DeviceDomain {
    pub(crate) fn create() -> Result<Arc<Self>> {
        let (device, context) = create_device(D3D_DRIVER_TYPE_HARDWARE)
            .or_else(|_| create_device(D3D_DRIVER_TYPE_WARP))
            .map_err(classify_native_error)?;
        Ok(Arc::new(Self {
            device,
            context,
            context_gate: Mutex::new(()),
        }))
    }

    pub(crate) fn winrt_device(&self) -> Result<IDirect3DDevice> {
        let dxgi_device: IDXGIDevice = self.device.cast().map_err(classify_native_error)?;
        // SAFETY: dxgi_device is obtained from this D3D11 device.
        let inspectable = create_direct3d_device(&dxgi_device).map_err(classify_native_error)?;
        inspectable.cast().map_err(classify_native_error)
    }

    /// Reports a terminal D3D device fault without replacing its native kind.
    pub(crate) fn device_fault(&self) -> Option<CaptureFault> {
        // SAFETY: GetDeviceRemovedReason reads device state only.
        unsafe { self.device.GetDeviceRemovedReason() }
            .err()
            .map(native_fault)
    }

    fn create_default_texture(
        &self,
        source: D3D11_TEXTURE2D_DESC,
    ) -> windows::core::Result<ID3D11Texture2D> {
        let mut descriptor = source;
        descriptor.Usage = D3D11_USAGE_DEFAULT;
        descriptor.BindFlags = 0;
        descriptor.CPUAccessFlags = 0;
        descriptor.MiscFlags = 0;
        let mut texture = None;
        // SAFETY: descriptor is fully initialized, initial data is absent, and
        // the output points to a valid Option owned by this call.
        unsafe {
            self.device
                .CreateTexture2D(&raw const descriptor, None, Some(&raw mut texture))
        }?;
        texture.ok_or_else(windows::core::Error::from_thread)
    }

    /// Issues the callback's full-resource detach copy without waiting for a
    /// context already in use by lazy mapping.
    pub(crate) fn try_copy(
        &self,
        target: &ID3D11Texture2D,
        source: &ID3D11Texture2D,
    ) -> std::result::Result<bool, CaptureFault> {
        let _context = match self.context_gate.try_lock() {
            Ok(context) => context,
            Err(TryLockError::WouldBlock) => return Ok(false),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        // SAFETY: target was created from source's descriptor in this device
        // domain and neither texture is mutable through a public owner.
        unsafe { self.context.CopyResource(target, source) };
        // Submit the detach copy before the callback releases the WGC frame.
        // Flush does not wait for GPU completion; it makes the command and its
        // producer-surface dependency visible to the driver promptly instead
        // of leaving session teardown to flush an arbitrary pending batch.
        // SAFETY: this is the same immediate context protected by context_gate;
        // Flush submits its queued commands and reads no caller memory.
        unsafe { self.context.Flush() };
        // A void D3D11 context command reports removal through the device.
        // SAFETY: GetDeviceRemovedReason reads device state only.
        unsafe { self.device.GetDeviceRemovedReason() }.map_err(native_fault)?;
        Ok(true)
    }

    fn read_texture(
        &self,
        texture: &ID3D11Texture2D,
        descriptor: FrameDescriptor,
        operation: &OperationContext,
    ) -> Result<Arc<CpuPixels>> {
        let mut attempt = Operation::admit(operation)?;
        let _context = lock_with_operation(&self.context_gate, &mut attempt)?;

        let mut source_descriptor = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: source_descriptor is writable for the complete native struct.
        unsafe { texture.GetDesc(&raw mut source_descriptor) };
        let mut staging_descriptor = source_descriptor;
        staging_descriptor.Usage = D3D11_USAGE_STAGING;
        staging_descriptor.BindFlags = 0;
        staging_descriptor.CPUAccessFlags =
            u32::try_from(D3D11_CPU_ACCESS_READ.0).expect("read access flag is non-negative");
        staging_descriptor.MiscFlags = 0;

        let mut staging = None;
        // SAFETY: descriptor and output storage are valid for this call.
        unsafe {
            self.device
                .CreateTexture2D(&raw const staging_descriptor, None, Some(&raw mut staging))
        }
        .map_err(classify_native_error)?;
        let staging = staging.ok_or(CaptureFault::SourceInvalid)?;
        // SAFETY: both resources belong to this device and have compatible
        // dimensions, format, sample description, mip count, and array size.
        unsafe { self.context.CopyResource(&staging, texture) };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging is CPU-readable and mapped is writable.
        unsafe {
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped))
        }
        .map_err(classify_native_error)?;
        let mapped_guard = MappedGuard {
            context: &self.context,
            resource: staging.cast().map_err(classify_native_error)?,
        };

        let row_pitch =
            usize::try_from(mapped.RowPitch).map_err(|_| CaptureFault::InconsistentDescriptor)?;
        if row_pitch < descriptor.row_bytes() || mapped.pData.is_null() {
            return Err(CaptureFault::InconsistentDescriptor.into());
        }
        let height = usize::try_from(descriptor.extent().height())
            .map_err(|_| CaptureFault::InconsistentDescriptor)?;
        let mut bytes = vec![0u8; descriptor.byte_len()];
        for row in 0..height {
            let source_offset = row
                .checked_mul(row_pitch)
                .ok_or(CaptureFault::InconsistentDescriptor)?;
            let target_offset = row
                .checked_mul(descriptor.stride())
                .ok_or(CaptureFault::InconsistentDescriptor)?;
            // SAFETY: Map guarantees RowPitch bytes for every resource row;
            // descriptor.row_bytes is no greater, and both checked offsets stay
            // within the mapped texture and owned output.
            let source = unsafe {
                std::slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(source_offset),
                    descriptor.row_bytes(),
                )
            };
            let target_end = target_offset
                .checked_add(descriptor.row_bytes())
                .ok_or(CaptureFault::ByteLengthMismatch)?;
            let target = bytes
                .get_mut(target_offset..target_end)
                .ok_or(CaptureFault::ByteLengthMismatch)?;
            target.copy_from_slice(source);
        }
        drop(mapped_guard);
        attempt.checkpoint()?;
        Ok(attempt.commit(Arc::new(CpuPixels::new(bytes.into_boxed_slice())))?)
    }
}

impl fmt::Debug for DeviceDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceDomain")
            .field("api", &"D3D11/WGC")
            .finish_non_exhaustive()
    }
}

fn create_device(
    driver: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE,
) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device = None;
    let mut context = None;
    // SAFETY: all optional outputs point to initialized Options, feature levels
    // remain alive for the call, and no software module is used.
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            Some(&raw mut context),
        )
    }?;
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(windows::core::Error::from_thread()),
    }
}

struct MappedGuard<'context> {
    context: &'context ID3D11DeviceContext,
    resource: ID3D11Resource,
}

impl Drop for MappedGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard is constructed only after Map succeeds for
        // subresource zero and is dropped exactly once.
        unsafe { self.context.Unmap(&self.resource, 0) };
    }
}

#[derive(Debug)]
pub(crate) struct TexturePool {
    domain: Arc<DeviceDomain>,
    state: Mutex<PoolState>,
    capacity: usize,
    deferred_discards: AtomicUsize,
}

#[derive(Debug, Default)]
struct PoolState {
    allocated: usize,
    leased: usize,
    shape: Option<TextureShape>,
    free: Vec<ID3D11Texture2D>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextureShape {
    width: u32,
    height: u32,
    format: i32,
    sample_count: u32,
    sample_quality: u32,
}

impl TextureShape {
    fn from_descriptor(descriptor: &D3D11_TEXTURE2D_DESC) -> Self {
        Self {
            width: descriptor.Width,
            height: descriptor.Height,
            format: descriptor.Format.0,
            sample_count: descriptor.SampleDesc.Count,
            sample_quality: descriptor.SampleDesc.Quality,
        }
    }
}

impl TexturePool {
    pub(crate) fn new(domain: Arc<DeviceDomain>) -> Arc<Self> {
        Arc::new(Self {
            domain,
            state: Mutex::new(PoolState::default()),
            capacity: usize::try_from(DETACHED_TEXTURE_BUDGET.get())
                .expect("u32 budget fits usize"),
            deferred_discards: AtomicUsize::new(0),
        })
    }

    /// Acquires without waiting. `None` is the finite-pressure outcome.
    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        descriptor: D3D11_TEXTURE2D_DESC,
    ) -> Result<Option<TextureLease>> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        self.apply_deferred_discards(&mut state);
        let shape = TextureShape::from_descriptor(&descriptor);
        if state.shape != Some(shape) {
            let retired = state.free.len();
            state.free.clear();
            state.allocated = state.allocated.saturating_sub(retired);
            state.shape = Some(shape);
        }
        let texture = if let Some(texture) = state.free.pop() {
            texture
        } else {
            if state.allocated >= self.capacity {
                return Ok(None);
            }
            let texture = self
                .domain
                .create_default_texture(descriptor)
                .map_err(classify_native_error)?;
            state.allocated += 1;
            texture
        };
        state.leased += 1;
        Ok(Some(TextureLease {
            texture: Some(texture),
            shape,
            pool: Arc::downgrade(self),
        }))
    }

    /// Retires reusable textures without waiting for a consumer-side release.
    ///
    /// Returning `false` is safe: the next successful acquisition observes the
    /// new descriptor shape and performs the same retirement before reuse.
    pub(crate) fn try_retire_for_resize(&self) -> bool {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return false,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        self.apply_deferred_discards(&mut state);
        let retired = state.free.len();
        state.free.clear();
        state.allocated = state.allocated.saturating_sub(retired);
        state.shape = None;
        true
    }

    fn release(&self, texture: ID3D11Texture2D, shape: TextureShape) {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                // Discarding is always safe. Defer only the accounting so a
                // callback-side lease drop never waits for a consumer holding
                // the pool mutex.
                self.deferred_discards.fetch_add(1, Ordering::AcqRel);
                drop(texture);
                return;
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        self.apply_deferred_discards(&mut state);
        state.leased = state.leased.saturating_sub(1);
        if state.shape == Some(shape) {
            state.free.push(texture);
        } else {
            state.allocated = state.allocated.saturating_sub(1);
        }
    }

    fn apply_deferred_discards(&self, state: &mut PoolState) {
        let discarded = self.deferred_discards.swap(0, Ordering::AcqRel);
        state.leased = state.leased.saturating_sub(discarded);
        state.allocated = state.allocated.saturating_sub(discarded);
    }

    #[cfg(test)]
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.apply_deferred_discards(&mut state);
        state
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        let state = self.lock();
        (state.allocated, state.leased, state.free.len())
    }
}

pub(crate) struct TextureLease {
    texture: Option<ID3D11Texture2D>,
    shape: TextureShape,
    pool: Weak<TexturePool>,
}

impl TextureLease {
    pub(crate) fn texture(&self) -> &ID3D11Texture2D {
        self.texture
            .as_ref()
            .expect("texture exists until lease drop")
    }
}

impl fmt::Debug for TextureLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextureLease")
            .field("shape", &self.shape)
            .finish_non_exhaustive()
    }
}

impl Drop for TextureLease {
    fn drop(&mut self) {
        if let (Some(texture), Some(pool)) = (self.texture.take(), self.pool.upgrade()) {
            pool.release(texture, self.shape);
        }
    }
}

pub(crate) struct WindowsFrameStorage {
    descriptor: FrameDescriptor,
    domain: Arc<DeviceDomain>,
    lease: TextureLease,
    mapping: Mutex<MappingState>,
    mapped: Condvar,
    failure: Weak<dyn StorageFailureSink>,
    device_terminal: Arc<DeviceTerminal>,
}

pub(crate) trait StorageFailureSink: Send + Sync {
    fn storage_failed(&self, fault: CaptureFault);
}

#[derive(Debug, Default)]
struct MappingState {
    active: bool,
    pixels: Option<Arc<CpuPixels>>,
}

impl WindowsFrameStorage {
    pub(crate) fn new(
        descriptor: FrameDescriptor,
        domain: Arc<DeviceDomain>,
        lease: TextureLease,
        failure: Weak<dyn StorageFailureSink>,
        device_terminal: Arc<DeviceTerminal>,
    ) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            domain,
            lease,
            mapping: Mutex::new(MappingState::default()),
            mapped: Condvar::new(),
            failure,
            device_terminal,
        })
    }

    fn mapping(&self) -> MutexGuard<'_, MappingState> {
        self.mapping
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for WindowsFrameStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.mapping();
        formatter
            .debug_struct("WindowsFrameStorage")
            .field("descriptor", &self.descriptor)
            .field("mapping_active", &state.active)
            .field("mapped", &state.pixels.is_some())
            .finish()
    }
}

impl FrameStorage for WindowsFrameStorage {
    fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    fn cpu_pixels(&self) -> Option<Arc<CpuPixels>> {
        // The answer is fixed for this storage's lifetime. Even after a lazy
        // conversion is cached, native storage remains a conversion path rather
        // than changing from None to Some.
        None
    }

    fn read_cpu(&self, operation: &OperationContext) -> Result<Arc<CpuPixels>> {
        let mut attempt = Operation::admit(operation)?;
        loop {
            let mut state = self.mapping();
            if let Some(pixels) = &state.pixels {
                return Ok(attempt.commit(Arc::clone(pixels))?);
            }
            if let Some(fault) = self.device_terminal.fault() {
                return Err(fault.into());
            }
            if !state.active {
                state.active = true;
                drop(state);
                break;
            }
            let (_state, _) = self
                .mapped
                .wait_timeout(state, LOCK_POLL_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            attempt.checkpoint()?;
        }

        let mut result = self
            .domain
            .read_texture(self.lease.texture(), self.descriptor, operation);
        if let Some(fault) = self.domain.device_fault() {
            self.device_terminal.record(fault);
            if let Some(failure) = self.failure.upgrade() {
                failure.storage_failed(fault);
            }
            result = Err(fault.into());
        }
        let mut result = match result {
            Ok(pixels) => attempt.commit(pixels).map_err(Into::into),
            Err(error) => Err(error),
        };
        let mut state = self.mapping();
        finish_mapping_cache(&self.device_terminal, &mut state, &mut result, || {});
        drop(state);
        self.mapped.notify_all();
        result
    }
}

fn finish_mapping_cache(
    terminal: &DeviceTerminal,
    state: &mut MappingState,
    result: &mut Result<Arc<CpuPixels>>,
    after_assignment: impl FnOnce(),
) {
    state.active = false;
    if let Some(fault) = terminal.fault() {
        *result = Err(fault.into());
        return;
    }
    if let Ok(pixels) = result {
        state.pixels = Some(Arc::clone(pixels));
        after_assignment();
        if let Some(fault) = terminal.fault() {
            state.pixels = None;
            *result = Err(fault.into());
        }
    }
}

fn lock_with_operation<'mutex>(
    mutex: &'mutex Mutex<()>,
    operation: &mut Operation<'_>,
) -> Result<MutexGuard<'mutex, ()>> {
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => {
                thread::sleep(LOCK_POLL_INTERVAL);
                operation.checkpoint()?;
            }
        }
    }
}

pub(crate) fn descriptor_from_native(
    descriptor: &D3D11_TEXTURE2D_DESC,
    content_extent: PixelExtent,
) -> Result<FrameDescriptor> {
    if descriptor.Format != windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM
        || descriptor.Width < content_extent.width()
        || descriptor.Height < content_extent.height()
    {
        return Err(CaptureFault::UnsupportedFormat.into());
    }
    FrameDescriptor::packed(content_extent, PixelFormat::Bgra8).map_err(Into::into)
}

pub(crate) fn classify_native_error(error: windows::core::Error) -> mado_pilot_core::Error {
    native_fault(error).into()
}

pub(crate) fn native_fault(error: windows::core::Error) -> CaptureFault {
    let code = error.code();
    if code == DXGI_ERROR_DEVICE_REMOVED || code == DXGI_ERROR_DEVICE_HUNG {
        CaptureFault::DeviceRemoved
    } else if code == DXGI_ERROR_DEVICE_RESET {
        CaptureFault::DeviceReset
    } else if code == E_ACCESSDENIED || code == DXGI_ERROR_ACCESS_LOST {
        CaptureFault::AccessDenied
    } else if code == RO_E_CLOSED {
        CaptureFault::CaptureItemClosed
    } else if code == DXGI_ERROR_SESSION_DISCONNECTED {
        CaptureFault::DisplayDisconnected
    } else {
        CaptureFault::SourceInvalid
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use windows::Win32::Foundation::{E_ACCESSDENIED, RO_E_CLOSED};
    use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT};
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
        DXGI_ERROR_SESSION_DISCONNECTED,
    };

    use super::{
        DETACHED_TEXTURE_BUDGET, DeviceDomain, DeviceTerminal, MappingState, TexturePool,
        finish_mapping_cache, native_fault,
    };
    use mado_pilot_capture::{CaptureFault, CpuPixels};
    use mado_pilot_core::Status;

    fn descriptor(width: u32, height: u32) -> D3D11_TEXTURE2D_DESC {
        D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: 0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        }
    }

    #[test]
    fn detached_textures_never_overwrite_a_live_lease() {
        let domain = DeviceDomain::create().expect("D3D11 device");
        let pool = TexturePool::new(domain);
        let mut leases = Vec::new();
        for _ in 0..DETACHED_TEXTURE_BUDGET.get() {
            leases.push(
                pool.try_acquire(descriptor(4, 4))
                    .expect("acquire")
                    .expect("within budget"),
            );
        }
        assert!(
            pool.try_acquire(descriptor(4, 4))
                .expect("bounded refusal")
                .is_none(),
            "the callback must drop rather than overwrite or allocate"
        );
        assert_eq!(
            pool.counts(),
            (
                usize::try_from(DETACHED_TEXTURE_BUDGET.get()).expect("fits"),
                usize::try_from(DETACHED_TEXTURE_BUDGET.get()).expect("fits"),
                0,
            )
        );

        drop(leases.pop());
        let resumed = pool
            .try_acquire(descriptor(4, 4))
            .expect("acquire after release")
            .expect("released capacity is reusable");
        assert_eq!(pool.counts().1, leases.len() + 1);
        drop(resumed);
        drop(leases);
        assert_eq!(pool.counts().1, 0);
    }

    #[test]
    fn resize_retires_only_unleased_old_generation_textures() {
        let domain = DeviceDomain::create().expect("D3D11 device");
        let pool = TexturePool::new(Arc::clone(&domain));
        let old = pool
            .try_acquire(descriptor(4, 4))
            .expect("acquire")
            .expect("lease");
        let reusable = pool
            .try_acquire(descriptor(4, 4))
            .expect("acquire")
            .expect("lease");
        drop(reusable);
        assert_eq!(pool.counts(), (2, 1, 1));

        assert!(pool.try_retire_for_resize());
        assert_eq!(pool.counts(), (1, 1, 0));
        let new = pool
            .try_acquire(descriptor(8, 6))
            .expect("new generation")
            .expect("capacity");
        assert_eq!(pool.counts(), (2, 2, 0));

        drop(old);
        assert_eq!(pool.counts(), (1, 1, 0));
        drop(new);
        assert_eq!(pool.counts(), (1, 0, 1));
    }

    #[test]
    fn lease_release_never_waits_for_the_pool_mutex() {
        let domain = DeviceDomain::create().expect("D3D11 device");
        let pool = TexturePool::new(domain);
        let lease = pool
            .try_acquire(descriptor(4, 4))
            .expect("acquire")
            .expect("lease");
        let guard = pool.state.lock().expect("pool lock");

        drop(lease);

        drop(guard);
        assert_eq!(
            pool.counts(),
            (0, 0, 0),
            "the discarded texture is reconciled after contention"
        );
    }

    #[test]
    fn native_device_and_lifecycle_errors_are_typed() {
        let fault = |code| native_fault(windows::core::Error::from_hresult(code));

        assert_eq!(
            fault(DXGI_ERROR_DEVICE_REMOVED),
            CaptureFault::DeviceRemoved
        );
        assert_eq!(fault(DXGI_ERROR_DEVICE_RESET), CaptureFault::DeviceReset);
        assert_eq!(fault(E_ACCESSDENIED), CaptureFault::AccessDenied);
        assert_eq!(fault(DXGI_ERROR_ACCESS_LOST), CaptureFault::AccessDenied);
        assert_eq!(fault(RO_E_CLOSED), CaptureFault::CaptureItemClosed);
        assert_eq!(
            fault(DXGI_ERROR_SESSION_DISCONNECTED),
            CaptureFault::DisplayDisconnected
        );
    }

    #[test]
    fn device_terminal_cancels_a_cache_assignment_before_it_becomes_visible() {
        let terminal = DeviceTerminal::default();
        let mut state = MappingState {
            active: true,
            pixels: None,
        };
        let mut result = Ok(Arc::new(CpuPixels::new(
            vec![1, 2, 3, 4].into_boxed_slice(),
        )));
        finish_mapping_cache(&terminal, &mut state, &mut result, || {
            terminal.record(CaptureFault::DeviceRemoved);
        });

        assert_eq!(terminal.fault(), Some(CaptureFault::DeviceRemoved));
        assert!(!state.active);
        assert!(state.pixels.is_none());
        assert_eq!(
            result.expect_err("device terminal rejects cache").status(),
            Status::CaptureFailed
        );
    }
}
