//! Typed, privacy-allowlisted report schema for ScreenCaptureKit suspension diagnosis.
//!
//! One diagnostic process emits exactly one JSON row. Every string is a fixed-length
//! source or artifact digest; all other values are enums or bounded numeric fields.
#![expect(
    missing_docs,
    reason = "public names are the fixed machine-readable schema; per-field prose would duplicate them"
)]

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};

use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION: u32 = 1;
pub const STATUS_KIND_COUNT: usize = 8;
pub const TRANSITION_CAPACITY: usize = 16;
pub const PROCESS_COUNT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionVariant {
    None,
    MappingOnly,
    FrameOnly,
    Both,
}

impl RetentionVariant {
    pub const fn retains_frame(self) -> bool {
        matches!(self, Self::FrameOnly | Self::Both)
    }

    pub const fn retains_mapping(self) -> bool {
        matches!(self, Self::MappingOnly | Self::Both)
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "mapping_only" => Some(Self::MappingOnly),
            "frame_only" => Some(Self::FrameOnly),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrainPolicy {
    Unchanged,
    DrainSampleQueue,
}

impl DrainPolicy {
    pub const fn native_value(self) -> u32 {
        match self {
            Self::Unchanged => 0,
            Self::DrainSampleQueue => 1,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unchanged" => Some(Self::Unchanged),
            "drain_sample_queue" => Some(Self::DrainSampleQueue),
            _ => None,
        }
    }
}

pub const ORDER: [(DrainPolicy, RetentionVariant); PROCESS_COUNT] = [
    (DrainPolicy::Unchanged, RetentionVariant::None),
    (DrainPolicy::Unchanged, RetentionVariant::MappingOnly),
    (DrainPolicy::Unchanged, RetentionVariant::FrameOnly),
    (DrainPolicy::Unchanged, RetentionVariant::Both),
    (DrainPolicy::DrainSampleQueue, RetentionVariant::None),
    (DrainPolicy::DrainSampleQueue, RetentionVariant::MappingOnly),
    (DrainPolicy::DrainSampleQueue, RetentionVariant::FrameOnly),
    (DrainPolicy::DrainSampleQueue, RetentionVariant::Both),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topology {
    SingleDisplay,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_revision: String,
    pub source_tree: String,
    pub executable_sha256: String,
    pub fixture_sha256: String,
    pub fixture_source_sha256: String,
    pub host: HostProfile,
    pub os: OsProfile,
    pub sdk: SdkProfile,
    pub topology: Topology,
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
    pub drain_policy: u32,
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
    pub drain_request_generation: u64,
    pub drain_completion_generation: u64,
    pub drain_requested_nanos: u64,
    pub drain_completed_nanos: u64,
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
    pub frame: bool,
    pub mapping: bool,
    pub frame_readable_after_fresh_close: bool,
    pub mapping_readable_after_fresh_close: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Setup,
    OldCapture,
    OldClose,
    FreshOpen,
    FixtureRevision,
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
    UnsupportedTopology,
    OperationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub schema_version: u32,
    pub process_index: u32,
    pub policy: DrainPolicy,
    pub variant: RetentionVariant,
    pub provenance: Provenance,
    pub baseline: ProcessResources,
    pub final_resources: ProcessResources,
    pub owners: RetainedOwners,
    pub fixture_revision: u64,
    pub first_complete_latency_nanos: Option<u64>,
    pub old_snapshot: Option<Snapshot>,
    pub fresh_snapshot: Option<Snapshot>,
    pub stage: Stage,
    pub outcome: Outcome,
}

impl Row {
    pub fn to_json_line(&self) -> Result<String, ValidationFault> {
        validate_privacy_schema(self)?;
        serde_json::to_string(self).map_err(|_| ValidationFault::Schema)
    }

    pub const fn completed(&self) -> bool {
        matches!(self.outcome, Outcome::CompleteFrame) && matches!(self.stage, Stage::Complete)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationFault {
    Schema,
    Privacy,
    Order,
    Ownership,
    Policy,
    Snapshot,
    BaselineLeak,
}

pub fn parse_json_line(line: &str) -> Result<Row, ValidationFault> {
    if line.contains(['\n', '\r']) {
        return Err(ValidationFault::Privacy);
    }
    let row: Row = serde_json::from_str(line).map_err(|_| ValidationFault::Privacy)?;
    validate_privacy_schema(&row)?;
    Ok(row)
}

pub fn validate_row(row: &Row) -> Result<(), ValidationFault> {
    validate_privacy_schema(row)?;
    let index = usize::try_from(row.process_index - 1).map_err(|_| ValidationFault::Order)?;
    if ORDER[index] != (row.policy, row.variant) {
        return Err(ValidationFault::Order);
    }
    let selected_owners = (row.variant.retains_frame(), row.variant.retains_mapping());
    if (row.owners.frame && !selected_owners.0)
        || (row.owners.mapping && !selected_owners.1)
        || (row.owners.frame_readable_after_fresh_close && !row.owners.frame)
        || (row.owners.mapping_readable_after_fresh_close && !row.owners.mapping)
    {
        return Err(ValidationFault::Ownership);
    }
    let completed_observation = matches!(
        row.outcome,
        Outcome::CompleteFrame | Outcome::ExplicitProducerState
    );
    if completed_observation
        && ((row.owners.frame, row.owners.mapping) != selected_owners
            || row.owners.frame_readable_after_fresh_close != row.owners.frame
            || row.owners.mapping_readable_after_fresh_close != row.owners.mapping)
    {
        return Err(ValidationFault::Ownership);
    }
    for snapshot in [row.old_snapshot.as_ref(), row.fresh_snapshot.as_ref()]
        .into_iter()
        .flatten()
    {
        validate_snapshot(snapshot, row.policy)?;
    }
    if row.final_resources != row.baseline {
        return Err(ValidationFault::BaselineLeak);
    }
    if completed_observation
        && let Some(snapshot) = row.old_snapshot.as_ref()
        && ((snapshot.detached_leases != 0) != row.variant.retains_frame()
            || (snapshot.detached_bytes > row.baseline.detached_bytes)
                != row.variant.retains_frame())
    {
        return Err(ValidationFault::Ownership);
    }
    match row.outcome {
        Outcome::CompleteFrame => {
            if row.stage != Stage::Complete
                || row.first_complete_latency_nanos.is_none()
                || row.old_snapshot.is_none()
                || row.fresh_snapshot.is_none()
            {
                return Err(ValidationFault::Schema);
            }
        }
        Outcome::ExplicitProducerState => {
            if row.fresh_snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.observed_total == 0 || snapshot.last_status.is_none()
            }) {
                return Err(ValidationFault::Snapshot);
            }
        }
        Outcome::UnsupportedTopology | Outcome::OperationFailed => {}
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
        if row.process_index != expected_index || ORDER[index] != (row.policy, row.variant) {
            return Err(ValidationFault::Order);
        }
        if row.provenance != *provenance {
            return Err(ValidationFault::Order);
        }
        validate_row(row)?;
    }
    Ok(())
}
fn validate_privacy_schema(row: &Row) -> Result<(), ValidationFault> {
    if row.schema_version != SCHEMA_VERSION
        || row.process_index == 0
        || usize::try_from(row.process_index).map_or(true, |index| index > PROCESS_COUNT)
        || !is_hex(&row.provenance.source_revision, 40)
        || !is_hex(&row.provenance.source_tree, 40)
        || !is_hex(&row.provenance.executable_sha256, 64)
        || !is_hex(&row.provenance.fixture_sha256, 64)
        || !is_hex(&row.provenance.fixture_source_sha256, 64)
    {
        return Err(ValidationFault::Privacy);
    }
    Ok(())
}

fn validate_snapshot(snapshot: &Snapshot, policy: DrainPolicy) -> Result<(), ValidationFault> {
    const ACTIVE_NATIVE_SLOT_MASK: u32 = 0x3f;
    if snapshot.drain_policy != policy.native_value()
        || snapshot.close_phase > 6
        || snapshot.active_native_slots & !ACTIVE_NATIVE_SLOT_MASK != 0
        || snapshot.transition_count as usize > TRANSITION_CAPACITY
        || snapshot.callbacks_received
            != snapshot
                .callbacks_admitted
                .saturating_add(snapshot.callbacks_refused)
        || (snapshot.close_phase == 6 && snapshot.callbacks_received != snapshot.callbacks_exited)
        || snapshot
            .status_counts
            .iter()
            .copied()
            .fold(0_u64, u64::saturating_add)
            != snapshot.observed_total
    {
        return Err(ValidationFault::Policy);
    }
    match (
        snapshot.observed_total,
        snapshot.first_status,
        snapshot.last_status,
    ) {
        (0, None, None) => {}
        (0, _, _) | (_, None, _) | (_, _, None) => return Err(ValidationFault::Snapshot),
        (observed, Some(first), Some(last)) => {
            if !valid_event(first, observed)
                || !valid_event(last, observed)
                || first.sequence != 1
                || last.sequence != observed
            {
                return Err(ValidationFault::Snapshot);
            }
        }
    }
    let count = snapshot.transition_count as usize;
    if snapshot.transitions[..count].iter().any(Option::is_none)
        || snapshot.transitions[count..].iter().any(Option::is_some)
    {
        return Err(ValidationFault::Snapshot);
    }
    let mut prior_sequence = None;
    for event in snapshot.transitions[..count].iter().flatten().copied() {
        if !valid_event(event, snapshot.observed_total)
            || prior_sequence.is_some_and(|prior| {
                event.sequence < prior || (event.sequence == prior && event.sequence != u64::MAX)
            })
        {
            return Err(ValidationFault::Snapshot);
        }
        prior_sequence = Some(event.sequence);
    }
    Ok(())
}

fn valid_event(event: StatusEvent, observed_total: u64) -> bool {
    (event.kind as usize) < STATUS_KIND_COUNT
        && event.sequence != 0
        && event.sequence <= observed_total
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn sha256_file(path: &std::path::Path) -> io::Result<String> {
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
    use super::*;

    const SHA1: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA256: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn snapshot(policy: DrainPolicy) -> Snapshot {
        let event = StatusEvent {
            kind: 0,
            raw_value: 0,
            sequence: 1,
            monotonic_nanos: 10,
        };
        let mut transitions = [None; TRANSITION_CAPACITY];
        transitions[0] = Some(event);
        Snapshot {
            drain_policy: policy.native_value(),
            observed_total: 1,
            status_counts: [1, 0, 0, 0, 0, 0, 0, 0],
            first_status: Some(event),
            last_status: Some(event),
            callbacks_received: 1,
            callbacks_admitted: 1,
            callbacks_exited: 1,
            transition_count: 1,
            transitions,
            ..Snapshot::default()
        }
    }

    fn row(index: usize) -> Row {
        let (policy, variant) = ORDER[index];
        let mut old_snapshot = snapshot(policy);
        if variant.retains_frame() {
            old_snapshot.detached_leases = 1;
            old_snapshot.detached_bytes = 4_096;
        }
        Row {
            schema_version: SCHEMA_VERSION,
            process_index: u32::try_from(index + 1).expect("test index fits"),
            policy,
            variant,
            provenance: Provenance {
                source_revision: SHA1.to_owned(),
                source_tree: SHA1.to_owned(),
                executable_sha256: SHA256.to_owned(),
                fixture_sha256: SHA256.to_owned(),
                fixture_source_sha256: SHA256.to_owned(),
                host: HostProfile::AppleM1Pro10c32g,
                os: OsProfile::Macos26_6_2Build25g83,
                sdk: SdkProfile::Xcode26_5,
                topology: Topology::SingleDisplay,
            },
            baseline: ProcessResources::default(),
            final_resources: ProcessResources::default(),
            owners: RetainedOwners {
                frame: variant.retains_frame(),
                mapping: variant.retains_mapping(),
                frame_readable_after_fresh_close: variant.retains_frame(),
                mapping_readable_after_fresh_close: variant.retains_mapping(),
            },
            fixture_revision: 2,
            first_complete_latency_nanos: Some(20),
            old_snapshot: Some(old_snapshot),
            fresh_snapshot: Some(snapshot(policy)),
            stage: Stage::Complete,
            outcome: Outcome::CompleteFrame,
        }
    }

    #[test]
    fn variants_encode_exact_frame_and_mapping_owners() {
        for index in 0..PROCESS_COUNT {
            assert_eq!(validate_row(&row(index)), Ok(()));
        }
    }

    #[test]
    fn policy_label_must_match_both_native_snapshots() {
        let mut candidate = row(4);
        candidate
            .fresh_snapshot
            .as_mut()
            .expect("snapshot")
            .drain_policy = 0;
        assert_eq!(validate_row(&candidate), Err(ValidationFault::Policy));
    }

    #[test]
    fn aggregate_rejects_missing_duplicate_and_reordered_rows() {
        let rows = (0..PROCESS_COUNT).map(row).collect::<Vec<_>>();
        assert_eq!(validate_aggregate(&rows), Ok(()));
        assert_eq!(validate_aggregate(&rows[..7]), Err(ValidationFault::Order));
        let mut duplicate = rows.clone();
        duplicate[7] = duplicate[6].clone();
        assert_eq!(validate_aggregate(&duplicate), Err(ValidationFault::Order));
        let mut reordered = rows;
        reordered.swap(0, 1);
        assert_eq!(validate_aggregate(&reordered), Err(ValidationFault::Order));
    }

    #[test]
    fn aggregate_rejects_mixed_provenance() {
        let mut rows = (0..PROCESS_COUNT).map(row).collect::<Vec<_>>();
        rows[PROCESS_COUNT - 1].provenance.source_tree = "f".repeat(40);
        assert_eq!(validate_aggregate(&rows), Err(ValidationFault::Order));
    }

    #[test]
    fn baseline_leak_fails_without_discarding_the_row() {
        let mut candidate = row(0);
        candidate.final_resources.detached_bytes = 1;
        assert_eq!(validate_row(&candidate), Err(ValidationFault::BaselineLeak));
        assert_eq!(candidate.process_index, 1);
        let line = candidate
            .to_json_line()
            .expect("leak row remains retainable");
        assert_eq!(
            parse_json_line(&line)
                .expect("leak row remains parseable")
                .final_resources
                .detached_bytes,
            1
        );
    }

    #[test]
    fn saturated_transition_sequences_remain_fixed_and_reportable() {
        let mut candidate = row(0);
        let snapshot = candidate.fresh_snapshot.as_mut().expect("snapshot");
        snapshot.observed_total = u64::MAX;
        snapshot.status_counts = [u64::MAX, 0, 0, 0, 0, 0, 0, 0];
        snapshot.last_status = Some(StatusEvent {
            sequence: u64::MAX,
            ..snapshot.last_status.expect("last status")
        });
        snapshot.transition_overwrites = 9;
        snapshot.transition_count =
            u32::try_from(TRANSITION_CAPACITY).expect("fixed capacity fits u32");
        for slot in &mut snapshot.transitions {
            *slot = Some(StatusEvent {
                sequence: u64::MAX,
                ..StatusEvent::default()
            });
        }
        assert_eq!(validate_row(&candidate), Ok(()));
    }

    #[test]
    fn malformed_snapshot_bounds_and_status_endpoints_are_rejected() {
        let mut candidate = row(0);
        candidate
            .fresh_snapshot
            .as_mut()
            .expect("snapshot")
            .close_phase = 7;
        assert_eq!(validate_row(&candidate), Err(ValidationFault::Policy));

        let mut candidate = row(0);
        candidate
            .fresh_snapshot
            .as_mut()
            .expect("snapshot")
            .active_native_slots = 0x40;
        assert_eq!(validate_row(&candidate), Err(ValidationFault::Policy));

        let mut candidate = row(0);
        candidate
            .fresh_snapshot
            .as_mut()
            .expect("snapshot")
            .first_status
            .as_mut()
            .expect("first status")
            .kind = u32::try_from(STATUS_KIND_COUNT).expect("status kind count fits u32");
        assert_eq!(validate_row(&candidate), Err(ValidationFault::Snapshot));

        let mut candidate = row(0);
        candidate
            .fresh_snapshot
            .as_mut()
            .expect("snapshot")
            .last_status
            .as_mut()
            .expect("last status")
            .sequence = 0;
        assert_eq!(validate_row(&candidate), Err(ValidationFault::Snapshot));

        let mut candidate = row(0);
        candidate
            .fresh_snapshot
            .as_mut()
            .expect("snapshot")
            .status_counts[0] = 2;
        assert_eq!(validate_row(&candidate), Err(ValidationFault::Policy));
    }

    #[test]
    fn unsupported_topology_and_terminal_failure_rows_are_retained_for_every_variant() {
        for index in 0..PROCESS_COUNT {
            for outcome in [Outcome::UnsupportedTopology, Outcome::OperationFailed] {
                let mut candidate = row(index);
                candidate.outcome = outcome;
                candidate.stage = Stage::Setup;
                candidate.owners = RetainedOwners::default();
                candidate.old_snapshot = None;
                candidate.fresh_snapshot = None;
                candidate.first_complete_latency_nanos = None;
                assert_eq!(validate_row(&candidate), Ok(()));
                let line = candidate.to_json_line().expect("bounded failure row");
                assert_eq!(
                    parse_json_line(&line).expect("retained row").outcome,
                    outcome
                );
            }
        }
    }

    #[test]
    fn arbitrary_fields_and_malformed_hashes_are_rejected() {
        let candidate = row(0);
        let mut value = serde_json::to_value(&candidate).expect("typed row serializes");
        value
            .as_object_mut()
            .expect("row object")
            .insert("window_title".to_owned(), serde_json::json!("private"));
        assert_eq!(
            parse_json_line(&serde_json::to_string(&value).expect("json")),
            Err(ValidationFault::Privacy)
        );
        let mut candidate = candidate;
        candidate.provenance.fixture_sha256 = "/Users/private/fixture".to_owned();
        assert_eq!(validate_row(&candidate), Err(ValidationFault::Privacy));
    }
}
