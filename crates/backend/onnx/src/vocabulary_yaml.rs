//! Bounded event parsing of a verified Paddle inference-YAML CTC dictionary.
//!
//! The caller passes the same captured bytes whose length and SHA-256 were
//! verified. Only `PostProcess.character_dict` is read; other sections are
//! structurally checked and skipped, and their aliases are never resolved.

use std::collections::HashSet;

use yaml_rust2::parser::{Event, Parser};

use crate::fault::OnnxBackendFault;
use crate::vocabulary::Vocabulary;

pub(crate) const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_CANONICAL_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_EVENTS: usize = 100_000;
const MAX_SCALAR_BYTES: usize = 16 * 1024;
const SECTION_KEY: &str = "PostProcess";
const DICTIONARY_KEY: &str = "character_dict";

/// Builds the declared vocabulary from one captured, digest-verified YAML resource.
///
/// Structure, entry count, and the canonical LF-joined digest must all match.
pub(crate) fn vocabulary(
    source: &[u8],
    entries: u32,
    sha256: [u8; 32],
) -> Result<Vocabulary, OnnxBackendFault> {
    let expected_entries =
        usize::try_from(entries).map_err(|_| OnnxBackendFault::VocabularyMismatch)?;
    let canonical = canonical_dictionary(source, expected_entries)?;
    Vocabulary::parse(canonical, entries, sha256).map_err(|_| OnnxBackendFault::VocabularyMismatch)
}

/// Returns the ordered entries joined by LF without a terminal LF.
fn canonical_dictionary(
    source: &[u8],
    expected_entries: usize,
) -> Result<String, OnnxBackendFault> {
    parse(source, expected_entries).map_err(|Rejected| OnnxBackendFault::VocabularyMismatch)
}

struct Rejected;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Root,
    Section,
    Dictionary,
    Ignored,
}

enum Kind {
    Mapping {
        keys: HashSet<String>,
        pending_value: Option<Role>,
    },
    Sequence,
}

struct Frame {
    role: Role,
    kind: Kind,
}

struct Events<'a> {
    parser: Parser<std::str::Chars<'a>>,
    consumed: usize,
}

impl Events<'_> {
    fn next(&mut self) -> Result<Event, Rejected> {
        self.consumed += 1;
        if self.consumed > MAX_EVENTS {
            return Err(Rejected);
        }
        self.parser
            .next_token()
            .map(|(event, _)| event)
            .map_err(|_| Rejected)
    }

    fn expect(&mut self, expected: &Event) -> Result<(), Rejected> {
        if self.next()? == *expected {
            Ok(())
        } else {
            Err(Rejected)
        }
    }
}

struct Dictionary {
    canonical: String,
    entries: usize,
    expected_entries: usize,
    seen: bool,
}

impl Dictionary {
    fn push(&mut self, event: Event) -> Result<(), Rejected> {
        let Event::Scalar(entry, _, 0, None) = event else {
            return Err(Rejected);
        };
        if entry.is_empty()
            || entry.len() > MAX_SCALAR_BYTES
            || entry.chars().any(char::is_control)
            || self.entries == self.expected_entries
        {
            return Err(Rejected);
        }
        let separator = usize::from(self.entries != 0);
        if self
            .canonical
            .len()
            .checked_add(separator)
            .and_then(|length| length.checked_add(entry.len()))
            .is_none_or(|length| length > MAX_CANONICAL_BYTES)
        {
            return Err(Rejected);
        }
        if separator != 0 {
            self.canonical.push('\n');
        }
        self.canonical.push_str(&entry);
        self.entries += 1;
        Ok(())
    }
}

