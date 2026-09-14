//! OCR text presence shares acquisition and dispatch with the template watcher.

use super::*;
use crate::diagnostic::{OcrTextWatchDiagnostic, OcrTextWatchDiagnosticOutcome};
use mado_pilot_capture::{FrameDescriptor, PixelFormat};
use mado_pilot_core::{CancellationToken, ClipPolicy, CoordinateSpace, GeometryFault, Rect};
use mado_pilot_ocr::watch_support::{PreparedOcr, normalize_literal, prepare};
use mado_pilot_ocr::{
    MAX_CANDIDATES, MAX_TEXT_BYTES, OcrBackendDescriptor, OcrExecutionProvider, OcrFault,
    OcrModelIdentity, OcrProviderDescriptor, OcrRecognizer, OcrRegion, OcrRequest, OcrResult,
    RecognizedRegion,
};
use std::mem::size_of;

const MAX_SOURCE_BYTES: usize = 256 * 1024 * 1024;
const CPU_BACKEND: &str = "onnxruntime-cpu";
const CPU_BACKEND_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+ort-1.29.0-api17");
const CPU_RUNTIME_PROFILE: &str = "onnxruntime-1.29.0-api17-cpu";

static ACCEPTED_MODEL: std::sync::LazyLock<OcrModelIdentity> =
    std::sync::LazyLock::new(OcrModelIdentity::accepted_bounded_detector);

/// A positive minimum interval between OCR backend admissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcrTextAnalysisRate(Duration);

impl OcrTextAnalysisRate {
    /// Refuses a zero interval with `InvalidArgument`.
    pub fn from_minimum_interval(interval: Duration) -> Result<Self> {
        if interval.is_zero() {
            return Err(Error::new(
                Status::InvalidArgument,
                "OCR analysis interval must be positive",
            ));
        }
        Ok(Self(interval))
    }

    /// Returns the minimum interval between admitted analyses.
    #[must_use]
    pub const fn minimum_interval(self) -> Duration {
        self.0
    }
}

/// The closed OCR confirmation selection; elapsed time never confirms text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OcrTextStabilityKind {
    /// Complete on the first accepted positive analysis.
    Immediate,
    /// Require distinct consecutive accepted positive analyses.
    Consecutive,
}

/// A validated immediate or consecutive OCR confirmation requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcrTextStability {
    kind: OcrTextStabilityKind,
    observations: NonZeroU32,
}

impl OcrTextStability {
    /// Completes on the first accepted positive analysis.
    #[must_use]
    pub const fn immediate() -> Self {
        Self {
            kind: OcrTextStabilityKind::Immediate,
            observations: NonZeroU32::MIN,
        }
    }

    /// Refuses zero observations with `InvalidArgument`.
    pub fn consecutive(observations: u32) -> Result<Self> {
        let observations = NonZeroU32::new(observations).ok_or_else(|| {
            Error::new(
                Status::InvalidArgument,
                "OCR confirmations must be positive",
            )
        })?;
        Ok(Self {
            kind: OcrTextStabilityKind::Consecutive,
            observations,
        })
    }

    /// Returns the selected kind.
    #[must_use]
    pub const fn kind(self) -> OcrTextStabilityKind {
        self.kind
    }
    /// Returns the required number of distinct positive observations.
    #[must_use]
    pub const fn required_observations(self) -> NonZeroU32 {
        self.observations
    }
}

/// One explicit, owned and normalized text-presence request.
#[derive(Clone)]
pub struct OcrTextWatchRequest {
    region: Rect,
    clip_policy: ClipPolicy,
    literal: Arc<str>,
    minimum_confidence: f64,
    output_space: CoordinateSpace,
    rate: OcrTextAnalysisRate,
    stability: OcrTextStability,
    change_policy: ChangeDetectionPolicy,
    operation: OperationContext,
}

impl OcrTextWatchRequest {
    /// Normalizes a borrowed literal under the query's lifetime authority.
    ///
    /// Returns interruption, invalid geometry/confidence/timing, or a normalized
    /// text limit failure. Edge whitespace has no raw-input length ceiling.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        region: Rect,
        clip_policy: ClipPolicy,
        literal: &str,
        minimum_confidence: f64,
        output_space: CoordinateSpace,
        rate: OcrTextAnalysisRate,
        stability: OcrTextStability,
        change_policy: ChangeDetectionPolicy,
        operation: OperationContext,
    ) -> Result<Self> {
        let attempt = Operation::admit(&operation)?;
        region.require_non_empty()?;
        if !minimum_confidence.is_finite() || !(0.0..=1.0).contains(&minimum_confidence) {
            return Err(Error::new(
                Status::InvalidArgument,
                "OCR confidence must be finite and in range",
            ));
        }
        if operation
            .now()
            .checked_add(rate.minimum_interval())
            .is_none()
        {
            return Err(Error::new(
                Status::InvalidArgument,
                "OCR interval is not representable",
            ));
        }
        let literal = normalize_literal(literal, &operation)?;
        attempt.commit(())?;
        Ok(Self {
            region,
            clip_policy,
            literal,
            minimum_confidence,
            output_space,
            rate,
            stability,
            change_policy,
            operation,
        })
    }

    /// Returns the requested coordinate-qualified rectangle.
    #[must_use]
    pub const fn region(&self) -> Rect {
        self.region
    }
    /// Returns the explicit clipping policy.
    #[must_use]
    pub const fn clip_policy(&self) -> ClipPolicy {
        self.clip_policy
    }
    /// Returns the inclusive confidence threshold without rounding it.
    #[must_use]
    pub const fn minimum_confidence(&self) -> f64 {
        self.minimum_confidence
    }
    /// Returns the requested output coordinate space.
    #[must_use]
    pub const fn output_space(&self) -> CoordinateSpace {
        self.output_space
    }
    /// Returns the analysis rate.
    #[must_use]
    pub const fn rate(&self) -> OcrTextAnalysisRate {
        self.rate
    }
    /// Returns the required confirmation rule.
    #[must_use]
    pub const fn stability(&self) -> OcrTextStability {
        self.stability
    }
    /// Returns the selected closed change policy.
    #[must_use]
    pub const fn change_policy(&self) -> ChangeDetectionPolicy {
        self.change_policy
    }
    /// Returns query-lifetime authority; an absent deadline remains unbounded.
    #[must_use]
    pub const fn operation(&self) -> &OperationContext {
        &self.operation
    }
}

impl fmt::Debug for OcrTextWatchRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OcrTextWatchRequest")
            .field("region", &self.region)
            .field("clip_policy", &self.clip_policy)
            .field("literal_bytes", &self.literal.len())
            .field("minimum_confidence", &self.minimum_confidence)
            .field("output_space", &self.output_space)
            .field("rate", &self.rate)
            .field("stability", &self.stability)
            .field("change_policy", &self.change_policy)
            .finish()
    }
}

/// One non-reused engine-local OCR query identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OcrTextQueryId(std::num::NonZeroU64);

impl OcrTextQueryId {
    /// Returns the nonzero identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Work-lifecycle transitions; more than one may apply to one work item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OcrTextWorkDisposition {
    /// Backend analysis admitted.
    Admitted,
    /// Exact compatible pixels skipped a routine negative analysis.
    SkippedChange,
    /// Analysis awaits its legal rate instant.
    DeferredRate,
    /// Reserved vocabulary; OCR coalescing is disabled and this count stays zero.
    Coalesced,
    /// Newer source or terminal authority displaced work.
    Superseded,
    /// Publication or admission was refused.
    Rejected,
    /// Eligible pending work exceeded fixed residence.
    QueueExpired,
    /// Validated positive or negative analysis completed.
    Completed,
    /// Mapping, capture or OCR failed.
    Failed,
}

/// Saturating, content-free work counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OcrTextWorkCounts([u64; 9]);

impl OcrTextWorkCounts {
    /// Returns the count for a work transition.
    #[must_use]
    pub const fn get(self, disposition: OcrTextWorkDisposition) -> u64 {
        self.0[disposition as usize]
    }
    fn increment(&mut self, disposition: OcrTextWorkDisposition) {
        let count = &mut self.0[disposition as usize];
        *count = count.saturating_add(1);
    }
}

/// Fixed finite policy that prevented further progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OcrTextOverload {
    /// Eligible pending work exceeded thirty seconds.
    QueueExpired,
}

/// Logical retained extent, not allocator overhead or native resident memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OcrTextRetainedExtent {
    source_bytes: u64,
    text_bytes: u64,
    index_bytes: u64,
    metadata_bytes: u64,
}

impl OcrTextRetainedExtent {
    /// Full source layout including stride padding.
    #[must_use]
    pub const fn source_bytes(self) -> u64 {
        self.source_bytes
    }
    /// Literal and complete normalized output UTF-8 bytes.
    #[must_use]
    pub const fn text_bytes(self) -> u64 {
        self.text_bytes
    }
    /// Compact satisfying-region indexes.
    #[must_use]
    pub const fn index_bytes(self) -> u64 {
        self.index_bytes
    }
    /// Fixed result and recognized-region metadata extent.
    #[must_use]
    pub const fn metadata_bytes(self) -> u64 {
        self.metadata_bytes
    }
    /// Sum of the checked logical extents, not de-duplicated native allocations.
    #[must_use]
    pub const fn total_bytes(self) -> u64 {
        self.source_bytes + self.text_bytes + self.index_bytes + self.metadata_bytes
    }
}

/// Exact immutable OCR success, independent of every parent owner.
#[derive(Clone)]
pub struct OcrTextWatchResult {
    target: TargetId,
    frame: Frame,
    result: Arc<OcrResult>,
    provider: OcrProviderDescriptor,
    literal: Arc<str>,
    minimum_confidence: f64,
    stability: OcrTextStability,
    confirmed_observations: u32,
    first_confirmed_frame: FrameStamp,
    satisfying_indexes: Arc<[u16]>,
    retention: Arc<RetainedResult>,
}

