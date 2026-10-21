# ADR 0089: Direct C ABI consumption from C and C++

- **Status:** Accepted
- **Date:** 2026-10-21
- **Resolves gate:** none
- **Supersedes:** official-wrapper provision in ADR 0005 and wrapper-specific projections in ADRs 0017, 0023, 0035, 0043 and 0046; their C ABI and historical evidence decisions remain intact

## Context

The C++ wrapper duplicated the C ABI in owning classes, result types, request
projections, examples and tests. The operator chooses application control over
those policies and a smaller project-maintained integration surface. Retiring
the wrapper does not require reducing the C ABI or adding another binding.

The native Phase 2 benchmark also depended on the wrapper's native executable.
Its C workload has four events, while the old C++ checks cover sixteen events
on Windows and five on macOS. Those verification cases must survive without
silently changing the existing C benchmark sequence.

## Decision

C and C++ consume `madopilot.h` and the released C ABI directly, through the
single development-tree `MadoPilot::C` target. Applications own resource
management, request storage, errors and higher-level policy. The project ships
small direct-call samples, not an official C++ abstraction layer.

Remove `madopilot.hpp`, the wrapper types, `madopilot_cpp` / `MadoPilot::Cpp`, and
wrapper-only examples/tests without aliases or a deprecation period. Preserve
all C ABI 1.5 declarations, functions, layouts and behavior, and every released
older-header compatibility obligation.

## Alternatives

- Keep or slim the wrapper: retains a second set of ownership and projection
  contracts that the operator does not want the project to maintain.
- Reduce the C ABI: removes required functionality and does not address the
  duplicate abstraction layer.
- Keep compatibility shims: prolongs the retired surface without an identified
  migration need. A standalone unreleased release-note entry records the break.

## Consequences

- C++ callers replace wrapper use with explicit table calls and owned-handle
  release. The [C ABI guide](../c-abi.md) and its C/C++ samples show this path.
- Library-semantic ownership, status, cancellation, concurrency, panic and
  result-retention cases remain in direct C ABI tests and verification consumers.
  Wrapper copy/move, projection and source-inventory tests are removed.
- The C native verifier preserves its four-event default and exposes the old
  extended cases only through `--extended-sequence`. Live execution still needs
  separately authorized owned fixtures.
- The current native benchmark drops its C++ executable option and workloads.
  Historical C++ rows, binaries, budgets and source identities remain unchanged
  and reproducible only with their recorded source; they are not direct-C++
  measurements. No replacement qualification is asserted.
- No C ABI version, platform minimum, dependency, packaging policy or open
  release/performance gate changes. Older ADRs and released notes stay historical.

## Verification

On 2026-10-21, Apple Silicon with Apple Clang 21.0.0, CMake 4.4.4 and Rust 1.97.1
built and ran both direct samples: ABI 1.5, 736-byte table/extent, cancellation
`0 -> 1`, and successful release. The same ordinary checker passed independent
C/C++ CMake consumers, a fresh C-only build with unavailable CXX, all four frozen
header/layout profiles, and controlled production C OCR. The external direct
C++ link/load consumer passed as `development-host`, `qualification: not-selected`.

[`c-abi-check`](../../crates/bindings/capi/examples/c-abi-check.rs), the existing
C ABI contract suites, and both native [CI jobs](../../.github/workflows/rust.yml)
enforce the maintained boundary. Ordinary and private-fixture artifacts use
separate target directories. The Change `retire-official-cpp-wrapper` retains
per-target commands and artifact identities; local observations are not Windows
results, live-input approval or release/performance qualification. Review checks
that no current wrapper artifact or alias is reintroduced.
