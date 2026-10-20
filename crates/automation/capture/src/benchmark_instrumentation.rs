//! Opt-in cumulative observations of physical mapping copies.

use std::sync::atomic::{AtomicU64, Ordering};

static COPIES: AtomicU64 = AtomicU64::new(0);
static COPIED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Completed copies since process start, not logical mapping byte extents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MappingSnapshot {
    /// Completed packed-region copies.
    pub copies: u64,
    /// Bytes written by those copies.
    pub copied_bytes: u64,
}

/// Observe interval boundaries with no active mappings; counters are never reset.
#[must_use]
pub fn snapshot() -> MappingSnapshot {
    MappingSnapshot {
        copies: COPIES.load(Ordering::Acquire),
        copied_bytes: COPIED_BYTES.load(Ordering::Acquire),
    }
}

pub(crate) fn record_copy(bytes: usize) {
    COPIES.fetch_add(1, Ordering::Relaxed);
    COPIED_BYTES.fetch_add(bytes as u64, Ordering::Release);
}