impl OcrTextWatchResult {
    /// Returns the exact captured target.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }
    /// Returns the exact satisfying source, never the session's newest frame.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        &self.frame
    }
    /// Returns complete validated OCR output and source/geometry/backend facts.
    #[must_use]
    pub fn result(&self) -> &OcrResult {
        &self.result
    }
    /// Returns the bound CPU runtime/provider initialization facts.
    #[must_use]
    pub const fn provider(&self) -> &OcrProviderDescriptor {
        &self.provider
    }
    /// Explicitly reads the normalized literal.
    #[must_use]
    pub fn literal(&self) -> &str {
        &self.literal
    }
    /// Returns the unrounded inclusive confidence threshold.
    #[must_use]
    pub const fn minimum_confidence(&self) -> f64 {
        self.minimum_confidence
    }
    /// Returns the requested stability rule.
    #[must_use]
    pub const fn stability(&self) -> OcrTextStability {
        self.stability
    }
    /// Returns distinct accepted positives at completion.
    #[must_use]
    pub const fn confirmed_observations(&self) -> u32 {
        self.confirmed_observations
    }
    /// Returns the first source in the final uninterrupted positive sequence.
    #[must_use]
    pub const fn first_confirmed_frame(&self) -> FrameStamp {
        self.first_confirmed_frame
    }
    /// Returns checked indexes in immutable detector order.
    #[must_use]
    pub fn satisfying_region_indexes(&self) -> &[u16] {
        &self.satisfying_indexes
    }
    /// Reads satisfying regions without copying their strings.
    pub fn matching_regions(&self) -> impl ExactSizeIterator<Item = &RecognizedRegion> {
        self.satisfying_indexes
            .iter()
            .map(|&index| &self.result.regions()[usize::from(index)])
    }
    /// Returns this result's bounded logical extent.
    #[must_use]
    pub fn retained_extent(&self) -> OcrTextRetainedExtent {
        self.retention.extent
    }
}

impl fmt::Debug for OcrTextWatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OcrTextWatchResult")
            .field("target", &self.target)
            .field("frame", &self.frame.stamp())
            .field("region", &self.result.effective_region())
            .field("output_space", &self.result.output_space())
            .field("confirmations", &self.confirmed_observations)
            .field("satisfying_regions", &self.satisfying_indexes.len())
            .field("retained_extent", &self.retained_extent())
            .finish()
    }
}

/// The single immutable winner of all query terminal authorities.
#[derive(Clone)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum OcrTextTerminalOutcome {
    /// Confirmed text in an exact retained source.
    Matched(OcrTextWatchResult),
    /// Explicit or external query cancellation.
    Cancelled,
    /// Query-lifetime deadline.
    DeadlineExceeded,
    /// Session closed or exhausted without satisfying final work.
    SessionClosed,
    /// Engine dispatch sealed.
    SchedulerClosed,
    /// Capture target lost.
    TargetLost,
    /// Fixed eligible residence exceeded.
    Overloaded(OcrTextOverload),
    /// Typed failure with content-redacted detail.
    Failed(Error),
}

impl OcrTextTerminalOutcome {
    /// Returns no status for a match, otherwise its typed terminal status.
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
    /// Reports a successful match.
    #[must_use]
    pub const fn is_match(&self) -> bool {
        matches!(self, Self::Matched(_))
    }
}

impl fmt::Debug for OcrTextTerminalOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Matched(result) => f.debug_tuple("Matched").field(result).finish(),
            _ => f
                .debug_struct("OcrTextTerminalOutcome")
                .field("outcome", &diagnostic_outcome(self))
                .finish(),
        }
    }
}

/// Coarse lifecycle of a text query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OcrTextQueryState {
    /// Work or source authority remains live.
    Pending,
    /// One outcome committed.
    Terminal,
}

/// Content-free query facts; physical occupancy survives logical cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcrTextQueryProgress {
    id: OcrTextQueryId,
    last_frame: Option<FrameStamp>,
    last_accepted_frame: Option<FrameStamp>,
    generation: u64,
    confirmed_observations: u32,
    work: OcrTextWorkCounts,
    pending_count: u32,
    in_flight_count: u32,
    physical_in_flight_count: u32,
}

impl OcrTextQueryProgress {
    /// Returns query identity.
    #[must_use]
    pub const fn id(self) -> OcrTextQueryId {
        self.id
    }
    /// Returns the newest source considered by acquisition.
    #[must_use]
    pub const fn last_frame(self) -> Option<FrameStamp> {
        self.last_frame
    }
    /// Returns the last fully validated authoritative analysis source.
    #[must_use]
    pub const fn last_accepted_frame(self) -> Option<FrameStamp> {
        self.last_accepted_frame
    }
    /// Returns the last issued generation, never reused.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }
    /// Returns distinct consecutive positive observations.
    #[must_use]
    pub const fn confirmed_observations(self) -> u32 {
        self.confirmed_observations
    }
    /// Returns work counts, frozen after terminal publication.
    #[must_use]
    pub const fn work(self) -> OcrTextWorkCounts {
        self.work
    }
    /// Returns zero or one latest pending sources.
    #[must_use]
    pub const fn pending_count(self) -> u32 {
        self.pending_count
    }
    /// Returns zero or one logically authoritative execution.
    #[must_use]
    pub const fn in_flight_count(self) -> u32 {
        self.in_flight_count
    }
    /// Returns zero or one physical execution, even after cancellation.
    #[must_use]
    pub const fn physical_in_flight_count(self) -> u32 {
        self.physical_in_flight_count
    }
}

/// One nonblocking observation.
#[derive(Debug, Clone)]
pub enum OcrTextQueryOutcome {
    /// Copied pending progress.
    Pending(OcrTextQueryProgress),
    /// Independently retained terminal authority.
    Terminal(Arc<OcrTextTerminalOutcome>),
}

impl OcrTextQueryOutcome {
    /// Returns coarse lifecycle.
    #[must_use]
    pub const fn state(&self) -> OcrTextQueryState {
        match self {
            Self::Pending(_) => OcrTextQueryState::Pending,
            Self::Terminal(_) => OcrTextQueryState::Terminal,
        }
    }
}

/// One owning poll/wait/cancel handle. Dropping it cancels pending work.
pub struct OcrTextQuery {
    shared: Arc<OcrQueryShared>,
    _threads: Arc<WatchThreadOwner>,
}

impl OcrTextQuery {
    /// Returns non-reused engine-local identity.
    #[must_use]
    pub fn id(&self) -> OcrTextQueryId {
        self.shared.id
    }
    /// Copies state without entering mapping or the backend.
    #[must_use]
    pub fn poll(&self) -> OcrTextQueryOutcome {
        let state = lock(&self.shared.state);
        match &state.terminal {
            Some(outcome) => OcrTextQueryOutcome::Terminal(Arc::clone(outcome)),
            None => OcrTextQueryOutcome::Pending(self.shared.progress(&state)),
        }
    }
    /// Observes work including physical retirement after a terminal outcome.
    #[must_use]
    pub fn progress(&self) -> OcrTextQueryProgress {
        self.shared.progress(&lock(&self.shared.state))
    }
    /// Waits under independent authority; interruption ends only this call.
    pub fn wait(&self, wait: &OperationContext) -> Result<Arc<OcrTextTerminalOutcome>> {
        loop {
            if let Some(interruption) = wait.interruption() {
                return Err(interruption.into());
            }
            let duration = wait
                .remaining()
                .map_or(WAIT_POLL, |left| left.min(WAIT_POLL));
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
    /// Proposes cancellation and returns the immutable winner. Idempotent.
    #[must_use]
    pub fn cancel(&self) -> Arc<OcrTextTerminalOutcome> {
        self.shared.terminate(OcrTextTerminalOutcome::Cancelled)
    }
}

impl fmt::Debug for OcrTextQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OcrTextQuery")
            .field("id", &self.id())
            .field("state", &self.poll().state())
            .finish()
    }
}
impl Drop for OcrTextQuery {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

/// Immutable engine OCR watcher safety policies, not measured workload budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcrTextSchedulerDescriptor;

impl OcrTextSchedulerDescriptor {
    /// Shared engine query capacity.
    #[must_use]
    pub const fn max_engine_queries(self) -> u32 {
        256
    }
    /// Shared active/retiring session capacity.
    #[must_use]
    pub const fn max_active_sessions(self) -> u32 {
        16
    }
    /// Shared per-session query capacity.
    #[must_use]
    pub const fn max_session_queries(self) -> u32 {
        64
    }
    /// Existing total analysis worker count.
    #[must_use]
    pub const fn max_in_flight_analyses(self) -> u32 {
        2
    }
    /// Physical OCR slots, held through conversion and backend return.
    #[must_use]
    pub const fn max_physical_ocr_analyses(self) -> u32 {
        1
    }
    /// Latest pending sources per query.
    #[must_use]
    pub const fn latest_pending_frames_per_query(self) -> u32 {
        1
    }
    /// Maximum eligible residence, excluding intentional rate deferral.
    #[must_use]
    pub const fn eligible_queue_expiry(self) -> Duration {
        ELIGIBLE_QUEUE_EXPIRY
    }
    /// Shared mapping cache bytes.
    #[must_use]
    pub const fn mapped_cache_bytes(self) -> u64 {
        MAPPED_CACHE_BYTES as u64
    }
    /// Shared mapping cache entries.
    #[must_use]
    pub const fn max_mapped_cache_entries(self) -> u32 {
        256
    }
    /// Full source layout and returned mapping safety ceiling.
    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        MAX_SOURCE_BYTES as u64
    }
    /// Normalized predicate UTF-8 ceiling.
    #[must_use]
    pub const fn max_literal_bytes(self) -> u32 {
        4096
    }
    /// Fully validated output region ceiling.
    #[must_use]
    pub const fn max_regions(self) -> u32 {
        1000
    }
    /// Normalized UTF-8 ceiling for each output region.
    #[must_use]
    pub const fn max_region_text_bytes(self) -> u32 {
        4096
    }
    /// Absolute bounded-v2 detector input ceiling, including small-source upscaling.
    #[must_use]
    pub const fn max_detector_input_tensor_bytes(self) -> u64 {
        1312 * 736 * 3 * 4
    }
    /// Aggregate normalized output plus predicate UTF-8 extent.
    #[must_use]
    pub const fn max_result_text_bytes(self) -> u64 {
        1000 * 4096 + 4096
    }
    /// Compact satisfying-index extent.
    #[must_use]
    pub const fn max_result_index_bytes(self) -> u64 {
        1000 * 2
    }
    /// OCR result coalescing is not admitted.
    #[must_use]
    pub const fn coalescing_enabled(self) -> bool {
        false
    }
}

