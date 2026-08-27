//! Deterministic offline evaluation for the frozen G-005 recorded sequences.
//!
//! This module is test and evidence support. It does not participate in capture
//! publication, runtime scheduling, or product policy selection.

use std::fmt;
use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MANIFEST_PATH: &str = "fixtures/change-detection/g-005/fixture-manifest.json";
const EXPECTED_ROWS_PATH: &str = "fixtures/change-detection/g-005/expected-rows.json";
const FRAME_PREFIX: &str = "fixtures/change-detection/g-005/frames/";
const MANIFEST_SCHEMA: &str = "mado-pilot-change-sequence-manifest-v1";
const EXPECTED_SCHEMA: &str = "mado-pilot-change-expected-v1";
const REPORT_SCHEMA: &str = "mado-pilot-change-evaluation-report-v1";
const FIXTURE_SET: &str = "g-005-v1";
const REQUIRED_LICENSE: &str = "Apache-2.0";
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;
const MAX_FRAME_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_FRAME_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FRAMES: usize = 64;
const MAX_SEQUENCES: usize = 32;
const MAX_TRANSITIONS: usize = 128;
const MAX_DIMENSION: u64 = 4096;
const BYTES_PER_PIXEL: u64 = 4;

const CANDIDATE_PLAN: &str = concat!(
    "g-005-candidate-plan-v1\n",
    "exact-rgba-v1\n",
    "changed-pixel-count-v1/min-2\n",
    "changed-pixel-count-v1/min-4\n",
    "changed-pixel-count-v1/min-8\n",
    "sampled-exact-v1/stride-2\n",
    "sampled-exact-v1/stride-4\n",
    "sampled-exact-v1/stride-8\n",
    "fallback=analysis-always-v1\n",
    "select=min-admitted-analysis-then-declaration-order\n",
);

const CANDIDATES: [Candidate; 7] = [
    Candidate::new("exact-rgba-v1", CandidateKind::Exact),
    Candidate::new(
        "changed-pixel-count-v1/min-2",
        CandidateKind::ChangedPixelCount(2),
    ),
    Candidate::new(
        "changed-pixel-count-v1/min-4",
        CandidateKind::ChangedPixelCount(4),
    ),
    Candidate::new(
        "changed-pixel-count-v1/min-8",
        CandidateKind::ChangedPixelCount(8),
    ),
    Candidate::new("sampled-exact-v1/stride-2", CandidateKind::SampledExact(2)),
    Candidate::new("sampled-exact-v1/stride-4", CandidateKind::SampledExact(4)),
    Candidate::new("sampled-exact-v1/stride-8", CandidateKind::SampledExact(8)),
];

/// Closed reason an input or evaluation was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EvaluationErrorKind {
    /// A required repository component could not be read.
    ComponentUnavailable,
    /// A path component was a link, special file, or unexpected directory.
    ComponentNotRegular,
    /// A document or frame exceeded its declared bound.
    ComponentTooLarge,
    /// A JSON document violated its strict typed shape.
    InvalidJson,
    /// A document named an unsupported schema or fixture set.
    UnsupportedSchema,
    /// The fixture license was not the predeclared repository license.
    UnsupportedLicense,
    /// A declared component count did not match the ordered data.
    InvalidComponentLength,
    /// An externally supplied id was not bounded safe ASCII.
    InvalidIdentifier,
    /// A component id, path, frame use, or transition was duplicated.
    DuplicateComponent,
    /// A frame reference was missing or non-canonical.
    InvalidFrameReference,
    /// Frame or transition order differed from the manifest.
    InvalidFrameOrder,
    /// Dimensions, stride, or byte length were inconsistent.
    InvalidFrameShape,
    /// A declared digest was not canonical lowercase SHA-256.
    InvalidDigest,
    /// Loaded frame bytes did not match the declared digest.
    DigestMismatch,
    /// A sequence was empty, unbounded, or had an invalid ROI.
    InvalidSequence,
    /// An oracle row was missing, reordered, or semantically inconsistent.
    InvalidExpectedRow,
    /// Frame identity or geometry contradicted the compatibility label.
    InvalidCompatibility,
    /// Checked size, coordinate, or counter arithmetic overflowed.
    ArithmeticOverflow,
    /// Canonical aggregate JSON could not be serialized.
    ReportSerialization,
}

impl EvaluationErrorKind {
    const fn message(self) -> &'static str {
        match self {
            Self::ComponentUnavailable => "component unavailable",
            Self::ComponentNotRegular => "component is not a regular repository file",
            Self::ComponentTooLarge => "component exceeds its declared bound",
            Self::InvalidJson => "component does not match its strict JSON schema",
            Self::UnsupportedSchema => "unsupported component schema",
            Self::UnsupportedLicense => "unsupported fixture license",
            Self::InvalidComponentLength => "component count does not match its declaration",
            Self::InvalidIdentifier => "component identifier is invalid",
            Self::DuplicateComponent => "component is duplicated",
            Self::InvalidFrameReference => "frame reference is invalid",
            Self::InvalidFrameOrder => "frame or transition order is invalid",
            Self::InvalidFrameShape => "frame shape or byte length is invalid",
            Self::InvalidDigest => "fixture digest declaration is invalid",
            Self::DigestMismatch => "fixture bytes do not match their declaration",
            Self::InvalidSequence => "recorded sequence is invalid",
            Self::InvalidExpectedRow => "expected transition row is invalid",
            Self::InvalidCompatibility => "transition compatibility is invalid",
            Self::ArithmeticOverflow => "checked evaluator arithmetic overflowed",
            Self::ReportSerialization => "aggregate report serialization failed",
        }
    }
}

