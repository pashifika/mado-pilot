//! Bounded template-presence queries over maintained session frames.
//!
//! One engine owns one finite scheduler. Sessions contribute maintained frames;
//! query handles own terminal authority. Capture acquisition, mapping, backend
//! work, and caller waits never run while a scheduler or query lock is held.
//!
//! # Lock order
//!
//! When held together, scheduler admission precedes session activation;
//! activation precedes terminal authority, acquisition state, or registry
//! mutation; terminal authority precedes one query state. Registry and session
//! snapshots release before acquiring activation or query state. A per-query
//! diagnostic-emission lock precedes query state only while copying one payload;
//! state-mutation paths never acquire it. Mapping-cache and worker-wake locks are
//! independent. Capture waits, pixel mapping, change comparison, backend work,
//! clock calls, diagnostic queue emission, waiter notification, and thread
//! teardown run after every state guard is released. This is the enforced order
//! for start, publication, admission, completion, cancellation, and close; no
//! callback or executor surface exists in this module.

use std::cmp::Ordering as CmpOrdering;
use std::fmt;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::diagnostic::{
    DiagnosticOperationKind, DiagnosticPayload, DiagnosticSink, ObservedOperation,
    TemplateWatchDiagnostic, TemplateWatchDiagnosticOutcome,
};
use mado_pilot_capture::{CaptureSession, CpuMapping, Frame, FrameRequest, SessionDescription};
use mado_pilot_core::{
    Error, FrameOrder, FrameStamp, Interruption, MonotonicInstant, Operation, OperationContext,
    PixelRect, Result, Status, TargetId,
};
use mado_pilot_vision::prepared::PreparedTemplateInstance;
use mado_pilot_vision::{
    ChangeDecision, ChangeDetectionPolicy, ChangeDetector, MappedMatch, MatchOptions, MatchRequest,
    MatchResult, Matcher, PreparedTemplate, RegionSelection, TemplateId,
};

const WORKER_WAIT: Duration = Duration::from_millis(10);
const WAIT_POLL: Duration = Duration::from_millis(10);
const MAX_ENGINE_QUERIES: usize = 256;
const MAX_ACTIVE_SESSIONS: usize = 16;
const MAX_SESSION_QUERIES: usize = 64;
const MAX_IN_FLIGHT_ANALYSES: usize = 2;
const MAPPED_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_MAPPED_CACHE_ENTRIES: usize = 256;
const ELIGIBLE_QUEUE_EXPIRY: Duration = Duration::from_secs(30);

/// Immutable fixed limits and behavior of the engine template scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemplateSchedulerDescriptor {
    max_engine_queries: u32,
    max_active_sessions: u32,
    max_session_queries: u32,
    max_in_flight_analyses: u32,
    latest_pending_frames_per_query: u32,
    mapped_cache_bytes: u64,
    max_mapped_cache_entries: u32,
    eligible_queue_expiry: Duration,
}

impl TemplateSchedulerDescriptor {
    /// Returns the selected finite scheduler contract.
    #[must_use]
    pub const fn selected_default() -> Self {
        Self {
            max_engine_queries: 256,
            max_active_sessions: 16,
            max_session_queries: 64,
            max_in_flight_analyses: 2,
            latest_pending_frames_per_query: 1,
            mapped_cache_bytes: 64 * 1024 * 1024,
            max_mapped_cache_entries: 256,
            eligible_queue_expiry: ELIGIBLE_QUEUE_EXPIRY,
        }
    }

    /// Returns the maximum live queries owned by one engine.
    #[must_use]
    pub const fn max_engine_queries(self) -> u32 {
        self.max_engine_queries
    }

    /// Returns the maximum sessions holding live watcher-work reservations.
    #[must_use]
    pub const fn max_active_sessions(self) -> u32 {
        self.max_active_sessions
    }

    /// Returns the maximum live queries owned by one session.
    #[must_use]
    pub const fn max_session_queries(self) -> u32 {
        self.max_session_queries
    }

    /// Returns the fixed engine-wide backend concurrency.
    #[must_use]
    pub const fn max_in_flight_analyses(self) -> u32 {
        self.max_in_flight_analyses
    }

    /// Returns the latest-wins pending-frame capacity of one query.
    #[must_use]
    pub const fn latest_pending_frames_per_query(self) -> u32 {
        self.latest_pending_frames_per_query
    }

    /// Returns the maximum number of retained mapped-region entries.
    #[must_use]
    pub const fn max_mapped_cache_entries(self) -> u32 {
        self.max_mapped_cache_entries
    }

    /// Returns the shared mapped-region cache ceiling.
    #[must_use]
    pub const fn mapped_cache_bytes(self) -> u64 {
        self.mapped_cache_bytes
    }

    /// Returns how long eligible work may wait before typed overload.
    #[must_use]
    pub const fn eligible_queue_expiry(self) -> Duration {
        self.eligible_queue_expiry
    }
}

impl Default for TemplateSchedulerDescriptor {
    fn default() -> Self {
        Self::selected_default()
    }
}

/// The supported maximum backend-analysis rate for one query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TemplateAnalysisRate {
    minimum_interval: Duration,
}

impl TemplateAnalysisRate {
    /// Allows every accepted frame transition to enter analysis.
    #[must_use]
    pub const fn unrestricted() -> Self {
        Self {
            minimum_interval: Duration::ZERO,
        }
    }

    /// Limits analysis to at most one admission per `minimum_interval`.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] for a zero interval; use
    /// [`Self::unrestricted`] to request no rate limit.
    pub fn at_most_every(minimum_interval: Duration) -> Result<Self> {
        if minimum_interval.is_zero() {
            return Err(Error::new(
                Status::InvalidArgument,
                "a bounded template analysis interval must be nonzero",
            ));
        }
        Ok(Self { minimum_interval })
    }

    /// Returns the minimum duration between admitted analyses.
    #[must_use]
    pub const fn minimum_interval(self) -> Duration {
        self.minimum_interval
    }
}

/// The closed kind of confirmed template-presence stability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TemplateStabilityKind {
    /// Complete on the first confirmed match.
    Immediate,
    /// Require a fixed number of confirmed consecutive matches.
    Consecutive,
    /// Require confirmed matches spanning a minimum duration.
    Duration,
}

/// A validated confirmed-only stability requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemplateStability {
    kind: TemplateStabilityKind,
    observations: u32,
    duration: Duration,
}

impl TemplateStability {
    /// Completes on the first confirmed match.
    #[must_use]
    pub const fn immediate() -> Self {
        Self {
            kind: TemplateStabilityKind::Immediate,
            observations: 1,
            duration: Duration::ZERO,
        }
    }

    /// Requires `observations` confirmed consecutive matches.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] when `observations` is zero.
    pub fn consecutive(observations: u32) -> Result<Self> {
        let observations = NonZeroU32::new(observations).ok_or_else(|| {
            Error::new(
                Status::InvalidArgument,
                "template stability observations must be nonzero",
            )
        })?;
        Ok(Self {
            kind: TemplateStabilityKind::Consecutive,
            observations: observations.get(),
            duration: Duration::ZERO,
        })
    }

    /// Requires confirmed matches spanning `duration`.
    ///
    /// # Errors
    ///
    /// Returns [`Status::InvalidArgument`] when `duration` is zero.
    pub fn duration(duration: Duration) -> Result<Self> {
        if duration.is_zero() {
            return Err(Error::new(
                Status::InvalidArgument,
                "template stability duration must be nonzero",
            ));
        }
        Ok(Self {
            kind: TemplateStabilityKind::Duration,
            observations: 0,
            duration,
        })
    }

    /// Returns the selected stability kind.
    #[must_use]
    pub const fn kind(self) -> TemplateStabilityKind {
        self.kind
    }

    /// Returns the required confirmed observations when applicable.
    #[must_use]
    pub const fn required_observations(self) -> Option<NonZeroU32> {
        if matches!(self.kind, TemplateStabilityKind::Consecutive) {
            NonZeroU32::new(self.observations)
        } else {
            None
        }
    }

    /// Returns the required confirmed duration when applicable.
    #[must_use]
    pub const fn required_duration(self) -> Option<Duration> {
        if matches!(self.kind, TemplateStabilityKind::Duration) {
            Some(self.duration)
        } else {
            None
        }
    }
}

impl Default for TemplateStability {
    fn default() -> Self {
        Self::immediate()
    }
}

/// One explicit template-presence watcher request.
#[derive(Debug, Clone)]
pub struct TemplateWatchRequest {
    template: PreparedTemplate,
    region: RegionSelection,
    options: MatchOptions,
    rate: TemplateAnalysisRate,
    stability: TemplateStability,
    change_policy: ChangeDetectionPolicy,
    operation: OperationContext,
}

impl TemplateWatchRequest {
    /// Builds a watcher over one prepared template and query-lifetime authority.
    #[must_use]
    pub fn new(
        template: PreparedTemplate,
        options: MatchOptions,
        operation: OperationContext,
    ) -> Self {
        Self {
            template,
            region: RegionSelection::FullFrame,
            options,
            rate: TemplateAnalysisRate::unrestricted(),
            stability: TemplateStability::immediate(),
            change_policy: ChangeDetectionPolicy::default(),
            operation,
        }
    }

    /// Replaces the coordinate-qualified search-region policy.
    #[must_use]
    pub const fn with_region(mut self, region: RegionSelection) -> Self {
        self.region = region;
        self
    }

    /// Replaces the maximum analysis rate.
    #[must_use]
    pub const fn with_rate(mut self, rate: TemplateAnalysisRate) -> Self {
        self.rate = rate;
        self
    }

    /// Replaces the confirmed-only stability requirement.
    #[must_use]
    pub const fn with_stability(mut self, stability: TemplateStability) -> Self {
        self.stability = stability;
        self
    }

    /// Replaces the closed change-detection policy.
    #[must_use]
    pub const fn with_change_policy(mut self, policy: ChangeDetectionPolicy) -> Self {
        self.change_policy = policy;
        self
    }

    /// Returns the owned prepared template.
    #[must_use]
    pub const fn template(&self) -> &PreparedTemplate {
        &self.template
    }

    /// Returns the coordinate-qualified search selection.
    #[must_use]
    pub const fn region(&self) -> RegionSelection {
        self.region
    }

    /// Returns the validated matching options.
    #[must_use]
    pub const fn options(&self) -> MatchOptions {
        self.options
    }

    /// Returns the maximum analysis rate.
    #[must_use]
    pub const fn rate(&self) -> TemplateAnalysisRate {
        self.rate
    }

    /// Returns the confirmed-only stability requirement.
    #[must_use]
    pub const fn stability(&self) -> TemplateStability {
        self.stability
    }

    /// Returns the closed change policy.
    #[must_use]
    pub const fn change_policy(&self) -> ChangeDetectionPolicy {
        self.change_policy
    }

    /// Returns the query-lifetime operation context.
    #[must_use]
    pub const fn operation(&self) -> &OperationContext {
        &self.operation
    }
}

/// One engine-local nonzero template-query identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TemplateQueryId(std::num::NonZeroU64);

