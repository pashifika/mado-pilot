# ADR 0039: Separate bounded OCR precursor and budget enforcement

- **Status:** Accepted
- **Date:** 2026-08-25
- **Resolves gate:** _none_; bounded-profile rows under `G-013` remain open
- **Supersedes:** the numeric-enforcement procedure in the first bounded-profile qualification plan; its fixtures, hard correctness/resource gates, and failed rows remain evidence

## Context

The first approved-host bounded run used source `278256459d41cf0a4cf2265b6345182784da3ee1` and executable SHA-256 `3a4bb04477b14092c9b3b34153275819684790d072aa1659e44183bafbd1f8b4`. Five fresh Apple processes ran every predeclared workload with three warmups and 20 retained samples. All 800 retained workload samples passed text, count, order, geometry, confidence, source, profile, detector dimensions/bytes/runs, recognizer runs, mapping, session topology, cancellation, and zero-growth gates.

All five process verdicts nevertheless failed. The plan classified a 3840×2160 blank frame by empty result count and applied the released 64×64 empty-result latency budget. That workload still maps 33,177,600 source bytes and runs a `1312x736` detector tensor; its worst process p50/p95/maximum was 237.369/244.698/245.890 ms. Comparing it to the 64×64 175/210/300 ms row is a work-shape category error. Process one also observed a 2.697 ms first close above the prior revision's 2 ms final regression ceiling; the other four observed 1.064–1.246 ms.

The original Phase 3 process established budgets from retained precursor evidence and then built a fresh budget-enforcing executable. Applying those final revision-bound budgets as prerequisites to a changed profile's precursor is circular and contradicts this Change's task to measure before accepting final target budgets.

## Decision

The rejected v1 run remains immutable evidence and is not retried, excluded, or relabeled as a pass. Qualification v2 separates three phases. The current executable exposes the first two and refuses final enforcement until accepted budgets are committed:

1. `smoke` uses one warmup and three retained samples and enforces target-independent correctness, arithmetic, privacy-safe observations, session topology, cancellation, cleanup, and allocation-growth gates only.
2. `precursor` uses three warmups and 20 retained samples in each of five fresh processes. It requires exact source, executable, host, fixture, model, and runtime identity and enforces every hard correctness/resource gate, but records target timing and resident memory without a numeric verdict.
3. `enforce-budgets` is added only after both target precursor records and a final ceiling ADR. It uses the same full sample policy in fresh processes and enforces the accepted target-specific numeric budgets in addition to every precursor gate.

Result cardinality does not select a latency class. Each workload has its own final budget because mapping extent, detector dimensions, candidate count, and recognizer work determine cost. `bounded_blank_4k` remains a 4K detector workload even when it returns no regions.

Before v2 precursor execution, the final budget-selection rule is fixed as follows:

- every precursor process must pass all hard correctness/resource gates; a failed workload rejects the candidate before numeric selection;
- for each workload and target, candidate p50, p95, and maximum ceilings are 1.25 times the worst corresponding precursor observation, rounded upward to the next 25 ms;
- those latency ceilings are capped by the released target's full-frame absolute p50/p95/maximum limits: 600/750/900 ms on Apple Silicon and 900/1,000/1,200 ms on Windows; if a worst precursor observation already exceeds the applicable cap, the candidate is rejected rather than given a larger budget;
- cold open and reopen-close use 1.25 times the worst observation rounded upward to 25 ms, capped at 175/100 ms on Apple Silicon and 250/225 ms on Windows; exceeding a cap rejects the candidate;
- first close uses 1.5 times the worst observation rounded upward to 1 ms, with an absolute 10 ms ceiling on both targets;
- peak resident memory uses 1.25 times the worst target-native observation rounded upward to 16 MiB, capped at 768 MiB on Apple Silicon and 320 MiB on Windows; exceeding a cap rejects the candidate;
- live Rust growth remains at most 4,096 bytes and attributable live Rust peak remains at most 20 MiB; detector tensor, output, concurrency, model/session, resize/run, mapping, cancellation, and cleanup bounds are unchanged and receive no numeric relaxation.

The final ceiling ADR may choose a smaller rounded ceiling but cannot exceed this rule, omit a process, or change a fixture/oracle. A fresh executable with those constants must then pass five new processes on each approved target.

## Alternatives

- **Keep the v1 overall rejection and abandon the profile.** Rejected because every profile-quality and bounded-resource row passed; the failures came from applying unrelated final regression categories before precursor measurement.
- **Raise only the observed 4K blank and close values.** Rejected because choosing thresholds directly from one failed run would be post-hoc tuning without a reusable procedure.
- **Treat any empty result as the released empty workload.** Rejected because result cardinality does not describe mapping or detector work.
- **Accept precursor timing as final evidence.** Rejected because the final executable must contain and enforce revision-bound budgets selected after both target precursor records.

## Consequences

The implementation adds an explicit precursor mode, refuses `--qualify` before final budgets exist, and reports its selected mode in schema version 2. The original schema-version-1 reports remain rejected evidence. Benchmark smoke in CI is unchanged. Support remains withheld until Windows and Apple precursor records pass, a final budget ADR applies the fixed rule, and fresh budget-enforcing runs pass on both targets.

This decision changes qualification procedure only. It does not change profile identity, detector pixels, expected text/geometry, resource ceilings, runtime/model bytes, defaults, or historical Phase 3 evidence.

## Verification

- The v1 Apple record retains all five failed process rows and exact source/executable/runtime/fixture identities.
- `--precursor` requires source, host, and process bindings and emits schema version 2 with `mode = "precursor"`.
- `--qualify` is refused until the final budget ADR and executable constants land; smoke and precursor cannot silently enforce historical values.
- Review compares the v1 and v2 source diff and requires it to contain only benchmark-mode/reporting procedure plus this ADR/evidence.
- Final profiles and hard-budget registries are added only after both target precursor records and the final ceiling ADR.
