//! Adapter-owned D3D11 texture leases and lazy CPU mapping.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
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

use crate::benchmark_metrics;
use crate::optional_api::create_direct3d_device;

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// The finite production detached-texture budget selected from the G-002
/// retained-frame workload.
pub(crate) const DETACHED_TEXTURE_BUDGET: NonZeroU32 = NonZeroU32::new(40).unwrap();

/// D3D11's feature-level 11 two-dimensional texture-axis ceiling.
pub(crate) const MAX_TEXTURE_AXIS: u32 = 16_384;

/// Largest single BGRA capture surface admitted by this Adapter: 128 MiB.
///
/// This includes 8K UHD (7,680 x 4,320 is about 126.6 MiB) while rejecting a
/// 16,384-square texture even though D3D11 permits both axes independently.
pub(crate) const MAX_SURFACE_BYTES: u64 = 128 * 1024 * 1024;

/// Retained native and CPU bytes one session may own: 2 GiB.
///
/// Two GiB admits the G-002 4K workload (two producer surfaces, 30 retained
/// frames, one staging texture, and one CPU output) and all 40 detached
/// allocations advertised at 4K with one mapping in flight. It remains a
/// reviewed safety ceiling, not the still-open Phase 2 G-013 performance
/// budget.
pub(crate) const SESSION_RETAINED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Retained native and CPU bytes across all Windows sessions: 4 GiB.
///
/// This admits at least two fully active 4K sessions with retained history while
/// bounding concurrent sessions independently of the teardown permit count.
pub(crate) const GLOBAL_RETAINED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SurfaceLayout {
    row_bytes: u64,
    bytes: u64,
}

impl SurfaceLayout {
    pub(crate) const fn bytes(self) -> u64 {
        self.bytes
    }
}

/// Returns the retained-allocation count supportable at this extent while two
/// producer surfaces and one conservative staging-plus-output mapping remain
/// reserved under the session byte ceiling.
pub(crate) fn retained_storage_capacity(
    layout: SurfaceLayout,
) -> std::result::Result<NonZeroU32, CaptureFault> {
    const REQUIRED_OVERHEAD_SURFACES: u64 = 4;

    let overhead = layout
        .bytes()
        .checked_mul(REQUIRED_OVERHEAD_SURFACES)
        .ok_or(CaptureFault::ResourceLimitExceeded)?;
    let available = SESSION_RETAINED_BYTES
        .checked_sub(overhead)
        .ok_or(CaptureFault::ResourceLimitExceeded)?;
    let by_bytes = available / layout.bytes();
    let capacity = by_bytes.min(u64::from(DETACHED_TEXTURE_BUDGET.get()));
    let capacity = u32::try_from(capacity).map_err(|_| CaptureFault::ResourceLimitExceeded)?;
    NonZeroU32::new(capacity).ok_or(CaptureFault::ResourceLimitExceeded)
}

/// Validates every axis and checked BGRA multiplication before allocation.
pub(crate) fn validate_surface(
    width: u32,
    height: u32,
) -> std::result::Result<SurfaceLayout, CaptureFault> {
    if width == 0 || height == 0 || width > MAX_TEXTURE_AXIS || height > MAX_TEXTURE_AXIS {
        return Err(CaptureFault::ResourceLimitExceeded);
    }
    let row_bytes = u64::from(width)
        .checked_mul(4)
        .ok_or(CaptureFault::ResourceLimitExceeded)?;
    let bytes = row_bytes
        .checked_mul(u64::from(height))
        .ok_or(CaptureFault::ResourceLimitExceeded)?;
    if bytes > MAX_SURFACE_BYTES {
        return Err(CaptureFault::ResourceLimitExceeded);
    }
    Ok(SurfaceLayout { row_bytes, bytes })
}

#[cfg(test)]
fn checked_bgra_bytes(width: u32, height: u32) -> Option<u64> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
}

#[derive(Debug)]
struct ByteBudget {
    limit: u64,
    used: AtomicU64,
}

impl ByteBudget {
    const fn new(limit: u64) -> Self {
        Self {
            limit,
            used: AtomicU64::new(0),
        }
    }

    fn try_reserve(self: &Arc<Self>, bytes: u64) -> Option<ByteLease> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .ok()
            .map(|_| ByteLease {
                budget: Arc::clone(self),
                bytes,
            })
    }

    #[cfg(test)]
    fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }
}

struct ByteLease {
    budget: Arc<ByteBudget>,
    bytes: u64,
}

