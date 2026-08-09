//! The `[benchmark]` keys the harness emits against the ones the committed
//! profiles carry.
//!
//! `docs/performance.md` says a committed profile is the harness's output with
//! budgets added. That was not true: the harness emitted `budgets_set`, which no
//! profile carried, and omitted `measurements_recorded`, which profiles carry, so
//! the output could not be turned into a profile by adding budgets to it. The
//! two key sets are the same set now, and this is what keeps them that way —
//! nothing else compares them, and a reader diffing one file against the other
//! is how the last divergence went unnoticed.
//!
//! The values are deliberately not compared. `status` and `normative` are the
//! two answers that differ between harness output and a recorded profile, and
//! that difference is the point of both keys.
//!
//! The files are read at compile time with `include_str!`, so a profile that is
//! renamed or removed fails the build rather than the assertion.

use mado_pilot_testkit::bench_harness::{Benchmark, benchmark_block};

/// Every committed benchmark profile, by repository path and content.
///
/// `example-synthetic.toml` is deliberately absent for the reason
/// `hard_budget_drift.rs` states: it documents the format with invented numbers
/// rather than recording either a measurement or an explicit native evidence gap.
const PROFILES: [(&str, &str); 8] = [
    (
        "docs/benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-1-c-boundary-aarch64-apple-darwin.toml",
        include_str!("../../../../docs/benchmarks/phase-1-c-boundary-aarch64-apple-darwin.toml"),
    ),
    (
        "docs/benchmarks/phase-1-c-boundary-x86_64-pc-windows-msvc.toml",
        include_str!("../../../../docs/benchmarks/phase-1-c-boundary-x86_64-pc-windows-msvc.toml"),
    ),
    (
        "docs/benchmarks/phase-2-native-capture-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-capture-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-native-transitions-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-transitions-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-native-input-aarch64-apple-darwin.toml",
        include_str!("../../../../docs/benchmarks/phase-2-native-input-aarch64-apple-darwin.toml"),
    ),
    (
        "docs/benchmarks/phase-2-native-x86_64-pc-windows-msvc-evidence-gap.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-x86_64-pc-windows-msvc-evidence-gap.toml"
        ),
    ),
];

/// Returns the assigned keys of `profile`'s `[benchmark]` table, in file order.
///
/// A line reader rather than a TOML parser, as in `hard_budget_drift.rs`: the
/// comparison is one table's key set. The next table header ends the block, so a
/// key written after it belongs to that table and is not counted here.
fn benchmark_keys(profile: &str) -> Vec<&str> {
    let mut keys = Vec::new();
    let mut inside = false;

    for line in profile.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[benchmark]";
            continue;
        }
        if !inside || line.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            keys.push(key.trim());
        }
    }

    keys
}

/// A benchmark identity, so the block can be built. Its values are not read.
fn benchmark() -> Benchmark {
    Benchmark {
        id: "phase-1-key-comparison",
        workload: "the keys, not the numbers",
        phase: "1",
    }
}

#[test]
fn the_harness_emits_exactly_the_benchmark_keys_a_committed_profile_carries() {
    let emitted: Vec<&str> = benchmark_block(&benchmark())
        .iter()
        .map(|(key, _)| *key)
        .collect();

    for (path, profile) in PROFILES {
        assert_eq!(
            benchmark_keys(profile),
            emitted,
            "{path} and the harness disagree about the `[benchmark]` block, so \
             the harness's output is not that file with budgets added"
        );
    }
}

#[test]
fn the_harness_says_its_own_output_is_not_normative_and_is_measured() {
    let block = benchmark_block(&benchmark());
    let value = |key: &str| {
        block
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or_else(|| panic!("the block states {key}"))
    };

    // The two keys whose answers separate harness output from a committed
    // profile, and the one whose answer is the same in both.
    assert_eq!(value("status"), "\"harness-output\"");
    assert_eq!(value("normative"), "false");
    assert_eq!(
        value("measurements_recorded"),
        "true",
        "the harness's numbers are readings; a file that records none says so \
         with this key, and the harness is not such a file"
    );
}

/// A `[benchmark]` block missing a key the harness emits.
const SHORT: &str = "
[benchmark]
id = \"x\"
workload = \"y\"
phase = \"1\"
status = \"measured\"
normative = true
";

/// The same block with a key the harness does not emit.
const EXTRA: &str = "
[benchmark]
id = \"x\"
workload = \"y\"
phase = \"1\"
status = \"measured\"
normative = true
measurements_recorded = true
budgets_set = false
";

#[test]
fn a_block_that_is_short_or_long_does_not_match_the_harness() {
    let emitted: Vec<&str> = benchmark_block(&benchmark())
        .iter()
        .map(|(key, _)| *key)
        .collect();

    assert_ne!(
        benchmark_keys(SHORT),
        emitted,
        "a missing key is a mismatch"
    );
    assert_ne!(benchmark_keys(EXTRA), emitted, "an extra key is one too");
}

#[test]
fn a_key_after_the_block_belongs_to_the_table_it_is_under() {
    const AFTER: &str = "
[benchmark]
id = \"x\"

[profile]
fixture = \"somewhere\"
";

    assert_eq!(
        benchmark_keys(AFTER),
        vec!["id"],
        "the next table header ends the block"
    );
}
