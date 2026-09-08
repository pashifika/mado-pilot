//! Benchmark-private, capture-free controller for independently linked native consumers.
//!
//! Only the consumer opens public sessions. Fixture acknowledgements are control
//! facts, never evidence that a consumer observed the corresponding pixels.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use super::scaled_u32;
use super::{
    Arguments, ControlAcknowledgement, MARKER_CELL_LOGICAL, MARKER_PRIMARY, MARKER_SECONDARY,
    MARKER_X_LOGICAL, MARKER_Y_LOGICAL, NativeFixture, NativeResourceFacts, POLL_WAIT,
    TOKEN_CELL_LOGICAL, TOKEN_X_LOGICAL, TOKEN_Y_LOGICAL, VisualMarkerState,
    accept_control_acknowledgement, issue_fixture_visual_state, native_watch_report, png,
    scaled_i32,
};

#[path = "native_foreign_resources.rs"]
mod resources;

use resources::{ResourceAdmission, ResourceArtifacts, ResourceDecision, ResourceRequest};

const LINE_LIMIT: usize = 2_048;
const QUEUE_LIMIT: usize = 64;
const RECORD_LIMIT: usize = 512;
const FACTS_PER_ROW: usize = 64;
const STDERR_LIMIT: u64 = 65_536;
const REPLY_WAIT: Duration = Duration::from_secs(5);
const LIFECYCLE_WAIT: Duration = Duration::from_secs(10);
const RUN_WAIT: Duration = Duration::from_secs(600);
const ROW_WAIT: Duration = Duration::from_secs(120);
const MAX_DIMENSION: u64 = 32_768;
const MAX_CELL: u64 = 384;
const MAX_ARTIFACT_BYTES: u64 = 2 * 1_024 * 1_024 * 1_024;
const MAX_NANOS: u64 = 600_000_000_000;
const WARMUP_COUNT: usize = 1;
const MEASUREMENT_COUNT: usize = 3;
const CYCLE_COUNT: usize = WARMUP_COUNT + MEASUREMENT_COUNT;
const CYCLE_DIRECTORIES: [&str; CYCLE_COUNT] = [
    "warmup",
    "measurement-001",
    "measurement-002",
    "measurement-003",
];
type Failure = &'static str;
type Checked<T> = Result<T, Failure>;

fn require_authority(deadline: Instant, reserve: Duration) -> Checked<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() || remaining < reserve {
        return Err("cohort_deadline_exhausted");
    }
    Ok(())
}

struct ForeignArguments {
    consumer: PathBuf,
    fixture: PathBuf,
    directory: PathBuf,
    library: PathBuf,
    resources: Option<ResourceRequest>,
}

impl ForeignArguments {
    fn parse(raw: &[OsString]) -> Checked<Self> {
        if !matches!(raw.len(), 4 | 12) {
            return Err("invalid_arguments");
        }
        let mut paths: [Option<PathBuf>; 4] = [None, None, None, None];
        let mut resources = [None; ResourceRequest::KEYS.len()];
        for arg in raw {
            let text = arg.to_str().ok_or("invalid_arguments")?;
            let (key, value) = text.split_once('=').ok_or("invalid_arguments")?;
            let index = match key {
                "--foreign-consumer" => Some(0),
                "--fixture-executable" => Some(1),
                "--foreign-artifact-dir" => Some(2),
                "--foreign-library" => Some(3),
                _ => None,
            };
            let Some(index) = index else {
                let index = ResourceRequest::KEYS
                    .iter()
                    .position(|candidate| *candidate == key)
                    .ok_or("invalid_arguments")?;
                if resources[index].replace(value).is_some() {
                    return Err("invalid_arguments");
                }
                continue;
            };
            let path = PathBuf::from(value);
            if paths[index].is_some() || !path.is_absolute() || !safe_payload(value) {
                return Err("invalid_arguments");
            }
            paths[index] = Some(path);
        }
        let [consumer, fixture, directory, library] = paths;
        Ok(Self {
            consumer: consumer.ok_or("invalid_arguments")?,
            fixture: fixture.ok_or("invalid_arguments")?,
            directory: directory.ok_or("invalid_arguments")?,
            library: library.ok_or("invalid_arguments")?,
            resources: ResourceRequest::parse(resources)?,
        })
    }
}

/// Dispatch before either legacy argument parser; foreign mode never inherits
/// qualification defaults or silently accepts a legacy switch.
pub(super) fn dispatch() -> bool {
    let raw: Vec<_> = std::env::args_os().skip(1).collect();
    if !raw
        .iter()
        .any(|arg| arg.to_string_lossy().starts_with("--foreign-"))
    {
        return false;
    }
    let code = match ForeignArguments::parse(&raw).and_then(execute) {
        Ok(code) => code,
        Err(reason) => {
            eprintln!("native_foreign_watch:{reason}");
            2
        }
    };
    std::process::exit(code);
}

fn safe_payload(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LINE_LIMIT - 16
        && !value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    sha256: String,
    size: u64,
}

struct Artifact {
    role: &'static str,
    path: PathBuf,
    canonical: PathBuf,
    // Retain the original file object through finalization, even if its name is
    // replaced. The original and named bytes must both retain their metadata.
    file: File,
    metadata: fs::Metadata,
    before: Identity,
    after: Option<Identity>,
    unchanged: bool,
}

fn identity(path: &Path, deadline: Instant) -> Checked<Identity> {
    require_authority(deadline, Duration::ZERO)?;
    let before = fs::metadata(path).map_err(|_| "artifact_unavailable")?;
    if !before.is_file() || before.len() == 0 || before.len() > MAX_ARTIFACT_BYTES {
        return Err("artifact_invalid");
    }
    require_authority(deadline, Duration::ZERO)?;
    let sha256 = native_watch_report::artifact_sha256(path).ok_or("artifact_unreadable")?;
    require_authority(deadline, Duration::ZERO)?;
    let after = fs::metadata(path).map_err(|_| "artifact_unavailable")?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("artifact_changed");
    }
    Ok(Identity {
        sha256,
        size: after.len(),
    })
}

fn same_file_metadata(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return false;
        }
    }
    before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && before.created().ok() == after.created().ok()
}

impl Artifact {
    fn pin(role: &'static str, path: PathBuf, deadline: Instant) -> Checked<Self> {
        require_authority(deadline, Duration::ZERO)?;
        let canonical = path.canonicalize().map_err(|_| "artifact_unavailable")?;
        require_authority(deadline, Duration::ZERO)?;
        let file = File::open(&canonical).map_err(|_| "artifact_unavailable")?;
        let metadata = file.metadata().map_err(|_| "artifact_unavailable")?;
        let before = identity(&path, deadline)?;
        if metadata.len() != before.size
            || !same_file_metadata(
                &metadata,
                &fs::metadata(&canonical).map_err(|_| "artifact_unavailable")?,
            )
        {
            return Err("artifact_changed");
        }
        Ok(Self {
            role,
            path,
            canonical,
            file,
            metadata,
            before,
            after: None,
            unchanged: false,
        })
    }

    fn finalize(&mut self, deadline: Instant) -> bool {
        self.after = identity(&self.path, deadline).ok();
        self.unchanged = self.after.as_ref() == Some(&self.before)
            && self.path.canonicalize().ok().as_ref() == Some(&self.canonical)
            && self
                .file
                .metadata()
                .is_ok_and(|meta| same_file_metadata(&self.metadata, &meta))
            && identity(&self.canonical, deadline).ok().as_ref() == Some(&self.before);
        self.unchanged
    }
}

fn private_directory(path: &Path) -> Checked<()> {
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt as _;
        fs::DirBuilder::new().mode(0o700).create(path)
    };
    #[cfg(windows)]
    let created = fs::create_dir(path);
    created.map_err(|_| "output_directory_unavailable")
}

fn write_new(path: &Path, bytes: &[u8], immutable: bool) -> Checked<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| "output_create_failed")?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "output_write_failed")?;
    if immutable {
        let mut permissions = file
            .metadata()
            .map_err(|_| "output_write_failed")?
            .permissions();
        permissions.set_readonly(true);
        file.set_permissions(permissions)
            .map_err(|_| "output_write_failed")?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Pass,
    Fail,
    Unsupported,
    Unexecuted,
    Infra,
}

impl Outcome {
    fn parse(value: &str) -> Checked<Self> {
        match value {
            "PASS" => Ok(Self::Pass),
            "FAIL" => Ok(Self::Fail),
            "UNSUPPORTED" => Ok(Self::Unsupported),
            "UNEXECUTED" => Ok(Self::Unexecuted),
            "INFRA" => Ok(Self::Infra),
            _ => Err("invalid_outcome"),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Unsupported => "UNSUPPORTED",
            Self::Unexecuted => "UNEXECUTED",
            Self::Infra => "INFRA",
        }
    }

    const fn exit_code(self) -> i32 {
        match self {
            Self::Pass => 0,
            Self::Infra => 2,
            _ => 1,
        }
    }

    fn combine(self, other: Self) -> Self {
        for outcome in [Self::Infra, Self::Fail, Self::Unsupported, Self::Unexecuted] {
            if self == outcome || other == outcome {
                return outcome;
            }
        }
        Self::Pass
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Before,
    Final,
}

impl Phase {
    const fn name(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::Final => "final",
        }
    }

