//! Private, feature-gated ScreenCaptureKit producer diagnostics.
//!
//! The facade owns concrete capture sessions behind platform-neutral trait
//! objects. Qualification therefore reaches a session through this weak registry,
//! keyed by its public stream identity. The registry never extends a session or
//! native resource lifetime. Snapshotting is a cold operation and all returned
//! values are fixed-size numeric state; no target identity, title, image, or OCR
//! payload crosses this seam.

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use mado_pilot_capture::CaptureSession;
use mado_pilot_core::StreamId;

use crate::native::NativeSession;
use crate::shim::{
    self, SCK_STATUS_BLANK, SCK_STATUS_COMPLETE, SCK_STATUS_IDLE, SCK_STATUS_MISSING,
    SCK_STATUS_STARTED, SCK_STATUS_STOPPED, SCK_STATUS_SUSPENDED, SCK_STATUS_UNKNOWN,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SampleQueueDrainPolicy {
    Unchanged = 0,
    DrainSampleQueue = 1,
}

static SAMPLE_QUEUE_DRAIN_POLICY: AtomicU32 =
    AtomicU32::new(SampleQueueDrainPolicy::Unchanged as u32);

pub fn set_sample_queue_drain_policy(policy: SampleQueueDrainPolicy) {
    SAMPLE_QUEUE_DRAIN_POLICY.store(policy as u32, Ordering::Release);
}

pub(crate) fn sample_queue_drain_policy() -> u32 {
    SAMPLE_QUEUE_DRAIN_POLICY.load(Ordering::Acquire)
}
pub const STATUS_KIND_COUNT: usize = 8;
pub const TRANSITION_CAPACITY: usize = 16;

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
            drain_policy: snapshot.drain_policy,
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
            drain_request_generation: snapshot.drain_request_generation,
            drain_completion_generation: snapshot.drain_completion_generation,
            drain_requested_nanos: snapshot.drain_requested_nanos,
            drain_completed_nanos: snapshot.drain_completed_nanos,
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

pub fn session_snapshot(stream: StreamId) -> Result<Snapshot, SnapshotError> {
    let session = {
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
        matching.ok_or(SnapshotError::SessionNotFound)?
    };
    session
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