impl TemplateQueryId {
    pub(crate) fn new(value: u64) -> Option<Self> {
        std::num::NonZeroU64::new(value).map(Self)
    }

    /// Returns the nonzero engine-local value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Every bounded watcher work disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TemplateWorkDisposition {
    /// Backend analysis was admitted.
    Admitted = 0,
    /// Exact compatible pixels permitted routine analysis to be skipped.
    SkippedChange = 1,
    /// Analysis remained pending until its rate interval elapsed.
    DeferredRate = 2,
    /// An exact immutable analysis was shared with another query.
    Coalesced = 3,
    /// Newer work or authority displaced this work.
    Superseded = 4,
    /// Work was refused before backend admission.
    Rejected = 5,
    /// Eligible work exceeded the fixed queue residence bound.
    QueueExpired = 6,
    /// Backend work completed successfully, including a no-match.
    Completed = 7,
    /// Mapping or backend work failed.
    Failed = 8,
}

impl TemplateWorkDisposition {
    const COUNT: usize = 9;

    const fn index(self) -> usize {
        self as usize
    }
}

/// Saturating per-query counts for every work disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemplateWorkCounts {
    counts: [u64; TemplateWorkDisposition::COUNT],
}

impl TemplateWorkCounts {
    const fn new() -> Self {
        Self {
            counts: [0; TemplateWorkDisposition::COUNT],
        }
    }

    fn increment(&mut self, disposition: TemplateWorkDisposition) {
        let count = &mut self.counts[disposition.index()];
        *count = count.saturating_add(1);
    }

    /// Returns the count for `disposition`.
    #[must_use]
    pub const fn get(self, disposition: TemplateWorkDisposition) -> u64 {
        self.counts[disposition.index()]
    }
}

impl Default for TemplateWorkCounts {
    fn default() -> Self {
        Self::new()
    }
}

/// A finite scheduler condition that made a query unsatisfiable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TemplateOverload {
    /// Eligible work exceeded the fixed residence bound.
    QueueExpired,
}

/// The immutable successful presence result.
#[derive(Debug, Clone)]
pub struct TemplateWatchResult {
    template: TemplateId,
    target: TargetId,
    frame: Frame,
    result: Arc<MatchResult>,
    confirmed_observations: u32,
    confirmed_duration: Duration,
}

impl TemplateWatchResult {
    /// Returns the target observed by the exact source frame.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Returns the prepared template's public identity.
    #[must_use]
    pub const fn template(&self) -> &TemplateId {
        &self.template
    }

    /// Returns the exact source frame retained by the result.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Returns the immutable backend/options/region/match facts.
    #[must_use]
    pub fn result(&self) -> &MatchResult {
        &self.result
    }

    /// Returns the confirmed consecutive observation count at completion.
    #[must_use]
    pub const fn confirmed_observations(&self) -> u32 {
        self.confirmed_observations
    }

    /// Returns the confirmed match span at completion.
    #[must_use]
    pub const fn confirmed_duration(&self) -> Duration {
        self.confirmed_duration
    }
}

/// One immutable authoritative terminal query outcome.
#[derive(Debug, Clone)]
// The complete enum is retained behind one `Arc`; boxing only the matched
// variant would add an allocation without reducing copies or retained bytes.
#[allow(clippy::large_enum_variant)]
#[non_exhaustive]
pub enum TemplateTerminalOutcome {
    /// The selected stability rule completed on an exact source frame.
    Matched(TemplateWatchResult),
    /// Explicit or query-context cancellation won.
    Cancelled,
    /// The query-lifetime deadline won.
    DeadlineExceeded,
    /// The owning session began closing.
    SessionClosed,
    /// The engine scheduler began closing.
    SchedulerClosed,
    /// The capture target was lost.
    TargetLost,
    /// Finite scheduler policy could no longer satisfy the query.
    Overloaded(TemplateOverload),
    /// Capture, mapping, or backend work failed.
    Failed(Error),
}

impl TemplateTerminalOutcome {
    /// Returns the shared failure status, or `None` for a successful match.
    #[must_use]
    pub const fn status(&self) -> Option<Status> {
        match self {
            Self::Matched(_) => None,
            Self::Cancelled => Some(Status::Cancelled),
            Self::DeadlineExceeded => Some(Status::DeadlineExceeded),
            Self::SessionClosed | Self::SchedulerClosed => Some(Status::Closed),
            Self::TargetLost => Some(Status::TargetLost),
            Self::Overloaded(_) => Some(Status::LimitExceeded),
            Self::Failed(error) => Some(error.status()),
        }
    }

    /// Reports whether this terminal outcome contains a successful match.
    #[must_use]
    pub const fn is_match(&self) -> bool {
        matches!(self, Self::Matched(_))
    }
}

/// Coarse immutable query lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TemplateQueryState {
    /// Waiting for a frame, rate eligibility, or backend completion.
    Pending,
    /// One terminal outcome has committed.
    Terminal,
}

/// One immutable non-terminal query snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemplateQueryProgress {
    id: TemplateQueryId,
    last_frame: Option<FrameStamp>,
    generation: u64,
    confirmed_observations: u32,
    confirmed_duration: Duration,
    work: TemplateWorkCounts,
    pending_count: u32,
    in_flight_count: u32,
}

impl TemplateQueryProgress {
    /// Returns the query identity.
    #[must_use]
    pub const fn id(self) -> TemplateQueryId {
        self.id
    }

    /// Returns the newest frame transition considered by this query.
    #[must_use]
    pub const fn last_frame(self) -> Option<FrameStamp> {
        self.last_frame
    }

    /// Returns the latest admitted analysis generation.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// Returns the current confirmed consecutive-match count.
    #[must_use]
    pub const fn confirmed_observations(self) -> u32 {
        self.confirmed_observations
    }

    /// Returns the current confirmed match span.
    #[must_use]
    pub const fn confirmed_duration(self) -> Duration {
        self.confirmed_duration
    }

    /// Returns the bounded work-disposition counters.
    #[must_use]
    pub const fn work(self) -> TemplateWorkCounts {
        self.work
    }

    /// Returns the latest pending-frame depth, always zero or one.
    #[must_use]
    pub const fn pending_count(self) -> u32 {
        self.pending_count
    }

    /// Returns the current backend analysis depth.
    #[must_use]
    pub const fn in_flight_count(self) -> u32 {
        self.in_flight_count
    }

    /// Reports whether one latest frame is waiting for consideration.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        self.pending_count != 0
    }

    /// Reports whether backend analysis is in flight.
    #[must_use]
    pub const fn is_in_flight(self) -> bool {
        self.in_flight_count != 0
    }
}

/// The result of one non-blocking query poll.
#[derive(Debug, Clone)]
pub enum TemplateQueryOutcome {
    /// The query remains live.
    Pending(TemplateQueryProgress),
    /// The query has one immutable terminal result.
    Terminal(Arc<TemplateTerminalOutcome>),
}

impl TemplateQueryOutcome {
    /// Returns the coarse lifecycle state.
    #[must_use]
    pub const fn state(&self) -> TemplateQueryState {
        match self {
            Self::Pending(_) => TemplateQueryState::Pending,
            Self::Terminal(_) => TemplateQueryState::Terminal,
        }
    }
}

/// One owning template-query handle.
///
/// Dropping the handle performs the same idempotent cancellation as
/// [`Self::cancel`]. Completed outcomes remain readable until the handle drops.
pub struct TemplateQuery {
    shared: Arc<QueryShared>,
}

impl TemplateQuery {
    /// Returns the engine-local query identity.
    #[must_use]
    pub fn id(&self) -> TemplateQueryId {
        self.shared.id
    }

    /// Returns an immutable snapshot without blocking.
    #[must_use]
    pub fn poll(&self) -> TemplateQueryOutcome {
        self.shared.snapshot()
    }

    /// Waits for the query under a separate caller-wait operation context.
    ///
    /// A wait deadline or cancellation ends only this call. It cannot mutate the
    /// query's own authority or terminal outcome.
    ///
    /// # Errors
    ///
    /// Returns only the caller-wait interruption.
    pub fn wait(&self, wait: &OperationContext) -> Result<Arc<TemplateTerminalOutcome>> {
        loop {
            if let Some(interruption) = wait.interruption() {
                return Err(interruption.into());
            }
            if let Some(outcome) = self.shared.terminal() {
                return Ok(outcome);
            }
            let duration = wait
                .remaining()
                .map_or(WAIT_POLL, |remaining| remaining.min(WAIT_POLL));
            if duration.is_zero() {
                continue;
            }
            let state = lock(&self.shared.state);
            if let Some(outcome) = &state.terminal {
                return Ok(Arc::clone(outcome));
            }
            let _ = self
                .shared
                .changed
                .wait_timeout(state, duration)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Competes explicit cancellation at the single terminal commit seam.
    ///
    /// Repeated calls return the already-authoritative outcome.
    #[must_use]
    pub fn cancel(&self) -> Arc<TemplateTerminalOutcome> {
        self.shared.terminate(TemplateTerminalOutcome::Cancelled).0
    }
}

impl fmt::Debug for TemplateQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TemplateQuery")
            .field("id", &self.id())
            .field("state", &self.poll().state())
            .finish()
    }
}

