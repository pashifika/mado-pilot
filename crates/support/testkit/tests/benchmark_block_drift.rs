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
    PHASE2_2_PROCESS_APPKIT_LATENCY_BUDGETS, PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS,
    PHASE2_2_PROCESS_GAME_LIKE_LATENCY_BUDGETS, PHASE2_2_PROCESS_HEAP_LIMIT_BYTES,
    PHASE2_2_TRANSITION_LATENCY_BUDGETS, PHASE2_PRODUCTION_CAPTURE_HEAP_LIMIT_BYTES,
    PHASE2_PRODUCTION_CAPTURE_LATENCY_BUDGETS, PHASE2_PRODUCTION_MAPPED_BYTES_LIMIT,
    PHASE2_PRODUCTION_TRANSITION_HEAP_LIMIT_BYTES, PHASE2_PRODUCTION_TRANSITION_LATENCY_BUDGETS,
    PHASE2_WINDOWS_PRODUCTION_1280_COPIED_BYTES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_DETACHED_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_GPU_RESOURCES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_HEAP_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_1280_LATENCY_BUDGETS,
    PHASE2_WINDOWS_PRODUCTION_1280_RESIDENT_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_1280_STAGING_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_1280_STALE_WORK_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_COPIED_BYTES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_DETACHED_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_GPU_RESOURCES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_HEAP_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_LATENCY_BUDGETS,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_RESIDENT_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STAGING_TEXTURES_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STALE_WORK_LIMIT,
    PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_HEAP_LIMIT_BYTES,
    PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_LATENCY_BUDGETS, benchmark_block,
};

/// Every committed benchmark profile, by repository path and content.
///
/// `example-synthetic.toml` is deliberately absent because it documents the
/// format with invented numbers rather than recording a measurement.
const PROFILES: [(&str, &str); 22] = [
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
    (
        "docs/benchmarks/phase-2-production-capture-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-capture-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-production-transitions-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-transitions-aarch64-apple-darwin.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc.toml"
        ),
    ),
    (
        "docs/benchmarks/phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml"
        ),
    ),
];

/// Native profiles and the latency ceilings enforced by their benchmark.
const NATIVE_LATENCY_PROFILES: [(&str, &str, &[LatencyBudget]); 10] = [
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
        &PHASE2_2_PROCESS_APPKIT_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml"
        ),
        &PHASE2_2_PROCESS_GAME_LIKE_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml"
        ),
        &PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-production-capture-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-capture-aarch64-apple-darwin.toml"
        ),
        &PHASE2_PRODUCTION_CAPTURE_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-production-transitions-aarch64-apple-darwin.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-transitions-aarch64-apple-darwin.toml"
        ),
        &PHASE2_PRODUCTION_TRANSITION_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml"
        ),
        &PHASE2_WINDOWS_PRODUCTION_1280_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc.toml"
        ),
        &PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_LATENCY_BUDGETS,
    ),
    (
        "docs/benchmarks/phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml",
        include_str!(
            "../../../../docs/benchmarks/phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml"
        ),
        &PHASE2_WINDOWS_PRODUCTION_DUAL_4K_LATENCY_BUDGETS,
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
/// The authority-timing profiles that must distinguish current measurements
/// from rejected output whose source or oracle binding cannot be reproduced.
const PHASE2_2_TUNING_PROFILES: [(&str, &str); 2] = [
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

/// Returns one quoted assignment from a named top-level table.
fn table_assignment<'a>(profile: &'a str, table: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in profile.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == table;
            continue;
        }
        if inside && let Some(value) = quoted_assignment(line, key) {
            return Some(value);
        }
    }
    None
}

/// Returns the source commit and tree carried in the profile's opening comment.
fn source_header(profile: &str) -> Option<(&str, &str)> {
    let mut lines = profile.lines();
    lines.next()?.strip_suffix(" at source")?;
    let commit = lines
        .next()?
        .strip_prefix("# ")?
        .strip_suffix(" and tree")?;
    let tree = lines.next()?.strip_prefix("# ")?.strip_suffix('.')?;
    Some((commit, tree))
}

