//! Bounded, pull-based, engine-scoped diagnostics.
//!
//! Diagnostics are structured owned data rather than log output. Producers never
//! call host code and never wait for queue capacity. The disabled path is
//! represented by the absence of a `DiagnosticSink`, so it allocates no queue
//! and issues no operation or template identities.

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, TryLockError, Weak};

use mado_pilot_core::{
    ActivityTag, CoordinateSpace, Error, FrameStamp, InputAddressScope, InputDelivery,
    InputOperationKind, Lifecycle, MonotonicInstant, Operation, OperationContext, PermissionKind,
    PermissionState, Status, SubmissionEvidence, TargetId,
};
use mado_pilot_input::{CleanupState, InputFault, InputReceipt, InputRequest, SequenceOutcome};
use mado_pilot_vision::PreparedTemplate;

/// Maximum number of retained diagnostic records per engine.
pub const MAX_DIAGNOSTIC_CAPACITY: usize = 65_536;

/// The amount of diagnostic detail an engine retains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DiagnosticLevel {
    /// Allocate no queue and issue no diagnostic identities.
    #[default]
    Off,
    /// Retain terminal public-operation summaries.
    Normal,
    /// Retain normal summaries and bounded decision detail.
    Debug,
}

impl DiagnosticLevel {
    const fn admits(self, record: Self) -> bool {
        matches!(
            (self, record),
            (Self::Normal, Self::Normal) | (Self::Debug, Self::Normal) | (Self::Debug, Self::Debug)
        )
    }
}

/// Validated engine diagnostic configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagnosticOptions {
    level: DiagnosticLevel,
    capacity: usize,
}

impl DiagnosticOptions {
    /// Returns the allocation-free default.
    #[must_use]
    pub const fn off() -> Self {
        Self {
            level: DiagnosticLevel::Off,
            capacity: 0,
        }
    }

    /// Validates a bounded diagnostic configuration.
    ///
    /// # Errors
    ///
    /// Enabled levels require `1..=MAX_DIAGNOSTIC_CAPACITY`; `Off` requires a
    /// zero capacity so an apparently disabled engine cannot reserve memory.
    pub fn new(level: DiagnosticLevel, capacity: usize) -> Result<Self, Error> {
        match level {
            DiagnosticLevel::Off if capacity == 0 => Ok(Self::off()),
            DiagnosticLevel::Off => Err(Error::new(
                Status::InvalidArgument,
                "diagnostics Off requires zero capacity",
            )),
            DiagnosticLevel::Normal | DiagnosticLevel::Debug
                if (1..=MAX_DIAGNOSTIC_CAPACITY).contains(&capacity) =>
            {
                Ok(Self { level, capacity })
            }
            DiagnosticLevel::Normal | DiagnosticLevel::Debug if capacity == 0 => Err(Error::new(
                Status::InvalidArgument,
                "enabled diagnostics require nonzero capacity",
            )),
            DiagnosticLevel::Normal | DiagnosticLevel::Debug => Err(Error::new(
                Status::LimitExceeded,
                "diagnostic capacity exceeds the implementation ceiling",
            )),
        }
    }

    /// Returns a validated normal-level configuration.
    pub fn normal(capacity: usize) -> Result<Self, Error> {
        Self::new(DiagnosticLevel::Normal, capacity)
    }

    /// Returns a validated debug-level configuration.
    pub fn debug(capacity: usize) -> Result<Self, Error> {
        Self::new(DiagnosticLevel::Debug, capacity)
    }

    /// Returns the selected level.
    #[must_use]
    pub const fn level(self) -> DiagnosticLevel {
        self.level
    }

    /// Returns the retained-record capacity, or zero for `Off`.
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }
}

impl Default for DiagnosticOptions {
    fn default() -> Self {
        Self::off()
    }
}

macro_rules! nonzero_identity {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(NonZeroU64);

        impl $name {
            const fn new(value: NonZeroU64) -> Self {
                Self(value)
            }

            /// Returns the engine-local numeric value.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }
    };
}

nonzero_identity!(
    DiagnosticOperationId,
    "A checked engine-local identity for one observed public operation."
);
nonzero_identity!(
    DiagnosticRecordSequence,
    "The authoritative engine-local commit order of retained records."
);
nonzero_identity!(
    DiagnosticTemplateId,
    "An opaque engine-local identity for one prepared template instance."
);

/// Public operations that may produce diagnostic records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticOperationKind {
    /// Target discovery.
    Discovery,
    /// Input capability description.
    InputDescription,
    /// Permission observation.
    Permission,
    /// Session opening.
    SessionOpen,
    /// Frame acquisition.
    FrameAcquire,
    /// Frame mapping.
    Mapping,
    /// Template preparation.
    TemplatePreparation,
    /// Template matching.
    Search,
    /// Input submission.
    InputSubmission,
    /// Session close.
    SessionClose,
}

