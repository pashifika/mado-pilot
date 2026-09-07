//! Target-specific boundary budgets independently accepted under ADR 0068.
//!
//! Like the shared harness, this keeps reviewed numeric gates compiled rather
//! than introducing a TOML parser or a predicate language. A profile id selects
//! only these exact metrics, never arbitrary input ceilings. The embedded
//! document's byte pin makes profile drift fail closed before measurement.

use std::time::Duration;

use mado_pilot::ContentDigest;
use mado_pilot_testkit::bench_harness::{self, LatencyBudget, Plan};

use super::{BATCH, Case, Measurement, PENDING_INTERVAL, QUERY_LIFETIME, SETUP_WAIT};

pub(super) const METRIC_SCHEMA: &str = "phase-5-template-watch-boundary-v2";
const BUILD_PROFILE: &str = "template-watch-boundary; debug_assertions=false; private-fixture=false; coreml-provider=false; cuda-provider=false; qualification-unsupported-api=false";
const FIXTURE_SHA256: &str = "293050009da09f14f272919e3df33738c138a26863948309736303db9fc04492";
const SCENE_SHA256: &str = "f64b1df743b1204cf70f459866371602ffbd99b1cee383e38837c562b2889aa2";
const READABLE_FRAME_BYTES: usize = 96 * 64 * 4;

struct Budget {
    latency: LatencyBudget,
    caller_allocation_calls: u64,
    peak_allocated_bytes: usize,
    steady_allocated_bytes: usize,
    allocated_growth_bytes: i64,
    lifecycle_live_delta_bytes: Option<i64>,
}

impl Budget {
    const fn new(name: &'static str, nanos: [u64; 3], calls: u64, heap: [usize; 2]) -> Self {
        Self {
            latency: LatencyBudget::new(
                name,
                Duration::from_nanos(nanos[0]),
                Duration::from_nanos(nanos[1]),
                Duration::from_nanos(nanos[2]),
            ),
            caller_allocation_calls: calls,
            peak_allocated_bytes: heap[0],
            steady_allocated_bytes: heap[1],
            // Every accepted precursor row observed zero positive growth.
            // This remains opt-in, not an extra default hard gate.
            allocated_growth_bytes: 0,
            lifecycle_live_delta_bytes: None,
        }
    }
}

pub(super) struct BoundaryProfile {
    id: &'static str,
    target: &'static str,
    hardware: &'static str,
    os_version: &'static str,
    document: &'static str,
    document_sha256: &'static str,
    // Only an independently reviewed evidence reference can make this Some.
    // No CLI option can manufacture acceptance or supply replacement limits.
    independent_review: Option<&'static str>,
    budgets: &'static [Budget],
}

const APPLE: BoundaryProfile = BoundaryProfile {
    id: "phase-5-template-watch-boundary-aarch64-apple-darwin",
    target: "aarch64-apple-darwin",
    hardware: "Apple M1 Pro",
    os_version: "macOS 26.6.2 (25G83)",
    document: include_str!(
        "../../../../../docs/benchmarks/phase-5-template-watch-boundary-aarch64-apple-darwin.toml"
    ),
    document_sha256: "bb805cafa6f1a0c7c007b380e05251b19c59786325277cef49571321ad117b9d",
    independent_review: Some(
        "rasen/changes/pull-based-c-and-cpp-template-watch-query/evidence/apple-boundary-acceptance-001.json",
    ),
    budgets: &APPLE_BUDGETS,
};

const WINDOWS: BoundaryProfile = BoundaryProfile {
    id: "phase-5-template-watch-boundary-x86_64-pc-windows-msvc",
    target: "x86_64-pc-windows-msvc",
    hardware: "Intel Core i7-12700KF",
    os_version: "Windows 11 Pro 10.0.26200",
    document: include_str!(
        "../../../../../docs/benchmarks/phase-5-template-watch-boundary-x86_64-pc-windows-msvc.toml"
    ),
    document_sha256: "6455810bd2c2e2076aa2ababfdbc5ea5713edd8027be9deeee499af46c3b8c18",
    independent_review: Some(
        "rasen/changes/pull-based-c-and-cpp-template-watch-query/evidence/windows-boundary-acceptance-001.json",
    ),
    budgets: &WINDOWS_BUDGETS,
};

