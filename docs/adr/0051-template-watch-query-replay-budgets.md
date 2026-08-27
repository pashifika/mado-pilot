# ADR 0051: Accept target-specific deterministic template watcher budgets

- **Status:** Accepted
- **Date:** 2026-08-27
- **Gate:** `G-005` remains resolved by ADR 0050; this closes the numeric replay/OpenCV watcher qualification decision for the active Slice
- **Direction / Slice:** `version-one-delivery` / `phase-4-bounded-template-watch-query`
- **Depends on:** ADR 0050, protected prerequisite tree `4fa4f18cefec150895eae30af174f9ef999976eb`, and precursor source `bef47a647693d07c88ef52a55620fd55d7e50f12`

## Context

PR #56 established the bounded Rust template watcher/query contract but intentionally deferred approved-host regression budgets. Qualification source `bef47a647693d07c88ef52a55620fd55d7e50f12`, tree `485b586d48d193c0c1974af89e84b73ebce88dab`, runs ten fixed repository-owned replay/OpenCV and controlled-scheduler workloads. Each target ran five fresh sequential processes, three warmups and 20 retained samples per workload, with identical source/match/state/work/ownership oracles and no retry, exclusion, overlap, extra priming, or sample replacement.

Earlier apparatus sources remain rejected evidence. The first Apple cohort assumed finite replay would backpressure semantic appearance/disappearance frames. A corrected cohort found one terminal record missing from a diagnostic queue. The first Windows cohort then proved the deeper issue: the benchmark raced `DiagnosticStream`'s deliberate nonblocking `try_lock` emission by using diagnostics as a completion/work counter. The accepted source observes work directly through gated query progress, matcher counters, publications, and typed outcomes while leaving diagnostic privacy/loss/order to the deterministic contract suite. Thirty optimized Apple smoke processes and both complete v3 precursor cohorts passed after that correction.

The exact accepted precursor reports are:

- Apple: executable `a5f3c6fd64854873aac5a3032b95d2f6b6e5d8b47de19ae06722a6b3ff368cc5`, evidence `rasen/changes/phase-4-template-watch-query-qualification/evidence/apple-precursor-v3.md`;
- Windows: executable `428fe0ee04d88bf2f2b7a6ccaee5fccd06036f78db3cbb074e54ae0f0edfce53`, evidence `rasen/changes/phase-4-template-watch-query-qualification/evidence/windows-precursor-v3.md`;
- comparison and derivation: `rasen/changes/phase-4-template-watch-query-qualification/evidence/cross-target-precursor-v3.md`.

All 1,000 retained target/workload samples passed. The targets have identical workload order, mapped bytes, backend runs, query completions, stale discards, publications, superseded/expired/completed work, source/match/state results, and zero unexpected failure. Target-native latency and RSS differ materially, so they cannot share a numeric profile.

## Decision

Accept independent absolute regression ceilings for the exact deterministic watcher qualification profile. For each workload and target, p50, p95, and maximum latency are twice the worst precursor statistic rounded upward to 0.001 ms:

| Workload | Apple p50 ms | Apple p95 ms | Apple max ms | Windows p50 ms | Windows p95 ms | Windows max ms |
|---|---:|---:|---:|---:|---:|---:|
| `current_match` | 0.673 | 0.743 | 0.933 | 0.756 | 0.896 | 1.006 |
| `appearance_stable` | 1.920 | 2.135 | 2.270 | 2.213 | 2.509 | 2.606 |
| `disappearance_reset` | 2.530 | 2.652 | 2.994 | 2.867 | 3.387 | 3.446 |
| `roi_match` | 0.231 | 0.313 | 0.370 | 0.246 | 0.331 | 0.348 |
| `static_duration` | 1.951 | 2.710 | 2.850 | 2.118 | 2.468 | 2.911 |
| `coalesced_pair` | 0.214 | 0.266 | 0.309 | 0.312 | 0.550 | 0.715 |
| `saturation_latest_wins` | 30.526 | 60.634 | 86.239 | 61.980 | 94.648 | 123.699 |
| `two_session_fairness` | 0.729 | 30.338 | 30.722 | 30.820 | 45.971 | 57.311 |
| `cancel_in_flight` | 0.203 | 0.254 | 0.271 | 1.544 | 2.458 | 2.749 |
| `close_and_retain` | 0.360 | 0.437 | 0.467 | 0.487 | 0.992 | 1.317 |

Accept these target-wide resource ceilings, derived independently from each target's precursor maximum:

| Target | Peak resident bytes | Peak live Rust bytes | Post-warm growth bytes |
|---|---:|---:|---:|
| `aarch64-apple-darwin` | 69206016 | 245760 | 4096 |
| `x86_64-pc-windows-msvc` | 19922944 | 245760 | 4096 |