/// Stable record payload categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// An operation was admitted for observation.
    OperationStarted,
    /// A frame was acquired.
    Frame,
    /// A frame mapping was resolved.
    Mapping,
    /// A search reached a terminal result.
    Search,
    /// An input submission reached a terminal receipt.
    Input,
    /// One route attempt was made or refused.
    RouteAttempt,
    /// A lifecycle operation failed or completed.
    Lifecycle,
    /// A permission state was observed or failed.
    Permission,
}

/// A compact set of input operation kinds without event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InputOperationSet(u8);

impl InputOperationSet {
    /// Returns whether `kind` is present.
    #[must_use]
    pub const fn contains(self, kind: InputOperationKind) -> bool {
        let index = match kind {
            InputOperationKind::Pointer => 0,
            InputOperationKind::Keyboard => 1,
            InputOperationKind::Text => 2,
            _ => return false,
        };
        self.0 & (1 << index) != 0
    }

    pub(crate) fn from_request(request: &InputRequest) -> Self {
        let mut set = Self::default();
        for kind in request.sequence().operation_kinds() {
            let index = match kind {
                InputOperationKind::Pointer => 0,
                InputOperationKind::Keyboard => 1,
                InputOperationKind::Text => 2,
                _ => continue,
            };
            set.0 |= 1 << index;
        }
        set
    }

    /// Returns the fixed bit representation for foreign boundaries.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// The terminal result of a template search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SearchDiagnosticOutcome {
    /// At least one result met the requested threshold.
    Matched,
    /// Matching completed successfully with no result.
    NoMatch,
    /// The operation failed with the enclosed status.
    Failed(Status),
}

/// A debug operation-admission record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationStartedDiagnostic {
    /// The admitted operation kind.
    pub operation: DiagnosticOperationKind,
}

/// A copied frame identity that retains no frame storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameDiagnostic {
    /// The public target identity.
    pub target: TargetId,
    /// The complete source-frame identity.
    pub frame: FrameStamp,
}

/// A copied mapping fact that retains no mapping bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MappingDiagnostic {
    /// The public target identity.
    pub target: TargetId,
    /// The mapped source frame.
    pub frame: FrameStamp,
    /// The source coordinate space.
    pub source: CoordinateSpace,
    /// The destination coordinate space.
    pub destination: CoordinateSpace,
}

/// A terminal search summary with no caller template name or pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SearchDiagnostic {
    /// The searched target.
    pub target: TargetId,
    /// The complete source-frame identity, absent when acquisition failed.
    pub frame: Option<FrameStamp>,
    /// The engine-issued template instance identity.
    pub template: DiagnosticTemplateId,
    /// The searched region's coordinate space, when the selection exposes one.
    pub region_space: Option<CoordinateSpace>,
    /// The terminal search result.
    pub outcome: SearchDiagnosticOutcome,
    /// The semantic result count.
    pub result_count: u64,
}

/// One immutable route attempt without input event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouteAttemptDiagnostic {
    /// The selected public target.
    pub target: TargetId,
    /// The attempted route.
    pub route: InputDelivery,
    /// How the route addresses its recipient.
    pub address_scope: InputAddressScope,
    /// The strongest transport fact available for the route.
    pub evidence: Option<SubmissionEvidence>,
    /// The attempt outcome.
    pub outcome: SequenceOutcome,
    /// Complete logical events submitted by this attempt.
    pub submitted: u64,
    /// Whether the current native unit may have had partial effect.
    pub partial_native_effect: bool,
    /// The typed terminal fault, if any.
    pub fault: Option<InputFault>,
}

/// A terminal input summary without text, keys, or native identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputDiagnostic {
    /// The selected public target.
    pub target: TargetId,
    /// Operation kinds present in the request.
    pub operations: InputOperationSet,
    /// Number of logical events in the bounded request.
    pub requested: u64,
    /// The selected route, if any route was attempted.
    pub route: Option<InputDelivery>,
    /// The selected route's address scope.
    pub address_scope: Option<InputAddressScope>,
    /// The selected route's strongest evidence.
    pub evidence: Option<SubmissionEvidence>,
    /// Terminal sequence outcome.
    pub outcome: SequenceOutcome,
    /// Complete logical events submitted to the native API.
    pub submitted: u64,
    /// Whether the current native unit may have had partial effect.
    pub partial_native_effect: bool,
    /// Typed terminal input fault, if any.
    pub fault: Option<InputFault>,
    /// The public terminal status, when submission failed.
    pub status: Option<Status>,
    /// Whether an earlier route was refused before the selected route.
    pub fallback: bool,
    /// Cleanup terminal state.
    pub cleanup: CleanupState,
    /// Sequence-owned states cleanup released.
    pub cleanup_released: u64,
    /// Sequence-owned states cleanup owed when it began.
    pub cleanup_owed: u64,
}