/// Bounded current/high-water observations. Retention excludes separate frame clones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OcrTextSchedulerObservation {
    /// Shared live logical query reservations.
    pub query_count: u32,
    /// Shared active or physically retiring sessions.
    pub active_sessions: u32,
    /// Logical OCR executions.
    pub logical_ocr_in_flight: u32,
    /// Physical OCR executions, never freed by a logical terminal.
    pub physical_ocr_in_flight: u32,
    /// Maximum physical OCR executions observed.
    pub physical_ocr_high_water: u32,
    /// Reserved template mapping stages, including dispatched work.
    pub template_mapping_reservations: u32,
    /// OCR mapping is selected or physically active.
    pub mapping_barrier: bool,
    /// Current shared cache bytes.
    pub mapped_cache_bytes: u64,
    /// High-water shared cache bytes.
    pub mapped_cache_high_water_bytes: u64,
    /// Number of live successful result allocations, independent of handle clones.
    pub retained_results: u64,
    /// Sum of their logical extents; not RSS or de-duplicated storage bytes.
    pub retained_result_extent_bytes: u64,
    /// High-water sum of result logical extents.
    pub retained_result_high_water_bytes: u64,
}

#[derive(Debug, Default)]
pub(super) struct RetentionCounts {
    live: u64,
    bytes: u64,
    high_water: u64,
}

#[derive(Debug)]
struct RetainedResult {
    extent: OcrTextRetainedExtent,
    counts: Arc<Mutex<RetentionCounts>>,
}

impl RetainedResult {
    fn new(
        extent: OcrTextRetainedExtent,
        counts: &Arc<Mutex<RetentionCounts>>,
    ) -> Result<Arc<Self>> {
        let mut state = lock(counts);
        let live = state.live.checked_add(1).ok_or_else(retention_limit)?;
        let bytes = state
            .bytes
            .checked_add(extent.total_bytes())
            .ok_or_else(retention_limit)?;
        state.live = live;
        state.bytes = bytes;
        state.high_water = state.high_water.max(bytes);
        drop(state);
        Ok(Arc::new(Self {
            extent,
            counts: Arc::clone(counts),
        }))
    }
}
impl Drop for RetainedResult {
    fn drop(&mut self) {
        let mut counts = lock(&self.counts);
        counts.live -= 1;
        counts.bytes -= self.extent.total_bytes();
    }
}

fn retention_limit() -> Error {
    Error::new(
        Status::LimitExceeded,
        "OCR retained extent is not representable",
    )
}
fn redacted_failure(error: Error) -> OcrTextTerminalOutcome {
    OcrTextTerminalOutcome::Failed(Error::new(
        error.status(),
        "OCR text watch operation failed",
    ))
}
fn interrupted(
    context: &OperationContext,
    now: MonotonicInstant,
) -> Option<OcrTextTerminalOutcome> {
    if context
        .cancellation()
        .is_some_and(CancellationToken::is_cancelled)
    {
        Some(OcrTextTerminalOutcome::Cancelled)
    } else if context.deadline().is_some_and(|deadline| now >= deadline) {
        Some(OcrTextTerminalOutcome::DeadlineExceeded)
    } else {
        None
    }
}

fn refresh_interruption(
    authority: Option<OcrTextTerminalOutcome>,
    context: &OperationContext,
    now: MonotonicInstant,
) -> Option<OcrTextTerminalOutcome> {
    match authority {
        None
        | Some(OcrTextTerminalOutcome::Cancelled | OcrTextTerminalOutcome::DeadlineExceeded) => {
            interrupted(context, now).or(authority)
        }
        _ => authority,
    }
}
pub(super) fn source_outcome(outcome: &TemplateTerminalOutcome) -> OcrTextTerminalOutcome {
    match outcome {
        TemplateTerminalOutcome::TargetLost => OcrTextTerminalOutcome::TargetLost,
        TemplateTerminalOutcome::SchedulerClosed => OcrTextTerminalOutcome::SchedulerClosed,
        TemplateTerminalOutcome::Failed(error) => redacted_failure(error.clone()),
        _ => OcrTextTerminalOutcome::SessionClosed,
    }
}
fn diagnostic_outcome(outcome: &OcrTextTerminalOutcome) -> OcrTextWatchDiagnosticOutcome {
    match outcome {
        OcrTextTerminalOutcome::Matched(_) => OcrTextWatchDiagnosticOutcome::Matched,
        OcrTextTerminalOutcome::Cancelled => OcrTextWatchDiagnosticOutcome::Cancelled,
        OcrTextTerminalOutcome::DeadlineExceeded => OcrTextWatchDiagnosticOutcome::DeadlineExceeded,
        OcrTextTerminalOutcome::SessionClosed => OcrTextWatchDiagnosticOutcome::SessionClosed,
        OcrTextTerminalOutcome::SchedulerClosed => OcrTextWatchDiagnosticOutcome::SchedulerClosed,
        OcrTextTerminalOutcome::TargetLost => OcrTextWatchDiagnosticOutcome::TargetLost,
        OcrTextTerminalOutcome::Overloaded(_) => OcrTextWatchDiagnosticOutcome::Overloaded,
        OcrTextTerminalOutcome::Failed(error) => {
            OcrTextWatchDiagnosticOutcome::Failed(error.status())
        }
    }
}

#[derive(Debug)]
pub(super) struct OcrQueryShared {
    id: OcrTextQueryId,
    target: TargetId,
    request: OcrTextWatchRequest,
    recognizer: OcrRecognizer,
    descriptor: OcrBackendDescriptor,
    provider: OcrProviderDescriptor,
    session: Weak<WatchSession>,
    scheduler: Weak<WatchScheduler>,
    state: Mutex<OcrQueryData>,
    changed: Condvar,
    physical: AtomicBool,
    reservation_released: AtomicBool,
    diagnostic: Option<ObservedOperation>,
    diagnostic_emission: Mutex<()>,
    started_at: MonotonicInstant,
}

#[derive(Debug)]
struct OcrQueryData {
    terminal: Option<Arc<OcrTextTerminalOutcome>>,
    needs_current: bool,
    source: Option<FrameStamp>,
    accepted: Option<FrameStamp>,
    pending: Option<PendingFrame>,
    rate_eligible_at: Option<MonotonicInstant>,
    generation: u64,
    execution: Option<OcrExecution>,
    previous_mapping: Option<Weak<CpuMapping>>,
    last_admitted: Option<MonotonicInstant>,
    confirmed: u32,
    first_confirmed: Option<FrameStamp>,
    effective_region: Option<PixelRect>,
    work: OcrTextWorkCounts,
}

#[derive(Debug)]
struct OcrExecution {
    generation: u64,
    stamp: FrameStamp,
    cancellation: CancellationToken,
}

pub(super) struct OcrWork {
    query: Arc<OcrQueryShared>,
    session: Arc<WatchSession>,
    frame: Frame,
    generation: u64,
    operation: OperationContext,
}

impl OcrQueryShared {
    fn progress(&self, state: &OcrQueryData) -> OcrTextQueryProgress {
        OcrTextQueryProgress {
            id: self.id,
            last_frame: state.source,
            last_accepted_frame: state.accepted,
            generation: state.generation,
            confirmed_observations: state.confirmed,
            work: state.work,
            pending_count: u32::from(state.pending.is_some()),
            in_flight_count: u32::from(state.execution.is_some()),
            physical_in_flight_count: u32::from(self.physical.load(Ordering::Acquire)),
        }
    }

    pub(super) fn needs_current(&self) -> bool {
        let state = lock(&self.state);
        state.terminal.is_none() && state.needs_current
    }

    // OCR source publication never waits for its physical slot or conversion.
    pub(super) fn considered(&self, stamp: FrameStamp) -> bool {
        let state = lock(&self.state);
        state.terminal.is_some()
            || state
                .source
                .is_some_and(|source| !matches!(source.order(&stamp), Ok(FrameOrder::Before)))
    }

    fn authority(
        &self,
        session: Option<&WatchSession>,
        scheduler: Option<&WatchScheduler>,
        now: MonotonicInstant,
    ) -> Option<OcrTextTerminalOutcome> {
        session
            .and_then(|session| {
                lock(&session.terminal_authority)
                    .as_ref()
                    .map(source_outcome)
            })
            .or_else(|| {
                session
                    .filter(|session| session.closed.load(Ordering::Acquire))
                    .map(|_| OcrTextTerminalOutcome::SessionClosed)
            })
            .or_else(|| {
                scheduler
                    .is_none_or(|scheduler| scheduler.closed.load(Ordering::Acquire))
                    .then_some(OcrTextTerminalOutcome::SchedulerClosed)
            })
            .or_else(|| interrupted(&self.request.operation, now))
    }

    fn clear_work(state: &mut OcrQueryData) -> Option<PendingFrame> {
        let pending = state.pending.take();
        if pending.is_some() {
            state.work.increment(OcrTextWorkDisposition::Superseded);
        }
        if let Some(execution) = state.execution.take() {
            execution.cancellation.cancel();
            state.work.increment(OcrTextWorkDisposition::Superseded);
        }
        state.previous_mapping = None;
        state.rate_eligible_at = None;
        pending
    }

    pub(super) fn terminate(
        &self,
        proposed: OcrTextTerminalOutcome,
    ) -> Arc<OcrTextTerminalOutcome> {
        let now = self.request.operation.now();
        let scheduler = self.scheduler.upgrade();
        let admission = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let session = self.session.upgrade();
        let activation = session.as_ref().map(|session| lock(&session.activation));
        let authority = self.authority(session.as_deref(), scheduler.as_deref(), now);
        let mut state = lock(&self.state);
        if let Some(existing) = &state.terminal {
            return Arc::clone(existing);
        }
        let authority = refresh_interruption(authority, &self.request.operation, now);
        let proposed = match (authority, proposed) {
            (
                None | Some(OcrTextTerminalOutcome::DeadlineExceeded),
                OcrTextTerminalOutcome::Cancelled,
            ) => OcrTextTerminalOutcome::Cancelled,
            (Some(authority), _) => authority,
            (None, proposed) => proposed,
        };
        let candidate = Arc::new(proposed);
        let pending = Self::clear_work(&mut state);
        state.terminal = Some(Arc::clone(&candidate));
        drop(state);
        drop(activation);
        drop(admission);
        drop(pending);
        self.after_terminal();
        candidate
    }

