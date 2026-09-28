//! Public Windows capture provider and its native-identity registry.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, NativeWindowDescription, NativeWindowId,
    OpenRequest, PixelFormat, ResolvedCapturePacing, TargetDescription, WindowGeometry,
};
use mado_pilot_core::{IdentityIssuer, Operation, OperationContext, ProviderId, Result, TargetId};
use mado_pilot_input::{
    InputController, InputDescriptor, InputFault, InputOpenRequest, InputProvider,
};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::GraphicsCaptureItem;
#[cfg(feature = "benchmark-instrumentation")]
use windows::Win32::Foundation::HWND;
#[cfg(feature = "benchmark-instrumentation")]
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::core::IInspectable;

use crate::availability::{ensure_capture_available, ensure_winrt_apartment};
use crate::discovery::{
    Candidate, CaptureItem, NativeKey, TargetMetadata, current_window_geometry, inventory,
};
use crate::input::{GeometryLedger, WindowsInputController};
use crate::native::{NativeSession, NativeSessionSource, native_target_fault};
use crate::pacing::interval_ticks;
use crate::storage::validate_surface;
use crate::window_authority::{RetainedWindowAuthority, WindowAuthorityStatus};

/// Provider name qualifying every native Windows target identity.
pub const PROVIDER: ProviderId = ProviderId::new("windows");

/// Picker-free Windows target discovery and WGC capture.
///
/// Construction touches no native API. Discovery and open perform the runtime
/// availability checks, which lets an application include this package without
/// making an unresolved minimum-Windows claim.
pub struct WindowsCaptureProvider {
    issuer: Arc<IdentityIssuer>,
    capture_pacing: ResolvedCapturePacing,
    discovery_gate: Mutex<()>,
    registry: Mutex<Registry>,
}

const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Current and immediately previous discovery selections remain openable.
const RETAINED_DISCOVERY_GENERATIONS: usize = 2;

#[derive(Debug, Default, Clone)]
struct Registry {
    records: HashMap<TargetId, Arc<TargetRecord>>,
    generations: VecDeque<Vec<TargetId>>,
}

pub(crate) struct TargetRecord {
    id: TargetId,
    key: NativeKey,
    metadata: TargetMetadata,
    item: CaptureItem,
    lost: Arc<AtomicBool>,
    geometry: Arc<GeometryLedger>,
    authority: Option<RetainedWindowAuthority>,
    closed_token: Option<i64>,
}

struct PreparedSnapshot {
    registry: Registry,
    descriptions: Vec<TargetDescription>,
}

impl WindowsCaptureProvider {
    /// Creates a provider using identities from `issuer`.
    #[must_use]
    pub fn new(issuer: Arc<IdentityIssuer>) -> Self {
        Self {
            issuer,
            capture_pacing: ResolvedCapturePacing::source_default(),
            discovery_gate: Mutex::new(()),
            registry: Mutex::new(Registry::default()),
        }
    }

    /// Creates a provider with an immutable, already-resolved capture default.
    ///
    /// Only the selected interval's representation is checked. Construction
    /// performs no discovery, capture allocation, or native capability probe.
    pub fn with_capture_pacing(
        issuer: Arc<IdentityIssuer>,
        pacing: ResolvedCapturePacing,
    ) -> Result<Self> {
        Self::validate_capture_pacing(pacing)?;
        Ok(Self {
            capture_pacing: pacing,
            ..Self::new(issuer)
        })
    }

    /// Checks selected Windows interval representability without native work.
    ///
    /// Both required and preferred values must fit positive signed 64-bit
    /// 100-nanosecond ticks after rounding upward.
    pub fn validate_capture_pacing(pacing: ResolvedCapturePacing) -> Result<()> {
        if let Some(interval) = pacing.interval() {
            interval_ticks(interval)?;
        }
        Ok(())
    }

    fn session_pacing(&self, request: &OpenRequest) -> Result<ResolvedCapturePacing> {
        let pacing = request.capture_pacing().resolve(self.capture_pacing);
        Self::validate_capture_pacing(pacing)?;
        Ok(pacing)
    }

    #[cfg(feature = "benchmark-instrumentation")]
    pub(crate) fn fixture_target_has_process(&self, target: TargetId, process_id: u32) -> bool {
        let registry = self.registry();
        let Some(record) = registry.records.get(&target) else {
            return false;
        };
        let NativeKey::Window(raw) = record.key else {
            return false;
        };
        let window = HWND(std::ptr::with_exposed_provenance_mut::<core::ffi::c_void>(
            raw,
        ));
        let mut observed_process = 0;
        // SAFETY: `window` is reconstructed only as the opaque HWND captured by
        // this provider's retained discovery record. USER32 validates it and
        // writes to the live local `observed_process` scalar.
        unsafe { GetWindowThreadProcessId(window, Some(&raw mut observed_process)) };
        observed_process != 0 && observed_process == process_id
    }

