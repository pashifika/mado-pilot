//! Public macOS capture provider and its snapshot-scoped selection registry.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, OpenRequest, PixelFormat, TargetDescription,
};
use mado_pilot_core::{
    IdentityIssuer, Operation, OperationContext, PermissionKind, ProviderId, Result, TargetId,
    TargetKind,
};
use mado_pilot_input::{
    InputController, InputDescriptor, InputFault, InputOpenRequest, InputProvider,
};

use crate::availability::ensure_capture_available;
use crate::discovery::{Candidate, Fingerprint, NativeKey, TargetMetadata, inventory};
use crate::input::{GeometryLedger, MacosInputController};
use crate::native::{NativeSession, SessionTarget};
use crate::shim::{MAX_NATIVE_WAIT, NativeBounds, ShimStatus, TargetToken};

/// Provider name qualifying every native macOS target identity.
pub const PROVIDER: ProviderId = ProviderId::new("macos");

const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Current and immediately previous discovery selections remain openable.
const RETAINED_DISCOVERY_GENERATIONS: usize = 2;

/// Picker-free macOS target discovery and ScreenCaptureKit capture.
///
/// Construction touches no native API and requests no authorization. Discovery
/// and open perform the runtime availability check and the non-prompting
/// authorization preflight. The implementation is qualified on Apple Silicon
/// macOS 26.5.2 (25F84); older hosts are outside its support contract. Merely
/// constructing the provider still cannot change what the operating system asks
/// the user.
pub struct MacosCaptureProvider {
    issuer: Arc<IdentityIssuer>,
    discovery_gate: Mutex<()>,
    registry: Mutex<Registry>,
}

#[derive(Debug, Default)]
struct Registry {
    records: HashMap<TargetId, Arc<TargetRecord>>,
    generations: VecDeque<Vec<TargetId>>,
}

pub(crate) struct TargetRecord {
    id: TargetId,
    key: NativeKey,
    fingerprint: Fingerprint,
    selection: TargetToken,
    metadata: TargetMetadata,
    geometry: Arc<GeometryLedger>,
}

struct PreparedSnapshot {
    records: Vec<Arc<TargetRecord>>,
    descriptions: Vec<TargetDescription>,
    generation: Vec<TargetId>,
}

impl MacosCaptureProvider {
    /// Creates a provider using identities from `issuer`.
    #[must_use]
    pub fn new(issuer: Arc<IdentityIssuer>) -> Self {
        Self {
            issuer,
            discovery_gate: Mutex::new(()),
            registry: Mutex::new(Registry::default()),
        }
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

        // Final arbitration precedes the registry mutation. A late result may
        // consume issuer values while being staged, but it cannot add a generation
        // or evict an openable selection unless this operation commits success.
        let prepared = attempt.commit(prepared)?;
        Ok(self.commit_snapshot(prepared))
    }

    fn prepare_snapshot(&self, candidates: Vec<Candidate>) -> Result<PreparedSnapshot> {
        let mut records = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            records.push(self.create_record(candidate)?);
        }
        let descriptions = records.iter().map(|record| record.description()).collect();
        let generation = records.iter().map(|record| record.id).collect();
        Ok(PreparedSnapshot {
            records,
            descriptions,
            generation,
        })
    }

    fn commit_snapshot(&self, prepared: PreparedSnapshot) -> Vec<TargetDescription> {
        let PreparedSnapshot {
            records,
            descriptions,
            generation,
        } = prepared;
        let mut registry = self.registry();
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
        descriptions
    }

    fn create_record(&self, candidate: Candidate) -> Result<Arc<TargetRecord>> {
        let id = self.issuer.issue_target(PROVIDER)?;
        let Candidate {
            key,
            fingerprint,
            target,
            metadata,
        } = candidate;
        Ok(Arc::new(TargetRecord {
            id,
            key,
            fingerprint,
            selection: target,
            metadata,
            geometry: Arc::new(GeometryLedger::default()),
        }))
    }

    /// Returns the record an input operation names, or why it cannot be used.
    ///
    /// `TargetId` is snapshot-scoped exactly as it is for capture, so an accepted
    /// identity absent from the retained generations is conservatively stale
    /// rather than an invitation to re-resolve a native object by number.
    fn select_input_record(
        &self,
        target: TargetId,
        wait: Duration,
    ) -> std::result::Result<Arc<TargetRecord>, InputFault> {
        let record = self
            .registry()
            .records
            .get(&target)
            .cloned()
            .ok_or(InputFault::TargetLost)?;
        record.ensure_live(wait)?;
        Ok(record)
    }

    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Returns how long a native inventory query may wait, bounded by the caller's budget.