    fn aggregate(self, observed: Outcome, infra: bool) -> Outcome {
        if infra {
            Outcome::Infra
        } else if self == Self::Before {
            Outcome::Unexecuted
        } else {
            observed
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CycleRole {
    Warmup,
    Measurement,
}

impl CycleRole {
    const fn at(index: usize) -> Self {
        if index == 0 {
            Self::Warmup
        } else {
            Self::Measurement
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::Measurement => "measurement",
        }
    }

    const fn scope(self) -> &'static str {
        match self {
            Self::Warmup => "initialization_observation",
            Self::Measurement => "steady_state",
        }
    }

    const fn baseline_reference(self) -> &'static str {
        match self {
            Self::Warmup => "cycle_before",
            Self::Measurement => "fixed_postwarmup",
        }
    }

    const fn baseline_enforced(self) -> bool {
        matches!(self, Self::Measurement)
    }
}

struct CycleSummary {
    aggregate: Outcome,
    eligible: bool,
    before_written: bool,
    report_written: bool,
    output_error: Option<Failure>,
    resource: Option<ResourceDecision>,
}

impl CycleSummary {
    fn persist_report(&mut self, path: &Path, text: &str, mut resource: ResourceDecision) {
        let written = write_new(path, text.as_bytes(), true);
        self.report_written = written.is_ok();
        if let Err(error) = written {
            self.output_error = Some(error);
            resource.mark_infrastructure_failure();
        }
        self.aggregate = resource.aggregate();
        self.eligible = resource.eligible();
        self.resource = Some(resource);
    }
}

fn cycle_may_launch(
    cycles: &[Option<CycleSummary>; CYCLE_COUNT],
    index: usize,
    infra: bool,
) -> bool {
    !infra
        && index < CYCLE_COUNT
        && cycles[index].is_none()
        && cycles[..index]
            .iter()
            .all(|cycle| cycle.as_ref().is_some_and(|cycle| cycle.eligible))
}

fn cohort_aggregate(
    cycles: &[Option<CycleSummary>; CYCLE_COUNT],
    phase: Phase,
    infra: bool,
) -> Outcome {
    let observed = cycles.iter().fold(Outcome::Pass, |aggregate, cycle| {
        aggregate.combine(
            cycle
                .as_ref()
                .map_or(Outcome::Unexecuted, |cycle| cycle.aggregate),
        )
    });
    phase.aggregate(observed, infra)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FactKey {
    Frame,
    Bootstrap,
    Transform,
    Match,
    Status,
    Startup,
    Elapsed,
    Mapped,
    Pending,
    Permissions,
    Resource,
}

impl FactKey {
    fn parse(key: &str) -> Checked<Self> {
        match key {
            "frame" => Ok(Self::Frame),
            "bootstrap" => Ok(Self::Bootstrap),
            "transform" => Ok(Self::Transform),
            "match" => Ok(Self::Match),
            "status" => Ok(Self::Status),
            "startup_nanos" => Ok(Self::Startup),
            "elapsed_nanos" => Ok(Self::Elapsed),
            "mapped_view_bytes" => Ok(Self::Mapped),
            "pending" => Ok(Self::Pending),
            "permissions" => Ok(Self::Permissions),
            "resource" => Ok(Self::Resource),
            _ => Err("unknown_fact"),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Bootstrap => "bootstrap",
            Self::Transform => "transform",
            Self::Match => "match",
            Self::Status => "status",
            Self::Startup => "startup_nanos",
            Self::Elapsed => "elapsed_nanos",
            Self::Mapped => "mapped_view_bytes",
            Self::Pending => "pending",
            Self::Permissions => "permissions",
            Self::Resource => "resource",
        }
    }

    const fn arity(self) -> usize {
        match self {
            Self::Frame | Self::Bootstrap => 6,
            Self::Transform => 8,
            Self::Match => 9,
            Self::Status | Self::Permissions => 2,
            Self::Pending => 3,
            Self::Startup | Self::Elapsed | Self::Mapped => 1,
            Self::Resource => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Number {
    Unsigned(u64),
    Real(f64),
}

impl Number {
    fn unsigned(self) -> Option<u64> {
        match self {
            Self::Unsigned(value) => Some(value),
            Self::Real(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Fact {
    key: FactKey,
    values: Vec<Number>,
}

fn unsigned(text: &str, min: u64, max: u64) -> Checked<u64> {
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid_integer");
    }
    text.parse::<u64>()
        .ok()
        .filter(|value| (min..=max).contains(value))
        .ok_or("invalid_integer")
}

fn real(text: &str, min: f64, max: f64) -> Checked<f64> {
    if text.is_empty()
        || text.len() > 32
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E'))
    {
        return Err("invalid_number");
    }
    text.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && (min..=max).contains(value))
        .ok_or("invalid_number")
}

impl Fact {
    fn parse(key: FactKey, words: &[&str]) -> Checked<Self> {
        if words.len() != key.arity() {
            return Err("invalid_fact_arity");
        }
        let mut values = Vec::with_capacity(words.len());
        for (index, word) in words.iter().enumerate() {
            let value = match (key, index) {
                (FactKey::Transform, 1 | 2) => Number::Real(real(word, -1_000_000.0, 1_000_000.0)?),
                (FactKey::Transform, 3 | 4) => Number::Real(real(word, 1.0, MAX_DIMENSION as f64)?),
                (FactKey::Transform, 5 | 6) => Number::Real(real(word, 0.125, 16.0)?),
                (FactKey::Match, 5..=8) => Number::Real(real(word, 0.0, MAX_DIMENSION as f64)?),
                _ => {
                    let (min, max) = match (key, index) {
                        (FactKey::Frame | FactKey::Bootstrap, 0) => (1, u64::MAX),
                        (FactKey::Frame | FactKey::Bootstrap, 1..=3) => (0, u64::MAX),
                        (FactKey::Frame | FactKey::Bootstrap, 4 | 5) => (1, MAX_DIMENSION),
                        (FactKey::Transform, 0) => (0, u64::MAX),
                        (FactKey::Match, 0) => (1, u64::MAX),
                        (FactKey::Transform, 7) | (FactKey::Match, 1) => (0, u64::from(u32::MAX)),
                        (FactKey::Match, 2) => (1, 64),
                        (FactKey::Match, 3) | (FactKey::Pending, 0 | 1) => (0, 1_000_000_000),
                        (FactKey::Status, 0) => (0, 13),
                        (FactKey::Status, 1) => (0, 8),
                        (FactKey::Permissions, _) => (0, 3),
                        (FactKey::Mapped, _) => (1, 1_073_741_824),
                        (FactKey::Resource, 0) => (0, 2),
                        (FactKey::Resource, 1) => (0, 4),
                        (FactKey::Resource, 2 | 3) => (0, 1 << 50),
                        (FactKey::Resource, 4) => (0, 16_777_216),
                        _ => (0, MAX_NANOS),
                    };
                    Number::Unsigned(unsigned(word, min, max)?)
                }
            };
            values.push(value);
        }
        if key == FactKey::Match {
            let [
                Number::Real(left),
                Number::Real(top),
                Number::Real(right),
                Number::Real(bottom),
            ] = values[5..]
            else {
                return Err("invalid_match_bounds");
            };
            if left >= right || top >= bottom {
                return Err("invalid_match_bounds");
            }
        }
        Ok(Self { key, values })
    }
}

fn row_index(text: &str) -> Checked<usize> {
    match text.as_bytes() {
        [b'F', value @ b'1'..=b'9'] => Ok(usize::from(*value - b'1')),
        _ => Err("invalid_row"),
    }
}

const PASS_REASONS: [&str; 9] = [
    "admission_observed",
    "absent_completed_pending",
    "exact_match_correlated",
    "resize_correlated",
    "movement_correlated",
    "independent_authority",
    "retained_ownership",
    "lifecycle_terminals",
    "consumer_cleanup",
];

fn reason_allowed(reason: &str) -> bool {
    PASS_REASONS.contains(&reason)
        || matches!(
            reason,
            "topology_unavailable"
                | "resize_unavailable"
                | "prior_row_failed"
                | "contract_mismatch"
                | "public_call_failed"
                | "clock_overflow"
                | "observation_deadline"
                | "protocol_failed"
                | "frame_contract_failed"
                | "mapping_contract_failed"
                | "native_platform_unavailable"
                | "admission_contract_failed"
                | "scheduler_contract_failed"
                | "permission_contract_failed"
                | "capture_permission_not_granted"
                | "target_count_exceeded"
                | "target_capability_unavailable"
                | "owned_target_not_unique"
                | "ownership_contract_failed"
                | "prepared_template_mismatch"
                | "terminal_contract_failed"
                | "bootstrap_not_matched"
                | "bootstrap_geometry_mismatch"
                | "failure_output_not_reset"
                | "unexpected_geometry"
                | "frame_order_mismatch"
                | "pending_contract_failed"
                | "matched_contract_failed"
                | "match_geometry_mismatch"
                | "match_bounds_mismatch"
                | "exact_result_token_mismatch"
                | "nonmatched_contract_failed"
                | "terminal_winner_changed"
                | "geometry_transition_missing"
                | "movement_not_observed"
                | "scale_transition_missing"
                | "resize_not_observed"
                | "pending_generation_mismatch"
                | "caller_wait_cancel_failed"
                | "caller_wait_deadline_failed"
                | "retained_ownership_failed"
                | "abi_extent_unavailable"
                | "abi_lifecycle_unavailable"
                | "consumer_cleanup_failed"
                | "resource_snapshot_failed"
                | "resource_budget_unaccepted"
                | "abi_extent_mismatch"
                | "abi_negotiation_refused"
                | "abi_unavailable"
                | "asset_protocol_failure"
                | "backend_version_changed"
                | "backend_version_invalid"
                | "before_visual_not_pending"
                | "before_visual_poll_failed"
                | "bootstrap_frame_failed"
                | "bootstrap_frame_info_failed"
                | "bootstrap_frame_mismatch"
                | "bootstrap_geometry_stale"
                | "bootstrap_metadata_unavailable"
                | "bootstrap_result_failed"
                | "bootstrap_session_mismatch"
                | "bootstrap_stamp_failed"
                | "build_descriptor_failed"
                | "caller_cancel_authority_mismatch"
                | "caller_cancellation_create_failed"
                | "caller_cancellation_failed"
                | "caller_deadline_authority_mismatch"
                | "caller_wait_replaced_query"
                | "cancelled_poll_failed"
                | "cancelled_poll_mismatch"
                | "capabilities_failed"
                | "clock_failed"
                | "consumer_exception"
                | "deadline_overflow"
                | "expected_pending_query"
                | "expired_query_cancel_failed"
                | "frame_after_result_mismatch"
                | "frame_describe_failed"
                | "frame_geometry_mismatch"
                | "frame_map_failed"
                | "frame_stamp_failed"
                | "fresh_session_stream_mismatch"
                | "geometry_action_failed"
                | "geometry_frame_failed"
                | "geometry_frame_info_failed"
                | "geometry_protocol_failure"
                | "geometry_stamp_failed"
                | "geometry_stream_replaced"
                | "indexed_match_contract_mismatch"
                | "indexed_match_failed"
                | "lifecycle_repeat_cancel_failed"
                | "mapping_after_frame_failed"
                | "mapping_after_frame_mismatch"
                | "mapping_after_frame_stamp_failed"
                | "mapping_correlation_mismatch"
                | "mapping_describe_failed"
                | "mapping_stamp_failed"
                | "match_result_failed"
                | "matched_cancel_failed"
                | "matched_cancel_info_failed"
                | "matched_contract_mismatch"
                | "matched_error_failed"
                | "matched_frame_failed"
                | "matched_frame_identity_mismatch"
                | "matched_frame_stamp_failed"
                | "matched_terminal_mutated"
                | "moved_query_refusal_mismatch"
                | "movement_effective_scale_unchanged"
                | "movement_origin_unchanged"
                | "movement_topology_unavailable"
                | "native_capability_mismatch"
                | "native_discovery_refused"
                | "native_engine_refused"
                | "native_session_refused"
                | "no_consumer_owners"
                | "nonmatched_terminal_contract_mismatch"
                | "owned_title_not_discovered"
                | "owned_title_not_unique"
                | "package_load_failed"
                | "pending_poll_failed"
                | "pending_stability_mismatch"
                | "permission_capability_mismatch"
                | "permission_kind_mismatch"
                | "permission_probe_failed"
                | "permission_refusal_mismatch"
                | "prepared_asset_mismatch"
                | "prior_row_unavailable"
                | "protocol_failure"
                | "query_cancel_failed"
                | "query_clone_failed"
                | "query_move_mismatch"
                | "query_owner_missing"
                | "query_repeat_cancel_failed"
                | "query_start_failed"
                | "query_wait_failed"
                | "resize_extent_unchanged"
                | "result_clone_failed"
                | "retained_borrowed_views_mismatch"
                | "retained_frame_failed"
                | "retained_index_failed"
                | "retained_mapping_failed"
                | "retained_mapping_stamp_failed"
                | "retained_move_mismatch"
                | "retained_result_failed"
                | "scheduler_contract_mismatch"
                | "scheduler_descriptor_failed"
                | "session_close_failed"
                | "session_close_not_observed"
                | "session_closed_state_failed"
                | "session_describe_failed"
                | "session_repeat_close_failed"
                | "session_target_mismatch"
                | "startup_clock_mismatch"
                | "target_capability_mismatch"
                | "target_capture_unsupported"
                | "target_count_failed"
                | "target_descriptor_failed"
                | "target_destroy_failed"
                | "template_describe_failed"
                | "template_prepare_failed"
                | "terminal_describe_failed"
                | "terminal_error_failed"
                | "token_frame_failed"
                | "token_stamp_failed"
                | "visible_query_identity_mismatch"
                | "visible_result_info_failed"
                | "visual_protocol_failure"
        )
}

#[derive(Default)]
struct Row {
    outcome: Option<Outcome>,
    reason: Option<String>,
    facts: Vec<Fact>,
}

struct Ledger {
    rows: [Row; 9],
    next: usize,
    done: bool,
    loaded: bool,
}

impl Ledger {
    fn new() -> Self {
        Self {
            rows: std::array::from_fn(|_| Row::default()),
            next: 0,
            done: false,
            loaded: false,
        }
    }

    fn fact(&mut self, row: usize, fact: Fact) -> Checked<()> {
        if !self.loaded || self.done || row != self.next {
            return Err("fact_order");
        }
        if self.rows[row].facts.len() == FACTS_PER_ROW {
            return Err("fact_overflow");
        }
        if fact.key == FactKey::Bootstrap && !matches!(row, 0 | 1 | 3 | 4 | 6 | 7) {
            return Err("inapplicable_fact");
        }
        if fact.key == FactKey::Resource {
            let index = self.rows[row]
                .facts
                .iter()
                .filter(|fact| fact.key == FactKey::Resource)
                .count();
            resources::validate_sample(row, index, &fact)?;
            if row == 0
                && self.rows[row]
                    .facts
                    .iter()
                    .filter(|fact| fact.key == FactKey::Bootstrap)
                    .count()
                    != index + 1
            {
                return Err("resource_bootstrap_order");
            }
        }
        self.rows[row].facts.push(fact);
        Ok(())
    }

    fn row(&mut self, row: usize, outcome: Outcome, reason: &str) -> Checked<()> {
        if !self.loaded || self.done || row != self.next {
            return Err("row_order");
        }
        if !reason_allowed(reason) {
            return Err("unknown_reason");
        }
        if outcome == Outcome::Pass {
            if reason != PASS_REASONS[row] {
                return Err("invalid_pass_reason");
            }
            self.required_facts(row)?;
        } else if matches!(
            reason,
            "topology_unavailable" | "movement_topology_unavailable" | "resize_unavailable"
        ) && outcome != Outcome::Unexecuted
        {
            return Err("invalid_prerequisite_outcome");
        }
        self.rows[row].outcome = Some(outcome);
        self.rows[row].reason = Some(reason.to_owned());
        self.next += 1;
        Ok(())
    }

    fn required_facts(&self, row: usize) -> Checked<()> {
        use FactKey::{
            Bootstrap, Frame, Mapped, Match, Pending, Permissions, Resource, Startup, Status,
            Transform,
        };
        let required: &[(FactKey, usize)] = match row {
            0 => &[(Permissions, 1), (Status, 1), (Bootstrap, 4), (Resource, 4)],
            1 => &[
                (Bootstrap, 1),
                (Frame, 1),
                (Transform, 1),
                (Pending, 1),
                (Startup, 1),
            ],
            2 => &[(Frame, 1), (Transform, 1), (Match, 1), (Mapped, 1)],
            3 | 4 => &[
                (Bootstrap, 1),
                (Frame, 2),
                (Transform, 2),
                (Pending, 1),
                (Match, 1),
                (Mapped, 1),
            ],
            5 => &[(Status, 4), (Pending, 1)],
            6 => &[
                (Frame, 2),
                (Transform, 2),
                (Match, 1),
                (Mapped, 1),
                (Bootstrap, 1),
            ],
            7 => &[(Status, 3), (Bootstrap, 2)],
            8 => &[(Status, 1), (Resource, 1)],
            _ => return Err("invalid_row"),
        };
        let facts = &self.rows[row].facts;
        if required
            .iter()
            .any(|(key, count)| facts.iter().filter(|fact| fact.key == *key).count() < *count)
        {
            return Err("required_fact_missing");
        }
        let bootstrap_count = facts.iter().filter(|fact| fact.key == Bootstrap).count();
        let expected_bootstrap = match row {
            0 => 4,
            1 | 3 | 4 | 6 => 1,
            7 => 2,
            _ => 0,
        };
        if bootstrap_count != expected_bootstrap {
            return Err("bootstrap_stage_count");
        }
        if matches!(row, 1 | 3 | 4)
            && !facts.iter().any(|fact| {
                fact.key == Pending
                    && fact.values[0].unsigned().is_some_and(|value| value > 0)
                    && fact.values[1..] == [Number::Unsigned(0), Number::Unsigned(0)]
            })
        {
            return Err("pending_not_proven");
        }
        if matches!(row, 3 | 4) {
            let mut frames = facts.iter().filter(|fact| fact.key == Frame);
            let first = &frames.next().ok_or("required_fact_missing")?.values;
            let last = &frames.next_back().ok_or("required_fact_missing")?.values;
            if first[0] != last[0] || (first[1] == last[1] && first[3] == last[3]) {
                return Err("geometry_transition_missing");
            }
            if row == 3 && first[4..] == last[4..] {
                return Err("resize_extent_unchanged");
            }
            let mut transforms = facts.iter().filter(|fact| fact.key == Transform);
            let first_transform = &transforms.next().ok_or("required_fact_missing")?.values;
            let last_transform = &transforms
                .next_back()
                .ok_or("required_fact_missing")?
                .values;
            if first_transform[0] != first[3] || last_transform[0] != last[3] {
                return Err("frame_geometry_mismatch");
            }
            if row == 4 {
                if first_transform[1..3] == last_transform[1..3] {
                    return Err("movement_origin_unchanged");
                }
                #[cfg(windows)]
                if first_transform[5..7] == last_transform[5..7] {
                    return Err("movement_effective_scale_unchanged");
                }
            }
        }
        let admitted_capture_permission = if cfg!(windows) { 3 } else { 1 };
        if row == 0
            && (!facts.iter().any(|fact| {
                fact.key == Permissions
                    && fact.values[0] == Number::Unsigned(admitted_capture_permission)
            }) || !facts.iter().any(|fact| {
                fact.key == Status && fact.values == [Number::Unsigned(0), Number::Unsigned(0)]
            }))
        {
            return Err("admission_contract_failed");
        }
        if row == 5 {
            for (call, terminal) in [(3, 0), (4, 0), (0, 2), (0, 3)] {
                if !facts.iter().any(|fact| {
                    fact.key == Status
                        && fact.values == [Number::Unsigned(call), Number::Unsigned(terminal)]
                }) {
                    return Err("independent_authority_missing");
                }
            }
        }
        if row == 7 {
            for terminal in [4, 5, 6] {
                if !facts.iter().any(|fact| {
                    fact.key == Status
                        && fact.values == [Number::Unsigned(0), Number::Unsigned(terminal)]
                }) {
                    return Err("lifecycle_terminal_missing");
                }
            }
        }
        if row == 8
            && !facts.iter().any(|fact| {
                fact.key == Status && fact.values == [Number::Unsigned(0), Number::Unsigned(0)]
            })
        {
            return Err("consumer_cleanup_missing");
        }
        Ok(())
    }

    fn finish(&mut self) -> Checked<()> {
        if self.done || self.next != 9 || !self.loaded {
            return Err("incomplete_ledger");
        }
        self.done = true;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Geometry {
    width: u32,
    height: u32,
    #[cfg(windows)]
    desktop: [f64; 2],
    #[cfg(target_os = "macos")]
    logical: [f64; 2],
    #[cfg(target_os = "macos")]
    scale: [f64; 2],
}

impl Geometry {
    fn parse(words: &[&str]) -> Checked<Self> {
        if words.len() != 8 {
            return Err("invalid_geometry_arity");
        }
        let _desktop = [
            real(words[2], -1_000_000.0, 1_000_000.0)?,
            real(words[3], -1_000_000.0, 1_000_000.0)?,
        ];
        let width =
            u32::try_from(unsigned(words[0], 1, MAX_DIMENSION)?).map_err(|_| "invalid_integer")?;
        let height =
            u32::try_from(unsigned(words[1], 1, MAX_DIMENSION)?).map_err(|_| "invalid_integer")?;
        let _logical = [
            real(words[4], 1.0, MAX_DIMENSION as f64)?,
            real(words[5], 1.0, MAX_DIMENSION as f64)?,
        ];
        let _scale = [real(words[6], 0.125, 16.0)?, real(words[7], 0.125, 16.0)?];
        Ok(Self {
            width,
            height,
            #[cfg(windows)]
            desktop: _desktop,
            #[cfg(target_os = "macos")]
            logical: _logical,
            #[cfg(target_os = "macos")]
            scale: _scale,
        })
    }
}

enum Request<'a> {
    Loaded(&'a str),
    Visual(VisualMarkerState),
    Geometry(Geometry),
    Asset(u32, u32),
    Resize,
    Move,
    Destroy,
    Fact(usize, Fact),
    Row(usize, Outcome, &'a str),
    Done,
}

fn parse_request(line: &str) -> Checked<Request<'_>> {
    if let Some(path) = line.strip_prefix("LOADED ") {
        if safe_payload(path) && Path::new(path).is_absolute() {
            return Ok(Request::Loaded(path));
        }
        return Err("invalid_loaded_path");
    }
    if !line.is_ascii()
        || line.is_empty()
        || line.len() > LINE_LIMIT
        || line.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err("invalid_record");
    }
    let mut words = [""; 12];
    let mut count = 0;
    for word in line.split(' ') {
        if word.is_empty() || count == words.len() {
            return Err("invalid_record");
        }
        words[count] = word;
        count += 1;
    }
    match &words[..count] {
        ["VISUAL", "absent"] => Ok(Request::Visual(VisualMarkerState::Absent)),
        ["VISUAL", "visible"] => Ok(Request::Visual(VisualMarkerState::Visible)),
        ["GEOMETRY", rest @ ..] => Ok(Request::Geometry(Geometry::parse(rest)?)),
        ["ASSET", width, height] => Ok(Request::Asset(
            u32::try_from(unsigned(width, 1, MAX_CELL)?).map_err(|_| "invalid_integer")?,
            u32::try_from(unsigned(height, 1, MAX_CELL)?).map_err(|_| "invalid_integer")?,
        )),
        ["RESIZE"] => Ok(Request::Resize),
        ["MOVE"] => Ok(Request::Move),
        ["DESTROY"] => Ok(Request::Destroy),
        ["DONE"] => Ok(Request::Done),
        ["FACT", row, key, rest @ ..] => Ok(Request::Fact(
            row_index(row)?,
            Fact::parse(FactKey::parse(key)?, rest)?,
        )),
        ["ROW", row, outcome, reason] => Ok(Request::Row(
            row_index(row)?,
            Outcome::parse(outcome)?,
            reason,
        )),
        _ => Err("unknown_request"),
    }
}

// The reader is deliberately independent of the protocol parser. EOF never
// manufactures a final line, and a partial line cannot restart a reply deadline.
#[derive(Default)]
struct LineDecoder {
    bytes: Vec<u8>,
}

impl LineDecoder {
    fn push(&mut self, byte: u8) -> Checked<Option<String>> {
        if byte == b'\n' {
            if self.bytes.last() == Some(&b'\r') {
                self.bytes.pop();
            }
            if self.bytes.is_empty() || self.bytes.iter().any(|byte| matches!(byte, 0 | b'\r')) {
                return Err("invalid_line");
            }
            let line =
                String::from_utf8(std::mem::take(&mut self.bytes)).map_err(|_| "invalid_utf8")?;
            return Ok(Some(line));
        }
        // Reserve the final wire byte for LF; CR, when present, counts too.
        if self.bytes.len() >= LINE_LIMIT - 1 {
            return Err("line_overflow");
        }
        self.bytes.push(byte);
        Ok(None)
    }

    fn eof(&self) -> Checked<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err("partial_line_eof")
        }
    }
}

struct Received {
    line: String,
    at: Instant,
}

#[derive(Debug)]
enum ReadEvent {
    Started(Instant),
    Complete(String),
}

#[derive(Default)]
struct ReaderState {
    failed: AtomicBool,
    eof: AtomicBool,
    bytes: AtomicU64,
}

fn read_stdout(mut input: impl Read, sender: SyncSender<ReadEvent>, state: Arc<ReaderState>) {
    let mut decoder = LineDecoder::default();
    let mut buffer = [0u8; 1_024];
    let mut discard = false;
    loop {
        match input.read(&mut buffer) {
            Ok(0) => {
                if !discard && decoder.eof().is_err() {
                    state.failed.store(true, Ordering::Release);
                }
                state.eof.store(true, Ordering::Release);
                return;
            }
            Ok(count) => {
                if discard {
                    continue;
                }
                for byte in &buffer[..count] {
                    if decoder.bytes.is_empty()
                        && sender.try_send(ReadEvent::Started(Instant::now())).is_err()
                    {
                        state.failed.store(true, Ordering::Release);
                        discard = true;
                        break;
                    }
                    let accepted = match decoder.push(*byte) {
                        Ok(Some(line)) => sender.try_send(ReadEvent::Complete(line)).is_ok(),
                        Ok(None) => true,
                        Err(_) => false,
                    };
                    if !accepted {
                        state.failed.store(true, Ordering::Release);
                        discard = true;
                        break;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                state.failed.store(true, Ordering::Release);
                return;
            }
        }
    }
}

fn read_stderr(mut input: impl Read, state: Arc<ReaderState>) {
    let mut buffer = [0u8; 1_024];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => {
                state.eof.store(true, Ordering::Release);
                return;
            }
            Ok(count) => {
                let prior = state.bytes.load(Ordering::Relaxed);
                let total = prior.saturating_add(count as u64).min(STDERR_LIMIT + 1);
                state.bytes.store(total, Ordering::Release);
                if total > STDERR_LIMIT {
                    state.failed.store(true, Ordering::Release);
                }
                // Arbitrary child diagnostics never enter memory retained in a report.
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                state.failed.store(true, Ordering::Release);
                return;
            }
        }
    }
}

#[derive(Default)]
struct ConsumerFinalization {
    reaped: bool,
    exit_code: Option<i32>,
    exit_success: bool,
    termination_requested: bool,
    stdout_joined: bool,
    stderr_joined: bool,
    writer_joined: bool,
    stdout_eof: bool,
    stderr_eof: bool,
    output_clean: bool,
    stderr_bytes: u64,
}

impl ConsumerFinalization {
    fn accepted(&self) -> bool {
        self.reaped
            && self.exit_success
            && !self.termination_requested
            && self.stdout_joined
            && self.stderr_joined
            && self.writer_joined
            && self.stdout_eof
            && self.stderr_eof
            && self.output_clean
    }
}

struct Consumer {
    child: Option<Child>,
    lines: Receiver<ReadEvent>,
    replies: Option<SyncSender<String>>,
    written: Receiver<bool>,
    stdout: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    output: Arc<ReaderState>,
    errors: Arc<ReaderState>,
}

fn join_before(handle: Option<JoinHandle<()>>, deadline: Instant) -> bool {
    let Some(handle) = handle else {
        return true;
    };
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(POLL_WAIT);
    }
    // Dropping a live handle detaches it; never turn a timeout into an unbounded join.
    handle.is_finished() && handle.join().is_ok()
}

impl Consumer {
    fn spawn(arguments: &ForeignArguments, title: &str, deadline: Instant) -> Checked<Self> {
        require_authority(deadline, Duration::ZERO)?;
        if !safe_payload(title) {
            return Err("invalid_fixture_title");
        }
        let mut command = Command::new(&arguments.consumer);
        command.args(["--native", "--title", title]);
        command.env("LC_ALL", "C").env("LANG", "C");
        let directory = arguments
            .library
            .parent()
            .ok_or("library_directory_missing")?;
        #[cfg(target_os = "macos")]
        let loader = "DYLD_LIBRARY_PATH";
        #[cfg(windows)]
        let loader = "PATH";
        let mut paths = vec![directory.to_path_buf()];
        if let Some(current) = std::env::var_os(loader) {
            paths.extend(std::env::split_paths(&current));
        }
        command.env(
            loader,
            std::env::join_paths(paths).map_err(|_| "loader_path_invalid")?,
        );
        require_authority(deadline, Duration::ZERO)?;
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "consumer_spawn_failed")?;
        let (sender, lines) = mpsc::sync_channel(QUEUE_LIMIT);
        let (replies, requests) = mpsc::sync_channel::<String>(1);
        let (written_sender, written) = mpsc::sync_channel(1);
        let mut owner = Self {
            child: Some(child),
            lines,
            replies: Some(replies),
            written,
            stdout: None,
            stderr: None,
            writer: None,
            output: Arc::new(ReaderState::default()),
            errors: Arc::new(ReaderState::default()),
        };
        let child = owner.child.as_mut().ok_or("consumer_spawn_failed")?;
        let input = child.stdin.take().ok_or("consumer_pipe_missing")?;
        let stdout = child.stdout.take().ok_or("consumer_pipe_missing")?;
        let stderr = child.stderr.take().ok_or("consumer_pipe_missing")?;
        require_authority(deadline, Duration::ZERO)?;
        let output_state = Arc::clone(&owner.output);
        owner.stdout = Some(
            thread::Builder::new()
                .name("foreign-watch-stdout".into())
                .spawn(move || read_stdout(stdout, sender, output_state))
                .map_err(|_| "reader_spawn_failed")?,
        );
        require_authority(deadline, Duration::ZERO)?;
        let error_state = Arc::clone(&owner.errors);
        owner.stderr = Some(
            thread::Builder::new()
                .name("foreign-watch-stderr".into())
                .spawn(move || read_stderr(stderr, error_state))
                .map_err(|_| "reader_spawn_failed")?,
        );
        require_authority(deadline, Duration::ZERO)?;
        owner.writer = Some(
            thread::Builder::new()
                .name("foreign-watch-stdin".into())
                .spawn(move || {
                    let mut input = input;
                    while let Ok(reply) = requests.recv() {
                        let success = writeln!(input, "{reply}")
                            .and_then(|()| input.flush())
                            .is_ok();
                        if written_sender.try_send(success).is_err() || !success {
                            break;
                        }
                    }
                })
                .map_err(|_| "writer_spawn_failed")?,
        );
        require_authority(deadline, Duration::ZERO)?;
        Ok(owner)
    }

    fn read(&self, deadline: Instant) -> Checked<Received> {
        let mut started: Option<Instant> = None;
        loop {
            if self.output.failed.load(Ordering::Acquire)
                || self.errors.failed.load(Ordering::Acquire)
            {
                return Err("consumer_output_invalid");
            }
            let partial = started.map(|at| at + REPLY_WAIT);
            let limit = partial.map_or(deadline, |partial| partial.min(deadline));
            let timeout = if partial.is_some_and(|partial| partial <= deadline) {
                "consumer_partial_line_timeout"
            } else {
                "consumer_reply_timeout"
            };
            let remaining = limit.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(timeout);
            }
            match self.lines.recv_timeout(remaining.min(POLL_WAIT)) {
                Ok(ReadEvent::Started(at)) => {
                    if started.replace(at).is_some() {
                        return Err("consumer_output_invalid");
                    }
                }
                Ok(ReadEvent::Complete(line)) => {
                    let at = started.ok_or("consumer_output_invalid")?;
                    if Instant::now() >= limit {
                        return Err(timeout);
                    }
                    return Ok(Received { line, at });
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err("consumer_eof"),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn reply(&self, reply: String, deadline: Instant) -> Checked<Instant> {
        if reply.len() > LINE_LIMIT || !safe_payload(&reply) {
            return Err("reply_overflow");
        }
        let sent = Instant::now();
        if sent >= deadline {
            return Err("controller_reply_timeout");
        }
        self.replies
            .as_ref()
            .ok_or("consumer_stdin_closed")?
            .try_send(reply)
            .map_err(|_| "consumer_stdin_closed")?;
        match self
            .written
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(true) => Ok(sent),
            Ok(false) => Err("consumer_write_failed"),
            Err(_) => Err("consumer_write_timeout"),
        }
    }

    fn finalize(&mut self, clean_protocol: bool, cohort_deadline: Instant) -> ConsumerFinalization {
        let deadline = Instant::now() + LIFECYCLE_WAIT;
        self.replies.take();
        let mut result = ConsumerFinalization::default();
        if let Some(mut child) = self.child.take() {
            if !clean_protocol {
                result.termination_requested = true;
                let _ = child.kill();
            }
            let graceful_deadline = (Instant::now() + REPLY_WAIT).min(cohort_deadline);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        result.reaped = true;
                        result.exit_code = status.code();
                        result.exit_success = status.success();
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => {
                        if !result.termination_requested && Instant::now() >= graceful_deadline {
                            result.termination_requested = true;
                            let _ = child.kill();
                        }
                        thread::sleep(POLL_WAIT);
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
        result.stdout_joined = join_before(self.stdout.take(), deadline);
        result.stderr_joined = join_before(self.stderr.take(), deadline);
        result.writer_joined = join_before(self.writer.take(), deadline);
        result.stdout_eof = self.output.eof.load(Ordering::Acquire);
        result.stderr_eof = self.errors.eof.load(Ordering::Acquire);
        result.stderr_bytes = self.errors.bytes.load(Ordering::Acquire);
        result.output_clean = !self.output.failed.load(Ordering::Acquire)
            && !self.errors.failed.load(Ordering::Acquire)
            && self.lines.try_recv().is_err();
        result
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.finalize(false, Instant::now());
        }
    }
}

struct Assets {
    root: PathBuf,
    directories: Vec<(PathBuf, u32, u32)>,
    created: bool,
}

impl Assets {
    fn new(parent: &Path) -> Self {
        Self {
            root: parent.join("assets"),
            directories: Vec::new(),
            created: false,
        }
    }

    fn package(&mut self, cell_width: u32, cell_height: u32, deadline: Instant) -> Checked<String> {
        require_authority(deadline, Duration::ZERO)?;
        if let Some((directory, _, _)) = self
            .directories
            .iter()
            .find(|(_, width, height)| *width == cell_width && *height == cell_height)
        {
            let path = directory
                .to_str()
                .filter(|path| safe_payload(path))
                .ok_or("asset_path_invalid")?;
            return Ok(format!("PACKAGE {path}"));
        }
        if self.directories.len() == 8 {
            return Err("asset_limit");
        }
        if !self.created {
            require_authority(deadline, Duration::ZERO)?;
            private_directory(&self.root)?;
            self.created = true;
        }
        let directory = self.root.join(format!("marker-{}", self.directories.len()));
        require_authority(deadline, Duration::ZERO)?;
        private_directory(&directory)?;
        self.directories
            .push((directory.clone(), cell_width, cell_height));
        let width = cell_width.checked_mul(3).ok_or("asset_extent_invalid")?;
        let height = cell_height.checked_mul(2).ok_or("asset_extent_invalid")?;
        let capacity = (width as usize)
            .checked_mul(height as usize)
            .and_then(|value| value.checked_mul(3))
            .filter(|value| *value <= 4 * 1_024 * 1_024)
            .ok_or("asset_extent_invalid")?;
        let mut rgb = Vec::with_capacity(capacity);
        for y in 0..height {
            for x in 0..width {
                rgb.extend_from_slice(
                    if matches!((x / cell_width, y / cell_height), (1, 0) | (0, 1)) {
                        &MARKER_SECONDARY
                    } else {
                        &MARKER_PRIMARY
                    },
                );
            }
        }
        let encoded = png::encode_rgb(width, height, &rgb);
        let hash = native_watch_report::qualification_bytes_sha256(&encoded);
        require_authority(deadline, Duration::ZERO)?;
        write_new(&directory.join("marker.png"), &encoded, false)?;
        let manifest = format!(
            "{{\"schema_version\":1,\"package\":{{\"id\":\"madopilot.native.foreign-watch\",\"version\":\"1.0.0\"}},\"license\":\"Apache-2.0\",\"provenance\":{{\"created_by\":\"mado-pilot-testkit png\",\"created_for\":\"native foreign watch\"}},\"templates\":[{{\"id\":\"native.marker\",\"path\":\"marker.png\",\"width\":{width},\"height\":{height},\"coordinate_space\":\"capture_pixels\",\"content\":{{\"algorithm\":\"sha256\",\"value\":\"{hash}\"}},\"match_defaults\":{{\"min_score\":0.95,\"max_results\":1}}}}]}}\n"
        );
        require_authority(deadline, Duration::ZERO)?;
        write_new(
            &directory.join("madopilot-package.json"),
            manifest.as_bytes(),
            false,
        )?;
        let path = directory
            .to_str()
            .filter(|path| safe_payload(path))
            .ok_or("asset_path_invalid")?;
        Ok(format!("PACKAGE {path}"))
    }

    fn finish(&mut self) -> bool {
        let mut clean = true;
        for (directory, _, _) in self.directories.drain(..) {
            for name in ["marker.png", "madopilot-package.json"] {
                if let Err(error) = fs::remove_file(directory.join(name)) {
                    clean &= error.kind() == std::io::ErrorKind::NotFound;
                }
            }
            clean &= fs::remove_dir(directory).is_ok();
        }
        if self.created {
            clean &= fs::remove_dir(&self.root).is_ok();
            self.created = false;
        }
        clean
    }
}

struct FixtureBinding {
    last_ack: ControlAcknowledgement,
    last_token: u32,
    destroyed: bool,
    action_attempted: [bool; 3],
    action_acknowledged: [bool; 3],
    #[cfg(windows)]
    window: windows::Win32::Foundation::HWND,
}

impl FixtureBinding {
    fn bind(fixture: &mut NativeFixture) -> Checked<Self> {
        #[cfg(target_os = "macos")]
        fixture.controller.enable_bounded_command_writes();
        let mut result = Self {
            last_ack: ControlAcknowledgement {
                generation: fixture.generation,
                revision: fixture.revision,
                visual_token: None,
            },
            last_token: 0,
            destroyed: false,
            action_attempted: [false; 3],
            action_acknowledged: [false; 3],
            #[cfg(windows)]
            window: fixture.window().map_err(|_| "fixture_window_missing")?,
        };
        result.guard(fixture)?;
        Ok(result)
    }

    fn guard(&mut self, fixture: &mut NativeFixture) -> Checked<()> {
        if self.destroyed {
            return Err("fixture_already_destroyed");
        }
        #[cfg(target_os = "macos")]
        if fixture.controller.authenticated_process().is_none() {
            return Err("fixture_lifetime_lost");
        }
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
            let child = fixture.child.as_mut().ok_or("fixture_lifetime_lost")?;
            if child
                .try_wait()
                .map_err(|_| "fixture_lifetime_failed")?
                .is_some()
            {
                return Err("fixture_lifetime_lost");
            }
            let expected_pid = child.id();
            let window = fixture.window().map_err(|_| "fixture_window_missing")?;
            let mut pid = 0;
            // SAFETY: the retained child is still live, HWND validity is checked,
            // and the writable PID is call-local. No input/focus operation occurs.
            let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&raw mut pid)) };
            if window != self.window {
                return Err("fixture_owner_mismatch");
            }
            // SAFETY: IsWindow accepts a potentially stale HWND without dereferencing it.
            let window_valid = unsafe { IsWindow(Some(window)) }.as_bool();
            if !window_valid || thread_id == 0 || pid != expected_pid {
                return Err("fixture_owner_mismatch");
            }
        }
        Ok(())
    }

    fn title(fixture: &NativeFixture) -> String {
        #[cfg(target_os = "macos")]
        {
            super::protocol::fixture_title(fixture.process_id())
        }
        #[cfg(windows)]
        {
            fixture.title.clone()
        }
    }

    fn visual(
        &mut self,
        fixture: &mut NativeFixture,
        marker: VisualMarkerState,
        deadline: Instant,
    ) -> Checked<String> {
        self.guard(fixture)?;
        if deadline.saturating_duration_since(Instant::now()) < super::FIXTURE_COMMAND_WAIT {
            return Err("fixture_command_budget_exhausted");
        }
        let token = issue_fixture_visual_state(fixture, &mut self.last_ack, marker)
            .map_err(|_| "fixture_visual_failed")?;
        if token.value() <= self.last_token {
            return Err("fixture_token_reused");
        }
        self.last_token = token.value();
        let cells: String = token
            .encode()
            .into_iter()
            .map(|cell| if cell { '1' } else { '0' })
            .collect();
        Ok(format!(
            "ACK {} {} {cells}",
            token.value(),
            u8::from(marker.is_visible())
        ))
    }

    fn action(
        &mut self,
        fixture: &mut NativeFixture,
        action: &str,
        row: usize,
        deadline: Instant,
    ) -> Checked<String> {
        self.guard(fixture)?;
        let index = match (action, row) {
            ("RESIZE", 3) => 0,
            ("MOVE", 4) => 1,
            ("DESTROY", 7) => 2,
            _ => return Err("action_row_mismatch"),
        };
        if self.action_attempted[index] {
            return Err("fixture_action_repeated");
        }
        if deadline.saturating_duration_since(Instant::now()) < super::FIXTURE_COMMAND_WAIT {
            return Err("fixture_command_budget_exhausted");
        }
        self.action_attempted[index] = true;
        let ack = match action {
            "RESIZE" => fixture.resize_target(),
            "MOVE" => fixture.move_next_display(),
            "DESTROY" => {
                #[cfg(target_os = "macos")]
                {
                    // A closed-window filter may stay quiescent. Use the same
                    // authenticated process-loss stimulus as the Rust native row.
                    if !fixture.finish_before(deadline).is_accepted() {
                        return Err("fixture_action_failed");
                    }
                    self.action_acknowledged[index] = true;
                    self.destroyed = true;
                    return Ok("OK".into());
                }
                #[cfg(windows)]
                {
                    fixture.close_target()
                }
            }
            _ => return Err("unknown_action"),
        };
        let ack = match ack {
            Ok(ack) => ack,
            Err(reason) if action == "MOVE" && reason == "capability_unavailable:topology" => {
                return Ok("UNSUPPORTED".into());
            }
            Err(_) => return Err("fixture_action_failed"),
        };
        accept_control_acknowledgement(&mut self.last_ack, ack)
            .map_err(|_| "fixture_ack_invalid")?;
        self.action_acknowledged[index] = true;
        self.destroyed = action == "DESTROY";
        Ok("OK".into())
    }

    fn shape(&mut self, fixture: &mut NativeFixture, geometry: Geometry) -> Checked<String> {
        self.guard(fixture)?;
        let marker = self.cell(
            fixture,
            geometry,
            MARKER_X_LOGICAL,
            MARKER_Y_LOGICAL,
            MARKER_CELL_LOGICAL,
        )?;
        let token = self.cell(
            fixture,
            geometry,
            TOKEN_X_LOGICAL,
            TOKEN_Y_LOGICAL,
            TOKEN_CELL_LOGICAL,
        )?;
        for (cell, columns, rows) in [(marker, 3u32, 2u32), (token, 10, 9)] {
            let (width, height, x, y) = cell;
            let x = u32::try_from(x).map_err(|_| "shape_out_of_frame")?;
            let y = u32::try_from(y).map_err(|_| "shape_out_of_frame")?;
            if width == 0
                || height == 0
                || u64::from(width) > MAX_CELL
                || u64::from(height) > MAX_CELL
                || x.checked_add(width * columns)
                    .is_none_or(|right| right > geometry.width)
                || y.checked_add(height * rows)
                    .is_none_or(|bottom| bottom > geometry.height)
            {
                return Err("shape_out_of_frame");
            }
        }
        Ok(format!(
            "SHAPE {} {} {} {} {} {} {} {}",
            marker.0, marker.1, marker.2, marker.3, token.0, token.1, token.2, token.3
        ))
    }

    #[cfg(target_os = "macos")]
    fn cell(
        &self,
        _fixture: &NativeFixture,
        geometry: Geometry,
        x: f64,
        y: f64,
        size: f64,
    ) -> Checked<(u32, u32, i32, i32)> {
        let (content_width, content_height) =
            super::controlled_content_logical_size((geometry.logical[0], geometry.logical[1]))
                .ok_or("fixture_geometry_unknown")?;
        let horizontal = (geometry.logical[0] - content_width) / 2.0;
        let top = geometry.logical[1] - content_height;
        if horizontal < 0.0 || top < 0.0 {
            return Err("fixture_geometry_unknown");
        }
        Ok((
            scaled_u32(size, geometry.scale[0]).ok_or("shape_invalid")?,
            scaled_u32(size, geometry.scale[1]).ok_or("shape_invalid")?,
            scaled_i32(horizontal + x, geometry.scale[0]).ok_or("shape_invalid")?,
            scaled_i32(top + y, geometry.scale[1]).ok_or("shape_invalid")?,
        ))
    }

    #[cfg(windows)]
    fn cell(
        &self,
        _fixture: &NativeFixture,
        geometry: Geometry,
        x: f64,
        y: f64,
        size: f64,
    ) -> Checked<(u32, u32, i32, i32)> {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
        };
        let mut origin = POINT {
            x: scaled_i32(x, 1.0).ok_or("shape_invalid")?,
            y: scaled_i32(y, 1.0).ok_or("shape_invalid")?,
        };
        let size = scaled_i32(size, 1.0).ok_or("shape_invalid")?;
        let mut far = POINT {
            x: origin.x.checked_add(size).ok_or("shape_invalid")?,
            y: origin.y.checked_add(size).ok_or("shape_invalid")?,
        };
        // SAFETY: only this thread's coordinate virtualization is changed. The
        // approved PMv2 context and returned previous context are OS-owned values.
        let prior =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if prior.0.is_null() {
            return Err("dpi_context_failed");
        }
        // SAFETY: binding.guard checked this exact live owned HWND; both points
        // are writable call-local objects. Restore the context before any return.
        let converted = unsafe { ClientToScreen(self.window, &raw mut origin) }.as_bool();
        // SAFETY: same owned HWND and valid far point as above.
        let far_converted = unsafe { ClientToScreen(self.window, &raw mut far) }.as_bool();
        // SAFETY: prior is the context returned by the successful switch above.
        let restored = unsafe { SetThreadDpiAwarenessContext(prior) };
        if !converted || !far_converted || restored.0.is_null() {
            return Err("client_projection_failed");
        }
        Ok((
            u32::try_from(far.x.checked_sub(origin.x).ok_or("shape_invalid")?)
                .map_err(|_| "shape_invalid")?,
            u32::try_from(far.y.checked_sub(origin.y).ok_or("shape_invalid")?)
                .map_err(|_| "shape_invalid")?,
            scaled_i32(f64::from(origin.x) - geometry.desktop[0], 1.0).ok_or("shape_invalid")?,
            scaled_i32(f64::from(origin.y) - geometry.desktop[1], 1.0).ok_or("shape_invalid")?,
        ))
    }
}

#[cfg(target_os = "macos")]
type Baseline = crate::macos_fixture_control::FixtureCleanupCounts;
#[cfg(windows)]
type Baseline = u32;

#[cfg(target_os = "macos")]
fn resource_baseline() -> Checked<Baseline> {
    crate::macos_fixture_control::fixture_cleanup_counts()
        .map_err(|_| "resource_baseline_unavailable")
}

#[cfg(windows)]
fn resource_baseline() -> Checked<Baseline> {
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
    let mut count = 0;
    // SAFETY: the pseudo-handle names this process, and count is writable locally.
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &raw mut count) }
        .map_err(|_| "resource_baseline_unavailable")?;
    Ok(count)
}