impl Drop for TemplateQuery {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

#[derive(Debug)]
struct QueryShared {
    id: TemplateQueryId,
    target: TargetId,
    session: Weak<WatchSession>,
    scheduler: Weak<WatchScheduler>,
    template: PreparedTemplate,
    template_instance: PreparedTemplateInstance,
    region: RegionSelection,
    options: MatchOptions,
    rate: TemplateAnalysisRate,
    stability: TemplateStability,
    change_policy: ChangeDetectionPolicy,
    operation: OperationContext,
    diagnostic: Option<ObservedOperation>,
    diagnostic_emission: Mutex<()>,
    started_at: MonotonicInstant,
    state: Mutex<QueryData>,
    changed: Condvar,
    reservation_released: AtomicBool,
}

#[derive(Debug)]
struct QueryData {
    terminal: Option<Arc<TemplateTerminalOutcome>>,
    needs_current: bool,
    pending: Option<PendingFrame>,
    processing: Option<FrameStamp>,
    in_flight: [Option<(u64, FrameStamp)>; MAX_IN_FLIGHT_ANALYSES],
    source_frame: Option<FrameStamp>,
    last_frame: Option<FrameStamp>,
    terminal_frame: Option<FrameStamp>,
    effective_region: Option<PixelRect>,
    previous_mapping: Option<Weak<CpuMapping>>,
    generation: u64,
    last_admitted: Option<MonotonicInstant>,
    rate_eligible_at: Option<MonotonicInstant>,
    stability: StabilityState,
    work: TemplateWorkCounts,
}

#[derive(Debug, Clone)]
struct PendingFrame {
    frame: Frame,
    eligible_since: MonotonicInstant,
}

#[derive(Debug, Clone, Copy, Default)]
struct StabilityState {
    observations: u32,
    started: Option<MonotonicInstant>,
    duration: Duration,
}

#[derive(Debug, Clone, Copy)]
struct CandidateSnapshot {
    stamp: FrameStamp,
    eligible_since: MonotonicInstant,
    rate_eligible_at: Option<MonotonicInstant>,
}

#[derive(Debug)]
struct ClaimedWork {
    query: Arc<QueryShared>,
    frame: Frame,
    peers: Vec<(Arc<QueryShared>, Frame)>,
}

#[derive(Debug)]
struct Participant {
    query: Arc<QueryShared>,
    generation: u64,
    disposition_recorded: bool,
}

#[derive(Debug, Clone, Copy)]
struct RejectedDiagnostic {
    observed: Option<ObservedOperation>,
    query: TemplateQueryId,
    target: TargetId,
    started_at: MonotonicInstant,
    session_query_count: usize,
    outcome: TemplateWatchDiagnosticOutcome,
}
#[derive(Debug, Clone)]
enum ImmutableAnalysisResult {
    Empty,
    Shared(Arc<MatchResult>),
}

impl ImmutableAnalysisResult {
    fn new(result: MatchResult) -> Self {
        if result.is_empty() {
            Self::Empty
        } else {
            Self::Shared(Arc::new(result))
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    fn into_shared(self) -> Option<Arc<MatchResult>> {
        match self {
            Self::Empty => None,
            Self::Shared(result) => Some(result),
        }
    }
}

#[derive(Debug)]
enum Admission {
    Admitted(u64),
    Skipped,
    Deferred,
    Obsolete,
}

impl QueryShared {
    fn snapshot(&self) -> TemplateQueryOutcome {
        let state = lock(&self.state);
        match &state.terminal {
            Some(outcome) => TemplateQueryOutcome::Terminal(Arc::clone(outcome)),
            None => TemplateQueryOutcome::Pending(TemplateQueryProgress {
                id: self.id,
                last_frame: state.last_frame,
                generation: state.generation,
                confirmed_observations: state.stability.observations,
                confirmed_duration: state.stability.duration,
                work: state.work,
                pending_count: u32::from(state.pending.is_some()),
                in_flight_count: in_flight_count(&state),
            }),
        }
    }

    fn terminal(&self) -> Option<Arc<TemplateTerminalOutcome>> {
        lock(&self.state).terminal.clone()
    }

    fn emit(&self, disposition: Option<TemplateWorkDisposition>, terminal: bool) {
        let Some(observed) = self.diagnostic else {
            return;
        };
        let Some(scheduler) = self.scheduler.upgrade() else {
            return;
        };
        let Some(diagnostics) = scheduler.diagnostics.as_ref() else {
            return;
        };
        if !terminal && !diagnostics.admits_debug() {
            return;
        }
        let now = self.operation.now();
        let _emission = lock(&self.diagnostic_emission);
        let session_query_count = self
            .session
            .upgrade()
            .map_or(0, |session| session.active_queries.load(Ordering::Acquire));
        let engine_query_count = scheduler.query_count.load(Ordering::Acquire);
        let payload = {
            let state = lock(&self.state);
            if !terminal && state.terminal.is_some() {
                return;
            }
            let frame = query_frame(&state);
            TemplateWatchDiagnostic {
                query: self.id,
                target: self.target,
                frame,
                region: state.effective_region,
                state: if state.terminal.is_some() {
                    TemplateQueryState::Terminal
                } else {
                    TemplateQueryState::Pending
                },
                confirmed_observations: state.stability.observations,
                confirmed_duration_nanos: duration_nanos(state.stability.duration),
                disposition,
                work: state.work,
                pending_count: u32::from(state.pending.is_some()),
                in_flight_count: in_flight_count(&state),
                session_query_count: u32::try_from(session_query_count).unwrap_or(u32::MAX),
                engine_query_count: u32::try_from(engine_query_count).unwrap_or(u32::MAX),
                elapsed_nanos: duration_nanos(now.saturating_duration_since(self.started_at)),
                outcome: state
                    .terminal
                    .as_deref()
                    .map(template_watch_diagnostic_outcome),
            }
        };
        if terminal {
            diagnostics.normal_at(observed, now, || DiagnosticPayload::TemplateWatch(payload));
        } else {
            diagnostics.debug_at(observed, now, || DiagnosticPayload::TemplateWatch(payload));
        }
    }

    fn record_disposition(&self, disposition: TemplateWorkDisposition) -> bool {
        let recorded = {
            let mut state = lock(&self.state);
            if state.terminal.is_some() {
                false
            } else {
                state.work.increment(disposition);
                true
            }
        };
        if recorded {
            self.emit(Some(disposition), false);
        }
        recorded
    }

    fn after_terminal_commit(&self) {
        let session = self.session.upgrade();
        self.release_reservation();
        self.emit(None, true);
        self.changed.notify_all();
        if let Some(session) = session {
            session.unregister_query(self.id);
            session.notify_progress();
        }
    }

    fn terminate(
        self: &Arc<Self>,
        outcome: TemplateTerminalOutcome,
    ) -> (Arc<TemplateTerminalOutcome>, bool) {
        let scheduler = self.scheduler.upgrade();
        let scheduler_authority = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let session = self.session.upgrade();
        let session_authority = session.as_ref().map(|session| lock(&session.activation));
        let outcome = session
            .as_ref()
            .and_then(|session| lock(&session.terminal_authority).clone())
            .or_else(|| {
                scheduler
                    .as_ref()
                    .is_none_or(|scheduler| scheduler.closed.load(Ordering::Acquire))
                    .then_some(TemplateTerminalOutcome::SchedulerClosed)
            })
            .unwrap_or(outcome);
        let (outcome, committed) = {
            let mut state = lock(&self.state);
            if let Some(existing) = &state.terminal {
                (Arc::clone(existing), false)
            } else {
                state.terminal_frame = query_frame(&state);
                if state.pending.take().is_some() || state.processing.take().is_some() {
                    state.work.increment(TemplateWorkDisposition::Superseded);
                }
                let in_flight = in_flight_count(&state);
                state.in_flight.fill(None);
                for _ in 0..in_flight {
                    state.work.increment(TemplateWorkDisposition::Superseded);
                }
                let outcome = Arc::new(outcome);
                state.terminal = Some(Arc::clone(&outcome));
                (outcome, true)
            }
        };
        drop(session_authority);
        drop(scheduler_authority);
        if committed {
            self.after_terminal_commit();
        }
        (outcome, committed)
    }

    fn release_reservation(&self) {
        if self.reservation_released.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(scheduler) = self.scheduler.upgrade() {
            scheduler.release_query_reservation();
        }
        if let Some(session) = self.session.upgrade() {
            session.release_query_slot();
        }
    }

    fn needs_current(&self) -> bool {
        let state = lock(&self.state);
        state.terminal.is_none() && state.needs_current
    }

    fn enqueue(self: &Arc<Self>, frame: Frame, now: MonotonicInstant) {
        let stamp = frame.stamp();
        let mut state = lock(&self.state);
        if state.terminal.is_some() || stamp.stream() != self.session_stream() {
            return;
        }
        if state
            .source_frame
            .is_some_and(|last| !matches!(last.order(&stamp), Ok(FrameOrder::Before)))
        {
            return;
        }
        state.needs_current = false;
        let incompatible = state.source_frame.is_some_and(|last| {
            last.epoch() != stamp.epoch() || last.geometry() != stamp.geometry()
        });
        state.source_frame = Some(stamp);
        let mut superseded_count = 0;
        if incompatible {
            state.previous_mapping = None;
            state.stability = StabilityState::default();
            for in_flight in &mut state.in_flight {
                if in_flight.take().is_some() {
                    superseded_count += 1;
                }
            }
        }
        let superseded = state
            .pending
            .replace(PendingFrame {
                frame,
                eligible_since: now,
            })
            .is_some();
        if superseded {
            superseded_count += 1;
        }
        for _ in 0..superseded_count {
            state.work.increment(TemplateWorkDisposition::Superseded);
        }
        state.rate_eligible_at = None;
        drop(state);
        if superseded_count != 0 {
            self.emit(Some(TemplateWorkDisposition::Superseded), false);
        }
        self.changed.notify_all();
    }

    fn session_stream(&self) -> mado_pilot_core::StreamId {
        self.session.upgrade().map_or_else(
            || panic!("live query lost its session"),
            |session| session.description.stream(),
        )
    }

    fn analysis_capacity_reached(&self, state: &QueryData) -> bool {
        let limit = if self.stability.kind() == TemplateStabilityKind::Immediate {
            MAX_IN_FLIGHT_ANALYSES
        } else {
            1
        };
        state.in_flight.iter().flatten().count() >= limit
    }

    fn has_admitted_or_considered(&self, stamp: FrameStamp) -> bool {
        let state = lock(&self.state);
        state.terminal.is_some()
            || state
                .in_flight
                .iter()
                .flatten()
                .any(|(_, frame)| *frame == stamp)
            || (state.rate_eligible_at.is_some()
                && state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.frame.stamp() == stamp))
            || (self.analysis_capacity_reached(&state)
                && state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.frame.stamp() == stamp))
            || state
                .last_frame
                .is_some_and(|last| !matches!(last.order(&stamp), Ok(FrameOrder::Before)))
    }

    fn candidate(&self) -> Option<CandidateSnapshot> {
        let state = lock(&self.state);
        if state.terminal.is_some()
            || state.processing.is_some()
            || self.analysis_capacity_reached(&state)
        {
            return None;
        }
        state.pending.as_ref().map(|pending| CandidateSnapshot {
            stamp: pending.frame.stamp(),
            eligible_since: pending.eligible_since,
            rate_eligible_at: state.rate_eligible_at,
        })
    }

    fn expiry_candidate(&self) -> Option<CandidateSnapshot> {
        let state = lock(&self.state);
        if state.terminal.is_some() {
            return None;
        }
        let pending = state.pending.as_ref()?;
        if state.processing == Some(pending.frame.stamp()) {
            return None;
        }
        Some(CandidateSnapshot {
            stamp: pending.frame.stamp(),
            eligible_since: pending.eligible_since,
            rate_eligible_at: state.rate_eligible_at,
        })
    }

    fn expire_if_current(
        self: &Arc<Self>,
        candidate: CandidateSnapshot,
        now: MonotonicInstant,
        expiry: Duration,
    ) -> bool {
        let scheduler = self.scheduler.upgrade();
        let scheduler_authority = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let session = self.session.upgrade();
        let session_authority = session.as_ref().map(|session| lock(&session.activation));
        let authority = session
            .as_ref()
            .and_then(|session| lock(&session.terminal_authority).clone())
            .or_else(|| {
                scheduler
                    .as_ref()
                    .is_none_or(|scheduler| scheduler.closed.load(Ordering::Acquire))
                    .then_some(TemplateTerminalOutcome::SchedulerClosed)
            });
        if let Some(outcome) = authority {
            drop(session_authority);
            drop(scheduler_authority);
            self.terminate(outcome);
            return false;
        }
        let committed = {
            let mut state = lock(&self.state);
            let current = state.pending.as_ref().is_some_and(|pending| {
                pending.frame.stamp() == candidate.stamp
                    && pending.eligible_since == candidate.eligible_since
            });
            if state.terminal.is_some()
                || !current
                || state.processing == Some(candidate.stamp)
                || state.rate_eligible_at != candidate.rate_eligible_at
                || candidate
                    .rate_eligible_at
                    .is_some_and(|eligible| now < eligible)
                || now.saturating_duration_since(
                    candidate
                        .rate_eligible_at
                        .unwrap_or(candidate.eligible_since),
                ) <= expiry
            {
                false
            } else {
                state.work.increment(TemplateWorkDisposition::QueueExpired);
                state.pending = None;
                state.processing = None;
                state.terminal_frame = Some(candidate.stamp);
                let in_flight = in_flight_count(&state);
                state.in_flight.fill(None);
                for _ in 0..in_flight {
                    state.work.increment(TemplateWorkDisposition::Superseded);
                }
                state.terminal = Some(Arc::new(TemplateTerminalOutcome::Overloaded(
                    TemplateOverload::QueueExpired,
                )));
                true
            }
        };
        drop(session_authority);
        drop(scheduler_authority);
        if committed {
            self.emit(Some(TemplateWorkDisposition::QueueExpired), false);
            self.after_terminal_commit();
        }
        committed
    }

