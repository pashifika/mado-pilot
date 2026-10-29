//! Shared NFC/edge-trim machinery with distinct caller and backend admission.

use std::sync::Arc;

use mado_pilot_core::{Error, Operation, OperationContext, Result, Status};
use unicode_normalization::char::decompose_canonical;
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};

use crate::normalization_bound::MAX_CANONICAL_SCALARS;
use crate::{MAX_BACKEND_TEXT_BYTES, MAX_TEXT_BYTES, OcrFault};

/// Normalizes one caller literal without retaining raw text or bounding its edges.
///
/// # Errors
///
/// Returns invalid argument for empty text, limit exceeded for more than 4096
/// normalized UTF-8 bytes, or the operation's interruption. Errors contain no text.
pub fn normalize_literal(text: &str, operation: &OperationContext) -> Result<Arc<str>> {
    let mut attempt = Operation::admit(operation)?;
    let outcome = (|| {
        let trimmed = trim_edges(text, &mut attempt)?;
        if trimmed.is_empty() {
            return Err(Error::new(
                Status::InvalidArgument,
                "OCR watch literal is empty after normalization",
            ));
        }
        normalize_trimmed(trimmed, &mut attempt, literal_above_ceiling)
    })();
    attempt.checkpoint()?;
    attempt.commit(outcome?).map_err(Error::from)
}

pub(crate) fn normalize_backend(raw: &[u8], operation: &OperationContext) -> Result<Arc<str>> {
    // Backend raw length and UTF-8 failures remain malformed-output outcomes,
    // including oversized whitespace that a caller literal is allowed to trim.
    if raw.len() > MAX_BACKEND_TEXT_BYTES {
        return Err(OcrFault::BackendTextAboveCeiling.into());
    }
    let text = std::str::from_utf8(raw).map_err(|_| Error::from(OcrFault::BackendTextNotUtf8))?;
    let mut attempt = Operation::admit(operation)?;
    let trimmed = trim_edges(text, &mut attempt)?;
    let outcome = normalize_trimmed(trimmed, &mut attempt, backend_above_ceiling);
    attempt.checkpoint()?;
    attempt.commit(outcome?).map_err(Error::from)
}

fn literal_above_ceiling() -> Error {
    Error::new(
        Status::LimitExceeded,
        "OCR watch literal exceeds the normalized byte ceiling",
    )
}

fn backend_above_ceiling() -> Error {
    OcrFault::BackendTextAboveCeiling.into()
}

fn trim_edges<'a>(text: &'a str, attempt: &mut Operation<'_>) -> Result<&'a str> {
    let mut start = text.len();
    for (offset, ch) in text.char_indices() {
        attempt.checkpoint()?;
        if !ch.is_whitespace() {
            start = offset;
            break;
        }
    }
    let mut end = start;
    for (offset, ch) in text[start..].char_indices().rev() {
        attempt.checkpoint()?;
        if !ch.is_whitespace() {
            end = start + offset + ch.len_utf8();
            break;
        }
    }
    Ok(&text[start..end])
}

fn normalize_trimmed(
    text: &str,
    attempt: &mut Operation<'_>,
    above_ceiling: fn() -> Error,
) -> Result<Arc<str>> {
    // The quick check shares the allocation-free decomposition preflight. It
    // may stop early; finish preflight before creating any buffering iterator.
    let mut decomposed = 0_usize;
    let mut failure = None;
    let normalized = {
        let mut checked = text
            .chars()
            .map_while(|ch| {
                let step = (|| {
                    attempt.checkpoint()?;
                    let mut count = 0_usize;
                    decompose_canonical(ch, |_| count += 1);
                    decomposed = decomposed
                        .checked_add(count)
                        .filter(|&count| count <= MAX_CANONICAL_SCALARS)
                        .ok_or_else(above_ceiling)?;
                    Ok::<_, Error>(ch)
                })();
                match step {
                    Ok(ch) => Some(ch),
                    Err(error) => {
                        failure = Some(error);
                        None
                    }
                }
            })
            .fuse();
        let normalized = is_nfc_quick(checked.by_ref());
        for _ in checked {}
        normalized
    };
    attempt.checkpoint()?;
    if let Some(error) = failure {
        return Err(error);
    }
    if normalized == IsNormalized::Yes {
        if text.len() > MAX_TEXT_BYTES {
            return Err(above_ceiling());
        }
        return Ok(Arc::from(text));
    }

    let context = attempt.context();
    let mut interruption = None;
    let outcome = {
        let checked = text
            .chars()
            .map_while(|ch| {
                if let Some(error) = context.interruption() {
                    interruption = Some(error);
                    None
                } else {
                    Some(ch)
                }
            })
            .fuse();
        collect_bounded(checked.nfc(), attempt, above_ceiling)
    };
    if let Some(error) = interruption {
        return Err(error.into());
    }
    outcome
}

