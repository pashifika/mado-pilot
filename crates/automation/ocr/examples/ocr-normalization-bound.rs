//! Exhaustive, model-free derivation of the pinned OCR literal admission bound.
//!
//! Run with `cargo run --locked -p mado-pilot-ocr --example ocr-normalization-bound
//! --release --target-dir <new-proof-target-root>`. No native OCR is loaded.

use std::mem::size_of;

use mado_pilot_core::{OperationContext, Status};
use mado_pilot_ocr::{MAX_TEXT_BYTES, watch_support::normalize_literal};
use unicode_normalization::char::{canonical_combining_class, compose, decompose_canonical};
use unicode_normalization::{UNICODE_VERSION, UnicodeNormalization};

#[path = "../src/normalization_bound.rs"]
mod bound;

fn scalars() -> impl Iterator<Item = char> {
    (0..=0x10_ffff).filter_map(char::from_u32)
}

fn main() {
    let whitespace: Vec<char> = scalars().filter(|ch| ch.is_whitespace()).collect();
    let mut scalar_count = 0_usize;
    let mut maximum = 0_usize;
    let mut maximum_scalar = '\0';
    let mut hangul_maximum = 0_usize;
    let mut hangul_count = 0_usize;
    let mut whitespace_decompositions = 0_usize;

    for ch in scalars() {
        scalar_count += 1;
        let is_whitespace = ch.is_whitespace();
        let mut count = 0_usize;
        decompose_canonical(ch, |decomposed| {
            count += 1;
            assert_eq!(
                decomposed.is_whitespace(),
                is_whitespace,
                "canonical decomposition changes whitespace membership at U+{:04X}",
                u32::from(ch)
            );
            if is_whitespace {
                assert_eq!(canonical_combining_class(decomposed), 0);
                whitespace_decompositions += 1;
            }
            // Confirm that the public callback emits fully decomposed scalars,
            // not one level of a recursively larger canonical decomposition.
            let mut terminal_count = 0;
            decompose_canonical(decomposed, |terminal| {
                assert_eq!(terminal, decomposed);
                terminal_count += 1;
            });
            assert_eq!(terminal_count, 1);
        });
        assert!(count > 0);
        if count > maximum {
            maximum = count;
            maximum_scalar = ch;
        }
        if (0xac00..0xd7a4).contains(&u32::from(ch)) {
            hangul_count += 1;
            hangul_maximum = hangul_maximum.max(count);
            let expected = if (u32::from(ch) - 0xac00) % 28 == 0 {
                2
            } else {
                3
            };
            assert_eq!(count, expected);
        }

        // Cover every possible other operand, including algorithmic Hangul,
        // without depending on the private composition-table representation.
        for &edge in &whitespace {
            assert!(compose(edge, ch).is_none());
            assert!(compose(ch, edge).is_none());
        }
    }

    assert_eq!(UNICODE_VERSION, (17, 0, 0));
    assert_eq!(scalar_count, 1_112_064);
    assert_eq!(maximum, bound::CANONICAL_DECOMPOSITION_BOUND);
    assert_eq!(hangul_count, 11_172);
    assert_eq!(hangul_maximum, 3);
    assert_eq!(whitespace.len(), 25);
    assert_eq!(whitespace_decompositions, whitespace.len());
    assert_eq!(bound::MAX_CANONICAL_SCALARS, 16_384);
    prove_boundaries(&whitespace);

    let maximum_buffer_capacity = bound::MAX_CANONICAL_SCALARS.checked_mul(2).unwrap();
    let maximum_reallocation_entries = bound::MAX_CANONICAL_SCALARS.checked_mul(3).unwrap();
    let buffers_with_reallocation = maximum_reallocation_entries
        .checked_mul(size_of::<(u8, char)>() + size_of::<char>())
        .unwrap();
    let sort_scratch_bytes = bound::MAX_CANONICAL_SCALARS
        .max(48)
        .checked_mul(size_of::<(u8, char)>())
        .unwrap();
    println!("unicode_normalization=0.1.25 unicode_tables={UNICODE_VERSION:?}");
    println!("rust_unicode_tables={:?}", std::char::UNICODE_VERSION);
    println!("scalar_count={scalar_count} maximum_decomposition={maximum}");
    println!("maximum_witness=U+{:04X}", u32::from(maximum_scalar));
    println!("hangul_syllables={hangul_count} hangul_maximum={hangul_maximum}");
    println!(
        "whitespace_scalars={} whitespace_decomposition_scalars={whitespace_decompositions}",
        whitespace.len()
    );
    println!("whitespace_compositions=0 non_whitespace_decomposing_to_whitespace=0");
    println!("trim_equivalence=proved_from_exhaustive_boundary_facts");
    println!(
        "maximum_admitted_decomposed_scalars={}",
        bound::MAX_CANONICAL_SCALARS
    );
    println!("each_nfc_buffer_capacity_bound={maximum_buffer_capacity}");
    println!(
        "both_buffers_including_reallocation_requested_bytes_bound={buffers_with_reallocation}"
    );
    println!("sort_heap_scratch_requested_bytes_bound={sort_scratch_bytes}");
    println!("output_scratch_bytes={MAX_TEXT_BYTES} boundary_cases=passed");
    println!("scope=requested_allocation_bounds_not_allocator_metadata_or_RSS_or_recoverable_OOM");
}

fn prove_boundaries(whitespace: &[char]) {
    let context = OperationContext::new();
    let cases = [
        "e\u{301}".repeat(MAX_TEXT_BYTES / 2),
        format!("{}x", "\u{1100}\u{1161}\u{11a8}".repeat(MAX_TEXT_BYTES / 3)),
        "\u{0344}".repeat(MAX_TEXT_BYTES / 4),
        format!("q{}{}x", "\u{0315}".repeat(1_023), "\u{0316}".repeat(1_024)),
        "\u{1f82}".repeat(MAX_TEXT_BYTES / 3),
    ];
    for case in &cases {
        let expected: String = case.nfc().collect();
        assert_eq!(
            &*normalize_literal(case, &context).unwrap(),
            expected.trim()
        );
        for &edge in whitespace {
            let padded = format!("{edge}{case}{edge}");
            let original_rule: String = padded.nfc().collect();
            assert_eq!(
                &*normalize_literal(&padded, &context).unwrap(),
                original_rule.trim()
            );
        }
    }
    let padding = "\u{2000}".repeat(100_000);
    assert_eq!(
        &*normalize_literal(&format!("{padding}e\u{301}{padding}"), &context).unwrap(),
        "é"
    );
    assert_eq!(
        normalize_literal(&padding, &context).unwrap_err().status(),
        Status::InvalidArgument
    );
    assert_eq!(
        normalize_literal(
            &"\u{0315}".repeat(bound::MAX_CANONICAL_SCALARS + 1),
            &context
        )
        .unwrap_err()
        .status(),
        Status::LimitExceeded
    );
    assert_eq!(
        normalize_literal(&format!("{}x", cases[0]), &context)
            .unwrap_err()
            .status(),
        Status::LimitExceeded
    );
}