    fn discover_with<F>(
        &self,
        operation: &OperationContext,
        inventory: F,
    ) -> Result<Vec<TargetDescription>>
    where
        F: FnOnce() -> Result<Vec<Candidate>>,
    {
        let mut attempt = Operation::admit(operation)?;
        let _discovery = lock_with_operation(&self.discovery_gate, &mut attempt)?;
        let candidates = match inventory() {
            Ok(candidates) => candidates,
            Err(error) => {
                attempt.checkpoint()?;
                return Err(error);
            }
        };
        attempt.checkpoint()?;
        let prepared = match self.prepare_snapshot(candidates) {
            Ok(prepared) => prepared,
            Err(error) => {
                attempt.checkpoint()?;
                return Err(error);
            }
        };

        // R1-1: final arbitration precedes every externally visible registry
        // mutation. Staging may consume issuer values and install handlers only
        // on unpublished records; a losing operation cannot add, retire,
        // reorder, relabel, or mark lost any live selection.
        let prepared = attempt.commit(prepared)?;
        Ok(self.commit_snapshot(prepared))
    }

    fn prepare_snapshot(&self, candidates: Vec<Candidate>) -> Result<PreparedSnapshot> {
        let mut records = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            records.push(self.create_record(candidate)?);
        }
        let descriptions = records
            .iter()
            .map(|record| record.capture_description())
            .collect();
        let generation = records.iter().map(|record| record.id).collect();
        let mut registry = self.registry().clone();
        for record in records {
            registry.records.insert(record.id, record);
        }
        registry.generations.push_back(generation);
        while registry.generations.len() > RETAINED_DISCOVERY_GENERATIONS {
            if let Some(expired) = registry.generations.pop_front() {
                for id in expired {
                    registry.records.remove(&id);
                }
            }
        }
        Ok(PreparedSnapshot {
            registry,
            descriptions,
        })
    }

    fn commit_snapshot(&self, prepared: PreparedSnapshot) -> Vec<TargetDescription> {
        let PreparedSnapshot {
            registry,
            descriptions,
        } = prepared;
        *self.registry() = registry;
        descriptions
    }

    fn create_record(&self, candidate: Candidate) -> Result<Arc<TargetRecord>> {
        let id = self.issuer.issue_target(PROVIDER)?;
        let lost = Arc::new(AtomicBool::new(false));
        let closed_lost = Arc::clone(&lost);
        let closed_handler =
            TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                closed_lost.store(true, Ordering::Release);
                Ok(())
            });
        let closed_token = match &candidate.item {
            CaptureItem::Native(item) => Some(
                item.Closed(&closed_handler)
                    .map_err(|error| native_target_fault(error, candidate.key.kind()))?,
            ),
            #[cfg(test)]
            CaptureItem::Synthetic(_) => None,
        };
        Ok(Arc::new(TargetRecord {
            id,
            key: candidate.key,
            metadata: candidate.metadata,
            item: candidate.item,
            lost,
            geometry: Arc::new(GeometryLedger::default()),
            authority: candidate.authority,
            closed_token,
        }))
    }

    fn select_record(&self, target: TargetId) -> Result<Arc<TargetRecord>> {
        let record = self
            .registry()
            .records
            .get(&target)
            .cloned()
            .ok_or(CaptureFault::TargetLost)?;
        if record.ensure_live().is_err() {
            return Err(CaptureFault::TargetLost.into());
        }
        Ok(record)
    }

    fn select_input_record(
        &self,
        target: TargetId,
    ) -> std::result::Result<Arc<TargetRecord>, InputFault> {
        let record = self
            .registry()
            .records
            .get(&target)
            .cloned()
            .ok_or(InputFault::TargetLost)?;
        record.ensure_live()?;
        Ok(record)
    }

    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for WindowsCaptureProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let registry = self.registry();
        formatter
            .debug_struct("WindowsCaptureProvider")
            .field("engine", &self.issuer.engine())
            .field("capture_pacing", &self.capture_pacing)
            .field("known_targets", &registry.records.len())
            .field("retained_generations", &registry.generations.len())
            .finish()
    }
}