fn baseline_matches(before: Baseline, after: Baseline) -> bool {
    #[cfg(target_os = "macos")]
    {
        before.active == 0
            && after.active == before.active
            && after.exhausted == before.exhausted
            && after.scheduled.checked_sub(before.scheduled)
                == after.completed.checked_sub(before.completed)
            && after.scheduled >= before.scheduled
            && after.completed >= before.completed
    }
    #[cfg(windows)]
    {
        before == after
    }
}

#[derive(Default)]
struct Finalization {
    consumer: ConsumerFinalization,
    fixture: Option<NativeResourceFacts>,
    fixture_accepted: bool,
    baseline_observed: bool,
    baseline_restored: bool,
    scratch_removed: bool,
    artifacts_unchanged: bool,
    baseline_before: Option<Baseline>,
    baseline_after: Option<Baseline>,
    baseline_before_matches_reference: bool,
}

impl Finalization {
    fn accepted(&self, role: CycleRole) -> bool {
        self.consumer.accepted()
            && self.fixture_accepted
            && self.baseline_observed
            && self.baseline_after.is_some()
            && self.scratch_removed
            && self.artifacts_unchanged
            && (!role.baseline_enforced()
                || (self.baseline_before_matches_reference && self.baseline_restored))
    }
}

#[derive(Default)]
struct CohortFinalization {
    fixed_baseline: Option<Baseline>,
    final_baseline: Option<Baseline>,
    baseline_restored: bool,
    artifacts_unchanged: bool,
}

