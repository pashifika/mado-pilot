# macOS capture-start and fixture-cleanup remediation

This record verifies two internal macOS lifecycle repairs. It is not native template-watch support qualification, a performance profile, or evidence that either repaired defect caused the archived Apple `CaptureFailed` event. ADR 0057 continues to withhold native watcher support, and the historical cohorts remain bound to their original sources.

## Reproduced defects

Two deterministic regressions established independent implementation defects before the repair.

- A caller with a ten-second operation budget accepted one native start whose completion arrived after the shim's first two-second wait slice. Rust returned `CaptureFailed` instead of continuing under the caller's remaining authority.
- Fixture cleanup modeled an exact process lifetime that was already dead while `NSRunningApplication.isTerminated` remained stale. The old containment path still sent a termination message or recursively scheduled another global-queue reaper.

The initial repair campaign exposed a third fixture-only defect in the first implementation. Source `6eb10a8e19cc7f03a36705a6eb435e4af0fe74c7`, tree `8f47906d02ecc9be047e467badae9b3afa748875`, ran all twenty predeclared processes without retry or replacement. Sixteen exited zero. Processes 09, 11, and 18 failed because an immediate workspace process-registry lookup lagged the successful `NSWorkspace` launch callback and rejected its valid application identity. Process 19 returned `typed_operation_failure:DeadlineExceeded:native_run_start=engine_open`; that caller-owned deadline outcome did not recur in the next complete campaign and is recorded without a retry, recovery change, or reclassification.

The registration repair accepts the callback's retained application, PID, and launch date as the new identity. The callback-to-registry gap is an explicit unknown lifetime: it cannot authorize launch retry or termination. Registry equality establishes a live exact lifetime, after which registry loss is definitive. A deterministic scenario holds the workspace lookup in the transient unknown state during the callback, proves launch succeeds without a replacement, then establishes the exact live identity during bounded synchronous containment before either termination request.

## Selected behavior

The production start gate now has `idle`, `pending`, and `settled(status)` phases. The first caller alone submits `startCaptureWithCompletionHandler:`. Other open callers and close join the pending attempt, and every later caller reads the same normalized status. Rust treats only `ShimStatus::TimedOut` from this gate as an internal wait slice, checkpoints the original operation, and rejoins without resubmission. Success stores `started` before settlement so close or drop can submit at most one stop.

Private fixture cleanup retains the exact application object, PID, and launch time. Its fixture-specific registry lookup ignores advisory `isTerminated`; registry presence plus application, PID, and launch-time equality is the authority. Containment checks that identity before at most one graceful and one forced termination request. A transient unknown lifetime waits at most one second for exact registry equality and sends neither request unless equality establishes authority. Synchronous containment runs before delayed-cleanup state is allocated; if exact exit is still unconfirmed, a single serial queue performs observation only every 100 ms for at most twenty observations. Failure to allocate that delayed record increments saturating scheduled and exhausted diagnostics and signals waiters instead of discarding cleanup debt. Scheduled, active, completed, and exhausted counts remain private; no released ABI field or feature-disabled symbol was added.

## Repaired focused campaign

The complete accepted diagnostic campaign is bound to these identities:

| Identity | Value |
|---|---|
| Source revision | `843d0143668f9bdbe49482b9a11ebdc15289efbc` |
| Source tree | `47712d7d10f470d888123c45215241be14aa409c` |
| Benchmark executable SHA-256 | `36859823273ceab4b315a65b8c56dd64aadbcd5707beef50509c4ad493f4f1e1` |
| Fixture executable SHA-256 | `4591eb891a93e133be7f9b7f5d55007618809cc72ee99e198ec56fe92a94fdfe` |
| Fixture source-inventory SHA-256 | `94709c984d89c1252780c25ec25e9e9099bd34708815f4f7ff235e3499e0fa08` |
| Workload | `retained_result_mapping` |
| Plan per process | 3 warmups, 20 retained samples |
| Process order | 20 fresh sequential processes |
| Retry or replacement | 0 |

All 20 processes exited zero and retained all 400 measured samples with `result_correctness = 0`, `query_failures = 0`, `work_failed = 0`, and `allocated_growth_bytes = 0`. Every process reported backend `active = 0`, conserved backend calls, cleanup `active = 0`, cleanup `exhausted = 0`, and conserved cleanup counts. No process timed out, terminated by signal, emitted `CaptureFailed`, retained a fixture process after exit, or diverged from its declared resource baseline. The campaign ran in the available one-display environment; no display geometry, mode, scale value, or per-display record was retained. Mixed-scale topology remains part of separate native watcher qualification and was not needed to verify these lifecycle repairs.

The benchmark and fixture were built with:

```sh
cargo build --locked --release -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
cargo build --locked --release -p mado-pilot --bench native-template-watch \
  --features native-template-watch-qualification
```

Each fresh process invoked the committed release executable with `--retained-result-lifecycle-diagnostic`, `--workload=retained_result_mapping`, the identities above, and its unique `--process-index=01` through `20`.

## Deterministic and contract verification

The following checks passed on the repair worktree:

```sh
cargo test --locked -p mado-pilot-platform-macos fixture_ -- --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos start -- --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --all-targets
cargo test --locked -p mado-pilot-platform-macos --test linkage
cargo test --locked -p mado-pilot --all-targets
cargo test --locked -p mado-pilot-core -p mado-pilot-capture \
  -p mado-pilot-runtime -p mado-pilot-testkit
cargo clippy --locked -p mado-pilot-platform-macos --all-targets -- -D warnings
cargo fmt --all --check
```

The linkage suite verifies that `mp_shim_fixture_cleanup_counts` is absent when `private-fixture` is disabled. Deterministic start scenarios cover multi-slice and no-deadline success, caller deadline, cancellation, normalized failure, submission and completion exceptions, simultaneous callers, and close/drop races, including an accepted native completion failure that starts no producer and submits no stop. Fixture scenarios cover advisory termination stale in either direction, PID reuse, exact-probe failure, delayed exact death, observation exhaustion, overlapping releases, launch callback registration lag, and delayed-cleanup allocation failure on both launch abandonment and handle release after bounded synchronous containment.

The first hosted all-target run for the campaign source had one failing test:
the delayed-death seam injected its state change on a shared global utility
queue and timed out under full-suite load. Moving that injected mutation onto
the cleanup serial queue passed locally, but the next hosted run still timed
out; dispatch timing was still part of the oracle. The final seam changes the
test lifetime on its third cleanup observation immediately before the exact
probe. It therefore proves rescheduling and eventual exact death without an
independently scheduled mutation. The focused test and complete local macOS
all-target suite passed with this seam. No production timeout, termination
count, observation interval, or observation ceiling changed.

## Historical and privacy boundary

No frozen benchmark row, archived cohort result, artifact digest, or support decision was edited. In particular, later focused green evidence does not replace the archived failed Apple process and does not prove its cause. Diagnostic text remains bounded to static stage names, typed status names, allowlisted provenance, numeric resource counters, and aggregate measurements. It contains no captured pixels, OCR or input text, credentials, application titles, filesystem payloads, or detailed display configuration.
