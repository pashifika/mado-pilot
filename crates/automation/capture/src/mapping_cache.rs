//! Optional reuse of live cropped or channel-converted pixels.

use std::sync::{Arc, Mutex, Weak};

use mado_pilot_core::PixelRect;

use crate::descriptor::FrameDescriptor;
use crate::storage::CpuPixels;

// Inline metadata avoids a separate allocation or lazy-initialization lock.
#[derive(Default)]
pub(crate) struct MappingCache {
    entries: Mutex<[Option<Entry>; 4]>,
}

struct Entry {
    region: PixelRect,
    descriptor: FrameDescriptor,
    pixels: Weak<CpuPixels>,
}

impl MappingCache {
    pub(crate) fn find(
        &self,
        region: PixelRect,
        descriptor: FrameDescriptor,
    ) -> Option<Arc<CpuPixels>> {
        let mut entries = self.entries.try_lock().ok()?;
        let index = entries.iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.region == region && entry.descriptor == descriptor)
        })?;
        let pixels = entries[index].as_ref()?.pixels.upgrade();
        if pixels.is_some() {
            entries[..=index].rotate_right(1);
        }
        drop(entries);
        pixels
    }

    // The caller checks operation authority immediately before publication.
    pub(crate) fn publish(
        &self,
        region: PixelRect,
        descriptor: FrameDescriptor,
        pixels: Arc<CpuPixels>,
    ) -> Arc<CpuPixels> {
        let entry = Entry {
            region,
            descriptor,
            pixels: Arc::downgrade(&pixels),
        };
        let (winner, retired) = {
            let Some(mut entries) = self.entries.try_lock().ok() else {
                return pixels;
            };
            let matching = entries.iter().position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.region == region && entry.descriptor == descriptor)
            });
            let winner = matching.and_then(|index| entries[index].as_ref()?.pixels.upgrade());
            if let (Some(index), Some(winner)) = (matching, winner) {
                entries[..=index].rotate_right(1);
                (Some(winner), None)
            } else {
                let index = matching.unwrap_or_else(|| {
                    entries
                        .iter()
                        .position(|entry| {
                            entry
                                .as_ref()
                                .is_none_or(|entry| entry.pixels.strong_count() == 0)
                        })
                        .unwrap_or(entries.len() - 1)
                });
                let retired = entries[index].replace(entry);
                entries[..=index].rotate_right(1);
                (None, retired)
            }
        };
        // Even weak-owner deallocation and losing pixel retainers run unlocked.
        drop(retired);
        winner.unwrap_or(pixels)
    }
}