impl fmt::Debug for ByteLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ByteLease")
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl Drop for ByteLease {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Session-local and process-global retained-byte accounting.
#[derive(Debug)]
pub(crate) struct SessionMemory {
    session: Arc<ByteBudget>,
    global: Arc<ByteBudget>,
}

impl SessionMemory {
    pub(crate) fn production() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<ByteBudget>> = OnceLock::new();
        Arc::new(Self {
            session: Arc::new(ByteBudget::new(SESSION_RETAINED_BYTES)),
            global: Arc::clone(
                GLOBAL.get_or_init(|| Arc::new(ByteBudget::new(GLOBAL_RETAINED_BYTES))),
            ),
        })
    }

    fn try_reserve(self: &Arc<Self>, bytes: u64) -> Option<RetainedBytes> {
        let session = self.session.try_reserve(bytes)?;
        let global = self.global.try_reserve(bytes)?;
        Some(RetainedBytes {
            _session: session,
            _global: global,
        })
    }

    pub(crate) fn reserve(self: &Arc<Self>, bytes: u64) -> Result<RetainedBytes> {
        self.try_reserve(bytes)
            .ok_or_else(|| CaptureFault::ResourceLimitExceeded.into())
    }

    #[cfg(test)]
    fn testing(session_limit: u64, global: Arc<ByteBudget>) -> Arc<Self> {
        Arc::new(Self {
            session: Arc::new(ByteBudget::new(session_limit)),
            global,
        })
    }

    #[cfg(test)]
    pub(crate) fn testing_isolated(session_limit: u64, global_limit: u64) -> Arc<Self> {
        Self::testing(session_limit, Arc::new(ByteBudget::new(global_limit)))
    }

    #[cfg(test)]
    pub(crate) fn testing_shared(
        session_limit: u64,
        global_limit: u64,
        session_count: usize,
    ) -> Vec<Arc<Self>> {
        let global = Arc::new(ByteBudget::new(global_limit));
        (0..session_count)
            .map(|_| Self::testing(session_limit, Arc::clone(&global)))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn usage(&self) -> (u64, u64) {
        (self.session.used(), self.global.used())
    }
}

/// Keeps both byte reservations alive for exactly the native/CPU owner lifetime.
#[derive(Debug)]
pub(crate) struct RetainedBytes {
    _session: ByteLease,
    _global: ByteLease,
}

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
        copied_bytes: u64,
        stream: u64,
    ) -> std::result::Result<Option<benchmark_metrics::CompletedCallbackCopy>, CaptureFault> {
        let _context = match self.context_gate.try_lock() {
            Ok(context) => context,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let timer = benchmark_metrics::time_callback_copy(copied_bytes, stream);
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
        Ok(Some(timer.finish()))
    }

    fn read_texture(
        &self,
        texture: &ID3D11Texture2D,
        descriptor: FrameDescriptor,
        memory: &Arc<SessionMemory>,
        operation: &OperationContext,
    ) -> Result<Arc<CpuPixels>> {
        let mut attempt = Operation::admit(operation)?;
        let _context = lock_with_operation(&self.context_gate, &mut attempt)?;

        let mut source_descriptor = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: source_descriptor is writable for the complete native struct.
        unsafe { texture.GetDesc(&raw mut source_descriptor) };
        let staging_layout = validate_surface(source_descriptor.Width, source_descriptor.Height)?;
        let cpu_bytes = u64::try_from(descriptor.byte_len())
            .map_err(|_| CaptureFault::ResourceLimitExceeded)?;
        let retained_bytes = mapping_retained_bytes(staging_layout.bytes(), cpu_bytes)?;
        // R1-2: both the staging texture and exact CPU output are admitted before
        // either allocation. Keeping the conservative combined lease with the
        // returned bytes also bounds mappings that outlive their source session.
        let mapping_bytes = memory.reserve(retained_bytes)?;
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
        let _staging_metric = benchmark_metrics::staging_texture_created();
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
        let native_row_bytes = usize::try_from(staging_layout.row_bytes)
            .map_err(|_| CaptureFault::ResourceLimitExceeded)?;
        if row_pitch < native_row_bytes
            || row_pitch < descriptor.row_bytes()
            || mapped.pData.is_null()
        {
            return Err(CaptureFault::InconsistentDescriptor.into());
        }
        let height = usize::try_from(descriptor.extent().height())
            .map_err(|_| CaptureFault::InconsistentDescriptor)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(descriptor.byte_len())
            .map_err(|_| CaptureFault::ResourceLimitExceeded)?;
        bytes.resize(descriptor.byte_len(), 0);
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
        let retainer: Arc<dyn Send + Sync> = Arc::new(mapping_bytes);
        Ok(attempt.commit(Arc::new(CpuPixels::with_retainer(
            bytes.into_boxed_slice(),
            retainer,
        )))?)
    }
}