/// Content-redacted evaluator failure.
#[derive(Clone, PartialEq, Eq)]
pub struct EvaluationError {
    kind: EvaluationErrorKind,
    component_id: Option<Box<str>>,
}

impl EvaluationError {
    const fn new(kind: EvaluationErrorKind) -> Self {
        Self {
            kind,
            component_id: None,
        }
    }

    fn for_component(kind: EvaluationErrorKind, component_id: &str) -> Self {
        Self {
            kind,
            component_id: Some(component_id.into()),
        }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(&self) -> EvaluationErrorKind {
        self.kind
    }

    /// Returns a validated, non-sensitive component id when one is useful.
    #[must_use]
    pub fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }
}

impl fmt::Debug for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvaluationError")
            .field("kind", &self.kind)
            .field("component_id", &self.component_id)
            .finish()
    }
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.message())?;
        if let Some(component_id) = &self.component_id {
            write!(formatter, " ({component_id})")?;
        }
        Ok(())
    }
}

impl std::error::Error for EvaluationError {}

/// One fully validated, immutable recorded-sequence set.
pub struct RecordedSequenceSet {
    fixture_set: String,
    manifest_sha256: String,
    expected_rows_sha256: String,
    frames: Vec<LoadedFrame>,
    transitions: Vec<LoadedTransition>,
}

impl fmt::Debug for RecordedSequenceSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordedSequenceSet")
            .field("fixture_set", &self.fixture_set)
            .field("frame_count", &self.frames.len())
            .field("transition_count", &self.transitions.len())
            .finish()
    }
}

impl RecordedSequenceSet {
    /// Loads and validates the frozen G-005 documents and frame bytes.
    ///
    /// Errors contain only a closed kind and an already validated component id;
    /// filesystem paths, decoder strings, and digest values are never surfaced.
    pub fn load(repository_root: &Path) -> Result<Self, EvaluationError> {
        let manifest_bytes =
            read_repository_file(repository_root, MANIFEST_PATH, MAX_DOCUMENT_BYTES, None)?;
        let expected_bytes = read_repository_file(
            repository_root,
            EXPECTED_ROWS_PATH,
            MAX_DOCUMENT_BYTES,
            None,
        )?;

        let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| EvaluationError::new(EvaluationErrorKind::InvalidJson))?;
        let expected: ExpectedRows = serde_json::from_slice(&expected_bytes)
            .map_err(|_| EvaluationError::new(EvaluationErrorKind::InvalidJson))?;

        if manifest.schema != MANIFEST_SCHEMA || expected.schema != EXPECTED_SCHEMA {
            return Err(EvaluationError::new(EvaluationErrorKind::UnsupportedSchema));
        }
        if manifest.fixture_set != FIXTURE_SET || expected.fixture_set != FIXTURE_SET {
            return Err(EvaluationError::new(EvaluationErrorKind::UnsupportedSchema));
        }
        if manifest.license != REQUIRED_LICENSE {
            return Err(EvaluationError::new(
                EvaluationErrorKind::UnsupportedLicense,
            ));
        }

        validate_component_lengths(&manifest)?;
        let frames = load_frames(repository_root, &manifest.frames)?;
        let skeletons = validate_sequences(&manifest, &frames)?;
        let transitions = validate_expected_rows(&expected.rows, skeletons)?;

        Ok(Self {
            fixture_set: manifest.fixture_set,
            manifest_sha256: sha256_hex(&manifest_bytes),
            expected_rows_sha256: sha256_hex(&expected_bytes),
            frames,
            transitions,
        })
    }

    /// Returns the frozen fixture-set id.
    #[must_use]
    pub fn fixture_set(&self) -> &str {
        &self.fixture_set
    }

    /// Returns the number of adjacent transitions in the oracle.
    #[must_use]
    pub fn transition_count(&self) -> usize {
        self.transitions.len()
    }
}

/// A candidate's bounded decision for one transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationDecision {
    /// The transition must proceed to routine visual analysis.
    AnalysisRequired,
    /// The compatible ROI bytes authorize skipping routine analysis only.
    Unchanged,
    /// The candidate produced a typed failure and is rejected.
    Failed,
}

/// A closed candidate failure retained in the aggregate report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateFailureCode {
    /// A checked candidate counter overflowed.
    ArithmeticOverflow,
    /// Validated frame metadata did not cover a requested pixel.
    PixelBounds,
    /// A deterministic test-only adapter injected a failure.
    InjectedFailure,
}

