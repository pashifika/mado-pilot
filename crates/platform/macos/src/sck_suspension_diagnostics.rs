//! Private, feature-gated ScreenCaptureKit producer diagnostics.
//!
//! The facade owns concrete capture sessions behind platform-neutral trait
//! objects. Qualification reaches a live Rust session through a weak registry
//! keyed by its public stream identity. The registry never extends a session
//! lifetime. An acquired [`SessionObserver`] keeps only a closed native
//! allocation's bookkeeping alive; it retains no ScreenCaptureKit object and
//! does not contribute to structural session ownership. Snapshotting is cold,
//! bounded, and identifier-free.

use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use mado_pilot_capture::CaptureSession;
use mado_pilot_core::StreamId;

use crate::native::NativeSession;
use crate::shim::{
    self, SCK_STATUS_BLANK, SCK_STATUS_COMPLETE, SCK_STATUS_IDLE, SCK_STATUS_MISSING,
    SCK_STATUS_STARTED, SCK_STATUS_STOPPED, SCK_STATUS_SUSPENDED, SCK_STATUS_UNKNOWN,
};

pub const STATUS_KIND_COUNT: usize = 8;
pub const TRANSITION_CAPACITY: usize = 16;
pub const DISPLAY_CAPACITY: usize = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayMode {
    pub logical_width: u32,
    pub logical_height: u32,
    pub refresh_millihertz: u32,
    pub built_in: bool,
    pub mirrored: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayTopology {
    pub display_count: u32,
    pub modes: [Option<DisplayMode>; DISPLAY_CAPACITY],
}

static SESSIONS: LazyLock<Mutex<Vec<Weak<NativeSession>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static LAST_OPEN_FAILURE: LazyLock<Mutex<Option<Snapshot>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum StatusKind {
    Complete = SCK_STATUS_COMPLETE,
    Idle = SCK_STATUS_IDLE,
    Blank = SCK_STATUS_BLANK,
    Started = SCK_STATUS_STARTED,
    Suspended = SCK_STATUS_SUSPENDED,
    Stopped = SCK_STATUS_STOPPED,
    Missing = SCK_STATUS_MISSING,
    Unknown = SCK_STATUS_UNKNOWN,
}

impl StatusKind {
    fn from_raw(raw: u32) -> Self {
        match raw {
            SCK_STATUS_COMPLETE => Self::Complete,
            SCK_STATUS_IDLE => Self::Idle,
            SCK_STATUS_BLANK => Self::Blank,
            SCK_STATUS_STARTED => Self::Started,
            SCK_STATUS_SUSPENDED => Self::Suspended,
            SCK_STATUS_STOPPED => Self::Stopped,
            SCK_STATUS_MISSING => Self::Missing,
            SCK_STATUS_UNKNOWN => Self::Unknown,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusEvent {
    pub kind: StatusKind,
    pub raw_value: i64,
    pub sequence: u64,
    pub monotonic_nanos: u64,
}

impl StatusEvent {
    fn from_native(event: shim::SckDiagnosticsStatusEvent) -> Option<Self> {
        (event.struct_size != 0).then(|| Self {
            kind: StatusKind::from_raw(event.kind),
            raw_value: event.raw_value,
            sequence: event.sequence,
            monotonic_nanos: event.monotonic_nanos,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub transition_count: usize,
    pub transitions: [Option<StatusEvent>; TRANSITION_CAPACITY],
}

impl Snapshot {
    #[must_use]
    pub fn status_count(&self, kind: StatusKind) -> u64 {
        self.status_counts[kind as usize]
    }

    fn from_native(snapshot: shim::SckDiagnosticsSnapshot) -> Self {
        let transition_count = usize::try_from(snapshot.transition_count)
            .unwrap_or(TRANSITION_CAPACITY)
            .min(TRANSITION_CAPACITY);
        let start =
            usize::try_from(snapshot.transition_start).unwrap_or_default() % TRANSITION_CAPACITY;
        let mut transitions = [None; TRANSITION_CAPACITY];
        for (destination, slot) in transitions.iter_mut().take(transition_count).enumerate() {
            let source = (start + destination) % TRANSITION_CAPACITY;
            *slot = StatusEvent::from_native(snapshot.transitions[source]);
        }
        Self {
            close_phase: snapshot.close_phase,
            active_native_slots: snapshot.active_native_slots,
            observed_total: snapshot.observed_total,
            status_counts: snapshot.status_counts,
            first_status: StatusEvent::from_native(snapshot.first_status),
            last_status: StatusEvent::from_native(snapshot.last_status),
            callbacks_received: snapshot.callbacks_received,
            callbacks_admitted: snapshot.callbacks_admitted,
            callbacks_refused: snapshot.callbacks_refused,
            callbacks_exited: snapshot.callbacks_exited,
            stream_start_completed_nanos: snapshot.stream_start_completed_nanos,
            stream_stop_requested_nanos: snapshot.stream_stop_requested_nanos,
            stream_stop_completed_nanos: snapshot.stream_stop_completed_nanos,
            callback_admission_stopped_nanos: snapshot.callback_admission_stopped_nanos,
            callback_fence_completed_nanos: snapshot.callback_fence_completed_nanos,
            close_completed_nanos: snapshot.close_completed_nanos,
            first_complete_nanos: snapshot.first_complete_nanos,
            session_references: snapshot.session_references,
            detached_leases: snapshot.detached_leases,
            native_objects: snapshot.native_objects,
            detached_bytes: snapshot.detached_bytes,
            transition_overwrites: snapshot.transition_overwrites,
            transition_count,
            transitions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessResources {
    pub native_objects: u64,
    pub detached_bytes: u64,
    pub live_sessions: u64,
    pub callbacks_in_flight: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    SessionNotFound,
    AmbiguousSession,
    NativeFailure,
}

pub struct SessionObserver {
    observer: shim::SckDiagnosticsObserver,
}

impl fmt::Debug for SessionObserver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SessionObserver").finish()
    }
}

impl SessionObserver {
    pub fn snapshot(&self) -> Result<Snapshot, SnapshotError> {
        self.native_snapshot().map(Snapshot::from_native)
    }

    pub(crate) fn native_snapshot(&self) -> Result<shim::SckDiagnosticsSnapshot, SnapshotError> {
        self.observer
            .snapshot()
            .map_err(|_| SnapshotError::NativeFailure)
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::SessionNotFound => "diagnostic session not found",
            Self::AmbiguousSession => "diagnostic stream identity is ambiguous",
            Self::NativeFailure => "native diagnostic snapshot failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SnapshotError {}

pub(crate) fn register_session(session: &Arc<NativeSession>) {
    let mut sessions = SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    sessions.retain(|candidate| candidate.strong_count() != 0);
    sessions.push(Arc::downgrade(session));
}

pub(crate) fn clear_open_failure() {
    *LAST_OPEN_FAILURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

pub(crate) fn record_open_failure(session: &NativeSession) {
    let snapshot = session
        .sck_diagnostics_snapshot()
        .ok()
        .map(Snapshot::from_native);
    *LAST_OPEN_FAILURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot;
}

pub fn take_open_failure_snapshot() -> Option<Snapshot> {
    LAST_OPEN_FAILURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

fn find_session(stream: StreamId) -> Result<Arc<NativeSession>, SnapshotError> {
    let mut sessions = SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    sessions.retain(|candidate| candidate.strong_count() != 0);
    let mut matching = None;
    for candidate in sessions.iter().filter_map(Weak::upgrade) {
        if candidate.description().stream() != stream {
            continue;
        }
        if matching.is_some() {
            return Err(SnapshotError::AmbiguousSession);
        }
        matching = Some(candidate);
    }
    matching.ok_or(SnapshotError::SessionNotFound)
}

pub fn session_observer(stream: StreamId) -> Result<SessionObserver, SnapshotError> {
    find_session(stream)?
        .sck_diagnostics_observer()
        .map(|observer| SessionObserver { observer })
        .map_err(|_| SnapshotError::NativeFailure)
}

pub fn session_snapshot(stream: StreamId) -> Result<Snapshot, SnapshotError> {
    find_session(stream)?
        .sck_diagnostics_snapshot()
        .map(Snapshot::from_native)
        .map_err(|_| SnapshotError::NativeFailure)
}

pub fn process_resources() -> Result<ProcessResources, SnapshotError> {
    shim::sck_diagnostics_process_resources()
        .map(
            |(native_objects, detached_bytes, live_sessions, callbacks_in_flight)| {
                ProcessResources {
                    native_objects,
                    detached_bytes,
                    live_sessions,
                    callbacks_in_flight,
                }
            },
        )
        .map_err(|_| SnapshotError::NativeFailure)
}

pub fn display_topology() -> Result<DisplayTopology, SnapshotError> {
    let native =
        shim::sck_diagnostics_display_topology().map_err(|_| SnapshotError::NativeFailure)?;
    display_topology_from_native(native)
}

fn display_topology_from_native(
    native: shim::SckDiagnosticsDisplayTopology,
) -> Result<DisplayTopology, SnapshotError> {
    let mode_count =
        usize::try_from(native.mode_count).map_err(|_| SnapshotError::NativeFailure)?;
    let expected_count = usize::try_from(native.display_count)
        .unwrap_or(usize::MAX)
        .min(DISPLAY_CAPACITY);
    if mode_count > DISPLAY_CAPACITY || mode_count != expected_count {
        return Err(SnapshotError::NativeFailure);
    }
    let mut modes = [None; DISPLAY_CAPACITY];
    for (destination, source) in modes.iter_mut().zip(native.modes).take(mode_count) {
        if source.logical_width == 0
            || source.logical_height == 0
            || source.refresh_millihertz > 1_000_000
            || source.built_in > 1
            || source.mirrored > 1
        {
            return Err(SnapshotError::NativeFailure);
        }
        *destination = Some(DisplayMode {
            logical_width: source.logical_width,
            logical_height: source.logical_height,
            refresh_millihertz: source.refresh_millihertz,
            built_in: source.built_in != 0,
            mirrored: source.mirrored != 0,
        });
    }
    Ok(DisplayTopology {
        display_count: native.display_count,
        modes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_mode() -> shim::SckDiagnosticsDisplayMode {
        let mut mode = shim::SckDiagnosticsDisplayMode::default();
        mode.logical_width = 1512;
        mode.logical_height = 982;
        mode.refresh_millihertz = 120_000;
        mode.built_in = 1;
        mode
    }

    fn native_topology() -> shim::SckDiagnosticsDisplayTopology {
        let mut topology = shim::SckDiagnosticsDisplayTopology::requested();
        topology.display_count = 1;
        topology.mode_count = 1;
        topology.modes[0] = native_mode();
        topology
    }

    #[test]
    fn display_topology_rejects_unsupported_refresh_boolean_and_count_values() {
        assert!(display_topology_from_native(native_topology()).is_ok());

        let mut invalid = native_topology();
        invalid.modes[0].refresh_millihertz = 1_000_001;
        assert_eq!(
            display_topology_from_native(invalid),
            Err(SnapshotError::NativeFailure)
        );
        let mut invalid = native_topology();
        invalid.modes[0].mirrored = 2;
        assert_eq!(
            display_topology_from_native(invalid),
            Err(SnapshotError::NativeFailure)
        );
        let mut invalid = native_topology();
        invalid.display_count = 2;
        assert_eq!(
            display_topology_from_native(invalid),
            Err(SnapshotError::NativeFailure)
        );
    }
}