fn mapping_retained_bytes(
    staging_bytes: u64,
    cpu_bytes: u64,
) -> std::result::Result<u64, CaptureFault> {
    staging_bytes
        .checked_add(cpu_bytes)
        .ok_or(CaptureFault::ResourceLimitExceeded)
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
    memory: Arc<SessionMemory>,
    state: Mutex<PoolState>,
    capacity: usize,
    deferred_discards: AtomicUsize,
}

#[derive(Debug, Default)]
struct PoolState {
    allocated: usize,
    leased: usize,
    shape: Option<TextureShape>,
    free: Vec<PooledTexture>,
}

struct PooledTexture {
    texture: ID3D11Texture2D,
    _bytes: RetainedBytes,
}

impl fmt::Debug for PooledTexture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PooledTexture")
    }
}

impl Drop for PooledTexture {
    fn drop(&mut self) {
        benchmark_metrics::record_detached_texture_destroyed();
    }
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
    pub(crate) fn new(
        domain: Arc<DeviceDomain>,
        memory: Arc<SessionMemory>,
        retained_storage_capacity: NonZeroU32,
    ) -> Arc<Self> {
        Arc::new(Self {
            domain,
            memory,
            state: Mutex::new(PoolState::default()),
            capacity: usize::try_from(retained_storage_capacity.get())
                .expect("u32 retained-storage capacity fits usize"),
            deferred_discards: AtomicUsize::new(0),
        })
    }