impl InputDiagnostic {
    pub(crate) fn from_receipt(request: &InputRequest, receipt: &InputReceipt) -> Self {
        Self {
            target: receipt.target(),
            operations: InputOperationSet::from_request(request),
            requested: request.sequence().len() as u64,
            route: receipt.selected_route(),
            address_scope: receipt.address_scope(),
            evidence: receipt.evidence(),
            outcome: receipt.outcome(),
            submitted: receipt.submitted() as u64,
            partial_native_effect: receipt.partial_native_effect(),
            fault: receipt.fault(),
            status: receipt.fault().map(InputFault::status),
            fallback: receipt.used_fallback(),
            cleanup: receipt.cleanup(),
            cleanup_released: receipt.cleanup_released() as u64,
            cleanup_owed: receipt.cleanup_owed() as u64,
        }
    }

    pub(crate) fn from_failure(request: &InputRequest, status: Status) -> Self {
        Self {
            target: request.target(),
            operations: InputOperationSet::from_request(request),
            requested: request.sequence().len() as u64,
            route: None,
            address_scope: None,
            evidence: None,
            outcome: SequenceOutcome::Unexecuted,
            submitted: 0,
            partial_native_effect: false,
            fault: None,
            status: Some(status),
            fallback: false,
            cleanup: CleanupState::NotNeeded,
            cleanup_released: 0,
            cleanup_owed: 0,
        }
    }
}

/// A lifecycle completion or failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LifecycleDiagnostic {
    /// The target, when the operation is target-scoped.
    pub target: Option<TargetId>,
    /// The observed terminal lifecycle.
    pub lifecycle: Lifecycle,
    /// A failure status, or `None` for successful completion.
    pub fault: Option<Status>,
}

/// A non-prompting permission observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PermissionDiagnostic {
    /// The permission that was read.
    pub permission: PermissionKind,
    /// The observed state, absent when the read failed.
    pub state: Option<PermissionState>,
    /// A failure status, or `None` for a successful read.
    pub fault: Option<Status>,
}

/// Closed, privacy-reviewed diagnostic payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticPayload {
    /// Debug operation-admission detail.
    OperationStarted(OperationStartedDiagnostic),
    /// Frame acquisition detail.
    Frame(FrameDiagnostic),
    /// Mapping detail.
    Mapping(MappingDiagnostic),
    /// Terminal search summary.
    Search(SearchDiagnostic),
    /// Terminal input summary.
    Input(InputDiagnostic),
    /// Per-route attempt detail.
    RouteAttempt(RouteAttemptDiagnostic),
    /// Lifecycle summary.
    Lifecycle(LifecycleDiagnostic),
    /// Permission summary.
    Permission(PermissionDiagnostic),
}

impl DiagnosticPayload {
    /// Returns the stable category of this payload.
    #[must_use]
    pub const fn kind(self) -> DiagnosticKind {
        match self {
            Self::OperationStarted(_) => DiagnosticKind::OperationStarted,
            Self::Frame(_) => DiagnosticKind::Frame,
            Self::Mapping(_) => DiagnosticKind::Mapping,
            Self::Search(_) => DiagnosticKind::Search,
            Self::Input(_) => DiagnosticKind::Input,
            Self::RouteAttempt(_) => DiagnosticKind::RouteAttempt,
            Self::Lifecycle(_) => DiagnosticKind::Lifecycle,
            Self::Permission(_) => DiagnosticKind::Permission,
        }
    }
}

/// One immutable retained diagnostic record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiagnosticRecord {
    sequence: DiagnosticRecordSequence,
    timestamp: MonotonicInstant,
    level: DiagnosticLevel,
    operation: DiagnosticOperationId,
    activity: Option<ActivityTag>,
    payload: DiagnosticPayload,
}

impl DiagnosticRecord {
    /// Returns the authoritative engine-local commit sequence.
    #[must_use]
    pub const fn sequence(&self) -> DiagnosticRecordSequence {
        self.sequence
    }

    /// Returns the observational timestamp in the library monotonic domain.
    #[must_use]
    pub const fn timestamp(&self) -> MonotonicInstant {
        self.timestamp
    }

    /// Returns the record level.
    #[must_use]
    pub const fn level(&self) -> DiagnosticLevel {
        self.level
    }

    /// Returns the checked operation identity.
    #[must_use]
    pub const fn operation(&self) -> DiagnosticOperationId {
        self.operation
    }

    /// Returns the optional caller activity correlation tag.
    #[must_use]
    pub const fn activity(&self) -> Option<ActivityTag> {
        self.activity
    }

