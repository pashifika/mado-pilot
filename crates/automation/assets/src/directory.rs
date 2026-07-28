//! Reading a package out of a directory tree the operating system owns.
//!
//! A directory is the one source that can change underneath the loader. The
//! walk therefore retains the root, enumerates and opens Unix children relative
//! to retained directory handles, and pins Windows paths by denying write/delete
//! sharing. Regular-file handles remain retained through validation. Later path
//! replacement cannot redirect a read, while in-place mutation of an opened file
//! is detected before and after its bytes are consumed.
//!
//! The walk visits names in sorted order. Directory iteration order is the
//! filesystem's business and differs between the two release targets, so a
//! loader that inherited it would report different failures for the same tree.

use std::path::Path;
use std::sync::Arc;

use mado_pilot_core::Operation;

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::filesystem::{self, NodeKind, OpenedFile, OpenedNode};
use crate::limits::AssetLimits;
use crate::reader::{EntryKind, EntryReader, EntryStorage, RawEntry, read_capped};

enum Snapshot {
    Regular(OpenedFile),
    Unsupported,
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
            .get_mut(index)
            .ok_or_else(|| AssetFault::new(AssetFaultKind::MissingEntry, stage))?;
        let Snapshot::Regular(snapshot) = snapshot else {
            return Err(AssetFault::new(AssetFaultKind::UnsupportedEntryType, stage));
        };

        if snapshot.changed() {
            return Err(AssetFault::new(AssetFaultKind::SourceChanged, stage));
        }
        let bytes =
            read_capped(snapshot.file_mut(), declared, stage, operation).map_err(|fault| {
                if fault.kind() == AssetFaultKind::DeclaredSizeMismatch && snapshot.changed() {
                    return AssetFault::new(AssetFaultKind::SourceChanged, stage);
                }
                fault
            })?;
        if snapshot.changed() {
            return Err(AssetFault::new(AssetFaultKind::SourceChanged, stage));
        }
        Ok(Arc::from(bytes))
    }
}

/// Walks `root` and returns its entry table.
///
/// # Errors
///
/// Returns [`AssetFaultKind::SourceUnreadable`] when the root cannot be walked,
/// [`AssetFaultKind::SourceChanged`] when stable identity cannot be established,
/// and [`AssetFaultKind::ArchiveLimit`] when the tree holds more entries or
/// bytes than the limits admit. All report [`LoadStage::Source`].
pub(crate) fn open(
    root: &Path,
    limits: AssetLimits,
    operation: &mut Operation<'_>,
) -> Result<(Box<dyn EntryReader>, Vec<RawEntry>), AssetFault> {
    let root_node = filesystem::open_stable(root, LoadStage::Source, operation)?;
    if root_node.kind() != NodeKind::Directory {
        return Err(fault(AssetFaultKind::SourceUnreadable));
    }
    let mut walked = Walk {
        limits,
        operation,
        entries: Vec::new(),
        snapshots: Vec::new(),
        total_bytes: 0,
        traversed_nodes: 0,
    };
    walked.visit(root_node, &mut Vec::new())?;

    Ok((
        Box::new(DirectoryReader {
            snapshots: walked.snapshots,
        }),
        walked.entries,
    ))
}

struct Walk<'operation, 'context> {
    limits: AssetLimits,
    operation: &'operation mut Operation<'context>,
    entries: Vec<RawEntry>,
    snapshots: Vec<Snapshot>,
    total_bytes: u64,
    traversed_nodes: u64,
}

impl Walk<'_, '_> {
    fn visit(&mut self, opened: OpenedNode, prefix: &mut Vec<String>) -> Result<(), AssetFault> {
        let children = self.read_sorted(&opened)?;
        if opened.changed() {
            return Err(fault(AssetFaultKind::SourceChanged));
        }

        for name in children {
            let child =
                filesystem::open_child_stable(&opened, &name, LoadStage::Source, self.operation)?;

            if child.kind() == NodeKind::Directory {
                prefix.push(name);
                self.visit(child, prefix)?;
                prefix.pop();
                continue;
            }

            prefix.push(name);
            let recorded = prefix.join("/");
            prefix.pop();

            let declared_size = child.len();
            let accepted = child.kind() == NodeKind::Regular && child.has_single_link();
            self.push(RawEntry {
                name: recorded.into_bytes(),
                kind: if accepted {
                    EntryKind::Regular
                } else {
                    EntryKind::Other
                },
                storage: EntryStorage::Accepted,
                declared_size,
                compressed_size: declared_size,
            })?;
            self.snapshots.push(if accepted {
                Snapshot::Regular(
                    child
                        .into_file()
                        .ok_or_else(|| fault(AssetFaultKind::SourceChanged))?,
                )
            } else {
                Snapshot::Unsupported
            });
        }
        if opened.changed() {
            return Err(fault(AssetFaultKind::SourceChanged));
        }
        Ok(())
    }

