//! Public macOS capture provider and its native-identity registry.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::Duration;

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, OpenRequest, PixelFormat, TargetDescription,
};
use mado_pilot_core::{IdentityIssuer, Operation, OperationContext, ProviderId, Result, TargetId};

use crate::availability::ensure_capture_available;
use crate::discovery::{Candidate, Fingerprint, NativeKey, TargetMetadata, inventory};
use crate::native::NativeSession;
use crate::shim::MAX_NATIVE_WAIT;

/// Provider name qualifying every native macOS target identity.
pub const PROVIDER: ProviderId = ProviderId::new("macos");

const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Picker-free macOS target discovery and ScreenCaptureKit capture.
///
/// Construction touches no native API and requests no authorization. Discovery
/// and open perform the runtime availability check and the non-prompting
/// authorization preflight, which lets an application include this package
/// without making an unresolved minimum-macOS claim and without the presence of
/// the package changing what the operating system asks the user.
pub struct MacosCaptureProvider {
    issuer: Arc<IdentityIssuer>,
    discovery_gate: Mutex<()>,
    registry: Mutex<Registry>,
}

#[derive(Debug, Default)]
struct Registry {
    records: HashMap<TargetId, Arc<TargetRecord>>,
    current: HashMap<NativeKey, TargetId>,
}

struct TargetRecord {
    id: TargetId,
    key: NativeKey,
    fingerprint: Fingerprint,
    metadata: Mutex<TargetMetadata>,
    lost: AtomicBool,
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
        let descriptions = self.with_snapshot_order(&mut attempt, || {
            let candidates = inventory()?;
            self.synchronize(candidates)
        })?;
        Ok(attempt.commit(descriptions)?)
    }

    fn with_snapshot_order<T>(
        &self,
        operation: &mut Operation<'_>,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let _discovery = lock_with_operation(&self.discovery_gate, operation)?;
        action()
    }

    fn synchronize(&self, candidates: Vec<Candidate>) -> Result<Vec<TargetDescription>> {
        // Declared before the registry guard so every early return also unlocks
        // the registry before releasing the records it retired.
        let mut retired = Vec::new();
        let mut registry = self.registry();
        let mut seen = HashSet::new();
        let mut descriptions = Vec::with_capacity(candidates.len());

        for candidate in candidates {
            seen.insert(candidate.key);
            let existing_id = registry.current.get(&candidate.key).copied();
            let existing = existing_id
                .and_then(|id| registry.records.get(&id))
                .cloned();
            let record = if let Some(record) = existing.as_ref()
                && same_incarnation(
                    record.lost.load(Ordering::Acquire),
                    record.fingerprint,
                    candidate.fingerprint,
                ) {
                *record.metadata() = candidate.metadata;
                Arc::clone(record)
            } else {
                let replacement = self.create_record(candidate)?;
                if let Some(existing) = existing {
                    existing.lost.store(true, Ordering::Release);
                }
                if let Some(existing_id) = existing_id
                    && let Some(existing) = registry.records.remove(&existing_id)
                {
                    retired.push(existing);
                }
                registry.current.insert(replacement.key, replacement.id);
                registry
                    .records
                    .insert(replacement.id, Arc::clone(&replacement));
                replacement
            };
            descriptions.push(record.description());
        }

        let missing: Vec<NativeKey> = registry
            .current
            .keys()
            .copied()
            .filter(|key| !seen.contains(key))
            .collect();
        for key in missing {
            if let Some(id) = registry.current.remove(&key)
                && let Some(record) = registry.records.remove(&id)
            {
                record.lost.store(true, Ordering::Release);
                retired.push(record);
            }
        }
        drop(registry);
        drop(retired);
        Ok(descriptions)
    }

    fn create_record(&self, candidate: Candidate) -> Result<Arc<TargetRecord>> {
        let id = self.issuer.issue_target(PROVIDER)?;
        Ok(Arc::new(TargetRecord {
            id,
            key: candidate.key,
            fingerprint: candidate.fingerprint,
            metadata: Mutex::new(candidate.metadata),
            lost: AtomicBool::new(false),
        }))
    }

    fn validate_current(
        &self,
        record: &TargetRecord,
        operation: &mut Operation<'_>,
    ) -> Result<TargetMetadata> {
        let wait = inventory_wait(operation.context().remaining());
        self.with_snapshot_order(operation, || {
            if record.lost.load(Ordering::Acquire) || !record.key.is_present() {
                record.lost.store(true, Ordering::Release);
                return Err(CaptureFault::TargetLost.into());
            }
            let current = inventory(wait)?
                .into_iter()
                .find(|candidate| candidate.key == record.key);
            match current {
                Some(candidate)
                    if candidate.fingerprint == record.fingerprint
                        && !record.lost.load(Ordering::Acquire) =>
                {
                    *record.metadata() = candidate.metadata.clone();
                    Ok(candidate.metadata)
                }
                _ => {
                    record.lost.store(true, Ordering::Release);
                    Err(CaptureFault::TargetLost.into())
                }
            }
        })
    }

    fn remove_lost_record(&self, record: &Arc<TargetRecord>) {
        if !record.lost.load(Ordering::Acquire) {
            return;
        }
        let mut registry = self.registry();
        let is_current_record = registry
            .records
            .get(&record.id)
            .is_some_and(|known| Arc::ptr_eq(known, record));
        if !is_current_record {
            return;
        }
        registry.records.remove(&record.id);
        if registry.current.get(&record.key) == Some(&record.id) {
            registry.current.remove(&record.key);
        }
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
            .field("current_targets", &registry.current.len())
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
        self.accepts_target(target, self.issuer.engine())?;
        ensure_capture_available()?;
        if let Some(required) = request.required_format()
            && required != PixelFormat::Bgra8
        {
            return Err(CaptureFault::UnsupportedOption.into());
        }

        // accepts_target established this engine and provider. TargetId is
        // opaque, so an accepted identity absent from the live registry is
        // conservatively stale rather than an invitation to retain history.
        let record = self
            .registry()
            .records
            .get(&target)
            .cloned()
            .ok_or(CaptureFault::TargetLost)?;
        let metadata = match self.validate_current(&record, &mut attempt) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.remove_lost_record(&record);
                return Err(error);
            }
        };
        let stream = self.issuer.issue_stream()?;
        let session = NativeSession::open(
            target,
            stream,
            record.key,
            record.fingerprint,
            metadata,
            &mut attempt,
        )?;
        Ok(attempt.commit(session as Arc<dyn CaptureSession>)?)
    }
}

