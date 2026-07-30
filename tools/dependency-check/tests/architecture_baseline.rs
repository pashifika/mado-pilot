//! Keeps the architecture baseline's contract values from going stale.
//!
//! The root manifest, the `REQUIRED_*` constants, and `rust-toolchain.toml` are
//! checked against each other by the architecture checker, so a contract bump that
//! misses one of them already fails. Nothing reads `docs/architecture.md`, which
//! makes it the one contract location with no mechanical guard: its value table can
//! drift while every gate passes, and that table is exactly what the checker's own
//! `UnexpectedWorkspaceMetadata` diagnostic tells the next contributor to consult.
//!
//! The same reasoning covers the baseline's gate tally, which restates the
//! validation-gate registry as a sentence. That sentence is a derived value with the
//! registry as its source, so resolving a gate in one file and forgetting the other
//! leaves a document that contradicts itself while every gate passes. It has already
//! happened once: the change that resolved `G-002` left the tally at its previous
//! counts, and only a review caught it.
//!
//! These are the only cases in the suite that read repository files, which is why they
//! live apart from the synthetic-input suites in `metadata_policy.rs`.

use std::fs;
use std::path::PathBuf;

use mado_pilot_dependency_check::graph::{
    REQUIRED_EDITION, REQUIRED_LICENSE, REQUIRED_REPOSITORY, REQUIRED_RUST_VERSION,
    REQUIRED_VERSION,
};

/// Workspace-relative location of the architecture baseline, for diagnostics.
const ARCHITECTURE_BASELINE: &str = "docs/architecture.md";

/// Workspace-relative location of the gate registry, for diagnostics.
const VALIDATION_GATES: &str = "docs/validation-gates.md";

/// Resolves a tracked document from the crate directory rather than the process
/// working directory, so the tests pass wherever the test binary is invoked from.
fn tracked_document(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("docs");
    path.push(name);
    path
}

/// Resolves the baseline from the crate directory rather than the process working
/// directory, so the test passes wherever the test binary is invoked from.
fn architecture_baseline() -> PathBuf {
    tracked_document("architecture.md")
}

#[test]
fn every_contract_value_is_documented_in_the_architecture_baseline() {
    let path = architecture_baseline();
    let document = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "`{ARCHITECTURE_BASELINE}` must be readable at `{}`: {error}",
            path.display()
        )
    });

    let contract = [
        (stringify!(REQUIRED_VERSION), REQUIRED_VERSION),
        (stringify!(REQUIRED_EDITION), REQUIRED_EDITION),
        (stringify!(REQUIRED_RUST_VERSION), REQUIRED_RUST_VERSION),
        (stringify!(REQUIRED_LICENSE), REQUIRED_LICENSE),
        (stringify!(REQUIRED_REPOSITORY), REQUIRED_REPOSITORY),
    ];

    for (constant, value) in contract {
        // Each value is searched for as inline code anywhere in the document. The
        // document writes every contract value that way, and the narrower needle is
        // what keeps the check meaningful for values that also read as ordinary
        // prose: `2024` is a plausible year, and the licensing section already names
        // Apache-2.0 in running text, so a bare substring would keep passing after
        // the value table had drifted. Searching the whole document rather than a
        // table cell keeps the check independent of column layout, row order,
        // headings, and line numbers, so reformatting cannot break it.
        let documented = format!("`{value}`");
        assert!(
            document.contains(&documented),
            "`{ARCHITECTURE_BASELINE}` does not document `{constant}`: the value \
             {documented} does not appear as inline code anywhere in the document. \
             This document is the one contract location nothing else reads, so moving a \
             contract value means moving the root `Cargo.toml`, the `REQUIRED_*` constants \
             in `tools/dependency-check/src/graph.rs`, `rust-toolchain.toml`, and \
             `{ARCHITECTURE_BASELINE}` together in the same change"
        );
    }
}

/// Number words the baseline's tally sentence uses, indexed by the count they name.
/// The registry holds fourteen gates, so the table covers every reachable count and a
/// count outside it is itself a finding rather than a lookup failure.
const COUNT_WORDS: [&str; 15] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven", "twelve", "thirteen", "fourteen",
];