    /// Returns the closed typed payload.
    #[must_use]
    pub const fn payload(&self) -> DiagnosticPayload {
        self.payload
    }

    /// Returns the payload category.
    #[must_use]
    pub const fn kind(&self) -> DiagnosticKind {
        self.payload.kind()
    }
}

/// Exact pending losses for one drain interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DiagnosticLosses {
    normal: u64,
    debug: u64,
}

impl DiagnosticLosses {
    /// Returns discarded normal-record count.
    #[must_use]
    pub const fn normal(self) -> u64 {
        self.normal
    }

    /// Returns discarded debug-record count.
    #[must_use]
    pub const fn debug(self) -> u64 {
        self.debug
    }

    /// Returns whether no records were discarded.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.normal == 0 && self.debug == 0
    }

    fn increment(&mut self, level: DiagnosticLevel) {
        match level {
            DiagnosticLevel::Normal => self.normal = self.normal.saturating_add(1),
            DiagnosticLevel::Debug => self.debug = self.debug.saturating_add(1),
            DiagnosticLevel::Off => {}
        }
    }
}

/// An immutable owned diagnostic batch independent of its engine and reader.
#[derive(Debug, Clone)]
pub struct DiagnosticBatch {
    records: Arc<[DiagnosticRecord]>,
    losses: DiagnosticLosses,
}

impl DiagnosticBatch {
    /// Returns the retained records in strict sequence order.
    #[must_use]
    pub fn records(&self) -> &[DiagnosticRecord] {
        &self.records
    }

    /// Returns the record count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns whether this is a loss-only batch.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Returns exact losses since the preceding committed batch.
    #[must_use]
    pub const fn losses(&self) -> DiagnosticLosses {
        self.losses
    }
}

/// The three observable outcomes of draining a diagnostic reader.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DiagnosticDrain {
    /// Records and/or pending loss counts were committed to an owned batch.
    Batch(DiagnosticBatch),
    /// No records or losses exist and production remains open.
    OpenEmpty,
    /// No records or losses exist and production is sealed.
    EndOfStream,
}

/// The one independently owned pull reader for an enabled engine.
#[derive(Debug)]
pub struct DiagnosticReader {
    stream: Arc<DiagnosticStream>,
}

