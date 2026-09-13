//! Revision-bound NFC admission proof, exercised by `ocr-normalization-bound`.
//!
//! For unicode-normalization 0.1.25 (Unicode 17.0.0), every scalar canonically
//! decomposes to 1..=D scalars, including Hangul. NFC preserves that decomposition.
//! A <=4096-byte NFC string has <=4096 scalars, hence <=4096*D decomposed scalars:
//! rejecting a larger decomposition cannot exclude an admissible literal.
//!
//! The derivation also checks that whitespace decomposes only to class-zero
//! whitespace, non-whitespace never decomposes to whitespace, and no canonical
//! composition consumes whitespace. Ordering cannot cross a class-zero edge;
//! therefore trimming borrowed edges before NFC equals trimming after NFC.
//!
//! At N admitted decomposed scalars, the pinned NFC iterators each buffer <=N
//! entries. Tinyvec 1.12.0 / Rust 1.97.1 request capacities <=2*max(N,4), with
//! <=3*max(N,4) entries during realloc. Stable sort requests <=max(N,48) pairs
//! plus fixed stack scratch. Thus even a single combining run has bounded
//! requested allocation and O(N log N) ordering work; allocator metadata/RSS and
//! recoverable OOM are not promised. Input/output checkpoints surround that
//! bounded internal work. Output scratch is exactly 4096 bytes, never a growing
//! String; the sole retained text allocation is the final Arc.

pub(crate) const CANONICAL_DECOMPOSITION_BOUND: usize = 4;
pub(crate) const MAX_CANONICAL_SCALARS: usize =
    match crate::MAX_TEXT_BYTES.checked_mul(CANONICAL_DECOMPOSITION_BOUND) {
        Some(bound) => bound,
        None => panic!("OCR normalization bound must be representable"),
    };

const _: () = {
    let version = unicode_normalization::UNICODE_VERSION;
    assert!(version.0 == 17 && version.1 == 0 && version.2 == 0);
};