// All eighteen rows use the independently accepted target-local derivation.
// Apple retains the explicit latency/heap floor from provenance-incomplete
// supplementary observations; only source-bound precursors accept lifecycle data.
// Frozen proposals and raw observations remain in the pinned profile's provenance.
const APPLE_BUDGETS: [Budget; 18] = [
    Budget::new("pending_poll_rust", [1200, 1400, 1500], 0, [131072, 131072]),
    Budget::new("pending_poll_c", [3500, 3900, 9700], 0, [135168, 135168]),
    Budget::new(
        "first_closed_terminal_poll_rust",
        [200, 500, 4300],
        0,
        [245760, 0],
    ),
    Budget::new(
        "first_closed_terminal_poll_c",
        [800, 2200, 74200],
        1,
        [245760, 0],
    ),
    Budget::new(
        "retained_terminal_poll_rust",
        [1700, 1900, 15600],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "retained_terminal_poll_c",
        [8500, 9200, 40200],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "terminal_info_match_read_rust",
        [1300, 1500, 1700],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "terminal_info_match_read_c",
        [4500, 5000, 5200],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "caller_wait_cancelled_rust",
        [1400, 1600, 6600],
        0,
        [131072, 131072],
    ),
    Budget::new(
        "caller_wait_cancelled_c",
        [8400, 9000, 23300],
        64,
        [135168, 135168],
    ),
    Budget::new(
        "query_clone_release_rust",
        [1400, 1500, 1800],
        0,
        [131072, 131072],
    ),
    Budget::new(
        "query_clone_release_c",
        [4000, 4300, 30000],
        0,
        [135168, 135168],
    ),
    Budget::new(
        "result_clone_release_rust",
        [2500, 2600, 2800],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "result_clone_release_c",
        [8900, 9500, 16700],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "exact_frame_after_parents_rust",
        [48500, 50500, 83800],
        0,
        [65536, 65536],
    ),
    Budget::new(
        "exact_frame_after_parents_c",
        [66000, 69900, 136600],
        96,
        [65536, 65536],
    ),
    Budget {
        lifecycle_live_delta_bytes: Some(0),
        ..Budget::new(
            "create_cancel_release_rust",
            [1419400, 1587500, 31429800],
            188,
            [245760, 0],
        )
    },
    Budget {
        lifecycle_live_delta_bytes: Some(0),
        ..Budget::new(
            "create_cancel_release_c",
            [1429300, 1645600, 31570000],
            201,
            [245760, 0],
        )
    },
];

const WINDOWS_BUDGETS: [Budget; 18] = [
    Budget::new("pending_poll_rust", [2200, 2200, 9200], 0, [131072, 131072]),
    Budget::new("pending_poll_c", [3200, 3400, 30800], 0, [131072, 131072]),
    Budget::new(
        "first_closed_terminal_poll_rust",
        [400, 400, 14600],
        0,
        [245760, 0],
    ),
    Budget::new(
        "first_closed_terminal_poll_c",
        [1400, 3000, 29600],
        1,
        [245760, 0],
    ),
    Budget::new(
        "retained_terminal_poll_rust",
        [2000, 2200, 2400],
        0,
        [98304, 98304],
    ),
    Budget::new(
        "retained_terminal_poll_c",
        [9400, 9600, 33000],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "terminal_info_match_read_rust",
        [1000, 1200, 1200],
        0,
        [98304, 98304],
    ),
    Budget::new(
        "terminal_info_match_read_c",
        [5000, 5200, 12400],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "caller_wait_cancelled_rust",
        [2200, 2400, 3000],
        0,
        [131072, 131072],
    ),
    Budget::new(
        "caller_wait_cancelled_c",
        [11600, 11800, 24400],
        64,
        [131072, 131072],
    ),
    Budget::new(
        "query_clone_release_rust",
        [2800, 2800, 7400],
        0,
        [131072, 131072],
    ),
    Budget::new(
        "query_clone_release_c",
        [3800, 3800, 25600],
        0,
        [131072, 131072],
    ),
    Budget::new(
        "result_clone_release_rust",
        [2400, 2400, 99800],
        0,
        [98304, 98304],
    ),
    Budget::new(
        "result_clone_release_c",
        [10600, 10800, 114600],
        0,
        [102400, 102400],
    ),
    Budget::new(
        "exact_frame_after_parents_rust",
        [47800, 50800, 186200],
        0,
        [65536, 65536],
    ),
    Budget::new(
        "exact_frame_after_parents_c",
        [74400, 89600, 249400],
        96,
        [65536, 65536],
    ),
    Budget {
        lifecycle_live_delta_bytes: Some(0),
        ..Budget::new(
            "create_cancel_release_rust",
            [2515600, 3036200, 3970800],
            226,
            [245760, 0],
        )
    },
    Budget {
        lifecycle_live_delta_bytes: Some(0),
        ..Budget::new(
            "create_cancel_release_c",
            [2361000, 2746000, 4030400],
            239,
            [245760, 0],
        )
    },
];