impl DiagnosticReader {
    /// Drains all currently retained records and pending losses.
    ///
    /// The call is self-silent: it never produces a diagnostic record.
    #[must_use]
    pub fn drain(&self) -> DiagnosticDrain {
        self.stream.drain()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ObservedOperation {
    id: DiagnosticOperationId,
    activity: Option<ActivityTag>,
}

impl ObservedOperation {
    #[cfg(test)]
    pub(crate) const fn id(self) -> DiagnosticOperationId {
        self.id
    }
}

#[derive(Debug)]
struct StreamState {
    records: VecDeque<DiagnosticRecord>,
    losses: DiagnosticLosses,
    next_record: u64,
    sealed: bool,
}

#[derive(Debug)]
struct DiagnosticStream {
    level: DiagnosticLevel,
    capacity: usize,
    state: Mutex<StreamState>,
    contended_normal: AtomicU64,
    producers: AtomicUsize,
    contended_debug: AtomicU64,
    next_operation: AtomicU64,
    next_template: AtomicU64,
    templates: Mutex<HashMap<usize, DiagnosticTemplateId>>,
}

impl DiagnosticStream {
    fn new(options: DiagnosticOptions) -> Arc<Self> {
        Arc::new(Self {
            level: options.level(),
            capacity: options.capacity(),
            state: Mutex::new(StreamState {
                records: VecDeque::with_capacity(options.capacity()),
                losses: DiagnosticLosses::default(),
                next_record: 1,
                sealed: false,
            }),
            contended_normal: AtomicU64::new(0),
            contended_debug: AtomicU64::new(0),
            producers: AtomicUsize::new(1),
            next_operation: AtomicU64::new(1),
            next_template: AtomicU64::new(1),
            templates: Mutex::new(HashMap::new()),
        })
    }

    fn issue(counter: &AtomicU64, exhausted: &'static str) -> Result<NonZeroU64, Error> {
        let mut current = counter.load(Ordering::Acquire);
        loop {
            let Some(identity) = NonZeroU64::new(current) else {
                return Err(Error::new(Status::LimitExceeded, exhausted));
            };
            let next = current.checked_add(1).unwrap_or(0);
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(identity),
                Err(observed) => current = observed,
            }
        }
    }

    fn issue_operation(&self) -> Result<DiagnosticOperationId, Error> {
        Self::issue(
            &self.next_operation,
            "diagnostic operation identity space is exhausted",
        )
        .map(DiagnosticOperationId::new)
    }

    fn template_key(template: &PreparedTemplate) -> usize {
        std::ptr::from_ref(template.payload()).cast::<()>() as usize
    }

    fn issue_template(&self) -> Result<DiagnosticTemplateId, Error> {
        Self::issue(
            &self.next_template,
            "diagnostic template identity space is exhausted",
        )
        .map(DiagnosticTemplateId::new)
    }

    fn register_template(
        &self,
        template: &PreparedTemplate,
    ) -> Result<DiagnosticTemplateId, Error> {
        let key = Self::template_key(template);
        let mut templates = self
            .templates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !templates.contains_key(&key) && templates.len() == MAX_DIAGNOSTIC_CAPACITY {
            return Err(Error::new(
                Status::LimitExceeded,
                "diagnostic template registry capacity is exhausted",
            ));
        }
        let identity = self.issue_template()?;
        templates.insert(key, identity);
        Ok(identity)
    }

    fn template(&self, template: &PreparedTemplate) -> Result<DiagnosticTemplateId, Error> {
        let key = Self::template_key(template);
        let mut templates = self
            .templates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(identity) = templates.get(&key) {
            return Ok(*identity);
        }
        if templates.len() == MAX_DIAGNOSTIC_CAPACITY {
            return Err(Error::new(
                Status::LimitExceeded,
                "diagnostic template registry capacity is exhausted",
            ));
        }
        let identity = self.issue_template()?;
        templates.insert(key, identity);
        Ok(identity)
    }

    fn count_contended(&self, level: DiagnosticLevel) {
        let counter = match level {
            DiagnosticLevel::Normal => &self.contended_normal,
            DiagnosticLevel::Debug => &self.contended_debug,
            DiagnosticLevel::Off => return,
        };
        let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            Some(value.saturating_add(1))
        });
    }

    fn emit(
        &self,
        level: DiagnosticLevel,
        operation: ObservedOperation,
        timestamp: MonotonicInstant,
        payload: DiagnosticPayload,
    ) {
        if !self.level.admits(level) {
            return;
        }
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                self.count_contended(level);
                return;
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        if state.sealed {
            return;
        }

        if state.records.len() == self.capacity {
            let oldest_debug = state
                .records
                .iter()
                .position(|record| record.level == DiagnosticLevel::Debug);
            match (level, oldest_debug) {
                (DiagnosticLevel::Normal, index) => {
                    let removed = state
                        .records
                        .remove(index.unwrap_or(0))
                        .expect("a full queue has a removable record");
                    state.losses.increment(removed.level);
                }
                (DiagnosticLevel::Debug, Some(index)) => {
                    let removed = state
                        .records
                        .remove(index)
                        .expect("the indexed debug record exists");
                    state.losses.increment(removed.level);
                }
                (DiagnosticLevel::Debug, None) => {
                    state.losses.increment(DiagnosticLevel::Debug);
                    return;
                }
                (DiagnosticLevel::Off, _) => return,
            }
        }

        let Some(sequence) = NonZeroU64::new(state.next_record) else {
            state.losses.increment(level);
            return;
        };
        state.next_record = state.next_record.checked_add(1).unwrap_or(0);
        state.records.push_back(DiagnosticRecord {
            sequence: DiagnosticRecordSequence::new(sequence),
            timestamp,
            level,
            operation: operation.id,
            activity: operation.activity,
            payload,
        });
    }

    fn drain(&self) -> DiagnosticDrain {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let contended = DiagnosticLosses {
            normal: self.contended_normal.swap(0, Ordering::AcqRel),
            debug: self.contended_debug.swap(0, Ordering::AcqRel),
        };
        let losses = DiagnosticLosses {
            normal: state.losses.normal.saturating_add(contended.normal),
            debug: state.losses.debug.saturating_add(contended.debug),
        };
        if state.records.is_empty() && losses.is_empty() {
            return if state.sealed {
                DiagnosticDrain::EndOfStream
            } else {
                DiagnosticDrain::OpenEmpty
            };
        }

        let records: Vec<_> = state.records.drain(..).collect();
        state.losses = DiagnosticLosses::default();
        DiagnosticDrain::Batch(DiagnosticBatch {
            records: records.into(),
            losses,
        })
    }

    fn seal(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sealed = true;
    }
}

/// Internal non-blocking producer retained by an enabled engine and its sessions.
#[derive(Debug)]
pub(crate) struct DiagnosticSink {
    stream: Arc<DiagnosticStream>,
}

impl Clone for DiagnosticSink {
    fn clone(&self) -> Self {
        self.stream.producers.fetch_add(1, Ordering::Relaxed);
        Self {
            stream: Arc::clone(&self.stream),
        }
    }
}

impl Drop for DiagnosticSink {
    fn drop(&mut self) {
        // Weak mapping observers do not count as producers. The last real
        // engine/session producer seals the stream regardless of whether the
        // reader or retained frame handles still exist.
        if self.stream.producers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.stream.seal();
        }
    }
}

