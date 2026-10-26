# ADR 0107: Clean-build native CI lanes

- **Status:** Accepted
- **Date:** 2026-10-25
- **Resolves gate:** none
- **Supersedes:** none

## Context

The serial Windows job in [PR110](https://github.com/pashifika/mado-pilot/actions/runs/40198160940)
took 34 minutes. Its matching step alone took 552 seconds, including about
329 seconds of normal/instrumented lint and build work. The macOS job took
24 minutes 50 seconds. These are CI observations, not product performance budgets.

Contributors require fresh compilation rather than persistent Cargo-output
caches. The existing matching workflow already copies the Cargo-reported binary
before attestation and smoke, and the collector verifies executable, metadata
and native-library hashes before and after execution.

## Decision

Run six independent native lanes per target, with fresh build outputs and reuse
only inside the producing job. Keep normal/instrumented matching and
ordinary/private-fixture C ABI artifacts separate. Do not introduce cross-job
compiled-artifact transport or a new artifact manifest when the existing
same-job build/consumer boundary suffices.

Preserve both required native status names as fail-closed matrix result gates.
All existing verification cases remain required. Initial dev-push queuing and
its cancellation policy are unchanged.

## Alternatives

- Persistent compiler-output caches: rejected by the selected freshness policy.
- One producer for all builds: preserves the serial compilation bottleneck.
- Cross-job artifact transport: adds transfer, native-loader compatibility and
  identity admission without a demonstrated need for the current consumers.
- Removing checks or moving them to nightly: changes verification coverage.

## Consequences

Each lane owns setup, compilation and execution. A rerun worker builds afresh;
previously completed checks retain their original attempt attribution, not new
build evidence. Full timing samples use complete attempts.

Hosted concurrency, repeated native setup and dependency builds can increase
total runner time. Worker timeouts are 30-minute safety limits, not proof of the
15-minute tuning target. Rust/C/C++ APIs, deployment floors, native qualification
and redistribution policy are unchanged.

## Verification

`actionlint .github/workflows/rust.yml` checks workflow syntax.
`python3 -B -m unittest discover -s .github/tests -v` exercises the actual gate
CLI for success, failure, cancellation, skip, missing and malformed results,
alongside the existing dev-push selector cases. Existing matching collector
regressions reject changed executable and native dependency bytes.

Both native matrices must execute on the PR. Review compares the retained
command inventory against the serial baseline; a static YAML check alone does
not prove native execution or timing. CI tuning records complete run/attempt
identities, critical-path duration, initial wait and aggregate runner time;
it does not change a product benchmark budget.