    fn after_terminal(&self) {
        self.release_reservation();
        self.emit(None, true);
        if let Some(session) = self.session.upgrade() {
            session.unregister_ocr(self.id);
            session.notify_progress();
        }
        self.changed.notify_all();
        if let Some(scheduler) = self.scheduler.upgrade() {
            scheduler.wake();
        }
    }

    fn release_reservation(&self) {
        if !self.reservation_released.swap(true, Ordering::AcqRel) {
            if let Some(scheduler) = self.scheduler.upgrade() {
                scheduler.release_query_reservation();
            }
            if let Some(session) = self.session.upgrade() {
                session.release_query_slot();
            }
        }
    }

    fn emit(&self, disposition: Option<OcrTextWorkDisposition>, terminal: bool) {
        let Some(observed) = self.diagnostic else {
            return;
        };
        let Some(scheduler) = self.scheduler.upgrade() else {
            return;
        };
        let Some(diagnostics) = &scheduler.diagnostics else {
            return;
        };
        if !terminal && !diagnostics.admits_debug() {
            return;
        }
        let now = self.request.operation.now();
        let _emission = lock(&self.diagnostic_emission);
        let state = lock(&self.state);
        if !terminal && state.terminal.is_some() {
            return;
        }
        let payload = OcrTextWatchDiagnostic {
            query: Some(self.id),
            target: self.target,
            frame: state.source,
            region: state.effective_region,
            progress: Some(self.progress(&state)),
            disposition,
            outcome: state.terminal.as_deref().map(diagnostic_outcome),
            elapsed_nanos: duration_nanos(now.saturating_duration_since(self.started_at)),
        };
        drop(state);
        if terminal {
            diagnostics.normal_at(observed, now, || DiagnosticPayload::OcrTextWatch(payload));
        } else {
            diagnostics.debug_at(observed, now, || DiagnosticPayload::OcrTextWatch(payload));
        }
    }

    pub(super) fn enqueue(&self, frame: Frame) {
        let stamp = frame.stamp();
        {
            let state = lock(&self.state);
            if state.terminal.is_some()
                || state
                    .source
                    .is_some_and(|last| !matches!(last.order(&stamp), Ok(FrameOrder::Before)))
            {
                return;
            }
        }
        // Full native conversion can precede cropping. Refuse before retaining
        // this source, not merely after a small ROI has been mapped.
        let geometry = self.resolve_frame(&frame);
        let now = self.request.operation.now();
        let scheduler = self.scheduler.upgrade();
        let admission = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let session = self.session.upgrade();
        let activation = session.as_ref().map(|session| lock(&session.activation));
        if let Some(outcome) = self.authority(session.as_deref(), scheduler.as_deref(), now) {
            drop(activation);
            drop(admission);
            self.terminate(outcome);
            return;
        }
        let mut state = lock(&self.state);
        if state.terminal.is_some()
            || session
                .as_ref()
                .is_none_or(|session| session.description.stream() != stamp.stream())
            || state
                .source
                .is_some_and(|last| !matches!(last.order(&stamp), Ok(FrameOrder::Before)))
        {
            return;
        }
        let incompatible = state.source.is_some_and(|last| {
            last.epoch() != stamp.epoch() || last.geometry() != stamp.geometry()
        });
        state.needs_current = false;
        state.source = Some(stamp);
        if incompatible {
            state.previous_mapping = None;
            state.confirmed = 0;
            state.first_confirmed = None;
            state.accepted = None;
            if let Some(execution) = state.execution.take() {
                execution.cancellation.cancel();
                state.work.increment(OcrTextWorkDisposition::Superseded);
            }
        }
        let effective = match geometry {
            Ok(region) => region,
            Err(error) => {
                drop(state);
                drop(activation);
                drop(admission);
                self.terminate(redacted_failure(error));
                return;
            }
        };
        state.effective_region = Some(effective);
        let rate_at = state
            .last_admitted
            .and_then(|last| last.checked_add(self.request.rate.0));
        let eligible_since = state.pending.as_ref().map_or_else(
            || rate_at.filter(|eligible| *eligible > now).unwrap_or(now),
            |pending| pending.eligible_since,
        );
        let old = state.pending.replace(PendingFrame {
            frame,
            eligible_since,
        });
        if old.is_some() {
            state.work.increment(OcrTextWorkDisposition::Superseded);
        }
        let deferred = old.is_none() && rate_at.is_some_and(|eligible| now < eligible);
        if old.is_none() {
            state.rate_eligible_at = rate_at.filter(|eligible| *eligible > now);
        }
        if deferred {
            state.work.increment(OcrTextWorkDisposition::DeferredRate);
        }
        drop(state);
        drop(activation);
        drop(admission);
        if old.is_some() {
            self.emit(Some(OcrTextWorkDisposition::Superseded), false);
        }
        drop(old);
        if deferred {
            self.emit(Some(OcrTextWorkDisposition::DeferredRate), false);
        }
        self.changed.notify_all();
    }

    fn resolve_frame(&self, frame: &Frame) -> Result<PixelRect> {
        if frame.descriptor().byte_len() > MAX_SOURCE_BYTES {
            return Err(OcrFault::MappingAboveCeiling.into());
        }
        let transform = frame.transform();
        let region =
            transform.resolve_capture_pixels(self.request.region, self.request.clip_policy)?;
        if !transform.supports(self.request.output_space) {
            return Err(GeometryFault::ConversionUnsupported.into());
        }
        transform.convert_point(
            mado_pilot_core::Point::new(
                CoordinateSpace::CapturePixels,
                f64::from(region.left()),
                f64::from(region.top()),
            )?,
            self.request.output_space,
        )?;
        let mapped_bytes =
            FrameDescriptor::packed(region.extent(), self.descriptor.format())?.byte_len();
        if mapped_bytes > MAX_SOURCE_BYTES {
            return Err(OcrFault::MappingAboveCeiling.into());
        }
        Ok(region)
    }

    // Called without scheduler admission: clock implementations may be host code.
    fn ready(&self) -> Option<(FrameStamp, MonotonicInstant)> {
        self.sweep();
        let now = self.request.operation.now();
        let state = lock(&self.state);
        if state.terminal.is_some()
            || state.execution.is_some()
            || self.physical.load(Ordering::Acquire)
            || state
                .rate_eligible_at
                .is_some_and(|eligible| now < eligible)
        {
            return None;
        }
        state
            .pending
            .as_ref()
            .map(|pending| (pending.frame.stamp(), now))
    }

    pub(super) fn sweep(&self) {
        let now = self.request.operation.now();
        let scheduler = self.scheduler.upgrade();
        let admission = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let session = self.session.upgrade();
        let activation = session.as_ref().map(|session| lock(&session.activation));
        let authority = self.authority(session.as_deref(), scheduler.as_deref(), now);
        let mut state = lock(&self.state);
        if state.terminal.is_some() {
            return;
        }
        let authority = refresh_interruption(authority, &self.request.operation, now);
        let expired = state.pending.as_ref().is_some_and(|pending| {
            now.saturating_duration_since(pending.eligible_since) > ELIGIBLE_QUEUE_EXPIRY
        });
        let source_ended = session
            .as_ref()
            .is_some_and(|session| session.source_ended.load(Ordering::Acquire));
        let exhausted =
            state.generation == u64::MAX && state.pending.is_some() && state.execution.is_none();
        let timing_exhausted = state.pending.is_some()
            && state
                .last_admitted
                .is_some_and(|last| last.checked_add(self.request.rate.0).is_none());
        let outcome = authority
            .or_else(|| {
                expired.then_some(OcrTextTerminalOutcome::Overloaded(
                    OcrTextOverload::QueueExpired,
                ))
            })
            .or_else(|| exhausted.then(|| redacted_failure(retention_limit())))
            .or_else(|| {
                timing_exhausted.then(|| {
                    redacted_failure(Error::new(
                        Status::InvalidArgument,
                        "OCR rate instant is not representable",
                    ))
                })
            })
            .or_else(|| {
                (source_ended && state.pending.is_none() && state.execution.is_none())
                    .then_some(OcrTextTerminalOutcome::SessionClosed)
            });
        let Some(outcome) = outcome else {
            return;
        };
        if expired && matches!(outcome, OcrTextTerminalOutcome::Overloaded(_)) {
            state.work.increment(OcrTextWorkDisposition::QueueExpired);
        }
        let pending = Self::clear_work(&mut state);
        state.terminal = Some(Arc::new(outcome));
        drop(state);
        drop(activation);
        drop(admission);
        drop(pending);
        self.after_terminal();
    }

    // Scheduler admission and session activation are held by the caller.
    fn claim(
        self: &Arc<Self>,
        session: &Arc<WatchSession>,
        stamp: FrameStamp,
        now: MonotonicInstant,
    ) -> Option<OcrWork> {
        if interrupted(&self.request.operation, now).is_some() {
            return None;
        }
        let cancellation = CancellationToken::new();
        let operation = self
            .request
            .operation
            .clone()
            .with_cancellation(cancellation.clone());
        let mut state = lock(&self.state);
        if state.terminal.is_some()
            || state.execution.is_some()
            || self.physical.load(Ordering::Acquire)
            || state
                .pending
                .as_ref()
                .is_none_or(|pending| pending.frame.stamp() != stamp)
            || state
                .rate_eligible_at
                .is_some_and(|eligible| now < eligible)
            || interrupted(&self.request.operation, now).is_some()
        {
            return None;
        }
        let generation = state.generation.checked_add(1)?;
        let pending = state.pending.take()?;
        state.generation = generation;
        state.rate_eligible_at = None;
        state.execution = Some(OcrExecution {
            generation,
            stamp,
            cancellation,
        });
        self.physical.store(true, Ordering::Release);
        session.physical_ocr.fetch_add(1, Ordering::AcqRel);
        Some(OcrWork {
            query: Arc::clone(self),
            session: Arc::clone(session),
            frame: pending.frame,
            generation,
            operation,
        })
    }

    fn execution_current(state: &OcrQueryData, work: &OcrWork) -> bool {
        state.terminal.is_none()
            && state.execution.as_ref().is_some_and(|execution| {
                execution.generation == work.generation && execution.stamp == work.frame.stamp()
            })
    }

