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

use crate::watch::{
    TemplateQueryId, TemplateQueryState, TemplateWorkCounts, TemplateWorkDisposition,
};
use mado_pilot_core::{
    ActivityTag, ClipPolicy, CoordinateSpace, Error, FrameStamp, InputAddressScope, InputDelivery,
    InputOperationKind, Lifecycle, MonotonicInstant, Operation, OperationContext, PermissionKind,
    PermissionState, PixelRect, Rect, Status, SubmissionEvidence, TargetId,
};
use mado_pilot_input::{
    CleanupState, InputAttempt, InputFault, InputReceipt, InputRequest, SequenceOutcome,
};
use mado_pilot_ocr::{
    ACCEPTED_BOUNDED_PROFILE_ID, ACCEPTED_G004_PROFILE_ID, OcrBackendDescriptor,
    OcrRegion as RequestedOcrRegion,
};
use mado_pilot_vision::prepared::{PreparedTemplate, PreparedTemplateInstance};

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
nonzero_identity!(
    DiagnosticOcrModelInstanceId,
    "An opaque library-issued engine-local identity for one configured OCR model instance."
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
    /// Bounded template-presence query.
    TemplateWatch,
    /// Input submission.
    InputSubmission,
    /// One-shot OCR recognition.
    OcrRecognition,
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
    /// Template watcher state, disposition, or terminal summary.
    TemplateWatch,
    /// OCR recognition reached a terminal result.
    Ocr,
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
    /// The exact effective searched region in full-frame capture pixels.
    pub region: Option<PixelRect>,
    /// The terminal search result.
    pub outcome: SearchDiagnosticOutcome,
    /// The semantic result count.
    pub result_count: u64,
}

/// The terminal result projected into a content-redacted watcher diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TemplateWatchDiagnosticOutcome {
    /// Confirmed stability produced a match.
    Matched,
    /// Explicit or query-context cancellation won.
    Cancelled,
    /// The query deadline won.
    DeadlineExceeded,
    /// Session close won.
    SessionClosed,
    /// Scheduler close won.
    SchedulerClosed,
    /// The target was lost.
    TargetLost,
    /// Finite queue policy could no longer satisfy the query.
    Overloaded,
    /// Mapping or backend work failed.
    Failed(Status),
}

/// Bounded watcher state with no template names, pixels, hashes, or backend payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemplateWatchDiagnostic {
    /// Engine-local query identity.
    pub query: TemplateQueryId,
    /// Target observed by the owning session.
    pub target: TargetId,
    /// Complete source identity when one has been considered.
    pub frame: Option<FrameStamp>,
    /// Effective capture-pixel region when mapping resolved it.
    pub region: Option<PixelRect>,
    /// Coarse query lifecycle state.
    pub state: TemplateQueryState,
    /// Confirmed consecutive-match count.
    pub confirmed_observations: u32,
    /// Confirmed matching span.
    pub confirmed_duration_nanos: u64,
    /// The transition represented by this record, when any.
    pub disposition: Option<TemplateWorkDisposition>,
    /// Saturating counts for every disposition.
    pub work: TemplateWorkCounts,
    /// Latest pending-frame depth, always zero or one.
    pub pending_count: u32,
    /// Current backend analysis depth, bounded by the scheduler descriptor.
    pub in_flight_count: u32,
    /// Current live query count for the owning session.
    pub session_query_count: u32,
    /// Current live query count for the engine scheduler.
    pub engine_query_count: u32,
    /// Caller-clock elapsed duration since query publication.
    pub elapsed_nanos: u64,
    /// Immutable terminal result, absent while pending.
    pub outcome: Option<TemplateWatchDiagnosticOutcome>,
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

impl RouteAttemptDiagnostic {
    pub(crate) fn from_attempt(target: TargetId, attempt: InputAttempt) -> Self {
        Self {
            target,
            route: attempt.route(),
            address_scope: attempt.address_scope(),
            evidence: attempt.evidence(),
            outcome: attempt.outcome(),
            submitted: attempt.submitted() as u64,
            partial_native_effect: attempt.partial_native_effect(),
            fault: attempt.fault(),
        }
    }
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
    /// The route on which native effect became possible, or the final refused
    /// route when every attempt was unexecuted.
    pub route: Option<InputDelivery>,
    /// The reported route's address scope.
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
        let refused_route = receipt
            .attempts()
            .last()
            .filter(|attempt| attempt.outcome() == SequenceOutcome::Unexecuted);
        Self {
            target: receipt.target(),
            operations: InputOperationSet::from_request(request),
            requested: request.sequence().len() as u64,
            route: receipt
                .selected_route()
                .or_else(|| refused_route.map(|attempt| attempt.route())),
            address_scope: receipt
                .address_scope()
                .or_else(|| refused_route.map(|attempt| attempt.address_scope())),
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

/// The accepted public OCR profile, without backend or caller model identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OcrDiagnosticProfile {
    /// Accepted G-004 RapidOCR PP-OCRv4 detector / PP-OCRv6 small recognizer profile.
    AcceptedG004,
    /// Accepted ADR 0040/0041 bounded-detector profile.
    BoundedDetector,
    /// No accepted public profile claim is made, as for a deterministic test double.
    Unspecified,
}

/// Exact caller-requested OCR geometry without storing floating-point values directly.
///
/// Edge bit patterns preserve the request exactly while keeping the diagnostic
/// payload closed, copyable, comparable, and hashable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcrRequestedRegionDiagnostic {
    space: CoordinateSpace,
    left_bits: u64,
    top_bits: u64,
    right_bits: u64,
    bottom_bits: u64,
    /// The request's clipping policy.
    pub clip_policy: ClipPolicy,
}