fn request_deadline(
    previous_reply: Option<Instant>,
    received: Instant,
    now: Instant,
) -> Checked<Instant> {
    if previous_reply.is_some_and(|reply| received < reply) {
        return Err("unsolicited_record");
    }
    let deadline = received + REPLY_WAIT;
    if now >= deadline {
        return Err("controller_reply_timeout");
    }
    Ok(deadline)
}

fn run_protocol(
    consumer: &Consumer,
    fixture: &mut NativeFixture,
    binding: &mut FixtureBinding,
    assets: &mut Assets,
    ledger: &mut Ledger,
    library: &Artifact,
    cohort_deadline: Instant,
) -> Checked<()> {
    let mut row_deadline = Instant::now() + ROW_WAIT;
    let mut previous_reply: Option<Instant> = None;
    for _ in 0..RECORD_LIMIT {
        // Consumer work between requests uses the row/process containment bound.
        // The first byte starts a separate fixed request/reply deadline.
        require_authority(cohort_deadline, Duration::ZERO)?;
        let request = consumer.read(cohort_deadline.min(row_deadline))?;
        let deadline = request_deadline(previous_reply, request.at, Instant::now())?
            .min(cohort_deadline)
            .min(row_deadline);
        if Instant::now() >= deadline {
            return Err("controller_reply_timeout");
        }
        let parsed = parse_request(&request.line)?;
        if !ledger.loaded && !matches!(parsed, Request::Loaded(_)) {
            return Err("loaded_library_missing");
        }
        if matches!(
            parsed,
            Request::Visual(_) | Request::Resize | Request::Move | Request::Destroy
        ) && deadline.saturating_duration_since(Instant::now()) < super::FIXTURE_COMMAND_WAIT
        {
            return Err("fixture_command_budget_exhausted");
        }
        let reply = match parsed {
            Request::Loaded(path) => {
                if ledger.loaded || ledger.next != 0 {
                    return Err("loaded_library_repeated");
                }
                let path = Path::new(path);
                if path.canonicalize().ok().as_ref() != Some(&library.canonical)
                    || identity(path, deadline).ok().as_ref() != Some(&library.before)
                {
                    return Err("loaded_library_mismatch");
                }
                ledger.loaded = true;
                "OK".to_owned()
            }
            Request::Visual(marker) => binding.visual(fixture, marker, deadline)?,
            Request::Geometry(geometry) => binding.shape(fixture, geometry)?,
            Request::Asset(width, height) => assets.package(width, height, deadline)?,
            Request::Resize => binding.action(fixture, "RESIZE", ledger.next, deadline)?,
            Request::Move => binding.action(fixture, "MOVE", ledger.next, deadline)?,
            Request::Destroy => binding.action(fixture, "DESTROY", ledger.next, deadline)?,
            Request::Fact(row, fact) => {
                ledger.fact(row, fact)?;
                "OK".to_owned()
            }
            Request::Row(row, outcome, reason) => {
                let control = match row {
                    3 => Some(0),
                    4 => Some(1),
                    7 => Some(2),
                    _ => None,
                };
                if outcome == Outcome::Pass
                    && control.is_some_and(|index| !binding.action_acknowledged[index])
                {
                    return Err("required_control_not_acknowledged");
                }
                ledger.row(row, outcome, reason)?;
                row_deadline = Instant::now() + ROW_WAIT;
                "OK".to_owned()
            }
            Request::Done => {
                ledger.finish()?;
                "OK".to_owned()
            }
        };
        previous_reply = Some(consumer.reply(reply, deadline)?);
        if ledger.done {
            return Ok(());
        }
    }
    Err("record_limit")
}

