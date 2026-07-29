//! Where a package's bytes come from.
//!
//! The three sources are strategies behind one pipeline, not three loaders. A
//! source's only job is to answer two questions — what entries do you record,
//! and give me the bytes of entry *n* — and every rule about what those answers
//! are allowed to be is applied once, afterwards, for all of them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mado_pilot_core::{Operation, OperationContext};

use crate::fault::{AssetFault, LoadStage};
use crate::reader::CHUNK_BYTES;

/// One entry a caller supplies from memory.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    name: String,
    content: Arc<[u8]>,
}

impl MemoryEntry {
    /// Builds an entry from an owned name and owned content.
    ///
    /// The content is reference-counted rather than borrowed, so a committed
    /// package stays valid after the caller drops or overwrites whatever it
    /// built the entry from.
    #[must_use]
    pub fn new(name: impl Into<String>, content: impl Into<Arc<[u8]>>) -> Self {
        Self {
            name: name.into(),
            content: content.into(),
        }
    }

    /// Returns the recorded name, before normalization.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the entry content.
    #[must_use]
    pub fn content(&self) -> &Arc<[u8]> {
        &self.content
    }
}

/// A package described entirely by caller-owned memory.
///
/// Entries are kept in the order they were added and are never de-duplicated
/// here. Two entries whose names normalize to the same package path are a
/// package the loader must refuse, and a builder that quietly dropped one of
/// them would turn that refusal into a silent choice of which entry wins.
#[derive(Debug, Clone, Default)]
pub struct MemoryPackage {
    entries: Vec<MemoryEntry>,
}

impl MemoryPackage {
    /// Returns an empty package description.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an entry.
    #[must_use]
    pub fn with_entry(mut self, name: impl Into<String>, content: impl Into<Arc<[u8]>>) -> Self {
        self.entries.push(MemoryEntry::new(name, content));
        self
    }

    /// Returns the entries, in the order they were added.
    #[must_use]
    pub fn entries(&self) -> &[MemoryEntry] {
        &self.entries
    }
}

/// A package source.
///
/// This enum is `#[non_exhaustive]`: later phases may add sources, and a caller
/// must keep a fallback arm.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PackageSource {
    /// A local directory whose tree is the package.
    Directory(PathBuf),
    /// A description held entirely in caller-owned memory.
    Memory(MemoryPackage),
    /// A local ZIP archive read from the filesystem.
    ArchiveFile(PathBuf),
    /// A local ZIP archive already in memory.
    ArchiveBytes(Arc<[u8]>),
}

impl PackageSource {
    /// Names a directory source.
    #[must_use]
    pub fn directory(root: impl AsRef<Path>) -> Self {
        PackageSource::Directory(root.as_ref().to_path_buf())
    }

    /// Names an archive file source.
    #[must_use]
    pub fn archive_file(path: impl AsRef<Path>) -> Self {
        PackageSource::ArchiveFile(path.as_ref().to_path_buf())
    }

    /// Names an archive source already in memory.
    #[must_use]
    pub fn archive_bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        PackageSource::ArchiveBytes(bytes.into())
    }

    /// Copies a borrowed archive into an owned source, under `context`.
    ///
    /// [`PackageSource::archive_bytes`] is for a caller that already owns its
    /// bytes, and it copies nothing when handed an `Arc<[u8]>`. This is for a
    /// caller that does not own them: a boundary holding a borrowed view for the
    /// duration of one call has to take a copy, that copy is as large as the
    /// archive its limits admit, and work that large is work the operation should
    /// be able to interrupt. The context is checked before the first chunk and
    /// between chunks, so cancellation or an expiry that lands during the copy
    /// stops it instead of being noticed once the loader already holds the result.
    ///
    /// One allocation and one copy. The owned representation is filled in place
    /// rather than assembled in a `Vec` and converted, because a conversion holds
    /// both buffers at once and an archive at the source ceiling would double the
    /// peak this copy is supposed to bound. What that costs is stated rather than
    /// hidden: `Arc`'s allocation is infallible on stable Rust, so a host that
    /// cannot satisfy it aborts instead of receiving a fault. The alternative —
    /// a fallible `Vec` reservation and then a second copy into the `Arc` — trades
    /// a reportable failure for a doubled peak, which makes the failure more
    /// likely; and the surrounding loader allocates its own entry buffers
    /// infallibly too, so a fallible reservation here would be a local exception
    /// rather than a property a caller could rely on.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome at [`LoadStage::Source`] when
    /// cancellation or the deadline wins before or during the copy.
    pub fn copy_archive_bytes(
        bytes: &[u8],
        context: &OperationContext,
    ) -> Result<Self, AssetFault> {
        let mut operation = Operation::admit(context)
            .map_err(|interruption| AssetFault::interrupted(interruption, LoadStage::Source))?;

        let mut buffer = Arc::new_uninit_slice(bytes.len());
        let slots =
            Arc::get_mut(&mut buffer).expect("a buffer allocated here is shared with nothing");
        for (uninitialized, chunk) in slots.chunks_mut(CHUNK_BYTES).zip(bytes.chunks(CHUNK_BYTES)) {
            operation
                .checkpoint()
                .map_err(|interruption| AssetFault::interrupted(interruption, LoadStage::Source))?;
            uninitialized.write_copy_of_slice(chunk);
        }

        // SAFETY: the two chunk iterators walk one length in the same steps, so
        // the loop wrote every slot the buffer holds. An interruption returns
        // above instead, dropping the partially written buffer without reading
        // it — `MaybeUninit` is what makes that drop sound.
        Ok(PackageSource::ArchiveBytes(unsafe { buffer.assume_init() }))
    }

    /// Names a memory source.
    #[must_use]
    pub fn memory(package: MemoryPackage) -> Self {
        PackageSource::Memory(package)
    }

    /// Reports whether this source is an archive.
    ///
    /// Archive structure carries limits a directory tree cannot express: an
    /// entry count recorded in a trailer, and an expansion ratio.
    #[must_use]
    pub const fn is_archive(&self) -> bool {
        matches!(
            self,
            PackageSource::ArchiveFile(_) | PackageSource::ArchiveBytes(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryPackage, PackageSource};

    #[test]
    fn a_memory_package_keeps_duplicates_for_the_loader_to_refuse() {
        let package = MemoryPackage::new()
            .with_entry("templates/button.png", b"first".to_vec())
            .with_entry("./templates//button.png", b"second".to_vec());

        assert_eq!(package.entries().len(), 2);
        assert_eq!(package.entries()[0].content().as_ref(), b"first");
    }

    #[test]
    fn only_archive_sources_report_as_archives() {
        assert!(PackageSource::archive_file("p.zip").is_archive());
        assert!(PackageSource::archive_bytes(Vec::new()).is_archive());
        assert!(!PackageSource::directory("p").is_archive());
        assert!(!PackageSource::memory(MemoryPackage::new()).is_archive());
    }
}