    fn admit_prepared(
        &self,
        work: &OcrWork,
        prepared: &PreparedOcr,
        mapping: Option<Weak<CpuMapping>>,
        unchanged: bool,
    ) -> bool {
        let now = self.request.operation.now();
        let scheduler = self.scheduler.upgrade();
        let admission = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let activation = lock(&work.session.activation);
        if let Some(outcome) = self.authority(Some(&work.session), scheduler.as_deref(), now) {
            drop(activation);
            drop(admission);
            self.terminate(outcome);
            return false;
        }
        let mut state = lock(&self.state);
        if !Self::execution_current(&state, work) {
            return false;
        }
        if let Some(outcome) = interrupted(&self.request.operation, now) {
            drop(state);
            drop(activation);
            drop(admission);
            self.terminate(outcome);
            return false;
        }
        state.effective_region = Some(prepared.effective_region());
        if unchanged && state.confirmed == 0 {
            state.execution = None;
            state.previous_mapping = mapping;
            state.work.increment(OcrTextWorkDisposition::SkippedChange);
            drop(state);
            drop(activation);
            drop(admission);
            self.emit(Some(OcrTextWorkDisposition::SkippedChange), false);
            self.sweep();
            return false;
        }
        state.last_admitted = Some(now);
        state.work.increment(OcrTextWorkDisposition::Admitted);
        // A newer obligation observed during mapping must inherit this admission's
        // legal rate instant, while an older eligible obligation never gets younger.
        if let Some(eligible) = now.checked_add(self.request.rate.0)
            && let Some(pending) = state.pending.as_mut()
        {
            pending.eligible_since = pending.eligible_since.max(eligible);
            state.rate_eligible_at = Some(eligible);
            state.work.increment(OcrTextWorkDisposition::DeferredRate);
        }
        drop(state);
        drop(activation);
        drop(admission);
        self.emit(Some(OcrTextWorkDisposition::Admitted), false);
        true
    }

    fn complete(
        &self,
        work: &OcrWork,
        mapping: Option<Weak<CpuMapping>>,
        output: Result<OcrResult>,
    ) {
        let (confirmed, first) = {
            let state = lock(&self.state);
            if !Self::execution_current(&state, work) {
                return;
            }
            (state.confirmed, state.first_confirmed)
        };
        let evaluated = output.and_then(|output| self.evaluate(work, output, confirmed, first));
        let now = self.request.operation.now();
        let scheduler = self.scheduler.upgrade();
        let admission = scheduler
            .as_ref()
            .map(|scheduler| lock(&scheduler.admission));
        let activation = lock(&work.session.activation);
        let authority = self.authority(Some(&work.session), scheduler.as_deref(), now);
        let mut state = lock(&self.state);
        if !Self::execution_current(&state, work) {
            return;
        }
        let mut terminal =
            refresh_interruption(authority, &self.request.operation, now).map(Arc::new);
        let authority_discard = terminal.is_some();
        let mut disposition = None;
        if terminal.is_none() {
            match evaluated {
                Err(error) => {
                    state.work.increment(OcrTextWorkDisposition::Failed);
                    terminal = Some(Arc::new(redacted_failure(error)));
                }
                Ok((positive, result)) => {
                    state.work.increment(OcrTextWorkDisposition::Completed);
                    state.accepted = Some(work.frame.stamp());
                    state.previous_mapping = mapping;
                    state.confirmed = if positive {
                        confirmed.saturating_add(1)
                    } else {
                        0
                    };
                    state.first_confirmed = if positive {
                        first.or(Some(work.frame.stamp()))
                    } else {
                        None
                    };
                    terminal =
                        result.map(|result| Arc::new(OcrTextTerminalOutcome::Matched(result)));
                    disposition = Some(OcrTextWorkDisposition::Completed);
                }
            }
        }
        if !authority_discard {
            state.execution = None;
        }
        let pending = if let Some(outcome) = terminal {
            let pending = Self::clear_work(&mut state);
            state.terminal = Some(outcome);
            pending
        } else {
            None
        };
        let terminal = state.terminal.is_some();
        drop(state);
        drop(activation);
        drop(admission);
        drop(pending);
        if terminal {
            self.after_terminal();
        } else {
            self.emit(disposition, false);
            self.changed.notify_all();
            self.sweep();
        }
    }

    fn evaluate(
        &self,
        work: &OcrWork,
        output: OcrResult,
        confirmed: u32,
        first: Option<FrameStamp>,
    ) -> Result<(bool, Option<OcrTextWatchResult>)> {
        if output.stamp() != work.frame.stamp()
            || output.transform() != work.frame.transform()
            || output.backend() != &self.descriptor
            || output.output_space() != self.request.output_space
            || output.effective_region() != self.resolve_frame(&work.frame)?
        {
            return Err(Error::new(
                Status::Internal,
                "OCR source correlation failed",
            ));
        }
        let regions = output.regions();
        if regions.len() > MAX_CANDIDATES {
            return Err(retention_limit());
        }
        let mut indexes = [0_u16; MAX_CANDIDATES];
        let mut satisfying = 0;
        let mut text_bytes = self.request.literal.len();
        for (index, region) in regions.iter().enumerate() {
            if let Some(interruption) = self.request.operation.interruption() {
                return Err(interruption.into());
            }
            if region.text().len() > MAX_TEXT_BYTES {
                return Err(retention_limit());
            }
            text_bytes = text_bytes
                .checked_add(region.text().len())
                .ok_or_else(retention_limit)?;
            if region.confidence().get() >= self.request.minimum_confidence
                && region.text().contains(self.request.literal.as_ref())
            {
                indexes[satisfying] = u16::try_from(index).map_err(|_| retention_limit())?;
                satisfying += 1;
            }
        }
        if satisfying == 0 {
            return Ok((false, None));
        }
        let confirmed = confirmed.saturating_add(1);
        if confirmed < self.request.stability.observations.get() {
            return Ok((true, None));
        }
        let index_bytes = satisfying
            .checked_mul(size_of::<u16>())
            .ok_or_else(retention_limit)?;
        let metadata_bytes = regions
            .len()
            .checked_mul(size_of::<RecognizedRegion>())
            .and_then(|bytes| {
                bytes.checked_add(size_of::<OcrTextWatchResult>() + size_of::<OcrResult>())
            })
            .ok_or_else(retention_limit)?;
        let extent = OcrTextRetainedExtent {
            source_bytes: u64::try_from(work.frame.descriptor().byte_len())
                .map_err(|_| retention_limit())?,
            text_bytes: u64::try_from(text_bytes).map_err(|_| retention_limit())?,
            index_bytes: u64::try_from(index_bytes).map_err(|_| retention_limit())?,
            metadata_bytes: u64::try_from(metadata_bytes).map_err(|_| retention_limit())?,
        };
        let scheduler = self
            .scheduler
            .upgrade()
            .ok_or_else(|| Error::new(Status::Closed, "OCR scheduler closed"))?;
        let counts = &scheduler.retention;
        let retention = RetainedResult::new(extent, counts)?;
        Ok((
            true,
            Some(OcrTextWatchResult {
                target: self.target,
                frame: work.frame.clone(),
                result: Arc::new(output),
                provider: self.provider.clone(),
                literal: Arc::clone(&self.request.literal),
                minimum_confidence: self.request.minimum_confidence,
                stability: self.request.stability,
                confirmed_observations: confirmed,
                first_confirmed_frame: first.unwrap_or(work.frame.stamp()),
                satisfying_indexes: Arc::from(&indexes[..satisfying]),
                retention,
            }),
        ))
    }
}

impl Drop for OcrQueryShared {
    fn drop(&mut self) {
        self.release_reservation();
    }
}

impl WatchSession {
    pub(crate) fn start_ocr_query(
        self: &Arc<Self>,
        request: OcrTextWatchRequest,
        recognizer: Option<&OcrRecognizer>,
    ) -> Result<OcrTextQuery> {
        let scheduler = self
            .scheduler
            .upgrade()
            .ok_or_else(|| Error::new(Status::Closed, "OCR scheduler is unavailable"))?;
        let started_at = request.operation.now();
        let diagnostic = scheduler
            .diagnostics
            .as_ref()
            .map(|sink| sink.observe(&request.operation, DiagnosticOperationKind::OcrTextWatch))
            .transpose()?;
        let operation = request.operation.clone();
        let result =
            self.publish_ocr_query(request, recognizer, &scheduler, diagnostic, started_at);
        if let Err(error) = &result
            && let (Some(sink), Some(observed)) = (&scheduler.diagnostics, diagnostic)
        {
            let now = operation.now();
            sink.normal_at(observed, now, || {
                DiagnosticPayload::OcrTextWatch(OcrTextWatchDiagnostic {
                    query: None,
                    target: self.description.target(),
                    frame: None,
                    region: None,
                    progress: None,
                    disposition: Some(OcrTextWorkDisposition::Rejected),
                    outcome: Some(OcrTextWatchDiagnosticOutcome::Failed(error.status())),
                    elapsed_nanos: duration_nanos(now.saturating_duration_since(started_at)),
                })
            });
        }
        result
    }

