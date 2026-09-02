# ADR 0063: Retain integrated native-watch build failures

- **Status:** Accepted
- **Date:** 2026-09-01
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

ADR 0060 retained the distinct Apple cleanup/latency and Windows report-admission failures on source `030398e` without attributing either to the historical `f16591f` ScreenCaptureKit suspension. ADRs 0061 and 0062 selected private qualification-boundary repairs. Integrated successor `9cf746fe788a627e5b048616d0fa96e3fbc1f5dd`, tree `055f1b841ad1470af6fcac52f1b912e7b738ddca`, contains both repairs. Before any formal build, planning revision `9753065f0cb4dbeb9f8281935686adda0e2ef774` froze the source inventory, tools, dependency inputs, isolated build rules, target preflights, artifact authority schema, and five-process fail-fast protocol. Independent security review found no remaining Blocker or Major and authorized one builder invocation per host, but no formal process.

The Apple builder was invoked exactly once. Its first offline Cargo command completed, then the post-command controlled-`CARGO_HOME` check terminated red with `controlled_cargo_home_manifest_drift`. The pre-command rule required `config.toml` to be the only member and incorrectly reapplied that member set after Cargo had created its private `.global-cache`, `.package-cache`, `.package-cache-mutate`, and `registry/CACHEDIR.TAG` state. The run produced no build record, complete artifact pair, authority, benchmark build, bundle assembly, or formal process. Raw Cargo diagnostics remain ignored because they contain local namespace paths; tracked evidence retains only the fixed failure token, timing, counts, member names, byte counts, and hashes.

The Windows builder was also invoked exactly once. Exact source, planning, inventory, root-binding names, pinned interpreter, protocol, wrapper, and build-contract checks passed. Before creating any formal namespace or invoking Cargo, environment admission terminated red with `formal_environment_drift:LIBCLANG_PATH`. No value was read, retained, or transmitted. No target, build, raw, or evidence namespace, build record, artifact, authority, or formal process exists for that attempt.

## Decision

Keep Rust native template watching over Windows WGC and macOS ScreenCaptureKit implemented but unsupported. Both authorized build attempts are consumed terminal-red protocol outcomes. Do not retry either builder, mutate the host environment to reuse the attempt, publish an artifact authority without a complete build record, or launch process 1 under this protocol.

Classify both failures as qualification-apparatus control failures, not watcher correctness, lifecycle, latency, resource, privacy, or platform-support results. They neither promote support nor establish a product regression. A future evidence-backed iteration must freeze and independently review a successor protocol before any new build. Its controlled Cargo-home invariant must distinguish immutable configuration from bounded Cargo-generated private state, and its Windows environment boundary must reject inherited influence without treating an ignored host toolchain binding as child-process authority.

## Alternatives

- Re-run Apple after allowing the observed Cargo cache members. Rejected because the protocol consumed its sole builder invocation before that rule was changed.
- Remove `LIBCLANG_PATH` and re-run Windows. Rejected because mutating the environment after observing a terminal precondition would reuse the consumed attempt.
- Publish the Apple fixture binary as a formal artifact. Rejected because the benchmark, bundle assembly, build record, runtime closure, and reviewed authority do not exist.
- Treat a pre-process apparatus failure as evidence that watcher behavior is red. Rejected because neither benchmark process started and no product gate was observed.

## Consequences

- ADRs 0057 and 0060 continue to withhold native watcher support. ADRs 0061 and 0062 remain implemented repair decisions, not qualification evidence.
- Candidate `9cf746f` has one consumed Apple builder and one consumed Windows builder, zero retries, zero artifact authorities, and zero formal processes on both hosts.
- No current-source native watcher latency, cleanup, resource, correctness, or privacy result exists for `9cf746f`. Historical revision-bound results and accepted ADR 0053 budgets remain unchanged.
- Replay/OpenCV template watching remains supported. OCR predicates, watcher callbacks, C ABI/C++ watcher APIs, automatic input, target activation, arbitrary application compatibility, packaging, artifacts, tags, and `v0.4.0` remain outside the qualified boundary.

## Verification

- Planning evidence records both fixed failure tokens, source identities, invocation and process counts, timings, namespace disposition, and privacy handling.
- The Apple ignored namespace preserves the first Cargo command's raw output; only its hashes and bounded derived facts enter Git. Windows confirms all four formal namespaces remained absent and retains no environment value.
- Independent review classified both builders terminal red, prohibited retry and process 1, and retained the `WITHHELD` support decision.
- Strict repository and Rasen validation must pass after the support tables, architecture baseline, validation gates, native watcher guidance, task state, and final verification record are reconciled.