/// Resolves a number word to the count it names, ignoring case and trailing prose.
fn count_word(word: &str) -> Option<usize> {
    let word = word.trim().to_ascii_lowercase();
    COUNT_WORDS.iter().position(|candidate| *candidate == word)
}

/// Reads the baseline's gate tally as `(open, deferred, resolved)`.
///
/// The tally is prose rather than a table, and each count carries a verb that agrees
/// with it — "eight remain open", "one is deferred", "five are resolved". Splitting the
/// one sentence on its three nouns and taking the count from each fragment reads the
/// numbers without the test having to reproduce that agreement, so a count changing
/// number cannot fail the test for the wrong reason. A `None` means the sentence no
/// longer has the shape this reads, which is itself worth failing on: the tally would
/// otherwise be unguarded again without anyone noticing.
fn stated_tally(document: &str) -> Option<(usize, usize, usize)> {
    let (before_open, rest) = document.split_once(" remain open, ")?;
    let (deferred_fragment, rest) = rest.split_once(" deferred, and ")?;
    let (resolved_fragment, _) = rest.split_once(" resolved.")?;

    Some((
        count_word(before_open.split_whitespace().next_back()?)?,
        count_word(deferred_fragment.split_whitespace().next()?)?,
        count_word(resolved_fragment.split_whitespace().next()?)?,
    ))
}

/// Tallies the registry table by status, returning `(open, deferred, resolved)`.
///
/// Rows are matched on the gate link that opens every registry row, so prose elsewhere
/// in the document that happens to mention a status cannot be counted. The status is
/// the row's last populated cell; anything that is neither resolved nor deferred counts
/// as open, which is how `G-013` — open per workload, with Phase 1 resolved — lands on
/// the side the sentence puts it.
fn tally_registry(document: &str) -> (usize, usize, usize) {
    let mut open = 0;
    let mut deferred = 0;
    let mut resolved = 0;

    for line in document.lines() {
        let line = line.trim();
        if !line.starts_with("| [`G-") {
            continue;
        }
        let Some(status) = line
            .trim_end_matches('|')
            .rsplit('|')
            .map(str::trim)
            .find(|cell| !cell.is_empty())
        else {
            continue;
        };

        if status.starts_with("Resolved") {
            resolved += 1;
        } else if status.starts_with("Deferred") {
            deferred += 1;
        } else {
            open += 1;
        }
    }

    (open, deferred, resolved)
}

#[test]
fn the_architecture_gate_tally_matches_the_registry_it_summarizes() {
    let registry_path = tracked_document("validation-gates.md");
    let registry = fs::read_to_string(&registry_path).unwrap_or_else(|error| {
        panic!(
            "`{VALIDATION_GATES}` must be readable at `{}`: {error}",
            registry_path.display()
        )
    });
    let baseline_path = architecture_baseline();
    let baseline = fs::read_to_string(&baseline_path).unwrap_or_else(|error| {
        panic!(
            "`{ARCHITECTURE_BASELINE}` must be readable at `{}`: {error}",
            baseline_path.display()
        )
    });

    let (open, deferred, resolved) = tally_registry(&registry);
    assert_eq!(
        open + deferred + resolved,
        14,
        "`{VALIDATION_GATES}` should hold fourteen registry rows, found {}: {open} open, \
         {deferred} deferred, {resolved} resolved. Either a gate was added without \
         extending this expectation, or a row stopped matching the `| [`G-` prefix the \
         tally reads",
        open + deferred + resolved
    );

    let stated = stated_tally(&baseline);

    assert_eq!(
        stated,
        Some((open, deferred, resolved)),
        "`{ARCHITECTURE_BASELINE}` states a gate tally of {stated:?} but \
         `{VALIDATION_GATES}` holds {open} open, {deferred} deferred, and {resolved} \
         resolved. A `None` means the sentence no longer reads \
         `<count> remain open, <count> is deferred, and <count> are resolved.`; a \
         mismatched number means the two documents disagree. Resolving a gate moves \
         the registry row, the summary sentence, and the gate's own section together \
         in the same change"
    );
}
