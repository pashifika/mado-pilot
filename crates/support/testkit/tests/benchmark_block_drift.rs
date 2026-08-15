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

use mado_pilot_testkit::bench_harness::{
    Benchmark, LatencyBudget, PHASE2_2_CAPTURE_LATENCY_BUDGETS,
    PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS, PHASE2_2_PROCESS_HEAP_LIMIT_BYTES,
    PHASE2_2_PROCESS_LATENCY_BUDGETS, PHASE2_2_TRANSITION_LATENCY_BUDGETS, benchmark_block,
};

/// Every committed benchmark profile, by repository path and content.
///
/// `example-synthetic.toml` is deliberately absent because it documents the
/// format with invented numbers rather than recording a measurement.
const PROFILES: [(&str, &str); 17] = [
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
        "docs/benchmarks/phase-2-input-diagnostic-overhead-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-input-diagnostic-overhead-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-input-diagnostic-overhead-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-input-diagnostic-overhead-x86_64-pc-windows-msvc.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-native-capture-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-capture-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-native-capture-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-capture-x86_64-pc-windows-msvc.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-native-transitions-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-transitions-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-native-transitions-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-transitions-x86_64-pc-windows-msvc.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-native-input-aarch64-apple-darwin.toml",
        include_str!("../../../../docs/benchmarks/phase-2-native-input-aarch64-apple-darwin.toml"),
    ),
    (
        "docs/benchmarks/phase-2-native-input-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-native-input-x86_64-pc-windows-msvc.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-2-controlled-capture-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-controlled-capture-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-2-controlled-transitions-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-controlled-transitions-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml"
        ),
    ),
];

/// The Phase 2.2 profiles and the latency ceilings enforced by their benchmark.
const PHASE2_2_PROFILES: [(&str, &str, &[LatencyBudget]); 5] = [
    (
        "docs/benchmarks/phase-2-2-controlled-capture-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-controlled-capture-aarch64-apple-darwin.toml"
        ),
        &PHASE2_2_CAPTURE_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-2-controlled-transitions-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-controlled-transitions-aarch64-apple-darwin.toml"
        ),
        &PHASE2_2_TRANSITION_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml"
        ),
        &PHASE2_2_PROCESS_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml"
        ),
        &PHASE2_2_PROCESS_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml"
        ),
        &PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS,
    ),
];

/// The process-directed profiles governed by the shared live-heap ceiling.
const PHASE2_2_PROCESS_PROFILES: [(&str, &str); 3] = [
    (
        "docs/benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml"
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

/// One profile budget reduced to the fields needed to compare recorded and
/// enforced absolute ceilings.
#[derive(Debug, PartialEq)]
struct BudgetBlock<'a> {
    workload: Option<&'a str>,
    measure: Option<&'a str>,
    kind: Option<&'a str>,
    unit: Option<&'a str>,
    direction: Option<&'a str>,
    limit: Option<f64>,
}

impl<'a> BudgetBlock<'a> {
    const fn new(workload: Option<&'a str>) -> Self {
        Self {
            workload,
            measure: None,
            kind: None,
            unit: None,
            direction: None,
            limit: None,
        }
    }
}

/// Returns every top-level or measurement-local budget in file order.
fn budget_blocks(profile: &str) -> Vec<BudgetBlock<'_>> {
    let mut blocks = Vec::new();
    let mut workload = None;
    let mut in_measurement = false;
    let mut in_budget = false;

    for line in profile.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_budget = line == "[[budget]]" || line == "[[measurement.budget]]";
            match line {
                "[[measurement]]" => {
                    workload = None;
                    in_measurement = true;
                }
                "[[measurement.budget]]" => {
                    assert!(
                        in_measurement,
                        "a measurement budget must follow its measurement"
                    );
                    blocks.push(BudgetBlock::new(workload));
                }
                "[[budget]]" => {
                    workload = None;
                    in_measurement = false;
                    blocks.push(BudgetBlock::new(None));
                }
                _ => {
                    in_measurement = false;
                }
            }
            continue;
        }

        if in_measurement && !in_budget && workload.is_none() {
            workload = quoted_assignment(line, "workload");
        }
        if !in_budget {
            continue;
        }

        let block = blocks.last_mut().expect("a budget block was started");
        block.measure = block.measure.or_else(|| quoted_assignment(line, "measure"));
        block.kind = block.kind.or_else(|| quoted_assignment(line, "kind"));
        block.unit = block.unit.or_else(|| quoted_assignment(line, "unit"));
        block.direction = block
            .direction
            .or_else(|| quoted_assignment(line, "direction"));
        block.limit = block.limit.or_else(|| number_assignment(line, "limit"));
    }

    blocks
}

fn quoted_assignment<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim()
        .strip_prefix('"')?
        .strip_suffix('"')
}

fn number_assignment(line: &str, key: &str) -> Option<f64> {
    line.strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim()
        .parse()
        .ok()
}

fn expected_latency_blocks(budgets: &[LatencyBudget]) -> Vec<BudgetBlock<'static>> {
    budgets
        .iter()
        .flat_map(|budget| {
            [
                ("latency_p50", budget.p50()),
                ("latency_p95", budget.p95()),
                ("latency_max", budget.hard_max()),
            ]
            .map(|(measure, limit)| BudgetBlock {
                workload: Some(budget.workload()),
                measure: Some(measure),
                kind: Some("absolute"),
                unit: Some("milliseconds"),
                direction: Some("at_most"),
                limit: Some(limit.as_secs_f64() * 1_000.0),
            })
        })
        .collect()
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
fn phase2_2_profiles_state_exactly_the_latency_budgets_the_harness_enforces() {
    for (path, profile, enforced) in PHASE2_2_PROFILES {
        let recorded: Vec<BudgetBlock<'_>> = budget_blocks(profile)
            .into_iter()
            .filter(|budget| {
                budget
                    .measure
                    .is_some_and(|measure| measure.starts_with("latency_"))
            })
            .collect();
        assert_eq!(
            recorded,
            expected_latency_blocks(enforced),
            "{path} must record every frozen p50, p95, and maximum latency \
             ceiling enforced by `native-phase2`, with no stale extra ceiling"
        );
    }
}

#[test]
fn process_profiles_state_the_same_live_heap_ceiling_the_harness_enforces() {
    let limit = f64::from(
        u32::try_from(PHASE2_2_PROCESS_HEAP_LIMIT_BYTES)
            .expect("the frozen process heap limit fits u32"),
    );
    for (path, profile) in PHASE2_2_PROCESS_PROFILES {
        let recorded: Vec<BudgetBlock<'_>> = budget_blocks(profile)
            .into_iter()
            .filter(|budget| {
                budget.workload.is_none() && budget.measure == Some("peak_allocated_bytes")
            })
            .collect();
        assert_eq!(
            recorded,
            vec![BudgetBlock {
                workload: None,
                measure: Some("peak_allocated_bytes"),
                kind: Some("absolute"),
                unit: Some("bytes"),
                direction: Some("at_most"),
                limit: Some(limit),
            }],
            "{path} must record the frozen process-directed live-heap ceiling \
             enforced by `native-phase2`"
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