    /// Acquires without waiting. `None` is the finite-pressure outcome.
    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        descriptor: D3D11_TEXTURE2D_DESC,
    ) -> Result<Option<TextureLease>> {
        let layout = validate_surface(descriptor.Width, descriptor.Height)?;
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
        let pooled = if let Some(texture) = state.free.pop() {
            texture
        } else {
            if state.allocated >= self.capacity {
                return Ok(None);
            }
            let Some(bytes) = self.memory.try_reserve(layout.bytes()) else {
                return Ok(None);
            };
            let texture = self
                .domain
                .create_default_texture(descriptor)
                .map_err(classify_native_error)?;
            benchmark_metrics::record_detached_texture_created();
            state.allocated += 1;
            PooledTexture {
                texture,
                _bytes: bytes,
            }
        };
        state.leased += 1;
        Ok(Some(TextureLease {
            pooled: Some(pooled),
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

    fn release(&self, pooled: PooledTexture, shape: TextureShape) {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                // Discarding is always safe. Defer only the accounting so a
                // callback-side lease drop never waits for a consumer holding
                // the pool mutex.
                self.deferred_discards.fetch_add(1, Ordering::AcqRel);
                drop(pooled);
                return;
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        self.apply_deferred_discards(&mut state);
        state.leased = state.leased.saturating_sub(1);
        if state.shape == Some(shape) {
            state.free.push(pooled);
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
    pooled: Option<PooledTexture>,
    shape: TextureShape,
    pool: Weak<TexturePool>,
}

impl TextureLease {
    pub(crate) fn texture(&self) -> &ID3D11Texture2D {
        &self
            .pooled
            .as_ref()
            .expect("texture exists until lease drop")
            .texture
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
        if let (Some(pooled), Some(pool)) = (self.pooled.take(), self.pool.upgrade()) {
            pool.release(pooled, self.shape);
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
    memory: Arc<SessionMemory>,
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
        memory: Arc<SessionMemory>,
    ) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            domain,
            lease,
            mapping: Mutex::new(MappingState::default()),
            mapped: Condvar::new(),
            failure,
            device_terminal,
            memory,
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

        let mut result = self.domain.read_texture(
            self.lease.texture(),
            self.descriptor,
            &self.memory,
            operation,
        );
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
) -> std::result::Result<Option<FrameDescriptor>, CaptureFault> {
    if descriptor.Format != windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM {
        return Err(CaptureFault::UnsupportedFormat);
    }
    validate_surface(descriptor.Width, descriptor.Height)?;
    validate_surface(content_extent.width(), content_extent.height())?;
    if descriptor.Width < content_extent.width() || descriptor.Height < content_extent.height() {
        return Ok(None);
    }
    FrameDescriptor::packed(content_extent, PixelFormat::Bgra8).map(Some)
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
        ByteBudget, DETACHED_TEXTURE_BUDGET, DeviceDomain, DeviceTerminal, GLOBAL_RETAINED_BYTES,
        MAX_SURFACE_BYTES, MAX_TEXTURE_AXIS, MappingState, SESSION_RETAINED_BYTES, SessionMemory,
        TexturePool, checked_bgra_bytes, descriptor_from_native, finish_mapping_cache,
        mapping_retained_bytes, native_fault, retained_storage_capacity, validate_surface,
    };
    use mado_pilot_capture::{CaptureFault, CpuPixels};
    use mado_pilot_core::{PixelExtent, Status};

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
    fn a_resize_transition_surface_is_dropped_until_the_pool_catches_up() {
        let content = PixelExtent::new(480, 320);

        assert!(
            descriptor_from_native(&descriptor(342, 231), content)
                .expect("valid transitional surface")
                .is_none()
        );
        assert!(
            descriptor_from_native(&descriptor(480, 320), content)
                .expect("current surface")
                .is_some()
        );
    }

    #[test]
    fn r1_2_surface_axes_and_bytes_accept_the_exact_boundary_and_reject_one_over() {
        assert_eq!(
            validate_surface(8192, 4096)
                .expect("exact 128 MiB surface")
                .bytes(),
            MAX_SURFACE_BYTES
        );
        assert!(
            validate_surface(8192, 4097).is_err(),
            "one byte-pair row over"
        );
        assert!(
            validate_surface(MAX_TEXTURE_AXIS, 1).is_ok(),
            "exact D3D11 axis"
        );
        assert!(
            validate_surface(MAX_TEXTURE_AXIS + 1, 1).is_err(),
            "one over the D3D11 axis"
        );
    }

    #[test]
    fn r1_2_surface_and_mapping_multiplication_overflow_is_typed_before_allocation() {
        assert_eq!(checked_bgra_bytes(u32::MAX, u32::MAX), None);
        assert_eq!(
            mapping_retained_bytes(u64::MAX, 1),
            Err(CaptureFault::ResourceLimitExceeded)
        );
        assert_eq!(
            validate_surface(u32::MAX, u32::MAX),
            Err(CaptureFault::ResourceLimitExceeded)
        );
    }

    #[test]
    fn r1_2_production_limits_admit_the_exact_4k_retention_and_mapping_workload() {
        let layout = validate_surface(3840, 2160).expect("4K BGRA surface");
        assert_eq!(layout.bytes(), 33_177_600);
        let workload = layout
            .bytes()
            .checked_mul(34)
            .expect("two producers + 30 retained + staging + CPU output");
        assert_eq!(workload, 1_128_038_400);
        assert!(workload <= SESSION_RETAINED_BYTES);
        assert!(workload.checked_mul(2).expect("two sessions") <= GLOBAL_RETAINED_BYTES);

        let global = Arc::new(ByteBudget::new(GLOBAL_RETAINED_BYTES));
        let memory = SessionMemory::testing(SESSION_RETAINED_BYTES, global);
        let held = memory
            .reserve(workload)
            .expect("production workload admitted");
        drop(held);
    }

    #[test]
    fn r1_2_reported_capacity_is_truthful_for_4k_and_reduced_for_8k() {
        let four_k = retained_storage_capacity(validate_surface(3840, 2160).expect("4K"))
            .expect("4K retained capacity");
        let eight_k = retained_storage_capacity(validate_surface(7680, 4320).expect("8K"))
            .expect("8K retained capacity");

        assert_eq!(four_k, DETACHED_TEXTURE_BUDGET);
        assert_eq!(eight_k.get(), 12);
        assert!(eight_k < four_k);
    }

    #[test]
    fn r1_2_production_session_and_global_limits_are_exact_and_shared() {
        let global = Arc::new(ByteBudget::new(GLOBAL_RETAINED_BYTES));
        let first = SessionMemory::testing(SESSION_RETAINED_BYTES, Arc::clone(&global));
        let second = SessionMemory::testing(SESSION_RETAINED_BYTES, Arc::clone(&global));
        let third = SessionMemory::testing(SESSION_RETAINED_BYTES, Arc::clone(&global));

        let first_held = first
            .reserve(SESSION_RETAINED_BYTES)
            .expect("exact session ceiling");
        assert_eq!(
            first.reserve(1).expect_err("one over session").status(),
            Status::LimitExceeded
        );
        let second_held = second
            .reserve(SESSION_RETAINED_BYTES)
            .expect("second session reaches exact global ceiling");
        assert_eq!(global.used(), GLOBAL_RETAINED_BYTES);
        assert_eq!(
            third.reserve(1).expect_err("one over global").status(),
            Status::LimitExceeded
        );

        drop(first_held);
        let resumed = third.reserve(1).expect("lease release restores admission");
        drop(resumed);
        drop(second_held);
        assert_eq!(global.used(), 0);

        assert_eq!(
            third
                .reserve(SESSION_RETAINED_BYTES + 1)
                .expect_err("single request over production session ceiling")
                .status(),
            Status::LimitExceeded
        );
    }

    #[test]
    fn r1_2_mapping_budget_follows_pixels_that_outlive_the_session_owner() {
        let global = Arc::new(ByteBudget::new(256));
        let memory = SessionMemory::testing(256, Arc::clone(&global));
        let retained = memory.reserve(128).expect("mapping budget");
        let retainer: Arc<dyn Send + Sync> = Arc::new(retained);
        let pixels = Arc::new(CpuPixels::with_retainer(
            vec![0; 64].into_boxed_slice(),
            retainer,
        ));
        drop(memory);

        assert_eq!(global.used(), 128, "mapped bytes retain their accounting");
        drop(pixels);
        assert_eq!(global.used(), 0, "final mapping release returns the budget");
    }

    #[test]
    fn r1_2_concurrent_sessions_share_one_finite_global_byte_budget() {
        let global = Arc::new(ByteBudget::new(200));
        let first = SessionMemory::testing(150, Arc::clone(&global));
        let second = SessionMemory::testing(150, Arc::clone(&global));
        let held = first.reserve(120).expect("first session admitted");

        assert_eq!(
            first.reserve(40).expect_err("session ceiling").status(),
            Status::LimitExceeded
        );
        assert_eq!(
            second.reserve(90).expect_err("global ceiling").status(),
            Status::LimitExceeded
        );
        drop(held);
        let resumed = second.reserve(90).expect("release admits another session");
        assert_eq!(global.used(), 90);
        drop(resumed);
        assert_eq!(global.used(), 0);
    }

    #[test]
    fn r1_2_derived_4k_capacity_admits_first_and_fortieth_then_refuses_forty_one() {
        let domain = DeviceDomain::create().expect("D3D11 device");
        let retained_storage_capacity =
            retained_storage_capacity(validate_surface(3840, 2160).expect("4K"))
                .expect("4K retained-storage capacity");
        let pool = TexturePool::new(
            domain,
            SessionMemory::production(),
            retained_storage_capacity,
        );
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
    fn r1_2_derived_8k_capacity_admits_the_first_and_twelfth_then_refuses_thirteen() {
        let domain = DeviceDomain::create().expect("D3D11 device");
        let retained_storage_capacity =
            retained_storage_capacity(validate_surface(7680, 4320).expect("8K"))
                .expect("8K retained-storage capacity");
        assert_eq!(retained_storage_capacity.get(), 12);
        let pool = TexturePool::new(
            domain,
            SessionMemory::testing_isolated(1024, 1024),
            retained_storage_capacity,
        );
        let mut leases = Vec::new();
        for index in 0..retained_storage_capacity.get() {
            leases.push(
                pool.try_acquire(descriptor(4, 4))
                    .expect("bounded acquire")
                    .unwrap_or_else(|| panic!("derived allocation {index} must be admitted")),
            );
        }
        assert_eq!(leases.len(), 12);
        assert!(
            pool.try_acquire(descriptor(4, 4))
                .expect("bounded refusal")
                .is_none(),
            "the thirteenth allocation exceeds the advertised 8K count"
        );

        drop(leases.remove(0));
        let resumed = pool
            .try_acquire(descriptor(4, 4))
            .expect("acquire after release")
            .expect("the released derived slot resumes allocation");
        drop(resumed);
        drop(leases);
    }

    #[test]
    fn resize_retires_only_unleased_old_generation_textures() {
        let domain = DeviceDomain::create().expect("D3D11 device");
        let global = Arc::new(ByteBudget::new(1024));
        let memory = SessionMemory::testing(1024, Arc::clone(&global));
        let pool = TexturePool::new(Arc::clone(&domain), memory, DETACHED_TEXTURE_BUDGET);
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
        drop(pool);
        assert_eq!(
            global.used(),
            0,
            "resize generations release all byte leases"
        );
    }

    #[test]
    fn lease_release_never_waits_for_the_pool_mutex() {
        let domain = DeviceDomain::create().expect("D3D11 device");
        let pool = TexturePool::new(domain, SessionMemory::production(), DETACHED_TEXTURE_BUDGET);
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
