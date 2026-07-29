//! Reading a ZIP package without ever trusting what it says about itself.
//!
//! # One archive is one sequence of bytes
//!
//! Every stage below reads from a [`Cursor`] over memory, including the
//! filesystem source. A retained handle stops the *path* from being redirected,
//! and on Unix it stops nothing else: a writer holding the same inode can rewrite
//! the file in place while the loader is between two of its own checks, and the
//! metadata comparison that notices only runs afterwards. That matters because
//! the reservation the trailer pre-parse exists to bound happens inside the
//! dependency, from a directory the pre-parser proved and a later reader
//! re-reads: two reads of a mutable file are two archives, and the second one is
//! not the one that passed.
//!
//! So the source-size gate is followed by one bounded copy, and the handle is
//! re-proved after it. What every later stage sees is that copy — the pre-parse,
//! the reader's own trailer search, the central directory, and every entry — so
//! there is one sequence of bytes for the whole load and no window between two
//! reads of it. The copy is bounded by the same configured source ceiling that
//! admitted the file, and a rewrite during the copy is reported as a changed
//! source rather than absorbed. The `SourceChanged` checks after it are kept:
//! nothing can redirect what the loader reads any more, but a source that changed
//! mid-load is still a load a caller should not be handed a package from.
//!
//! # The archive-only stages
//!
//! The trailer pre-parse exists because
//! opening a central directory allocates in proportion to the entry count, so
//! an entry-count ceiling checked after the open is checked too late: the
//! measurements in `docs/evidence/g-014` read 60,000 recorded entries for 144
//! bytes through the trailer and 32,679,704 bytes through the central
//! directory.
//!
//! Enforcing the count before the open is worth something only if the reader
//! opens the trailer that was counted, and the reader selects its own. So the
//! pre-parse takes the one record whose comment accounts for the exact suffix,
//! refuses any further record beginning after it — every candidate the reader
//! would reach before that one — and proves with a bounded no-allocation header
//! scan that the recorded directory tiles the space it claims. The count is
//! enforced against that directory, the opened archive is then re-proved to be
//! reading the same directory, and the declared total is checked while the
//! central directory is still the largest thing that has been allocated, before
//! entry content is read.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

use mado_pilot_core::Operation;
use zip::{CompressionMethod, ZipArchive, read::ZipFile};

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::filesystem::OpenedFile;
use crate::limits::AssetLimits;
use crate::reader::{CHUNK_BYTES, EntryKind, EntryReader, EntryStorage, RawEntry, read_capped};

const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const ZIP64_LOCATOR_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
const ZIP64_EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x06];

const EOCD_LEN: u64 = 22;
const ZIP64_LOCATOR_LEN: u64 = 20;
const ZIP64_EOCD_LEN: usize = 56;
const ZIP64_EOCD_RECORD_SIZE: u64 = 44;
const CENTRAL_DIRECTORY_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const CENTRAL_DIRECTORY_HEADER_LEN: usize = 46;
const MAX_COMMENT_LEN: u64 = u16::MAX as u64;

/// The Unix file-type mask, and the two types an entry may carry.
const FILE_TYPE_MASK: u32 = 0o170000;
const FILE_TYPE_REGULAR: u32 = 0o100000;
const FILE_TYPE_DIRECTORY: u32 = 0o040000;

struct ArchiveReader<R> {
    archive: ZipArchive<R>,
    source: Option<OpenedFile>,
}

