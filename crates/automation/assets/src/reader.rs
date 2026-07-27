//! What every source must answer, and the one place bytes are counted.
//!
//! A reader reports the entries its container records and hands back the bytes
//! of one entry. It decides nothing: a reader that thought an entry name was
//! safe, or that a declared size was plausible, would be a second place those
//! rules could disagree.

use std::io::Read;
use std::sync::Arc;

use mado_pilot_core::Operation;

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};

/// The chunk size expansion is streamed in.
///
/// It is also the granularity at which a declared size is re-checked and the
/// operation context is consulted, so an entry that lies about its length is
/// caught after one chunk rather than after a ceiling's worth of it.
pub(crate) const CHUNK_BYTES: usize = 64 * 1024;

/// What kind of thing an entry is, as its container records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    /// A regular file, or a container that records no type at all.
    Regular,
    /// A directory.
    Directory,
    /// A link, device, socket, FIFO, or anything else that is not a file.
    Other,
}

/// How an entry's bytes are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryStorage {
    /// Stored or deflated, and unencrypted.
    Accepted,
    /// A compression method the contract does not accept.
    UnsupportedMethod,
    /// Encrypted content.
    Encrypted,
}

/// One entry exactly as its container records it, before any rule is applied.
#[derive(Debug, Clone)]
pub(crate) struct RawEntry {
    /// The recorded name, in its raw bytes. It may not be valid UTF-8.
    pub(crate) name: Vec<u8>,
    pub(crate) kind: EntryKind,
    pub(crate) storage: EntryStorage,
    /// The uncompressed length the container claims. Attacker-controlled.
    pub(crate) declared_size: u64,
    /// The stored length the container claims. Attacker-controlled.
    pub(crate) compressed_size: u64,
}

/// A source that can hand back the bytes of one recorded entry.
pub(crate) trait EntryReader {
    /// Reads entry `index`, producing exactly `declared` bytes or failing.
    ///
    /// `declared` is a length to stop at, never a length to trust: an entry
    /// that produces more is refused at the chunk that crosses it, and one that
    /// produces less is refused at the end.
    fn read_entry(
        &mut self,
        index: usize,
        declared: u64,
        stage: LoadStage,
        operation: &mut Operation<'_>,
    ) -> Result<Arc<[u8]>, AssetFault>;
}

/// Reads `source` in chunks, refusing to produce more than `declared` bytes.
pub(crate) fn read_capped<R: Read>(
    source: &mut R,
    declared: u64,
    stage: LoadStage,
    operation: &mut Operation<'_>,
) -> Result<Vec<u8>, AssetFault> {
    let mut produced: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; CHUNK_BYTES];
    let mut total: u64 = 0;

    loop {
        operation
            .checkpoint()
            .map_err(|interruption| AssetFault::interrupted(interruption, stage))?;

        let read = source
            .read(&mut chunk)
            .map_err(|_| AssetFault::new(AssetFaultKind::SourceUnreadable, stage))?;
        if read == 0 {
            break;
        }

        let read = u64::try_from(read)
            .map_err(|_| AssetFault::new(AssetFaultKind::ArithmeticOverflow, stage))?;
        total = total
            .checked_add(read)
            .ok_or_else(|| AssetFault::new(AssetFaultKind::ArithmeticOverflow, stage))?;
        if total > declared {
            return Err(AssetFault::new(AssetFaultKind::DeclaredSizeMismatch, stage));
        }

        let read = usize::try_from(read)
            .map_err(|_| AssetFault::new(AssetFaultKind::ArithmeticOverflow, stage))?;
        produced.extend_from_slice(&chunk[..read]);
    }

    if total != declared {
        return Err(AssetFault::new(AssetFaultKind::DeclaredSizeMismatch, stage));
    }
    Ok(produced)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use mado_pilot_core::{CancellationToken, Operation, OperationContext};

    use super::{CHUNK_BYTES, read_capped};
    use crate::fault::{AssetFaultKind, LoadStage};

    fn capped(content: &[u8], declared: u64) -> Result<Vec<u8>, AssetFaultKind> {
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        read_capped(
            &mut Cursor::new(content.to_vec()),
            declared,
            LoadStage::Expansion,
            &mut operation,
        )
        .map_err(|fault| fault.kind())
    }

    #[test]
    fn exactly_the_declared_length_is_produced() {
        assert_eq!(capped(b"abcd", 4), Ok(b"abcd".to_vec()));
        assert_eq!(capped(b"", 0), Ok(Vec::new()));
    }

    #[test]
    fn an_understated_declaration_stops_at_the_first_chunk_that_crosses_it() {
        let content = vec![0u8; 8 * 1024 * 1024];

        assert_eq!(
            capped(&content, 1_024),
            Err(AssetFaultKind::DeclaredSizeMismatch),
            "a declared size is a length to stop at, never a length to trust"
        );
    }

    #[test]
    fn an_overstated_declaration_is_caught_at_the_end() {
        assert_eq!(
            capped(b"abcd", 5),
            Err(AssetFaultKind::DeclaredSizeMismatch)
        );
    }

    #[test]
    fn content_larger_than_one_chunk_is_streamed_whole() {
        let content = vec![7u8; CHUNK_BYTES * 3 + 11];
        let declared = u64::try_from(content.len()).expect("fits");

        assert_eq!(capped(&content, declared), Ok(content));
    }

    #[test]
    fn cancellation_is_observed_between_chunks() {
        let token = CancellationToken::new();
        let context = OperationContext::new().with_cancellation(token.clone());
        let mut operation = Operation::admit(&context).expect("admitted");
        token.cancel();

        let fault = read_capped(
            &mut Cursor::new(vec![0u8; CHUNK_BYTES * 2]),
            u64::try_from(CHUNK_BYTES * 2).expect("fits"),
            LoadStage::Expansion,
            &mut operation,
        )
        .expect_err("cancelled");

        assert_eq!(fault.kind(), AssetFaultKind::Cancelled);
        assert_eq!(fault.stage(), LoadStage::Expansion);
    }
}