/// Whether a candidate satisfied every frozen correctness gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    /// Every must-detect and completeness gate passed.
    Passed,
    /// At least one false skip, failure, or completeness gate failed.
    Rejected,
}

/// One ordered transition decision in an aggregate report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransitionReport {
    ordinal: u64,
    transition_id: String,
    decision: EvaluationDecision,
    failure_code: Option<CandidateFailureCode>,
}

impl TransitionReport {
    /// Returns the oracle ordinal.
    #[must_use]
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Returns the non-sensitive transition id.
    #[must_use]
    pub fn transition_id(&self) -> &str {
        &self.transition_id
    }

    /// Returns the candidate decision.
    #[must_use]
    pub const fn decision(&self) -> EvaluationDecision {
        self.decision
    }
}

/// Bounded aggregate counters for one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EvaluationAggregates {
    transition_count: u64,
    must_detect_count: u64,
    false_skip_count: u64,
    admitted_analysis_count: u64,
    skipped_analysis_count: u64,
    candidate_failure_count: u64,
    inspected_pixel_count: u64,
}

impl EvaluationAggregates {
    /// Returns mandatory changes the candidate incorrectly skipped.
    #[must_use]
    pub const fn false_skip_count(self) -> u64 {
        self.false_skip_count
    }

    /// Returns transitions admitted to routine analysis.
    #[must_use]
    pub const fn admitted_analysis_count(self) -> u64 {
        self.admitted_analysis_count
    }

    /// Returns typed candidate failures.
    #[must_use]
    pub const fn candidate_failure_count(self) -> u64 {
        self.candidate_failure_count
    }
}

/// Complete result for one predeclared candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateReport {
    candidate_id: String,
    transitions: Vec<TransitionReport>,
    aggregates: EvaluationAggregates,
    status: CandidateStatus,
}

impl CandidateReport {
    /// Returns the predeclared candidate id.
    #[must_use]
    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    /// Returns every transition decision in oracle order.
    #[must_use]
    pub fn transitions(&self) -> &[TransitionReport] {
        &self.transitions
    }

    /// Returns bounded aggregate counters.
    #[must_use]
    pub const fn aggregates(&self) -> EvaluationAggregates {
        self.aggregates
    }

    /// Returns whether every correctness gate passed.
    #[must_use]
    pub const fn status(&self) -> CandidateStatus {
        self.status
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct AuthorityFacts {
    unchanged_may_skip_routine_analysis: bool,
    unchanged_confirms_presence: bool,
    unchanged_advances_consecutive_stability: bool,
    unchanged_creates_duration_stability: bool,
    unchanged_crosses_incompatible_identity_or_geometry: bool,
}

/// Target-neutral, deterministic G-005 aggregate output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvaluationReport {
    schema: &'static str,
    fixture_set: String,
    manifest_sha256: String,
    expected_rows_sha256: String,
    evaluator_source_sha256: String,
    candidate_plan_sha256: String,
    candidates: Vec<CandidateReport>,
    selected_policy_id: String,
    authority: AuthorityFacts,
}

impl EvaluationReport {
    /// Returns candidates in their normative comparison order.
    #[must_use]
    pub fn candidates(&self) -> &[CandidateReport] {
        &self.candidates
    }

    /// Returns the selected closed runtime policy id.
    #[must_use]
    pub fn selected_policy_id(&self) -> &str {
        &self.selected_policy_id
    }

