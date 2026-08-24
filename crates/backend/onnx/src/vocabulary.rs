//! Compact validation and lookup for the recognizer's embedded vocabulary.

use std::ops::Range;

use sha2::{Digest, Sha256};

use crate::fault::OnnxBackendFault;

const ENTRIES: usize = 18_708;
const CLASSES: usize = ENTRIES + 2;

#[derive(Clone)]
pub(crate) struct Vocabulary {
    raw: String,
    entries: Vec<Range<usize>>,
}

impl std::fmt::Debug for Vocabulary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Vocabulary")
            .field("entries", &self.entries.len())
            .field("bytes", &self.raw.len())
            .finish()
    }
}

impl Vocabulary {
    pub(crate) fn parse(
        raw: String,
        expected_entries: u32,
        expected_sha256: [u8; 32],
    ) -> Result<Self, OnnxBackendFault> {
        if usize::try_from(expected_entries).ok() != Some(ENTRIES)
            || Sha256::digest(raw.as_bytes()).as_slice() != expected_sha256
        {
            return Err(OnnxBackendFault::GraphMismatch);
        }

        let bytes = raw.as_bytes();
        let mut entries = Vec::with_capacity(ENTRIES);
        let mut start = 0usize;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte == b'\n' {
                let end = index
                    .checked_sub(usize::from(index > start && bytes[index - 1] == b'\r'))
                    .ok_or(OnnxBackendFault::GraphMismatch)?;
                entries.push(start..end);
                start = index + 1;
            }
        }
        if start < bytes.len() {
            let end = bytes
                .len()
                .checked_sub(usize::from(bytes.last() == Some(&b'\r')))
                .ok_or(OnnxBackendFault::GraphMismatch)?;
            entries.push(start..end);
        }
        if entries.len() != ENTRIES
            || entries.iter().any(|range| range.is_empty())
            || entries
                .iter()
                .any(|range| !raw.is_char_boundary(range.start) || !raw.is_char_boundary(range.end))
        {
            return Err(OnnxBackendFault::GraphMismatch);
        }

        Ok(Self { raw, entries })
    }

    pub(crate) const fn classes() -> usize {
        CLASSES
    }

    pub(crate) fn token(&self, class: usize) -> Option<&str> {
        match class {
            0 => None,
            1..=ENTRIES => self
                .entries
                .get(class - 1)
                .map(|range| &self.raw[range.clone()]),
            CLASSES_MINUS_ONE => Some(" "),
            _ => None,
        }
    }
}

const CLASSES_MINUS_ONE: usize = CLASSES - 1;

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{ENTRIES, Vocabulary};

    #[test]
    fn compact_vocabulary_maps_blank_entries_and_appended_space() {
        let raw = (0..ENTRIES)
            .map(|index| format!("x{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let digest: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
        let vocabulary = Vocabulary::parse(
            raw,
            u32::try_from(ENTRIES).expect("entry count fits"),
            digest,
        )
        .expect("valid vocabulary");

        assert_eq!(vocabulary.token(0), None);
        assert_eq!(vocabulary.token(1), Some("x0"));
        assert_eq!(vocabulary.token(ENTRIES), Some("x18707"));
        assert_eq!(vocabulary.token(ENTRIES + 1), Some(" "));
        assert_eq!(vocabulary.token(ENTRIES + 2), None);
    }
}