    fn is_idle(&self) -> bool {
        let state = lock(&self.state);
        state.terminal.is_none()
            && state.pending.is_none()
            && state.processing.is_none()
            && state.in_flight.iter().all(Option::is_none)
    }

    fn finish_source_end_if_idle(self: &Arc<Self>) -> bool {
        if self
            .session
            .upgrade()
            .is_some_and(|session| session.source_ended.load(Ordering::Acquire))
            && self.is_idle()
        {
            self.terminate(TemplateTerminalOutcome::SessionClosed);
            true
        } else {
            false
        }
    }

    fn claim(self: &Arc<Self>, stamp: FrameStamp) -> Option<ClaimedWork> {
        let frame = {
            let mut state = lock(&self.state);
            if state.terminal.is_some()
                || state.processing.is_some()
                || self.analysis_capacity_reached(&state)
            {
                return None;
            }
            let pending = state.pending.as_ref()?;
            if pending.frame.stamp() != stamp {
                return None;
            }
            let frame = pending.frame.clone();
            state.processing = Some(stamp);
            frame
        };
        if let Some(session) = self.session.upgrade() {
            session.notify_progress();
        }
        Some(ClaimedWork {
            query: Arc::clone(self),
            frame,
            peers: Vec::new(),
        })
    }

    fn release_claim(&self, stamp: FrameStamp) {
        let mut state = lock(&self.state);
        if state.processing == Some(stamp) {
            state.processing = None;
        }
    }

    fn next_generation(&self) -> Option<u64> {
        let state = lock(&self.state);
        if state.terminal.is_some()
            || state.processing.is_some()
            || self.analysis_capacity_reached(&state)
        {
            None
        } else {
            state.generation.checked_add(1)
        }
    }

    fn mapping_decision(&self, mapped: &MappedMatch) -> ChangeDecision {
        let (previous, stability_started) = {
            let state = lock(&self.state);
            (
                state.previous_mapping.as_ref().and_then(Weak::upgrade),
                state.stability.observations != 0,
            )
        };
        if stability_started {
            return ChangeDecision::AnalysisRequired;
        }
        let Some(previous) = previous else {
            return ChangeDecision::AnalysisRequired;
        };
        let previous = previous.as_ref();
        let Some(current) = mapped.pixels() else {
            return ChangeDecision::AnalysisRequired;
        };
        ChangeDetector::new(self.change_policy).compare(previous, current)
    }

    fn finish_mapping(
        self: &Arc<Self>,
        mapped: &MappedMatch,
        mapping: Option<Weak<CpuMapping>>,
        decision: ChangeDecision,
        now: MonotonicInstant,
    ) -> Admission {
        let stamp = mapped.frame().stamp();
        let Some(scheduler) = self.scheduler.upgrade() else {
            return Admission::Obsolete;
        };
        let scheduler_authority = lock(&scheduler.admission);
        let Some(session) = self.session.upgrade() else {
            return Admission::Obsolete;
        };
        let authority = lock(&session.activation);
        let closed = lock(&session.terminal_authority)
            .clone()
            .or_else(|| {
                session
                    .closed
                    .load(Ordering::Acquire)
                    .then_some(TemplateTerminalOutcome::SessionClosed)
            })
            .or_else(|| {
                scheduler
                    .closed
                    .load(Ordering::Acquire)
                    .then_some(TemplateTerminalOutcome::SchedulerClosed)
            });
        if let Some(outcome) = closed {
            drop(authority);
            drop(scheduler_authority);
            self.terminate(outcome);
            return Admission::Obsolete;
        }
        let mut state = lock(&self.state);
        if state.terminal.is_some() || state.processing != Some(stamp) {
            return Admission::Obsolete;
        }
        if state.pending.as_ref().map(|pending| pending.frame.stamp()) != Some(stamp) {
            state.processing = None;
            return Admission::Obsolete;
        }
        state.effective_region = Some(mapped.searched());
        if decision == ChangeDecision::Unchanged && state.stability.observations == 0 {
            state.pending = None;
            state.processing = None;
            state.last_frame = Some(stamp);
            state.previous_mapping = mapping;
            state.rate_eligible_at = None;
            state.work.increment(TemplateWorkDisposition::SkippedChange);
            drop(state);
            drop(authority);
            drop(scheduler_authority);
            self.emit(Some(TemplateWorkDisposition::SkippedChange), false);
            self.changed.notify_all();
            if let Some(session) = self.session.upgrade() {
                session.notify_progress();
            }
            self.finish_source_end_if_idle();
            return Admission::Skipped;
        }
        let interval = self.rate.minimum_interval();
        if !interval.is_zero()
            && state
                .last_admitted
                .is_some_and(|last| now.saturating_duration_since(last) < interval)
        {
            let Some(eligible) = state
                .last_admitted
                .and_then(|last| last.checked_add(interval))
            else {
                state.processing = None;
                state.work.increment(TemplateWorkDisposition::Failed);
                drop(state);
                drop(authority);
                drop(scheduler_authority);
                self.emit(Some(TemplateWorkDisposition::Failed), false);
                self.terminate(TemplateTerminalOutcome::Failed(Error::new(
                    Status::InvalidArgument,
                    "template analysis interval is not representable in the query clock domain",
                )));
                return Admission::Obsolete;
            };
            let newly_deferred = state.rate_eligible_at != Some(eligible);
            if newly_deferred {
                state.work.increment(TemplateWorkDisposition::DeferredRate);
            }
            state.rate_eligible_at = Some(eligible);
            if let Some(pending) = state.pending.as_mut() {
                pending.eligible_since = eligible;
            }
            state.processing = None;
            drop(state);
            drop(authority);
            drop(scheduler_authority);
            if newly_deferred {
                self.emit(Some(TemplateWorkDisposition::DeferredRate), false);
            }
            self.changed.notify_all();
            return Admission::Deferred;
        }
        let Some(generation) = state.generation.checked_add(1) else {
            drop(state);
            drop(authority);
            drop(scheduler_authority);
            self.terminate(TemplateTerminalOutcome::Failed(Error::new(
                Status::LimitExceeded,
                "template query analysis generation was exhausted",
            )));
            return Admission::Obsolete;
        };
        state.generation = generation;
        state.last_admitted = Some(now);
        state.rate_eligible_at = None;
        state.pending = None;
        state.processing = None;
        let Some(in_flight) = state.in_flight.iter_mut().find(|slot| slot.is_none()) else {
            state.processing = None;
            return Admission::Obsolete;
        };
        *in_flight = Some((generation, stamp));
        drop(state);
        drop(authority);
        drop(scheduler_authority);
        Admission::Admitted(generation)
    }

    fn complete(
        self: &Arc<Self>,
        generation: u64,
        mapped: &MappedMatch,
        mapping: Option<Weak<CpuMapping>>,
        result: std::result::Result<ImmutableAnalysisResult, Error>,
        now: MonotonicInstant,
    ) {
        if let Some(interruption) = self.operation.interruption() {
            self.terminate(interruption_outcome(interruption));
            return;
        }
        let scheduler = self.scheduler.upgrade();
        let scheduler_authority = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let session = self.session.upgrade();
        let session_authority = session.as_ref().map(|session| lock(&session.activation));
        let authority = session
            .as_ref()
            .and_then(|session| lock(&session.terminal_authority).clone())
            .or_else(|| {
                scheduler
                    .as_ref()
                    .is_none_or(|scheduler| scheduler.closed.load(Ordering::Acquire))
                    .then_some(TemplateTerminalOutcome::SchedulerClosed)
            });
        if let Some(outcome) = authority {
            drop(session_authority);
            drop(scheduler_authority);
            self.terminate(outcome);
            return;
        }
        let stamp = mapped.frame().stamp();
        let completion_disposition = if result.is_ok() {
            TemplateWorkDisposition::Completed
        } else {
            TemplateWorkDisposition::Failed
        };
        let mut terminal_committed = false;
        let mut stale = false;
        {
            let mut state = lock(&self.state);
            let Some(slot) = state
                .in_flight
                .iter()
                .position(|slot| *slot == Some((generation, stamp)))
            else {
                return;
            };
            state.in_flight[slot] = None;
            if state.terminal.is_some() {
                return;
            }
            let superseded_by_completed_frame = state
                .last_frame
                .is_some_and(|last| !matches!(last.order(&stamp), Ok(FrameOrder::Before)));
            if superseded_by_completed_frame {
                state.work.increment(TemplateWorkDisposition::Superseded);
                stale = true;
            } else {
                state.last_frame = Some(stamp);
                state.effective_region = Some(mapped.searched());
                match result {
                    Err(error) => {
                        state.work.increment(TemplateWorkDisposition::Failed);
                        let superseded = in_flight_count(&state);
                        state.in_flight.fill(None);
                        for _ in 0..superseded {
                            state.work.increment(TemplateWorkDisposition::Superseded);
                        }
                        state.terminal_frame = Some(stamp);
                        state.terminal = Some(Arc::new(TemplateTerminalOutcome::Failed(error)));
                        terminal_committed = true;
                    }
                    Ok(result) => {
                        state.work.increment(TemplateWorkDisposition::Completed);
                        state.previous_mapping = mapping;
                        if result.is_empty() {
                            state.stability = StabilityState::default();
                        } else {
                            state.stability.observations =
                                state.stability.observations.saturating_add(1);
                            let started = *state.stability.started.get_or_insert(now);
                            state.stability.duration = now.saturating_duration_since(started);
                            let stable = match self.stability.kind() {
                                TemplateStabilityKind::Immediate => true,
                                TemplateStabilityKind::Consecutive => self
                                    .stability
                                    .required_observations()
                                    .is_some_and(|required| {
                                        state.stability.observations >= required.get()
                                    }),
                                TemplateStabilityKind::Duration => self
                                    .stability
                                    .required_duration()
                                    .is_some_and(|required| state.stability.duration >= required),
                            };
                            if stable {
                                let outcome =
                                    TemplateTerminalOutcome::Matched(TemplateWatchResult {
                                        template: self.template.id().clone(),
                                        target: self.target,
                                        frame: mapped.frame().clone(),
                                        result: result
                                            .into_shared()
                                            .expect("a stable match result is non-empty"),
                                        confirmed_observations: state.stability.observations,
                                        confirmed_duration: state.stability.duration,
                                    });
                                let superseded = in_flight_count(&state);
                                state.in_flight.fill(None);
                                for _ in 0..superseded {
                                    state.work.increment(TemplateWorkDisposition::Superseded);
                                }
                                state.terminal = Some(Arc::new(outcome));
                                state.terminal_frame = Some(stamp);
                                terminal_committed = true;
                            }
                        }
                    }
                }
            }
        }
        drop(session_authority);
        drop(scheduler_authority);
        if stale {
            self.emit(Some(TemplateWorkDisposition::Superseded), false);
            self.changed.notify_all();
            if !self.finish_source_end_if_idle()
                && let Some(scheduler) = self.scheduler.upgrade()
            {
                scheduler.wake();
            }
            return;
        }
        self.emit(Some(completion_disposition), false);
        if terminal_committed {
            self.after_terminal_commit();
            return;
        }
        self.changed.notify_all();
        if let Some(session) = self.session.upgrade() {
            session.notify_progress();
        }
        if !self.finish_source_end_if_idle()
            && let Some(scheduler) = self.scheduler.upgrade()
        {
            scheduler.wake();
        }
    }