fn finalize_artifacts(artifacts: &mut [Artifact], deadline: Instant) -> bool {
    let mut unchanged = artifacts.len() == 4;
    for artifact in artifacts {
        unchanged &= artifact.finalize(deadline);
    }
    unchanged
}

fn record_expiration(errors: &mut Vec<Failure>, deadline: Instant) {
    if let Err(error) = require_authority(deadline, Duration::ZERO)
        && !errors.contains(&error)
    {
        errors.push(error);
    }
}

fn execute_cycle(
    arguments: &ForeignArguments,
    artifacts: &mut [Artifact],
    index: usize,
    fixed_baseline: Option<Baseline>,
    cohort_deadline: Instant,
    admission: &ResourceAdmission<'_>,
) -> CycleSummary {
    let role = CycleRole::at(index);
    let directory = arguments.directory.join(CYCLE_DIRECTORIES[index]);
    let mut summary = CycleSummary {
        aggregate: Outcome::Infra,
        eligible: false,
        before_written: false,
        report_written: false,
        output_error: None,
        resource: None,
    };
    if let Err(error) = private_directory(&directory) {
        summary.output_error = Some(error);
        return summary;
    }
    let mut ledger = Ledger::new();
    let mut finalization = Finalization::default();
    let mut errors = Vec::new();
    let mut assets = Assets::new(&directory);
    match resource_baseline() {
        Ok(before) => {
            finalization.baseline_before = Some(before);
            finalization.baseline_observed = true;
            if role.baseline_enforced() {
                finalization.baseline_before_matches_reference =
                    fixed_baseline.is_some_and(|fixed| baseline_matches(fixed, before));
                if !finalization.baseline_before_matches_reference {
                    errors.push("resource_baseline_before_mismatch");
                }
            }
        }
        Err(error) => errors.push(error),
    }
    let resource_before =
        ResourceDecision::evaluate(admission, &ledger, Phase::Before, !errors.is_empty());
    match write_new(
        &directory.join("before.json"),
        report(
            &ledger,
            artifacts,
            &finalization,
            &errors,
            Phase::Before,
            CycleReportContext {
                index,
                fixed_baseline,
                admission,
                resource: &resource_before,
            },
        )
        .as_bytes(),
        true,
    ) {
        Ok(()) => summary.before_written = true,
        Err(error) => {
            summary.output_error = Some(error);
            errors.push(error);
        }
    }
    if errors.is_empty() {
        let fixture_arguments = Arguments {
            fixture_executable: arguments.fixture.clone(),
            raw: vec![format!("--fixture-sha256={}", artifacts[2].before.sha256)],
            qualification: true,
            full_load_diagnostic: false,
            retained_result_lifecycle_diagnostic: false,
            enforce_budgets: false,
            native_contract: false,
            root_cause_stress: false,
            workload_filter: None,
        };
        // Reserve the existing launch allowance and propagate the same authority
        // through identity, launch and handshake. Filesystem/native calls are not
        // preemptible; an overrun permits only bounded teardown.
        let started = require_authority(cohort_deadline, super::FIXTURE_WAIT).and_then(|()| {
            NativeFixture::start(&fixture_arguments, Some(cohort_deadline))
                .map_err(|_| "fixture_start_failed")
        });
        match started {
            Err(error) => errors.push(error),
            Ok(mut fixture) => {
                let protocol = (|| {
                    require_authority(cohort_deadline, Duration::ZERO)?;
                    let mut binding = FixtureBinding::bind(&mut fixture)?;
                    require_authority(cohort_deadline, Duration::ZERO)?;
                    let title = FixtureBinding::title(&fixture);
                    let mut consumer = Consumer::spawn(arguments, &title, cohort_deadline)?;
                    let result = run_protocol(
                        &consumer,
                        &mut fixture,
                        &mut binding,
                        &mut assets,
                        &mut ledger,
                        &artifacts[1],
                        cohort_deadline,
                    );
                    finalization.consumer = consumer.finalize(result.is_ok(), cohort_deadline);
                    result
                })();
                if let Err(error) = protocol {
                    errors.push(error);
                }
                if !finalization.consumer.accepted() {
                    errors.push("consumer_finalization_failed");
                }
                #[cfg(target_os = "macos")]
                let finished = {
                    let now = Instant::now();
                    let deadline = if now >= cohort_deadline {
                        now + LIFECYCLE_WAIT
                    } else {
                        (now + LIFECYCLE_WAIT).min(cohort_deadline)
                    };
                    fixture.finish_before(deadline)
                };
                #[cfg(windows)]
                let finished = fixture.finish();
                finalization.fixture_accepted = finished.is_accepted();
                finalization.fixture = Some(finished.resources());
                if !finalization.fixture_accepted {
                    errors.push("fixture_finalization_failed");
                }
                drop(fixture);
            }
        }
    }
    finalization.scratch_removed = assets.finish();
    if !finalization.scratch_removed {
        errors.push("scratch_cleanup_failed");
    }
    finalization.artifacts_unchanged = finalize_artifacts(artifacts, cohort_deadline);
    if !finalization.artifacts_unchanged {
        errors.push("artifact_changed");
    }
    // The consumer, fixture, pipe/thread owners and scratch files have all been
    // finalized and dropped. The same four pinned File owners remain open.
    match resource_baseline() {
        Ok(after) => finalization.baseline_after = Some(after),
        Err(error) => {
            finalization.baseline_observed = false;
            errors.push(error);
        }
    }
    let reference = if role.baseline_enforced() {
        fixed_baseline
    } else {
        finalization.baseline_before
    };
    finalization.baseline_restored = reference
        .zip(finalization.baseline_after)
        .is_some_and(|(before, after)| baseline_matches(before, after));
    if role.baseline_enforced() && !finalization.baseline_restored {
        errors.push("resource_baseline_not_restored");
    }
    record_expiration(&mut errors, cohort_deadline);
    let infra = !errors.is_empty() || !finalization.accepted(role);
    let resource = ResourceDecision::evaluate(admission, &ledger, Phase::Final, infra);
    // Evidence persistence is not a new qualification operation. Expired cycles
    // still retain their complete ledger and bounded-finalization observations.
    summary.persist_report(
        &directory.join("report.json"),
        &report(
            &ledger,
            artifacts,
            &finalization,
            &errors,
            Phase::Final,
            CycleReportContext {
                index,
                fixed_baseline,
                admission,
                resource: &resource,
            },
        ),
        resource,
    );
    summary
}

fn execute(arguments: ForeignArguments) -> Checked<i32> {
    // This is the only creation of the cohort's qualification authority.
    let cohort_deadline = Instant::now() + RUN_WAIT;
    private_directory(&arguments.directory)?;
    let mut artifacts = Vec::with_capacity(4);
    let mut errors = Vec::new();
    let runner = std::env::current_exe().map_err(|_| "runner_identity_unavailable")?;
    for (role, path) in [
        ("consumer", arguments.consumer.clone()),
        ("library", arguments.library.clone()),
        ("fixture", arguments.fixture.clone()),
        ("runner", runner),
    ] {
        match Artifact::pin(role, path, cohort_deadline) {
            Ok(artifact) => artifacts.push(artifact),
            Err(error) => {
                errors.push(error);
                break;
            }
        }
    }
    let admission = ResourceAdmission::select(
        arguments.resources.as_ref(),
        mado_pilot_testkit::bench_harness::RELEASE_TARGET,
        !cfg!(debug_assertions),
        ResourceArtifacts::from_artifacts(&artifacts),
    );
    if let Some(error) = admission.error {
        errors.push(error);
    }
    let mut cycles = std::array::from_fn(|_| None);
    write_new(
        &arguments.directory.join("before.json"),
        cohort_report(
            &cycles,
            &artifacts,
            &CohortFinalization::default(),
            &errors,
            Phase::Before,
            &admission,
        )
        .as_bytes(),
        true,
    )?;
    record_expiration(&mut errors, cohort_deadline);
    if cycle_may_launch(&cycles, 0, !errors.is_empty()) {
        cycles[0] = Some(execute_cycle(
            &arguments,
            &mut artifacts,
            0,
            None,
            cohort_deadline,
            &admission,
        ));
    }
    record_expiration(&mut errors, cohort_deadline);
    // Establish exactly once, after eligible warmup, report persistence and the
    // complete cycle-local ownership scope. Measurements cannot rebase it.
    let fixed_baseline = if cycle_may_launch(&cycles, 1, !errors.is_empty()) {
        match resource_baseline() {
            Ok(baseline) if baseline_matches(baseline, baseline) => Some(baseline),
            Ok(_) => {
                errors.push("postwarmup_baseline_not_clean");
                None
            }
            Err(error) => {
                errors.push(error);
                None
            }
        }
    } else {
        None
    };
    for index in WARMUP_COUNT..CYCLE_COUNT {
        record_expiration(&mut errors, cohort_deadline);
        if !cycle_may_launch(&cycles, index, !errors.is_empty()) {
            break;
        }
        cycles[index] = Some(execute_cycle(
            &arguments,
            &mut artifacts,
            index,
            fixed_baseline,
            cohort_deadline,
            &admission,
        ));
    }
    let artifacts_unchanged = finalize_artifacts(&mut artifacts, cohort_deadline);
    if !artifacts_unchanged {
        errors.push("artifact_changed");
    }
    let final_baseline = match resource_baseline() {
        Ok(baseline) => Some(baseline),
        Err(error) => {
            errors.push(error);
            None
        }
    };
    let baseline_restored = fixed_baseline
        .zip(final_baseline)
        .is_some_and(|(fixed, final_count)| baseline_matches(fixed, final_count));
    if fixed_baseline.is_some() && !baseline_restored {
        errors.push("resource_baseline_not_restored");
    }
    record_expiration(&mut errors, cohort_deadline);
    let finalization = CohortFinalization {
        fixed_baseline,
        final_baseline,
        baseline_restored,
        artifacts_unchanged,
    };
    let outcome = cohort_aggregate(&cycles, Phase::Final, !errors.is_empty());
    write_new(
        &arguments.directory.join("report.json"),
        cohort_report(
            &cycles,
            &artifacts,
            &finalization,
            &errors,
            Phase::Final,
            &admission,
        )
        .as_bytes(),
        true,
    )?;
    println!("native_foreign_watch:{}", outcome.name());
    Ok(outcome.exit_code())
}

fn option_json<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn baseline_json(value: Option<Baseline>) -> String {
    let Some(value) = value else {
        return "null".to_owned();
    };
    #[cfg(target_os = "macos")]
    {
        format!(
            "{{\"scheduled\":{},\"active\":{},\"completed\":{},\"exhausted\":{}}}",
            value.scheduled, value.active, value.completed, value.exhausted
        )
    }
    #[cfg(windows)]
    {
        format!("{{\"controller_process_handles\":{value}}}")
    }
}

fn baseline_predicate() -> &'static str {
    if cfg!(windows) {
        "controller_process_handle_equality"
    } else {
        "active_zero_exhausted_unchanged_balanced_cumulative_cleanup"
    }
}

fn append_report_policy(text: &mut String, admission: &ResourceAdmission<'_>) {
    admission.append_json(text);
    let _ = write!(
        text,
        ",\"exit_semantics\":{{\"pass\":0,\"nonpass\":1,\"infra\":2}},\"limits\":{{\"line_bytes\":{LINE_LIMIT},\"queue_events\":{QUEUE_LIMIT},\"records_per_consumer\":{RECORD_LIMIT},\"facts_per_row\":{FACTS_PER_ROW},\"stderr_bytes\":{STDERR_LIMIT},\"reply_ms\":5000,\"partial_line_ms\":5000,\"fixture_ack_ms\":2000,\"fixture_start_ms\":10000,\"lifecycle_ms\":10000,\"row_ms\":120000,\"cohort_ms\":600000,\"authority\":\"one_absolute_monotonic_deadline\",\"filesystem_native_call_preemption\":false,\"bounded_cleanup_after_expiry\":true}}"
    );
}

fn append_errors(text: &mut String, errors: &[Failure]) {
    text.push_str(",\"errors\":[");
    for (index, error) in errors.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        let _ = write!(text, "\"{error}\"");
    }
    text.push(']');
}

fn append_artifacts(text: &mut String, artifacts: &[Artifact]) {
    text.push_str(",\"artifacts\":[");
    for (index, artifact) in artifacts.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        let _ = write!(
            text,
            "{{\"role\":\"{}\",\"before\":{{\"sha256\":\"{}\",\"size\":{}}},\"after\":",
            artifact.role, artifact.before.sha256, artifact.before.size
        );
        if let Some(after) = &artifact.after {
            let _ = write!(
                text,
                "{{\"sha256\":\"{}\",\"size\":{}}}",
                after.sha256, after.size
            );
        } else {
            text.push_str("null");
        }
        let _ = write!(text, ",\"unchanged\":{}}}", artifact.unchanged);
    }
    text.push(']');
}