fn parse(source: &[u8], expected_entries: usize) -> Result<String, Rejected> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(Rejected);
    }
    let text = std::str::from_utf8(source).map_err(|_| Rejected)?;
    let mut events = Events {
        parser: Parser::new_from_str(text),
        consumed: 0,
    };
    let mut dictionary = Dictionary {
        canonical: String::with_capacity(source.len().min(MAX_CANONICAL_BYTES)),
        entries: 0,
        expected_entries,
        seen: false,
    };
    let mut frames = Vec::<Frame>::new();

    events.expect(&Event::StreamStart)?;
    events.expect(&Event::DocumentStart)?;
    open_node(events.next()?, Role::Root, &mut frames, &mut dictionary)?;
    while let Some(frame) = frames.last_mut() {
        let event = events.next()?;
        let role = frame.role;
        let child = match &mut frame.kind {
            Kind::Sequence if event == Event::SequenceEnd => None,
            Kind::Sequence if role == Role::Dictionary => {
                dictionary.push(event)?;
                continue;
            }
            Kind::Sequence => Some((event, Role::Ignored)),
            Kind::Mapping {
                keys,
                pending_value,
            } => match (pending_value.take(), event) {
                (Some(value_role), event) => Some((event, value_role)),
                (None, Event::MappingEnd) => None,
                (None, Event::Scalar(key, _, 0, None)) if key.len() <= MAX_SCALAR_BYTES => {
                    let value_role = match (role, key.as_str()) {
                        (Role::Root, SECTION_KEY) => Role::Section,
                        (Role::Section, DICTIONARY_KEY) => Role::Dictionary,
                        _ => Role::Ignored,
                    };
                    if !keys.insert(key) {
                        return Err(Rejected);
                    }
                    *pending_value = Some(value_role);
                    continue;
                }
                // Collection, alias, anchored, tagged, or oversized keys.
                (None, _) => return Err(Rejected),
            },
        };
        match child {
            None => {
                frames.pop();
            }
            Some((event, child_role)) => {
                open_node(event, child_role, &mut frames, &mut dictionary)?;
            }
        }
    }
    events.expect(&Event::DocumentEnd)?;
    events.expect(&Event::StreamEnd)?;
    if !dictionary.seen || dictionary.entries != dictionary.expected_entries {
        return Err(Rejected);
    }
    Ok(dictionary.canonical)
}