impl OcrRequestedRegionDiagnostic {
    fn new(rect: Rect, clip_policy: ClipPolicy) -> Self {
        Self {
            space: rect.space(),
            left_bits: rect.left().to_bits(),
            top_bits: rect.top().to_bits(),
            right_bits: rect.right().to_bits(),
            bottom_bits: rect.bottom().to_bits(),
            clip_policy,
        }
    }

    /// Returns the request coordinate space.
    #[must_use]
    pub const fn space(self) -> CoordinateSpace {
        self.space
    }

    /// Returns the requested left edge.
    #[must_use]
    pub const fn left(self) -> f64 {
        f64::from_bits(self.left_bits)
    }

    /// Returns the requested top edge.
    #[must_use]
    pub const fn top(self) -> f64 {
        f64::from_bits(self.top_bits)
    }

    /// Returns the requested right edge.
    #[must_use]
    pub const fn right(self) -> f64 {
        f64::from_bits(self.right_bits)
    }

    /// Returns the requested bottom edge.
    #[must_use]
    pub const fn bottom(self) -> f64 {
        f64::from_bits(self.bottom_bits)
    }
}

/// Typed terminal OCR outcome without recognized text or backend output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OcrDiagnosticOutcome {
    /// One or more normalized regions committed.
    Recognized,
    /// Recognition committed successfully with no non-empty normalized text.
    Empty,
    /// Recognition failed with the enclosed public status.
    Failed(Status),
}

/// One terminal OCR observation with content-redacted identities and counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcrDiagnostic {
    /// Opaque library-issued configured-model instance identity.
    pub model_instance: DiagnosticOcrModelInstanceId,
    /// Accepted public profile classification.
    pub profile: OcrDiagnosticProfile,
    /// Complete exact source-frame identity.
    pub source: FrameStamp,
    /// Requested source region, or `None` for the full frame.
    pub requested_region: Option<OcrRequestedRegionDiagnostic>,
    /// Effective clipped source region, when geometry resolution completed.
    pub effective_region: Option<PixelRect>,
    /// Shared grouped source envelope, absent for singular recognition or
    /// before grouped geometry resolution.
    pub source_envelope: Option<PixelRect>,
    /// Coordinate space of returned quadrilaterals.
    pub output_space: CoordinateSpace,
    /// Typed terminal outcome.
    pub outcome: OcrDiagnosticOutcome,
    /// Semantic committed result count, zero on failure.
    pub result_count: u64,
    /// Bounded caller zone count for grouped recognition.
    pub zone_count: Option<u64>,
    /// Unique immutable candidate count for grouped recognition.
    pub unique_candidate_count: Option<u64>,
    /// Caller-group candidate membership count for grouped recognition.
    pub membership_count: Option<u64>,
    /// Exact immutable result semantic bytes when the operation owns that evidence.
    pub result_bytes: Option<u64>,
    /// Exact detector runs for this request when the backend reports them.
    pub detector_runs: Option<u64>,
    /// Exact detector bytes for this request when the backend reports them.
    pub detector_bytes: Option<u64>,
    /// Exact recognizer runs for this request when the backend reports them.
    pub recognizer_runs: Option<u64>,
    /// Exact recognizer bytes for this request when the backend reports them.
    pub recognizer_bytes: Option<u64>,
    /// Caller-clock elapsed duration from runtime admission through final arbitration.
    pub elapsed_nanos: u64,
    /// Number of source pixels in the effective region, zero before resolution.
    pub source_pixels: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OcrDiagnosticContext {
    pub(crate) model_instance: DiagnosticOcrModelInstanceId,
    pub(crate) profile: OcrDiagnosticProfile,
}