impl DiagnosticSink {
    pub(crate) fn create(options: DiagnosticOptions) -> Option<(Self, DiagnosticReader)> {
        if options.level() == DiagnosticLevel::Off {
            return None;
        }
        let stream = DiagnosticStream::new(options);
        Some((
            Self {
                stream: Arc::clone(&stream),
            },
            DiagnosticReader { stream },
        ))
    }

    pub(crate) fn emitter(&self) -> DiagnosticEmitter {
        DiagnosticEmitter {
            stream: Arc::downgrade(&self.stream),
        }
    }

    pub(crate) fn observe(
        &self,
        context: &OperationContext,
        kind: DiagnosticOperationKind,
    ) -> Result<ObservedOperation, Error> {
        Operation::admit(context)?;
        let observed = ObservedOperation {
            id: self.stream.issue_operation()?,
            activity: context.activity_tag(),
        };
        self.debug(observed, context, || {
            DiagnosticPayload::OperationStarted(OperationStartedDiagnostic { operation: kind })
        });
        Ok(observed)
    }

    pub(crate) fn normal(
        &self,
        operation: ObservedOperation,
        context: &OperationContext,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        self.stream
            .emit(DiagnosticLevel::Normal, operation, context.now(), payload());
    }

    pub(crate) fn debug(
        &self,
        operation: ObservedOperation,
        context: &OperationContext,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        if self.stream.level != DiagnosticLevel::Debug {
            return;
        }
        self.stream
            .emit(DiagnosticLevel::Debug, operation, context.now(), payload());
    }

    pub(crate) fn register_template(
        &self,
        template: &PreparedTemplate,
    ) -> Result<DiagnosticTemplateId, Error> {
        self.stream.register_template(template)
    }

    pub(crate) fn template(
        &self,
        template: &PreparedTemplate,
    ) -> Result<DiagnosticTemplateId, Error> {
        self.stream.template(template)
    }

    #[cfg(test)]
    pub(crate) fn seal(&self) {
        self.stream.seal();
    }

    #[cfg(test)]
    fn set_next_operation(&self, value: u64) {
        self.stream.next_operation.store(value, Ordering::Release);
    }
}

/// A non-owning producer used by retained outputs that may outlive the engine.
///
/// Upgrading this reference for one operation keeps the queue allocation valid
/// while emitting, but it never delays the final engine/session producer from
/// sealing the stream.
#[derive(Debug, Clone)]
pub(crate) struct DiagnosticEmitter {
    stream: Weak<DiagnosticStream>,
}

#[derive(Debug)]
pub(crate) struct DetachedObservation {
    stream: Arc<DiagnosticStream>,
    operation: ObservedOperation,
}

impl DiagnosticEmitter {
    pub(crate) fn observe(
        &self,
        context: &OperationContext,
        kind: DiagnosticOperationKind,
    ) -> Result<Option<DetachedObservation>, Error> {
        let Some(stream) = self.stream.upgrade() else {
            return Ok(None);
        };
        if stream.producers.load(Ordering::Acquire) == 0 {
            return Ok(None);
        }

        Operation::admit(context)?;
        let operation = ObservedOperation {
            id: stream.issue_operation()?,
            activity: context.activity_tag(),
        };
        if stream.level == DiagnosticLevel::Debug {
            stream.emit(
                DiagnosticLevel::Debug,
                operation,
                context.now(),
                DiagnosticPayload::OperationStarted(OperationStartedDiagnostic { operation: kind }),
            );
        }
        Ok(Some(DetachedObservation { stream, operation }))
    }
}