    /// Serializes fixed-field compact JSON with one trailing newline.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, EvaluationError> {
        let mut bytes = serde_json::to_vec(self)
            .map_err(|_| EvaluationError::new(EvaluationErrorKind::ReportSerialization))?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

/// Runs every predeclared candidate once and applies the frozen comparison rule.
#[must_use]
pub fn evaluate_g005(sequences: &RecordedSequenceSet) -> EvaluationReport {
    let candidates: Vec<CandidateReport> = CANDIDATES
        .iter()
        .map(|candidate| run_candidate(sequences, *candidate))
        .collect();

    let selected_policy_id = candidates
        .iter()
        .filter(|report| report.status == CandidateStatus::Passed)
        .min_by_key(|report| report.aggregates.admitted_analysis_count)
        .map_or_else(
            || "analysis-always-v1".to_owned(),
            |report| report.candidate_id.clone(),
        );

    EvaluationReport {
        schema: REPORT_SCHEMA,
        fixture_set: sequences.fixture_set.clone(),
        manifest_sha256: sequences.manifest_sha256.clone(),
        expected_rows_sha256: sequences.expected_rows_sha256.clone(),
        evaluator_source_sha256: sha256_hex(include_bytes!("change_detection.rs")),
        candidate_plan_sha256: sha256_hex(CANDIDATE_PLAN.as_bytes()),
        candidates,
        selected_policy_id,
        authority: AuthorityFacts {
            unchanged_may_skip_routine_analysis: true,
            unchanged_confirms_presence: false,
            unchanged_advances_consecutive_stability: false,
            unchanged_creates_duration_stability: false,
            unchanged_crosses_incompatible_identity_or_geometry: false,
        },
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    fixture_set: String,
    license: String,
    component_lengths: ComponentLengths,
    frames: Vec<FrameRecord>,
    sequences: Vec<SequenceRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentLengths {
    frames: u64,
    sequences: u64,
    transitions: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameRecord {
    id: String,
    path: String,
    width: u64,
    height: u64,
    row_stride: u64,
    byte_len: u64,
    sha256: String,
    stream: u64,
    epoch: u64,
    sequence: u64,
    geometry_revision: u64,
    pixel_width: u64,
    pixel_height: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceRecord {
    id: String,
    frame_ids: Vec<String>,
    roi: RectRecord,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RectRecord {
    x: u64,
    y: u64,
    width: u64,
    height: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedRows {
    schema: String,
    fixture_set: String,
    rows: Vec<ExpectedRow>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedRow {
    ordinal: u64,
    transition_id: String,
    sequence_id: String,
    from_frame: String,
    to_frame: String,
    compatibility: Compatibility,
    expected: ExpectedDecision,
    reason: ExpectedReason,
    must_detect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Compatibility {
    Compatible,
    GeometryChanged,
    StreamDiscontinuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedDecision {
    UnchangedAllowed,
    AnalysisRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedReason {
    NoChange,
    OutsideRoiChange,
    LowAreaChange,
    TransientAppearance,
    PersistentAppearance,
    Disappearance,
    RepeatedPixels,
    GeometryChange,
    StreamDiscontinuity,
}

struct LoadedFrame {
    id: String,
    width: usize,
    height: usize,
    row_stride: usize,
    stream: u64,
    epoch: u64,
    sequence: u64,
    geometry_revision: u64,
    pixels: Box<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Roi {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

struct TransitionSkeleton {
    ordinal: u64,
    transition_id: String,
    sequence_id: String,
    from_frame: usize,
    to_frame: usize,
    from_frame_id: String,
    to_frame_id: String,
    roi: Roi,
    compatibility: Compatibility,
}

struct LoadedTransition {
    ordinal: u64,
    transition_id: String,
    from_frame: usize,
    to_frame: usize,
    roi: Roi,
    compatibility: Compatibility,
    must_detect: bool,
}

const fn count_within(value: u64, maximum: usize) -> bool {
    value > 0 && value <= maximum as u64
}

fn validate_component_lengths(manifest: &Manifest) -> Result<(), EvaluationError> {
    let declared = &manifest.component_lengths;
    if !count_within(declared.frames, MAX_FRAMES)
        || !count_within(declared.sequences, MAX_SEQUENCES)
        || !count_within(declared.transitions, MAX_TRANSITIONS)
        || usize::try_from(declared.frames).ok() != Some(manifest.frames.len())
        || usize::try_from(declared.sequences).ok() != Some(manifest.sequences.len())
    {
        return Err(EvaluationError::new(
            EvaluationErrorKind::InvalidComponentLength,
        ));
    }

    let transition_count =
        manifest
            .sequences
            .iter()
            .try_fold(0_u64, |count, sequence| {
                let transitions = sequence.frame_ids.len().checked_sub(1).ok_or_else(|| {
                    EvaluationError::new(EvaluationErrorKind::InvalidComponentLength)
                })?;
                count
                    .checked_add(u64::try_from(transitions).map_err(|_| {
                        EvaluationError::new(EvaluationErrorKind::ArithmeticOverflow)
                    })?)
                    .ok_or_else(|| EvaluationError::new(EvaluationErrorKind::ArithmeticOverflow))
            })?;
    if transition_count != declared.transitions {
        return Err(EvaluationError::new(
            EvaluationErrorKind::InvalidComponentLength,
        ));
    }
    Ok(())
}

fn load_frames(
    repository_root: &Path,
    records: &[FrameRecord],
) -> Result<Vec<LoadedFrame>, EvaluationError> {
    let mut loaded = Vec::with_capacity(records.len());
    let mut total_bytes = 0_u64;

    for record in records {
        validate_identifier(&record.id)?;
        if loaded
            .iter()
            .any(|frame: &LoadedFrame| frame.id == record.id)
        {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::DuplicateComponent,
                &record.id,
            ));
        }
        let expected_path = format!("{FRAME_PREFIX}{}.rgba", record.id);
        if record.path != expected_path || record.path.contains('\\') {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::InvalidFrameReference,
                &record.id,
            ));
        }
        if records
            .iter()
            .filter(|other| other.path == record.path)
            .count()
            != 1
        {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::DuplicateComponent,
                &record.id,
            ));
        }
        validate_digest(&record.sha256, &record.id)?;
        validate_frame_shape(record)?;

        total_bytes = total_bytes.checked_add(record.byte_len).ok_or_else(|| {
            EvaluationError::for_component(EvaluationErrorKind::ArithmeticOverflow, &record.id)
        })?;
        if total_bytes > MAX_TOTAL_FRAME_BYTES {
            return Err(EvaluationError::new(EvaluationErrorKind::ComponentTooLarge));
        }

        let bytes = read_repository_file(
            repository_root,
            &record.path,
            MAX_FRAME_BYTES,
            Some(&record.id),
        )?;
        if u64::try_from(bytes.len()).ok() != Some(record.byte_len) {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::InvalidFrameShape,
                &record.id,
            ));
        }
        if sha256_hex(&bytes) != record.sha256 {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::DigestMismatch,
                &record.id,
            ));
        }

        loaded.push(LoadedFrame {
            id: record.id.clone(),
            width: checked_usize(record.width, &record.id)?,
            height: checked_usize(record.height, &record.id)?,
            row_stride: checked_usize(record.row_stride, &record.id)?,
            stream: record.stream,
            epoch: record.epoch,
            sequence: record.sequence,
            geometry_revision: record.geometry_revision,
            pixels: bytes.into_boxed_slice(),
        });
    }

    Ok(loaded)
}

fn validate_frame_shape(record: &FrameRecord) -> Result<(), EvaluationError> {
    if record.width == 0
        || record.height == 0
        || record.width > MAX_DIMENSION
        || record.height > MAX_DIMENSION
        || record.pixel_width != record.width
        || record.pixel_height != record.height
    {
        return Err(EvaluationError::for_component(
            EvaluationErrorKind::InvalidFrameShape,
            &record.id,
        ));
    }
    let row_stride = record.width.checked_mul(BYTES_PER_PIXEL).ok_or_else(|| {
        EvaluationError::for_component(EvaluationErrorKind::ArithmeticOverflow, &record.id)
    })?;
    let byte_len = row_stride.checked_mul(record.height).ok_or_else(|| {
        EvaluationError::for_component(EvaluationErrorKind::ArithmeticOverflow, &record.id)
    })?;
    if record.row_stride != row_stride
        || record.byte_len != byte_len
        || record.byte_len > MAX_FRAME_BYTES
    {
        return Err(EvaluationError::for_component(
            EvaluationErrorKind::InvalidFrameShape,
            &record.id,
        ));
    }
    Ok(())
}

fn validate_sequences(
    manifest: &Manifest,
    frames: &[LoadedFrame],
) -> Result<Vec<TransitionSkeleton>, EvaluationError> {
    let mut sequence_ids: Vec<&str> = Vec::with_capacity(manifest.sequences.len());
    let mut frame_order = Vec::with_capacity(frames.len());
    let mut transitions = Vec::with_capacity(
        usize::try_from(manifest.component_lengths.transitions)
            .map_err(|_| EvaluationError::new(EvaluationErrorKind::ArithmeticOverflow))?,
    );

    for sequence in &manifest.sequences {
        validate_identifier(&sequence.id)?;
        if sequence_ids.contains(&sequence.id.as_str()) {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::DuplicateComponent,
                &sequence.id,
            ));
        }
        sequence_ids.push(&sequence.id);
        if sequence.frame_ids.len() < 2 {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::InvalidSequence,
                &sequence.id,
            ));
        }