impl CaptureProvider for WindowsCaptureProvider {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        ensure_capture_available()?;
        self.discover_with(operation, inventory)
    }

    fn describe_window(
        &self,
        target: TargetId,
        operation: &OperationContext,
    ) -> Result<TargetDescription> {
        let mut attempt = Operation::admit(operation)?;
        CaptureProvider::accepts_target(self, target, self.issuer.engine())?;
        let record = self
            .registry()
            .records
            .get(&target)
            .cloned()
            .ok_or(CaptureFault::TargetLost)?;
        if matches!(&record.item, CaptureItem::Native(_)) {
            ensure_winrt_apartment()?;
        }
        let observed = record.observe_window(None);
        attempt.checkpoint()?;
        let window = observed?;
        let description = record
            .description_at_extent(window.geometry().extent(), true)
            .with_window(window);
        Ok(attempt.commit(description)?)
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        let mut attempt = Operation::admit(operation)?;
        CaptureProvider::accepts_target(self, target, self.issuer.engine())?;
        let pacing = self.session_pacing(request)?;
        ensure_capture_available()?;
        if let Some(required) = request.required_format()
            && required != PixelFormat::Bgra8
        {
            return Err(CaptureFault::UnsupportedOption.into());
        }

        // accepts_target established this engine and provider. TargetId is
        // opaque, so an accepted identity absent from the live registry is
        // conservatively stale rather than an invitation to retain history.
        let record = self.select_record(target)?;
        let mut metadata = record.metadata.clone();
        if let Some(required) = request.window_geometry() {
            let observed = record.required_geometry(required, None);
            attempt.checkpoint()?;
            let geometry = observed?;
            metadata.extent = geometry.extent();
            metadata.placement = geometry.placement();
        }
        let item = match &record.item {
            CaptureItem::Native(item) => item.clone(),
            #[cfg(test)]
            CaptureItem::Synthetic(_) => return Err(CaptureFault::SourceInvalid.into()),
        };
        let stream = self.issuer.issue_stream()?;
        let session = NativeSession::open(
            NativeSessionSource::new(
                target,
                stream,
                record.key.kind(),
                record.key,
                metadata,
                item,
                Arc::clone(&record.geometry),
            )
            .with_requirements(&record, request),
            pacing,
            &mut attempt,
        )?;
        Ok(attempt.commit(session as Arc<dyn CaptureSession>)?)
    }
}

impl InputProvider for WindowsCaptureProvider {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn permission(&self) -> Option<mado_pilot_core::PermissionKind> {
        None
    }

    fn describe(&self, target: TargetId, operation: &OperationContext) -> Result<InputDescriptor> {
        let attempt = Operation::admit(operation)?;
        InputProvider::accepts_target(self, target, self.issuer.engine())?;
        let record = self.select_input_record(target)?;
        Ok(attempt.commit(record.input_descriptor())?)
    }

    fn open(
        &self,
        target: TargetId,
        request: &InputOpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn InputController>> {
        let attempt = Operation::admit(operation)?;
        InputProvider::accepts_target(self, target, self.issuer.engine())?;
        let record = self.select_input_record(target)?;
        let descriptor = record.input_descriptor();
        request.check(descriptor.capability())?;
        let controller = WindowsInputController::new(record, descriptor);
        Ok(attempt.commit(controller as Arc<dyn InputController>)?)
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
                thread::sleep(DISCOVERY_POLL_INTERVAL);
                operation.checkpoint()?;
            }
        }
    }
}

impl TargetRecord {
    fn description(&self) -> TargetDescription {
        let authority_is_current =
            !self.lost.load(Ordering::Acquire) && self.window_message_authority_is_current();
        self.description_at_extent(self.metadata.extent, authority_is_current)
    }

    fn description_at_extent(
        &self,
        extent: mado_pilot_core::PixelExtent,
        authority_is_current: bool,
    ) -> TargetDescription {
        let description =
            self.metadata
                .describe(self.id, self.key.kind(), authority_is_current, extent);
        if authority_is_current
            && let Some(identity) = self
                .authority
                .as_ref()
                .and_then(RetainedWindowAuthority::process_identity)
        {
            description.with_process_identity(identity.clone())
        } else {
            description
        }
    }

    fn capture_description(&self) -> TargetDescription {
        match self.observe_window(None) {
            Ok(window) => self
                .description_at_extent(window.geometry().extent(), true)
                .with_window(window),
            Err(_) => self.description(),
        }
    }

    fn retained_window_status(&self) -> std::result::Result<(), CaptureFault> {
        if !matches!(self.key, NativeKey::Window(_)) {
            return Err(CaptureFault::UnsupportedOption);
        }
        retained_window_authority(
            self.lost.load(Ordering::Acquire),
            self.closed_token.is_some(),
            self.authority.as_ref().map(RetainedWindowAuthority::status),
        )
    }