///
/// A fixed ceiling is not the caller's deadline. A request whose remaining budget is
/// shorter would block past it and then report whatever the native query returned — a
/// capture failure, say — rather than the deadline it actually missed. `None` means no
/// deadline, which is the one case the ceiling alone answers.
fn inventory_wait(remaining: Option<Duration>) -> Duration {
    remaining.map_or(MAX_NATIVE_WAIT, |remaining| remaining.min(MAX_NATIVE_WAIT))
}

impl fmt::Debug for MacosCaptureProvider {
    /// Formats counts only. A window title is desktop content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let registry = self.registry();
        formatter
            .debug_struct("MacosCaptureProvider")
            .field("engine", &self.issuer.engine())
            .field("known_targets", &registry.records.len())
            .field("retained_generations", &registry.generations.len())
            .finish()
    }
}

impl CaptureProvider for MacosCaptureProvider {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        ensure_capture_available()?;
        let wait = inventory_wait(operation.remaining());
        self.discover_with(operation, || inventory(wait))
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        let mut attempt = Operation::admit(operation)?;
        CaptureProvider::accepts_target(self, target, self.issuer.engine())?;
        ensure_capture_available()?;
        if let Some(required) = request.required_format()
            && required != PixelFormat::Bgra8
        {
            return Err(CaptureFault::UnsupportedOption.into());
        }

        // accepts_target established this engine and provider. TargetId is
        // snapshot-scoped, so only the current and previous discovery leases are
        // openable; an older identity is conservatively stale.
        let record = self
            .registry()
            .records
            .get(&target)
            .cloned()
            .ok_or(CaptureFault::TargetLost)?;
        let stream = self.issuer.issue_stream()?;
        let selected = SessionTarget::new(
            target,
            stream,
            record.key,
            record.fingerprint,
            record.selection.clone(),
            record.metadata.clone(),
            Arc::clone(&record.geometry),
        );
        let session = NativeSession::open(selected, &mut attempt)?;
        Ok(attempt.commit(session as Arc<dyn CaptureSession>)?)
    }
}

impl InputProvider for MacosCaptureProvider {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    /// Reports the authorization macOS grants for input separately from capture.
    ///
    /// Naming it is not a claim that it is held: the probe reads the decision and
    /// every irreversible event re-reads it, because macOS can revoke it between
    /// the two.
    fn permission(&self) -> Option<PermissionKind> {
        Some(PermissionKind::InputControl)
    }

    fn describe(&self, target: TargetId, operation: &OperationContext) -> Result<InputDescriptor> {
        let attempt = Operation::admit(operation)?;
        InputProvider::accepts_target(self, target, self.issuer.engine())?;
        let record = self.select_input_record(target, inventory_wait(operation.remaining()))?;
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
        let record = self.select_input_record(target, inventory_wait(operation.remaining()))?;
        request.check(record.input_descriptor().capability())?;
        let controller = MacosInputController::new(record);
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
        self.metadata.describe(self.id, self.key.kind())
    }

    pub(crate) fn target(&self) -> TargetId {
        self.id
    }

    pub(crate) fn kind(&self) -> TargetKind {
        self.key.kind()
    }

    /// Returns the owning process this target's discovery pass recorded.
    ///
    /// Zero for a display, which has none. This is descriptive validation
    /// metadata only: input may use it to narrow a current-window search, but
    /// only logical equality with the retained `SCWindow` authorizes that match.
    pub(crate) fn owner_process(&self) -> i64 {
        self.fingerprint.native_owner()
    }

    pub(crate) fn geometry(&self) -> &Arc<GeometryLedger> {
        &self.geometry
    }

    pub(crate) fn input_descriptor(&self) -> InputDescriptor {
        InputDescriptor::new(self.id, self.description().capability().input())
    }

    /// Reads the retained selection's current rectangle from a fresh bounded
    /// native observation, which is also how liveness is decided.
    ///
    /// For a window, the shim compares the current logical `SCWindow` with the
    /// object retained by the discovery filter. PID and native number only
    /// narrow that comparison and cannot select a replacement.
    pub(crate) fn current_bounds(
        &self,
        wait: Duration,
    ) -> std::result::Result<NativeBounds, InputFault> {
        self.selection
            .input_bounds(wait)
            .map_err(|status| match status {
                ShimStatus::TargetLost => InputFault::TargetLost,
                ShimStatus::InvalidArgument => InputFault::UnsupportedCoordinate,
                _ => InputFault::SubmissionFailed,
            })
    }

