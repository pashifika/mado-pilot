//! Accepted greedy CTC decoder over the validated embedded vocabulary.

use std::mem::size_of;

use crate::fault::OnnxBackendFault;
use crate::vocabulary::Vocabulary;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecodedText {
    pub(crate) text: String,
    pub(crate) confidence: f64,
}

pub(crate) fn decode(
    shape: &[i64],
    probabilities: &[f32],
    vocabulary: &Vocabulary,
    expected_batch: usize,
    max_text_bytes: usize,
) -> Result<Vec<DecodedText>, OnnxBackendFault> {
    if shape.len() != 3 || shape[0] <= 0 || shape[1] <= 0 || shape[2] <= 0 {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let batch = usize::try_from(shape[0]).map_err(|_| OnnxBackendFault::MalformedOutput)?;
    let time_steps = usize::try_from(shape[1]).map_err(|_| OnnxBackendFault::MalformedOutput)?;
    let classes = usize::try_from(shape[2]).map_err(|_| OnnxBackendFault::MalformedOutput)?;
    if batch != expected_batch || classes != Vocabulary::classes() {
        return Err(OnnxBackendFault::MalformedOutput);
    }
    let elements = batch
        .checked_mul(time_steps)
        .and_then(|value| value.checked_mul(classes))
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    let bytes = elements
        .checked_mul(size_of::<f32>())
        .ok_or(OnnxBackendFault::ResourceLimit)?;
    if elements != probabilities.len() || bytes > crate::MAX_OUTPUT_BYTES {
        return Err(OnnxBackendFault::ResourceLimit);
    }

    let mut decoded = Vec::with_capacity(batch);
    for batch_index in 0..batch {
        let mut text = String::new();
        let mut previous = usize::MAX;
        let mut confidence_sum = 0.0_f64;
        let mut confidence_count = 0usize;
        for time_index in 0..time_steps {
            let offset = (batch_index * time_steps + time_index) * classes;
            let scores = &probabilities[offset..offset + classes];
            let mut selected_index = 0usize;
            let mut selected_score = f32::NEG_INFINITY;
            for (class, score) in scores.iter().copied().enumerate() {
                if !score.is_finite() || !(0.0..=1.0).contains(&score) {
                    return Err(OnnxBackendFault::MalformedOutput);
                }
                if score > selected_score {
                    selected_index = class;
                    selected_score = score;
                }
            }

            let duplicate = selected_index == previous;
            previous = selected_index;
            if duplicate || selected_index == 0 {
                continue;
            }
            let token = vocabulary
                .token(selected_index)
                .ok_or(OnnxBackendFault::MalformedOutput)?;
            if text
                .len()
                .checked_add(token.len())
                .is_none_or(|length| length > max_text_bytes)
            {
                return Err(OnnxBackendFault::ResourceLimit);
            }
            text.push_str(token);
            confidence_sum += round_five(f64::from(selected_score));
            confidence_count += 1;
        }
        let confidence = if confidence_count == 0 {
            0.0
        } else {
            round_five(confidence_sum / confidence_count as f64)
        };
        decoded.push(DecodedText { text, confidence });
    }
    Ok(decoded)
}

fn round_five(value: f64) -> f64 {
    (value * 100_000.0).round_ties_even() / 100_000.0
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::decode;
    use crate::fault::OnnxBackendFault;
    use crate::vocabulary::Vocabulary;

    fn vocabulary() -> Vocabulary {
        let raw = (0..18_708)
            .map(|index| format!("x{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let digest: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
        Vocabulary::parse(raw, 18_708, digest).expect("valid vocabulary")
    }

    #[test]
    fn greedy_ctc_removes_blanks_and_adjacent_duplicates() {
        let vocabulary = vocabulary();
        let classes = Vocabulary::classes();
        let mut output = vec![0.0_f32; 5 * classes];
        for (step, (class, score)) in [(1, 0.9), (1, 0.8), (0, 1.0), (2, 0.7), (2, 0.6)]
            .into_iter()
            .enumerate()
        {
            output[step * classes + class] = score;
        }
        let decoded = decode(
            &[1, 5, i64::try_from(classes).expect("classes")],
            &output,
            &vocabulary,
            1,
            64,
        )
        .expect("valid output");

        assert_eq!(decoded[0].text, "x0x1");
        assert_eq!(decoded[0].confidence, 0.8);
    }

    #[test]
    fn malformed_numeric_output_is_rejected() {
        let vocabulary = vocabulary();
        let classes = Vocabulary::classes();
        let mut output = vec![0.0_f32; classes];
        output[3] = f32::NAN;

        assert_eq!(
            decode(
                &[1, 1, i64::try_from(classes).expect("classes")],
                &output,
                &vocabulary,
                1,
                64,
            ),
            Err(OnnxBackendFault::MalformedOutput)
        );
    }

    #[test]
    fn wrong_output_rank_is_rejected_before_indexing() {
        let vocabulary = vocabulary();
        assert_eq!(
            decode(&[1, 18_710], &[], &vocabulary, 1, 64),
            Err(OnnxBackendFault::MalformedOutput)
        );
    }
}
