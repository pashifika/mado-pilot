//! Reading a ZIP package without ever trusting what it says about itself.
//!
//! The two archive-only stages are here. The trailer pre-parse exists because
//! opening a central directory allocates in proportion to the entry count, so
//! an entry-count ceiling checked after the open is checked too late: the
//! measurements in `docs/evidence/g-014` read 60,000 recorded entries for 144
//! bytes through the trailer and 32,679,704 bytes through the central
//! directory. After enforcing the count, a bounded no-allocation header scan
//! proves that the unambiguous trailer selects the directory the ZIP reader will
//! open. The declared total is then checked while the central directory is still
//! the largest thing that has been allocated, before entry content is read.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use mado_pilot_core::Operation;
use zip::{CompressionMethod, ZipArchive, read::ZipFile};

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::filesystem::OpenedFile;
use crate::limits::AssetLimits;
use crate::reader::{EntryKind, EntryReader, EntryStorage, RawEntry, read_capped};

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

/// Runs the archive's own stages and returns its recorded entry table.
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
    if source_len > limits.max_total_compressed_bytes() {
        return Err(AssetFault::new(
            AssetFaultKind::ArchiveLimit,
            LoadStage::Source,
        ));
    }

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
/// fixed record directly preceding their locator. This intentionally rejects
/// ambiguous trailers that `zip` could otherwise backtrack past after the
/// pre-parser had trusted a smaller count.
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
    for index in 0..=window.len() - fixed {
        if window[index..index + 4] != EOCD_SIGNATURE {
            continue;
        }
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
    let archive_offset = start
        .checked_sub(directory_offset)
        .ok_or_else(pre_parse_malformed)?;
    let advertised_record_offset = read_u64(&locator, 8);
    if archive_offset
        .checked_add(advertised_record_offset)
        .ok_or_else(overflow)?
        != physical_record_offset
    {
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
    AssetFault::new(
        AssetFaultKind::ArithmeticOverflow,
        LoadStage::DirectoryPreParse,
    )
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

    use super::{EOCD_SIGNATURE, open, recorded_directory};
    use crate::{AssetFaultKind, AssetLimits, LoadStage};

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
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["first", "second"] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .expect("entry starts");
            writer.write_all(b"x").expect("entry bytes");
        }
        let mut bytes = writer.finish().expect("archive finishes").into_inner();
        bytes.extend_from_slice(&eocd(1, 1, 46, 0));
        let source_len = u64::try_from(bytes.len()).expect("archive length fits");
        let limits = AssetLimits::ceiling()
            .with_max_entry_count(1)
            .expect("below the ceiling");
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");

        let fault = open(Cursor::new(bytes), source_len, limits, &mut operation, None)
            .err()
            .expect("the fake trailer is refused without opening the earlier directory");

        assert_eq!(fault.kind(), AssetFaultKind::MalformedArchive);
        assert_eq!(fault.stage(), LoadStage::DirectoryPreParse);
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
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&super::ZIP64_EOCD_SIGNATURE);
        bytes.extend_from_slice(&record_size.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&45u16.to_le_bytes());
        bytes.extend_from_slice(&record_disk.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&super::ZIP64_LOCATOR_SIGNATURE);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&advertised_record_offset.to_le_bytes());
        bytes.extend_from_slice(&number_of_disks.to_le_bytes());
        bytes.extend_from_slice(&eocd(zip32_count, zip32_count, u32::MAX, u32::MAX));
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