impl DetachedObservation {
    pub(crate) fn debug(
        &self,
        context: &OperationContext,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        if self.stream.level == DiagnosticLevel::Debug {
            self.stream.emit(
                DiagnosticLevel::Debug,
                self.operation,
                context.now(),
                payload(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Barrier, thread};

    use super::*;
    use mado_pilot_core::{IdentityIssuer, ProviderId};

    fn target() -> TargetId {
        IdentityIssuer::new()
            .issue_target(ProviderId::new("diagnostic-test"))
            .expect("issued")
    }

    fn input_payload() -> DiagnosticPayload {
        DiagnosticPayload::Lifecycle(LifecycleDiagnostic {
            target: Some(target()),
            lifecycle: Lifecycle::Open,
            fault: None,
        })
    }

    fn enabled(level: DiagnosticLevel, capacity: usize) -> (DiagnosticSink, DiagnosticReader) {
        DiagnosticSink::create(DiagnosticOptions::new(level, capacity).expect("valid"))
            .expect("enabled")
    }

    fn observe(sink: &DiagnosticSink, context: &OperationContext) -> ObservedOperation {
        sink.observe(context, DiagnosticOperationKind::Discovery)
            .expect("identity")
    }

    fn batch(reader: &DiagnosticReader) -> DiagnosticBatch {
        match reader.drain() {
            DiagnosticDrain::Batch(batch) => batch,
            other => panic!("expected batch, got {other:?}"),
        }
    }

    #[test]
    fn payloads_are_fixed_width_and_text_is_reduced_to_a_typed_operation() {
        fn assert_copy<T: Copy>() {}

        assert_copy::<DiagnosticPayload>();
        assert_copy::<DiagnosticRecord>();

        let target = target();
        let secret = "diagnostics-must-not-retain-this-text";
        let request = InputRequest::new(
            target,
            mado_pilot_input::InputSequence::new(vec![mado_pilot_input::InputEvent::Text(
                secret.to_owned(),
            )])
            .expect("one valid text event"),
            mado_pilot_input::DeliveryPlan::require(InputDelivery::System),
        );
        let receipt = InputReceipt::complete(
            target,
            InputDelivery::System,
            SubmissionEvidence::SystemInputAdmission,
            1,
        );
        let payload = DiagnosticPayload::Input(InputDiagnostic::from_receipt(&request, &receipt));

        assert!(
            matches!(
                payload,
                DiagnosticPayload::Input(input)
                    if input.operations.contains(InputOperationKind::Text)
            ),
            "the safe typed operation kind remains observable"
        );
        assert!(
            !format!("{payload:?}").contains(secret),
            "caller text has no diagnostic field to enter"
        );
    }

    #[test]
    fn disabled_options_allocate_no_stream() {
        assert!(DiagnosticSink::create(DiagnosticOptions::off()).is_none());
        assert!(matches!(
            DiagnosticOptions::new(DiagnosticLevel::Off, 1),
            Err(error) if error.status() == Status::InvalidArgument
        ));
    }

    #[test]
    fn capacities_are_bounded() {
        assert!(DiagnosticOptions::normal(0).is_err());
        assert!(matches!(
            DiagnosticOptions::debug(MAX_DIAGNOSTIC_CAPACITY + 1),
            Err(error) if error.status() == Status::LimitExceeded
        ));
    }

    #[test]
    fn normal_filters_debug_and_preserves_activity() {
        let (sink, reader) = enabled(DiagnosticLevel::Normal, 4);
        let tag = ActivityTag::new(77).expect("nonzero");
        let context = OperationContext::new().with_activity_tag(tag);
        let operation = observe(&sink, &context);
        sink.debug(operation, &context, input_payload);
        sink.normal(operation, &context, input_payload);

        let batch = batch(&reader);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.records()[0].level(), DiagnosticLevel::Normal);
        assert_eq!(batch.records()[0].activity(), Some(tag));
    }

    #[test]
    fn normal_evicts_oldest_debug_before_normal() {
        let (sink, reader) = enabled(DiagnosticLevel::Debug, 2);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        sink.normal(operation, &context, input_payload);
        sink.normal(operation, &context, input_payload);

        let batch = batch(&reader);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.losses().debug(), 1, "the start record was evicted");
        assert_eq!(batch.losses().normal(), 0);
    }

    #[test]
    fn debug_loses_to_an_all_normal_queue() {
        let (sink, reader) = enabled(DiagnosticLevel::Debug, 2);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        sink.normal(operation, &context, input_payload);
        sink.normal(operation, &context, input_payload);
        sink.debug(operation, &context, input_payload);

        let batch = batch(&reader);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.losses().debug(), 2);
        assert_eq!(batch.losses().normal(), 0);
    }

    #[test]
    fn contention_produces_a_loss_only_batch() {
        let (sink, reader) = enabled(DiagnosticLevel::Normal, 2);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        let _guard = sink.stream.state.lock().expect("uncontended");
        sink.normal(operation, &context, input_payload);
        drop(_guard);

        let batch = batch(&reader);
        assert!(batch.is_empty());
        assert_eq!(batch.losses().normal(), 1);
    }

    #[test]
    fn records_have_distinct_total_order_under_concurrency() {
        let (sink, reader) = enabled(DiagnosticLevel::Normal, 64);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        let context = &context;
        thread::scope(|scope| {
            for _ in 0..8 {
                let sink = sink.clone();
                scope.spawn(move || {
                    for _ in 0..4 {
                        sink.normal(operation, context, input_payload);
                    }
                });
            }
        });

        let batch = batch(&reader);
        let sequences: Vec<_> = batch
            .records()
            .iter()
            .map(|record| record.sequence().get())
            .collect();
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(sequences.len() as u64 + batch.losses().normal(), 32);
    }

