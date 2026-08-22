# ADR 0018: C ABI contract-test runtime wiring

- **Status:** Accepted
- **Date:** 2026-08-08
- **Amended:** 2026-08-10
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

The production `mado-pilot-capi` package reaches orchestration only through the
public `mado-pilot` facade. The current ABI 1.2 surface also needs deterministic
boundary tests for post-admission receipts, partial native effects, cleanup, and
serialization of concurrent `session_send_input` calls. Those states require a
controlled capture and input provider wired into a runtime engine.

The facade deliberately exposes no adapter-injection or `EngineWiring` surface.
Replacing a direct test import with facade imports produced Rust privacy errors
for `EngineWiring`, `CaptureProvider`, `InputProvider`, `Matcher`, and
`PackageLoader`; that is the intended public boundary, not a missing re-export.
The existing dependency checker then rejected the test-only runtime dependency.
A real native engine cannot deterministically create the required failures or
races and could send input to the developer's desktop.

## Decision

`mado-pilot-capi` may depend directly on `mado-pilot-runtime` only as a Cargo
development dependency and only from `#[cfg(test)]` code that constructs a
controlled ABI contract fixture. Its production dependency remains exclusively
`mado-pilot`, and every other direct C-ABI-to-runtime or contract edge remains
forbidden.

## Alternatives

- Re-export runtime wiring from the facade. Rejected: it would add public adapter
  injection solely to satisfy a private test and contradict the facade's role as
  the default composition root.
- Exercise only native engines. Rejected: permission, focus, partial delivery,
  cleanup refusal, and concurrency races would be nondeterministic, platform
  dependent, and potentially destructive.
- Test record conversion without calling the table. Rejected: it cannot prove
  admission, terminal-receipt, ownership, or serialization behavior through the
  released entry.
- Move the tests into runtime or testkit. Rejected: neither package can call the
  C boundary without reversing the production graph or creating a dependency
  cycle, and a new package for one fixture adds a second composition surface.

## Consequences

Integrators see no new dependency or API: Cargo excludes the edge from released
artifacts. C ABI unit tests may use runtime wiring types that the facade
intentionally keeps private, so this exception must remain named and test-only.
If the boundary tests can later consume a non-public workspace test facility
without a cycle, the development edge and this exception can be removed.

The architecture baseline and dependency checker record the exception. No
platform, packaging, licensing, or performance obligation changes; the runtime
package and all controlled providers were already workspace dependencies used by
the same test build.

## Verification

`tools/dependency-check/tests/graph_validation.rs` accepts exactly the
development edge `mado-pilot-capi -> mado-pilot-runtime`, rejects the same edge
in production, and rejects other development bypasses from the C ABI. The
repository command `cargo run --locked --package mado-pilot-dependency-check`
checks the actual manifest. `cargo clippy --workspace --all-targets` compiles the
fixture under `#[cfg(test)]`, while the ordinary library build proves production
code does not import runtime directly.