    fn observe_window(
        &self,
        required: Option<WindowGeometry>,
    ) -> std::result::Result<NativeWindowDescription, CaptureFault> {
        self.retained_window_status()?;
        let item = match &self.item {
            CaptureItem::Native(item) => item,
            #[cfg(test)]
            CaptureItem::Synthetic(_) => return Err(CaptureFault::UnsupportedOption),
        };
        let size = item.Size().map_err(|_| CaptureFault::TargetLost)?;
        let width = u32::try_from(size.Width)
            .ok()
            .filter(|value| *value > 0)
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        let height = u32::try_from(size.Height)
            .ok()
            .filter(|value| *value > 0)
            .ok_or(CaptureFault::InconsistentDescriptor)?;
        let extent = mado_pilot_core::PixelExtent::new(width, height);
        if required.is_some_and(|required| required.extent() != extent) {
            return Err(CaptureFault::WindowGeometryChanged);
        }
        let geometry = current_window_geometry(self.key, extent)?;
        // Bracket geometry with the original item's Closed state and retained
        // process creation-time authority, never a newly created capture item.
        let after = item.Size().map_err(|_| CaptureFault::TargetLost)?;
        self.retained_window_status()?;
        if after.Width != size.Width || after.Height != size.Height {
            return Err(CaptureFault::WindowGeometryChanged);
        }
        let NativeKey::Window(raw) = self.key else {
            return Err(CaptureFault::UnsupportedOption);
        };
        let id = u64::try_from(raw)
            .ok()
            .and_then(NonZeroU64::new)
            .ok_or(CaptureFault::SourceInvalid)?;
        Ok(NativeWindowDescription::new(
            NativeWindowId::Windows(id),
            geometry,
        ))
    }

    pub(crate) fn required_geometry(
        &self,
        required: WindowGeometry,
        frame_extent: Option<mado_pilot_core::PixelExtent>,
    ) -> std::result::Result<WindowGeometry, CaptureFault> {
        let observed = self.observe_window(Some(required))?.geometry();
        require_geometry(required, observed, frame_extent)?;
        Ok(observed)
    }

    fn window_message_authority_is_current(&self) -> bool {
        self.authority
            .as_ref()
            .is_some_and(|authority| authority.status() == WindowAuthorityStatus::SameTarget)
    }

    pub(crate) fn target(&self) -> TargetId {
        self.id
    }

    pub(crate) fn key(&self) -> NativeKey {
        self.key
    }

    pub(crate) fn kind(&self) -> mado_pilot_core::TargetKind {
        self.key.kind()
    }

    pub(crate) fn class_name(&self) -> Option<&str> {
        self.metadata.class_name.as_deref()
    }

    pub(crate) fn geometry(&self) -> &Arc<GeometryLedger> {
        &self.geometry
    }
    pub(crate) fn authority(&self) -> Option<&RetainedWindowAuthority> {
        self.authority.as_ref()
    }

    pub(crate) fn input_descriptor(&self) -> InputDescriptor {
        InputDescriptor::new(self.id, self.description().capability().input())
    }

    pub(crate) fn current_extent(
        &self,
    ) -> std::result::Result<mado_pilot_core::PixelExtent, InputFault> {
        match &self.item {
            CaptureItem::Native(item) => {
                let size = item.Size().map_err(|_| InputFault::TargetLost)?;
                let width = u32::try_from(size.Width)
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or(InputFault::TargetLost)?;
                let height = u32::try_from(size.Height)
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or(InputFault::TargetLost)?;
                validate_surface(width, height).map_err(|_| InputFault::RouteUnavailable)?;
                Ok(mado_pilot_core::PixelExtent::new(width, height))
            }
            #[cfg(test)]
            CaptureItem::Synthetic(_) => Ok(self.metadata.extent),
        }
    }

    pub(crate) fn ensure_live(&self) -> std::result::Result<(), InputFault> {
        let (present, usable) = match &self.item {
            CaptureItem::Native(item) => (self.key.is_present(), item.Size().is_ok()),
            #[cfg(test)]
            CaptureItem::Synthetic(_) => (true, true),
        };
        // Native-key presence may prove loss, but never proves identity: a
        // recycled key remains present and input still consults this retained
        // GraphicsCaptureItem's authoritative Closed state.
        if self.lost.load(Ordering::Acquire) || !present || !usable {
            self.lost.store(true, Ordering::Release);
            Err(InputFault::TargetLost)
        } else {
            Ok(())
        }
    }
}

fn retained_window_authority(
    closed: bool,
    closed_registered: bool,
    authority: Option<WindowAuthorityStatus>,
) -> std::result::Result<(), CaptureFault> {
    if closed {
        return Err(CaptureFault::TargetLost);
    }
    match (closed_registered, authority) {
        (true, Some(WindowAuthorityStatus::SameTarget)) => Ok(()),
        (
            _,
            Some(
                WindowAuthorityStatus::TargetLost
                | WindowAuthorityStatus::ReplacementOrReuse
                | WindowAuthorityStatus::RelationshipChanged,
            ),
        ) => Err(CaptureFault::TargetLost),
        _ => Err(CaptureFault::UnsupportedOption),
    }
}