fn target_profile() -> Option<&'static BoundaryProfile> {
    match bench_harness::RELEASE_TARGET {
        "aarch64-apple-darwin" => Some(&APPLE),
        "x86_64-pc-windows-msvc" => Some(&WINDOWS),
        _ => None,
    }
}

// Preserve the shared --name value / --name=value spelling while rejecting
// ambiguity, especially --enforce-budgets=false silently becoming smoke mode.
fn request(arguments: &[String]) -> (bool, Option<&str>) {
    let mut seen = [false; 6];
    let mut profile = None;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let (name, inline) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(name, value)| {
                (name, Some(value))
            });
        let index = match name {
            "--bench" => 0,
            "--enforce-budgets" => 1,
            "--budget-profile" => 2,
            "--hardware" => 3,
            "--os-version" => 4,
            "--label" => 5,
            _ => panic!("unknown template-watch-boundary argument: {name}"),
        };
        assert!(
            !seen[index],
            "duplicate template-watch-boundary argument: {name}"
        );
        seen[index] = true;
        if index < 2 {
            assert!(inline.is_none(), "{name} is a flag, not a key/value option");
        } else {
            let value = inline
                .or_else(|| arguments.next().map(String::as_str))
                .unwrap_or_else(|| panic!("{name} requires a value"));
            assert!(
                !value.trim().is_empty() && !value.starts_with("--"),
                "{name} requires a nonempty value, not another option"
            );
            if index == 2 {
                profile = Some(value);
            }
        }
    }
    assert!(
        !seen[2] || seen[1],
        "--budget-profile requires --enforce-budgets"
    );
    assert!(!seen[1] || seen[0], "--enforce-budgets requires --bench");
    (seen[1], profile)
}

pub(super) fn select(
    arguments: &[String],
    plan: Plan,
    hardware: &str,
    os_version: &str,
) -> Option<&'static BoundaryProfile> {
    let (enforce, requested) = request(arguments);
    if !enforce {
        return None;
    }
    let profile = target_profile().expect("numeric budgets unavailable for this release target");
    assert_eq!(
        requested.unwrap_or(profile.id),
        profile.id,
        "--budget-profile must name the compiled release target's exact profile id, not a file"
    );
    assert_eq!(
        profile.target,
        bench_harness::RELEASE_TARGET,
        "budget target mismatch"
    );
    assert_eq!(hardware, profile.hardware, "budget hardware mismatch");
    assert_eq!(os_version, profile.os_version, "budget OS profile mismatch");
    assert_eq!(
        super::build_profile(),
        BUILD_PROFILE,
        "budget build/features mismatch"
    );
    assert_eq!(
        (plan.warmup(), plan.samples()),
        (20, 200),
        "budget sampling mismatch"
    );
    assert_eq!(BATCH, 32, "budget observation batch mismatch");
    assert_eq!(
        SETUP_WAIT,
        Duration::from_secs(5),
        "budget setup authority mismatch"
    );
    assert_eq!(
        QUERY_LIFETIME,
        Duration::from_secs(60),
        "budget query authority mismatch"
    );
    assert_eq!(
        PENDING_INTERVAL,
        Duration::from_secs(3_600),
        "budget pending rate mismatch"
    );
    assert_eq!(
        super::fixture_digest().to_string(),
        FIXTURE_SHA256,
        "budget fixture mismatch"
    );
    assert_eq!(
        ContentDigest::of(&mado_pilot_testkit::match_fixtures::scene_pixels(
            mado_pilot::PixelFormat::Rgba8,
        ))
        .to_string(),
        SCENE_SHA256,
        "budget generated scene mismatch"
    );
    assert_eq!(
        ContentDigest::of(profile.document.as_bytes()).to_string(),
        profile.document_sha256,
        "budget document changed without reviewing its compiled limits and updating its byte pin"
    );
    assert!(
        profile
            .independent_review
            .is_some_and(|review| !review.trim().is_empty()),
        "ADR 0068 numeric budgets are proposed; independent acceptance is pending; no enforcement override"
    );
    profile.require_complete();
    Some(profile)
}

