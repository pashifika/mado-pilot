//! Reading a package out of a directory tree the operating system owns.
//!
//! A directory is the one source that can change underneath the loader, so
//! every file's length and modification time are recorded when the tree is
//! walked and re-checked after its bytes are read. A disagreement is reported
//! as a changed source rather than repaired, because a package assembled from
//! two versions of a directory is not a package anybody asked for.
//!
//! The walk visits names in sorted order. Directory iteration order is the
//! filesystem's business and differs between the two release targets, so a
//! loader that inherited it would report different failures for the same tree.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use mado_pilot_core::Operation;

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::limits::AssetLimits;
use crate::reader::{EntryKind, EntryReader, EntryStorage, RawEntry, read_capped};

/// What the walk recorded about one file, so a later read can prove it did not
/// move underneath us.
#[derive(Debug, Clone)]
struct Snapshot {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

struct DirectoryReader {
    snapshots: Vec<Snapshot>,
}

impl EntryReader for DirectoryReader {
    fn read_entry(
        &mut self,
        index: usize,
        declared: u64,
        stage: LoadStage,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<[u8]>, AssetFault> {
        let snapshot = self
            .snapshots
            .get(index)
            .ok_or_else(|| AssetFault::new(AssetFaultKind::MissingEntry, stage))?;

        let mut file = File::open(&snapshot.path)
            .map_err(|_| AssetFault::new(AssetFaultKind::SourceUnreadable, stage))?;
        let bytes = read_capped(&mut file, declared, stage, operation).map_err(|fault| {
            // A length disagreement on a mutable source is more likely to be the
            // source moving than the loader miscounting, and saying so is the
            // more actionable diagnostic.
            if fault.kind() == AssetFaultKind::DeclaredSizeMismatch && changed(snapshot) {
                return AssetFault::new(AssetFaultKind::SourceChanged, stage);
            }
            fault
        })?;

        if changed(snapshot) {
            return Err(AssetFault::new(AssetFaultKind::SourceChanged, stage));
        }
        Ok(Arc::from(bytes))
    }
}

fn changed(snapshot: &Snapshot) -> bool {
    let Ok(metadata) = fs::symlink_metadata(&snapshot.path) else {
        return true;
    };
    metadata.len() != snapshot.len || metadata.modified().ok() != snapshot.modified
}

/// Walks `root` and returns its entry table.
///
/// # Errors
///
/// Returns [`AssetFaultKind::SourceUnreadable`] when the root cannot be walked
/// and [`AssetFaultKind::ArchiveLimit`] when the tree holds more entries or
/// more bytes than the limits admit. Both report [`LoadStage::Source`]: a
/// directory has no trailer and no central directory to stage the checks
/// against, so the walk is the earliest point either can be known.
pub(crate) fn open(
    root: &Path,
    limits: AssetLimits,
) -> Result<(Box<dyn EntryReader>, Vec<RawEntry>), AssetFault> {
    let mut walked = Walk {
        limits,
        entries: Vec::new(),
        snapshots: Vec::new(),
        total_bytes: 0,
    };
    walked.visit(root, &mut Vec::new())?;

    Ok((
        Box::new(DirectoryReader {
            snapshots: walked.snapshots,
        }),
        walked.entries,
    ))
}

struct Walk {
    limits: AssetLimits,
    entries: Vec<RawEntry>,
    snapshots: Vec<Snapshot>,
    total_bytes: u64,
}

impl Walk {
    fn visit(&mut self, directory: &Path, prefix: &mut Vec<String>) -> Result<(), AssetFault> {
        let mut children = read_sorted(directory)?;
        for (name, path) in children.drain(..) {
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| fault(AssetFaultKind::SourceUnreadable))?;
            let file_type = metadata.file_type();

            if file_type.is_dir() {
                prefix.push(name);
                self.visit(&path, prefix)?;
                prefix.pop();
                continue;
            }

            prefix.push(name);
            let recorded = prefix.join("/");
            prefix.pop();

            let kind = if file_type.is_file() {
                EntryKind::Regular
            } else {
                // Symbolic links, hard-link targets that no longer resolve to a
                // file, devices, sockets, and FIFOs all land here. None of them
                // is opened, followed, or read.
                EntryKind::Other
            };
            let declared_size = metadata.len();

            self.push(RawEntry {
                name: recorded.into_bytes(),
                kind,
                storage: EntryStorage::Accepted,
                declared_size,
                compressed_size: declared_size,
            })?;
            self.snapshots.push(Snapshot {
                path,
                len: declared_size,
                modified: metadata.modified().ok(),
            });
        }
        Ok(())
    }