pub(crate) fn require_geometry(
    required: WindowGeometry,
    observed: WindowGeometry,
    frame_extent: Option<mado_pilot_core::PixelExtent>,
) -> std::result::Result<(), CaptureFault> {
    if required != observed || frame_extent.is_some_and(|extent| extent != required.extent()) {
        Err(CaptureFault::WindowGeometryChanged)
    } else {
        Ok(())
    }
}

impl Drop for TargetRecord {
    fn drop(&mut self) {
        if let (CaptureItem::Native(item), Some(token)) = (&self.item, self.closed_token.take()) {
            let _removed = item.RemoveClosed(token);
        }
    }
}

impl fmt::Debug for TargetRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TargetRecord")
            .field("id", &self.id)
            .field("kind", &self.key.kind())
            .field("lost", &self.lost.load(Ordering::Acquire))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use mado_pilot_capture::{
        CapturePacingRequest, CaptureProvider, OpenRequest, PixelFormat, ResolvedCapturePacing,
    };
    use mado_pilot_core::{
        CancellationToken, Clock, IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent,
        Scale, Status, TargetPlacement,
    };

    use crate::WindowsConfig;
    use crate::discovery::{Candidate, CaptureItem, NativeKey, TargetMetadata};

    use super::{RETAINED_DISCOVERY_GENERATIONS, WindowsCaptureProvider};

    fn window_candidate(raw: usize, incarnation: u64, name: &str) -> Candidate {
        let extent = PixelExtent::new(64, 48);
        Candidate {
            key: NativeKey::Window(raw),
            metadata: TargetMetadata {
                name: name.to_owned(),
                class_name: None,
                extent,
                placement: TargetPlacement::new(
                    (0.0, 0.0),
                    (64.0, 48.0),
                    Scale::new(1.0, 1.0).expect("scale"),
                )
                .expect("placement"),
            },
            item: CaptureItem::Synthetic(incarnation),
            authority: None,
        }
    }

    fn commit_candidates(
        provider: &WindowsCaptureProvider,
        candidates: Vec<Candidate>,
    ) -> Vec<mado_pilot_capture::TargetDescription> {
        let prepared = provider
            .prepare_snapshot(candidates)
            .expect("snapshot stages");
        provider.commit_snapshot(prepared)
    }

    /// Expires exactly when `Operation::commit` performs final arbitration.
    #[derive(Debug, Default)]
    struct CommitDeadlineClock {
        reads: AtomicUsize,
    }

    impl Clock for CommitDeadlineClock {
        fn now(&self) -> MonotonicInstant {
            let read = self.reads.fetch_add(1, Ordering::AcqRel) + 1;
            if read >= 3 {
                MonotonicInstant::from_origin(Duration::from_millis(1))
            } else {
                MonotonicInstant::ORIGIN
            }
        }
    }

    #[test]
    fn windows_configuration_validates_only_the_selected_layer() {
        let source = ResolvedCapturePacing::source_default();
        let common = CapturePacingRequest::preferred(Duration::MAX)
            .expect("positive common request")
            .resolve(source);
        let selected = CapturePacingRequest::required(Duration::from_millis(200))
            .expect("positive Windows request");
        let config = WindowsConfig::new().with_capture_pacing(selected);
        let provider = WindowsCaptureProvider::with_capture_pacing(
            Arc::new(IdentityIssuer::new()),
            config.capture_pacing().resolve(common),
        )
        .expect("overridden common overflow is irrelevant");
        assert_eq!(
            provider
                .session_pacing(&OpenRequest::new())
                .expect("inherit"),
            selected.resolve(source)
        );

        const RESET: WindowsConfig =
            WindowsConfig::new().with_capture_pacing(CapturePacingRequest::source_default());
        let reset = WindowsCaptureProvider::with_capture_pacing(
            Arc::new(IdentityIssuer::new()),
            RESET.capture_pacing().resolve(common),
        )
        .expect("source default bypasses an unrepresentable common interval");
        assert_eq!(
            reset
                .session_pacing(&OpenRequest::new())
                .expect("reset session"),
            source
        );
        let error = WindowsCaptureProvider::with_capture_pacing(
            Arc::new(IdentityIssuer::new()),
            WindowsConfig::new().capture_pacing().resolve(common),
        )
        .expect_err("replacing the OS block with inheritance exposes common overflow");
        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn provider_defaults_and_session_overrides_remain_independent() {
        let source = ResolvedCapturePacing::source_default();
        let required = CapturePacingRequest::required(Duration::from_millis(200))
            .expect("required")
            .resolve(source);
        let preferred = CapturePacingRequest::preferred(Duration::from_millis(70))
            .expect("preferred")
            .resolve(source);
        let first =
            WindowsCaptureProvider::with_capture_pacing(Arc::new(IdentityIssuer::new()), required)
                .expect("first provider");
        let second =
            WindowsCaptureProvider::with_capture_pacing(Arc::new(IdentityIssuer::new()), preferred)
                .expect("second provider");
        let override_request =
            CapturePacingRequest::preferred(Duration::from_millis(60)).expect("session preference");
        let request = OpenRequest::new().with_capture_pacing(override_request);
        assert_eq!(
            first.session_pacing(&request).expect("session override"),
            override_request.resolve(source)
        );
        assert_eq!(
            first
                .session_pacing(&request.with_capture_pacing(CapturePacingRequest::inherit()))
                .expect("cleared override"),
            required
        );
        assert_eq!(
            first
                .session_pacing(
                    &OpenRequest::new().with_capture_pacing(CapturePacingRequest::source_default())
                )
                .expect("explicit reset"),
            source
        );
        assert_eq!(
            first
                .session_pacing(&OpenRequest::new())
                .expect("first unchanged"),
            required
        );
        assert_eq!(
            second
                .session_pacing(&OpenRequest::new())
                .expect("second unchanged"),
            preferred
        );
        let legacy = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        assert_eq!(
            legacy
                .session_pacing(&OpenRequest::new())
                .expect("legacy open default"),
            source
        );
    }

    #[test]
    fn selected_session_overflow_fails_before_native_work_but_after_interruption_admission() {
        let issuer = Arc::new(IdentityIssuer::new());
        let target = issuer
            .issue_target(super::PROVIDER)
            .expect("owned identity");
        let provider = WindowsCaptureProvider::new(issuer);
        let request = OpenRequest::new().with_capture_pacing(
            CapturePacingRequest::preferred(Duration::MAX).expect("positive preference"),
        );
        let error = provider
            .open(target, &request, &OperationContext::new())
            .expect_err("overflow fails without a native availability probe or target allocation");
        assert_eq!(error.status(), Status::InvalidArgument);

        let token = CancellationToken::new();
        token.cancel();
        let error = provider
            .open(
                target,
                &request,
                &OperationContext::new().with_cancellation(token),
            )
            .expect_err("existing interruption admission wins");
        assert_eq!(error.status(), Status::Cancelled);
    }

    #[test]
    fn a_foreign_identity_is_rejected_before_native_open() {
        let own = Arc::new(IdentityIssuer::new());
        let foreign = IdentityIssuer::new()
            .issue_target(super::PROVIDER)
            .expect("issued");
        let provider = WindowsCaptureProvider::new(own);
        let error = provider
            .open(
                foreign,
                &OpenRequest::new().require_format(PixelFormat::Bgra8),
                &OperationContext::new(),
            )
            .expect_err("foreign");
        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn an_absent_identity_accepted_by_this_provider_is_conservatively_lost() {
        let issuer = Arc::new(IdentityIssuer::new());
        let absent = issuer
            .issue_target(super::PROVIDER)
            .expect("issued by this engine for this provider");
        let provider = WindowsCaptureProvider::new(issuer);
        let error = provider
            .open(
                absent,
                &OpenRequest::new().require_format(PixelFormat::Bgra8),
                &OperationContext::new(),
            )
            .expect_err("an absent accepted identity is not live");
        if error.status() == Status::Unsupported {
            return;
        }
        assert_eq!(error.status(), Status::TargetLost);
    }

    #[test]
    fn discovery_order_is_deterministic_and_each_snapshot_has_fresh_ids() {
        let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let first = match provider.discover(&OperationContext::new()) {
            Ok(first) => first,
            Err(error) if error.status() == Status::Unsupported => return,
            Err(error) => panic!("discovery failed on a supported host: {error}"),
        };
        let second = provider
            .discover(&OperationContext::new())
            .expect("availability cannot disappear during one test");
        let first_ids: Vec<_> = first.iter().map(|target| target.id()).collect();
        let second_ids: Vec<_> = second.iter().map(|target| target.id()).collect();
        assert!(
            first_ids.iter().all(|id| !second_ids.contains(id)),
            "R1-4: retained native selections, not recycled raw keys, own identity"
        );
        assert!(
            first
                .iter()
                .all(|target| target.provider() == super::PROVIDER)
        );
    }

    #[test]
    fn discovery_snapshots_commit_in_query_order() {
        let provider = Arc::new(WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new())));
        let (first_entered_tx, first_entered_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first = {
            let provider = Arc::clone(&provider);
            thread::spawn(move || {
                provider.discover_with(&OperationContext::new(), || {
                    first_entered_tx.send(()).expect("signal first inventory");
                    release_first_rx.recv().expect("release first inventory");
                    Ok(Vec::new())
                })
            })
        };
        first_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first inventory entered");

        let (second_entered_tx, second_entered_rx) = mpsc::channel();
        let second = {
            let provider = Arc::clone(&provider);
            thread::spawn(move || {
                provider.discover_with(&OperationContext::new(), || {
                    second_entered_tx.send(()).expect("signal second inventory");
                    Ok(Vec::new())
                })
            })
        };
        assert!(
            second_entered_rx
                .recv_timeout(Duration::from_millis(40))
                .is_err(),
            "the newer inventory waits until the older inventory commits"
        );

        release_first_tx.send(()).expect("release first");
        first
            .join()
            .expect("first thread")
            .expect("first discovery");
        second_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second inventory enters after the first commit");
        second
            .join()
            .expect("second thread")
            .expect("second discovery");
    }

    #[test]
    fn r1_1_deadline_at_final_arbitration_leaves_all_registry_state_unchanged() {
        let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let first = commit_candidates(&provider, vec![window_candidate(7, 1, "first")])[0].id();
        let second = commit_candidates(&provider, vec![window_candidate(7, 2, "second")])[0].id();
        let before = {
            let registry = provider.registry();
            let records = registry
                .records
                .iter()
                .map(|(id, record)| {
                    let incarnation = match record.item {
                        CaptureItem::Synthetic(incarnation) => incarnation,
                        CaptureItem::Native(_) => unreachable!("controlled record"),
                    };
                    (
                        *id,
                        record.key,
                        record.metadata.clone(),
                        record.lost.load(Ordering::Acquire),
                        incarnation,
                    )
                })
                .collect::<Vec<_>>();
            (registry.generations.clone(), records)
        };

        let clock = Arc::new(CommitDeadlineClock::default());
        let operation = OperationContext::new()
            .with_clock(clock)
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(1)));
        let error = provider
            .discover_with(&operation, || Ok(vec![window_candidate(7, 3, "losing")]))
            .expect_err("R1-1: deadline wins after staging and before registry commit");

        assert_eq!(error.status(), Status::DeadlineExceeded);
        let registry = provider.registry();
        assert_eq!(registry.generations, before.0, "generation order unchanged");
        assert_eq!(
            registry.records.len(),
            before.1.len(),
            "membership unchanged"
        );
        for (id, key, metadata, lost, incarnation) in before.1 {
            let record = registry.records.get(&id).expect("same ID remains present");
            assert_eq!(record.key, key);
            assert_eq!(record.metadata, metadata);
            assert_eq!(record.lost.load(Ordering::Acquire), lost);
            assert!(matches!(record.item, CaptureItem::Synthetic(value) if value == incarnation));
        }
        assert!(registry.records.contains_key(&first));
        assert!(registry.records.contains_key(&second));
    }

    #[test]
    fn r1_4_identical_raw_identity_with_a_fresh_lifetime_never_retargets_the_old_id() {
        // Both candidates model the identical HWND/PID/TID/class path. Their
        // distinct retained selection lives are the only incarnation fact.
        let provider = WindowsCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let first = commit_candidates(&provider, vec![window_candidate(7, 101, "same")])[0].id();
        let replacement =
            commit_candidates(&provider, vec![window_candidate(7, 202, "same")])[0].id();

        assert_ne!(first, replacement, "R1-4: every snapshot mints a fresh ID");
        let old = provider
            .select_record(first)
            .expect("previous generation lease");
        let new = provider
            .select_record(replacement)
            .expect("replacement generation lease");
        assert!(matches!(old.item, CaptureItem::Synthetic(101)));
        assert!(matches!(new.item, CaptureItem::Synthetic(202)));

        old.lost.store(true, Ordering::Release);
        assert_eq!(
            provider
                .select_record(first)
                .expect_err("old lifetime is lost")
                .status(),
            Status::TargetLost
        );
        assert!(matches!(
            provider
                .select_record(replacement)
                .expect("replacement remains independent")
                .item,
            CaptureItem::Synthetic(202)
        ));
        assert!(provider.registry().generations.len() <= RETAINED_DISCOVERY_GENERATIONS);
    }

    #[test]
    fn retained_window_description_refuses_foreign_retired_display_and_unverified_targets() {
        let issuer = Arc::new(IdentityIssuer::new());
        let provider = WindowsCaptureProvider::new(Arc::clone(&issuer));
        let foreign = IdentityIssuer::new()
            .issue_target(super::PROVIDER)
            .expect("foreign");
        assert_eq!(
            provider
                .describe_window(foreign, &OperationContext::new())
                .expect_err("foreign authority")
                .status(),
            Status::InvalidArgument,
        );
        let retired = commit_candidates(&provider, vec![window_candidate(7, 1, "old")])[0].id();
        commit_candidates(&provider, vec![window_candidate(7, 2, "replacement")]);
        let current = commit_candidates(&provider, vec![window_candidate(7, 3, "current")])[0].id();
        assert_eq!(
            provider
                .describe_window(retired, &OperationContext::new())
                .expect_err("retired identity cannot reconstruct authority")
                .status(),
            Status::TargetLost,
        );
        let record = provider.select_record(current).expect("current record");
        assert_eq!(
            record.observe_window(None),
            Err(mado_pilot_capture::CaptureFault::UnsupportedOption)
        );
        assert!(record.capture_description().window().is_none());
        let mut display = window_candidate(9, 4, "display");
        display.key = NativeKey::Display(9);
        let display = commit_candidates(&provider, vec![display])[0].id();
        assert_eq!(
            provider
                .describe_window(display, &OperationContext::new())
                .expect_err("display is not a window")
                .status(),
            Status::Unsupported,
        );
    }

    #[test]
    fn retained_window_authority_requires_closed_registration_and_original_owner() {
        use crate::window_authority::WindowAuthorityStatus as Authority;
        use mado_pilot_capture::CaptureFault;

        assert_eq!(
            super::retained_window_authority(false, true, Some(Authority::SameTarget)),
            Ok(())
        );
        assert_eq!(
            super::retained_window_authority(true, true, Some(Authority::SameTarget)),
            Err(CaptureFault::TargetLost),
        );
        assert_eq!(
            super::retained_window_authority(false, false, Some(Authority::SameTarget)),
            Err(CaptureFault::UnsupportedOption),
        );
        for authority in [
            Authority::TargetLost,
            Authority::ReplacementOrReuse,
            Authority::RelationshipChanged,
        ] {
            assert_eq!(
                super::retained_window_authority(false, true, Some(authority)),
                Err(CaptureFault::TargetLost),
            );
        }
        for authority in [None, Some(Authority::Unavailable)] {
            assert_eq!(
                super::retained_window_authority(false, true, authority),
                Err(CaptureFault::UnsupportedOption),
            );
        }
    }

    #[test]
    fn required_geometry_refuses_movement_scale_area_resize_and_transitional_content() {
        use mado_pilot_capture::{CaptureFault, WindowCaptureArea, WindowGeometry};
        let placement = TargetPlacement::new(
            (-1600.0, -100.0),
            (640.0, 480.0),
            Scale::new(1.5, 1.5).expect("scale"),
        )
        .expect("placement");
        let extent = PixelExtent::new(960, 720);
        let required =
            WindowGeometry::new(WindowCaptureArea::WindowsExtendedFrame, placement, extent);
        assert_eq!(
            super::require_geometry(required, required, Some(extent)),
            Ok(())
        );
        let moved = TargetPlacement::new(
            (-1599.0, -100.0),
            (640.0, 480.0),
            Scale::new(1.5, 1.5).expect("scale"),
        )
        .expect("moved");
        let scaled = TargetPlacement::new(
            (-1600.0, -100.0),
            (960.0, 720.0),
            Scale::new(1.0, 1.0).expect("scale"),
        )
        .expect("scaled");
        for observed in [
            WindowGeometry::new(required.area(), moved, extent),
            WindowGeometry::new(required.area(), scaled, extent),
            WindowGeometry::new(WindowCaptureArea::WindowsClient, placement, extent),
            WindowGeometry::new(required.area(), placement, PixelExtent::new(961, 720)),
        ] {
            assert_eq!(
                super::require_geometry(required, observed, None),
                Err(CaptureFault::WindowGeometryChanged),
            );
        }
        assert_eq!(
            super::require_geometry(required, required, Some(PixelExtent::new(959, 720))),
            Err(CaptureFault::WindowGeometryChanged),
        );
    }

    #[test]
    fn describe_window_interruption_precedes_retained_lookup() {
        let issuer = Arc::new(IdentityIssuer::new());
        let missing = issuer
            .issue_target(super::PROVIDER)
            .expect("missing selection");
        let provider = WindowsCaptureProvider::new(issuer);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            provider
                .describe_window(
                    missing,
                    &OperationContext::new().with_cancellation(cancellation)
                )
                .expect_err("cancelled before stale lookup")
                .status(),
            Status::Cancelled,
        );
        assert_eq!(
            provider
                .describe_window(
                    missing,
                    &OperationContext::new().with_deadline(MonotonicInstant::ORIGIN),
                )
                .expect_err("deadline before stale lookup")
                .status(),
            Status::DeadlineExceeded,
        );
    }
}
