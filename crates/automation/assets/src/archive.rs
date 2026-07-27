//! Reading a ZIP package without ever trusting what it says about itself.
//!
//! The two archive-only stages are here. The trailer pre-parse exists because
//! opening a central directory allocates in proportion to the entry count, so
//! an entry-count ceiling checked after the open is checked too late: the
//! measurements in `docs/evidence/g-014` read 60,000 recorded entries for 144
//! bytes through the trailer and 32,679,704 bytes through the central
//! directory. The declared total is then checked while the central directory is
//! still the largest thing that has been allocated, and before any entry is
//! looked at individually.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use mado_pilot_core::Operation;
use zip::{CompressionMethod, ZipArchive, read::ZipFile};

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::limits::AssetLimits;
use crate::reader::{EntryKind, EntryReader, EntryStorage, RawEntry, read_capped};

const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const ZIP64_LOCATOR_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
const ZIP64_EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x06];

const EOCD_LEN: u64 = 22;
const ZIP64_LOCATOR_LEN: u64 = 20;
const ZIP64_EOCD_LEN: usize = 56;
const MAX_COMMENT_LEN: u64 = u16::MAX as u64;

/// The Unix file-type mask, and the two types an entry may carry.
const FILE_TYPE_MASK: u32 = 0o170000;
const FILE_TYPE_REGULAR: u32 = 0o100000;
const FILE_TYPE_DIRECTORY: u32 = 0o040000;

struct ArchiveReader<R> {
    archive: ZipArchive<R>,
}

impl<R: Read + Seek> EntryReader for ArchiveReader<R> {
    fn read_entry(
        &mut self,
        index: usize,
        declared: u64,
        stage: LoadStage,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<[u8]>, AssetFault> {
        let mut entry = self
            .archive
            .by_index(index)
            .map_err(|_| AssetFault::new(AssetFaultKind::MalformedArchive, stage))?;
        let bytes = read_capped(&mut entry, declared, stage, operation)?;
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
) -> Result<(Box<dyn EntryReader>, Vec<RawEntry>), AssetFault> {
    if source_len > limits.max_total_compressed_bytes() {
        return Err(AssetFault::new(
            AssetFaultKind::ArchiveLimit,
            LoadStage::Source,
        ));
    }

    let recorded = recorded_entry_count(&mut reader, source_len)?;
    if recorded > u64::from(limits.max_entry_count()) {
        return Err(AssetFault::new(
            AssetFaultKind::ArchiveLimit,
            LoadStage::DirectoryPreParse,
        ));
    }

    let mut archive = ZipArchive::new(reader).map_err(|_| malformed())?;
    let present = u64::try_from(archive.len()).map_err(|_| overflow())?;
    if present != recorded {
        return Err(malformed());
    }

    let mut entries = Vec::with_capacity(archive.len());
    let mut total_uncompressed: u64 = 0;
    for index in 0..archive.len() {
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

    Ok((Box::new(ArchiveReader { archive }), entries))
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

/// Reads the entry count the archive records, without opening its directory.
///
/// The common case is one 22-byte read: an archive with no comment ends with
/// its end-of-central-directory record. Only an archive that does carry a
/// comment pays for the backwards scan, and that scan is bounded by the largest
/// comment the format can express.
fn recorded_entry_count<R: Read + Seek>(
    reader: &mut R,
    source_len: u64,
) -> Result<u64, AssetFault> {
    if source_len < EOCD_LEN {
        return Err(pre_parse_malformed());
    }

    let (eocd, eocd_offset) = locate_eocd(reader, source_len)?;
    let recorded = u64::from(u16::from_le_bytes([eocd[10], eocd[11]]));
    let directory_offset = u32::from_le_bytes([eocd[16], eocd[17], eocd[18], eocd[19]]);

    if recorded != u64::from(u16::MAX) && directory_offset != u32::MAX {
        return Ok(recorded);
    }
    zip64_entry_count(reader, eocd_offset)
}

fn locate_eocd<R: Read + Seek>(
    reader: &mut R,
    source_len: u64,
) -> Result<([u8; 22], u64), AssetFault> {
    let tail_offset = source_len - EOCD_LEN;
    let mut tail = [0u8; 22];
    seek_read_exact(reader, tail_offset, &mut tail)?;
    if tail[..4] == EOCD_SIGNATURE {
        return Ok((tail, tail_offset));
    }

    let window_len = (EOCD_LEN + MAX_COMMENT_LEN).min(source_len);
    let window_offset = source_len - window_len;
    let mut window = vec![0u8; usize::try_from(window_len).map_err(|_| overflow())?];
    seek_read_exact(reader, window_offset, &mut window)?;

    let last = window.len() - usize::try_from(EOCD_LEN).map_err(|_| overflow())?;
    let found = (0..=last)
        .rev()
        .find(|&index| window[index..index + 4] == EOCD_SIGNATURE)
        .ok_or_else(pre_parse_malformed)?;

    let mut eocd = [0u8; 22];
    eocd.copy_from_slice(&window[found..found + 22]);
    let offset = window_offset
        .checked_add(u64::try_from(found).map_err(|_| overflow())?)
        .ok_or_else(overflow)?;
    Ok((eocd, offset))
}

fn zip64_entry_count<R: Read + Seek>(reader: &mut R, eocd_offset: u64) -> Result<u64, AssetFault> {
    let locator_offset = eocd_offset
        .checked_sub(ZIP64_LOCATOR_LEN)
        .ok_or_else(pre_parse_malformed)?;
    let mut locator = [0u8; 20];
    seek_read_exact(reader, locator_offset, &mut locator)?;
    if locator[..4] != ZIP64_LOCATOR_SIGNATURE {
        return Err(pre_parse_malformed());
    }

    let record_offset = u64::from_le_bytes([
        locator[8],
        locator[9],
        locator[10],
        locator[11],
        locator[12],
        locator[13],
        locator[14],
        locator[15],
    ]);
    let mut record = [0u8; ZIP64_EOCD_LEN];
    seek_read_exact(reader, record_offset, &mut record)?;
    if record[..4] != ZIP64_EOCD_SIGNATURE {
        return Err(pre_parse_malformed());
    }

    Ok(u64::from_le_bytes([
        record[32], record[33], record[34], record[35], record[36], record[37], record[38],
        record[39],
    ]))
}

fn seek_read_exact<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    buffer: &mut [u8],
) -> Result<(), AssetFault> {
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| pre_parse_unreadable())?;
    reader
        .read_exact(buffer)
        .map_err(|_| pre_parse_unreadable())
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
