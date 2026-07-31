//! Public Windows capture provider and its native-identity registry.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, OpenRequest, PixelFormat, TargetDescription,
};
use mado_pilot_core::{IdentityIssuer, Operation, OperationContext, ProviderId, Result, TargetId};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::GraphicsCaptureItem;
use windows::core::IInspectable;

use crate::availability::ensure_capture_available;
use crate::discovery::{Candidate, Fingerprint, NativeKey, TargetMetadata, inventory};
use crate::native::NativeSession;
use crate::storage::native_fault;

/// Provider name qualifying every native Windows target identity.
pub const PROVIDER: ProviderId = ProviderId::new("windows");

/// Picker-free Windows target discovery and WGC capture.
///
/// Construction touches no native API. Discovery and open perform the runtime
/// availability checks, which lets an application include this package without
/// making an unresolved minimum-Windows claim.
pub struct WindowsCaptureProvider {
    issuer: Arc<IdentityIssuer>,
    registry: Mutex<Registry>,
}

#[derive(Debug, Default)]
struct Registry {
    records: HashMap<TargetId, TargetState>,
    current: HashMap<NativeKey, TargetId>,
}

#[derive(Debug)]
enum TargetState {
    Live(Arc<TargetRecord>),
    Lost,
}

struct TargetRecord {
    id: TargetId,
    key: NativeKey,
    fingerprint: Fingerprint,
    metadata: Mutex<TargetMetadata>,
    item: GraphicsCaptureItem,
    lost: Arc<AtomicBool>,
    _closed_token: i64,
}

impl WindowsCaptureProvider {
    /// Creates a provider using identities from `issuer`.
    #[must_use]
    pub fn new(issuer: Arc<IdentityIssuer>) -> Self {
        Self {
            issuer,
            registry: Mutex::new(Registry::default()),
        }
    }

    fn synchronize(&self, candidates: Vec<Candidate>) -> Result<Vec<TargetDescription>> {
        let mut registry = self.registry();
        let mut seen = HashSet::new();
        let mut descriptions = Vec::with_capacity(candidates.len());

        for candidate in candidates {
            seen.insert(candidate.key);
            let existing_id = registry.current.get(&candidate.key).copied();
            let existing = existing_id
                .and_then(|id| registry.records.get(&id))
                .and_then(TargetState::live);
            let record = if let Some(record) = existing.as_ref()
                && same_incarnation(
                    record.lost.load(Ordering::Acquire),
                    &record.fingerprint,
                    &candidate.fingerprint,
                ) {
                *record.metadata() = candidate.metadata;
                Arc::clone(record)
            } else {
                if let Some(existing) = existing {
                    existing.lost.store(true, Ordering::Release);
                }
                if let Some(existing_id) = existing_id {
                    registry.records.insert(existing_id, TargetState::Lost);
                }
                let record = self.create_record(candidate)?;
                registry.current.insert(record.key, record.id);
                registry
                    .records
                    .insert(record.id, TargetState::Live(Arc::clone(&record)));
                record
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
            if let Some(id) = registry.current.remove(&key) {
                if let Some(TargetState::Live(record)) = registry.records.get(&id) {
                    record.lost.store(true, Ordering::Release);
                }
                registry.records.insert(id, TargetState::Lost);
            }
        }
        Ok(descriptions)
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
        let closed_token = candidate
            .item
            .Closed(&closed_handler)
            .map_err(native_fault)?;
        Ok(Arc::new(TargetRecord {
            id,
            key: candidate.key,
            fingerprint: candidate.fingerprint,
            metadata: Mutex::new(candidate.metadata),
            item: candidate.item,
            lost,
            _closed_token: closed_token,
        }))
    }

    fn validate_current(&self, record: &TargetRecord) -> Result<TargetMetadata> {
        if record.lost.load(Ordering::Acquire)
            || !record.key.is_present()
            || record.item.Size().is_err()
        {
            record.lost.store(true, Ordering::Release);
            return Err(CaptureFault::TargetLost.into());
        }
        let current = inventory()?
            .into_iter()
            .find(|candidate| candidate.key == record.key);
        match current {
            Some(candidate) if candidate.fingerprint == record.fingerprint => {
                *record.metadata() = candidate.metadata.clone();
                Ok(candidate.metadata)
            }
            _ => {
                record.lost.store(true, Ordering::Release);
                Err(CaptureFault::TargetLost.into())
            }
        }
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
            .field("known_targets", &registry.records.len())
            .field("current_targets", &registry.current.len())
            .finish()
    }
}

impl CaptureProvider for WindowsCaptureProvider {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        let attempt = Operation::admit(operation)?;
        ensure_capture_available()?;
        let candidates = inventory()?;
        let descriptions = self.synchronize(candidates)?;
        Ok(attempt.commit(descriptions)?)
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        let attempt = Operation::admit(operation)?;
        self.accepts_target(target, self.issuer.engine())?;
        ensure_capture_available()?;
        if let Some(required) = request.required_format()
            && required != PixelFormat::Bgra8
        {
            return Err(CaptureFault::UnsupportedOption.into());
        }

        let record = self
            .registry()
            .records
            .get(&target)
            .map(|record| match record {
                TargetState::Live(record) => Ok(Arc::clone(record)),
                TargetState::Lost => Err(CaptureFault::TargetLost),
            })
            .transpose()?
            .ok_or(CaptureFault::UnknownTarget)?;
        let metadata = self.validate_current(&record)?;
        let stream = self.issuer.issue_stream()?;
        let session = NativeSession::open(
            target,
            stream,
            record.key.kind(),
            record.key,
            metadata,
            record.item.clone(),
        )?;
        Ok(attempt.commit(session as Arc<dyn CaptureSession>)?)
    }
}

fn same_incarnation(lost: bool, existing: &Fingerprint, candidate: &Fingerprint) -> bool {
    !lost && existing == candidate
}

impl TargetState {
    fn live(&self) -> Option<Arc<TargetRecord>> {
        match self {
            Self::Live(record) => Some(Arc::clone(record)),
            Self::Lost => None,
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
    use std::sync::Arc;

    use mado_pilot_capture::{CaptureProvider, OpenRequest, PixelFormat};
    use mado_pilot_core::{IdentityIssuer, OperationContext, Status};

    use crate::discovery::Fingerprint;

    use super::{WindowsCaptureProvider, same_incarnation};

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
    fn discovery_is_deterministic_when_wgc_is_available() {
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
        assert_eq!(first_ids, second_ids);
        assert!(
            first
                .iter()
                .all(|target| target.provider() == super::PROVIDER)
        );
    }

    #[test]
    fn a_reused_native_handle_never_reuses_the_target_identity() {
        let original = Fingerprint::Window {
            process_id: 17,
            thread_id: 23,
            class_name: "MadoPilotSynthetic".to_owned(),
        };
        let replacement = Fingerprint::Window {
            process_id: 29,
            thread_id: 31,
            class_name: "MadoPilotSynthetic".to_owned(),
        };

        assert!(same_incarnation(false, &original, &original));
        assert!(!same_incarnation(true, &original, &original));
        assert!(!same_incarnation(false, &original, &replacement));
    }
}