fn same_incarnation(lost: bool, existing: Fingerprint, candidate: Fingerprint) -> bool {
    !lost && existing == candidate
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
    fn metadata(&self) -> MutexGuard<'_, TargetMetadata> {
        self.metadata
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn description(&self) -> TargetDescription {
        self.metadata().describe(self.id, self.key.kind())
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
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use mado_pilot_capture::{CaptureProvider, OpenRequest, PixelFormat};
    use mado_pilot_core::{IdentityIssuer, Operation, OperationContext, Status};

    use super::{Fingerprint, MacosCaptureProvider, inventory_wait, same_incarnation};

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
    fn discovery_is_deterministic_when_capture_is_authorized() {
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
        assert_eq!(
            first_ids, second_ids,
            "an unchanged desktop keeps its identities and its order"
        );
        assert!(
            first
                .iter()
                .all(|target| target.provider() == super::PROVIDER)
        );
        assert!(
            first
                .iter()
                .all(|target| target.format() == PixelFormat::Bgra8)
        );
    }

    #[test]
    fn open_validation_and_discovery_share_one_snapshot_order() {
        let provider = Arc::new(MacosCaptureProvider::new(Arc::new(IdentityIssuer::new())));
        let (first_entered_tx, first_entered_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first = {
            let provider = Arc::clone(&provider);
            thread::spawn(move || {
                let context = OperationContext::new();
                let mut operation = Operation::admit(&context).expect("open validation admitted");
                provider.with_snapshot_order(&mut operation, || {
                    first_entered_tx.send(()).expect("signal first inventory");
                    release_first_rx.recv().expect("release first inventory");
                    Ok(())
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
            .expect("open validation");
        second_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second inventory enters after the first commit");
        second
            .join()
            .expect("second thread")
            .expect("second discovery");
    }

    #[test]
    fn a_reused_window_number_never_reuses_the_target_identity() {
        let original = Fingerprint::Window { owner_process: 501 };
        let replacement = Fingerprint::Window { owner_process: 907 };

        assert!(same_incarnation(false, original, original));
        assert!(
            !same_incarnation(true, original, original),
            "a target already reported lost is never the same incarnation"
        );
        assert!(!same_incarnation(false, original, replacement));
    }
}
