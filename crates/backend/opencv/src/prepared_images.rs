//! Nonblocking reuse of owned BGR pixels; no frame or mapping owner is retained.

use std::fmt;
#[cfg(feature = "benchmark-instrumentation")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mado_pilot_capture::frame::FrameInstance;
use mado_pilot_capture::{CpuMapping, FrameDescriptor};
use mado_pilot_core::{FrameStamp, PixelRect};
use opencv::core::{Mat, MatTraitConst};

const CAPACITY: usize = 2;
pub(crate) const BYTE_LIMIT: usize = 33_554_432;

pub(crate) struct CachedImage {
    matrix: Mat,
    // Fields drop in declaration order: native pixels precede the release gauge.
    payload: Payload,
}

struct Payload {
    bytes: usize,
    #[cfg(feature = "benchmark-instrumentation")]
    evicted: AtomicBool,
}

#[cfg(feature = "benchmark-instrumentation")]
impl Payload {
    fn retire(&self) {
        if !self.evicted.swap(true, Ordering::AcqRel) {
            crate::benchmark_instrumentation::retire_bgr(self.bytes);
        }
    }
}

#[cfg(feature = "benchmark-instrumentation")]
impl Drop for Payload {
    fn drop(&mut self) {
        if *self.evicted.get_mut() {
            crate::benchmark_instrumentation::release_evicted_bgr(self.bytes);
        }
    }
}

pub(crate) enum PreparedRegion {
    Shared(Arc<CachedImage>),
    Owned(Mat),
}

impl PreparedRegion {
    pub(crate) fn new(matrix: Mat, cacheable: bool) -> Self {
        if !cacheable {
            return Self::Owned(matrix);
        }
        let bytes = matrix.total().saturating_mul(3);
        if bytes <= BYTE_LIMIT {
            Self::Shared(Arc::new(CachedImage {
                matrix,
                payload: Payload {
                    bytes,
                    #[cfg(feature = "benchmark-instrumentation")]
                    evicted: AtomicBool::new(false),
                },
            }))
        } else {
            Self::Owned(matrix)
        }
    }

    pub(crate) fn matrix(&self) -> &Mat {
        match self {
            Self::Shared(image) => &image.matrix,
            Self::Owned(matrix) => matrix,
        }
    }
}

pub(crate) enum Lookup {
    Hit(Arc<CachedImage>),
    Vacant,
    Bypass,
}

struct Key {
    source: FrameInstance,
    stamp: FrameStamp,
    region: PixelRect,
    descriptor: FrameDescriptor,
}

impl Key {
    fn new(pixels: &CpuMapping) -> Self {
        Self {
            source: pixels.source_instance().clone(),
            stamp: pixels.stamp(),
            region: pixels.region(),
            descriptor: pixels.descriptor(),
        }
    }

    fn matches(&self, pixels: &CpuMapping) -> bool {
        self.source.eq(pixels.source_instance())
            && self.stamp == pixels.stamp()
            && self.region == pixels.region()
            && self.descriptor == pixels.descriptor()
    }
}

struct Entry {
    key: Key,
    image: Arc<CachedImage>,
}

struct State {
    entries: [Option<Entry>; CAPACITY],
    bytes: usize,
}

impl State {
    fn retire(&mut self, index: usize, retired: &mut [Option<Entry>; CAPACITY]) {
        if let Some(entry) = self.entries[index].take() {
            self.bytes -= entry.image.payload.bytes;
            #[cfg(feature = "benchmark-instrumentation")]
            entry.image.payload.retire();
            retired[index] = Some(entry);
        }
    }

    fn prune(&mut self, retired: &mut [Option<Entry>; CAPACITY]) {
        for index in 0..CAPACITY {
            if self.entries[index]
                .as_ref()
                .is_some_and(|entry| !entry.key.source.is_alive())
            {
                self.retire(index, retired);
            }
        }
    }

    fn get(&mut self, pixels: &CpuMapping) -> Option<Arc<CachedImage>> {
        let index = self.entries.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.key.matches(pixels))
        })?;
        self.entries.swap(0, index);
        self.entries[0]
            .as_ref()
            .map(|entry| Arc::clone(&entry.image))
    }
}

pub(crate) struct PreparedImages {
    state: Mutex<State>,
}

impl PreparedImages {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                entries: [const { None }; CAPACITY],
                bytes: 0,
            }),
        }
    }

    pub(crate) fn lookup(&self, pixels: &CpuMapping) -> Lookup {
        let mut retired = [const { None }; CAPACITY];
        let result = match self.state.try_lock() {
            Ok(mut state) => {
                state.prune(&mut retired);
                match state.get(pixels) {
                    Some(image) => Lookup::Hit(image),
                    None if pixels.source_instance().is_alive() => Lookup::Vacant,
                    None => Lookup::Bypass,
                }
            }
            Err(_) => Lookup::Bypass,
        };
        drop(retired);
        result
    }

    /// Publishes only a fresh conversion from `pixels`, after its caller checkpoint.
    pub(crate) fn publish(&self, pixels: &CpuMapping, candidate: PreparedRegion) -> PreparedRegion {
        let PreparedRegion::Shared(image) = &candidate else {
            return candidate;
        };
        if !pixels.source_instance().is_alive() {
            return candidate;
        }
        let key = Key::new(pixels);
        let mut retired = [const { None }; CAPACITY];
        let winner = match self.state.try_lock() {
            Ok(mut state) => {
                state.prune(&mut retired);
                if let Some(existing) = state.get(pixels) {
                    Some(existing)
                } else {
                    while state.bytes + image.payload.bytes > BYTE_LIMIT
                        || state.entries.iter().all(Option::is_some)
                    {
                        let last = if state.entries[1].is_some() { 1 } else { 0 };
                        state.retire(last, &mut retired);
                    }
                    let empty = usize::from(state.entries[0].is_some());
                    state.entries.swap(0, empty);
                    state.bytes += image.payload.bytes;
                    #[cfg(feature = "benchmark-instrumentation")]
                    crate::benchmark_instrumentation::retain_bgr(image.payload.bytes);
                    state.entries[0] = Some(Entry {
                        key,
                        image: Arc::clone(image),
                    });
                    None
                }
            }
            Err(_) => None,
        };
        drop(retired);
        match winner {
            Some(image) => PreparedRegion::Shared(image),
            None => candidate,
        }
    }
}

impl fmt::Debug for PreparedImages {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let summary = self
            .state
            .try_lock()
            .ok()
            .map(|state| (state.entries.iter().flatten().count(), state.bytes));
        let mut debug = formatter.debug_struct("PreparedImages");
        if let Some((entries, bytes)) = summary {
            debug
                .field("entries", &entries)
                .field("retained_bytes", &bytes);
        }
        debug.finish_non_exhaustive()
    }
}

#[cfg(feature = "benchmark-instrumentation")]
impl Drop for PreparedImages {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in state.entries.iter().flatten() {
            entry.image.payload.retire();
        }
    }
}

#[cfg(test)]
mod tests;