    fn fail_mapping(self: &Arc<Self>, stamp: FrameStamp, error: Error) {
        if let Some(interruption) = self.operation.interruption() {
            self.terminate(interruption_outcome(interruption));
            return;
        }
        let scheduler = self.scheduler.upgrade();
        let scheduler_authority = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let session = self.session.upgrade();
        let session_authority = session.as_ref().map(|session| lock(&session.activation));
        let authority = session
            .as_ref()
            .and_then(|session| lock(&session.terminal_authority).clone())
            .or_else(|| {
                scheduler
                    .as_ref()
                    .is_none_or(|scheduler| scheduler.closed.load(Ordering::Acquire))
                    .then_some(TemplateTerminalOutcome::SchedulerClosed)
            });
        if let Some(outcome) = authority {
            drop(session_authority);
            drop(scheduler_authority);
            self.terminate(outcome);
            return;
        }
        let committed = {
            let mut state = lock(&self.state);
            if state.terminal.is_some()
                || state.processing != Some(stamp)
                || state.pending.as_ref().map(|pending| pending.frame.stamp()) != Some(stamp)
            {
                if state.processing == Some(stamp) {
                    state.processing = None;
                }
                false
            } else {
                state.processing = None;
                state.pending = None;
                let superseded = in_flight_count(&state);
                state.in_flight.fill(None);
                for _ in 0..superseded {
                    state.work.increment(TemplateWorkDisposition::Superseded);
                }
                state.work.increment(TemplateWorkDisposition::Failed);
                state.terminal = Some(Arc::new(TemplateTerminalOutcome::Failed(error)));
                state.terminal_frame = Some(stamp);
                true
            }
        };
        drop(session_authority);
        drop(scheduler_authority);
        if committed {
            self.emit(Some(TemplateWorkDisposition::Failed), false);
            self.after_terminal_commit();
        }
    }
}

impl Drop for QueryShared {
    fn drop(&mut self) {
        self.release_reservation();
    }
}

#[derive(Debug)]
pub(crate) struct WatchSession {
    id: u64,
    description: SessionDescription,
    capture: Arc<dyn CaptureSession>,
    scheduler: Weak<WatchScheduler>,
    closed: AtomicBool,
    source_ended: AtomicBool,
    activated: AtomicBool,
    active_queries: AtomicUsize,
    activation: Mutex<()>,
    terminal_authority: Mutex<Option<TemplateTerminalOutcome>>,
    state: Mutex<WatchSessionState>,
    progress: Condvar,
}

#[derive(Debug, Default)]
struct WatchSessionState {
    queries: Vec<Weak<QueryShared>>,
    query_cursor: usize,
    acquisition_cancel: Option<mado_pilot_core::CancellationToken>,
    acquisition_running: bool,
    acquisition_exiting: bool,
    acquisition_generation: u64,
    acquisition: Option<JoinHandle<()>>,
}

impl WatchSession {
    fn new(
        id: u64,
        description: SessionDescription,
        capture: Arc<dyn CaptureSession>,
        scheduler: Weak<WatchScheduler>,
    ) -> Self {
        Self {
            id,
            description,
            capture,
            scheduler,
            closed: AtomicBool::new(false),
            source_ended: AtomicBool::new(false),
            activated: AtomicBool::new(false),
            active_queries: AtomicUsize::new(0),
            activation: Mutex::new(()),
            terminal_authority: Mutex::new(None),
            state: Mutex::new(WatchSessionState::default()),
            progress: Condvar::new(),
        }
    }

    pub(crate) fn start_query(
        self: &Arc<Self>,
        request: TemplateWatchRequest,
    ) -> Result<TemplateQuery> {
        let publication_context = request.operation().clone();
        let publication = Operation::admit(&publication_context)?;
        validate_watch_region(request.region())?;
        validate_watch_timing(&request)?;
        if self.closed.load(Ordering::Acquire)
            || self.source_ended.load(Ordering::Acquire)
            || !self.capture.is_open()
        {
            return Err(Error::new(
                Status::Closed,
                "template watch session is closing",
            ));
        }
        let scheduler = self
            .scheduler
            .upgrade()
            .ok_or_else(|| Error::new(Status::Closed, "template watch scheduler is unavailable"))?;
        if scheduler.closed.load(Ordering::Acquire) {
            return Err(Error::new(
                Status::Closed,
                "template watch scheduler is closed",
            ));
        }
        if request.template().backend().as_str() != scheduler.matcher.descriptor().id() {
            return Err(Error::new(
                Status::InvalidArgument,
                "prepared template belongs to a different matching backend",
            ));
        }
        let diagnostic = scheduler
            .diagnostics
            .as_ref()
            .map(|diagnostics| {
                diagnostics.observe(request.operation(), DiagnosticOperationKind::TemplateWatch)
            })
            .transpose()?;
        let started_at = request.operation().now();
        let id = scheduler.issue_query_id().ok_or_else(|| {
            Error::new(
                Status::LimitExceeded,
                "template query identity space was exhausted",
            )
        })?;
        if let Err(error) = scheduler.ensure_workers() {
            scheduler.emit_rejected(
                request.operation(),
                RejectedDiagnostic {
                    observed: diagnostic,
                    query: id,
                    target: self.description.target(),
                    started_at,
                    session_query_count: self.active_queries.load(Ordering::Acquire),
                    outcome: TemplateWatchDiagnosticOutcome::Failed(error.status()),
                },
            );
            return Err(error);
        }
        if let Err(error) = reserve(
            &scheduler.query_count,
            MAX_ENGINE_QUERIES,
            "engine template query capacity",
        ) {
            scheduler.emit_rejected(
                request.operation(),
                RejectedDiagnostic {
                    observed: diagnostic,
                    query: id,
                    target: self.description.target(),
                    started_at,
                    session_query_count: self.active_queries.load(Ordering::Acquire),
                    outcome: TemplateWatchDiagnosticOutcome::Overloaded,
                },
            );
            return Err(error);
        }
        if let Err(error) = self.reserve_query_slot(&scheduler) {
            scheduler.release_query_reservation();
            scheduler.emit_rejected(
                request.operation(),
                RejectedDiagnostic {
                    observed: diagnostic,
                    query: id,
                    target: self.description.target(),
                    started_at,
                    session_query_count: self.active_queries.load(Ordering::Acquire),
                    outcome: TemplateWatchDiagnosticOutcome::Overloaded,
                },
            );
            return Err(error);
        }
        let query = Arc::new(QueryShared {
            id,
            target: self.description.target(),
            session: Arc::downgrade(self),
            scheduler: Arc::downgrade(&scheduler),
            template_instance: request.template().diagnostic_instance(),
            template: request.template,
            region: request.region,
            options: request.options,
            rate: request.rate,
            stability: request.stability,
            change_policy: request.change_policy,
            operation: request.operation,
            diagnostic,
            diagnostic_emission: Mutex::new(()),
            started_at,
            state: Mutex::new(QueryData {
                terminal: None,
                needs_current: true,
                pending: None,
                processing: None,
                in_flight: [None; MAX_IN_FLIGHT_ANALYSES],
                source_frame: None,
                last_frame: None,
                terminal_frame: None,
                effective_region: None,
                previous_mapping: None,
                generation: 0,
                last_admitted: None,
                rate_eligible_at: None,
                stability: StabilityState::default(),
                work: TemplateWorkCounts::new(),
            }),
            changed: Condvar::new(),
            reservation_released: AtomicBool::new(false),
        });
        if let Err(interruption) = publication.commit(()) {
            let error = Error::from(interruption);
            query.terminate(interruption_outcome(interruption));
            return Err(error);
        }
        let lost = {
            let mut state = lock(&self.state);
            if self.closed.load(Ordering::Acquire)
                || self.source_ended.load(Ordering::Acquire)
                || scheduler.closed.load(Ordering::Acquire)
            {
                Some(if scheduler.closed.load(Ordering::Acquire) {
                    TemplateTerminalOutcome::SchedulerClosed
                } else {
                    TemplateTerminalOutcome::SessionClosed
                })
            } else {
                state.queries.retain(|query| query.strong_count() != 0);
                state.queries.push(Arc::downgrade(&query));
                None
            }
        };
        if let Some(outcome) = lost {
            query.terminate(outcome);
            return Err(Error::new(
                Status::Closed,
                "template watch authority closed before query publication",
            ));
        }
        if let Err(error) = self.ensure_acquisition() {
            query.terminate(TemplateTerminalOutcome::Failed(error.clone()));
            return Err(error);
        }
        scheduler.wake();
        Ok(TemplateQuery { shared: query })
    }

    fn reserve_query_slot(self: &Arc<Self>, scheduler: &Arc<WatchScheduler>) -> Result<()> {
        let _activation = lock(&self.activation);
        if self.closed.load(Ordering::Acquire)
            || self.source_ended.load(Ordering::Acquire)
            || scheduler.closed.load(Ordering::Acquire)
        {
            return Err(Error::new(
                Status::Closed,
                "template watcher closed before query reservation",
            ));
        }
        let active = self.active_queries.load(Ordering::Acquire);
        if active >= MAX_SESSION_QUERIES {
            return Err(Error::new(
                Status::LimitExceeded,
                "session template query capacity was reached",
            ));
        }
        if active == 0 && !self.activated.load(Ordering::Acquire) {
            reserve(
                &scheduler.active_sessions,
                MAX_ACTIVE_SESSIONS,
                "active template watcher session capacity",
            )?;
            self.activated.store(true, Ordering::Release);
            let mut registry = lock(&scheduler.registry);
            if !registry
                .sessions
                .iter()
                .any(|session| Arc::ptr_eq(session, self))
            {
                registry.sessions.push(Arc::clone(self));
            }
        }
        self.active_queries.store(active + 1, Ordering::Release);
        Ok(())
    }