    fn publish_ocr_query(
        self: &Arc<Self>,
        request: OcrTextWatchRequest,
        recognizer: Option<&OcrRecognizer>,
        scheduler: &Arc<WatchScheduler>,
        diagnostic: Option<ObservedOperation>,
        started_at: MonotonicInstant,
    ) -> Result<OcrTextQuery> {
        let context = request.operation.clone();
        let publication = Operation::admit(&context)?;
        let recognizer = recognizer.ok_or_else(|| Error::from(OcrFault::BackendUnavailable))?;
        let descriptor = recognizer.descriptor();
        let provider = recognizer
            .provider_descriptor()
            .ok_or_else(unsupported_selection)?;
        validate_selection(&descriptor, &provider)?;
        validate_request_geometry(&request, &self.description)?;
        if self.closed.load(Ordering::Acquire)
            || self.source_ended.load(Ordering::Acquire)
            || !self.capture.is_open()
            || scheduler.closed.load(Ordering::Acquire)
        {
            return Err(Error::new(Status::Closed, "OCR watch authority is closed"));
        }
        let threads = self
            .threads
            .upgrade()
            .ok_or_else(|| Error::new(Status::Closed, "OCR thread owner is unavailable"))?;
        scheduler.ensure_workers()?;
        reserve(
            &scheduler.query_count,
            MAX_ENGINE_QUERIES,
            "engine watcher query capacity",
        )?;
        if let Err(error) = self.reserve_query_slot(scheduler) {
            scheduler.release_query_reservation();
            return Err(error);
        }
        let id = match scheduler.issue_query_id() {
            Some(id) => OcrTextQueryId(id.0),
            None => {
                scheduler.release_query_reservation();
                self.release_query_slot();
                return Err(Error::new(
                    Status::LimitExceeded,
                    "OCR query identity exhausted",
                ));
            }
        };
        let query = Arc::new(OcrQueryShared {
            id,
            target: self.description.target(),
            request,
            recognizer: recognizer.clone(),
            descriptor,
            provider,
            session: Arc::downgrade(self),
            scheduler: Arc::downgrade(scheduler),
            state: Mutex::new(OcrQueryData {
                terminal: None,
                needs_current: true,
                source: None,
                accepted: None,
                pending: None,
                rate_eligible_at: None,
                generation: 0,
                execution: None,
                previous_mapping: None,
                last_admitted: None,
                confirmed: 0,
                first_confirmed: None,
                effective_region: None,
                work: OcrTextWorkCounts::default(),
            }),
            changed: Condvar::new(),
            physical: AtomicBool::new(false),
            reservation_released: AtomicBool::new(false),
            diagnostic,
            diagnostic_emission: Mutex::new(()),
            started_at,
        });
        publication.commit(())?;
        {
            let _admission = lock(&scheduler.admission);
            let _activation = lock(&self.activation);
            if self.closed.load(Ordering::Acquire)
                || self.source_ended.load(Ordering::Acquire)
                || scheduler.closed.load(Ordering::Acquire)
            {
                return Err(Error::new(
                    Status::Closed,
                    "OCR authority closed before publication",
                ));
            }
            let mut state = lock(&self.state);
            state.ocr_queries.retain(|query| query.strong_count() != 0);
            state
                .ocr_queries
                .try_reserve(1)
                .map_err(|_| retention_limit())?;
            state.ocr_queries.push(Arc::downgrade(&query));
            scheduler.ocr_used.store(true, Ordering::Release);
        }
        if let Err(error) = self.ensure_acquisition() {
            query.terminate(redacted_failure(error.clone()));
            return Err(error);
        }
        scheduler.wake();
        Ok(OcrTextQuery {
            shared: query,
            _threads: threads,
        })
    }

    pub(super) fn ocr_snapshot(&self) -> Vec<Arc<OcrQueryShared>> {
        let mut state = lock(&self.state);
        let mut queries = Vec::with_capacity(state.ocr_queries.len());
        state.ocr_queries.retain(|query| match query.upgrade() {
            Some(query) => {
                queries.push(query);
                true
            }
            None => false,
        });
        queries
    }

    fn unregister_ocr(&self, id: OcrTextQueryId) {
        lock(&self.state)
            .ocr_queries
            .retain(|query| query.upgrade().is_some_and(|query| query.id != id));
    }
}

fn unsupported_selection() -> Error {
    Error::new(
        Status::Unsupported,
        "OCR text watch requires the complete initialized CPU bounded-v2 identity",
    )
}

fn validate_selection(
    descriptor: &OcrBackendDescriptor,
    provider: &OcrProviderDescriptor,
) -> Result<()> {
    if descriptor.model_identity() != &*ACCEPTED_MODEL
        || descriptor.id().as_str() != CPU_BACKEND
        || descriptor.version().as_str() != CPU_BACKEND_VERSION
        || descriptor.format() != PixelFormat::Bgra8
        || provider.active_provider() != OcrExecutionProvider::Cpu
        || provider.runtime_profile().as_str() != CPU_RUNTIME_PROFILE
    {
        return Err(unsupported_selection());
    }
    Ok(())
}

fn validate_request_geometry(
    request: &OcrTextWatchRequest,
    description: &SessionDescription,
) -> Result<()> {
    request.region.require_non_empty()?;
    if !description.coordinates().supports(request.region.space())
        || !description.coordinates().supports(request.output_space)
    {
        return Err(GeometryFault::ConversionUnsupported.into());
    }
    // Capture origin is invariant; open-time upper extents are not.
    let region = request.region;
    if region.space() == CoordinateSpace::CapturePixels
        && (region.right() <= 0.0
            || region.bottom() <= 0.0
            || (request.clip_policy == ClipPolicy::Reject
                && (region.left() < 0.0 || region.top() < 0.0)))
    {
        return Err(GeometryFault::EmptyRegion.into());
    }
    if request
        .operation
        .now()
        .checked_add(request.rate.0)
        .is_none()
    {
        return Err(Error::new(
            Status::InvalidArgument,
            "OCR interval is not representable",
        ));
    }
    Ok(())
}

// BGRA and RGBA equality are equivalent under the fixed channel permutation.
// Reusing the prepared BGRA mapping avoids a second conversion and pixel copy.
fn unchanged_pixels(previous: &CpuMapping, current: &CpuMapping) -> bool {
    let before = previous.stamp();
    let after = current.stamp();
    let descriptor = current.descriptor();
    if before.stream() != after.stream()
        || before.epoch() != after.epoch()
        || before.sequence() >= after.sequence()
        || before.geometry() != after.geometry()
        || previous.transform() != current.transform()
        || previous.region() != current.region()
        || previous.descriptor() != descriptor
        || descriptor.format() != PixelFormat::Bgra8
        || previous.bytes().len() != descriptor.byte_len()
        || current.bytes().len() != descriptor.byte_len()
    {
        return false;
    }
    (0..descriptor.extent().height() as usize).all(|row| {
        let start = row * descriptor.stride();
        let end = start + descriptor.row_bytes();
        previous.bytes()[start..end] == current.bytes()[start..end]
    })
}

impl OcrQueryShared {
    pub(super) fn ready_for_dispatch(&self) -> bool {
        let state = lock(&self.state);
        state.terminal.is_none() && state.pending.is_some()
    }
}

impl WatchRuntime {
    pub(crate) fn ocr_observation(&self) -> OcrTextSchedulerObservation {
        self.scheduler.ocr_observation()
    }
}

