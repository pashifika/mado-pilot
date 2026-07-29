//! The hard predicates the harness enforces against the ones the committed
//! profiles state.
//!
//! `bench_harness::HARD_BUDGET_PREDICATES` is a copy of two strings that live
//! in four files under `docs/benchmarks/`. A copy is the right shape here —
//! parsing the profiles would buy a TOML reader and a predicate evaluator for a
//! set of two — but a copy that nothing compares is a copy that drifts. These
//! tests are the comparison, in both directions: no profile may state a hard
//! predicate the harness does not enforce, and the harness may not enforce one
//! no profile states.
//!
//! The files are read at compile time with `include_str!`, so a profile that is
//! renamed or removed fails the build rather than the assertion.

use mado_pilot_testkit::bench_harness::{GROWTH_LIMIT_BYTES, HARD_BUDGET_PREDICATES};

/// Every committed Phase 1 profile, by repository path and content.
///
/// `example-synthetic.toml` is deliberately absent: it documents the format
/// with invented numbers and is not a measured profile, so it gates nothing.
const PROFILES: [(&str, &str); 4] = [
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
];

/// Returns the predicate of every budget in `profile`, in file order.
///
/// A `predicate` key belongs to a `kind = "hard"` budget: `absolute` and
/// `relative` budgets state a `limit` and a `direction` instead. So the
/// predicate lines are exactly the hard budgets, which is what makes this a
/// complete comparison rather than a spot check.
fn predicates(profile: &str) -> Vec<&str> {
    profile
        .lines()
        .filter_map(|line| line.trim().strip_prefix("predicate = \""))
        .filter_map(|value| value.strip_suffix('"'))
        .collect()
}

#[test]
fn every_committed_profile_states_exactly_the_predicates_the_harness_enforces() {
    for (path, profile) in PROFILES {
        assert_eq!(
            predicates(profile),
            HARD_BUDGET_PREDICATES.to_vec(),
            "{path} states hard budgets the harness does not enforce, or omits \
             one it does; the harness enforces them as constants, so the two \
             lists are kept equal by this test and by nothing else"
        );
    }
}

#[test]
fn the_enforced_growth_limit_is_the_number_the_predicate_states() {
    let predicate = HARD_BUDGET_PREDICATES[1];
    let limit = predicate
        .strip_prefix("allocated_growth_bytes <= ")
        .expect("the growth predicate is an at-most comparison")
        .parse::<i64>()
        .expect("its bound is an integer");

    assert_eq!(
        limit, GROWTH_LIMIT_BYTES,
        "the constant the harness compares against must be the bound the \
         predicate string states, or the enforcement and the record disagree"
    );
}