    fn ensure_acquisition(self: &Arc<Self>) -> Result<()> {
        let mut state = lock(&self.state);
        while state.acquisition_running && state.acquisition_exiting {
            state = self
                .progress
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if self.closed.load(Ordering::Acquire)
            || self.source_ended.load(Ordering::Acquire)
            || self
                .scheduler
                .upgrade()
                .is_none_or(|scheduler| scheduler.closed.load(Ordering::Acquire))
        {
            return Err(Error::new(
                Status::Closed,
                "template acquisition authority is closed",
            ));
        }
        if state.acquisition_running {
            if let Some(cancellation) = &state.acquisition_cancel {
                cancellation.cancel();
            }
            drop(state);
            self.progress.notify_all();
            return Ok(());
        }
        drop(state.acquisition.take());
        let generation = state.acquisition_generation.checked_add(1).ok_or_else(|| {
            Error::new(
                Status::LimitExceeded,
                "template acquisition generation was exhausted",
            )
        })?;
        let cancellation = mado_pilot_core::CancellationToken::new();
        let thread_cancellation = cancellation.clone();
        let session = Arc::clone(self);
        let handle = thread::Builder::new()
            .name(format!("mado-watch-acquire-{}", self.id))
            .spawn(move || session.acquisition_loop(thread_cancellation, generation))
            .map_err(|error| {
                Error::new(
                    Status::Internal,
                    format!("failed to start template acquisition worker: {error}"),
                )
            })?;
        state.acquisition_cancel = Some(cancellation);
        state.acquisition_running = true;
        state.acquisition_generation = generation;
        state.acquisition = Some(handle);
        Ok(())
    }

    fn acquisition_loop(
        self: Arc<Self>,
        initial_cancellation: mado_pilot_core::CancellationToken,
        generation: u64,
    ) {
        let mut cancellation = initial_cancellation;
        let mut operation = OperationContext::new().with_cancellation(cancellation.clone());
        let mut last = None;
        loop {
            let stop = {
                let mut state = lock(&self.state);
                let stop = state.acquisition_generation != generation
                    || self.closed.load(Ordering::Acquire)
                    || self.source_ended.load(Ordering::Acquire)
                    || self.active_queries.load(Ordering::Acquire) == 0;
                if stop && state.acquisition_generation == generation {
                    state.acquisition_exiting = true;
                }
                stop
            };
            if stop {
                break;
            }
            if cancellation.is_cancelled() {
                cancellation = mado_pilot_core::CancellationToken::new();
                operation = OperationContext::new().with_cancellation(cancellation.clone());
                let mut state = lock(&self.state);
                if state.acquisition_generation != generation
                    || self.closed.load(Ordering::Acquire)
                    || self.source_ended.load(Ordering::Acquire)
                    || self.active_queries.load(Ordering::Acquire) == 0
                {
                    if state.acquisition_generation == generation {
                        state.acquisition_exiting = true;
                    }
                    break;
                }
                state.acquisition_cancel = Some(cancellation.clone());
            }
            let needs_current = self
                .query_snapshot()
                .iter()
                .any(|query| query.needs_current());
            let request = match (needs_current, last) {
                (true, _) | (_, None) => FrameRequest::latest(),
                (false, Some(last)) => FrameRequest::newer_than(last),
            };
            match self.capture.frame(&request, &operation) {
                Ok(frame) => {
                    let stamp = frame.stamp();
                    last = Some(stamp);
                    self.publish(frame);
                    self.wait_until_admitted_or_considered(stamp, &cancellation);
                }
                Err(error)
                    if error.status() == Status::Cancelled
                        && !self.closed.load(Ordering::Acquire) =>
                {
                    continue;
                }
                Err(error) => {
                    if self.closed.load(Ordering::Acquire) {
                        break;
                    }
                    let terminal = match error.status() {
                        Status::TargetLost => Some(TemplateTerminalOutcome::TargetLost),
                        Status::Closed => None,
                        _ => Some(TemplateTerminalOutcome::Failed(error)),
                    };
                    if let Some(outcome) = terminal {
                        let outcome = {
                            let _activation = lock(&self.activation);
                            self.source_ended.store(true, Ordering::Release);
                            let mut terminal = lock(&self.terminal_authority);
                            terminal.get_or_insert(outcome).clone()
                        };
                        self.terminate_queries(outcome);
                    } else {
                        self.source_ended.store(true, Ordering::Release);
                        self.terminate_idle_queries();
                    }
                    self.deactivate_if_finished();
                    break;
                }
            }
        }
        let activation = lock(&self.activation);
        let current_generation = lock(&self.state).acquisition_generation == generation;
        if current_generation && self.active_queries.load(Ordering::Acquire) == 0 {
            let _ = self.deactivate_locked();
        }
        if current_generation {
            let mut state = lock(&self.state);
            state.acquisition_running = false;
            state.acquisition_exiting = false;
            state.acquisition_cancel = None;
        }
        drop(activation);
        self.progress.notify_all();
    }

    fn publish(&self, frame: Frame) {
        let scheduler = self.scheduler.upgrade();
        if let Some(scheduler) = &scheduler {
            scheduler.publishing.fetch_add(1, Ordering::AcqRel);
        }
        let queries = self.query_snapshot();
        for query in queries {
            if let Some(interruption) = query.operation.interruption() {
                query.terminate(interruption_outcome(interruption));
                continue;
            }
            let now = query.operation.now();
            query.enqueue(frame.clone(), now);
        }
        if let Some(scheduler) = scheduler {
            scheduler
                .publication_generation
                .fetch_add(1, Ordering::AcqRel);
            scheduler.publishing.fetch_sub(1, Ordering::AcqRel);
            scheduler.wake();
        }
    }

    fn query_snapshot(&self) -> Vec<Arc<QueryShared>> {
        let mut state = lock(&self.state);
        let mut queries = Vec::with_capacity(state.queries.len());
        state.queries.retain(|query| {
            if let Some(query) = query.upgrade() {
                queries.push(query);
                true
            } else {
                false
            }
        });
        queries
    }

    fn unregister_query(&self, id: TemplateQueryId) {
        let mut state = lock(&self.state);
        state
            .queries
            .retain(|query| query.upgrade().is_some_and(|query| query.id != id));
        if state.query_cursor >= state.queries.len() {
            state.query_cursor = 0;
        }
    }

    fn ordered_queries(&self) -> Vec<Arc<QueryShared>> {
        let mut state = lock(&self.state);
        let mut queries = Vec::with_capacity(state.queries.len());
        state.queries.retain(|query| {
            if let Some(query) = query.upgrade() {
                queries.push(query);
                true
            } else {
                false
            }
        });
        if !queries.is_empty() {
            let rotation = state.query_cursor % queries.len();
            queries.rotate_left(rotation);
            state.query_cursor = (state.query_cursor + 1) % queries.len();
        }
        queries
    }

    fn wait_until_admitted_or_considered(
        &self,
        stamp: FrameStamp,
        cancellation: &mado_pilot_core::CancellationToken,
    ) {
        loop {
            if self.closed.load(Ordering::Acquire) || cancellation.is_cancelled() {
                return;
            }
            if self
                .query_snapshot()
                .iter()
                .all(|query| query.has_admitted_or_considered(stamp))
            {
                return;
            }
            let state = lock(&self.state);
            let _ = self
                .progress
                .wait_timeout(state, WORKER_WAIT)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn notify_progress(&self) {
        self.progress.notify_all();
    }

    pub(crate) fn close(&self, outcome: TemplateTerminalOutcome) {
        let (first, outcome) = {
            let _activation = lock(&self.activation);
            let first = !self.closed.swap(true, Ordering::AcqRel);
            let mut terminal = lock(&self.terminal_authority);
            if first && terminal.is_none() {
                *terminal = Some(outcome);
            }
            (
                first,
                terminal
                    .clone()
                    .unwrap_or(TemplateTerminalOutcome::SessionClosed),
            )
        };
        if !first {
            return;
        }
        let acquisition = {
            let mut state = lock(&self.state);
            if let Some(cancellation) = &state.acquisition_cancel {
                cancellation.cancel();
            }
            state.acquisition_cancel = None;
            state.acquisition.take()
        };
        drop(acquisition);
        self.terminate_queries(outcome);
        self.deactivate_if_finished();
        self.progress.notify_all();
    }

    fn release_query_slot(&self) {
        let _activation = lock(&self.activation);
        let active = self.active_queries.load(Ordering::Acquire);
        if active == 0 {
            return;
        }
        self.active_queries.store(active - 1, Ordering::Release);
        let deactivated =
            active == 1 && self.request_acquisition_stop() && self.deactivate_locked();
        drop(_activation);
        if deactivated {
            self.progress.notify_all();
        }
    }

    fn deactivate_if_finished(&self) {
        let activation = lock(&self.activation);
        let deactivated = self.active_queries.load(Ordering::Acquire) == 0
            && self.request_acquisition_stop()
            && self.deactivate_locked();
        drop(activation);
        if deactivated {
            self.progress.notify_all();
        }
    }

    fn request_acquisition_stop(&self) -> bool {
        let state = lock(&self.state);
        if let Some(cancellation) = &state.acquisition_cancel {
            cancellation.cancel();
        }
        !state.acquisition_running
    }

    fn deactivate_locked(&self) -> bool {
        let deactivated = self.activated.swap(false, Ordering::AcqRel);
        if deactivated && let Some(scheduler) = self.scheduler.upgrade() {
            scheduler.active_sessions.fetch_sub(1, Ordering::AcqRel);
            lock(&scheduler.registry)
                .sessions
                .retain(|session| !std::ptr::eq(Arc::as_ptr(session), self));
        }
        deactivated
    }

    fn terminate_queries(&self, outcome: TemplateTerminalOutcome) {
        for query in self.query_snapshot() {
            query.terminate(outcome.clone());
        }
    }

    fn terminate_idle_queries(&self) {
        for query in self.query_snapshot() {
            if query.is_idle() {
                query.terminate(TemplateTerminalOutcome::SessionClosed);
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct WatchScheduler {
    matcher: Matcher,
    descriptor: TemplateSchedulerDescriptor,
    diagnostics: Option<DiagnosticSink>,
    closed: AtomicBool,
    next_query: AtomicU64,
    next_session: AtomicU64,
    query_count: AtomicUsize,
    active_sessions: AtomicUsize,
    publishing: AtomicUsize,
    publication_generation: AtomicU64,
    registry: Mutex<SchedulerRegistry>,
    admission: Mutex<()>,
    cache: Mutex<MappingCache>,
    wake_state: Mutex<u64>,
    wake_condition: Condvar,
    workers: Mutex<Vec<JoinHandle<()>>>,
    supervisor: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, Default)]
struct SchedulerRegistry {
    sessions: Vec<Arc<WatchSession>>,
    session_cursor: usize,
}

#[derive(Debug, Default)]
struct MappingCache {
    entries: std::collections::VecDeque<MappingCacheEntry>,
    bytes: usize,
}

#[derive(Debug)]
struct MappingCacheEntry {
    mapping: Arc<CpuMapping>,
    bytes: usize,
}

impl MappingCache {
    fn remember(&mut self, mapped: &MappedMatch) -> Option<Weak<CpuMapping>> {
        let mapping = mapped.pixels()?;
        let bytes = mapping.bytes().len();
        if bytes > MAPPED_CACHE_BYTES {
            return None;
        }
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.mapping.stamp() == mapping.stamp()
                && entry.mapping.region() == mapping.region()
                && entry.mapping.descriptor() == mapping.descriptor()
        }) && let Some(existing) = self.entries.remove(index)
        {
            let weak = Arc::downgrade(&existing.mapping);
            self.entries.push_back(existing);
            return Some(weak);
        }
        while self.entries.len() >= MAX_MAPPED_CACHE_ENTRIES
            || self.bytes.saturating_add(bytes) > MAPPED_CACHE_BYTES
        {
            let Some(evicted) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(evicted.bytes);
        }
        let mapping = Arc::new(mapping.clone());
        let weak = Arc::downgrade(&mapping);
        self.entries.push_back(MappingCacheEntry { mapping, bytes });
        self.bytes = self.bytes.saturating_add(bytes);
        Some(weak)
    }
}

impl WatchScheduler {
    pub(crate) fn new(matcher: Matcher, diagnostics: Option<DiagnosticSink>) -> Arc<Self> {
        Arc::new(Self {
            matcher,
            diagnostics,
            descriptor: TemplateSchedulerDescriptor::selected_default(),
            closed: AtomicBool::new(false),
            next_query: AtomicU64::new(1),
            next_session: AtomicU64::new(1),
            query_count: AtomicUsize::new(0),
            active_sessions: AtomicUsize::new(0),
            publishing: AtomicUsize::new(0),
            publication_generation: AtomicU64::new(0),
            registry: Mutex::new(SchedulerRegistry::default()),
            admission: Mutex::new(()),
            cache: Mutex::new(MappingCache::default()),
            wake_state: Mutex::new(0),
            wake_condition: Condvar::new(),
            workers: Mutex::new(Vec::new()),
            supervisor: Mutex::new(None),
        })
    }

    pub(crate) const fn descriptor(&self) -> TemplateSchedulerDescriptor {
        self.descriptor
    }

    fn release_query_reservation(&self) {
        if self.query_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            let mut cache = lock(&self.cache);
            if self.query_count.load(Ordering::Acquire) == 0 {
                cache.entries.clear();
                cache.bytes = 0;
            }
        }
    }

    pub(crate) fn register_session(
        self: &Arc<Self>,
        capture: Arc<dyn CaptureSession>,
    ) -> Arc<WatchSession> {
        let id = self.next_session.fetch_add(1, Ordering::AcqRel);
        Arc::new(WatchSession::new(
            id,
            capture.description(),
            capture,
            Arc::downgrade(self),
        ))
    }

    fn issue_query_id(&self) -> Option<TemplateQueryId> {
        self.next_query
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .ok()
            .and_then(TemplateQueryId::new)
    }

    fn emit_rejected(&self, operation: &OperationContext, rejection: RejectedDiagnostic) {
        let RejectedDiagnostic {
            observed,
            query,
            target,
            started_at,
            session_query_count,
            outcome,
        } = rejection;
        let (Some(diagnostics), Some(observed)) = (&self.diagnostics, observed) else {
            return;
        };
        let now = operation.now();
        let mut work = TemplateWorkCounts::new();
        work.increment(TemplateWorkDisposition::Rejected);
        let payload = TemplateWatchDiagnostic {
            query,
            target,
            frame: None,
            region: None,
            state: TemplateQueryState::Terminal,
            confirmed_observations: 0,
            confirmed_duration_nanos: 0,
            disposition: Some(TemplateWorkDisposition::Rejected),
            work,
            pending_count: 0,
            in_flight_count: 0,
            session_query_count: u32::try_from(session_query_count).unwrap_or(u32::MAX),
            engine_query_count: u32::try_from(self.query_count.load(Ordering::Acquire))
                .unwrap_or(u32::MAX),
            elapsed_nanos: duration_nanos(now.saturating_duration_since(started_at)),
            outcome: Some(outcome),
        };
        diagnostics.normal(observed, operation, || {
            DiagnosticPayload::TemplateWatch(payload)
        });
    }

    fn ensure_workers(self: &Arc<Self>) -> Result<()> {
        let mut workers = lock(&self.workers);
        while workers.len() < MAX_IN_FLIGHT_ANALYSES {
            let index = workers.len();
            let scheduler = Arc::clone(self);
            let worker = thread::Builder::new()
                .name(format!("mado-watch-worker-{index}"))
                .spawn(move || scheduler.worker_loop())
                .map_err(|error| {
                    Error::new(
                        Status::Internal,
                        format!("failed to start template analysis worker: {error}"),
                    )
                })?;
            workers.push(worker);
        }
        drop(workers);
        let mut supervisor = lock(&self.supervisor);
        if supervisor.is_none() {
            let scheduler = Arc::clone(self);
            *supervisor = Some(
                thread::Builder::new()
                    .name("mado-watch-supervisor".to_owned())
                    .spawn(move || scheduler.supervisor_loop())
                    .map_err(|error| {
                        Error::new(
                            Status::Internal,
                            format!("failed to start template scheduler supervisor: {error}"),
                        )
                    })?,
            );
        }
        Ok(())
    }

    fn supervisor_loop(self: Arc<Self>) {
        while !self.closed.load(Ordering::Acquire) {
            if self.query_count.load(Ordering::Acquire) == 0 {
                let state = lock(&self.wake_state);
                drop(
                    self.wake_condition
                        .wait_while(state, |_| {
                            !self.closed.load(Ordering::Acquire)
                                && self.query_count.load(Ordering::Acquire) == 0
                        })
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                continue;
            }
            self.sweep_authority();
            let state = lock(&self.wake_state);
            let _ = self
                .wake_condition
                .wait_timeout(state, WORKER_WAIT)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn sweep_authority(&self) {
        let mut ready = false;
        for session in self.session_snapshot() {
            for query in session.query_snapshot() {
                if let Some(interruption) = query.operation.interruption() {
                    query.terminate(interruption_outcome(interruption));
                    continue;
                }
                let Some(candidate) = query.expiry_candidate() else {
                    continue;
                };
                let now = query.operation.now();
                if candidate
                    .rate_eligible_at
                    .is_some_and(|eligible| now < eligible)
                {
                    continue;
                }
                if !query.expire_if_current(candidate, now, self.descriptor.eligible_queue_expiry())
                {
                    ready = true;
                }
            }
        }
        if ready {
            self.wake();
        }
    }

    fn worker_loop(self: Arc<Self>) {
        let mut observed_wake = 0;
        while !self.closed.load(Ordering::Acquire) {
            if let Some(work) = self.next_work() {
                self.process(work);
                continue;
            }
            let state = lock(&self.wake_state);
            if *state == observed_wake && !self.closed.load(Ordering::Acquire) {
                let (state, _) = self
                    .wake_condition
                    .wait_timeout(state, WORKER_WAIT)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                observed_wake = *state;
            } else {
                observed_wake = *state;
            }
        }
    }

    fn next_work(&self) -> Option<ClaimedWork> {
        if self.publishing.load(Ordering::Acquire) != 0 {
            return None;
        }
        let publication_generation = self.publication_generation.load(Ordering::Acquire);
        let sessions = self.ordered_sessions();
        for session in sessions {
            let queries = session.ordered_queries();
            let mut candidates = Vec::with_capacity(queries.len());
            for (rotation, query) in queries.into_iter().enumerate() {
                if let Some(interruption) = query.operation.interruption() {
                    query.terminate(interruption_outcome(interruption));
                    continue;
                }
                let Some(candidate) = query.candidate() else {
                    continue;
                };
                let now = query.operation.now();
                if candidate
                    .rate_eligible_at
                    .is_some_and(|eligible| now < eligible)
                {
                    continue;
                }
                if query.expire_if_current(candidate, now, self.descriptor.eligible_queue_expiry())
                {
                    continue;
                }
                candidates.push((
                    query.operation.remaining(),
                    rotation,
                    query,
                    candidate.stamp,
                ));
            }
            candidates.sort_by(compare_deadline);
            for (_, _, query, stamp) in candidates {
                let Some(generation) = query.next_generation() else {
                    continue;
                };
                let peers = self.eligible_coalescing_peers(&query, stamp, generation);
                let _admission = lock(&self.admission);
                if self.closed.load(Ordering::Acquire)
                    || self.publishing.load(Ordering::Acquire) != 0
                {
                    return None;
                }
                let Some(session) = query.session.upgrade() else {
                    continue;
                };
                let _session_authority = lock(&session.activation);
                let authority = lock(&session.terminal_authority).clone().or_else(|| {
                    session
                        .closed
                        .load(Ordering::Acquire)
                        .then_some(TemplateTerminalOutcome::SessionClosed)
                });
                if let Some(outcome) = authority {
                    drop(_session_authority);
                    drop(_admission);
                    query.terminate(outcome);
                    continue;
                }
                if query.next_generation() != Some(generation) {
                    continue;
                }
                if let Some(mut work) = query.claim(stamp) {
                    self.claim_coalesced(&mut work, &session, generation, peers);
                    if self.publishing.load(Ordering::Acquire) != 0
                        || self.publication_generation.load(Ordering::Acquire)
                            != publication_generation
                    {
                        release_work_claims(&work);
                        return None;
                    }
                    return Some(work);
                }
            }
        }
        None
    }

    fn eligible_coalescing_peers(
        &self,
        leader: &Arc<QueryShared>,
        stamp: FrameStamp,
        generation: u64,
    ) -> Vec<Arc<QueryShared>> {
        let mut peers = Vec::new();
        for session in self.session_snapshot() {
            for query in session.query_snapshot() {
                if Arc::ptr_eq(leader, &query)
                    || query.next_generation() != Some(generation)
                    || query.template_instance != leader.template_instance
                {
                    continue;
                }
                if let Some(interruption) = query.operation.interruption() {
                    query.terminate(interruption_outcome(interruption));
                    continue;
                }
                let Some(candidate) = query.candidate() else {
                    continue;
                };
                if candidate.stamp != stamp {
                    continue;
                }
                let now = query.operation.now();
                if candidate
                    .rate_eligible_at
                    .is_some_and(|eligible| now < eligible)
                    || query.expire_if_current(
                        candidate,
                        now,
                        self.descriptor.eligible_queue_expiry(),
                    )
                {
                    continue;
                }
                peers.push(query);
            }
        }
        peers
    }

    fn claim_coalesced(
        &self,
        work: &mut ClaimedWork,
        leader_session: &Arc<WatchSession>,
        generation: u64,
        peers: Vec<Arc<QueryShared>>,
    ) {
        let leader = MatchRequest::new(
            &work.frame,
            work.query.region,
            &work.query.template,
            work.query.options,
        );
        for query in peers {
            let Some(session) = query.session.upgrade() else {
                continue;
            };
            let _session_authority =
                (!Arc::ptr_eq(&session, leader_session)).then(|| lock(&session.activation));
            let authority = lock(&session.terminal_authority).clone().or_else(|| {
                session
                    .closed
                    .load(Ordering::Acquire)
                    .then_some(TemplateTerminalOutcome::SessionClosed)
            });
            if authority.is_some() {
                continue;
            }
            if query.next_generation() != Some(generation) {
                continue;
            }
            let Some(candidate) = query.candidate() else {
                continue;
            };
            let Some(peer) = query.claim(candidate.stamp) else {
                continue;
            };
            let request =
                MatchRequest::new(&peer.frame, query.region, &query.template, query.options);
            if self.matcher.requests_are_equivalent(&leader, &request) {
                work.peers.push((query, peer.frame));
            } else {
                query.release_claim(candidate.stamp);
            }
        }
    }

    fn session_snapshot(&self) -> Vec<Arc<WatchSession>> {
        let mut registry = lock(&self.registry);
        let mut sessions = Vec::with_capacity(registry.sessions.len());
        registry.sessions.retain(|session| {
            let active = session.active_queries.load(Ordering::Acquire);
            let activated = session.activated.load(Ordering::Acquire);
            let closed = session.closed.load(Ordering::Acquire);
            let keep = activated || active != 0;
            if keep && active != 0 && !closed {
                sessions.push(Arc::clone(session));
            }
            keep
        });
        sessions
    }

    fn ordered_sessions(&self) -> Vec<Arc<WatchSession>> {
        let mut registry = lock(&self.registry);
        let mut sessions = Vec::with_capacity(registry.sessions.len());
        registry.sessions.retain(|session| {
            let active = session.active_queries.load(Ordering::Acquire);
            let activated = session.activated.load(Ordering::Acquire);
            let closed = session.closed.load(Ordering::Acquire);
            let keep = activated || active != 0;
            if keep && active != 0 && !closed {
                sessions.push(Arc::clone(session));
            }
            keep
        });
        if !sessions.is_empty() {
            let rotation = registry.session_cursor % sessions.len();
            sessions.rotate_left(rotation);
            registry.session_cursor = (registry.session_cursor + 1) % sessions.len();
        }
        sessions
    }

    fn process(&self, work: ClaimedWork) {
        let mut claimed = Vec::with_capacity(work.peers.len() + 1);
        claimed.push((work.query, work.frame));
        claimed.extend(work.peers);
        for (query, _) in &claimed {
            if let Some(interruption) = query.operation.interruption() {
                query.terminate(interruption_outcome(interruption));
            }
        }
        let Some((representative, frame)) =
            claimed.iter().find(|(query, _)| query.terminal().is_none())
        else {
            return;
        };
        let cache_required = claimed
            .iter()
            .any(|(query, _)| query.change_policy != ChangeDetectionPolicy::AnalysisAlways);
        let representative = Arc::clone(representative);
        let frame = frame.clone();
        let request = MatchRequest::new(
            &frame,
            representative.region,
            &representative.template,
            representative.options,
        );
        let mapped = match self.matcher.map_match(&request, &representative.operation) {
            Ok(mapped) => mapped,
            Err(error)
                if matches!(error.status(), Status::Cancelled | Status::DeadlineExceeded)
                    && representative.operation.interruption().is_some() =>
            {
                if let Some(interruption) = representative.operation.interruption() {
                    representative.terminate(interruption_outcome(interruption));
                }
                for (query, frame) in claimed {
                    if !Arc::ptr_eq(&query, &representative) {
                        query.release_claim(frame.stamp());
                    }
                }
                self.wake();
                return;
            }
            Err(error) => {
                for (query, frame) in claimed {
                    query.fail_mapping(frame.stamp(), error.clone());
                }
                return;
            }
        };
        let mapping = {
            let mut cache = lock(&self.cache);
            if cache_required
                && !self.closed.load(Ordering::Acquire)
                && self.query_count.load(Ordering::Acquire) != 0
            {
                cache.remember(&mapped)
            } else {
                None
            }
        };
        let mut participants = Vec::with_capacity(claimed.len());
        let mut admitted_owner = false;
        for (query, frame) in claimed {
            if query.terminal().is_some() {
                query.release_claim(frame.stamp());
                continue;
            }
            if let Some(interruption) = query.operation.interruption() {
                query.terminate(interruption_outcome(interruption));
                continue;
            }
            let request = MatchRequest::new(&frame, query.region, &query.template, query.options);
            if !mapped.is_equivalent_request(&request) {
                query.release_claim(frame.stamp());
                continue;
            }
            let decision = query.mapping_decision(&mapped);
            let now = query.operation.now();
            if let Admission::Admitted(generation) =
                query.finish_mapping(&mapped, mapping.clone(), decision, now)
            {
                let disposition_recorded = if admitted_owner {
                    false
                } else {
                    admitted_owner = query.record_disposition(TemplateWorkDisposition::Admitted);
                    admitted_owner
                };
                participants.push(Participant {
                    query,
                    generation,
                    disposition_recorded,
                });
            }
        }
        if !participants.is_empty() {
            self.run_analysis(mapped, mapping, participants);
        }
    }

    fn run_analysis(
        &self,
        mapped: MappedMatch,
        mapping: Option<Weak<CpuMapping>>,
        mut participants: Vec<Participant>,
    ) {
        let mut result = None;
        while result.is_none() {
            let Some(index) = participants.iter().position(|participant| {
                participant.query.terminal().is_none()
                    && participant.query.operation.interruption().is_none()
            }) else {
                break;
            };
            if !participants[index].disposition_recorded {
                let recorded = participants[index]
                    .query
                    .record_disposition(TemplateWorkDisposition::Admitted);
                participants[index].disposition_recorded = recorded;
                if !recorded {
                    continue;
                }
            }
            let representative = &participants[index];
            let attempt = self.matcher.find_mapped(
                &mapped,
                &representative.query.template,
                &representative.query.operation,
            );
            if attempt.as_ref().err().is_some_and(|error| {
                matches!(error.status(), Status::Cancelled | Status::DeadlineExceeded)
            }) && let Some(interruption) = representative.query.operation.interruption()
            {
                representative
                    .query
                    .terminate(interruption_outcome(interruption));
                continue;
            }
            result = Some(attempt);
        }
        let Some(result) = result else {
            return;
        };
        let result = result.map(ImmutableAnalysisResult::new);
        for participant in participants.drain(..) {
            if !participant.disposition_recorded {
                participant
                    .query
                    .record_disposition(TemplateWorkDisposition::Coalesced);
            }
            let now = participant.query.operation.now();
            participant.query.complete(
                participant.generation,
                &mapped,
                mapping.clone(),
                result.clone(),
                now,
            );
        }
    }

    fn wake(&self) {
        let mut state = lock(&self.wake_state);
        *state = state.wrapping_add(1);
        drop(state);
        self.wake_condition.notify_all();
    }

    pub(crate) fn close(&self) {
        let first = {
            let _admission = lock(&self.admission);
            !self.closed.swap(true, Ordering::AcqRel)
        };
        if !first {
            return;
        }
        {
            let mut cache = lock(&self.cache);
            cache.entries.clear();
            cache.bytes = 0;
        }
        let sessions = std::mem::take(&mut lock(&self.registry).sessions);
        for session in sessions {
            session.close(TemplateTerminalOutcome::SchedulerClosed);
        }
        self.wake();
        lock(&self.workers).clear();
        lock(&self.supervisor).take();
    }
}
fn validate_watch_region(region: RegionSelection) -> Result<()> {
    match region {
        RegionSelection::FullFrame => Ok(()),
        RegionSelection::Region { rect, .. } => {
            rect.require_non_empty().map(|_| ()).map_err(Error::from)
        }
        _ => Err(Error::new(
            Status::Unsupported,
            "template watch region selection is not supported",
        )),
    }
}

fn validate_watch_timing(request: &TemplateWatchRequest) -> Result<()> {
    let now = request.operation().now();
    let rate = request.rate().minimum_interval();
    if (!rate.is_zero() && now.checked_add(rate).is_none())
        || request
            .stability()
            .required_duration()
            .is_some_and(|duration| now.checked_add(duration).is_none())
    {
        return Err(Error::new(
            Status::InvalidArgument,
            "template watcher timing policy is not representable in the query clock domain",
        ));
    }
    Ok(())
}

fn release_work_claims(work: &ClaimedWork) {
    work.query.release_claim(work.frame.stamp());
    for (query, frame) in &work.peers {
        query.release_claim(frame.stamp());
    }
}

fn compare_deadline(
    left: &(Option<Duration>, usize, Arc<QueryShared>, FrameStamp),
    right: &(Option<Duration>, usize, Arc<QueryShared>, FrameStamp),
) -> CmpOrdering {
    match (left.0, right.0) {
        (Some(left_remaining), Some(right_remaining)) => left_remaining
            .cmp(&right_remaining)
            .then(left.1.cmp(&right.1)),
        (Some(_), None) => CmpOrdering::Less,
        (None, Some(_)) => CmpOrdering::Greater,
        (None, None) => left.1.cmp(&right.1),
    }
}

fn in_flight_count(state: &QueryData) -> u32 {
    state
        .in_flight
        .iter()
        .fold(0, |count, item| count + u32::from(item.is_some()))
}

fn query_frame(state: &QueryData) -> Option<FrameStamp> {
    state
        .terminal_frame
        .or_else(|| state.pending.as_ref().map(|pending| pending.frame.stamp()))
        .or(state.processing)
        .or_else(|| {
            state
                .in_flight
                .iter()
                .flatten()
                .max_by_key(|(generation, _)| *generation)
                .map(|(_, frame)| *frame)
        })
        .or(state.last_frame)
}

fn reserve(counter: &AtomicUsize, limit: usize, name: &'static str) -> Result<()> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < limit).then_some(current + 1)
        })
        .map(|_| ())
        .map_err(|_| Error::new(Status::LimitExceeded, format!("{name} was reached")))
}

const fn interruption_outcome(interruption: Interruption) -> TemplateTerminalOutcome {
    match interruption {
        Interruption::Cancelled => TemplateTerminalOutcome::Cancelled,
        Interruption::DeadlineExceeded => TemplateTerminalOutcome::DeadlineExceeded,
    }
}

fn duration_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

const fn template_watch_diagnostic_outcome(
    outcome: &TemplateTerminalOutcome,
) -> TemplateWatchDiagnosticOutcome {
    match outcome {
        TemplateTerminalOutcome::Matched(_) => TemplateWatchDiagnosticOutcome::Matched,
        TemplateTerminalOutcome::Cancelled => TemplateWatchDiagnosticOutcome::Cancelled,
        TemplateTerminalOutcome::DeadlineExceeded => {
            TemplateWatchDiagnosticOutcome::DeadlineExceeded
        }
        TemplateTerminalOutcome::SessionClosed => TemplateWatchDiagnosticOutcome::SessionClosed,
        TemplateTerminalOutcome::SchedulerClosed => TemplateWatchDiagnosticOutcome::SchedulerClosed,
        TemplateTerminalOutcome::TargetLost => TemplateWatchDiagnosticOutcome::TargetLost,
        TemplateTerminalOutcome::Overloaded(_) => TemplateWatchDiagnosticOutcome::Overloaded,
        TemplateTerminalOutcome::Failed(error) => {
            TemplateWatchDiagnosticOutcome::Failed(error.status())
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