impl<R: Read + Seek> EntryReader for ArchiveReader<R> {
    fn read_entry(
        &mut self,
        index: usize,
        declared: u64,
        stage: LoadStage,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<[u8]>, AssetFault> {
        ensure_unchanged(self.source.as_ref(), stage)?;
        let bytes = {
            let mut entry = self
                .archive
                .by_index(index)
                .map_err(|_| AssetFault::new(AssetFaultKind::MalformedArchive, stage))?;
            read_capped(&mut entry, declared, stage, operation)?
        };
        ensure_unchanged(self.source.as_ref(), stage)?;
        Ok(Arc::from(bytes))
    }
}

/// Runs the archive's own stages over one immutable copy of a filesystem
/// source.
///
/// The copy is what makes the pre-parse worth performing on an externally
/// mutable file; see this module's own documentation. It happens after the
/// source-size gate, so the memory it takes is the memory that gate admitted,
/// and the retained handle is re-proved after it.
///
/// # Errors
///
/// As [`open`], and returns [`AssetFaultKind::SourceChanged`] at
/// [`LoadStage::Source`] when the handle or its length does not survive the copy.
pub(crate) fn open_file(
    mut source: OpenedFile,
    source_len: u64,
    limits: AssetLimits,
    operation: &mut Operation<'_>,
) -> Result<(Box<dyn EntryReader>, Vec<RawEntry>), AssetFault> {
    within_source_ceiling(source_len, limits)?;

    ensure_unchanged(Some(&source), LoadStage::Source)?;
    let snapshot = snapshot(&mut source, source_len, operation)?;
    // After the copy, not only before it: a rewrite that landed while the bytes
    // were being read would otherwise be a mixture of two archives that each
    // check on its own could believe.
    ensure_unchanged(Some(&source), LoadStage::Source)?;

    open(
        Cursor::new(snapshot),
        source_len,
        limits,
        operation,
        Some(source),
    )
}

/// Copies the retained source into one immutable sequence of bytes.
///
/// `source_len` is the length the handle itself reported, so it is authoritative
/// rather than declared: content that runs past it or stops short of it is a file
/// that changed since it was measured, which is what that fault says. The read is
/// chunked so the operation context is consulted while a large source is copied.
///
/// The handle's own file offset is left at the end. Nothing reads through it
/// again — every later stage reads the returned bytes, and the change detection
/// asks the handle for metadata rather than for content.
fn snapshot(
    source: &mut OpenedFile,
    source_len: u64,
    operation: &mut Operation<'_>,
) -> Result<Vec<u8>, AssetFault> {
    let length = usize::try_from(source_len).map_err(|_| overflow_at(LoadStage::Source))?;
    let mut bytes = Vec::new();
    // The one allocation the source ceiling bounds, and it is requested rather
    // than assumed: a host that cannot satisfy a length inside that ceiling
    // reports an unreadable source instead of aborting the process.
    bytes
        .try_reserve_exact(length)
        .map_err(|_| AssetFault::new(AssetFaultKind::SourceUnreadable, LoadStage::Source))?;

    let mut chunk = vec![0u8; CHUNK_BYTES];
    loop {
        checkpoint(operation, LoadStage::Source)?;
        let read = source
            .file_mut()
            .read(&mut chunk)
            .map_err(|_| AssetFault::new(AssetFaultKind::SourceUnreadable, LoadStage::Source))?;
        if read == 0 {
            break;
        }
        if read > length - bytes.len() {
            return Err(source_changed());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    if bytes.len() != length {
        return Err(source_changed());
    }

    Ok(bytes)
}

/// Runs the archive's own stages and returns its recorded entry table.
///
/// The reader is a sequence of bytes that cannot change while it is being read:
/// [`Cursor`] over caller-owned memory, or over the copy [`open_file`] takes of a
/// filesystem source.
///
/// # Errors
///
/// Returns an [`AssetFault`] naming the stage that stopped: source bytes above
/// the total-compressed limit at [`LoadStage::Source`], a recorded entry count
/// above its limit at [`LoadStage::DirectoryPreParse`], and a malformed
/// structure or a declared total above its limit at
/// [`LoadStage::DirectoryOpen`].
pub(crate) fn open<R: Read + Seek + 'static>(
    mut reader: R,
    source_len: u64,
    limits: AssetLimits,
    operation: &mut Operation<'_>,
    source: Option<OpenedFile>,
) -> Result<(Box<dyn EntryReader>, Vec<RawEntry>), AssetFault> {
    within_source_ceiling(source_len, limits)?;

    ensure_unchanged(source.as_ref(), LoadStage::DirectoryPreParse)?;
    checkpoint(operation, LoadStage::DirectoryPreParse)?;
    let directory = recorded_directory(&mut reader, source_len, operation)?;
    if directory.entry_count > u64::from(limits.max_entry_count()) {
        return Err(AssetFault::new(
            AssetFaultKind::ArchiveLimit,
            LoadStage::DirectoryPreParse,
        ));
    }
    validate_central_directory(&mut reader, directory, operation)?;
    ensure_unchanged(source.as_ref(), LoadStage::DirectoryPreParse)?;

    checkpoint(operation, LoadStage::DirectoryOpen)?;
    let mut archive = ZipArchive::new(reader).map_err(|_| malformed())?;
    ensure_unchanged(source.as_ref(), LoadStage::DirectoryOpen)?;
    // Re-proving the pre-parse against what the reader actually opened, rather
    // than assuming the two agree. `zip` 8.6.0 searches the whole file backwards
    // for a trailer (`spec.rs:805`), accepts one whose comment merely fits inside
    // the file rather than ending it (`spec.rs:823-828`), and falls back to an
    // earlier record when it rejects the first (`read/zip_archive.rs:167-173`).
    // The pre-parse refuses every record the fallback would reach first, so a
    // disagreement here means the reader rejected the validated trailer and
    // substituted another; that substitute was never counted, and an archive is
    // not two archives. A `zip` version bump must re-verify those three sites and
    // the reservation at `read/zip_archive.rs:183-205`.
    if archive.central_directory_start() != directory.start {
        return Err(malformed());
    }
    // Unreachable with `zip` 8.6.0, and deliberately kept. `validate_central_directory`
    // walks exactly `entry_count` headers from `start` and requires the walk to
    // land on `end`, so the pre-parse has already proved the recorded count is
    // the number of headers physically in the recorded region; the comparison
    // above has already proved the reader opened that same region. For this to
    // fire, `zip` would have to count something else out of bytes both sides
    // agree on. No fixture can reach it without faking the reader, which is why
    // there is no test for it — it is the third of the three things a `zip`
    // version bump has to re-verify, not a gap in coverage.
    let present = u64::try_from(archive.len()).map_err(|_| overflow())?;
    if present != directory.entry_count {
        return Err(malformed());
    }

    let mut entries = Vec::with_capacity(archive.len());
    let mut total_uncompressed: u64 = 0;
    for index in 0..archive.len() {
        checkpoint(operation, LoadStage::DirectoryOpen)?;
        let entry = archive.by_index_raw(index).map_err(|_| malformed())?;
        let declared_size = entry.size();
        total_uncompressed = total_uncompressed
            .checked_add(declared_size)
            .ok_or_else(overflow)?;
        entries.push(RawEntry {
            name: entry.name_raw().to_vec(),
            kind: entry_kind(&entry),
            storage: entry_storage(&entry),
            declared_size,
            compressed_size: entry.compressed_size(),
        });
    }

    if total_uncompressed > limits.max_total_uncompressed_bytes() {
        return Err(AssetFault::new(
            AssetFaultKind::ArchiveLimit,
            LoadStage::DirectoryOpen,
        ));
    }
    ensure_unchanged(source.as_ref(), LoadStage::DirectoryOpen)?;

    Ok((Box::new(ArchiveReader { archive, source }), entries))
}

fn entry_kind<R: Read>(entry: &ZipFile<'_, R>) -> EntryKind {
    // A trailing slash is deliberately not treated as a directory here: name
    // normalization runs first and refuses it, which keeps one name rule rather
    // than two that could disagree.
    match entry.unix_mode() {
        None => EntryKind::Regular,
        Some(mode) => match mode & FILE_TYPE_MASK {
            0 | FILE_TYPE_REGULAR => EntryKind::Regular,
            FILE_TYPE_DIRECTORY => EntryKind::Directory,
            _ => EntryKind::Other,
        },
    }
}

fn entry_storage<R: Read>(entry: &ZipFile<'_, R>) -> EntryStorage {
    if !matches!(
        entry.compression(),
        CompressionMethod::Stored | CompressionMethod::Deflated
    ) {
        return EntryStorage::UnsupportedMethod;
    }
    if entry.encrypted() {
        return EntryStorage::Encrypted;
    }
    EntryStorage::Accepted
}

#[derive(Debug, Clone, Copy)]
struct RecordedDirectory {
    entry_count: u64,
    start: u64,
    end: u64,
}

/// Reads and validates the one archive trailer profile accepted by Phase 1.
///
/// The selected EOCD must account for the exact suffix through its comment
/// length. Single-disk count fields must agree, and ZIP64 records must be the
/// fixed record directly preceding their locator, at the offset that locator
/// advertises. This intentionally rejects ambiguous trailers that `zip` could
/// otherwise backtrack past — or, in the ZIP64 case, search forward past — after
/// the pre-parser had trusted a smaller count.
fn recorded_directory<R: Read + Seek>(
    reader: &mut R,
    source_len: u64,
    operation: &mut Operation<'_>,
) -> Result<RecordedDirectory, AssetFault> {
    if source_len < EOCD_LEN {
        return Err(pre_parse_malformed());
    }

    let (eocd, eocd_offset) = locate_eocd(reader, source_len, operation)?;
    let disk = read_u16(&eocd, 4);
    let directory_disk = read_u16(&eocd, 6);
    let entries_on_disk = read_u16(&eocd, 8);
    let total_entries = read_u16(&eocd, 10);
    let directory_size = read_u32(&eocd, 12);
    let directory_offset = read_u32(&eocd, 16);
    if disk != 0 || directory_disk != 0 || entries_on_disk != total_entries {
        return Err(pre_parse_malformed());
    }

    let zip64 =
        total_entries == u16::MAX || directory_size == u32::MAX || directory_offset == u32::MAX;
    if zip64 {
        return zip64_directory(reader, &eocd, eocd_offset, operation);
    }

    let size = u64::from(directory_size);
    let start = eocd_offset
        .checked_sub(size)
        .ok_or_else(pre_parse_malformed)?;
    // A prepended archive stub is permitted, but the relative directory offset
    // must still fit before the physical directory selected by this EOCD.
    start
        .checked_sub(u64::from(directory_offset))
        .ok_or_else(pre_parse_malformed)?;
    Ok(RecordedDirectory {
        entry_count: u64::from(entries_on_disk),
        start,
        end: eocd_offset,
    })
}

fn locate_eocd<R: Read + Seek>(
    reader: &mut R,
    source_len: u64,
    operation: &mut Operation<'_>,
) -> Result<([u8; 22], u64), AssetFault> {
    let window_len = (EOCD_LEN + MAX_COMMENT_LEN).min(source_len);
    let window_offset = source_len - window_len;
    let mut window = vec![0u8; usize::try_from(window_len).map_err(|_| overflow())?];
    seek_read_exact(reader, window_offset, &mut window, operation)?;

    let fixed = usize::try_from(EOCD_LEN).map_err(|_| overflow())?;
    let mut selected = None;
    let mut last_candidate = None;
    for index in 0..=window.len() - fixed {
        if window[index..index + 4] != EOCD_SIGNATURE {
            continue;
        }
        // Recorded whether or not this record is selectable: a record the reader
        // could parse is a trailer it could choose, and the loop runs forward, so
        // this ends up holding the last one in the file.
        last_candidate = Some(index);
        let comment_len = usize::from(read_u16(&window[index..index + fixed], 20));
        if index
            .checked_add(fixed)
            .and_then(|end| end.checked_add(comment_len))
            != Some(window.len())
        {
            continue;
        }
        if selected.is_some() {
            return Err(pre_parse_malformed());
        }
        selected = Some(index);
    }
    let found = selected.ok_or_else(pre_parse_malformed)?;
    // The selected record accounts for the exact suffix, so everything after it
    // is its own comment — and the reader searches backwards from the end of the
    // file, so a record hidden in that comment is the trailer it would try first.
    // Refusing here is what makes the entry count enforced below the count of the
    // directory the reader opens, rather than of a directory it will not read.
    if last_candidate.is_some_and(|last| last > found) {
        return Err(pre_parse_malformed());
    }

    let mut eocd = [0u8; 22];
    eocd.copy_from_slice(&window[found..found + fixed]);
    let offset = window_offset
        .checked_add(u64::try_from(found).map_err(|_| overflow())?)
        .ok_or_else(overflow)?;
    Ok((eocd, offset))
}

fn zip64_directory<R: Read + Seek>(
    reader: &mut R,
    eocd: &[u8; 22],
    eocd_offset: u64,
    operation: &mut Operation<'_>,
) -> Result<RecordedDirectory, AssetFault> {
    let locator_offset = eocd_offset
        .checked_sub(ZIP64_LOCATOR_LEN)
        .ok_or_else(pre_parse_malformed)?;
    let mut locator = [0u8; 20];
    seek_read_exact(reader, locator_offset, &mut locator, operation)?;
    if locator[..4] != ZIP64_LOCATOR_SIGNATURE
        || read_u32(&locator, 4) != 0
        || read_u32(&locator, 16) != 1
    {
        return Err(pre_parse_malformed());
    }

    let physical_record_offset = locator_offset
        .checked_sub(u64::try_from(ZIP64_EOCD_LEN).map_err(|_| overflow())?)
        .ok_or_else(pre_parse_malformed)?;
    let mut record = [0u8; ZIP64_EOCD_LEN];
    seek_read_exact(reader, physical_record_offset, &mut record, operation)?;
    if record[..4] != ZIP64_EOCD_SIGNATURE
        || read_u64(&record, 4) != ZIP64_EOCD_RECORD_SIZE
        || read_u32(&record, 16) != 0
        || read_u32(&record, 20) != 0
    {
        return Err(pre_parse_malformed());
    }

    let entries_on_disk = read_u64(&record, 24);
    let total_entries = read_u64(&record, 32);
    let directory_size = read_u64(&record, 40);
    let directory_offset = read_u64(&record, 48);
    if entries_on_disk != total_entries
        || !matches_u16(read_u16(eocd, 8), entries_on_disk)
        || !matches_u16(read_u16(eocd, 10), total_entries)
        || !matches_u32(read_u32(eocd, 12), directory_size)
        || !matches_u32(read_u32(eocd, 16), directory_offset)
    {
        return Err(pre_parse_malformed());
    }

    let start = physical_record_offset
        .checked_sub(directory_size)
        .ok_or_else(pre_parse_malformed)?;
    // A prepended stub is permitted for a zip32 trailer but refused here, and the
    // asymmetry is the reader's, not a preference. `zip` finds the ZIP64 record by
    // a *forward* search from the offset the locator advertises
    // (`spec.rs:939-958`), taking the first record that parses. Any archive offset
    // therefore opens a `[advertised, physical)` window ahead of the record
    // validated above, and a decoy planted in that window is reached first: its
    // entry count is one nothing here counted, and `read/zip_archive.rs:184-200`
    // reserves for it before the cross-check at the call site can refuse. A zero
    // archive offset makes the search's own first probe land on this record, so
    // the window is empty by construction. ZIP64 exists in this profile for
    // writers that emit the markers unconditionally, not for archives too large
    // for zip32, so nothing a package needs is lost.
    let advertised_record_offset = read_u64(&locator, 8);
    if start != directory_offset || advertised_record_offset != physical_record_offset {
        return Err(pre_parse_malformed());
    }

    Ok(RecordedDirectory {
        entry_count: total_entries,
        start,
        end: physical_record_offset,
    })
}

fn validate_central_directory<R: Read + Seek>(
    reader: &mut R,
    directory: RecordedDirectory,
    operation: &mut Operation<'_>,
) -> Result<(), AssetFault> {
    let mut position = directory.start;
    for _ in 0..directory.entry_count {
        let mut header = [0u8; CENTRAL_DIRECTORY_HEADER_LEN];
        seek_read_exact(reader, position, &mut header, operation)?;
        if header[..4] != CENTRAL_DIRECTORY_SIGNATURE {
            return Err(pre_parse_malformed());
        }
        let variable = u64::from(read_u16(&header, 28))
            .checked_add(u64::from(read_u16(&header, 30)))
            .and_then(|value| value.checked_add(u64::from(read_u16(&header, 32))))
            .ok_or_else(overflow)?;
        position = position
            .checked_add(u64::try_from(CENTRAL_DIRECTORY_HEADER_LEN).map_err(|_| overflow())?)
            .and_then(|value| value.checked_add(variable))
            .ok_or_else(overflow)?;
        if position > directory.end {
            return Err(pre_parse_malformed());
        }
    }
    if position != directory.end {
        return Err(pre_parse_malformed());
    }
    Ok(())
}

fn matches_u16(encoded: u16, actual: u64) -> bool {
    encoded == u16::MAX || u64::from(encoded) == actual
}

fn matches_u32(encoded: u32, actual: u64) -> bool {
    encoded == u32::MAX || u64::from(encoded) == actual
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn seek_read_exact<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    buffer: &mut [u8],
    operation: &mut Operation<'_>,
) -> Result<(), AssetFault> {
    checkpoint(operation, LoadStage::DirectoryPreParse)?;
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| pre_parse_unreadable())?;
    reader
        .read_exact(buffer)
        .map_err(|_| pre_parse_unreadable())
}

/// Refuses a source whose own length is above the configured ceiling.
///
/// The first thing either entry point does, because every later cost — the copy,
/// the trailer window, the header scan — is bounded by this length.
const fn within_source_ceiling(source_len: u64, limits: AssetLimits) -> Result<(), AssetFault> {
    if source_len > limits.max_total_compressed_bytes() {
        return Err(AssetFault::new(
            AssetFaultKind::ArchiveLimit,
            LoadStage::Source,
        ));
    }
    Ok(())
}

fn ensure_unchanged(source: Option<&OpenedFile>, stage: LoadStage) -> Result<(), AssetFault> {
    if source.is_some_and(OpenedFile::changed) {
        return Err(AssetFault::new(AssetFaultKind::SourceChanged, stage));
    }
    Ok(())
}

fn checkpoint(operation: &mut Operation<'_>, stage: LoadStage) -> Result<(), AssetFault> {
    operation
        .checkpoint()
        .map_err(|interruption| AssetFault::interrupted(interruption, stage))
}

const fn malformed() -> AssetFault {
    AssetFault::new(AssetFaultKind::MalformedArchive, LoadStage::DirectoryOpen)
}

const fn overflow() -> AssetFault {
    overflow_at(LoadStage::DirectoryPreParse)
}

const fn overflow_at(stage: LoadStage) -> AssetFault {
    AssetFault::new(AssetFaultKind::ArithmeticOverflow, stage)
}

const fn source_changed() -> AssetFault {
    AssetFault::new(AssetFaultKind::SourceChanged, LoadStage::Source)
}

const fn pre_parse_malformed() -> AssetFault {
    AssetFault::new(
        AssetFaultKind::MalformedArchive,
        LoadStage::DirectoryPreParse,
    )
}

const fn pre_parse_unreadable() -> AssetFault {
    AssetFault::new(
        AssetFaultKind::SourceUnreadable,
        LoadStage::DirectoryPreParse,
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use mado_pilot_core::{Operation, OperationContext};
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::{
        AssetFault, CENTRAL_DIRECTORY_HEADER_LEN, CENTRAL_DIRECTORY_SIGNATURE, EOCD_SIGNATURE,
        RawEntry, ZIP64_EOCD_LEN, ZIP64_EOCD_RECORD_SIZE, open, recorded_directory,
    };
    use crate::{AssetFaultKind, AssetLimits, LoadStage};

    /// Opens `bytes` under an `max_entries` ceiling and returns its entry table.
    fn accept(bytes: Vec<u8>, max_entries: u32) -> Vec<RawEntry> {
        let (_, entries) = attempt(bytes, max_entries).expect("the archive loads");
        entries
    }

    /// Opens `bytes` under an `max_entries` ceiling and returns why it was
    /// refused.
    fn refuse(bytes: Vec<u8>, max_entries: u32) -> AssetFault {
        attempt(bytes, max_entries)
            .err()
            .expect("the archive is refused")
    }

    fn attempt(
        bytes: Vec<u8>,
        max_entries: u32,
    ) -> Result<(Box<dyn crate::reader::EntryReader>, Vec<RawEntry>), AssetFault> {
        let source_len = u64::try_from(bytes.len()).expect("archive length fits");
        let limits = AssetLimits::ceiling()
            .with_max_entry_count(max_entries)
            .expect("below the ceiling");
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");

        open(Cursor::new(bytes), source_len, limits, &mut operation, None)
    }

    /// The archive the trailer tests start from: two entries, and a comment-less
    /// trailer as the final twenty-two bytes.
    fn two_entry_archive() -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["first", "second"] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .expect("entry starts");
            writer.write_all(b"x").expect("entry bytes");
        }
        let bytes = writer.finish().expect("archive finishes").into_inner();

        assert_eq!(
            bytes[bytes.len() - 22..bytes.len() - 18],
            EOCD_SIGNATURE,
            "the tests below rewrite the comment length of this trailer"
        );
        bytes
    }