    #[test]
    fn slow_reader_reports_every_concurrent_overflow_and_keeps_total_order() {
        const CAPACITY: usize = 8;
        const PRODUCERS: usize = 64;
        const RECORDS_PER_PRODUCER: usize = 32;

        let (sink, reader) = enabled(DiagnosticLevel::Normal, CAPACITY);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        let start = Arc::new(Barrier::new(PRODUCERS));
        thread::scope(|scope| {
            for _ in 0..PRODUCERS {
                let sink = sink.clone();
                let start = Arc::clone(&start);
                let context = &context;
                scope.spawn(move || {
                    start.wait();
                    for _ in 0..RECORDS_PER_PRODUCER {
                        sink.normal(operation, context, input_payload);
                    }
                });
            }
        });

        let batch = batch(&reader);
        let sequences: Vec<_> = batch
            .records()
            .iter()
            .map(|record| record.sequence().get())
            .collect();
        let produced = u64::try_from(PRODUCERS * RECORDS_PER_PRODUCER).expect("small count");
        assert!(batch.losses().normal() > 0, "the slow reader was outrun");
        assert_eq!(
            u64::try_from(sequences.len()).expect("small count") + batch.losses().normal(),
            produced,
            "every retained or discarded record is accounted for exactly once"
        );
        assert_eq!(batch.losses().debug(), 0);
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn concurrent_operations_publish_one_immutable_ordered_batch_view() {
        const PRODUCERS: usize = 8;
        const RECORDS_PER_PRODUCER: usize = 32;

        let (sink, reader) = enabled(DiagnosticLevel::Normal, PRODUCERS * RECORDS_PER_PRODUCER);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        let start = Arc::new(Barrier::new(PRODUCERS));
        thread::scope(|scope| {
            for _ in 0..PRODUCERS {
                let sink = sink.clone();
                let start = Arc::clone(&start);
                let context = &context;
                scope.spawn(move || {
                    start.wait();
                    for _ in 0..RECORDS_PER_PRODUCER {
                        sink.normal(operation, context, input_payload);
                    }
                });
            }
        });

        let shared = Arc::new(batch(&reader));
        let expected: Vec<_> = shared
            .records()
            .iter()
            .map(|record| record.sequence().get())
            .collect();
        assert!(!expected.is_empty());
        assert!(expected.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            u64::try_from(expected.len()).expect("small count") + shared.losses().normal(),
            u64::try_from(PRODUCERS * RECORDS_PER_PRODUCER).expect("small count")
        );

        thread::scope(|scope| {
            for _ in 0..8 {
                let shared = Arc::clone(&shared);
                let expected = &expected;
                scope.spawn(move || {
                    let observed: Vec<_> = shared
                        .records()
                        .iter()
                        .map(|record| record.sequence().get())
                        .collect();
                    assert_eq!(&observed, expected);
                    assert!(observed.windows(2).all(|pair| pair[0] < pair[1]));
                });
            }
        });

        let unchanged: Vec<_> = shared
            .records()
            .iter()
            .map(|record| record.sequence().get())
            .collect();
        assert_eq!(unchanged, expected);
    }

    #[test]
    fn sealed_reader_drains_then_reaches_end_of_stream() {
        let (sink, reader) = enabled(DiagnosticLevel::Normal, 2);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        sink.normal(operation, &context, input_payload);
        sink.seal();

        let retained = batch(&reader);
        assert_eq!(retained.len(), 1);
        assert!(matches!(reader.drain(), DiagnosticDrain::EndOfStream));
        assert_eq!(retained.records()[0].kind(), DiagnosticKind::Lifecycle);
    }

    #[test]
    fn empty_open_reader_is_distinct_and_self_silent() {
        let (_sink, reader) = enabled(DiagnosticLevel::Normal, 2);
        assert!(matches!(reader.drain(), DiagnosticDrain::OpenEmpty));
        assert!(matches!(reader.drain(), DiagnosticDrain::OpenEmpty));
    }

    #[test]
    fn operation_identity_exhaustion_fails_without_wrapping() {
        let (sink, _reader) = enabled(DiagnosticLevel::Normal, 2);
        sink.set_next_operation(u64::MAX);
        let context = OperationContext::new();
        assert_eq!(observe(&sink, &context).id().get(), u64::MAX);
        assert!(matches!(
            sink.observe(&context, DiagnosticOperationKind::Discovery),
            Err(error) if error.status() == Status::LimitExceeded
        ));
    }

    #[test]
    fn immutable_batches_are_safe_to_read_concurrently() {
        let (sink, reader) = enabled(DiagnosticLevel::Normal, 2);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        sink.normal(operation, &context, input_payload);
        let batch = batch(&reader);
        let shared = Arc::new(batch);

        thread::scope(|scope| {
            for _ in 0..4 {
                let batch = Arc::clone(&shared);
                scope.spawn(move || {
                    assert_eq!(batch.records()[0].kind(), DiagnosticKind::Lifecycle)
                });
            }
        });
    }
}
