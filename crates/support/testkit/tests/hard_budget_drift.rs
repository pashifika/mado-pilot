//! The hard predicates the harness enforces against the ones the committed
//! profiles state.
//!
//! `bench_harness::HARD_BUDGET_PREDICATES` is a copy of two strings that live
//! in every measured profile under `docs/benchmarks/`. A copy is the right shape
//! here — parsing the profiles would buy a TOML reader and a predicate evaluator
//! for a set of two — but a copy that nothing compares is a copy that drifts. These
//! tests are the comparison, in both directions: no profile may state a hard
//! predicate the harness does not enforce, and the harness may not enforce one
//! no profile states. What makes a predicate a hard budget's is the budget's
//! own `kind`, so that is read rather than assumed — a budget retyped while it
//! kept its `predicate` key would otherwise still be counted as the hard one it
//! no longer is.
//!
//! The files are read at compile time with `include_str!`, so a profile that is
//! renamed or removed fails the build rather than the assertion.

use mado_pilot_testkit::bench_harness::{GROWTH_LIMIT_BYTES, HARD_BUDGET_PREDICATES};

/// Every committed measured profile, by repository path and content.
///
/// `example-synthetic.toml` is deliberately absent because it records no
/// measurements and therefore gates no run.
const PROFILES: [(&str, &str); 12] = [
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
];

/// One budget block, reduced to the two keys this comparison is about.
#[derive(Debug, Default)]
struct Budget<'a> {
    /// What the budget is: `hard`, `absolute`, or `relative`.
    kind: Option<&'a str>,
    /// The rule a hard budget states; no other kind states one.
    predicate: Option<&'a str>,
}

/// Returns every budget block in `profile`, in file order.
///
/// A line reader rather than a TOML parser, for the reason the module states:
/// the whole comparison is two keys of one kind of table. A table header ends
/// the block before it, so a `predicate` written outside a budget belongs to no
/// block and is counted by nothing.
fn budgets(profile: &str) -> Vec<Budget<'_>> {
    let mut budgets: Vec<Budget<'_>> = Vec::new();
    let mut inside = false;

    for line in profile.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[[budget]]" || line == "[[measurement.budget]]";
            if inside {
                budgets.push(Budget::default());
            }
            continue;
        }
        if !inside {
            continue;
        }

        let budget = budgets.last_mut().expect("a block was started");
        if let Some(value) = quoted(line, "kind") {
            budget.kind = Some(value);
        }
        if let Some(value) = quoted(line, "predicate") {
            budget.predicate = Some(value);
        }
    }

    budgets
}

/// Returns what `line` assigns to `key`, when it assigns a quoted string.
fn quoted<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim_start()
        .strip_prefix('"')?
        .strip_suffix('"')
}

/// Returns the predicate of every `kind = "hard"` budget in `profile`.
///
/// Read from the budget's own `kind` rather than from the presence of the key,
/// because the two can be made to disagree: retyping a budget while keeping its
/// `predicate` would otherwise still look like a hard budget here, and the
/// harness would go on enforcing a rule the profile no longer states as one.
fn hard_predicates(profile: &str) -> Vec<&str> {
    budgets(profile)
        .into_iter()
        .filter(|budget| budget.kind == Some("hard"))
        .filter_map(|budget| budget.predicate)
        .collect()
}

/// Returns every budget whose `kind` and `predicate` disagree about what it is.
///
/// A `predicate` key belongs to a `kind = "hard"` budget and to no other:
/// `absolute` and `relative` budgets state a `limit` and a `direction` instead.
/// That is what makes the predicate lines exactly the hard budgets, and it is
/// checked here rather than assumed.
fn mismatched(profile: &str) -> Vec<Budget<'_>> {
    budgets(profile)
        .into_iter()
        .filter(|budget| budget.predicate.is_some() != (budget.kind == Some("hard")))
        .collect()
}

#[test]
fn every_committed_profile_states_exactly_the_predicates_the_harness_enforces() {
    for (path, profile) in PROFILES {
        assert_eq!(
            hard_predicates(profile),
            HARD_BUDGET_PREDICATES.to_vec(),
            "{path} states hard budgets the harness does not enforce, or omits \
             one it does; the harness enforces them as constants, so the two \
             lists are kept equal by this test and by nothing else"
        );
    }
}

#[test]
fn a_predicate_in_a_committed_profile_is_stated_by_a_hard_budget_and_no_other() {
    for (path, profile) in PROFILES {
        let mismatched = mismatched(profile);
        assert!(
            mismatched.is_empty(),
            "{path} states {mismatched:?}: a predicate is how a hard budget \
             states its rule, so one under another kind is enforced by nothing \
             and a hard budget without one states nothing to enforce"
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

/// The growth budget as a committed profile states it, retyped but otherwise
/// untouched.
///
/// Written here rather than by editing a profile: the four committed files are
/// the measured evidence, so the mutation a reader has to catch is the one that
/// belongs in the test.
const RETYPED: &str = "
[[budget]]
measure = \"allocated_growth_bytes\"
kind = \"absolute\"
predicate = \"allocated_growth_bytes <= 4096\"
";

/// The same budget with its rule dropped instead of its kind.
const RULELESS: &str = "
[[budget]]
measure = \"allocated_growth_bytes\"
kind = \"hard\"
requirement = \"a repeated operation gives back what it took\"
";

#[test]
fn a_predicate_under_another_kind_is_not_read_as_a_hard_one() {
    assert!(
        hard_predicates(RETYPED).is_empty(),
        "a retyped budget states no hard predicate, whatever key it kept"
    );
    assert_eq!(
        mismatched(RETYPED).len(),
        1,
        "and the disagreement between its kind and its key is reported"
    );
}

#[test]
fn a_hard_budget_that_states_no_predicate_is_reported() {
    assert!(
        hard_predicates(RULELESS).is_empty(),
        "a hard budget with no predicate contributes none"
    );
    assert_eq!(
        mismatched(RULELESS).len(),
        1,
        "and is reported rather than quietly shortening the comparison"
    );
}