    /// [`two_entry_archive`] with `comment` declared as its trailer comment.
    fn commented_archive(comment: &[u8]) -> Vec<u8> {
        let mut bytes = two_entry_archive();
        let declared = u16::try_from(comment.len()).expect("the comment fits its field");
        let field = bytes.len() - 2;
        bytes[field..].copy_from_slice(&declared.to_le_bytes());
        bytes.extend_from_slice(comment);
        bytes
    }

    /// An archive whose recorded directory is not the one the reader opens.
    ///
    /// The trailer records one entry, a directory size of one header, and a
    /// directory offset pointing at the *first* of two adjacent headers. The
    /// pre-parse derives the directory from the size and so validates the second
    /// header; `zip` derives it from the offset and so opens the first. Both see
    /// exactly one entry, which is why the recorded-count comparison cannot tell
    /// the two directories apart, and both entries have a readable local header,
    /// so nothing downstream refuses the substitution either.
    fn divergent_directory_archive() -> Vec<u8> {
        let mut bytes = local_header(b"a");
        let second_entry = u32::try_from(bytes.len()).expect("one local header is small");
        bytes.extend_from_slice(&local_header(b"b"));
        let first_recorded = u32::try_from(bytes.len()).expect("two local headers are small");
        bytes.extend_from_slice(&central_header(b"a", 0));
        bytes.extend_from_slice(&central_header(b"b", second_entry));
        let one_header =
            u32::try_from(CENTRAL_DIRECTORY_HEADER_LEN + 1).expect("one header is small");
        bytes.extend_from_slice(&eocd(1, 1, one_header, first_recorded));
        bytes
    }