    fn read_sorted(&mut self, directory: &OpenedNode) -> Result<Vec<String>, AssetFault> {
        checkpoint(self.operation)?;
        let listing = filesystem::read_children(directory, LoadStage::Source)?;
        let mut children = Vec::new();
        for child in listing {
            checkpoint(self.operation)?;
            let child = child.map_err(|_| fault(AssetFaultKind::SourceUnreadable))?;
            self.traversed_nodes = self
                .traversed_nodes
                .checked_add(1)
                .ok_or_else(|| fault(AssetFaultKind::ArithmeticOverflow))?;
            if self.traversed_nodes > u64::from(self.limits.max_entry_count()) {
                return Err(fault(AssetFaultKind::ArchiveLimit));
            }
            // A name the platform cannot express as UTF-8 cannot be a package
            // path, and is refused rather than silently skipped.
            let name = child
                .into_string()
                .map_err(|_| fault(AssetFaultKind::UnsafePath))?;
            children.push(name);
        }
        checkpoint(self.operation)?;
        children.sort();
        Ok(children)
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

fn checkpoint(operation: &mut Operation<'_>) -> Result<(), AssetFault> {
    operation
        .checkpoint()
        .map_err(|interruption| AssetFault::interrupted(interruption, LoadStage::Source))
}

const fn fault(kind: AssetFaultKind) -> AssetFault {
    AssetFault::new(kind, LoadStage::Source)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use mado_pilot_core::{Operation, OperationContext};

    use std::fs;

    use super::{DirectoryReader, Snapshot};
    use crate::fault::{AssetFaultKind, LoadStage};
    use crate::filesystem;
    use crate::reader::EntryReader;

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

    fn read(snapshot: Snapshot, declared: u64) -> Result<Vec<u8>, AssetFaultKind> {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        let mut reader = DirectoryReader {
            snapshots: vec![snapshot],
        };
        reader
            .read_entry(0, declared, LoadStage::Expansion, &mut operation)
            .map(|bytes| bytes.to_vec())
            .map_err(|fault| {
                assert_eq!(fault.stage(), LoadStage::Expansion);
                fault.kind()
            })
    }

    fn snapshot_of(path: &std::path::Path) -> Snapshot {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        let opened = filesystem::open_stable(path, LoadStage::Source, &mut operation)
            .expect("stable regular file");
        Snapshot::Regular(opened.into_file().expect("opened handle"))
    }

    #[test]
    fn a_file_that_still_matches_its_snapshot_is_read() {
        let path = scratch("unchanged", b"abcd");
        let snapshot = snapshot_of(&path);

        assert_eq!(read(snapshot, 4), Ok(b"abcd".to_vec()));
        let _ = fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_grew_after_its_handle_was_retained_is_reported_as_changed() {
        let path = scratch("grown", b"abcd");
        let snapshot = snapshot_of(&path);
        fs::write(&path, b"abcdefgh").expect("the file remains writable");

        assert_eq!(read(snapshot, 4), Err(AssetFaultKind::SourceChanged));
        let _ = fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_edited_in_place_without_changing_length_is_still_caught() {
        let path = scratch("touched", b"abcd");
        let snapshot = snapshot_of(&path);
        fs::write(&path, b"wxyz").expect("the file remains writable");

        assert_eq!(read(snapshot, 4), Err(AssetFaultKind::SourceChanged));
        let _ = fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn removing_the_path_invalidates_the_retained_snapshot() {
        let path = scratch("removed", b"abcd");
        let snapshot = snapshot_of(&path);
        fs::remove_file(&path).expect("removable");

        assert_eq!(read(snapshot, 4), Err(AssetFaultKind::SourceChanged));
    }

    #[cfg(unix)]
    #[test]
    fn replacing_the_path_invalidates_without_reading_the_replacement() {
        let path = scratch("replaced", b"original");
        let replacement = scratch("replacement", b"external");
        let snapshot = snapshot_of(&path);
        fs::remove_file(&path).expect("removable");
        fs::rename(&replacement, &path).expect("replacement can take the path");

        assert_eq!(read(snapshot, 8), Err(AssetFaultKind::SourceChanged));
        let _ = fs::remove_file(&path);
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