impl WatchScheduler {
    fn ocr_observation(&self) -> OcrTextSchedulerObservation {
        let (physical, logical, high_water, mappings, barrier) = {
            let dispatch = lock(&self.admission);
            let logical = dispatch
                .ocr_query
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|query| lock(&query.state).execution.is_some());
            (
                u32::from(dispatch.ocr_worker.is_some()),
                u32::from(logical),
                dispatch.ocr_high_water,
                u32::try_from(dispatch.template_mappings).unwrap_or(u32::MAX),
                dispatch.ocr_mapping || dispatch.ocr_turn,
            )
        };
        let (bytes, cache_high_water) = {
            let cache = lock(&self.cache);
            (cache.bytes as u64, cache.high_water as u64)
        };
        let (retained, extent, retained_high_water) = if self.ocr_used.load(Ordering::Acquire) {
            let counts = lock(&self.retention);
            (counts.live, counts.bytes, counts.high_water)
        } else {
            (0, 0, 0)
        };
        OcrTextSchedulerObservation {
            query_count: u32::try_from(self.query_count.load(Ordering::Acquire))
                .unwrap_or(u32::MAX),
            active_sessions: u32::try_from(self.active_sessions.load(Ordering::Acquire))
                .unwrap_or(u32::MAX),
            logical_ocr_in_flight: logical,
            physical_ocr_in_flight: physical,
            physical_ocr_high_water: high_water,
            template_mapping_reservations: mappings,
            mapping_barrier: barrier,
            mapped_cache_bytes: bytes,
            mapped_cache_high_water_bytes: cache_high_water,
            retained_results: retained,
            retained_result_extent_bytes: extent,
            retained_result_high_water_bytes: retained_high_water,
        }
    }

    pub(super) fn next_dispatch(self: &Arc<Self>, worker: usize) -> Option<WatchWork> {
        if !self.ocr_used.load(Ordering::Acquire) {
            return self.next_work().map(WatchWork::Template);
        }
        if self.closed.load(Ordering::Acquire) || self.publishing.load(Ordering::Acquire) != 0 {
            return None;
        }
        let physical = {
            let admission = lock(&self.admission);
            admission.ocr_worker.map(|_| admission.ocr_mapping)
        };
        if let Some(mapping) = physical {
            // The occupied OCR lease makes every other OCR candidate
            // undispatchable. Authority sweeps and acquisition remain independent.
            return if mapping {
                None
            } else {
                self.next_work().map(WatchWork::Template)
            };
        }
        let publication = self.publication_generation.load(Ordering::Acquire);
        let session_cursor = lock(&self.admission).ocr_session_cursor;
        let mut sessions = self.session_snapshot();
        sessions.sort_by_key(|session| (session.id <= session_cursor, session.id));
        let mut ocr = Vec::new();
        let mut templates = Vec::new();
        let mut template_ready = false;
        for session in sessions {
            for query in session.query_snapshot() {
                if query.operation.interruption().is_none()
                    && let Some(candidate) = query.candidate()
                {
                    let now = query.operation.now();
                    template_ready |= candidate
                        .rate_eligible_at
                        .is_none_or(|eligible| now >= eligible);
                }
                templates.push(query);
            }
            let cursor = lock(&session.state).ocr_query_cursor;
            let mut queries = session.ocr_snapshot();
            queries.sort_by_key(|query| (query.id.get() <= cursor, query.id.get()));
            for query in queries {
                if let Some((stamp, now)) = query.ready() {
                    ocr.push((Arc::clone(&session), query, stamp, now, cursor));
                }
            }
        }
        let mut deferred = Vec::with_capacity(templates.len());
        let mut admission = lock(&self.admission);
        if self.closed.load(Ordering::Acquire)
            || self.publishing.load(Ordering::Acquire) != 0
            || self.publication_generation.load(Ordering::Acquire) != publication
            || admission.ocr_session_cursor != session_cursor
            || ocr
                .iter()
                .any(|(session, _, _, _, cursor)| lock(&session.state).ocr_query_cursor != *cursor)
        {
            return None;
        }
        admission.ocr_ready = !ocr.is_empty();
        if admission.ocr_worker.is_none() {
            if ocr.is_empty() {
                admission.ocr_turn = false;
                self.mapping_barrier.store(false, Ordering::Release);
            } else if admission.ocr_turn || admission.next_ocr || !template_ready {
                // No worker waits on the barrier. Dispatched template mappings
                // retain their reservations until their actual return.
                admission.ocr_turn = true;
                for query in &templates {
                    if query.defer_mapping_barrier() {
                        deferred.push(query);
                    }
                }
                self.mapping_barrier.store(true, Ordering::Release);
                if admission.template_mappings != 0 {
                    drop(admission);
                    for query in deferred {
                        query.emit(Some(TemplateWorkDisposition::DeferredRate), false);
                    }
                    return None;
                }
                for (session, query, stamp, now, _) in ocr {
                    let activation = lock(&session.activation);
                    if session.closed.load(Ordering::Acquire)
                        || lock(&session.terminal_authority).is_some()
                    {
                        continue;
                    }
                    if let Some(work) = query.claim(&session, stamp, now) {
                        admission.ocr_worker = Some(worker);
                        admission.ocr_query = Some(Arc::downgrade(&query));
                        admission.ocr_mapping = true;
                        admission.ocr_turn = false;
                        admission.ocr_high_water = 1;
                        admission.next_ocr = false;
                        admission.ocr_session_cursor = session.id;
                        lock(&session.state).ocr_query_cursor = query.id.get();
                        drop(activation);
                        drop(admission);
                        for query in deferred {
                            query.emit(Some(TemplateWorkDisposition::DeferredRate), false);
                        }
                        session.notify_progress();
                        return Some(WatchWork::Ocr(work));
                    }
                }
                admission.ocr_ready = false;
                admission.ocr_turn = false;
                self.mapping_barrier.store(false, Ordering::Release);
            }
        }
        let blocked = admission.ocr_mapping || admission.ocr_turn;
        drop(admission);
        for query in deferred {
            query.emit(Some(TemplateWorkDisposition::DeferredRate), false);
        }
        if blocked {
            None
        } else {
            self.next_work().map(WatchWork::Template)
        }
    }

    pub(super) fn process_ocr(&self, work: OcrWork) {
        let lease = OcrPhysicalLease {
            scheduler: self,
            work: Some(work),
        };
        let work = lease.work.as_ref().expect("physical work owner");
        let query = &work.query;
        query.sweep();
        if !OcrQueryShared::execution_current(&lock(&query.state), work) {
            return;
        }
        let mapping_result = (|| {
            query.resolve_frame(&work.frame)?;
            if query.recognizer.provider_descriptor().as_ref() != Some(&query.provider) {
                return Err(unsupported_selection());
            }
            prepare(
                &query.recognizer,
                OcrRequest::new(
                    &work.frame,
                    query.descriptor.backend_identity(),
                    query.descriptor.model_identity(),
                    OcrRegion::Region {
                        rect: query.request.region,
                        policy: query.request.clip_policy,
                    },
                    query.request.output_space,
                    &work.operation,
                ),
            )
        })();
        // No following template mapper enters native storage before preparation
        // actually returns, including its error paths.
        {
            let mut admission = lock(&self.admission);
            admission.ocr_mapping = false;
            self.mapping_barrier.store(false, Ordering::Release);
        }
        self.wake();
        let prepared = match mapping_result {
            Ok(prepared) => prepared,
            Err(error) => {
                query.complete(work, None, Err(error));
                return;
            }
        };
        if prepared.pixels().bytes().len() > MAX_SOURCE_BYTES {
            query.complete(work, None, Err(OcrFault::MappingAboveCeiling.into()));
            return;
        }
        let previous = {
            let state = lock(&query.state);
            if !OcrQueryShared::execution_current(&state, work) {
                return;
            }
            if state.confirmed == 0 {
                state.previous_mapping.as_ref().and_then(Weak::upgrade)
            } else {
                None
            }
        };
        let unchanged = query.request.change_policy != ChangeDetectionPolicy::AnalysisAlways
            && previous
                .as_ref()
                .is_some_and(|previous| unchanged_pixels(previous, prepared.pixels()));
        let mapping = if query.request.change_policy != ChangeDetectionPolicy::AnalysisAlways {
            let mut cache = lock(&self.cache);
            if !self.closed.load(Ordering::Acquire) && self.query_count.load(Ordering::Acquire) != 0
            {
                cache.remember_pixels(prepared.pixels())
            } else {
                None
            }
        } else {
            None
        };
        if query.admit_prepared(work, &prepared, mapping.clone(), unchanged) {
            let output = prepared.execute();
            query.complete(work, mapping, output);
        }
    }
}

struct OcrPhysicalLease<'a> {
    scheduler: &'a WatchScheduler,
    work: Option<OcrWork>,
}