fn open_node(
    event: Event,
    role: Role,
    frames: &mut Vec<Frame>,
    dictionary: &mut Dictionary,
) -> Result<(), Rejected> {
    let kind = match (event, role) {
        (Event::MappingStart(0, None), Role::Root | Role::Section) => Kind::Mapping {
            keys: HashSet::new(),
            pending_value: None,
        },
        (Event::MappingStart(_, None), Role::Ignored) => Kind::Mapping {
            keys: HashSet::new(),
            pending_value: None,
        },
        (Event::SequenceStart(0, None), Role::Dictionary) if !dictionary.seen => {
            dictionary.seen = true;
            Kind::Sequence
        }
        (Event::SequenceStart(_, None), Role::Ignored) => Kind::Sequence,
        (Event::Scalar(value, _, _, None), Role::Ignored) if value.len() <= MAX_SCALAR_BYTES => {
            return Ok(());
        }
        // Ignored aliases are retained as opaque events and never expanded.
        (Event::Alias(_), Role::Ignored) => return Ok(()),
        _ => return Err(Rejected),
    };
    if frames.len() >= MAX_DEPTH {
        return Err(Rejected);
    }
    frames.push(Frame { role, kind });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use sha2::{Digest, Sha256};

    use super::{MAX_EVENTS, MAX_SCALAR_BYTES, MAX_SOURCE_BYTES, canonical_dictionary, vocabulary};
    use crate::fault::OnnxBackendFault;
    use crate::vocabulary::Vocabulary;

    /// Mirrors the official file's ignored `Hpi` anchor and alias.
    const HPI: &str = concat!(
        "Hpi:\n",
        "  backend_configs:\n",
        "    paddle_infer:\n",
        "      trt_dynamic_shapes: &id001\n",
        "        x:\n",
        "        - - 1\n",
        "          - 3\n",
        "    tensorrt:\n",
        "      dynamic_shapes: *id001\n",
    );

    fn document(entries: &[&str]) -> String {
        let mut yaml = format!(
            "Global:\n  model_name: PP-OCRv6_small_rec\n{HPI}PostProcess:\n  name: CTCLabelDecode\n"
        );
        yaml.push_str("  character_dict:\n");
        for entry in entries {
            writeln!(yaml, "  - '{}'", entry.replace('\'', "''")).unwrap();
        }
        yaml
    }

    fn rejected(source: &str, expected_entries: usize) {
        assert_eq!(
            canonical_dictionary(source.as_bytes(), expected_entries),
            Err(OnnxBackendFault::VocabularyMismatch),
            "{source:.120}"
        );
    }

    #[test]
    fn official_shape_skips_unexpanded_hpi_aliases_and_joins_exact_entries() {
        let source = document(&["!", "'", "a", "魔", "\\", "#"]);
        assert_eq!(
            canonical_dictionary(source.as_bytes(), 6).as_deref(),
            Ok("!\n'\na\n魔\n\\\n#")
        );
        let flow = "PostProcess: {character_dict: [\"x\", y, 'z']}\n";
        assert_eq!(
            canonical_dictionary(flow.as_bytes(), 3).as_deref(),
            Ok("x\ny\nz")
        );
    }

    #[test]
    fn full_count_dictionary_builds_only_the_declared_digest() {
        let mut entries = (0..Vocabulary::entries())
            .map(|index| format!("x{index}"))
            .collect::<Vec<_>>();
        let borrowed = entries.iter().map(String::as_str).collect::<Vec<_>>();
        let source = document(&borrowed);
        let digest: [u8; 32] = Sha256::digest(borrowed.join("\n").as_bytes()).into();
        let count = u32::try_from(Vocabulary::entries()).unwrap();

        let built = vocabulary(source.as_bytes(), count, digest).expect("declared vocabulary");
        assert_eq!(built.token(0), None);
        assert_eq!(built.token(1), Some("x0"));
        assert_eq!(built.token(Vocabulary::entries()), Some("x18707"));
        assert_eq!(built.token(Vocabulary::classes() - 1), Some(" "));

        let mut wrong_digest = digest;
        wrong_digest[0] ^= 1;
        assert_eq!(
            vocabulary(source.as_bytes(), count, wrong_digest).map(|_| ()),
            Err(OnnxBackendFault::VocabularyMismatch)
        );
        assert_eq!(
            vocabulary(source.as_bytes(), count - 1, digest).map(|_| ()),
            Err(OnnxBackendFault::VocabularyMismatch)
        );

        entries.swap(0, 1);
        let reordered = entries.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(
            vocabulary(document(&reordered).as_bytes(), count, digest).map(|_| ()),
            Err(OnnxBackendFault::VocabularyMismatch),
            "order is part of the canonical digest"
        );
    }

    #[test]
    fn count_mismatch_and_missing_dictionary_are_refused() {
        rejected(&document(&["a", "b"]), 3);
        rejected(&document(&["a", "b", "c", "d"]), 3);
        rejected("Global:\n  x: 1\n", 1);
        rejected("PostProcess:\n  name: CTCLabelDecode\n", 1);
        rejected("character_dict:\n- a\n", 1);
        rejected("PostProcess:\n  Inner:\n    character_dict:\n    - a\n", 1);
        rejected("", 1);
    }

    #[test]
    fn malformed_structure_is_refused() {
        for source in [
            "PostProcess: [",
            "PostProcess:\n  character_dict:\n  - a\n - b\n",
            "PostProcess:\n  character_dict: a\n",
            "PostProcess: [a]\n",
            "- PostProcess\n",
            "PostProcess:\n  character_dict:\n  - [a]\n",
            "PostProcess:\n  character_dict:\n  - {a: b}\n",
            "Global: *missing\nPostProcess:\n  character_dict:\n  - a\n",
        ] {
            rejected(source, 1);
        }
    }

    #[test]
    fn duplicate_and_complex_keys_are_refused_in_every_section() {
        for source in [
            "PostProcess:\n  character_dict:\n  - a\nPostProcess:\n  character_dict:\n  - a\n",
            "PostProcess:\n  character_dict:\n  - a\n  character_dict:\n  - a\n",
            "Global:\n  x: 1\n  x: 2\nPostProcess:\n  character_dict:\n  - a\n",
            "? [x]\n: 1\nPostProcess:\n  character_dict:\n  - a\n",
            "? {x: 1}\n: 1\nPostProcess:\n  character_dict:\n  - a\n",
            "Global: &k x\n*k : 1\nPostProcess:\n  character_dict:\n  - a\n",
            "&k Global: 1\nPostProcess:\n  character_dict:\n  - a\n",
        ] {
            rejected(source, 1);
        }
    }

    #[test]
    fn tags_documents_and_dictionary_aliases_are_refused() {
        for source in [
            "PostProcess:\n  character_dict:\n  - !!str a\n",
            "Global: !custom 1\nPostProcess:\n  character_dict:\n  - a\n",
            "Global: !!map {x: 1}\nPostProcess:\n  character_dict:\n  - a\n",
            "PostProcess: !!map\n  character_dict:\n  - a\n",
            "PostProcess:\n  character_dict:\n  - a\n---\nPostProcess:\n  character_dict:\n  - a\n",
            "Global: &a z\nPostProcess:\n  character_dict:\n  - *a\n",
            "PostProcess:\n  character_dict:\n  - &a a\n",
            "PostProcess:\n  character_dict: &d\n  - a\n",
            "Global: &d [a]\nPostProcess:\n  character_dict: *d\n",
            "PostProcess: &p\n  character_dict:\n  - a\n",
            "Global: &p {character_dict: [a]}\nPostProcess: *p\n",
        ] {
            rejected(source, 1);
        }
    }

    #[test]
    fn empty_control_and_line_break_entries_are_refused() {
        for entry in [
            "''",
            "\"\"",
            "\"\\t\"",
            "\"a\\0\"",
            "\"a\\nb\"",
            "\"a\\rb\"",
            "\"\\x7f\"",
            "\"\\u0085\"",
        ] {
            rejected(
                &format!("PostProcess:\n  character_dict:\n  - {entry}\n"),
                1,
            );
        }
        rejected("PostProcess:\n  character_dict:\n  -\n", 1);
    }

    #[test]
    fn source_scalar_depth_event_and_encoding_bounds_are_finite() {
        let oversized_entry = "a".repeat(MAX_SCALAR_BYTES + 1);
        rejected(&document(&[&oversized_entry]), 1);
        let maximum_entry = "a".repeat(MAX_SCALAR_BYTES);
        assert!(canonical_dictionary(document(&[&maximum_entry]).as_bytes(), 1).is_ok());
        rejected(
            &format!("Global: '{oversized_entry}'\nPostProcess:\n  character_dict:\n  - a\n"),
            1,
        );

        let mut padded = document(&["a"]);
        padded.push_str(&"#".repeat(MAX_SOURCE_BYTES + 1 - padded.len()));
        rejected(&padded, 1);

        let nested = |depth: usize| {
            format!(
                "Global: {}x{}\nPostProcess:\n  character_dict:\n  - a\n",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        // The root mapping occupies one of the 32 frames.
        assert!(canonical_dictionary(nested(31).as_bytes(), 1).is_ok());
        rejected(&nested(32), 1);

        let wide = |items: usize| {
            format!(
                "Global: [{}]\nPostProcess:\n  character_dict:\n  - a\n",
                vec!["0"; items].join(",")
            )
        };
        assert!(canonical_dictionary(wide(MAX_EVENTS - 16).as_bytes(), 1).is_ok());
        rejected(&wide(MAX_EVENTS), 1);

        let mut invalid = document(&["a"]).into_bytes();
        invalid.push(0xff);
        assert_eq!(
            canonical_dictionary(&invalid, 1),
            Err(OnnxBackendFault::VocabularyMismatch)
        );
    }
}