        let mut indexes = Vec::with_capacity(sequence.frame_ids.len());
        for frame_id in &sequence.frame_ids {
            validate_identifier(frame_id)?;
            let index = frames
                .iter()
                .position(|frame| frame.id == *frame_id)
                .ok_or_else(|| {
                    EvaluationError::for_component(
                        EvaluationErrorKind::InvalidFrameReference,
                        &sequence.id,
                    )
                })?;
            if frame_order.contains(&index) {
                return Err(EvaluationError::for_component(
                    EvaluationErrorKind::DuplicateComponent,
                    &sequence.id,
                ));
            }
            frame_order.push(index);
            indexes.push(index);
        }

        for (transition_index, pair) in indexes.windows(2).enumerate() {
            let from = &frames[pair[0]];
            let to = &frames[pair[1]];
            let compatibility = derive_compatibility(from, to, &sequence.id)?;
            let from_roi = clip_roi(sequence.roi, from, &sequence.id)?;
            let to_roi = clip_roi(sequence.roi, to, &sequence.id)?;
            if compatibility == Compatibility::Compatible && from_roi != to_roi {
                return Err(EvaluationError::for_component(
                    EvaluationErrorKind::InvalidCompatibility,
                    &sequence.id,
                ));
            }
            let ordinal = u64::try_from(transitions.len())
                .map_err(|_| EvaluationError::new(EvaluationErrorKind::ArithmeticOverflow))?;
            transitions.push(TransitionSkeleton {
                ordinal,
                transition_id: format!("{}/{transition_index}", sequence.id),
                sequence_id: sequence.id.clone(),
                from_frame: pair[0],
                to_frame: pair[1],
                from_frame_id: from.id.clone(),
                to_frame_id: to.id.clone(),
                roi: from_roi,
                compatibility,
            });
        }
    }