    /// Reads focus for this exact retained selection within `wait`.
    pub(crate) fn is_focused(&self, wait: Duration) -> std::result::Result<bool, ShimStatus> {
        self.selection.input_focused(wait)
    }

    pub(crate) fn ensure_live(&self, wait: Duration) -> std::result::Result<(), InputFault> {
        self.current_bounds(wait).map(|_bounds| ())
    }
}

impl fmt::Debug for TargetRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TargetRecord")
            .field("id", &self.id)
            .field("kind", &self.key.kind())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use mado_pilot_capture::{CaptureProvider, OpenRequest, PixelFormat};
    use mado_pilot_core::{
        Clock, IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent, Scale, Status,
        TargetKind, TargetPlacement,
    };

    use crate::discovery::{Candidate, Fingerprint, NativeKey, TargetMetadata};
    use crate::shim::TargetToken;

    use super::{MacosCaptureProvider, RETAINED_DISCOVERY_GENERATIONS, inventory_wait};

    fn window_candidate(incarnation: u64) -> Candidate {
        let extent = PixelExtent::new(64, 48);
        Candidate {
            key: NativeKey::Window(7),
            fingerprint: Fingerprint::Window { owner_process: 501 },
            target: TargetToken::synthetic(incarnation),
            metadata: TargetMetadata {
                name: "window".to_owned(),
                extent,
                placement: TargetPlacement::new(
                    (0.0, 0.0),
                    (64.0, 48.0),
                    Scale::new(1.0, 1.0).expect("scale"),
                )
                .expect("placement"),
            },
        }
    }

    fn commit_candidates(
        provider: &MacosCaptureProvider,
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
    fn a_native_inventory_wait_never_exceeds_the_callers_own_budget() {
        // The defect this pins: discovery passed the fixed ceiling regardless, so a
        // request with a shorter deadline blocked past it and then reported whatever
        // the native query returned rather than the deadline it missed.
        assert_eq!(
            inventory_wait(Some(Duration::from_millis(100))),
            Duration::from_millis(100)
        );
        assert_eq!(inventory_wait(Some(Duration::ZERO)), Duration::ZERO);
        assert_eq!(
            inventory_wait(Some(Duration::from_secs(30))),
            super::MAX_NATIVE_WAIT,
            "a budget longer than the ceiling is still bounded by it"
        );
        assert_eq!(
            inventory_wait(None),
            super::MAX_NATIVE_WAIT,
            "no deadline is the one case the ceiling alone answers"
        );
    }

    /// Statuses a host without Screen Recording authorization or without the
    /// capture framework legitimately reports, which every test below tolerates.
    fn is_unavailable(status: Status) -> bool {
        matches!(status, Status::Unsupported | Status::CaptureFailed)
    }

    #[test]
    fn a_foreign_identity_is_rejected_before_any_native_call() {
        let own = Arc::new(IdentityIssuer::new());
        let foreign = IdentityIssuer::new()
            .issue_target(super::PROVIDER)
            .expect("issued");
        let provider = MacosCaptureProvider::new(own);

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
        let provider = MacosCaptureProvider::new(issuer);

        let error = provider
            .open(
                absent,
                &OpenRequest::new().require_format(PixelFormat::Bgra8),
                &OperationContext::new(),
            )
            .expect_err("an absent accepted identity is not live");

        if is_unavailable(error.status()) {
            return;
        }
        assert_eq!(error.status(), Status::TargetLost);
    }

    #[test]
    fn an_unsupported_required_format_is_refused_without_opening_anything() {
        let issuer = Arc::new(IdentityIssuer::new());
        let target = issuer.issue_target(super::PROVIDER).expect("issued");
        let provider = MacosCaptureProvider::new(issuer);

        let error = provider
            .open(
                target,
                &OpenRequest::new().require_format(PixelFormat::Rgba8),
                &OperationContext::new(),
            )
            .expect_err("this adapter publishes bgra8 only");

        if is_unavailable(error.status()) {
            return;
        }
        assert_eq!(error.status(), Status::Unsupported);
    }

    #[test]
    fn successive_discoveries_mint_fresh_ids_and_keep_the_previous_generation_openable() {
        let provider = MacosCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let first = match provider.discover(&OperationContext::new()) {
            Ok(first) => first,
            Err(error) if is_unavailable(error.status()) => return,
            Err(error) => panic!("discovery failed on an authorized host: {error}"),
        };
        let second = provider
            .discover(&OperationContext::new())
            .expect("availability cannot disappear during one test");

        let first_ids: Vec<_> = first.iter().map(|target| target.id()).collect();
        let second_ids: Vec<_> = second.iter().map(|target| target.id()).collect();
        assert!(
            first_ids.iter().all(|id| !second_ids.contains(id)),
            "each discovery snapshot owns fresh identities even when live metadata changes"
        );
        assert!(
            first
                .iter()
                .chain(&second)
                .all(|target| target.provider() == super::PROVIDER)
        );
        assert!(
            first
                .iter()
                .chain(&second)
                .all(|target| target.format() == PixelFormat::Bgra8)
        );

        if let Some(old_display) = first
            .iter()
            .find(|target| target.capability().kind() == Some(TargetKind::Display))
        {
            let session = provider
                .open(
                    old_display.id(),
                    &OpenRequest::new(),
                    &OperationContext::new(),
                )
                .expect("the previous generation's retained filter still opens");
            session
                .close(&OperationContext::new())
                .expect("old-generation session closes");
        }
    }

    #[test]
    fn discovery_snapshots_commit_in_query_order() {
        let provider = Arc::new(MacosCaptureProvider::new(Arc::new(IdentityIssuer::new())));
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
    fn identical_metadata_in_two_snapshots_mints_fresh_ids_without_retargeting() {
        let provider = MacosCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let first = commit_candidates(&provider, vec![window_candidate(1)]);
        let second = commit_candidates(&provider, vec![window_candidate(2)]);

        assert_ne!(first[0].id(), second[0].id());
        let registry = provider.registry();
        let old = registry
            .records
            .get(&first[0].id())
            .expect("old lease retained");
        let new = registry
            .records
            .get(&second[0].id())
            .expect("new lease retained");
        assert_eq!(old.selection.synthetic_identity(), 1);
        assert_eq!(new.selection.synthetic_identity(), 2);
    }

    #[test]
    fn a_same_process_replacement_with_a_recycled_window_number_cannot_retarget_an_old_id() {
        let provider = MacosCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let old_id = commit_candidates(&provider, vec![window_candidate(1)])[0].id();
        let old_selection = provider
            .registry()
            .records
            .get(&old_id)
            .expect("old selection is retained")
            .selection
            .clone();

        // The replacement deliberately has the same PID and native window
        // number. Only the retained selection incarnation differs.
        old_selection.mark_synthetic_lost();
        let replacement_id = commit_candidates(&provider, vec![window_candidate(2)])[0].id();

        let error =
            mado_pilot_input::InputProvider::describe(&provider, old_id, &OperationContext::new())
                .expect_err("the old public identity names the lost incarnation");
        assert_eq!(error.status(), Status::TargetLost);
        mado_pilot_input::InputProvider::describe(
            &provider,
            replacement_id,
            &OperationContext::new(),
        )
        .expect("the replacement has its own fresh public identity");
    }

    #[test]
    fn discovery_generations_are_bounded_and_evict_only_expired_unopened_selections() {
        let provider = MacosCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let mut ids = Vec::new();
        for generation in 1..=RETAINED_DISCOVERY_GENERATIONS + 1 {
            let descriptions =
                commit_candidates(&provider, vec![window_candidate(generation as u64)]);
            ids.push(descriptions[0].id());
        }
        let registry = provider.registry();
        assert_eq!(registry.generations.len(), RETAINED_DISCOVERY_GENERATIONS);
        assert!(
            !registry.records.contains_key(&ids[0]),
            "the generation older than the lease is evicted"
        );
        assert!(registry.records.contains_key(&ids[1]));
        assert!(registry.records.contains_key(&ids[2]));
    }

    #[test]
    fn a_deadline_winning_at_final_arbitration_does_not_advance_the_registry() {
        let provider = MacosCaptureProvider::new(Arc::new(IdentityIssuer::new()));
        let first = commit_candidates(&provider, vec![window_candidate(1)])[0].id();
        let second = commit_candidates(&provider, vec![window_candidate(2)])[0].id();
        let generations_before = provider.registry().generations.clone();

        let clock = Arc::new(CommitDeadlineClock::default());
        let operation = OperationContext::new()
            .with_clock(clock)
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(1)));
        let error = provider
            .discover_with(&operation, || Ok(vec![window_candidate(3)]))
            .expect_err("the deadline wins after staging and before state mutation");

        assert_eq!(error.status(), Status::DeadlineExceeded);
        let registry = provider.registry();
        assert_eq!(registry.generations, generations_before);
        assert_eq!(registry.records.len(), RETAINED_DISCOVERY_GENERATIONS);
        assert!(registry.records.contains_key(&first));
        assert!(registry.records.contains_key(&second));
    }
}