    /// A local file header for a stored, empty entry named `name`.
    fn local_header(name: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        bytes.extend_from_slice(&20u16.to_le_bytes()); // version needed
        bytes.extend_from_slice(&0u16.to_le_bytes()); // general purpose flags
        bytes.extend_from_slice(&0u16.to_le_bytes()); // stored
        bytes.extend_from_slice(&0u16.to_le_bytes()); // modification time
        bytes.extend_from_slice(&0x21u16.to_le_bytes()); // modification date
        bytes.extend_from_slice(&0u32.to_le_bytes()); // crc32
        bytes.extend_from_slice(&0u32.to_le_bytes()); // compressed size
        bytes.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
        let name_len = u16::try_from(name.len()).expect("the name fits its field");
        bytes.extend_from_slice(&name_len.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        bytes.extend_from_slice(name);
        bytes
    }

    /// A central directory header for a stored, empty entry named `name` whose
    /// local header begins at `local_offset`.
    fn central_header(name: &[u8], local_offset: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CENTRAL_DIRECTORY_HEADER_LEN + name.len());
        bytes.extend_from_slice(&CENTRAL_DIRECTORY_SIGNATURE);
        bytes.extend_from_slice(&20u16.to_le_bytes()); // version made by
        bytes.extend_from_slice(&20u16.to_le_bytes()); // version needed
        bytes.extend_from_slice(&0u16.to_le_bytes()); // general purpose flags
        bytes.extend_from_slice(&0u16.to_le_bytes()); // stored
        bytes.extend_from_slice(&0u16.to_le_bytes()); // modification time
        bytes.extend_from_slice(&0x21u16.to_le_bytes()); // modification date
        bytes.extend_from_slice(&0u32.to_le_bytes()); // crc32
        bytes.extend_from_slice(&0u32.to_le_bytes()); // compressed size
        bytes.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
        let name_len = u16::try_from(name.len()).expect("the name fits its field");
        bytes.extend_from_slice(&name_len.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        bytes.extend_from_slice(&0u16.to_le_bytes()); // entry comment length
        bytes.extend_from_slice(&0u16.to_le_bytes()); // first disk
        bytes.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        bytes.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        bytes.extend_from_slice(&local_offset.to_le_bytes());
        bytes.extend_from_slice(name);
        bytes
    }

    #[test]
    fn disagreeing_single_disk_count_fields_are_rejected_before_directory_open() {
        let bytes = eocd(2, 1, 0, 0);
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");

        let fault = recorded_directory(
            &mut Cursor::new(bytes.clone()),
            u64::try_from(bytes.len()).expect("EOCD length fits"),
            &mut operation,
        )
        .expect_err("single-disk counts must agree");

        assert_eq!(fault.kind(), AssetFaultKind::MalformedArchive);
        assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
    }

    #[test]
    fn a_trailing_low_count_eocd_cannot_redirect_preparse_from_an_earlier_archive() {
        let mut bytes = two_entry_archive();
        bytes.extend_from_slice(&eocd(1, 1, 46, 0));

        let fault = refuse(bytes, 1);

        assert_eq!(fault.kind(), AssetFaultKind::MalformedArchive);
        assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
    }

    #[test]
    fn a_second_trailer_hidden_in_the_comment_is_refused_before_the_directory_is_opened() {
        // The reader searches backwards from the end of the file, so a record
        // inside the selected trailer's comment is the trailer it would open
        // first. This decoy declares 65,534 entries against a four-entry
        // ceiling, and nothing would ever enforce that count against it.
        let mut comment = eocd(65_534, 65_534, 46, 0);
        // One byte of trailing garbage is what keeps the decoy from accounting
        // for the exact suffix, so selecting a single trailer does not see it.
        comment.push(0xff);
        let bytes = commented_archive(&comment);

        let fault = refuse(bytes, 4);

        assert_eq!(fault.kind(), AssetFaultKind::MalformedArchive);
        assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
    }

    #[test]
    fn an_archive_comment_carrying_no_second_trailer_is_still_accepted() {
        let bytes = commented_archive(b"packaged by a tool that writes a comment");

        assert_eq!(accept(bytes, 4).len(), 2);
    }

    #[test]
    fn a_directory_the_reader_opens_elsewhere_is_refused_even_when_the_count_agrees() {
        let fault = refuse(divergent_directory_archive(), 4);

        assert_eq!(fault.kind(), AssetFaultKind::MalformedArchive);
        assert_eq!(fault.stage(), LoadStage::DirectoryOpen);
    }

    #[test]
    fn a_prepended_stub_leaves_the_pre_parse_and_the_reader_on_one_directory() {
        let mut bytes = vec![0u8; 64];
        bytes.extend_from_slice(&two_entry_archive());

        assert_eq!(accept(bytes, 4).len(), 2);
    }

    #[test]
    fn a_minimal_single_disk_zip64_archive_is_accepted() {
        let bytes = zip64(0, 44, 0, 1, 0, u16::MAX);
        let source_len = u64::try_from(bytes.len()).expect("archive length fits");
        let limits = AssetLimits::ceiling()
            .with_max_entry_count(0)
            .expect("below the ceiling");
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");

        let (_, entries) = open(Cursor::new(bytes), source_len, limits, &mut operation, None)
            .expect("bounded ZIP64 archive");

        assert!(entries.is_empty());
    }

    #[test]
    fn zip64_count_disagreement_is_rejected_during_preparse() {
        let bytes = zip64(0, 44, 0, 1, 0, 1);
        assert_zip64_preparse_fault(bytes, AssetFaultKind::MalformedArchive, 1);
    }

    #[test]
    fn zip64_locator_offset_is_validated_against_the_physical_record() {
        let bytes = zip64(0, 44, 1, 1, 0, u16::MAX);
        assert_zip64_preparse_fault(bytes, AssetFaultKind::MalformedArchive, 1);
    }

    #[test]
    fn multidisk_zip64_is_rejected_during_preparse() {
        let bytes = zip64(0, 44, 0, 2, 0, u16::MAX);
        assert_zip64_preparse_fault(bytes, AssetFaultKind::MalformedArchive, 1);
    }

    #[test]
    fn an_extended_zip64_record_is_outside_the_phase_one_profile() {
        let bytes = zip64(0, 45, 0, 1, 0, u16::MAX);
        assert_zip64_preparse_fault(bytes, AssetFaultKind::MalformedArchive, 1);
    }

    #[test]
    fn zip64_entry_count_is_bounded_before_central_directory_work() {
        let bytes = zip64(2, 44, 0, 1, 0, u16::MAX);
        assert_zip64_preparse_fault(bytes, AssetFaultKind::ArchiveLimit, 1);
    }

    #[test]
    fn a_zip64_record_ahead_of_the_validated_one_is_refused_before_the_directory_is_opened() {
        // The reader searches forward from the offset the locator advertises, so
        // a stub puts every byte between that offset and the validated record
        // inside its window, and the first record that parses wins. This decoy
        // declares eight entries from inside the stub against a four-entry
        // ceiling the pre-parse enforced against the empty directory behind it,
        // and the reader reserves for that count before anything compares the
        // two directories.
        let bytes = zip64_behind_a_stub(512, Some((400, 8)));

        assert_zip64_preparse_fault(bytes, AssetFaultKind::MalformedArchive, 4);
    }

    #[test]
    fn a_zip64_trailer_behind_a_prepended_stub_is_outside_the_phase_one_profile() {
        // Refusing the archive offset outright is what leaves the forward search
        // no window, so a stub is refused whether or not one is planted in it.
        let bytes = zip64_behind_a_stub(512, None);

        assert_zip64_preparse_fault(bytes, AssetFaultKind::MalformedArchive, 4);
    }

    fn assert_zip64_preparse_fault(bytes: Vec<u8>, expected: AssetFaultKind, limit: u32) {
        let source_len = u64::try_from(bytes.len()).expect("archive length fits");
        let limits = AssetLimits::ceiling()
            .with_max_entry_count(limit)
            .expect("below the ceiling");
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");

        let fault = open(Cursor::new(bytes), source_len, limits, &mut operation, None)
            .err()
            .expect("ZIP64 fixture is refused during preparse");

        assert_eq!(fault.kind(), expected);
        assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
    }

    fn zip64(
        entries: u64,
        record_size: u64,
        advertised_record_offset: u64,
        number_of_disks: u32,
        record_disk: u32,
        zip32_count: u16,
    ) -> Vec<u8> {
        let mut bytes = zip64_record(record_size, entries, record_disk);
        bytes.extend_from_slice(&super::ZIP64_LOCATOR_SIGNATURE);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&advertised_record_offset.to_le_bytes());
        bytes.extend_from_slice(&number_of_disks.to_le_bytes());
        bytes.extend_from_slice(&eocd(zip32_count, zip32_count, u32::MAX, u32::MAX));
        bytes
    }