    if frame_order.len() != frames.len() || frame_order.iter().copied().ne(0..frames.len()) {
        return Err(EvaluationError::new(EvaluationErrorKind::InvalidFrameOrder));
    }
    Ok(transitions)
}

fn derive_compatibility(
    from: &LoadedFrame,
    to: &LoadedFrame,
    sequence_id: &str,
) -> Result<Compatibility, EvaluationError> {
    if from.stream != to.stream || from.epoch != to.epoch {
        return Ok(Compatibility::StreamDiscontinuous);
    }
    if to.sequence <= from.sequence {
        return Err(EvaluationError::for_component(
            EvaluationErrorKind::InvalidCompatibility,
            sequence_id,
        ));
    }
    if from.geometry_revision != to.geometry_revision
        || from.width != to.width
        || from.height != to.height
        || from.row_stride != to.row_stride
    {
        return Ok(Compatibility::GeometryChanged);
    }
    Ok(Compatibility::Compatible)
}

fn clip_roi(
    raw: RectRecord,
    frame: &LoadedFrame,
    sequence_id: &str,
) -> Result<Roi, EvaluationError> {
    if raw.width == 0 || raw.height == 0 {
        return Err(EvaluationError::for_component(
            EvaluationErrorKind::InvalidSequence,
            sequence_id,
        ));
    }
    let right = raw.width.checked_add(raw.x).ok_or_else(|| {
        EvaluationError::for_component(EvaluationErrorKind::ArithmeticOverflow, sequence_id)
    })?;
    let bottom = raw.height.checked_add(raw.y).ok_or_else(|| {
        EvaluationError::for_component(EvaluationErrorKind::ArithmeticOverflow, sequence_id)
    })?;
    let frame_width = u64::try_from(frame.width)
        .map_err(|_| EvaluationError::new(EvaluationErrorKind::ArithmeticOverflow))?;
    let frame_height = u64::try_from(frame.height)
        .map_err(|_| EvaluationError::new(EvaluationErrorKind::ArithmeticOverflow))?;
    let clipped_right = right.min(frame_width);
    let clipped_bottom = bottom.min(frame_height);
    if raw.x >= clipped_right || raw.y >= clipped_bottom {
        return Err(EvaluationError::for_component(
            EvaluationErrorKind::InvalidSequence,
            sequence_id,
        ));
    }
    Ok(Roi {
        x: checked_usize(raw.x, sequence_id)?,
        y: checked_usize(raw.y, sequence_id)?,
        width: checked_usize(clipped_right - raw.x, sequence_id)?,
        height: checked_usize(clipped_bottom - raw.y, sequence_id)?,
    })
}

fn validate_expected_rows(
    rows: &[ExpectedRow],
    skeletons: Vec<TransitionSkeleton>,
) -> Result<Vec<LoadedTransition>, EvaluationError> {
    if rows.len() != skeletons.len() || rows.len() > MAX_TRANSITIONS {
        return Err(EvaluationError::new(
            EvaluationErrorKind::InvalidComponentLength,
        ));
    }
    let mut transition_ids: Vec<&str> = Vec::with_capacity(rows.len());
    let mut loaded = Vec::with_capacity(rows.len());

    for (row, skeleton) in rows.iter().zip(skeletons) {
        validate_transition_identifier(&row.transition_id)?;
        validate_identifier(&row.sequence_id)?;
        validate_identifier(&row.from_frame)?;
        validate_identifier(&row.to_frame)?;
        if transition_ids.contains(&row.transition_id.as_str()) {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::DuplicateComponent,
                &row.transition_id,
            ));
        }
        transition_ids.push(&row.transition_id);
        if row.ordinal != skeleton.ordinal
            || row.transition_id != skeleton.transition_id
            || row.sequence_id != skeleton.sequence_id
            || row.from_frame != skeleton.from_frame_id
            || row.to_frame != skeleton.to_frame_id
            || row.compatibility != skeleton.compatibility
            || !expected_reason_is_valid(row)
        {
            return Err(EvaluationError::for_component(
                EvaluationErrorKind::InvalidExpectedRow,
                &row.transition_id,
            ));
        }
        loaded.push(LoadedTransition {
            ordinal: row.ordinal,
            transition_id: row.transition_id.clone(),
            from_frame: skeleton.from_frame,
            to_frame: skeleton.to_frame,
            roi: skeleton.roi,
            compatibility: skeleton.compatibility,
            must_detect: row.must_detect,
        });
    }
    Ok(loaded)
}

