//! Reading a package the caller already holds.
//!
//! A memory source cannot change while it is being read and has no container
//! metadata to disagree with, so it is the shortest path through the pipeline —
//! but not a shorter set of rules. The same names are normalized, the same
//! duplicates refused, and the same manifest parsed, which is what makes a
//! memory package and an equivalent directory package the same package.

use std::sync::Arc;

use mado_pilot_core::Operation;

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::limits::AssetLimits;
use crate::reader::{EntryKind, EntryReader, EntryStorage, RawEntry};
use crate::source::MemoryPackage;

struct MemoryReader {
    contents: Vec<Arc<[u8]>>,
}

impl EntryReader for MemoryReader {
    fn read_entry(
        &mut self,
        index: usize,
        declared: u64,
        stage: LoadStage,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<[u8]>, AssetFault> {
        operation
            .checkpoint()
            .map_err(|interruption| AssetFault::interrupted(interruption, stage))?;

        let content = self
            .contents
            .get(index)
            .ok_or_else(|| AssetFault::new(AssetFaultKind::MissingEntry, stage))?;
        let length = u64::try_from(content.len())
            .map_err(|_| AssetFault::new(AssetFaultKind::ArithmeticOverflow, stage))?;
        if length != declared {
            return Err(AssetFault::new(AssetFaultKind::DeclaredSizeMismatch, stage));
        }
        Ok(Arc::clone(content))
    }
}

/// Returns the entry table a caller supplied from memory.
///
/// # Errors
///
/// Returns [`AssetFaultKind::ArchiveLimit`] at [`LoadStage::Source`] when the
/// description holds more entries or more bytes than the limits admit.
pub(crate) fn open(
    package: &MemoryPackage,
    limits: AssetLimits,
) -> Result<(Box<dyn EntryReader>, Vec<RawEntry>), AssetFault> {
    let count = u64::try_from(package.entries().len())
        .map_err(|_| fault(AssetFaultKind::ArithmeticOverflow))?;
    if count > u64::from(limits.max_entry_count()) {
        return Err(fault(AssetFaultKind::ArchiveLimit));
    }

    let mut entries = Vec::with_capacity(package.entries().len());
    let mut contents = Vec::with_capacity(package.entries().len());
    let mut total_bytes: u64 = 0;

    for entry in package.entries() {
        let declared_size = u64::try_from(entry.content().len())
            .map_err(|_| fault(AssetFaultKind::ArithmeticOverflow))?;
        total_bytes = total_bytes
            .checked_add(declared_size)
            .ok_or_else(|| fault(AssetFaultKind::ArithmeticOverflow))?;
        if total_bytes > limits.max_total_uncompressed_bytes() {
            return Err(fault(AssetFaultKind::ArchiveLimit));
        }

        entries.push(RawEntry {
            name: entry.name().as_bytes().to_vec(),
            kind: EntryKind::Regular,
            storage: EntryStorage::Accepted,
            declared_size,
            compressed_size: declared_size,
        });
        contents.push(Arc::clone(entry.content()));
    }

    Ok((Box::new(MemoryReader { contents }), entries))
}

const fn fault(kind: AssetFaultKind) -> AssetFault {
    AssetFault::new(kind, LoadStage::Source)
}