fn collect_bounded(
    mut normalized: impl Iterator<Item = char>,
    attempt: &mut Operation<'_>,
    above_ceiling: fn() -> Error,
) -> Result<Arc<str>> {
    let mut scratch = [0_u8; MAX_TEXT_BYTES];
    let mut used = 0_usize;
    let mut last_non_whitespace = 0;
    let mut overflow = false;
    loop {
        attempt.checkpoint()?;
        let Some(ch) = normalized.next() else { break };
        attempt.checkpoint()?;
        let whitespace = ch.is_whitespace();
        if used == 0 && whitespace {
            continue;
        }
        let end = used.checked_add(ch.len_utf8()).ok_or_else(above_ceiling)?;
        if !overflow && end <= MAX_TEXT_BYTES {
            ch.encode_utf8(&mut scratch[used..end]);
            used = end;
        } else {
            // Excess trailing whitespace needs no storage. A later non-space
            // proves it was interior and that the final text is over the limit.
            overflow = true;
        }
        if !whitespace {
            if overflow {
                return Err(above_ceiling());
            }
            last_non_whitespace = used;
        }
    }
    let text = std::str::from_utf8(&scratch[..last_non_whitespace])
        .expect("normalization scratch contains complete UTF-8 encodings");
    Ok(Arc::from(text))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use mado_pilot_core::{CancellationToken, Clock, MonotonicInstant, OperationContext, Status};

    use super::{MAX_CANONICAL_SCALARS, MAX_TEXT_BYTES, normalize_literal};

    #[test]
    fn literal_accepts_arbitrary_unicode_edges_without_a_raw_byte_limit() {
        let edge = "\u{2000}\u{2001}\u{3000}\t\n".repeat(20_000);
        let literal = format!("{edge}e\u{301}{edge}");
        assert_eq!(
            &*normalize_literal(&literal, &OperationContext::new()).unwrap(),
            "é"
        );
        assert_eq!(
            normalize_literal(&edge, &OperationContext::new())
                .unwrap_err()
                .status(),
            Status::InvalidArgument
        );
    }

    #[test]
    fn literal_byte_limit_is_applied_after_composition_and_expansion() {
        let context = OperationContext::new();
        let already_normalized = "a".repeat(MAX_TEXT_BYTES);
        assert_eq!(
            &*normalize_literal(&already_normalized, &context).unwrap(),
            already_normalized
        );
        let decomposed = "e\u{301}".repeat(MAX_TEXT_BYTES / 2);
        assert_eq!(
            &*normalize_literal(&decomposed, &context).unwrap(),
            "é".repeat(MAX_TEXT_BYTES / 2)
        );
        // U+0344 is composition-excluded and expands into two combining marks.
        let expanding = "\u{0344}".repeat(MAX_TEXT_BYTES / 4);
        assert_eq!(
            &*normalize_literal(&expanding, &context).unwrap(),
            "\u{0308}\u{0301}".repeat(MAX_TEXT_BYTES / 4)
        );
        for overlong in [
            format!("{already_normalized}x"),
            format!("{decomposed}x"),
            format!("{expanding}\u{0344}"),
        ] {
            assert_eq!(
                normalize_literal(&overlong, &context).unwrap_err().status(),
                Status::LimitExceeded
            );
        }
    }

    #[test]
    fn literal_preserves_hangul_composition_at_the_byte_boundary() {
        let context = OperationContext::new();
        let jamo = "\u{1100}\u{1161}\u{11a8}".repeat(MAX_TEXT_BYTES / 3);
        assert_eq!(
            &*normalize_literal(&format!("{jamo}x"), &context).unwrap(),
            format!("{}x", "각".repeat(MAX_TEXT_BYTES / 3))
        );
        assert_eq!(
            normalize_literal(&format!("{jamo}xx"), &context)
                .unwrap_err()
                .status(),
            Status::LimitExceeded
        );
    }

    #[test]
    fn literal_reorders_a_single_long_combining_run_without_stream_safe_joiners() {
        let high = "\u{0315}".repeat(1_023);
        let low = "\u{0316}".repeat(1_024);
        let context = OperationContext::new();
        let input = format!("q{high}{low}x");
        assert_eq!(input.len(), MAX_TEXT_BYTES);
        assert_eq!(
            &*normalize_literal(&input, &context).unwrap(),
            format!("q{low}{high}x")
        );
        let overlong = "\u{0315}".repeat(MAX_CANONICAL_SCALARS + 1);
        assert_eq!(
            normalize_literal(&overlong, &context).unwrap_err().status(),
            Status::LimitExceeded
        );
    }

    #[test]
    fn a_combining_mark_keeps_preceding_whitespace_interior_at_the_byte_limit() {
        let context = OperationContext::new();
        let exact = format!("q{}\u{301}", " ".repeat(MAX_TEXT_BYTES - 3));
        assert_eq!(&*normalize_literal(&exact, &context).unwrap(), exact);
        let overlong = format!("q{}\u{301}", " ".repeat(MAX_TEXT_BYTES - 1));
        assert_eq!(
            normalize_literal(&overlong, &context).unwrap_err().status(),
            Status::LimitExceeded
        );
    }

    #[derive(Debug)]
    struct InterruptingClock {
        calls: AtomicUsize,
        cancellation: Option<CancellationToken>,
    }

    impl Clock for InterruptingClock {
        fn now(&self) -> MonotonicInstant {
            if self.calls.fetch_add(1, Ordering::AcqRel) >= 128 {
                if let Some(cancellation) = &self.cancellation {
                    cancellation.cancel();
                }
                MonotonicInstant::from_origin(Duration::from_secs(1))
            } else {
                MonotonicInstant::ORIGIN
            }
        }
    }

    #[test]
    fn literal_scans_observe_deadlines_and_cancellation_without_partial_results() {
        for text in [
            format!("{}x", " ".repeat(4_096)),
            format!("x{}", " ".repeat(4_096)),
            format!("q{}x", "\u{0315}\u{0316}".repeat(1_024)),
        ] {
            for cancelled in [false, true] {
                let cancellation = CancellationToken::new();
                let clock = Arc::new(InterruptingClock {
                    calls: AtomicUsize::new(0),
                    cancellation: cancelled.then(|| cancellation.clone()),
                });
                let context = OperationContext::new()
                    .with_clock(clock)
                    .with_cancellation(cancellation)
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(
                        if cancelled { 2 } else { 1 },
                    )));
                assert_eq!(
                    normalize_literal(&text, &context).unwrap_err().status(),
                    if cancelled {
                        Status::Cancelled
                    } else {
                        Status::DeadlineExceeded
                    }
                );
            }
        }
    }
}