fn expected_reason_is_valid(row: &ExpectedRow) -> bool {
    match row.reason {
        ExpectedReason::NoChange
        | ExpectedReason::OutsideRoiChange
        | ExpectedReason::RepeatedPixels => {
            row.compatibility == Compatibility::Compatible
                && row.expected == ExpectedDecision::UnchangedAllowed
                && !row.must_detect
        }
        ExpectedReason::LowAreaChange
        | ExpectedReason::TransientAppearance
        | ExpectedReason::PersistentAppearance
        | ExpectedReason::Disappearance => {
            row.compatibility == Compatibility::Compatible
                && row.expected == ExpectedDecision::AnalysisRequired
                && row.must_detect
        }
        ExpectedReason::GeometryChange => {
            row.compatibility == Compatibility::GeometryChanged
                && row.expected == ExpectedDecision::AnalysisRequired
                && row.must_detect
        }
        ExpectedReason::StreamDiscontinuity => {
            row.compatibility == Compatibility::StreamDiscontinuous
                && row.expected == ExpectedDecision::AnalysisRequired
                && row.must_detect
        }
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    id: &'static str,
    kind: CandidateKind,
}

impl Candidate {
    const fn new(id: &'static str, kind: CandidateKind) -> Self {
        Self { id, kind }
    }
}

#[derive(Clone, Copy)]
enum CandidateKind {
    Exact,
    ChangedPixelCount(u64),
    SampledExact(usize),
    #[cfg(test)]
    InjectedFailure,
}

fn run_candidate(sequences: &RecordedSequenceSet, candidate: Candidate) -> CandidateReport {
    let mut reports = Vec::with_capacity(sequences.transitions.len());
    let mut aggregates = EvaluationAggregates {
        transition_count: 0,
        must_detect_count: 0,
        false_skip_count: 0,
        admitted_analysis_count: 0,
        skipped_analysis_count: 0,
        candidate_failure_count: 0,
        inspected_pixel_count: 0,
    };

    for transition in &sequences.transitions {
        aggregates.transition_count += 1;
        if transition.must_detect {
            aggregates.must_detect_count += 1;
        }

        let evaluation = if transition.compatibility == Compatibility::Compatible {
            compare_transition(
                candidate.kind,
                &sequences.frames[transition.from_frame],
                &sequences.frames[transition.to_frame],
                transition.roi,
            )
        } else {
            Ok((EvaluationDecision::AnalysisRequired, 0))
        };

        let (decision, failure_code, inspected) = match evaluation {
            Ok((decision, inspected)) => (decision, None, inspected),
            Err(code) => (EvaluationDecision::Failed, Some(code), 0),
        };
        aggregates.inspected_pixel_count =
            aggregates.inspected_pixel_count.saturating_add(inspected);
        match decision {
            EvaluationDecision::AnalysisRequired => aggregates.admitted_analysis_count += 1,
            EvaluationDecision::Unchanged => {
                aggregates.skipped_analysis_count += 1;
                if transition.must_detect {
                    aggregates.false_skip_count += 1;
                }
            }
            EvaluationDecision::Failed => aggregates.candidate_failure_count += 1,
        }
        reports.push(TransitionReport {
            ordinal: transition.ordinal,
            transition_id: transition.transition_id.clone(),
            decision,
            failure_code,
        });
    }

    let status = if aggregates.false_skip_count == 0
        && aggregates.candidate_failure_count == 0
        && aggregates.inspected_pixel_count != u64::MAX
    {
        CandidateStatus::Passed
    } else {
        CandidateStatus::Rejected
    };
    CandidateReport {
        candidate_id: candidate.id.to_owned(),
        transitions: reports,
        aggregates,
        status,
    }
}

fn compare_transition(
    kind: CandidateKind,
    from: &LoadedFrame,
    to: &LoadedFrame,
    roi: Roi,
) -> Result<(EvaluationDecision, u64), CandidateFailureCode> {
    match kind {
        CandidateKind::Exact => compare_changed_pixel_count(from, to, roi, 1),
        CandidateKind::ChangedPixelCount(minimum) => {
            compare_changed_pixel_count(from, to, roi, minimum)
        }
        CandidateKind::SampledExact(stride) => compare_sampled_exact(from, to, roi, stride),
        #[cfg(test)]
        CandidateKind::InjectedFailure => Err(CandidateFailureCode::InjectedFailure),
    }
}

fn compare_changed_pixel_count(
    from: &LoadedFrame,
    to: &LoadedFrame,
    roi: Roi,
    minimum: u64,
) -> Result<(EvaluationDecision, u64), CandidateFailureCode> {
    let mut inspected = 0_u64;
    let mut changed = 0_u64;
    for y in roi.y..roi.y + roi.height {
        for x in roi.x..roi.x + roi.width {
            inspected = inspected
                .checked_add(1)
                .ok_or(CandidateFailureCode::ArithmeticOverflow)?;
            if pixel(from, x, y)? != pixel(to, x, y)? {
                changed = changed
                    .checked_add(1)
                    .ok_or(CandidateFailureCode::ArithmeticOverflow)?;
                if changed >= minimum {
                    return Ok((EvaluationDecision::AnalysisRequired, inspected));
                }
            }
        }
    }
    Ok((EvaluationDecision::Unchanged, inspected))
}

fn compare_sampled_exact(
    from: &LoadedFrame,
    to: &LoadedFrame,
    roi: Roi,
    stride: usize,
) -> Result<(EvaluationDecision, u64), CandidateFailureCode> {
    let mut inspected = 0_u64;
    for relative_y in (0..roi.height).step_by(stride) {
        for relative_x in (0..roi.width).step_by(stride) {
            inspected = inspected
                .checked_add(1)
                .ok_or(CandidateFailureCode::ArithmeticOverflow)?;
            if pixel(from, roi.x + relative_x, roi.y + relative_y)?
                != pixel(to, roi.x + relative_x, roi.y + relative_y)?
            {
                return Ok((EvaluationDecision::AnalysisRequired, inspected));
            }
        }
    }
    Ok((EvaluationDecision::Unchanged, inspected))
}

fn pixel(frame: &LoadedFrame, x: usize, y: usize) -> Result<&[u8], CandidateFailureCode> {
    let row = y
        .checked_mul(frame.row_stride)
        .ok_or(CandidateFailureCode::ArithmeticOverflow)?;
    let column = x
        .checked_mul(usize::try_from(BYTES_PER_PIXEL).unwrap_or(4))
        .ok_or(CandidateFailureCode::ArithmeticOverflow)?;
    let start = row
        .checked_add(column)
        .ok_or(CandidateFailureCode::ArithmeticOverflow)?;
    let end = start
        .checked_add(usize::try_from(BYTES_PER_PIXEL).unwrap_or(4))
        .ok_or(CandidateFailureCode::ArithmeticOverflow)?;
    frame
        .pixels
        .get(start..end)
        .ok_or(CandidateFailureCode::PixelBounds)
}

fn validate_identifier(identifier: &str) -> Result<(), EvaluationError> {
    let valid = !identifier.is_empty()
        && identifier.len() <= 64
        && identifier.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (byte == b'-' && index > 0 && index + 1 < identifier.len())
        });
    if valid {
        Ok(())
    } else {
        Err(EvaluationError::new(EvaluationErrorKind::InvalidIdentifier))
    }
}