impl Drop for OcrPhysicalLease<'_> {
    fn drop(&mut self) {
        let work = self.work.take().expect("one physical OCR owner");
        let query = Arc::clone(&work.query);
        let session = Arc::clone(&work.session);
        drop(work);
        query.physical.store(false, Ordering::Release);
        session.physical_ocr.fetch_sub(1, Ordering::AcqRel);
        {
            let mut admission = lock(&self.scheduler.admission);
            admission.ocr_mapping = false;
            admission.ocr_query = None;
            admission.ocr_worker = None;
            self.scheduler
                .mapping_barrier
                .store(admission.ocr_turn, Ordering::Release);
        }
        session.deactivate_if_finished();
        session.notify_progress();
        query.changed.notify_all();
        self.scheduler.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mado_pilot_capture::{CaptureProvider, OpenRequest};
    use mado_pilot_core::{IdentityIssuer, PixelExtent};
    use mado_pilot_ocr::{
        BackendId, BackendVersion, OcrBackendIdentity, OcrExecutionProviderPolicy,
        ProviderProfileId,
    };
    use mado_pilot_testkit::{
        ControlledCapture, ControlledMatcher, ControlledOcr, ControlledProducer,
        ScriptedOcrCandidate, match_fixtures,
    };

    fn until(mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "controlled dispatch did not become ready"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn cancelled_dispatched_template_releases_mapping_reservation_before_ocr_turn() {
        let issuer = Arc::new(IdentityIssuer::new());
        let extent = PixelExtent::new(32, 24);
        let capture = ControlledCapture::new(issuer, extent, PixelFormat::Bgra8).unwrap();
        let matcher = Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8)));
        let template = matcher
            .prepare(
                &match_fixtures::planted_template("cancel-before-map"),
                &OperationContext::new(),
            )
            .unwrap();
        let options = MatchOptions::from_defaults(template.defaults());
        let runtime = WatchRuntime::new(matcher, None);
        // The same finite dispatcher runs synchronously so cancellation is
        // placed precisely after claim and before the mapper starts.
        {
            let mut threads = lock(&runtime._threads.state);
            threads.workers = vec![thread::spawn(|| {}), thread::spawn(|| {})];
            threads.supervisor = Some(thread::spawn(|| {}));
        }
        let capture_session = capture
            .open(
                capture.target(),
                &OpenRequest::new(),
                &OperationContext::new(),
            )
            .unwrap();
        let session = runtime.register_session(capture_session).unwrap();
        let template_query = session
            .start_query(TemplateWatchRequest::new(
                template,
                options,
                OperationContext::new(),
            ))
            .unwrap();
        let producer = ControlledProducer::new(extent, PixelFormat::Bgra8, 1, 4).unwrap();
        capture.publish_from(&producer, 1).unwrap();
        let mut claimed = None;
        until(|| {
            claimed = runtime.scheduler.next_work();
            claimed.is_some()
        });
        let claimed = claimed.unwrap();
        let recognizer = OcrRecognizer::new(Arc::new(
            ControlledOcr::new(PixelFormat::Bgra8)
                .with_descriptor(OcrBackendDescriptor::new(
                    OcrBackendIdentity::new(
                        BackendId::new(CPU_BACKEND).unwrap(),
                        BackendVersion::new(CPU_BACKEND_VERSION).unwrap(),
                    ),
                    OcrModelIdentity::accepted_bounded_detector(),
                    PixelFormat::Bgra8,
                ))
                .with_provider_descriptor(OcrProviderDescriptor::new(
                    OcrExecutionProviderPolicy::Cpu,
                    OcrExecutionProvider::Cpu,
                    None,
                    ProviderProfileId::new(CPU_RUNTIME_PROFILE).unwrap(),
                ))
                .with_candidates(vec![ScriptedOcrCandidate::new(
                    b"ready".as_slice(),
                    [(1.0, 1.0), (8.0, 1.0), (8.0, 5.0), (1.0, 5.0)],
                    0.8,
                    0,
                )]),
        ));
        let request = OcrTextWatchRequest::new(
            Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 32.0, 24.0).unwrap(),
            ClipPolicy::Reject,
            "ready",
            0.8,
            CoordinateSpace::CapturePixels,
            OcrTextAnalysisRate::from_minimum_interval(Duration::from_nanos(1)).unwrap(),
            OcrTextStability::immediate(),
            ChangeDetectionPolicy::AnalysisAlways,
            OperationContext::new(),
        )
        .unwrap();
        let query = session.start_ocr_query(request, Some(&recognizer)).unwrap();
        until(|| query.progress().last_frame().is_some());
        assert!(runtime.scheduler.next_dispatch(0).is_none());
        assert_eq!(producer.conversion_attempts(), 0);
        assert!(matches!(
            &*template_query.cancel(),
            TemplateTerminalOutcome::Cancelled
        ));
        runtime.scheduler.process(claimed);
        let mut next = None;
        until(|| {
            next = runtime.scheduler.next_dispatch(0);
            next.is_some()
        });
        let WatchWork::Ocr(work) = next.unwrap() else {
            panic!("OCR mapping turn lost");
        };
        runtime.scheduler.process_ocr(work);
        assert!(
            query
                .wait(
                    &OperationContext::new()
                        .with_timeout(Duration::from_secs(3))
                        .unwrap()
                )
                .unwrap()
                .is_match()
        );
        assert_eq!(producer.conversion_attempts(), 1);
        session.close(TemplateTerminalOutcome::SessionClosed);
        runtime.close();
    }

    fn selected_recognizer(backend: ControlledOcr) -> OcrRecognizer {
        OcrRecognizer::new(Arc::new(
            backend
                .with_descriptor(OcrBackendDescriptor::new(
                    OcrBackendIdentity::new(
                        BackendId::new(CPU_BACKEND).unwrap(),
                        BackendVersion::new(CPU_BACKEND_VERSION).unwrap(),
                    ),
                    OcrModelIdentity::accepted_bounded_detector(),
                    PixelFormat::Bgra8,
                ))
                .with_provider_descriptor(OcrProviderDescriptor::new(
                    OcrExecutionProviderPolicy::Cpu,
                    OcrExecutionProvider::Cpu,
                    None,
                    ProviderProfileId::new(CPU_RUNTIME_PROFILE).unwrap(),
                )),
        ))
    }

    fn text_request(operation: OperationContext) -> OcrTextWatchRequest {
        OcrTextWatchRequest::new(
            Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 32.0, 24.0).unwrap(),
            ClipPolicy::Reject,
            "ready",
            0.8,
            CoordinateSpace::CapturePixels,
            OcrTextAnalysisRate::from_minimum_interval(Duration::from_nanos(1)).unwrap(),
            OcrTextStability::immediate(),
            ChangeDetectionPolicy::AnalysisAlways,
            operation,
        )
        .unwrap()
    }

    fn quiet_runtime(matcher: Matcher) -> WatchRuntime {
        let runtime = WatchRuntime::new(matcher, None);
        {
            let mut threads = lock(&runtime._threads.state);
            threads.workers = vec![thread::spawn(|| {}), thread::spawn(|| {})];
            threads.supervisor = Some(thread::spawn(|| {}));
        }
        runtime
    }

    fn next_ocr(runtime: &WatchRuntime, worker: usize) -> OcrWork {
        let mut selected = None;
        until(|| {
            selected = runtime.scheduler.next_dispatch(worker);
            selected.is_some()
        });
        let Some(WatchWork::Ocr(work)) = selected else {
            panic!("expected a ready OCR turn");
        };
        work
    }

    #[test]
    fn completion_authority_discard_records_supersession_without_a_supervisor_race() {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture =
            ControlledCapture::new(issuer, PixelExtent::new(32, 24), PixelFormat::Bgra8).unwrap();
        let runtime = quiet_runtime(Matcher::new(Arc::new(ControlledMatcher::new(
            PixelFormat::Rgba8,
        ))));
        let session = runtime
            .register_session(
                capture
                    .open(
                        capture.target(),
                        &OpenRequest::new(),
                        &OperationContext::new(),
                    )
                    .unwrap(),
            )
            .unwrap();
        let cancellation = CancellationToken::new();
        let recognizer = selected_recognizer(
            ControlledOcr::new(PixelFormat::Bgra8)
                .with_candidates(vec![ScriptedOcrCandidate::new(
                    b"ready".as_slice(),
                    [(1.0, 1.0), (8.0, 1.0), (8.0, 5.0), (1.0, 5.0)],
                    0.8,
                    0,
                )])
                .cancelling_after_output(cancellation.clone()),
        );
        let query = session
            .start_ocr_query(
                text_request(OperationContext::new().with_cancellation(cancellation)),
                Some(&recognizer),
            )
            .unwrap();
        capture
            .publish(1, mado_pilot_capture::Continuity::Continuous)
            .unwrap();
        runtime.scheduler.process_ocr(next_ocr(&runtime, 0));
        assert!(matches!(
            &*query.wait(&OperationContext::new()).unwrap(),
            OcrTextTerminalOutcome::Cancelled
        ));
        let progress = query.progress();
        assert_eq!(progress.work().get(OcrTextWorkDisposition::Admitted), 1);
        assert_eq!(progress.work().get(OcrTextWorkDisposition::Superseded), 1);
        assert_eq!(progress.work().get(OcrTextWorkDisposition::Completed), 0);
        assert_eq!(progress.physical_in_flight_count(), 0);
        runtime.close();
    }

    #[test]
    fn template_barrier_residence_uses_the_pending_obligation_and_its_future_rate_floor() {
        use mado_pilot_testkit::ManualClock;
        for (interval, pending_at, eligibility) in [(60, 1, 60), (1, 100, 100)] {
            let issuer = Arc::new(IdentityIssuer::new());
            let capture =
                ControlledCapture::new(issuer, PixelExtent::new(32, 24), PixelFormat::Bgra8)
                    .unwrap();
            let matcher = Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8)));
            let template = matcher
                .prepare(
                    &match_fixtures::planted_template("barrier-rate"),
                    &OperationContext::new(),
                )
                .unwrap();
            let options = MatchOptions::from_defaults(template.defaults());
            let runtime = quiet_runtime(matcher);
            let session = runtime
                .register_session(
                    capture
                        .open(
                            capture.target(),
                            &OpenRequest::new(),
                            &OperationContext::new(),
                        )
                        .unwrap(),
                )
                .unwrap();
            let clock = Arc::new(ManualClock::new());
            let template_query = session
                .start_query(
                    TemplateWatchRequest::new(
                        template,
                        options,
                        OperationContext::new().with_clock(clock.clone()),
                    )
                    .with_rate(
                        TemplateAnalysisRate::at_most_every(Duration::from_secs(interval)).unwrap(),
                    ),
                )
                .unwrap();
            capture
                .publish(1, mado_pilot_capture::Continuity::Continuous)
                .unwrap();
            let mut first = None;
            until(|| {
                first = runtime.scheduler.next_work();
                first.is_some()
            });
            runtime.scheduler.process(first.unwrap());
            assert_eq!(
                template_query.shared.snapshot().state(),
                TemplateQueryState::Pending
            );
            let recognizer = selected_recognizer(ControlledOcr::new(PixelFormat::Bgra8));
            let query = session
                .start_ocr_query(text_request(OperationContext::new()), Some(&recognizer))
                .unwrap();
            clock.advance(Duration::from_secs(pending_at));
            capture
                .publish(2, mado_pilot_capture::Continuity::Continuous)
                .unwrap();
            until(|| {
                query
                    .progress()
                    .last_frame()
                    .is_some_and(|stamp| stamp.sequence().value() >= 1)
            });
            let held_mapping = next_ocr(&runtime, 0);
            clock.advance(Duration::from_secs(eligibility - pending_at + 30));
            capture
                .publish(3, mado_pilot_capture::Continuity::Continuous)
                .unwrap();
            until(|| {
                query
                    .progress()
                    .last_frame()
                    .is_some_and(|stamp| stamp.sequence().value() >= 2)
            });
            runtime.scheduler.sweep_authority();
            assert!(matches!(
                template_query.poll(),
                TemplateQueryOutcome::Pending(_)
            ));
            clock.advance(Duration::from_secs(1));
            runtime.scheduler.sweep_authority();
            assert!(matches!(
                &*template_query.wait(&OperationContext::new()).unwrap(),
                TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired)
            ));
            runtime.scheduler.process_ocr(held_mapping);
            let _ = query.cancel();
            runtime.close();
        }
    }

    #[derive(Debug, Default)]
    struct PausedClock {
        state: Mutex<(bool, bool, bool)>,
        changed: Condvar,
    }

    impl PausedClock {
        fn arm(&self) {
            *lock(&self.state) = (true, false, false);
        }
        fn wait_entered(&self) {
            let (state, _) = self
                .changed
                .wait_timeout_while(lock(&self.state), Duration::from_secs(3), |state| !state.1)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(
                state.1,
                "dispatcher did not reach the controlled clock boundary"
            );
        }
        fn release(&self) {
            lock(&self.state).2 = true;
            self.changed.notify_all();
        }
    }

    impl mado_pilot_core::Clock for PausedClock {
        fn now(&self) -> MonotonicInstant {
            let mut state = lock(&self.state);
            if state.0 {
                state.0 = false;
                state.1 = true;
                self.changed.notify_all();
                while !state.2 {
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
            MonotonicInstant::ORIGIN
        }
    }

    struct ResumeClock(Arc<PausedClock>);
    impl Drop for ResumeClock {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    #[test]
    fn a_cursor_snapshot_cannot_spend_the_next_sessions_ocr_turn() {
        let issuer = Arc::new(IdentityIssuer::new());
        let capture =
            ControlledCapture::new(issuer, PixelExtent::new(32, 24), PixelFormat::Bgra8).unwrap();
        let runtime = quiet_runtime(Matcher::new(Arc::new(ControlledMatcher::new(
            PixelFormat::Rgba8,
        ))));
        let sessions: Vec<_> = (0..2)
            .map(|_| {
                runtime
                    .register_session(
                        capture
                            .open(
                                capture.target(),
                                &OpenRequest::new(),
                                &OperationContext::new(),
                            )
                            .unwrap(),
                    )
                    .unwrap()
            })
            .collect();
        let recognizer = selected_recognizer(ControlledOcr::new(PixelFormat::Bgra8));
        let clock = Arc::new(PausedClock::default());
        let queries: Vec<_> = [&sessions[0], &sessions[0], &sessions[1], &sessions[1]]
            .into_iter()
            .map(|session| {
                session
                    .start_ocr_query(
                        text_request(OperationContext::new().with_clock(clock.clone())),
                        Some(&recognizer),
                    )
                    .unwrap()
            })
            .collect();
        capture
            .publish(1, mado_pilot_capture::Continuity::Continuous)
            .unwrap();
        until(|| {
            queries
                .iter()
                .all(|query| query.progress().last_frame().is_some())
                && runtime.scheduler.publishing.load(Ordering::Acquire) == 0
        });
        let _resume = ResumeClock(clock.clone());
        clock.arm();
        let scheduler = runtime.scheduler.clone();
        let stale_worker = thread::spawn(move || scheduler.next_dispatch(0));
        clock.wait_entered();
        runtime.scheduler.process_ocr(next_ocr(&runtime, 1));
        assert_eq!(
            queries[0]
                .progress()
                .work()
                .get(OcrTextWorkDisposition::Completed),
            1
        );
        clock.release();
        let second = match stale_worker.join().unwrap() {
            Some(WatchWork::Ocr(work)) => work,
            Some(WatchWork::Template(_)) => panic!("no template query exists"),
            None => next_ocr(&runtime, 0),
        };
        runtime.scheduler.process_ocr(second);
        assert_eq!(
            queries[2]
                .progress()
                .work()
                .get(OcrTextWorkDisposition::Completed),
            1,
            "another worker's successful turn invalidates the older session ordering"
        );
        for query in &queries {
            let _ = query.cancel();
        }
        runtime.close();
    }
}