    fn push(&mut self, entry: RawEntry) -> Result<(), AssetFault> {
        let count = u64::try_from(self.entries.len() + 1)
            .map_err(|_| fault(AssetFaultKind::ArithmeticOverflow))?;
        if count > u64::from(self.limits.max_entry_count()) {
            return Err(fault(AssetFaultKind::ArchiveLimit));
        }
        self.total_bytes = self
            .total_bytes
            .checked_add(entry.declared_size)
            .ok_or_else(|| fault(AssetFaultKind::ArithmeticOverflow))?;
        if self.total_bytes > self.limits.max_total_uncompressed_bytes() {
            return Err(fault(AssetFaultKind::ArchiveLimit));
        }
        self.entries.push(entry);
        Ok(())
    }
}

fn read_sorted(directory: &Path) -> Result<Vec<(String, PathBuf)>, AssetFault> {
    let listing = fs::read_dir(directory).map_err(|_| fault(AssetFaultKind::SourceUnreadable))?;
    let mut children = Vec::new();
    for child in listing {
        let child = child.map_err(|_| fault(AssetFaultKind::SourceUnreadable))?;
        // A name the platform cannot express as UTF-8 cannot be a package path,
        // and is refused as an entry rather than skipped: a package that silently
        // lost a file is worse than one that failed to load.
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| fault(AssetFaultKind::UnsafePath))?;
        children.push((name, child.path()));
    }
    children.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(children)
}

const fn fault(kind: AssetFaultKind) -> AssetFault {
    AssetFault::new(kind, LoadStage::Source)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use mado_pilot_core::{Operation, OperationContext};

    use super::{DirectoryReader, Snapshot, fs};
    use crate::fault::{AssetFaultKind, LoadStage};
    use crate::reader::EntryReader;

    /// A source changing *during* a load cannot be staged through the public
    /// loader: it is synchronous, so there is no moment between the walk and the
    /// read for a test to act in. These tests stage the same disagreement the
    /// loader would see, by handing the reader a snapshot that no longer
    /// describes the file.
    fn scratch(label: &str, content: &[u8]) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mado-pilot-assets-snapshot-{}-{label}-{unique}",
            std::process::id()
        ));
        fs::write(&path, content).expect("a writable temporary file");
        path
    }

    fn read(snapshot: Snapshot, declared: u64) -> Result<usize, AssetFaultKind> {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        let mut reader = DirectoryReader {
            snapshots: vec![snapshot],
        };
        reader
            .read_entry(0, declared, LoadStage::Expansion, &mut operation)
            .map(|bytes| bytes.len())
            .map_err(|fault| {
                assert_eq!(fault.stage(), LoadStage::Expansion);
                fault.kind()
            })
    }

    fn snapshot_of(path: &std::path::Path) -> Snapshot {
        let metadata = fs::symlink_metadata(path).expect("readable");
        Snapshot {
            path: path.to_path_buf(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }

    #[test]
    fn a_file_that_still_matches_its_snapshot_is_read() {
        let path = scratch("unchanged", b"abcd");
        let snapshot = snapshot_of(&path);

        assert_eq!(read(snapshot, 4), Ok(4));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_file_that_grew_since_the_walk_is_reported_as_a_changed_source() {
        let path = scratch("grown", b"abcdefgh");
        let mut snapshot = snapshot_of(&path);
        // The walk saw four bytes; the file now holds eight.
        snapshot.len = 4;

        assert_eq!(read(snapshot, 4), Err(AssetFaultKind::SourceChanged));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_file_edited_in_place_without_changing_length_is_still_caught() {
        let path = scratch("touched", b"abcd");
        let mut snapshot = snapshot_of(&path);
        snapshot.modified = snapshot
            .modified
            .map(|instant| instant - Duration::from_secs(60));

        assert_eq!(read(snapshot, 4), Err(AssetFaultKind::SourceChanged));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_file_removed_since_the_walk_is_reported_as_unreadable() {
        let path = scratch("removed", b"abcd");
        let snapshot = snapshot_of(&path);
        fs::remove_file(&path).expect("removable");

        assert_eq!(read(snapshot, 4), Err(AssetFaultKind::SourceUnreadable));
    }

    #[test]
    fn an_index_outside_the_snapshot_table_is_a_missing_entry() {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        let mut reader = DirectoryReader { snapshots: vec![] };

        let fault = reader
            .read_entry(0, 0, LoadStage::Expansion, &mut operation)
            .expect_err("no such entry");

        assert_eq!(fault.kind(), AssetFaultKind::MissingEntry);
    }
}