Peak resident limits are 1.25 times the worst nonzero target-native high-water rounded upward to 1 MiB. Peak live Rust limits are 1.25 times the worst target high-water rounded upward to 4 KiB. Growth remains the predeclared fixed 4 KiB hard gate.

Numeric ceilings apply only after hard semantic/resource gates. Every final sample must still satisfy exact source/match/ROI/stability, work-disposition totals, backend/query/stale/publication counts, queue bounds, typed cancellation/close, retained ownership, producer progress, mapped bytes, and zero unexpected work/query failure. A faster result cannot compensate for a false skip, silent query loss, stale commit, starvation, or incorrect count.

## Alternatives

- **One shared target ceiling.** Rejected. Windows `two_session_fairness` p50 is 15.4098 ms while Apple is 0.364291 ms, and target-native RSS differs by more than 3×. A shared limit would either reject a correct target or weaken the other target's regression signal.
- **Borrow hosted CI timing/RSS.** Rejected. Hosted jobs qualify compilation and deterministic behavior but are not approved host classes and cannot replace desktop target-native measurement.
- **Reuse Apple v2 because it already passed.** Rejected. Final enforcement must use one exact source. Apple v3 reran after the work-observation correction and supersedes only current acceptance; v2 remains historical evidence.
- **Use terminal diagnostics as the work counter.** Rejected. Nonblocking diagnostic emission intentionally permits observable loss under lock contention and is not a query-completion fence. Deterministic diagnostic suites remain authoritative.
- **Choose or relax ceilings after final enforcement.** Rejected. It would turn an observed regression into a post-hoc budget. Any future relaxation requires new precursor evidence and an ADR amendment before enforcement.

## Consequences

The qualification benchmark and committed target profiles enforce separate Apple and Windows latency/RSS values but one identical semantic/work contract. Integrators receive no new runtime option or behavior. The budgets are repository regression boundaries for the named fixtures and hosts, not a guarantee for arbitrary templates, ROIs, native applications, capture cadence, or real-time response.

`current_match` and `roi_match` exercise the public replay/OpenCV path. Ordered appearance/disappearance/duration rows use controlled capture with the same repository bytes and OpenCV backend so semantic frames are published only after prior progress; they do not claim replay backpressure or native application timing. Controlled matcher rows qualify scheduler mechanics and do not contribute an OpenCV backend support claim beyond the mixed fixed profile.

OCR predicates, callbacks, C/C++, native application compatibility/timing, automatic input, arbitrary templates/ROIs, packaging, a tag, and the `v0.4.0` release remain unavailable. Lowering or widening this boundary requires separate evidence and a new or amended ADR.

## Verification

- `mado-pilot`'s `template-watch-query` benchmark fails each retained sample before latency when an exact source/match/state/work/lifecycle/ownership/producer oracle fails.
- `mado-pilot-testkit::bench_harness` carries the target latency arrays and resource ceilings; `--enforce-budgets` selects only the compile target's accepted values.
- `benchmark_block_drift.rs` compares every target profile's latency blocks with the enforced arrays and compares watcher heap/RSS limits.
- `hard_budget_drift.rs` requires the unchanged correctness and 4 KiB growth predicates in both watcher profiles.
- Historical benchmark files listed by `historical-benchmark-sha256.txt` must remain byte-identical; new watcher profiles and registry rows are additive.
- Five fresh final processes per target rerun the unchanged workloads after this ADR and after new budget-enforcing executables are built. No retry, exclusion, or sample replacement is accepted.

## Independent-review successor applicability

Independent pre-merge review later found that the first final apparatus did not
prove every claimed fairness, ROI-pairing, OpenCV-work, mapped-byte, provenance,
and startup oracle. The original precursor, ceilings, profiles, and final
processes above remain revision-bound historical evidence; they are not
relabeled as acceptance for the corrected apparatus.

The independently remediated profile adds `engine_session_startup`, direct
OpenCV call/completion/mapped-request observation, exact mapped-byte enforcement,
session-qualified fairness, separate equal/unequal ROI cases, observed OpenCV
`4.14.0`, and exact release-triple dispatch. Startup p50/p95/maximum are retained
but their numeric ceiling is explicitly withheld because no pre-remediation
precursor predeclared one. Its correctness, identity, RSS, heap, growth, zero
mapping, and zero work remain enforced.

Apple passed all five remediated processes under the unchanged ADR 0051 Apple
ceilings. The first remediated Windows cohort retained one 0.7458 ms ROI maximum
failure. [ADR 0052](0052-windows-template-watch-roi-maximum-budget.md)
supersedes only the Windows ROI maximum for the remediated successor profile;
all other values in this ADR remain the active bounds. Five fresh successor
Windows processes passed that focused correction.