impl BoundaryProfile {
    pub(super) fn id(&self) -> &'static str {
        self.id
    }

    fn require_complete(&self) {
        for case in Case::ALL {
            for name in case.names() {
                let mut matches = self
                    .budgets
                    .iter()
                    .filter(|budget| budget.latency.workload() == name);
                let budget = matches.next().unwrap_or_else(|| {
                    panic!("{name}: numeric budget unavailable; new target precursors and independent review required")
                });
                assert!(matches.next().is_none(), "{name}: duplicate numeric budget");
                assert_eq!(
                    budget.lifecycle_live_delta_bytes.is_some(),
                    case == Case::CreateCancelRelease,
                    "{name}: lifecycle metric/budget compatibility mismatch"
                );
                assert!(
                    budget.peak_allocated_bytes > 0,
                    "{name}: peak heap budget must be present and positive"
                );
                assert!(
                    budget.latency.p50() <= budget.latency.p95()
                        && budget.latency.p95() <= budget.latency.hard_max()
                        && !budget.latency.p50().is_zero(),
                    "{name}: invalid latency budget"
                );
            }
        }
        assert_eq!(
            self.budgets.len(),
            Case::ALL.len() * 2,
            "unknown numeric budget workload"
        );
    }

    pub(super) fn enforce(&self, measurements: &[Measurement], plan: Plan) {
        self.require_complete();
        assert_eq!(
            measurements.len(),
            self.budgets.len(),
            "numeric measurement set is incomplete"
        );
        for (measurement, budget) in measurements.iter().zip(self.budgets) {
            let workload = &measurement.workload;
            let name = workload.name();
            assert_eq!(
                name,
                budget.latency.workload(),
                "numeric workload order/identity mismatch"
            );
            assert!(
                measurement.case.names().contains(&name),
                "numeric case/metric mismatch"
            );
            let observations = measurement.metrics.observations.borrow();
            assert_eq!(
                observations.len(),
                plan.samples(),
                "{name}: missing caller observations"
            );
            assert!(
                !observations.is_empty(),
                "{name}: no samples is not a zero measurement"
            );
            bench_harness::enforce_latency_budgets(
                std::slice::from_ref(workload),
                std::slice::from_ref(&budget.latency),
            );
            assert!(
                workload.peak_allocated_bytes() <= budget.peak_allocated_bytes,
                "{name}: peak_allocated_bytes {} > {}",
                workload.peak_allocated_bytes(),
                budget.peak_allocated_bytes
            );
            let steady = measurement
                .metrics
                .ending_live
                .get()
                .saturating_sub(measurement.metrics.fixture_baseline.get());
            assert!(
                steady <= budget.steady_allocated_bytes,
                "{name}: steady_allocated_bytes {steady} > {}",
                budget.steady_allocated_bytes
            );
            assert!(
                workload.peak_allocated_bytes() > 0,
                "{name}: zero peak heap is not a recorded fixture allocation measurement"
            );
            assert!(
                workload.growth_bytes() <= budget.allocated_growth_bytes,
                "{name}: allocated_growth_bytes {} > {}",
                workload.growth_bytes(),
                budget.allocated_growth_bytes
            );
            for (index, sample) in observations.iter().enumerate() {
                assert!(
                    sample.live_before > 0 && sample.live_after > 0,
                    "{name} sample {index}: missing process-live heap readings"
                );
                assert!(
                    sample.calls.allocations() <= budget.caller_allocation_calls,
                    "{name} sample {index}: caller_allocation_calls {} > {}",
                    sample.calls.allocations(),
                    budget.caller_allocation_calls
                );
                if measurement.case == Case::RetainedFrame {
                    assert_eq!(
                        sample.readable_view,
                        Some(READABLE_FRAME_BYTES),
                        "{name}: missing/wrong readable view length, not incremental mapped bytes"
                    );
                } else {
                    assert!(
                        sample.readable_view.is_none(),
                        "{name}: unexpected mapping metric"
                    );
                }
                if let Some(limit) = budget.lifecycle_live_delta_bytes {
                    let delta = i64::try_from(sample.live_after)
                        .expect("live heap fits signed observation")
                        - i64::try_from(sample.live_before)
                            .expect("live heap fits signed observation");
                    assert!(
                        delta <= limit,
                        "{name} sample {index}: lifecycle_live_delta_bytes {delta} > {limit}"
                    );
                }
            }
        }
    }
}

pub(super) fn report_selection(enforcement: Option<&BoundaryProfile>) {
    super::text("metric_schema", METRIC_SCHEMA);
    println!("budget_enforcement_requested = {}", enforcement.is_some());
    if let Some(profile) = target_profile() {
        super::text("budget_profile", profile.id);
        super::text(
            "budget_profile_sha256",
            &ContentDigest::of(profile.document.as_bytes()).to_string(),
        );
        super::text("compiled_budget_profile_sha256", profile.document_sha256);
        super::text(
            "budget_acceptance",
            if profile.independent_review.is_some() {
                "accepted"
            } else {
                "proposed"
            },
        );
        super::array(
            "budget_unavailable_workloads",
            Case::ALL
                .into_iter()
                .flat_map(Case::names)
                .filter(|name| {
                    !profile
                        .budgets
                        .iter()
                        .any(|budget| budget.latency.workload() == *name)
                })
                .map(|name| format!("{name:?}")),
        );
    } else {
        super::text(
            "budget_acceptance",
            "unavailable: unsupported release target",
        );
    }
}