fn cohort_report(
    cycles: &[Option<CycleSummary>; CYCLE_COUNT],
    artifacts: &[Artifact],
    finalization: &CohortFinalization,
    errors: &[Failure],
    phase: Phase,
    admission: &ResourceAdmission<'_>,
) -> String {
    let infra = !errors.is_empty()
        || (phase == Phase::Final
            && (!finalization.artifacts_unchanged
                || (finalization.fixed_baseline.is_some() && !finalization.baseline_restored)));
    let outcome = cohort_aggregate(cycles, phase, infra);
    let mut text = format!(
        "{{\"schema\":\"madopilot.native-foreign-watch.cohort.v3\",\"phase\":\"{}\",\"target\":\"{}\",\"protocol\":\"native-foreign-watch.v1\",\"warmup_count\":{WARMUP_COUNT},\"measurement_count\":{MEASUREMENT_COUNT},\"aggregate\":\"{}\",\"exit_code\":{},\"controller_resource_scope\":{{\"warmup\":\"initialization_observation\",\"measurement\":\"steady_state\",\"baseline_reference\":\"fixed_postwarmup\",\"measurement_baseline_enforced\":true,\"measurement_equality_enforced\":{},\"comparison_predicate\":\"{}\",\"cold_leak_free_qualified\":false,\"artifact_owners\":\"same_four_pins_held_through_final_baseline\"}},\"fixed_baseline\":{},\"final_baseline\":{},\"finalization\":{{\"baseline_observed\":{},\"baseline_restored\":{},\"artifacts_unchanged\":{}}}",
        phase.name(),
        mado_pilot_testkit::bench_harness::RELEASE_TARGET,
        outcome.name(),
        option_json((phase == Phase::Final).then_some(outcome.exit_code())),
        cfg!(windows),
        baseline_predicate(),
        baseline_json(finalization.fixed_baseline),
        baseline_json(finalization.final_baseline),
        finalization.final_baseline.is_some(),
        finalization.baseline_restored,
        finalization.artifacts_unchanged,
    );
    append_report_policy(&mut text, admission);
    append_errors(&mut text, errors);
    append_artifacts(&mut text, artifacts);
    text.push_str(",\"cycles\":[");
    for (index, cycle) in cycles.iter().enumerate() {
        let cycle = cycle.as_ref();
        if index > 0 {
            text.push(',');
        }
        let directory = CYCLE_DIRECTORIES[index];
        let _ = write!(
            text,
            "{{\"index\":{index},\"role\":\"{}\",\"directory\":\"{directory}\",\"before\":\"{directory}/before.json\",\"before_written\":{},\"report\":",
            CycleRole::at(index).name(),
            cycle.is_some_and(|cycle| cycle.before_written),
        );
        if cycle.is_some_and(|cycle| cycle.report_written) {
            let _ = write!(text, "\"{directory}/report.json\"");
        } else {
            text.push_str("null");
        }
        let _ = write!(
            text,
            ",\"launched\":{},\"aggregate\":\"{}\",\"eligible\":{},\"reason\":",
            cycle.is_some(),
            cycle
                .map_or(Outcome::Unexecuted, |cycle| cycle.aggregate)
                .name(),
            cycle.is_some_and(|cycle| cycle.eligible),
        );
        let reason = match cycle {
            Some(cycle) => cycle.output_error.or_else(|| {
                cycle.resource.as_ref().and_then(|resource| {
                    (resource.outcome() != Outcome::Pass && cycle.aggregate == resource.outcome())
                        .then_some(resource.reason())
                })
            }),
            None if phase == Phase::Before => Some("cohort_not_started"),
            None => Some("prior_cycle_failed"),
        };
        if let Some(reason) = reason {
            let _ = write!(text, "\"{reason}\"");
        } else {
            text.push_str("null");
        }
        text.push_str(",\"resource_decision\":");
        if let Some(resource) = cycle.and_then(|cycle| cycle.resource.as_ref()) {
            resource.append_json(&mut text);
        } else {
            text.push_str("null");
        }
        text.push('}');
    }
    text.push_str("]}\n");
    text
}

struct CycleReportContext<'a> {
    index: usize,
    fixed_baseline: Option<Baseline>,
    admission: &'a ResourceAdmission<'a>,
    resource: &'a ResourceDecision,
}

fn report(
    ledger: &Ledger,
    artifacts: &[Artifact],
    finalization: &Finalization,
    errors: &[Failure],
    phase: Phase,
    context: CycleReportContext<'_>,
) -> String {
    let CycleReportContext {
        index,
        fixed_baseline,
        admission,
        resource,
    } = context;
    let role = CycleRole::at(index);
    let outcome = resource.aggregate();
    let mut text = format!(
        "{{\"schema\":\"madopilot.native-foreign-watch.cycle.v3\",\"phase\":\"{}\",\"target\":\"{}\",\"protocol\":\"native-foreign-watch.v1\",\"index\":{index},\"role\":\"{}\",\"aggregate\":\"{}\",\"exit_code\":{},\"eligible\":{},\"ledger_complete\":{},\"loaded_library_verified\":{},\"resource_observation\":{{\"scope\":\"{}\",\"baseline_reference\":\"{}\",\"baseline_enforced\":{},\"equality_enforced\":{},\"comparison_predicate\":\"{}\",\"fixed_baseline\":{},\"cold_leak_free_qualified\":false}}",
        phase.name(),
        mado_pilot_testkit::bench_harness::RELEASE_TARGET,
        role.name(),
        outcome.name(),
        option_json((phase == Phase::Final).then_some(outcome.exit_code())),
        resource.eligible(),
        ledger.done,
        ledger.loaded,
        role.scope(),
        role.baseline_reference(),
        role.baseline_enforced(),
        role.baseline_enforced() && cfg!(windows),
        baseline_predicate(),
        baseline_json(fixed_baseline),
    );
    append_report_policy(&mut text, admission);
    text.push_str(",\"resource_decision\":");
    resource.append_json(&mut text);
    append_errors(&mut text, errors);
    append_artifacts(&mut text, artifacts);
    text.push_str(",\"rows\":[");
    for (index, row) in ledger.rows.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        let consumer = row
            .outcome
            .map_or_else(|| "null".into(), |value| format!("\"{}\"", value.name()));
        let effective = resource.effective(index);
        let reason = row
            .reason
            .as_ref()
            .map_or_else(|| "null".into(), |value| format!("\"{value}\""));
        let controller_reason = if index == 8 && resource.effective(8) == resource.outcome() {
            format!("\"{}\"", resource.reason())
        } else {
            "null".to_owned()
        };
        let _ = write!(
            text,
            "{{\"id\":\"F{}\",\"consumer_outcome\":{consumer},\"effective_outcome\":\"{}\",\"reason\":{reason},\"controller_reason\":{controller_reason},\"facts\":[",
            index + 1,
            effective.name()
        );
        for (fact_index, fact) in row.facts.iter().enumerate() {
            if fact_index > 0 {
                text.push(',');
            }
            let _ = write!(text, "{{\"key\":\"{}\",\"values\":[", fact.key.name());
            for (value_index, value) in fact.values.iter().enumerate() {
                if value_index > 0 {
                    text.push(',');
                }
                match value {
                    Number::Unsigned(value) => {
                        let _ = write!(text, "{value}");
                    }
                    Number::Real(value) => {
                        let _ = write!(text, "{value}");
                    }
                }
            }
            text.push_str("]}");
        }
        text.push_str("]}");
    }
    let consumer = &finalization.consumer;
    let _ = write!(
        text,
        "],\"finalization\":{{\"consumer\":{{\"reaped\":{},\"exit_code\":{},\"exit_success\":{},\"termination_requested\":{},\"stdout_joined\":{},\"stderr_joined\":{},\"writer_joined\":{},\"stdout_eof\":{},\"stderr_eof\":{},\"output_clean\":{},\"stderr_bytes\":{}}},\"fixture_accepted\":{},\"baseline_observed\":{},\"baseline_before_matches_reference\":{},\"baseline_restored\":{},\"scratch_removed\":{},\"artifacts_unchanged\":{},\"fixture_resources\":",
        consumer.reaped,
        option_json(consumer.exit_code),
        consumer.exit_success,
        consumer.termination_requested,
        consumer.stdout_joined,
        consumer.stderr_joined,
        consumer.writer_joined,
        consumer.stdout_eof,
        consumer.stderr_eof,
        consumer.output_clean,
        consumer.stderr_bytes,
        finalization.fixture_accepted,
        finalization.baseline_observed,
        finalization.baseline_before_matches_reference,
        finalization.baseline_restored,
        finalization.scratch_removed,
        finalization.artifacts_unchanged,
    );
    if let Some(facts) = finalization.fixture {
        let _ = write!(
            text,
            "{{\"baseline_observed\":{},\"process_reaped\":{},\"reader_joined\":{},\"stop_acknowledged\":{},\"bounded_containment\":{},\"output_drained\":{},\"executable_unchanged\":{},\"cleanup_scheduled\":{},\"cleanup_active\":{},\"cleanup_completed\":{},\"cleanup_exhausted\":{}}}",
            facts.baseline_observed,
            facts.fixture_process_reaped,
            facts.fixture_reader_joined,
            option_json(facts.protocol_stop_acknowledged),
            facts.bounded_containment,
            facts.output_drained,
            option_json(facts.executable_identity_unchanged),
            option_json(facts.apple_cleanup_scheduled),
            option_json(facts.apple_cleanup_active),
            option_json(facts.apple_cleanup_completed),
            option_json(facts.apple_cleanup_exhausted),
        );
    } else {
        text.push_str("null");
    }
    let _ = write!(
        text,
        ",\"fixture_baseline\":{{\"before\":{},\"after\":{}}}",
        baseline_json(finalization.baseline_before),
        baseline_json(finalization.baseline_after)
    );
    text.push_str("}}\n");
    text
}

#[cfg(test)]
// Cargo checks harness=false benches with cfg(test), without collecting test entry points.
#[allow(dead_code)]
mod tests {
    use super::*;

    fn resource_arguments(profile: &resources::ResourceProfile) -> Vec<OsString> {
        #[cfg(unix)]
        let root = "/owned";
        #[cfg(windows)]
        let root = "C:\\owned";
        let mut raw: Vec<_> = [
            "--foreign-consumer",
            "--fixture-executable",
            "--foreign-artifact-dir",
            "--foreign-library",
        ]
        .map(|key| OsString::from(format!("{key}={root}")))
        .into_iter()
        .collect();
        let values = [
            profile.id.to_owned(),
            profile.hardware.to_owned(),
            profile.os_version.to_owned(),
            profile.topology.to_owned(),
            "1".repeat(40),
            "2".repeat(40),
            "3".repeat(64),
            "4".repeat(64),
        ];
        raw.extend(
            ResourceRequest::KEYS
                .into_iter()
                .zip(values)
                .map(|(key, value)| OsString::from(format!("{key}={value}"))),
        );
        raw
    }

    fn resource_request(profile: &resources::ResourceProfile) -> ResourceRequest {
        ForeignArguments::parse(&resource_arguments(profile))
            .unwrap()
            .resources
            .unwrap()
    }