/// Returns the source commit and tree carried in the profile's `notes`.
fn source_notes(profile: &str) -> Option<(&str, &str)> {
    let source = table_assignment(profile, "[profile]", "notes")?.strip_prefix("source commit ")?;
    let (commit, rest) = source.split_once(", tree ")?;
    let (tree, _) = rest.split_once(';')?;
    Some((commit, tree))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn fixture_executable_digest(profile: &str) -> Option<&str> {
    let notes = table_assignment(profile, "[profile]", "notes")?;
    let (_, digest) = notes.split_once("fixture executable sha256 ")?;
    Some(digest.split_once(';').map_or(digest, |(digest, _)| digest))
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
            .map(|(measure, limit)| {
                let micros = u32::try_from(limit.as_micros())
                    .expect("every frozen latency ceiling fits u32 microseconds");
                BudgetBlock {
                    workload: Some(budget.workload()),
                    measure: Some(measure),
                    kind: Some("absolute"),
                    unit: Some("milliseconds"),
                    direction: Some("at_most"),
                    limit: Some(f64::from(micros) / 1_000.0),
                }
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
fn native_profiles_state_exactly_the_latency_budgets_the_harness_enforces() {
    for (path, profile, enforced) in NATIVE_LATENCY_PROFILES {
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
fn macos_production_profiles_state_the_resource_budgets_the_harness_enforces() {
    let capture = include_str!(
        "../../../../docs/benchmarks/phase-2-production-capture-aarch64-apple-darwin.toml"
    );
    let transitions = include_str!(
        "../../../../docs/benchmarks/phase-2-production-transitions-aarch64-apple-darwin.toml"
    );
    let mapped_limit = f64::from(
        u32::try_from(PHASE2_PRODUCTION_MAPPED_BYTES_LIMIT)
            .expect("the accepted macOS mapped-byte limit fits u32"),
    );

    for (path, profile, heap_limit, mapped_workloads) in [
        (
            "capture",
            capture,
            PHASE2_PRODUCTION_CAPTURE_HEAP_LIMIT_BYTES,
            &[
                "publication_age",
                "steady_frame_acquisition",
                "latest_acquisition",
                "cpu_map_bgra8",
            ][..],
        ),
        (
            "transitions",
            transitions,
            PHASE2_PRODUCTION_TRANSITION_HEAP_LIMIT_BYTES,
            &["open_first_frame"][..],
        ),
    ] {
        let blocks = budget_blocks(profile);
        let heap = blocks
            .iter()
            .find(|budget| {
                budget.workload.is_none() && budget.measure == Some("peak_allocated_bytes")
            })
            .unwrap_or_else(|| panic!("{path} profile is missing its live-heap ceiling"));
        assert_eq!(heap.kind, Some("absolute"));
        assert_eq!(heap.unit, Some("bytes"));
        assert_eq!(heap.direction, Some("at_most"));
        assert_eq!(
            heap.limit,
            Some(f64::from(
                u32::try_from(heap_limit).expect("the accepted macOS heap limit fits u32")
            ))
        );

        let mapped_blocks: Vec<&BudgetBlock<'_>> = blocks
            .iter()
            .filter(|budget| budget.measure == Some("mapped_bytes_per_result"))
            .collect();
        assert_eq!(
            mapped_blocks.len(),
            mapped_workloads.len(),
            "{path} profile has stale or missing mapped-byte ceilings",
        );
        for workload in mapped_workloads {
            let mapped = mapped_blocks
                .iter()
                .find(|budget| budget.workload == Some(*workload))
                .unwrap_or_else(|| panic!("{path} profile is missing {workload}'s mapped ceiling"));
            assert_eq!(mapped.kind, Some("absolute"));
            assert_eq!(mapped.unit, Some("bytes"));
            assert_eq!(mapped.direction, Some("at_most"));
            assert_eq!(mapped.limit, Some(mapped_limit));
        }
    }
}

#[test]
fn windows_1280_profiles_state_the_resource_budgets_the_harness_enforces() {
    let capture = include_str!(
        "../../../../docs/benchmarks/phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml"
    );
    let transitions = include_str!(
        "../../../../docs/benchmarks/phase-2-production-transitions-1280x720-x86_64-pc-windows-msvc.toml"
    );
    let as_f64 = |value: u64| {
        f64::from(u32::try_from(value).expect("each accepted Windows resource limit fits u32"))
    };

    for (path, profile, heap_limit) in [
        (
            "capture",
            capture,
            PHASE2_WINDOWS_PRODUCTION_1280_HEAP_LIMIT_BYTES,
        ),
        (
            "transitions",
            transitions,
            PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_HEAP_LIMIT_BYTES,
        ),
    ] {
        let blocks = budget_blocks(profile);
        let global = |measure| {
            blocks
                .iter()
                .find(|budget| budget.workload.is_none() && budget.measure == Some(measure))
                .unwrap_or_else(|| panic!("{path} profile is missing {measure}"))
        };
        assert_eq!(
            global("peak_allocated_bytes").limit,
            Some(f64::from(
                u32::try_from(heap_limit).expect("the accepted Windows heap limit fits u32")
            ))
        );
        assert_eq!(
            global("peak_resident_bytes").limit,
            Some(as_f64(PHASE2_WINDOWS_PRODUCTION_1280_RESIDENT_LIMIT_BYTES))
        );
    }

    let capture_blocks = budget_blocks(capture);
    for (measure, unit, limit) in [
        (
            "copied_bytes_per_result",
            "bytes",
            as_f64(PHASE2_WINDOWS_PRODUCTION_1280_COPIED_BYTES_LIMIT),
        ),
        (
            "detached_textures_peak",
            "count",
            as_f64(PHASE2_WINDOWS_PRODUCTION_1280_DETACHED_TEXTURES_LIMIT),
        ),
        (
            "staging_textures_peak",
            "count",
            as_f64(PHASE2_WINDOWS_PRODUCTION_1280_STAGING_TEXTURES_LIMIT),
        ),
        (
            "gpu_resources_peak",
            "count",
            as_f64(PHASE2_WINDOWS_PRODUCTION_1280_GPU_RESOURCES_LIMIT),
        ),
    ] {
        let recorded = capture_blocks
            .iter()
            .find(|budget| {
                budget.workload == Some("callback_copy") && budget.measure == Some(measure)
            })
            .unwrap_or_else(|| panic!("capture profile is missing callback_copy {measure}"));
        assert_eq!(recorded.unit, Some(unit));
        assert_eq!(recorded.limit, Some(limit));
    }
    assert_eq!(
        capture_blocks
            .iter()
            .find(|budget| {
                budget.workload == Some("callback_copy")
                    && budget.measure == Some("stale_work_ratio")
            })
            .and_then(|budget| budget.limit),
        Some(PHASE2_WINDOWS_PRODUCTION_1280_STALE_WORK_LIMIT)
    );
}

#[test]
fn windows_dual_4k_profile_states_the_resource_budgets_the_harness_enforces() {
    let profile = include_str!(
        "../../../../docs/benchmarks/phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml"
    );
    let blocks = budget_blocks(profile);
    let as_f64 = |value: u64| {
        f64::from(u32::try_from(value).expect("each accepted dual-4K resource limit fits u32"))
    };
    let global = |measure| {
        blocks
            .iter()
            .find(|budget| budget.workload.is_none() && budget.measure == Some(measure))
            .unwrap_or_else(|| panic!("dual-4K profile is missing {measure}"))
    };
    assert_eq!(
        global("peak_allocated_bytes").limit,
        Some(f64::from(
            u32::try_from(PHASE2_WINDOWS_PRODUCTION_DUAL_4K_HEAP_LIMIT_BYTES)
                .expect("the accepted dual-4K heap limit fits u32")
        ))
    );
    assert_eq!(
        global("peak_resident_bytes").limit,
        Some(as_f64(
            PHASE2_WINDOWS_PRODUCTION_DUAL_4K_RESIDENT_LIMIT_BYTES
        ))
    );

    for workload in [
        "dual_display_frame_arrival",
        "dual_display_callback_copy",
        "dual_display_moving_seam",
    ] {
        for (measure, unit, limit) in [
            (
                "copied_bytes_per_result",
                "bytes",
                as_f64(PHASE2_WINDOWS_PRODUCTION_DUAL_4K_COPIED_BYTES_LIMIT),
            ),
            (
                "detached_textures_peak",
                "count",
                as_f64(PHASE2_WINDOWS_PRODUCTION_DUAL_4K_DETACHED_TEXTURES_LIMIT),
            ),
            (
                "staging_textures_peak",
                "count",
                as_f64(PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STAGING_TEXTURES_LIMIT),
            ),
            (
                "gpu_resources_peak",
                "count",
                as_f64(PHASE2_WINDOWS_PRODUCTION_DUAL_4K_GPU_RESOURCES_LIMIT),
            ),
        ] {
            let recorded = blocks
                .iter()
                .find(|budget| budget.workload == Some(workload) && budget.measure == Some(measure))
                .unwrap_or_else(|| panic!("{workload} is missing {measure}"));
            assert_eq!(recorded.unit, Some(unit));
            assert_eq!(recorded.limit, Some(limit));
        }
        assert_eq!(
            blocks
                .iter()
                .find(|budget| {
                    budget.workload == Some(workload) && budget.measure == Some("stale_work_ratio")
                })
                .and_then(|budget| budget.limit),
            Some(PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STALE_WORK_LIMIT)
        );
    }
}

#[test]
fn tuning_profiles_record_the_qualified_deployment_floor() {
    for (path, profile) in PHASE2_2_TUNING_PROFILES {
        let deployment_target = table_assignment(profile, "[profile]", "deployment_target")
            .unwrap_or_else(|| panic!("{path} must state its minimum deployment target"));
        assert_eq!(
            deployment_target, "macOS 26.5.2",
            "{path} must bind the measured native artifacts to the qualified deployment floor"
        );
    }
}

#[test]
fn process_profile_provenance_is_exact_and_internally_bound() {
    for (path, profile) in PHASE2_2_PROCESS_PROFILES {
        let header = source_header(profile)
            .unwrap_or_else(|| panic!("{path} must name its exact source commit and tree"));
        let notes = source_notes(profile).unwrap_or_else(|| {
            panic!("{path} must repeat its exact source commit and tree in `[profile].notes`")
        });
        assert_eq!(
            header, notes,
            "{path} binds its measurements to two different source revisions"
        );
        assert!(
            is_lower_hex(header.0, 40) && is_lower_hex(header.1, 40),
            "{path} must bind its measurements to full lowercase Git commit and tree object ids"
        );

        let fixture = table_assignment(profile, "[profile]", "fixture_sha256")
            .unwrap_or_else(|| panic!("{path} must bind its tracked fixture source"));
        assert!(
            is_lower_hex(fixture, 64),
            "{path} must carry a full lowercase SHA-256 fixture-source digest"
        );

        let fixture_executable = fixture_executable_digest(profile)
            .unwrap_or_else(|| panic!("{path} must bind the measured fixture executable"));
        assert!(
            is_lower_hex(fixture_executable, 64),
            "{path} must carry a full lowercase SHA-256 fixture-executable digest"
        );

        if let Some(benchmark_executable) =
            table_assignment(profile, "[profile]", "benchmark_executable_sha256")
        {
            assert!(
                is_lower_hex(benchmark_executable, 64),
                "{path} carries a malformed benchmark-executable SHA-256 digest"
            );
        }
    }
}
#[test]
fn tuning_profile_provenance_distinguishes_misbound_output_from_current_measurements() {
    for (path, profile) in PHASE2_2_TUNING_PROFILES {
        let status = table_assignment(profile, "[benchmark]", "status")
            .unwrap_or_else(|| panic!("{path} must state its evidence status"));
        let benchmark_executable =
            table_assignment(profile, "[profile]", "benchmark_executable_sha256");

        match status {
            "measured" => {
                let digest = benchmark_executable.unwrap_or_else(|| {
                    panic!("{path} must bind a current measurement to its benchmark executable")
                });
                assert!(
                    is_lower_hex(digest, 64),
                    "{path} must carry a full lowercase benchmark-executable SHA-256 digest"
                );
            }
            "misbound" => {
                assert!(
                    benchmark_executable.is_none(),
                    "{path} must not invent a benchmark-executable digest for misbound output"
                );
                assert!(
                    profile.contains("\nstatus = \"misbound\"\nnormative = false\n"),
                    "{path} must make misbound output explicitly non-normative"
                );
            }
            other => panic!(
                "{path} has unsupported authority-timing evidence status `{other}`; \
                 use `measured` or `misbound`"
            ),
        }
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