fn validate_transition_identifier(identifier: &str) -> Result<(), EvaluationError> {
    let Some((sequence, index)) = identifier.split_once('/') else {
        return Err(EvaluationError::new(EvaluationErrorKind::InvalidIdentifier));
    };
    validate_identifier(sequence)?;
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EvaluationError::new(EvaluationErrorKind::InvalidIdentifier));
    }
    Ok(())
}

fn validate_digest(digest: &str, component_id: &str) -> Result<(), EvaluationError> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(EvaluationError::for_component(
            EvaluationErrorKind::InvalidDigest,
            component_id,
        ))
    }
}

fn checked_usize(value: u64, component_id: &str) -> Result<usize, EvaluationError> {
    usize::try_from(value).map_err(|_| {
        EvaluationError::for_component(EvaluationErrorKind::ArithmeticOverflow, component_id)
    })
}

fn read_repository_file(
    repository_root: &Path,
    relative: &str,
    maximum_bytes: u64,
    component_id: Option<&str>,
) -> Result<Vec<u8>, EvaluationError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() || relative.contains('\\') {
        return Err(component_error(
            EvaluationErrorKind::ComponentNotRegular,
            component_id,
        ));
    }

    let mut path = repository_root.to_path_buf();
    let components: Vec<_> = relative_path.components().collect();
    if components.is_empty() {
        return Err(component_error(
            EvaluationErrorKind::ComponentNotRegular,
            component_id,
        ));
    }
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(component_error(
                EvaluationErrorKind::ComponentNotRegular,
                component_id,
            ));
        };
        path.push(name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            component_error(EvaluationErrorKind::ComponentUnavailable, component_id)
        })?;
        let is_last = index + 1 == components.len();
        if metadata.file_type().is_symlink()
            || (is_last && !metadata.file_type().is_file())
            || (!is_last && !metadata.file_type().is_dir())
        {
            return Err(component_error(
                EvaluationErrorKind::ComponentNotRegular,
                component_id,
            ));
        }
        if is_last && metadata.len() > maximum_bytes {
            return Err(component_error(
                EvaluationErrorKind::ComponentTooLarge,
                component_id,
            ));
        }
    }

    fs::read(path)
        .map_err(|_| component_error(EvaluationErrorKind::ComponentUnavailable, component_id))
}

fn component_error(kind: EvaluationErrorKind, component_id: Option<&str>) -> EvaluationError {
    component_id.map_or_else(
        || EvaluationError::new(kind),
        |id| EvaluationError::for_component(kind, id),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut result = String::with_capacity(64);
    for byte in digest {
        use fmt::Write as _;
        write!(&mut result, "{byte:02x}").expect("writing into String cannot fail");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_candidate_failure_is_retained_and_rejected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let sequences = RecordedSequenceSet::load(&root).expect("frozen sequence set");
        let report = run_candidate(
            &sequences,
            Candidate::new("injected-failure", CandidateKind::InjectedFailure),
        );

        assert_eq!(report.status(), CandidateStatus::Rejected);
        assert!(report.aggregates().candidate_failure_count() > 0);
        assert_eq!(report.transitions().len(), sequences.transition_count());
    }
}
