//! Typed, privacy-allowlisted report schema for the bounded ScreenCaptureKit
//! suspension failure-signature campaign.
//!
//! One fresh diagnostic process emits one terminal JSON row. The schema carries
//! only closed enums, fixed-length digests, bounded numeric state, and fixed-size
//! arrays; it has no field for native identifiers or free-form payloads.
#![expect(
    missing_docs,
    reason = "public names are the fixed machine-readable evidence schema"
)]
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const STATUS_KIND_COUNT: usize = 8;
pub const TRANSITION_CAPACITY: usize = 16;
pub const DISPLAY_CAPACITY: usize = 2;
pub const PROCESS_COUNT: usize = 10;
pub const OPERATION_DEADLINE_MILLIS: u32 = 5_000;
pub const LIFECYCLE_STEP_CAPACITY: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyClass {
    SingleDisplayControl,
    HistoricalTwoDisplay,
}

impl TopologyClass {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "single_display_control" => Some(Self::SingleDisplayControl),
            "historical_two_display" => Some(Self::HistoricalTwoDisplay),
            _ => None,
        }
    }
}

pub const ORDER: [TopologyClass; PROCESS_COUNT] = [
    TopologyClass::SingleDisplayControl,
    TopologyClass::SingleDisplayControl,
    TopologyClass::SingleDisplayControl,
    TopologyClass::SingleDisplayControl,
    TopologyClass::SingleDisplayControl,
    TopologyClass::HistoricalTwoDisplay,
    TopologyClass::HistoricalTwoDisplay,
    TopologyClass::HistoricalTwoDisplay,
    TopologyClass::HistoricalTwoDisplay,
    TopologyClass::HistoricalTwoDisplay,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostProfile {
    AppleM1Pro10c32g,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsProfile {
    Macos26_6_2Build25g83,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdkProfile {
    Xcode26_5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_revision: String,
    pub source_tree: String,
    pub protocol_sha256: String,
    pub executable_sha256: String,
    pub fixture_sha256: String,
    pub fixture_source_sha256: String,
    pub host: HostProfile,
    pub os: OsProfile,
    pub sdk: SdkProfile,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayMode {
    pub logical_width: u32,
    pub logical_height: u32,
    pub refresh_millihertz: u32,
    pub built_in: bool,
    pub mirrored: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayTopology {
    pub display_count: u32,
    pub modes: [Option<DisplayMode>; DISPLAY_CAPACITY],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusEvent {
    pub kind: u32,
    pub raw_value: i64,
    pub sequence: u64,
    pub monotonic_nanos: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub close_phase: u32,
    pub active_native_slots: u32,
    pub observed_total: u64,
    pub status_counts: [u64; STATUS_KIND_COUNT],
    pub first_status: Option<StatusEvent>,
    pub last_status: Option<StatusEvent>,
    pub callbacks_received: u64,
    pub callbacks_admitted: u64,
    pub callbacks_refused: u64,
    pub callbacks_exited: u64,
    pub stream_start_completed_nanos: u64,
    pub stream_stop_requested_nanos: u64,
    pub stream_stop_completed_nanos: u64,
    pub callback_admission_stopped_nanos: u64,
    pub callback_fence_completed_nanos: u64,
    pub close_completed_nanos: u64,
    pub first_complete_nanos: u64,
    pub session_references: u64,
    pub detached_leases: u64,
    pub native_objects: u64,
    pub detached_bytes: u64,
    pub transition_overwrites: u64,
    pub transition_count: u32,
    pub transitions: [Option<StatusEvent>; TRANSITION_CAPACITY],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessResources {
    pub native_objects: u64,
    pub detached_bytes: u64,
    pub live_sessions: u64,
    pub callbacks_in_flight: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedOwners {
    pub terminal_result: bool,
    pub mapping: bool,
    pub mapping_readable_after_fresh_close: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameIdentity {
    pub stream_ordinal: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub geometry_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStep {
    OldResultRetained,
    OldMappingRetained,
    OldSessionCloseAttempted,
    OldPublicOwnersDropped,
    OldSnapshotRecorded,
    FreshSessionOpened,
    FreshCaptureAttempted,
    FreshSessionCloseAttempted,
    MappingVerified,
    RetainedOwnersDropped,
    FixtureCloseAttempted,
}

pub const LIFECYCLE_ORDER: [LifecycleStep; LIFECYCLE_STEP_CAPACITY] = [
    LifecycleStep::OldResultRetained,
    LifecycleStep::OldMappingRetained,
    LifecycleStep::OldSessionCloseAttempted,
    LifecycleStep::OldPublicOwnersDropped,
    LifecycleStep::OldSnapshotRecorded,
    LifecycleStep::FreshSessionOpened,
    LifecycleStep::FreshCaptureAttempted,
    LifecycleStep::FreshSessionCloseAttempted,
    LifecycleStep::MappingVerified,
    LifecycleStep::RetainedOwnersDropped,
    LifecycleStep::FixtureCloseAttempted,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleTrace {
    pub count: u32,
    pub steps: [Option<LifecycleStep>; LIFECYCLE_STEP_CAPACITY],
}

impl LifecycleTrace {
    pub fn record(&mut self, step: LifecycleStep) -> Result<(), ValidationFault> {
        let index = usize::try_from(self.count).map_err(|_| ValidationFault::Order)?;
        if LIFECYCLE_ORDER.get(index) != Some(&step) {
            return Err(ValidationFault::Order);
        }
        self.steps[index] = Some(step);
        self.count = self.count.checked_add(1).ok_or(ValidationFault::Order)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Setup,
    TopologyPreflight,
    FixtureLaunch,
    OldCapture,
    OldClose,
    OldOwnerDrop,
    FreshOpen,
    FreshCapture,
    FreshClose,
    Cleanup,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    CompleteFrame,
    ExplicitProducerState,
    MissingProgress,
    OperationFailed,
    TopologyMismatch,
    BaselineLeak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    None,
    IdentityMismatch,
    TopologyMismatch,
    PermissionUnavailable,
    FixtureUnavailable,
    TargetUnavailable,
    OldCaptureFailed,
    OldCloseFailed,
    FreshOpenFailed,
    FreshCaptureDeadline,
    FreshCaptureFailed,
    FreshCloseFailed,
    MappingUnreadable,
    DiagnosticSnapshotFailed,
    CleanupFailed,
    BaselineLeak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub schema_version: u32,
    pub process_index: u32,
    pub topology: TopologyClass,
    pub provenance: Provenance,
    pub observed_topology: Option<DisplayTopology>,
    pub baseline: Option<ProcessResources>,
    pub post_old_drop_resources: Option<ProcessResources>,
    pub final_resources: Option<ProcessResources>,
    pub owners: RetainedOwners,
    pub old_fixture_revision: u64,
    pub operation_deadline_millis: u32,
    pub fresh_target_authenticated: Option<bool>,
    pub lifecycle: LifecycleTrace,
    pub fresh_fixture_revision: u64,
    pub old_frame: Option<FrameIdentity>,
    pub fresh_frame: Option<FrameIdentity>,
    pub streams_distinct: Option<bool>,
    pub first_complete_latency_nanos: Option<u64>,
    pub old_snapshot: Option<Snapshot>,
    pub fresh_snapshot: Option<Snapshot>,
    pub stage: Stage,
    pub outcome: Outcome,
    pub failure: FailureClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationFault {
    Schema,
    Privacy,
    Order,
    Topology,
    Snapshot,
    Ownership,
    BaselineLeak,
    Terminal,
}

impl Row {
    #[must_use]
    pub const fn completed(&self) -> bool {
        matches!(self.stage, Stage::Complete) && matches!(self.outcome, Outcome::CompleteFrame)
    }

    #[must_use]
    pub const fn exit_success(&self) -> bool {
        self.completed()
    }

    #[must_use]
    pub const fn terminal(&self) -> bool {
        match self.outcome {
            Outcome::CompleteFrame | Outcome::ExplicitProducerState => {
                matches!(self.stage, Stage::Complete) && matches!(self.failure, FailureClass::None)
            }
            Outcome::MissingProgress => {
                matches!(self.stage, Stage::FreshCapture)
                    && matches!(self.failure, FailureClass::FreshCaptureDeadline)
            }
            Outcome::OperationFailed => {
                !matches!(self.stage, Stage::Complete)
                    && !matches!(
                        self.failure,
                        FailureClass::None
                            | FailureClass::TopologyMismatch
                            | FailureClass::FreshCaptureDeadline
                            | FailureClass::BaselineLeak
                    )
            }
            Outcome::TopologyMismatch => {
                matches!(self.stage, Stage::TopologyPreflight)
                    && matches!(self.failure, FailureClass::TopologyMismatch)
            }
            Outcome::BaselineLeak => {
                matches!(self.stage, Stage::Cleanup)
                    && matches!(self.failure, FailureClass::BaselineLeak)
            }
        }
    }

    pub fn to_json_line(&self) -> Result<String, ValidationFault> {
        validate_row(self)?;
        serde_json::to_string(self).map_err(|_| ValidationFault::Schema)
    }
}

pub fn parse_json_line(line: &str) -> Result<Row, ValidationFault> {
    if line.contains(['\n', '\r']) {
        return Err(ValidationFault::Privacy);
    }
    let row: Row = serde_json::from_str(line).map_err(|_| ValidationFault::Privacy)?;
    validate_row(&row)?;
    Ok(row)
}

pub fn topology_matches(class: TopologyClass, observed: DisplayTopology) -> bool {
    observed == expected_topology(class)
}

pub fn validate_row(row: &Row) -> Result<(), ValidationFault> {
    validate_privacy_schema(row)?;
    let index = usize::try_from(row.process_index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .filter(|index| *index < PROCESS_COUNT)
        .ok_or(ValidationFault::Order)?;
    if ORDER[index] != row.topology || !row.terminal() {
        return Err(if ORDER[index] != row.topology {
            ValidationFault::Order
        } else {
            ValidationFault::Terminal
        });
    }
    if row.operation_deadline_millis != OPERATION_DEADLINE_MILLIS {
        return Err(ValidationFault::Schema);
    }
    validate_lifecycle(row)?;

    match (row.outcome, row.observed_topology) {
        (Outcome::OperationFailed, None)
            if matches!(row.stage, Stage::Setup | Stage::TopologyPreflight) => {}
        (Outcome::OperationFailed, _)
            if matches!(row.stage, Stage::Cleanup)
                && matches!(row.failure, FailureClass::DiagnosticSnapshotFailed) => {}
        (Outcome::BaselineLeak, _) => {}
        (Outcome::TopologyMismatch, Some(observed))
            if !topology_matches(row.topology, observed) => {}
        (Outcome::TopologyMismatch, _) => return Err(ValidationFault::Topology),
        (_, Some(observed)) if topology_matches(row.topology, observed) => {}
        _ => return Err(ValidationFault::Topology),
    }

    for snapshot in [row.old_snapshot.as_ref(), row.fresh_snapshot.as_ref()]
        .into_iter()
        .flatten()
    {
        validate_snapshot(snapshot)?;
    }

    if row.owners.mapping_readable_after_fresh_close && !row.owners.mapping {
        return Err(ValidationFault::Ownership);
    }
    if row.post_old_drop_resources.is_some()
        && (!row.owners.terminal_result
            || !row.owners.mapping
            || row.old_frame.is_none()
            || row.old_fixture_revision == 0
            || row.old_snapshot.is_none())
    {
        return Err(ValidationFault::Ownership);
    }
    if row.fresh_frame.is_some()
        && (row.streams_distinct != Some(true)
            || row.fresh_fixture_revision <= row.old_fixture_revision)
    {
        return Err(ValidationFault::Ownership);
    }

    match (row.baseline, row.final_resources, row.outcome, row.failure) {
        (
            Some(baseline),
            Some(final_resources),
            Outcome::BaselineLeak,
            FailureClass::BaselineLeak,
        ) if baseline != final_resources => {}
        (Some(baseline), Some(final_resources), outcome, _)
            if !matches!(outcome, Outcome::BaselineLeak) && baseline == final_resources => {}
        (None, _, Outcome::OperationFailed, FailureClass::DiagnosticSnapshotFailed)
        | (Some(_), None, Outcome::OperationFailed, FailureClass::DiagnosticSnapshotFailed) => {}
        _ => return Err(ValidationFault::BaselineLeak),
    }
    if matches!(
        row.outcome,
        Outcome::CompleteFrame | Outcome::ExplicitProducerState | Outcome::MissingProgress
    ) {
        let baseline = row.baseline.ok_or(ValidationFault::Ownership)?;
        let post_drop = row
            .post_old_drop_resources
            .ok_or(ValidationFault::Ownership)?;
        let old = row
            .old_snapshot
            .as_ref()
            .ok_or(ValidationFault::Ownership)?;
        let fresh = row
            .fresh_snapshot
            .as_ref()
            .ok_or(ValidationFault::Ownership)?;
        if old.detached_leases == 0
            || old.session_references == 0
            || old.native_objects == 0
            || old.detached_bytes <= baseline.detached_bytes
            || post_drop.native_objects <= baseline.native_objects
            || post_drop.detached_bytes <= baseline.detached_bytes
            || post_drop.live_sessions <= baseline.live_sessions
            || old.close_phase != 6
            || fresh.close_phase != 6
        {
            return Err(ValidationFault::Ownership);
        }
    }

    match row.outcome {
        Outcome::CompleteFrame => {
            let fresh = row
                .fresh_snapshot
                .as_ref()
                .ok_or(ValidationFault::Snapshot)?;
            if row.old_snapshot.is_none()
                || row.post_old_drop_resources.is_none()
                || row.old_frame.is_none()
                || row.fresh_frame.is_none()
                || row.first_complete_latency_nanos.is_none()
                || !row.owners.terminal_result
                || !row.owners.mapping
                || !row.owners.mapping_readable_after_fresh_close
                || fresh.status_counts[0] == 0
                || fresh.first_complete_nanos == 0
            {
                return Err(ValidationFault::Schema);
            }
        }
        Outcome::ExplicitProducerState => {
            let fresh = row
                .fresh_snapshot
                .as_ref()
                .ok_or(ValidationFault::Snapshot)?;
            if row.old_snapshot.is_none()
                || row.post_old_drop_resources.is_none()
                || row.old_frame.is_none()
                || row.fresh_frame.is_some()
                || row.first_complete_latency_nanos.is_some()
                || !row.owners.mapping_readable_after_fresh_close
                || fresh.observed_total == 0
                || fresh.status_counts[0] != 0
                || fresh.last_status.is_none()
            {
                return Err(ValidationFault::Schema);
            }
        }
        Outcome::MissingProgress => {
            if row.old_snapshot.is_none()
                || row.post_old_drop_resources.is_none()
                || row.old_frame.is_none()
                || row.fresh_frame.is_some()
                || row.first_complete_latency_nanos.is_some()
                || row.fresh_snapshot.is_none()
                || !row.owners.mapping_readable_after_fresh_close
            {
                return Err(ValidationFault::Schema);
            }
        }
        Outcome::OperationFailed => {}
        Outcome::TopologyMismatch => {
            if row.old_snapshot.is_some()
                || row.fresh_snapshot.is_some()
                || row.post_old_drop_resources.is_some()
                || row.old_frame.is_some()
                || row.fresh_frame.is_some()
                || row.owners != RetainedOwners::default()
            {
                return Err(ValidationFault::Topology);
            }
        }
        Outcome::BaselineLeak => {}
    }
    Ok(())
}

fn validate_lifecycle(row: &Row) -> Result<(), ValidationFault> {
    let count = usize::try_from(row.lifecycle.count).map_err(|_| ValidationFault::Order)?;
    if count > LIFECYCLE_STEP_CAPACITY {
        return Err(ValidationFault::Order);
    }
    for (index, slot) in row.lifecycle.steps.iter().enumerate() {
        let expected = (index < count).then_some(LIFECYCLE_ORDER[index]);
        if *slot != expected {
            return Err(ValidationFault::Order);
        }
    }
    let requires_complete_trace = matches!(
        row.outcome,
        Outcome::CompleteFrame | Outcome::ExplicitProducerState | Outcome::MissingProgress
    );
    if requires_complete_trace
        && (count != LIFECYCLE_STEP_CAPACITY || row.fresh_target_authenticated != Some(true))
    {
        return Err(ValidationFault::Order);
    }
    if matches!(row.outcome, Outcome::TopologyMismatch)
        && (count != 0 || row.fresh_target_authenticated.is_some())
    {
        return Err(ValidationFault::Order);
    }
    Ok(())
}

pub fn validate_aggregate(rows: &[Row]) -> Result<(), ValidationFault> {
    if rows.len() != PROCESS_COUNT {
        return Err(ValidationFault::Order);
    }
    let provenance = &rows[0].provenance;
    for (index, row) in rows.iter().enumerate() {
        let expected_index = u32::try_from(index + 1).map_err(|_| ValidationFault::Order)?;
        if row.process_index != expected_index
            || row.topology != ORDER[index]
            || row.provenance != *provenance
        {
            return Err(ValidationFault::Order);
        }
        validate_row(row)?;
    }
    Ok(())
}

const fn expected_topology(class: TopologyClass) -> DisplayTopology {
    match class {
        TopologyClass::SingleDisplayControl => DisplayTopology {
            display_count: 1,
            modes: [
                Some(DisplayMode {
                    logical_width: 1_512,
                    logical_height: 982,
                    refresh_millihertz: 120_000,
                    built_in: true,
                    mirrored: false,
                }),
                None,
            ],
        },
        TopologyClass::HistoricalTwoDisplay => DisplayTopology {
            display_count: 2,
            modes: [
                Some(DisplayMode {
                    logical_width: 1_512,
                    logical_height: 982,
                    refresh_millihertz: 120_000,
                    built_in: false,
                    mirrored: false,
                }),
                Some(DisplayMode {
                    logical_width: 3_840,
                    logical_height: 2_160,
                    refresh_millihertz: 120_000,
                    built_in: false,
                    mirrored: false,
                }),
            ],
        },
    }
}

fn validate_privacy_schema(row: &Row) -> Result<(), ValidationFault> {
    if row.schema_version != SCHEMA_VERSION
        || !is_hex(&row.provenance.source_revision, 40)
        || !is_hex(&row.provenance.source_tree, 40)
        || !is_hex(&row.provenance.protocol_sha256, 64)
        || !is_hex(&row.provenance.executable_sha256, 64)
        || !is_hex(&row.provenance.fixture_sha256, 64)
        || !is_hex(&row.provenance.fixture_source_sha256, 64)
    {
        return Err(ValidationFault::Privacy);
    }
    Ok(())
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), ValidationFault> {
    const ACTIVE_NATIVE_SLOT_MASK: u32 = 0x3f;
    let observed = snapshot
        .status_counts
        .iter()
        .copied()
        .fold(0_u64, u64::saturating_add);
    if snapshot.close_phase > 6
        || snapshot.active_native_slots & !ACTIVE_NATIVE_SLOT_MASK != 0
        || snapshot.transition_count as usize > TRANSITION_CAPACITY
        || snapshot.callbacks_received
            != snapshot
                .callbacks_admitted
                .saturating_add(snapshot.callbacks_refused)
        || snapshot.observed_total != observed
        || (snapshot.close_phase == 6
            && (snapshot.active_native_slots != 0
                || snapshot.callbacks_received != snapshot.callbacks_exited
                || snapshot.stream_stop_requested_nanos == 0
                || snapshot.stream_stop_completed_nanos == 0
                || snapshot.callback_admission_stopped_nanos == 0
                || snapshot.callback_fence_completed_nanos == 0
                || snapshot.close_completed_nanos == 0))
        || !ordered(
            snapshot.stream_stop_requested_nanos,
            snapshot.stream_stop_completed_nanos,
        )
        || !ordered(
            snapshot.stream_stop_completed_nanos,
            snapshot.callback_admission_stopped_nanos,
        )
        || !ordered(
            snapshot.callback_admission_stopped_nanos,
            snapshot.callback_fence_completed_nanos,
        )
        || !ordered(
            snapshot.callback_fence_completed_nanos,
            snapshot.close_completed_nanos,
        )
        || (snapshot.status_counts[0] == 0) != (snapshot.first_complete_nanos == 0)
    {
        return Err(ValidationFault::Snapshot);
    }

    match (
        snapshot.observed_total,
        snapshot.first_status,
        snapshot.last_status,
    ) {
        (0, None, None) => {}
        (0, _, _) | (_, None, _) | (_, _, None) => return Err(ValidationFault::Snapshot),
        (total, Some(first), Some(last)) => {
            if !valid_event(first, total)
                || !valid_event(last, total)
                || first.sequence != 1
                || last.sequence != total
                || first.monotonic_nanos > last.monotonic_nanos
            {
                return Err(ValidationFault::Snapshot);
            }
        }
    }

    let count = snapshot.transition_count as usize;
    if snapshot.transitions[..count].iter().any(Option::is_none)
        || snapshot.transitions[count..].iter().any(Option::is_some)
        || (count < TRANSITION_CAPACITY && snapshot.transition_overwrites != 0)
    {
        return Err(ValidationFault::Snapshot);
    }
    let mut prior = None;
    for event in snapshot.transitions[..count].iter().flatten().copied() {
        if !valid_event(event, snapshot.observed_total)
            || prior.is_some_and(|prior: StatusEvent| {
                event.sequence < prior.sequence
                    || (event.sequence == prior.sequence && event.sequence != u64::MAX)
                    || event.monotonic_nanos < prior.monotonic_nanos
            })
        {
            return Err(ValidationFault::Snapshot);
        }
        prior = Some(event);
    }
    Ok(())
}

const fn ordered(before: u64, after: u64) -> bool {
    before == 0 || after == 0 || before <= after
}

fn valid_event(event: StatusEvent, observed_total: u64) -> bool {
    (event.kind as usize) < STATUS_KIND_COUNT
        && event.sequence != 0
        && event.sequence <= observed_total
        && event.monotonic_nanos != 0
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    use super::*;

    const SHA1: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const FROZEN_V1_SHA256: &str =
        "682df0373126a695d2e2d5624efd6efa65527ef411d56aa45a94e30d6790f139";

    fn provenance() -> Provenance {
        Provenance {
            source_revision: SHA1.to_owned(),
            source_tree: SHA1.to_owned(),
            protocol_sha256: SHA256.to_owned(),
            executable_sha256: SHA256.to_owned(),
            fixture_sha256: SHA256.to_owned(),
            fixture_source_sha256: SHA256.to_owned(),
            host: HostProfile::AppleM1Pro10c32g,
            os: OsProfile::Macos26_6_2Build25g83,
            sdk: SdkProfile::Xcode26_5,
        }
    }

    const fn event(kind: u32, sequence: u64, monotonic_nanos: u64) -> StatusEvent {
        StatusEvent {
            kind,
            raw_value: i64::from_be_bytes((kind as i64).to_be_bytes()),
            sequence,
            monotonic_nanos,
        }
    }

    fn snapshot(kind: Option<u32>) -> Snapshot {
        let mut status_counts = [0; STATUS_KIND_COUNT];
        let observed = u64::from(kind.is_some());
        let status_event = kind.map(|kind| {
            status_counts[kind as usize] = 1;
            event(kind, 1, 10)
        });
        let mut transitions = [None; TRANSITION_CAPACITY];
        transitions[0] = status_event;
        Snapshot {
            close_phase: 6,
            active_native_slots: 0,
            observed_total: observed,
            status_counts,
            first_status: status_event,
            last_status: status_event,
            callbacks_received: observed,
            callbacks_admitted: observed,
            callbacks_refused: 0,
            callbacks_exited: observed,
            stream_start_completed_nanos: 20,
            stream_stop_requested_nanos: 30,
            stream_stop_completed_nanos: 40,
            callback_admission_stopped_nanos: 50,
            callback_fence_completed_nanos: 60,
            close_completed_nanos: 70,
            first_complete_nanos: u64::from(matches!(kind, Some(0))) * 10,
            session_references: 2,
            detached_leases: 1,
            native_objects: 1,
            detached_bytes: 64,
            transition_overwrites: 0,
            transition_count: u32::from(kind.is_some()),
            transitions,
        }
    }

    fn completed_row(index: u32) -> Row {
        let topology = ORDER[index as usize - 1];
        Row {
            schema_version: SCHEMA_VERSION,
            process_index: index,
            topology,
            provenance: provenance(),
            observed_topology: Some(expected_topology(topology)),
            baseline: Some(ProcessResources::default()),
            post_old_drop_resources: Some(ProcessResources {
                native_objects: 1,
                detached_bytes: 64,
                live_sessions: 1,
                callbacks_in_flight: 0,
            }),
            final_resources: Some(ProcessResources::default()),
            owners: RetainedOwners {
                terminal_result: true,
                mapping: true,
                mapping_readable_after_fresh_close: true,
            },
            old_fixture_revision: 1,
            fresh_fixture_revision: 2,
            operation_deadline_millis: OPERATION_DEADLINE_MILLIS,
            fresh_target_authenticated: Some(true),
            lifecycle: completed_lifecycle(),
            old_frame: Some(FrameIdentity {
                stream_ordinal: 1,
                epoch: 0,
                sequence: 1,
                geometry_revision: 0,
            }),
            fresh_frame: Some(FrameIdentity {
                stream_ordinal: 1,
                epoch: 0,
                sequence: 1,
                geometry_revision: 0,
            }),
            streams_distinct: Some(true),
            first_complete_latency_nanos: Some(0),
            old_snapshot: Some(snapshot(Some(0))),
            fresh_snapshot: Some(snapshot(Some(0))),
            stage: Stage::Complete,
            outcome: Outcome::CompleteFrame,
            failure: FailureClass::None,
        }
    }
    fn completed_lifecycle() -> LifecycleTrace {
        let mut trace = LifecycleTrace::default();
        for step in LIFECYCLE_ORDER {
            trace.record(step).unwrap();
        }
        trace
    }

    #[test]
    fn every_terminal_outcome_is_retainable_without_becoming_green() {
        let complete = completed_row(1);
        assert!(complete.completed());
        assert!(validate_row(&complete).is_ok());
        assert!(complete.exit_success());

        let mut explicit = completed_row(1);
        explicit.fresh_frame = None;
        explicit.first_complete_latency_nanos = None;
        explicit.fresh_snapshot = Some(snapshot(Some(4)));
        explicit.outcome = Outcome::ExplicitProducerState;
        assert!(!explicit.completed());
        assert!(validate_row(&explicit).is_ok());
        assert!(!explicit.exit_success());

        let mut missing = explicit.clone();
        missing.fresh_snapshot = Some(snapshot(None));
        missing.stage = Stage::FreshCapture;
        missing.outcome = Outcome::MissingProgress;
        missing.failure = FailureClass::FreshCaptureDeadline;
        assert!(validate_row(&missing).is_ok());
        assert!(!missing.exit_success());
        missing.owners.mapping_readable_after_fresh_close = false;
        assert_eq!(validate_row(&missing), Err(ValidationFault::Schema));

        let mut failed = completed_row(1);
        failed.fresh_target_authenticated = None;
        failed.lifecycle = LifecycleTrace::default();
        failed.post_old_drop_resources = None;
        failed.owners = RetainedOwners::default();
        failed.old_fixture_revision = 0;
        failed.fresh_fixture_revision = 0;
        failed.old_frame = None;
        failed.fresh_frame = None;
        failed.streams_distinct = None;
        failed.first_complete_latency_nanos = None;
        failed.old_snapshot = None;
        failed.fresh_snapshot = None;
        failed.stage = Stage::FixtureLaunch;
        failed.outcome = Outcome::OperationFailed;
        failed.failure = FailureClass::FixtureUnavailable;
        assert!(validate_row(&failed).is_ok());

        let mut identity_mismatch = failed.clone();
        identity_mismatch.observed_topology = None;
        identity_mismatch.stage = Stage::Setup;
        identity_mismatch.failure = FailureClass::IdentityMismatch;
        assert!(validate_row(&identity_mismatch).is_ok());

        let mut diagnostic_failure = identity_mismatch;
        diagnostic_failure.baseline = None;
        diagnostic_failure.final_resources = None;
        diagnostic_failure.failure = FailureClass::DiagnosticSnapshotFailed;
        assert!(validate_row(&diagnostic_failure).is_ok());

        let mut mismatch = failed.clone();
        mismatch.observed_topology = Some(expected_topology(TopologyClass::HistoricalTwoDisplay));
        mismatch.stage = Stage::TopologyPreflight;
        mismatch.outcome = Outcome::TopologyMismatch;
        mismatch.failure = FailureClass::TopologyMismatch;
        assert!(validate_row(&mismatch).is_ok());

        let mut cleanup_diagnostic_failure = mismatch;
        cleanup_diagnostic_failure.stage = Stage::Cleanup;
        cleanup_diagnostic_failure.outcome = Outcome::OperationFailed;
        cleanup_diagnostic_failure.failure = FailureClass::DiagnosticSnapshotFailed;
        cleanup_diagnostic_failure.final_resources = None;
        assert!(validate_row(&cleanup_diagnostic_failure).is_ok());

        let mut leak = failed;
        leak.final_resources = Some(ProcessResources {
            native_objects: 1,
            ..ProcessResources::default()
        });
        leak.stage = Stage::Cleanup;
        leak.outcome = Outcome::BaselineLeak;
        leak.failure = FailureClass::BaselineLeak;
        assert!(validate_row(&leak).is_ok());
        leak.observed_topology = None;
        assert!(validate_row(&leak).is_ok());
    }

    #[test]
    fn aggregate_requires_all_ten_rows_in_exact_order_and_provenance() {
        let process_count = u32::try_from(PROCESS_COUNT).expect("process count fits u32");
        let rows = (1..=process_count).map(completed_row).collect::<Vec<_>>();
        assert!(validate_aggregate(&rows).is_ok());

        let mut missing = rows.clone();
        missing.pop();
        assert_eq!(validate_aggregate(&missing), Err(ValidationFault::Order));

        let mut reordered = rows.clone();
        reordered.swap(0, 1);
        assert_eq!(validate_aggregate(&reordered), Err(ValidationFault::Order));

        let mut changed = rows;
        changed[9]
            .provenance
            .protocol_sha256
            .replace_range(0..1, "f");
        assert_eq!(validate_aggregate(&changed), Err(ValidationFault::Order));
    }

    #[test]
    fn privacy_schema_rejects_malformed_digests_unknown_fields_and_multiline_rows() {
        let mut row = completed_row(1);
        row.provenance.source_revision.pop();
        assert_eq!(validate_row(&row), Err(ValidationFault::Privacy));

        let row = completed_row(1);
        let mut value = serde_json::to_value(&row).expect("serializes");
        value
            .as_object_mut()
            .expect("row object")
            .insert("native_identifier".to_owned(), serde_json::json!(17));
        let unknown = serde_json::to_string(&value).expect("serializes");
        assert_eq!(parse_json_line(&unknown), Err(ValidationFault::Privacy));
        let line = completed_row(1).to_json_line().expect("valid row");
        assert_eq!(
            parse_json_line(&format!("{line}\n")),
            Err(ValidationFault::Privacy)
        );
    }

    #[test]
    fn topology_profiles_reject_unsupported_refresh_mirroring_and_counts() {
        let single = expected_topology(TopologyClass::SingleDisplayControl);

        let historical = expected_topology(TopologyClass::HistoricalTwoDisplay);
        assert!(topology_matches(
            TopologyClass::SingleDisplayControl,
            single
        ));
        assert!(topology_matches(
            TopologyClass::HistoricalTwoDisplay,
            historical
        ));

        let mut unsupported_refresh = single;
        unsupported_refresh.modes[0]
            .as_mut()
            .expect("single mode")
            .refresh_millihertz = 0;
        assert!(!topology_matches(
            TopologyClass::SingleDisplayControl,
            unsupported_refresh
        ));

        let mut mirrored = historical;
        mirrored.modes[1].as_mut().expect("second mode").mirrored = true;
        assert!(!topology_matches(
            TopologyClass::HistoricalTwoDisplay,
            mirrored
        ));

        let mut wrong_count = single;
        wrong_count.display_count = 2;
        assert!(!topology_matches(
            TopologyClass::SingleDisplayControl,
            wrong_count
        ));

        let mut built_in = historical;
        built_in.modes[0].as_mut().expect("first mode").built_in = true;
        assert!(!topology_matches(
            TopologyClass::HistoricalTwoDisplay,
            built_in
        ));
    }
    #[test]
    fn lifecycle_trace_rejects_retry_reopen_and_order_drift() {
        let mut trace = LifecycleTrace::default();
        assert_eq!(
            trace.record(LifecycleStep::OldMappingRetained),
            Err(ValidationFault::Order)
        );
        trace.record(LifecycleStep::OldResultRetained).unwrap();
        assert_eq!(
            trace.record(LifecycleStep::OldResultRetained),
            Err(ValidationFault::Order)
        );

        let mut row = completed_row(1);
        row.lifecycle.steps.swap(5, 6);
        assert_eq!(validate_row(&row), Err(ValidationFault::Order));

        let mut row = completed_row(1);
        row.fresh_target_authenticated = Some(false);
        assert_eq!(validate_row(&row), Err(ValidationFault::Order));

        let mut row = completed_row(1);
        row.operation_deadline_millis += 1;
        assert_eq!(validate_row(&row), Err(ValidationFault::Schema));
    }

    #[test]
    fn snapshot_validation_accepts_saturation_and_bounded_overwrite() {
        let saturated = Snapshot {
            observed_total: u64::MAX,
            status_counts: [u64::MAX, 0, 0, 0, 0, 0, 0, 0],
            first_status: Some(event(0, 1, 10)),
            last_status: Some(event(0, u64::MAX, 20)),
            callbacks_received: u64::MAX,
            callbacks_admitted: u64::MAX,
            callbacks_exited: u64::MAX,
            first_complete_nanos: 10,
            transition_count: 1,
            transitions: {
                let mut transitions = [None; TRANSITION_CAPACITY];
                transitions[0] = Some(event(0, 1, 10));
                transitions
            },
            ..snapshot(None)
        };
        assert!(validate_snapshot(&saturated).is_ok());

        let mut overwritten = snapshot(Some(0));
        overwritten.observed_total = TRANSITION_CAPACITY as u64;
        overwritten.status_counts = [8, 0, 0, 0, 8, 0, 0, 0];
        overwritten.first_status = Some(event(0, 1, 10));
        overwritten.last_status = Some(event(4, TRANSITION_CAPACITY as u64, 25));
        overwritten.callbacks_received = TRANSITION_CAPACITY as u64;
        overwritten.callbacks_admitted = TRANSITION_CAPACITY as u64;
        overwritten.callbacks_exited = TRANSITION_CAPACITY as u64;
        overwritten.transition_count =
            u32::try_from(TRANSITION_CAPACITY).expect("transition capacity fits u32");
        overwritten.transition_overwrites = 1;
        for (index, slot) in overwritten.transitions.iter_mut().enumerate() {
            *slot = Some(event(
                if index % 2 == 0 { 0 } else { 4 },
                index as u64 + 1,
                index as u64 + 10,
            ));
        }
        assert!(validate_snapshot(&overwritten).is_ok());
        overwritten.transition_count += 1;
        assert_eq!(
            validate_snapshot(&overwritten),
            Err(ValidationFault::Snapshot)
        );
    }

    #[test]
    fn required_observations_and_baseline_restoration_are_load_bearing() {
        let mut row = completed_row(1);
        row.old_snapshot = None;
        assert_eq!(validate_row(&row), Err(ValidationFault::Ownership));

        let mut row = completed_row(1);
        row.post_old_drop_resources = Some(ProcessResources::default());
        assert_eq!(validate_row(&row), Err(ValidationFault::Ownership));

        let mut row = completed_row(1);
        row.final_resources = Some(ProcessResources {
            detached_bytes: 1,
            ..ProcessResources::default()
        });
        assert_eq!(validate_row(&row), Err(ValidationFault::BaselineLeak));

        let mut row = completed_row(1);
        row.owners.mapping_readable_after_fresh_close = false;
        assert_eq!(validate_row(&row), Err(ValidationFault::Schema));
    }

    #[test]
    fn frozen_v1_report_source_remains_byte_identical() {
        let digest = Sha256::digest(include_bytes!("sck_suspension_report.rs"));
        let mut encoded = String::with_capacity(64);
        for byte in digest {
            write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }
        assert_eq!(encoded, FROZEN_V1_SHA256);
    }
}