    /// A ZIP64 end-of-central-directory record for an empty directory.
    fn zip64_record(record_size: u64, entries: u64, record_disk: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(ZIP64_EOCD_LEN);
        bytes.extend_from_slice(&super::ZIP64_EOCD_SIGNATURE);
        bytes.extend_from_slice(&record_size.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes()); // version made by
        bytes.extend_from_slice(&45u16.to_le_bytes()); // version needed
        bytes.extend_from_slice(&record_disk.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // the directory's disk
        bytes.extend_from_slice(&entries.to_le_bytes()); // entries on this disk
        bytes.extend_from_slice(&entries.to_le_bytes()); // entries in total
        bytes.extend_from_slice(&0u64.to_le_bytes()); // directory size
        bytes.extend_from_slice(&0u64.to_le_bytes()); // directory offset
        bytes
    }

    /// A ZIP64 trailer for an empty directory behind a `stub`-byte prepended
    /// stub, optionally carrying a decoy record at an offset inside that stub.
    ///
    /// The recorded directory is empty, so the validated record sits at `stub`,
    /// its locator at `stub + 56`, and the zip32 trailer at `stub + 76`. The
    /// locator advertises offset zero, which is what the pre-parse's own
    /// `archive_offset + advertised == physical` requirement forces once a stub
    /// is present — and zero is where `zip` starts searching *forward* for the
    /// record, so the whole stub lies inside its window.
    fn zip64_behind_a_stub(stub: usize, decoy: Option<(usize, u64)>) -> Vec<u8> {
        let physical = u64::try_from(stub).expect("the stub is small");
        let locator_offset = physical + u64::try_from(ZIP64_EOCD_LEN).expect("the record is small");
        let mut bytes = vec![0u8; stub];

        if let Some((offset, entries)) = decoy {
            // `zip` requires a record to end exactly where its locator begins
            // (`spec.rs:931-932`), which fixes the decoy's record size and makes
            // everything between the two its extensible data sector.
            let start = u64::try_from(offset).expect("the offset is small");
            let record = zip64_record(locator_offset - start - 12, entries, 0);
            bytes[offset..offset + record.len()].copy_from_slice(&record);
        }

        bytes.extend_from_slice(&zip64_record(ZIP64_EOCD_RECORD_SIZE, 0, 0));
        bytes.extend_from_slice(&super::ZIP64_LOCATOR_SIGNATURE);
        bytes.extend_from_slice(&0u32.to_le_bytes()); // the record's disk
        bytes.extend_from_slice(&0u64.to_le_bytes()); // advertised record offset
        bytes.extend_from_slice(&1u32.to_le_bytes()); // total disks
        bytes.extend_from_slice(&eocd(u16::MAX, u16::MAX, u32::MAX, u32::MAX));
        bytes
    }

    fn eocd(entries_on_disk: u16, total_entries: u16, directory_size: u32, offset: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(22);
        bytes.extend_from_slice(&EOCD_SIGNATURE);
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&entries_on_disk.to_le_bytes());
        bytes.extend_from_slice(&total_entries.to_le_bytes());
        bytes.extend_from_slice(&directory_size.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }
}