    fn approved_artifacts<'a>(
        profile: &'a resources::ResourceProfile,
        request: &'a ResourceRequest,
        cpp: bool,
    ) -> ResourceArtifacts<'a> {
        ResourceArtifacts {
            consumer: profile.consumer_sha256[usize::from(cpp)],
            library: profile.library_sha256,
            fixture: profile.fixture_sha256,
            runner: &request.runner_sha256,
        }
    }

    fn unselected_resources() -> ResourceAdmission<'static> {
        ResourceAdmission::select(
            None,
            mado_pilot_testkit::bench_harness::RELEASE_TARGET,
            !cfg!(debug_assertions),
            None,
        )
    }

    fn append_fact(ledger: &mut Ledger, row: usize, key: FactKey, values: &str) {
        ledger
            .fact(
                row,
                Fact::parse(key, &values.split(' ').collect::<Vec<_>>()).unwrap(),
            )
            .unwrap();
    }

    fn append_bootstrap(ledger: &mut Ledger, row: usize, geometry: &resources::ProfileGeometry) {
        let [width, height] = geometry.frame;
        append_fact(
            ledger,
            row,
            FactKey::Bootstrap,
            &format!("1 0 0 0 {width} {height}"),
        );
    }

    fn append_geometry(
        ledger: &mut Ledger,
        row: usize,
        profile: &resources::ResourceProfile,
        stage: usize,
    ) {
        let geometry = &profile.geometry[stage];
        let [width, height] = geometry.frame;
        append_fact(
            ledger,
            row,
            FactKey::Frame,
            &format!("1 {stage} 1 {stage} {width} {height}"),
        );
        let [width, height, x, y] = geometry.transform;
        append_fact(
            ledger,
            row,
            FactKey::Transform,
            &format!("{stage} {stage} {stage} {width} {height} {x} {y} 1"),
        );
    }

    fn resource_ledger(profile: &resources::ResourceProfile, samples: [[u64; 3]; 5]) -> Ledger {
        let mut ledger = Ledger::new();
        ledger.loaded = true;
        for (row, reason) in PASS_REASONS.iter().enumerate() {
            match row {
                0 => {
                    append_fact(
                        &mut ledger,
                        row,
                        FactKey::Permissions,
                        if cfg!(windows) { "3 3" } else { "1 3" },
                    );
                    append_fact(&mut ledger, row, FactKey::Status, "0 0");
                    for (index, [first, resident, count]) in samples[..4].iter().enumerate() {
                        append_bootstrap(&mut ledger, row, &profile.geometry[0]);
                        append_fact(
                            &mut ledger,
                            row,
                            FactKey::Resource,
                            &format!(
                                "{} {index} {first} {resident} {count}",
                                u8::from(index != 0)
                            ),
                        );
                    }
                }
                1 => {
                    append_bootstrap(&mut ledger, row, &profile.geometry[0]);
                    append_geometry(&mut ledger, row, profile, 0);
                    append_fact(&mut ledger, row, FactKey::Pending, "1 0 0");
                    append_fact(&mut ledger, row, FactKey::Startup, "1");
                }
                2 => append_geometry(&mut ledger, row, profile, 0),
                3 | 4 => {
                    append_geometry(&mut ledger, row, profile, row - 3);
                    append_fact(&mut ledger, row, FactKey::Pending, "1 0 0");
                    append_bootstrap(&mut ledger, row, &profile.geometry[row - 2]);
                    append_geometry(&mut ledger, row, profile, row - 2);
                    append_geometry(&mut ledger, row, profile, row - 2);
                }
                5 => {
                    for status in ["3 0", "4 0", "0 2", "0 3"] {
                        append_fact(&mut ledger, row, FactKey::Status, status);
                    }
                    append_fact(&mut ledger, row, FactKey::Pending, "1 0 0");
                }
                6 => {
                    append_bootstrap(&mut ledger, row, &profile.geometry[2]);
                    append_geometry(&mut ledger, row, profile, 0);
                    append_geometry(&mut ledger, row, profile, 2);
                }
                7 => {
                    for _ in 0..2 {
                        append_bootstrap(&mut ledger, row, &profile.geometry[2]);
                    }
                    for status in ["0 4", "0 5", "0 6"] {
                        append_fact(&mut ledger, row, FactKey::Status, status);
                    }
                }
                8 => {
                    let [first, resident, count] = samples[4];
                    append_fact(
                        &mut ledger,
                        row,
                        FactKey::Resource,
                        &format!("2 4 {first} {resident} {count}"),
                    );
                    append_fact(&mut ledger, row, FactKey::Status, "0 0");
                }
                _ => unreachable!(),
            }
            if matches!(row, 2 | 3 | 4 | 6) {
                append_fact(&mut ledger, row, FactKey::Match, "1 1 1 1 0 0 0 1 1");
                append_fact(&mut ledger, row, FactKey::Mapped, "4");
            }
            ledger.row(row, Outcome::Pass, reason).unwrap();
        }
        ledger.finish().unwrap();
        ledger
    }

    fn cycle_summary(
        ledger: &Ledger,
        admission: &ResourceAdmission<'_>,
        infra: bool,
    ) -> CycleSummary {
        let resource = ResourceDecision::evaluate(admission, ledger, Phase::Final, infra);
        CycleSummary {
            aggregate: resource.aggregate(),
            eligible: resource.eligible(),
            before_written: true,
            report_written: true,
            output_error: None,
            resource: Some(resource),
        }
    }

    #[test]
    fn cycle_report_persistence_failure_invalidates_the_effective_resource_decision() {
        let profile = &resources::PROFILES[0];
        let request = resource_request(profile);
        let admission = ResourceAdmission::select(
            Some(&request),
            profile.target,
            true,
            Some(approved_artifacts(profile, &request, false)),
        );
        let ledger = resource_ledger(profile, [[1, 1, 1]; 5]);
        let resource = ResourceDecision::evaluate(&admission, &ledger, Phase::Final, false);
        let mut text = String::new();
        resource.append_json(&mut text);
        let mut cycle = CycleSummary {
            aggregate: Outcome::Unexecuted,
            eligible: false,
            before_written: true,
            report_written: false,
            output_error: None,
            resource: None,
        };
        // A directory cannot become a report file. Exercise real create_new failure
        // without changing any existing file or relying on a permissions race.
        cycle.persist_report(&std::env::current_dir().unwrap(), &text, resource);
        let mut cycles = std::array::from_fn(|_| None);
        cycles[0] = Some(cycle);
        assert!(!cycle_may_launch(&cycles, 1, false));
        assert_eq!(
            cohort_aggregate(&cycles, Phase::Final, false).exit_code(),
            2
        );
        let cycle = cycles[0].as_ref().unwrap();
        assert_eq!(cycle.aggregate, Outcome::Infra);
        assert!(!cycle.eligible);
        assert_eq!(cycle.output_error, Some("output_create_failed"));
        assert!(!cycle.report_written);
        let resource = cycle.resource.as_ref().unwrap();
        assert_eq!(resource.outcome(), Outcome::Pass);
        assert_eq!(resource.effective(8), Outcome::Infra);
        assert_eq!(resource.aggregate(), Outcome::Infra);
        assert!(!resource.eligible());
    }

    #[test]
    fn successful_final_report_preserves_an_earlier_output_failure() {
        struct ReportDirectory(PathBuf);
        impl Drop for ReportDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mado-pilot-report-contract-{}-{nonce}",
            std::process::id()
        ));
        private_directory(&path).unwrap();
        let directory = ReportDirectory(path);
        let before_error = write_new(&directory.0, b"", true).unwrap_err();
        let resource =
            ResourceDecision::evaluate(&unselected_resources(), &Ledger::new(), Phase::Final, true);
        let mut text = String::new();
        resource.append_json(&mut text);
        let mut cycle = CycleSummary {
            aggregate: Outcome::Infra,
            eligible: false,
            before_written: false,
            report_written: false,
            output_error: Some(before_error),
            resource: None,
        };
        let report_path = directory.0.join("report.json");
        cycle.persist_report(&report_path, &text, resource);
        assert!(cycle.report_written);
        assert_eq!(cycle.output_error, Some(before_error));
        assert_eq!(cycle.aggregate, Outcome::Infra);
        assert!(!cycle.eligible);
        // Remove the owned immutable report without broadening its permissions.
        assert!(fs::metadata(&report_path).unwrap().permissions().readonly());
        fs::remove_file(report_path).unwrap();
    }

    #[test]
    fn resource_selection_requires_the_complete_bounded_cli_group() {
        let profile = &resources::PROFILES[0];
        let raw = resource_arguments(profile);
        assert!(
            ForeignArguments::parse(&raw[..4])
                .unwrap()
                .resources
                .is_none()
        );
        assert!(ForeignArguments::parse(&raw).unwrap().resources.is_some());
        for missing in 4..12 {
            let mut incomplete = raw.clone();
            incomplete.remove(missing);
            assert!(ForeignArguments::parse(&incomplete).is_err());
        }
        for (index, replacement) in [
            (5, raw[4].clone()),
            (5, OsString::from("--foreign-resource-max-bytes=100")),
            (5, OsString::from("--foreign-resource-hardware=")),
            (
                5,
                OsString::from(format!("--foreign-resource-hardware={}", "a".repeat(257))),
            ),
            (
                6,
                OsString::from("--foreign-resource-os-version=bad\nvalue"),
            ),
            (8, OsString::from("--foreign-resource-source-commit=1234")),
            (
                9,
                OsString::from(format!("--foreign-resource-source-tree={}", "z".repeat(40))),
            ),
            (
                10,
                OsString::from(format!(
                    "--foreign-resource-runner-sha256={}",
                    "a".repeat(63)
                )),
            ),
            (
                11,
                OsString::from(format!(
                    "--foreign-resource-context-sha256={}",
                    "g".repeat(64)
                )),
            ),
        ] {
            let mut invalid = raw.clone();
            invalid[index] = replacement;
            assert!(ForeignArguments::parse(&invalid).is_err());
        }
    }

    #[test]
    fn resource_admission_rejects_unreviewed_profiles_builds_and_artifacts() {
        for profile in &resources::PROFILES {
            let request = resource_request(profile);
            let pins = approved_artifacts(profile, &request, false);
            for (target, release) in [(profile.target, false), ("other-target", true)] {
                let admission =
                    ResourceAdmission::select(Some(&request), target, release, Some(pins));
                assert_eq!(
                    ResourceDecision::evaluate(&admission, &Ledger::new(), Phase::Before, false)
                        .outcome(),
                    Outcome::Infra,
                );
                assert!(!cycle_may_launch(
                    &std::array::from_fn(|_| None),
                    0,
                    admission.error.is_some()
                ));
            }
            assert!(
                profile
                    .admit(&request, profile.target, true, &"0".repeat(64), Some(pins))
                    .is_err()
            );
            let different = "0".repeat(64);
            for pins in [
                ResourceArtifacts {
                    consumer: &different,
                    ..pins
                },
                ResourceArtifacts {
                    library: &different,
                    ..pins
                },
                ResourceArtifacts {
                    fixture: &different,
                    ..pins
                },
                ResourceArtifacts {
                    runner: &different,
                    ..pins
                },
            ] {
                let admission =
                    ResourceAdmission::select(Some(&request), profile.target, true, Some(pins));
                let decision =
                    ResourceDecision::evaluate(&admission, &Ledger::new(), Phase::Before, false);
                assert_eq!(decision.outcome().exit_code(), 2);
                assert!(!decision.may_advance());
            }
            assert!(
                ResourceAdmission::select(Some(&request), profile.target, true, None)
                    .error
                    .is_some()
            );
            for field in 0..4 {
                let mut request = resource_request(profile);
                match field {
                    0 => request.profile_id.push_str("-unreviewed"),
                    1 => request.hardware.push_str("-different"),
                    2 => request.os_version.push_str("-different"),
                    3 => request.topology.push_str("-different"),
                    _ => unreachable!(),
                }
                let pins = approved_artifacts(profile, &request, true);
                let admission =
                    ResourceAdmission::select(Some(&request), profile.target, true, Some(pins));
                assert_eq!(
                    ResourceDecision::evaluate(&admission, &Ledger::new(), Phase::Before, false)
                        .outcome(),
                    Outcome::Infra,
                );
            }
        }
    }

    #[test]
    fn accepted_resource_profiles_can_pass_both_consumers_and_the_complete_cohort() {
        // Representative retained counts exercise the real compiled limits, not
        // a test-only profile or a disabled production enforcement branch.
        let observations = [
            [
                [119_276_480, 173_015_040, 210],
                [119_440_320, 173_211_648, 211],
                [119_489_472, 173_260_800, 214],
                [119_555_008, 173_342_720, 216],
                [442_172_616, 495_943_680, 220],
            ],
            [
                [21_655_552, 36_364_288, 415],
                [24_342_528, 37_982_208, 389],
                [24_694_784, 38_993_920, 423],
                [26_660_864, 39_391_232, 427],
                [40_079_360, 44_216_320, 431],
            ],
        ];
        for (profile, samples) in resources::PROFILES.iter().zip(observations) {
            let request = resource_request(profile);
            for cpp in [false, true] {
                let admission = ResourceAdmission::select(
                    Some(&request),
                    profile.target,
                    true,
                    Some(approved_artifacts(profile, &request, cpp)),
                );
                assert!(admission.error.is_none());
                let ledger = resource_ledger(profile, samples);
                let decision = ResourceDecision::evaluate(&admission, &ledger, Phase::Final, false);
                assert_eq!(decision.effective(8), Outcome::Pass);
                assert_eq!(decision.aggregate().exit_code(), 0);
                assert!(decision.eligible());
                let mut cycles = std::array::from_fn(|_| None);
                for index in 0..CYCLE_COUNT {
                    assert!(cycle_may_launch(&cycles, index, false));
                    cycles[index] = Some(cycle_summary(&ledger, &admission, false));
                }
                assert_eq!(
                    cohort_aggregate(&cycles, Phase::Final, false).exit_code(),
                    0
                );
            }
        }
    }

    #[test]
    fn every_baseline_lifecycle_and_final_limit_is_inclusive_and_binding() {
        for profile in &resources::PROFILES {
            let request = resource_request(profile);
            let admission = ResourceAdmission::select(
                Some(&request),
                profile.target,
                true,
                Some(approved_artifacts(profile, &request, false)),
            );
            for sample in 0..5 {
                for metric in 0..3 {
                    for delta in [false, true] {
                        if sample == 0 && delta {
                            continue;
                        }
                        let budget = &profile.budgets[metric];
                        let limit = if delta {
                            if sample == 4 {
                                budget.delta_final
                            } else {
                                budget.delta_lifecycle
                            }
                        } else if sample == 4 {
                            budget.absolute_final
                        } else {
                            budget.absolute_lifecycle
                        };
                        let baseline = if delta { 0 } else { budget.absolute_lifecycle };
                        let mut samples = [[0; 3]; 5];
                        for values in &mut samples {
                            values[metric] = baseline;
                        }
                        samples[sample][metric] = limit;
                        let at_limit = resource_ledger(profile, samples);
                        assert_eq!(
                            ResourceDecision::evaluate(&admission, &at_limit, Phase::Final, false)
                                .outcome(),
                            Outcome::Pass,
                        );
                        samples[sample][metric] += 1;
                        let over_limit = resource_ledger(profile, samples);
                        let decision = ResourceDecision::evaluate(
                            &admission,
                            &over_limit,
                            Phase::Final,
                            false,
                        );
                        assert_eq!(decision.outcome(), Outcome::Fail);
                        let failures: Vec<_> = decision
                            .comparisons
                            .as_ref()
                            .unwrap()
                            .iter()
                            .filter(|comparison| !comparison.passed())
                            .map(|comparison| {
                                (comparison.sample, comparison.metric, comparison.delta)
                            })
                            .collect();
                        assert_eq!(failures, [(sample, metric, delta)]);
                    }
                }
            }
        }
    }

    #[test]
    fn resource_differences_remain_signed_against_the_own_fixed_baseline() {
        let profile = &resources::PROFILES[0];
        let request = resource_request(profile);
        let admission = ResourceAdmission::select(
            Some(&request),
            profile.target,
            true,
            Some(approved_artifacts(profile, &request, false)),
        );
        let ledger = resource_ledger(
            profile,
            [
                [100, 200, 30],
                [99, 198, 27],
                [98, 196, 24],
                [97, 194, 21],
                [0, 1, 0],
            ],
        );
        let decision = ResourceDecision::evaluate(&admission, &ledger, Phase::Final, false);
        assert_eq!(decision.outcome(), Outcome::Pass);
        let differences: Vec<_> = decision
            .comparisons
            .as_ref()
            .unwrap()
            .iter()
            .filter(|comparison| comparison.delta)
            .map(|comparison| comparison.observed)
            .collect();
        assert_eq!(
            differences,
            [-1, -2, -3, -2, -4, -6, -3, -6, -9, -100, -199, -30]
        );
    }

    #[test]
    fn missing_duplicate_reordered_and_malformed_resource_evidence_is_infra() {
        let profile = &resources::PROFILES[0];
        let request = resource_request(profile);
        let selected = ResourceAdmission::select(
            Some(&request),
            profile.target,
            true,
            Some(approved_artifacts(profile, &request, false)),
        );
        for admission in [&selected, &unselected_resources()] {
            for defect in 0..8 {
                let mut ledger = resource_ledger(profile, [[100, 200, 3]; 5]);
                let indices: Vec<_> = ledger.rows[0]
                    .facts
                    .iter()
                    .enumerate()
                    .filter_map(|(index, fact)| (fact.key == FactKey::Resource).then_some(index))
                    .collect();
                match defect {
                    0 | 1 => {
                        ledger.rows[0].facts.remove(indices[defect]);
                    }
                    2 => {
                        ledger.rows[8].facts.remove(0);
                    }
                    3 => {
                        let duplicate = ledger.rows[0].facts[indices[1]].clone();
                        ledger.rows[0].facts.insert(indices[2], duplicate);
                    }
                    4 => ledger.rows[0].facts.swap(indices[1], indices[2]),
                    5 => {
                        ledger.rows[0].facts[indices[0]].values.pop();
                    }
                    6 => ledger.rows[8].facts[0].values[2] = Number::Real(1.0),
                    7 => ledger.rows[8].facts[0].values[4] = Number::Unsigned(16_777_217),
                    _ => unreachable!(),
                }
                let decision = ResourceDecision::evaluate(admission, &ledger, Phase::Final, false);
                assert_eq!(decision.effective(8), Outcome::Infra);
                assert_eq!(decision.aggregate().exit_code(), 2);
                assert!(!decision.eligible());
            }
        }
        for line in [
            "FACT F1 resource 0 0 1 2",
            "FACT F1 resource 0 0 -1 2 3",
            "FACT F9 resource 2 4 NaN 2 3",
            "FACT F9 resource 2 4 1 2 16777217",
        ] {
            assert!(parse_request(line).is_err());
        }
    }

    #[test]
    fn observed_geometry_not_declared_topology_controls_resource_applicability() {
        for profile in &resources::PROFILES {
            let request = resource_request(profile);
            let admission = ResourceAdmission::select(
                Some(&request),
                profile.target,
                true,
                Some(approved_artifacts(profile, &request, true)),
            );
            let mut ledger = resource_ledger(profile, [[100, 200, 3]; 5]);
            for row in &mut ledger.rows[1..5] {
                row.facts.sort_by_key(|fact| fact.key == FactKey::Frame);
            }
            assert_eq!(
                ResourceDecision::evaluate(&admission, &ledger, Phase::Final, false).outcome(),
                Outcome::Pass
            );
            for (row, key, occurrence, scalar) in [
                (1, FactKey::Frame, 0, 4),
                (2, FactKey::Transform, 0, 5),
                (3, FactKey::Frame, 1, 5),
                (4, FactKey::Transform, 2, 3),
            ] {
                let mut ledger = resource_ledger(profile, [[100, 200, 3]; 5]);
                let fact = ledger.rows[row]
                    .facts
                    .iter_mut()
                    .filter(|fact| fact.key == key)
                    .nth(occurrence)
                    .unwrap();
                fact.values[scalar] = match fact.values[scalar] {
                    Number::Unsigned(value) => Number::Unsigned(value + 1),
                    Number::Real(value) => Number::Real(value + 0.25),
                };
                let decision = ResourceDecision::evaluate(&admission, &ledger, Phase::Final, false);
                assert_eq!(decision.outcome(), Outcome::Infra);
                assert!(!decision.eligible());
            }
            let mut reversed = resource_ledger(profile, [[100, 200, 3]; 5]);
            let before = reversed.rows[3].facts[0].clone();
            reversed.rows[3].facts.push(before);
            assert_eq!(
                ResourceDecision::evaluate(&admission, &reversed, Phase::Final, false).outcome(),
                Outcome::Infra
            );
        }
    }

    #[test]
    fn resource_pass_preserves_stronger_consumer_and_cleanup_failures() {
        let profile = &resources::PROFILES[0];
        let request = resource_request(profile);
        let admission = ResourceAdmission::select(
            Some(&request),
            profile.target,
            true,
            Some(approved_artifacts(profile, &request, false)),
        );
        for (row, outcome) in [
            (0, Outcome::Infra),
            (2, Outcome::Fail),
            (4, Outcome::Unsupported),
            (8, Outcome::Unexecuted),
            (8, Outcome::Fail),
        ] {
            let mut ledger = resource_ledger(profile, [[100, 200, 3]; 5]);
            ledger.rows[row].outcome = Some(outcome);
            let decision = ResourceDecision::evaluate(&admission, &ledger, Phase::Final, false);
            assert_eq!(decision.outcome(), Outcome::Pass);
            assert_eq!(decision.aggregate(), outcome);
            assert!(!decision.eligible());
        }
        let ledger = resource_ledger(profile, [[100, 200, 3]; 5]);
        let decision = ResourceDecision::evaluate(&admission, &ledger, Phase::Final, true);
        assert_eq!(decision.effective(8), Outcome::Infra);
        assert_eq!(decision.aggregate(), Outcome::Infra);
        assert!(!decision.eligible());
    }

    #[test]
    fn an_intermediate_or_final_resource_overrun_stops_after_outer_warmup() {
        for profile in &resources::PROFILES {
            let request = resource_request(profile);
            let admission = ResourceAdmission::select(
                Some(&request),
                profile.target,
                true,
                Some(approved_artifacts(profile, &request, false)),
            );
            for sample in [2, 4] {
                let mut samples = [[100, 200, 3]; 5];
                samples[sample][0] += if sample == 4 {
                    profile.budgets[0].delta_final + 1
                } else {
                    profile.budgets[0].delta_lifecycle + 1
                };
                let ledger = resource_ledger(profile, samples);
                let mut cycles = std::array::from_fn(|_| None);
                cycles[0] = Some(cycle_summary(&ledger, &admission, false));
                for index in 1..CYCLE_COUNT {
                    assert!(!cycle_may_launch(&cycles, index, false));
                }
                assert_eq!(
                    cohort_aggregate(&cycles, Phase::Final, false).exit_code(),
                    1
                );
            }
        }
    }

    #[test]
    fn clean_warmup_can_advance_without_promoting_unaccepted_resource_budget() {
        use super::*;
        let ledger = resource_ledger(&resources::PROFILES[0], [[100, 200, 3]; 5]);
        let admission = unselected_resources();
        let mut cycles = std::array::from_fn(|_| None);
        cycles[0] = Some(cycle_summary(&ledger, &admission, false));
        assert!(cycle_may_launch(&cycles, 1, false));
        assert!(!cycle_may_launch(&cycles, 2, false));
        assert!(!cycle_may_launch(&cycles, 0, false));
        for index in 1..CYCLE_COUNT {
            assert!(cycle_may_launch(&cycles, index, false));
            cycles[index] = Some(cycle_summary(&ledger, &admission, false));
        }
        assert!(!cycle_may_launch(&cycles, CYCLE_COUNT, false));
        let resource = ResourceDecision::evaluate(&admission, &ledger, Phase::Final, false);
        assert_eq!(resource.effective(8), Outcome::Unexecuted);
        assert_eq!(
            cohort_aggregate(&cycles, Phase::Final, false),
            Outcome::Unexecuted
        );
    }

    #[test]
    fn consumer_nonpass_or_infrastructure_failure_blocks_later_cycles() {
        use super::*;
        for (row, outcome, infra) in [
            (0, Outcome::Fail, false),
            (4, Outcome::Unsupported, false),
            (8, Outcome::Unexecuted, false),
            (8, Outcome::Pass, true),
        ] {
            let ledger_profile = &resources::PROFILES[0];
            let mut ledger = resource_ledger(ledger_profile, [[100, 200, 3]; 5]);
            ledger.rows[row].outcome = Some(outcome);
            let mut cycles = std::array::from_fn(|_| None);
            cycles[0] = Some(cycle_summary(&ledger, &unselected_resources(), infra));
            assert!(!cycle_may_launch(&cycles, 1, false));
            assert_eq!(
                cohort_aggregate(&cycles, Phase::Final, false),
                if infra { Outcome::Infra } else { outcome }
            );
        }
    }

    #[test]
    fn fixture_start_reserve_cannot_renew_expired_cohort_authority() {
        use super::*;
        let deadline = Instant::now() - Duration::from_secs(1);
        assert!(require_authority(deadline, Duration::ZERO).is_err());
        assert!(require_authority(deadline, super::super::FIXTURE_WAIT).is_err());
        assert!(
            require_authority(
                Instant::now() + Duration::from_secs(1),
                super::super::FIXTURE_WAIT
            )
            .is_err()
        );
    }

    #[test]
    fn first_queued_handshake_has_no_fabricated_reply_boundary() {
        use super::*;
        let received = Instant::now();
        let setup_finished = received + Duration::from_millis(1);
        assert!(request_deadline(None, received, setup_finished).is_ok());
        assert_eq!(
            request_deadline(Some(setup_finished), received, setup_finished),
            Err("unsolicited_record")
        );
        assert_eq!(
            request_deadline(None, received, received + REPLY_WAIT),
            Err("controller_reply_timeout")
        );
    }

    #[test]
    fn partial_lines_keep_the_first_byte_deadline() {
        use super::*;
        let (sender, lines) = mpsc::sync_channel(QUEUE_LIMIT);
        let (_, written) = mpsc::channel();
        sender
            .try_send(ReadEvent::Started(Instant::now() - REPLY_WAIT))
            .unwrap();
        let consumer = Consumer {
            child: None,
            lines,
            replies: None,
            written,
            stdout: None,
            stderr: None,
            writer: None,
            output: Arc::new(ReaderState::default()),
            errors: Arc::new(ReaderState::default()),
        };
        assert!(matches!(
            consumer.read(Instant::now() + ROW_WAIT),
            Err("consumer_partial_line_timeout")
        ));
    }

    #[test]
    fn source_stamps_preserve_zero_based_counters() {
        use super::*;
        for record in [
            "FACT F2 frame 1 0 0 0 640 452",
            "FACT F2 bootstrap 1 0 0 0 640 452",
            "FACT F3 transform 0 0 0 640 452 1 1 0",
            "FACT F2 frame 1 18446744073709551615 18446744073709551615 18446744073709551615 640 452",
        ] {
            assert!(parse_request(record).is_ok(), "{record}");
        }
        assert!(parse_request("FACT F2 frame 0 0 0 0 640 452").is_err());
    }

    #[test]
    fn admission_preserves_platform_permission_probe_truth() {
        use super::*;
        for permission in [1, 2, 3] {
            let mut ledger = Ledger::new();
            ledger.loaded = true;
            let mut append = |key, values: String| {
                let fact = Fact::parse(key, &values.split(' ').collect::<Vec<_>>()).unwrap();
                ledger.fact(0, fact).unwrap();
            };
            for index in 0..4 {
                append(FactKey::Bootstrap, "1 1 1 1 640 452".into());
                append(
                    FactKey::Resource,
                    format!("{} {index} 100 200 3", u8::from(index != 0)),
                );
            }
            append(FactKey::Permissions, format!("{permission} 3"));
            append(FactKey::Status, "0 0".into());
            let result = ledger.row(0, Outcome::Pass, PASS_REASONS[0]);
            #[cfg(windows)]
            assert_eq!(result.is_ok(), permission == 3);
            #[cfg(target_os = "macos")]
            assert_eq!(result.is_ok(), permission == 1);
        }
    }

    #[test]
    fn resource_snapshots_require_one_warmup_and_ordered_completed_cycles() {
        use super::*;
        fn fact(key: FactKey, values: &str) -> Fact {
            Fact::parse(key, &values.split(' ').collect::<Vec<_>>()).unwrap()
        }
        let mut ledger = Ledger::new();
        ledger.loaded = true;
        let baseline = fact(FactKey::Resource, "0 0 100 200 3");
        assert_eq!(
            ledger.fact(0, baseline.clone()),
            Err("resource_bootstrap_order")
        );
        for index in 0..4 {
            ledger
                .fact(0, fact(FactKey::Bootstrap, "1 1 1 1 640 452"))
                .unwrap();
            let phase = u8::from(index != 0);
            ledger
                .fact(
                    0,
                    fact(FactKey::Resource, &format!("{phase} {index} 100 200 3")),
                )
                .unwrap();
            assert!(ledger.fact(0, baseline.clone()).is_err());
        }
        let permissions = if cfg!(windows) { "3 3" } else { "1 3" };
        ledger
            .fact(0, fact(FactKey::Permissions, permissions))
            .unwrap();
        ledger.fact(0, fact(FactKey::Status, "0 0")).unwrap();
        ledger.row(0, Outcome::Pass, PASS_REASONS[0]).unwrap();
        for row in 1..8 {
            ledger
                .row(row, Outcome::Unexecuted, "prior_row_failed")
                .unwrap();
        }
        ledger.fact(8, fact(FactKey::Status, "0 0")).unwrap();
        assert!(ledger.row(8, Outcome::Pass, PASS_REASONS[8]).is_err());
        ledger
            .fact(8, fact(FactKey::Resource, "2 4 100 200 3"))
            .unwrap();
        ledger.row(8, Outcome::Pass, PASS_REASONS[8]).unwrap();
        ledger.finish().unwrap();
        let resource =
            ResourceDecision::evaluate(&unselected_resources(), &ledger, Phase::Final, false);
        assert_eq!(resource.effective(8), Outcome::Unexecuted);
        let failed =
            ResourceDecision::evaluate(&unselected_resources(), &ledger, Phase::Final, true);
        assert_eq!(failed.effective(8), Outcome::Infra);
    }

    #[test]
    fn partial_eof_and_overflow_never_become_commands() {
        use super::*;
        let mut partial = LineDecoder::default();
        for byte in b"DONE" {
            assert!(partial.push(*byte).unwrap().is_none());
        }
        assert_eq!(partial.eof(), Err("partial_line_eof"));
        let mut complete = LineDecoder::default();
        for byte in b"DONE\r" {
            assert!(complete.push(*byte).unwrap().is_none());
        }
        assert_eq!(complete.push(b'\n').unwrap().as_deref(), Some("DONE"));
        assert!(complete.eof().is_ok());
        let mut oversized = LineDecoder::default();
        for _ in 0..LINE_LIMIT - 1 {
            assert!(oversized.push(b'x').unwrap().is_none());
        }
        assert_eq!(oversized.push(b'x'), Err("line_overflow"));
    }

    #[test]
    fn queue_overflow_is_a_failure_without_a_blocking_send() {
        use super::*;
        let (sender, receiver) = mpsc::sync_channel(2);
        let state = Arc::new(ReaderState::default());
        read_stdout(&b"DONE\nDONE\n"[..], sender, Arc::clone(&state));
        assert!(matches!(receiver.recv().unwrap(), ReadEvent::Started(_)));
        assert!(matches!(receiver.recv().unwrap(), ReadEvent::Complete(line) if line == "DONE"));
        assert!(state.failed.load(Ordering::Acquire));
    }

    #[test]
    fn finite_schema_rejects_malformed_or_unbounded_facts() {
        use super::*;
        for line in [
            "GEOMETRY 640 452 NaN 0 640 452 1 1",
            "GEOMETRY 640 452 0 0 0 452 1 1",
            "GEOMETRY 640 452 0 0 640 452 NaN 1",
            "FACT F2 frame 1 1 1 1 0 452",
            "FACT F2 bootstrap 1",
            "FACT F3 match 1 0 1 1 0 10 10 5 20",
            "FACT F1 permissions 1 1 1",
            "FACT F1 status 18446744073709551616 0",
            "FACT F1 title 1",
            "ROW F1 PASS arbitrary_title",
        ] {
            if let Ok(Request::Row(row, outcome, reason)) = parse_request(line) {
                let mut ledger = Ledger::new();
                ledger.loaded = true;
                assert!(ledger.row(row, outcome, reason).is_err());
            } else {
                assert!(parse_request(line).is_err());
            }
        }
    }

    #[test]
    fn sequential_complete_ledger_preserves_nonpass_and_infra_precedence() {
        use super::*;
        let mut ledger = Ledger::new();
        ledger.loaded = true;
        assert!(
            ledger
                .row(1, Outcome::Unexecuted, "prior_row_failed")
                .is_err()
        );
        assert!(ledger.row(0, Outcome::Pass, PASS_REASONS[0]).is_err());
        ledger
            .row(0, Outcome::Unexecuted, "prior_row_failed")
            .unwrap();
        assert!(
            ledger
                .row(0, Outcome::Unexecuted, "prior_row_failed")
                .is_err()
        );
        assert!(ledger.finish().is_err());
        for row in 1..9 {
            ledger
                .row(row, Outcome::Unexecuted, "prior_row_failed")
                .unwrap();
        }
        ledger.finish().unwrap();
        let resource =
            ResourceDecision::evaluate(&unselected_resources(), &ledger, Phase::Final, false);
        assert_eq!(resource.aggregate(), Outcome::Unexecuted);
        let failed =
            ResourceDecision::evaluate(&unselected_resources(), &ledger, Phase::Final, true);
        assert_eq!(failed.aggregate(), Outcome::Infra);
        assert!(ledger.finish().is_err());
    }

    #[test]
    fn valid_exit_cannot_hide_unjoined_or_forced_cleanup() {
        use super::*;
        let mut result = ConsumerFinalization {
            reaped: true,
            exit_code: Some(0),
            exit_success: true,
            stdout_joined: true,
            stderr_joined: true,
            writer_joined: true,
            stdout_eof: true,
            stderr_eof: true,
            output_clean: true,
            ..ConsumerFinalization::default()
        };
        assert!(result.accepted());
        result.stdout_joined = false;
        assert!(!result.accepted());
        result.stdout_joined = true;
        result.termination_requested = true;
        assert!(!result.accepted());
    }

    #[test]
    fn foreign_arguments_do_not_inherit_legacy_or_duplicate_switches() {
        use super::*;
        #[cfg(unix)]
        let root = "/owned";
        #[cfg(windows)]
        let root = "C:\\owned";
        let mut raw: Vec<OsString> = [
            "--foreign-consumer",
            "--fixture-executable",
            "--foreign-artifact-dir",
            "--foreign-library",
        ]
        .iter()
        .map(|key| OsString::from(format!("{key}={root}")))
        .collect();
        assert!(ForeignArguments::parse(&raw).is_ok());
        raw[3] = raw[0].clone();
        assert!(ForeignArguments::parse(&raw).is_err());
        raw[3] = OsString::from("--lane-c-evidence");
        assert!(ForeignArguments::parse(&raw).is_err());
    }
}