pub(crate) fn requested_ocr_region(
    region: RequestedOcrRegion,
) -> Option<OcrRequestedRegionDiagnostic> {
    match region {
        RequestedOcrRegion::FullFrame => None,
        RequestedOcrRegion::Region { rect, policy } => {
            Some(OcrRequestedRegionDiagnostic::new(rect, policy))
        }
        _ => None,
    }
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
    /// Bounded template watcher detail.
    TemplateWatch(TemplateWatchDiagnostic),
    /// Terminal OCR summary.
    Ocr(OcrDiagnostic),
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
            Self::TemplateWatch(_) => DiagnosticKind::TemplateWatch,
            Self::Ocr(_) => DiagnosticKind::Ocr,
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
    pending_normal: AtomicU64,
    producers: AtomicUsize,
    pending_debug: AtomicU64,
    next_operation: AtomicU64,
    next_template: AtomicU64,
    next_ocr_model: AtomicU64,
    templates: Mutex<HashMap<PreparedTemplateInstance, DiagnosticTemplateId>>,
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
            pending_normal: AtomicU64::new(0),
            pending_debug: AtomicU64::new(0),
            producers: AtomicUsize::new(1),
            next_operation: AtomicU64::new(1),
            next_template: AtomicU64::new(1),
            next_ocr_model: AtomicU64::new(1),
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

    fn issue_template(&self) -> Option<DiagnosticTemplateId> {
        Self::issue(
            &self.next_template,
            "diagnostic template identity space is exhausted",
        )
        .ok()
        .map(DiagnosticTemplateId::new)
    }

    fn issue_ocr_model(&self) -> Option<DiagnosticOcrModelInstanceId> {
        Self::issue(
            &self.next_ocr_model,
            "diagnostic OCR model identity space is exhausted",
        )
        .ok()
        .map(DiagnosticOcrModelInstanceId::new)
    }

    fn template(&self, template: &PreparedTemplate) -> Option<DiagnosticTemplateId> {
        let instance = template.diagnostic_instance();
        let mut templates = self
            .templates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(identity) = templates.get(&instance) {
            return Some(*identity);
        }
        if templates.len() >= MAX_DIAGNOSTIC_CAPACITY {
            templates.retain(|instance, _| instance.is_live());
            if templates.len() >= MAX_DIAGNOSTIC_CAPACITY {
                return None;
            }
        }

        let identity = self.issue_template()?;
        templates.insert(instance, identity);
        Some(identity)
    }

    fn count_loss(&self, level: DiagnosticLevel) {
        let counter = match level {
            DiagnosticLevel::Normal => &self.pending_normal,
            DiagnosticLevel::Debug => &self.pending_debug,
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
                self.count_loss(level);
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
        let pending = DiagnosticLosses {
            normal: self.pending_normal.swap(0, Ordering::AcqRel),
            debug: self.pending_debug.swap(0, Ordering::AcqRel),
        };
        let losses = DiagnosticLosses {
            normal: state.losses.normal.saturating_add(pending.normal),
            debug: state.losses.debug.saturating_add(pending.debug),
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

    pub(crate) fn ocr_model(
        &self,
        descriptor: &OcrBackendDescriptor,
    ) -> Option<OcrDiagnosticContext> {
        self.stream.issue_ocr_model().map(|model_instance| {
            let profile = if descriptor.profile().as_str() == ACCEPTED_G004_PROFILE_ID {
                OcrDiagnosticProfile::AcceptedG004
            } else if descriptor.profile().as_str() == ACCEPTED_BOUNDED_PROFILE_ID {
                OcrDiagnosticProfile::BoundedDetector
            } else {
                OcrDiagnosticProfile::Unspecified
            };
            OcrDiagnosticContext {
                model_instance,
                profile,
            }
        })
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
        self.normal_at(operation, context.now(), payload);
    }

    pub(crate) fn normal_at(
        &self,
        operation: ObservedOperation,
        timestamp: MonotonicInstant,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        self.stream
            .emit(DiagnosticLevel::Normal, operation, timestamp, payload());
    }

    pub(crate) fn admits_debug(&self) -> bool {
        self.stream.level == DiagnosticLevel::Debug
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
        self.debug_at(operation, context.now(), payload);
    }

    pub(crate) fn debug_at(
        &self,
        operation: ObservedOperation,
        timestamp: MonotonicInstant,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        if self.stream.level != DiagnosticLevel::Debug {
            return;
        }
        self.stream
            .emit(DiagnosticLevel::Debug, operation, timestamp, payload());
    }

    pub(crate) fn register_template(&self, template: &PreparedTemplate) {
        let _ = self.stream.template(template);
    }

    pub(crate) fn template(&self, template: &PreparedTemplate) -> Option<DiagnosticTemplateId> {
        self.stream.template(template)
    }

    pub(crate) fn normal_loss(&self) {
        self.stream.count_loss(DiagnosticLevel::Normal);
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
    use std::{sync::Arc, sync::Barrier, thread};

    use mado_pilot_capture::{CoordinateSupport, PixelFormat, TargetDescription};
    use mado_pilot_core::{
        FrameSequence, GeometryRevision, IdentityIssuer, PixelExtent, ProviderId, StreamEpoch,
    };
    use mado_pilot_input::{DeliveryPlan, InputAttempt, InputEvent, InputSequence, Key};
    use mado_pilot_testkit::{ControlledMatcher, match_fixtures};
    use mado_pilot_vision::{MatchBackend, Matcher};

    use super::*;

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

    fn ocr_payload() -> DiagnosticPayload {
        let issuer = IdentityIssuer::new();
        let source = FrameStamp::new(
            issuer.issue_stream().expect("issued"),
            StreamEpoch::FIRST,
            FrameSequence::FIRST,
            GeometryRevision::FIRST,
        );
        DiagnosticPayload::Ocr(OcrDiagnostic {
            model_instance: DiagnosticOcrModelInstanceId::new(NonZeroU64::new(1).expect("nonzero")),
            profile: OcrDiagnosticProfile::AcceptedG004,
            source,
            requested_region: None,
            effective_region: Some(PixelRect::new(0, 0, 8, 8).expect("valid")),
            source_envelope: None,
            output_space: CoordinateSpace::CapturePixels,
            outcome: OcrDiagnosticOutcome::Recognized,
            result_count: 1,
            zone_count: None,
            unique_candidate_count: None,
            membership_count: None,
            result_bytes: None,
            detector_runs: None,
            detector_bytes: None,
            recognizer_runs: None,
            recognizer_bytes: None,
            elapsed_nanos: 4,
            source_pixels: 64,
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

    fn process_request(target: TargetId, delivery: DeliveryPlan) -> InputRequest {
        InputRequest::new(
            target,
            InputSequence::new(vec![
                InputEvent::Text("diagnostic-secret-text".to_owned()),
                InputEvent::KeyPress(Key::Character('🔐')),
            ])
            .expect("valid input sequence"),
            delivery,
        )
    }

    #[test]
    fn complete_process_invocation_reports_process_scope_and_invocation_evidence() {
        let target = target();
        let request = process_request(
            target,
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        );
        let receipt = InputReceipt::complete(
            target,
            InputDelivery::ProcessDirected,
            SubmissionEvidence::InvocationOnly,
            2,
        );

        let normal = InputDiagnostic::from_receipt(&request, &receipt);
        assert_eq!(normal.target, target);
        assert!(normal.operations.contains(InputOperationKind::Text));
        assert!(normal.operations.contains(InputOperationKind::Keyboard));
        assert_eq!(normal.requested, 2);
        assert_eq!(normal.route, Some(InputDelivery::ProcessDirected));
        assert_eq!(normal.address_scope, Some(InputAddressScope::OwningProcess));
        assert_eq!(normal.evidence, Some(SubmissionEvidence::InvocationOnly));
        assert_eq!(normal.outcome, SequenceOutcome::Complete);
        assert_eq!(normal.submitted, 2);
        assert!(!normal.partial_native_effect);
        assert_eq!(normal.fault, None);
        assert_eq!(normal.status, None);
        assert!(!normal.fallback);
        assert_eq!(normal.cleanup, CleanupState::NotNeeded);
        assert_eq!(normal.cleanup_released, 0);
        assert_eq!(normal.cleanup_owed, 0);

        let debug = RouteAttemptDiagnostic::from_attempt(target, receipt.attempts()[0]);
        assert_eq!(debug.route, InputDelivery::ProcessDirected);
        assert_eq!(debug.address_scope, InputAddressScope::OwningProcess);
        assert_eq!(debug.evidence, Some(SubmissionEvidence::InvocationOnly));
        assert_eq!(debug.outcome, SequenceOutcome::Complete);
        assert_eq!(debug.submitted, 2);
        assert!(!debug.partial_native_effect);
        assert_eq!(debug.fault, None);
    }

    #[test]
    fn refused_process_route_reports_typed_fault_without_evidence_or_fallback() {
        let target = target();
        let request = process_request(
            target,
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        );
        let receipt = InputReceipt::unexecuted(target, InputFault::RouteUnavailable)
            .with_prior_attempts(vec![InputAttempt::refused(
                InputDelivery::ProcessDirected,
                InputFault::NotAuthorized,
            )]);

        let normal = InputDiagnostic::from_receipt(&request, &receipt);
        assert_eq!(normal.route, Some(InputDelivery::ProcessDirected));
        assert_eq!(normal.address_scope, Some(InputAddressScope::OwningProcess));
        assert_eq!(normal.evidence, None);
        assert_eq!(normal.outcome, SequenceOutcome::Unexecuted);
        assert_eq!(normal.submitted, 0);
        assert!(!normal.partial_native_effect);
        assert_eq!(normal.fault, Some(InputFault::RouteUnavailable));
        assert_eq!(normal.status, Some(Status::Unsupported));
        assert!(!normal.fallback);
        assert_eq!(normal.cleanup, CleanupState::NotNeeded);

        let debug = RouteAttemptDiagnostic::from_attempt(target, receipt.attempts()[0]);
        assert_eq!(debug.route, InputDelivery::ProcessDirected);
        assert_eq!(debug.address_scope, InputAddressScope::OwningProcess);
        assert_eq!(debug.evidence, None);
        assert_eq!(debug.outcome, SequenceOutcome::Unexecuted);
        assert_eq!(debug.submitted, 0);
        assert!(!debug.partial_native_effect);
        assert_eq!(debug.fault, Some(InputFault::NotAuthorized));
    }

    #[test]
    fn partial_process_invocation_reports_possible_native_effect_and_closes_fallback() {
        let target = target();
        let request = process_request(
            target,
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        );
        let receipt = InputReceipt::partial(
            target,
            InputDelivery::ProcessDirected,
            SubmissionEvidence::InvocationOnly,
            0,
            true,
            InputFault::SubmissionFailed,
        );

        let normal = InputDiagnostic::from_receipt(&request, &receipt);
        assert_eq!(normal.route, Some(InputDelivery::ProcessDirected));
        assert_eq!(normal.address_scope, Some(InputAddressScope::OwningProcess));
        assert_eq!(normal.evidence, Some(SubmissionEvidence::InvocationOnly));
        assert_eq!(normal.outcome, SequenceOutcome::Partial);
        assert_eq!(normal.submitted, 0);
        assert!(normal.partial_native_effect);
        assert_eq!(normal.fault, Some(InputFault::SubmissionFailed));
        assert_eq!(normal.status, Some(Status::InputFailed));
        assert!(!normal.fallback);

        let debug = RouteAttemptDiagnostic::from_attempt(target, receipt.attempts()[0]);
        assert_eq!(debug.outcome, SequenceOutcome::Partial);
        assert_eq!(debug.submitted, 0);
        assert!(debug.partial_native_effect);
        assert_eq!(debug.fault, Some(InputFault::SubmissionFailed));
    }

    #[test]
    fn cleanup_failed_process_invocation_reports_released_and_owed_counts() {
        let target = target();
        let request = process_request(
            target,
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        );
        let receipt = InputReceipt::partial(
            target,
            InputDelivery::ProcessDirected,
            SubmissionEvidence::InvocationOnly,
            1,
            false,
            InputFault::Cancelled,
        )
        .with_cleanup(1, 2);

        let normal = InputDiagnostic::from_receipt(&request, &receipt);
        assert_eq!(normal.route, Some(InputDelivery::ProcessDirected));
        assert_eq!(normal.address_scope, Some(InputAddressScope::OwningProcess));
        assert_eq!(normal.evidence, Some(SubmissionEvidence::InvocationOnly));
        assert_eq!(normal.outcome, SequenceOutcome::Partial);
        assert_eq!(normal.submitted, 1);
        assert!(!normal.partial_native_effect);
        assert_eq!(normal.fault, Some(InputFault::Cancelled));
        assert_eq!(normal.status, Some(Status::Cancelled));
        assert!(!normal.fallback);
        assert_eq!(normal.cleanup, CleanupState::Incomplete);
        assert_eq!(normal.cleanup_released, 1);
        assert_eq!(normal.cleanup_owed, 2);
    }

    #[test]
    fn explicit_fallback_is_distinct_from_a_refused_process_attempt() {
        let target = target();
        let request = process_request(
            target,
            DeliveryPlan::ordered(vec![InputDelivery::ProcessDirected, InputDelivery::System])
                .expect("distinct ordered routes"),
        );
        let receipt = InputReceipt::complete(
            target,
            InputDelivery::System,
            SubmissionEvidence::InvocationOnly,
            2,
        )
        .with_prior_attempts(vec![InputAttempt::refused(
            InputDelivery::ProcessDirected,
            InputFault::NotAuthorized,
        )]);

        let normal = InputDiagnostic::from_receipt(&request, &receipt);
        assert_eq!(normal.route, Some(InputDelivery::System));
        assert_eq!(normal.address_scope, Some(InputAddressScope::FocusedSystem));
        assert_eq!(normal.evidence, Some(SubmissionEvidence::InvocationOnly));
        assert_eq!(normal.outcome, SequenceOutcome::Complete);
        assert_eq!(normal.submitted, 2);
        assert!(!normal.partial_native_effect);
        assert_eq!(normal.fault, None);
        assert_eq!(normal.status, None);
        assert!(normal.fallback);

        let attempts = receipt.attempts();
        assert_eq!(attempts.len(), 2);
        let refused = RouteAttemptDiagnostic::from_attempt(target, attempts[0]);
        assert_eq!(refused.route, InputDelivery::ProcessDirected);
        assert_eq!(refused.address_scope, InputAddressScope::OwningProcess);
        assert_eq!(refused.evidence, None);
        assert_eq!(refused.outcome, SequenceOutcome::Unexecuted);
        assert_eq!(refused.fault, Some(InputFault::NotAuthorized));
        let fallback = RouteAttemptDiagnostic::from_attempt(target, attempts[1]);
        assert_eq!(fallback.route, InputDelivery::System);
        assert_eq!(fallback.address_scope, InputAddressScope::FocusedSystem);
        assert_eq!(fallback.evidence, Some(SubmissionEvidence::InvocationOnly));
        assert_eq!(fallback.outcome, SequenceOutcome::Complete);
        assert_eq!(fallback.fault, None);
    }

    #[test]
    fn every_payload_variant_excludes_caller_sensitive_material_from_public_fields_and_debug() {
        fn assert_copy<T: Copy>() {}

        fn public_fields(payload: DiagnosticPayload) -> Vec<String> {
            match payload {
                DiagnosticPayload::OperationStarted(value) => {
                    vec![format!("{:?}", value.operation)]
                }
                DiagnosticPayload::Frame(value) => {
                    vec![format!("{:?}", value.target), format!("{:?}", value.frame)]
                }
                DiagnosticPayload::Mapping(value) => vec![
                    format!("{:?}", value.target),
                    format!("{:?}", value.frame),
                    format!("{:?}", value.source),
                    format!("{:?}", value.destination),
                ],
                DiagnosticPayload::Search(value) => vec![
                    format!("{:?}", value.target),
                    format!("{:?}", value.frame),
                    format!("{:?}", value.template),
                    format!("{:?}", value.region),
                    format!("{:?}", value.outcome),
                    format!("{:?}", value.result_count),
                ],
                DiagnosticPayload::TemplateWatch(value) => vec![
                    format!("{:?}", value.query),
                    format!("{:?}", value.target),
                    format!("{:?}", value.frame),
                    format!("{:?}", value.region),
                    format!("{:?}", value.state),
                    format!("{:?}", value.confirmed_observations),
                    format!("{:?}", value.confirmed_duration_nanos),
                    format!("{:?}", value.disposition),
                    format!("{:?}", value.pending_count),
                    format!("{:?}", value.in_flight_count),
                    format!("{:?}", value.session_query_count),
                    format!("{:?}", value.engine_query_count),
                    format!("{:?}", value.elapsed_nanos),
                    format!("{:?}", value.outcome),
                ],
                DiagnosticPayload::Ocr(value) => vec![
                    format!("{:?}", value.model_instance),
                    format!("{:?}", value.profile),
                    format!("{:?}", value.source),
                    format!("{:?}", value.requested_region),
                    format!("{:?}", value.effective_region),
                    format!("{:?}", value.source_envelope),
                    format!("{:?}", value.output_space),
                    format!("{:?}", value.outcome),
                    format!("{:?}", value.result_count),
                    format!("{:?}", value.zone_count),
                    format!("{:?}", value.unique_candidate_count),
                    format!("{:?}", value.membership_count),
                    format!("{:?}", value.result_bytes),
                    format!("{:?}", value.detector_runs),
                    format!("{:?}", value.detector_bytes),
                    format!("{:?}", value.recognizer_runs),
                    format!("{:?}", value.recognizer_bytes),
                    format!("{:?}", value.elapsed_nanos),
                    format!("{:?}", value.source_pixels),
                ],
                DiagnosticPayload::Input(value) => vec![
                    format!("{:?}", value.target),
                    format!("{:?}", value.operations),
                    format!("{:?}", value.requested),
                    format!("{:?}", value.route),
                    format!("{:?}", value.address_scope),
                    format!("{:?}", value.evidence),
                    format!("{:?}", value.outcome),
                    format!("{:?}", value.submitted),
                    format!("{:?}", value.partial_native_effect),
                    format!("{:?}", value.fault),
                    format!("{:?}", value.status),
                    format!("{:?}", value.fallback),
                    format!("{:?}", value.cleanup),
                    format!("{:?}", value.cleanup_released),
                    format!("{:?}", value.cleanup_owed),
                ],
                DiagnosticPayload::RouteAttempt(value) => vec![
                    format!("{:?}", value.target),
                    format!("{:?}", value.route),
                    format!("{:?}", value.address_scope),
                    format!("{:?}", value.evidence),
                    format!("{:?}", value.outcome),
                    format!("{:?}", value.submitted),
                    format!("{:?}", value.partial_native_effect),
                    format!("{:?}", value.fault),
                ],
                DiagnosticPayload::Lifecycle(value) => vec![
                    format!("{:?}", value.target),
                    format!("{:?}", value.lifecycle),
                    format!("{:?}", value.fault),
                ],
                DiagnosticPayload::Permission(value) => vec![
                    format!("{:?}", value.permission),
                    format!("{:?}", value.state),
                    format!("{:?}", value.fault),
                ],
            }
        }

        const TEMPLATE_NAME: &str = "private/assets/account-name-template.png";
        const INPUT_TEXT: &str = "caller-secret-input-text";
        const INPUT_KEY: char = '🔐';
        const OCR_TEXT: &str = "caller-secret-recognized-text";
        const OCR_BACKEND: &str = "caller-selected-secret-backend";
        const OCR_MODEL: &str = "caller-selected-secret-model";
        const PID: &str = "1972681003";
        const NATIVE_WINDOW_NUMBER: &str = "1983792004";
        const APP_NAME: &str = "PrivateGameApplication";
        const UNRELATED_FOREGROUND: &str = "UnrelatedForegroundIdentity";
        const WINDOW_TITLE: &str = "Private Account — caller@example.invalid; pid=1972681003; window=1983792004; app=PrivateGameApplication; foreground=UnrelatedForegroundIdentity";
        const SIGNING_IDENTIFIER: &str = "invalid.example.private-game";
        const RAW_AUTHORIZATION_VALUES: &str = "AX=0xA11CE991;CG=0xC0DEC992";
        const PLATFORM_FAILURE: &str = "native failure mentioned caller-home/private.db; signing=invalid.example.private-game; auth=AX=0xA11CE991;CG=0xC0DEC992";

        assert_copy::<DiagnosticPayload>();
        assert_copy::<DiagnosticRecord>();

        let issuer = IdentityIssuer::new();
        let target = issuer
            .issue_target(ProviderId::new("privacy-matrix"))
            .expect("issued");
        let window = TargetDescription::new(
            target,
            WINDOW_TITLE,
            PixelExtent::new(32, 24),
            PixelFormat::Rgba8,
            CoordinateSupport::frame_only(),
        );
        let target = window.id();
        let frame = FrameStamp::new(
            issuer.issue_stream().expect("issued"),
            StreamEpoch::FIRST,
            FrameSequence::FIRST,
            GeometryRevision::FIRST,
        );

        let backend = Arc::new(ControlledMatcher::new(PixelFormat::Rgba8));
        let matcher = Matcher::new(Arc::clone(&backend) as Arc<dyn MatchBackend>);
        let pixel_contents = format!("{:?}", match_fixtures::patch_rgb());
        let source = match_fixtures::planted_template(TEMPLATE_NAME);
        let prepared = matcher
            .prepare(&source, &OperationContext::new())
            .expect("prepared");
        let (sink, _reader) = enabled(DiagnosticLevel::Normal, 1);
        let template = sink.template(&prepared).expect("template identity");

        let request = InputRequest::new(
            target,
            InputSequence::new(vec![
                InputEvent::Text(INPUT_TEXT.to_owned()),
                InputEvent::KeyPress(Key::Character(INPUT_KEY)),
            ])
            .expect("valid caller-sensitive events"),
            DeliveryPlan::require(InputDelivery::ProcessDirected),
        );
        let receipt = InputReceipt::complete(
            target,
            InputDelivery::ProcessDirected,
            SubmissionEvidence::InvocationOnly,
            2,
        );
        let input = InputDiagnostic::from_receipt(&request, &receipt);
        assert!(input.operations.contains(InputOperationKind::Text));
        assert!(input.operations.contains(InputOperationKind::Keyboard));

        let platform_failure = Error::new(Status::CaptureFailed, PLATFORM_FAILURE);
        let region = PixelRect::new(3, 4, 19, 20).expect("valid");
        let matrix = [
            (
                DiagnosticKind::OperationStarted,
                DiagnosticPayload::OperationStarted(OperationStartedDiagnostic {
                    operation: DiagnosticOperationKind::Search,
                }),
            ),
            (
                DiagnosticKind::Frame,
                DiagnosticPayload::Frame(FrameDiagnostic { target, frame }),
            ),
            (
                DiagnosticKind::Mapping,
                DiagnosticPayload::Mapping(MappingDiagnostic {
                    target,
                    frame,
                    source: CoordinateSpace::TargetLogical,
                    destination: CoordinateSpace::CapturePixels,
                }),
            ),
            (
                DiagnosticKind::Search,
                DiagnosticPayload::Search(SearchDiagnostic {
                    target,
                    frame: Some(frame),
                    template,
                    region: Some(region),
                    outcome: SearchDiagnosticOutcome::NoMatch,
                    result_count: 0,
                }),
            ),
            (
                DiagnosticKind::TemplateWatch,
                DiagnosticPayload::TemplateWatch(TemplateWatchDiagnostic {
                    query: TemplateQueryId::new(1).expect("nonzero"),
                    target,
                    frame: Some(frame),
                    region: Some(region),
                    state: TemplateQueryState::Pending,
                    confirmed_observations: 0,
                    confirmed_duration_nanos: 0,
                    disposition: Some(TemplateWorkDisposition::SkippedChange),
                    work: TemplateWorkCounts::default(),
                    pending_count: 0,
                    in_flight_count: 0,
                    session_query_count: 1,
                    engine_query_count: 1,
                    elapsed_nanos: 7,
                    outcome: None,
                }),
            ),
            (
                DiagnosticKind::Ocr,
                DiagnosticPayload::Ocr(OcrDiagnostic {
                    model_instance: DiagnosticOcrModelInstanceId::new(
                        NonZeroU64::new(1).expect("nonzero"),
                    ),
                    profile: OcrDiagnosticProfile::AcceptedG004,
                    source: frame,
                    requested_region: None,
                    effective_region: Some(region),
                    source_envelope: None,
                    output_space: CoordinateSpace::CapturePixels,
                    outcome: OcrDiagnosticOutcome::Recognized,
                    result_count: 1,
                    zone_count: None,
                    unique_candidate_count: None,
                    membership_count: None,
                    result_bytes: None,
                    detector_runs: None,
                    detector_bytes: None,
                    recognizer_runs: None,
                    recognizer_bytes: None,
                    elapsed_nanos: 9,
                    source_pixels: 64,
                }),
            ),
            (DiagnosticKind::Input, DiagnosticPayload::Input(input)),
            (
                DiagnosticKind::RouteAttempt,
                DiagnosticPayload::RouteAttempt(RouteAttemptDiagnostic::from_attempt(
                    target,
                    receipt.attempts()[0],
                )),
            ),
            (
                DiagnosticKind::Lifecycle,
                DiagnosticPayload::Lifecycle(LifecycleDiagnostic {
                    target: Some(target),
                    lifecycle: Lifecycle::Closed,
                    fault: Some(platform_failure.status()),
                }),
            ),
            (
                DiagnosticKind::Permission,
                DiagnosticPayload::Permission(PermissionDiagnostic {
                    permission: PermissionKind::InputControl,
                    state: None,
                    fault: Some(platform_failure.status()),
                }),
            ),
        ];

        assert_eq!(matrix.len(), 10);
        let input_key = INPUT_KEY.to_string();
        let sensitive = [
            TEMPLATE_NAME,
            INPUT_TEXT,
            input_key.as_str(),
            PID,
            NATIVE_WINDOW_NUMBER,
            APP_NAME,
            UNRELATED_FOREGROUND,
            WINDOW_TITLE,
            SIGNING_IDENTIFIER,
            RAW_AUTHORIZATION_VALUES,
            PLATFORM_FAILURE,
            OCR_TEXT,
            OCR_BACKEND,
            OCR_MODEL,
            pixel_contents.as_str(),
        ];
        for (kind, payload) in matrix {
            assert_eq!(payload.kind(), kind);
            for surface in std::iter::once(format!("{payload:?}")).chain(public_fields(payload)) {
                for secret in sensitive {
                    assert!(
                        !surface.contains(secret),
                        "{kind:?} exposed caller-sensitive material `{secret}` in `{surface}`"
                    );
                }
            }
        }
    }

    #[test]
    fn expired_template_metadata_cannot_alias_a_later_preparation() {
        let backend = Arc::new(ControlledMatcher::new(PixelFormat::Rgba8));
        let matcher = Matcher::new(Arc::clone(&backend) as Arc<dyn MatchBackend>);
        let source = match_fixtures::planted_template("reused-template-name");
        let first = matcher
            .prepare(&source, &OperationContext::new())
            .expect("first prepared");
        let first_instance = first.diagnostic_instance();
        let (sink, _reader) = enabled(DiagnosticLevel::Normal, 1);
        let first_identity = sink.template(&first).expect("first identity");
        let clone = first.clone();
        assert_eq!(sink.template(&clone), Some(first_identity));
        drop(clone);
        drop(first);
        assert!(!first_instance.is_live());

        let second = matcher
            .prepare(&source, &OperationContext::new())
            .expect("second prepared");
        let second_instance = second.diagnostic_instance();
        assert_ne!(
            first_instance, second_instance,
            "a retained weak token keeps the stale allocation distinct"
        );

        let second_identity = sink
            .template(&second)
            .expect("a later preparation receives its own identity");
        assert_ne!(second_identity, first_identity);
        let templates = sink
            .stream
            .templates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(templates.len(), 2);
        assert_eq!(templates.get(&second_instance), Some(&second_identity));
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
    fn contended_ocr_diagnostics_do_not_wait_and_report_exact_normal_loss() {
        let (sink, reader) = enabled(DiagnosticLevel::Normal, 2);
        let context = OperationContext::new();
        let operation = observe(&sink, &context);
        let _guard = sink.stream.state.lock().expect("uncontended");
        sink.normal(operation, &context, ocr_payload);
        drop(_guard);

        let batch = batch(&reader);
        assert!(batch.is_empty());
        assert_eq!(batch.losses().normal(), 1);
        assert_eq!(batch.losses().debug(), 0);
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
