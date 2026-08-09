# ADR 0025: macOS native input performance budgets

- **Status:** Accepted
- **Date:** 2026-08-10
- **Resolves gate:** the `aarch64-apple-darwin` native input and public-language
  workloads of [`G-013`](../validation-gates.md#g-013); the gate remains open for
  the macOS capture and transition workloads, the Windows diagnostic timing
  profile, and all Windows native workloads
- **Supersedes:** ADR 0021 for the macOS native input profile only

## Context

ADR 0021 invalidated the former Phase 2 native profiles after their source,
toolchain, and correctness oracles drifted. Phase 2.2 changes the public input
contract and C ABI, so input receipt accounting, visual observation, C and C++
process loading, mapped bytes, allocation growth, and child-process resident
memory need one current revision-bound run before the input profile can become a
regression gate again.

The first current-tree run exposed a benchmark defect rather than a product
failure. The C and C++ examples now submit four logical events and each produces
one fixture pointer movement and one balanced key pair. The benchmark still
expected the superseded six-event receipt and two key pairs. It also reused the
same animated fixture across common-flow iterations even though those examples
intentionally require the fixture's initial visual precondition. A successful
sample changed that state, so later samples could fail before submitting input.
The C and C++ examples also print different reviewed receipt summaries: the C
example names `fault`, while the C++ wrapper names `evidence`.

A fresh signed fixture run proved the current behavior directly. Each language
example reported four submitted events, found the expected condition in a newer
frame, emitted one pointer movement and one key-down/key-up pair at the fixture,
and completed. The repaired benchmark then retained 50 correct samples for each
of its six workloads at source
`dd0f38bc9dc209292ff946f277f442fba52b5d10`, tree
`99b26e9bc5bace43b5dc03e303ecc5c3f0f89f2a`. The tracked profile is
[`phase-2-native-input-aarch64-apple-darwin.toml`](../benchmarks/phase-2-native-input-aarch64-apple-darwin.toml).

## Decision

### Keep each public-language visual precondition independent

Every C or C++ common-flow sample provisions a fresh approved animated fixture
before its timed span. Fixture startup is test infrastructure and is excluded
from the public-language operation latency; the fresh language process, ABI
negotiation, discovery, capture, mapping, four-event input submission, newer
frame observation, diagnostics drain, and owned-handle cleanup remain inside the
sample. The oracle then requires the exact receipt summary for that language,
one pointer movement, one balanced key pair, a nonzero mapped byte count, a
nonzero child-process resident high-water mark, and the language-specific
completion line.

This is not a product retry or state reset. It prevents one benchmark iteration
from mutating the next iteration's action precondition and makes every retained
sample evaluate the same public workflow from the same fixture state.

### Accept the measured macOS input profile

The Apple M1 Pro run on macOS 26.5.2 (25F84) produced these measurements:

| Workload | p95 | Mapped bytes | Peak live Rust heap | Child peak resident | Growth |
|---|---:|---:|---:|---:|---:|
| `input_request_receipt` | 545.408792 ms | 0 B | 4,635,805 B | not applicable | 0 B |
| `rust_common_flow` | 518.689042 ms | 4,628,480 B | 4,635,665 B | not applicable | 0 B |
| `c_process_load` | 233.813625 ms | 0 B | 551 B | 51,707,904 B | 0 B |
| `c_common_flow` | 1,658.193250 ms | 4,628,480 B | 27,465 B | 94,437,376 B | 0 B |
| `cpp_process_load` | 241.062167 ms | 0 B | 563 B | 51,757,056 B | 0 B |
| `cpp_common_flow` | 1,662.811333 ms | 4,628,480 B | 27,477 B | 94,568,448 B | 0 B |

All 300 retained samples satisfied their workload oracle. Every workload
reported zero allocation growth. The two common language flows each mapped one
1280×904 BGRA8 frame, exactly 4,628,480 bytes.

### Set target-specific regression ceilings

Latency ceilings use the ADR 0008 policy: three times measured p95, rounded up.
They tolerate ordinary developer-host variation without becoming product
latency promises:

| Workload | p95 ceiling |
|---|---:|
| `input_request_receipt` | 1,700 ms |
| `rust_common_flow` | 1,600 ms |
| `c_process_load` | 750 ms |
| `c_common_flow` | 5,000 ms |
| `cpp_process_load` | 750 ms |
| `cpp_common_flow` | 5,000 ms |

The profile also enforces these non-timing bounds:

- every retained sample is correct;
- allocation growth is at most 4,096 bytes;
- peak live Rust heap is at most 16 MiB;
- Rust, C, and C++ common flows map at most 4,628,480 bytes per result;
- C and C++ process-load resident high-water marks are at most 192 MiB;
- C and C++ common-flow resident high-water marks are at most 288 MiB.

The 16 MiB heap ceiling is more than three times the largest measured Rust
peak. The resident ceilings are more than three times each corresponding child
process measurement. Changing a ceiling requires a new revision-bound
measurement and architecture decision.

## Alternatives

- **Keep the shared mutable fixture and reset it after each sample.** Rejected.
  Reset would add a benchmark-only control path or an unobserved input sequence,
  either of which could hide product work and create another state transition
  that the next sample must trust.
- **Allow either C or C++ receipt string for both examples.** Rejected. A loose
  parser could accept output from the wrong public surface and would not detect
  drift in the language-specific contract examples.
- **Treat the failed run as product input failure.** Rejected. Direct fixture
  event records and standalone C/C++ runs proved one pointer movement, one key
  pair, four submitted events, a newer matching frame, and successful cleanup.
  The stale benchmark expectations were the contradicted component.
- **Copy the old latency ceilings forward.** Rejected. They name a superseded
  tree and different oracles; current measurements, not historical numbers, set
  the current regression limits.
- **Infer Windows values from this host.** Rejected. Windows route, focus,
  capture, driver, and process behavior differ, and `G-013` resolves per target.

## Consequences

The macOS native input and public-language profile is normative again. Changes
to macOS input submission, fixture event generation, C ABI examples, the C++
wrapper flow, capture-to-observation behavior, or owned-handle cleanup must rerun
this workload set and compare the matching profile.

The cost is additional untimed fixture process setup around every C/C++ sample.
That increases full benchmark duration but preserves independent visual
preconditions and leaves the measured product operation unchanged. No public API
or integrator behavior changes because of the harness repair.

ADR 0021 continues to invalidate the macOS native capture and transition
profiles. No Windows native performance value is inferred, and the Windows
diagnostic timing gap also remains. Phase 2 therefore cannot claim complete
`G-013` resolution.

## Verification

The accepted profile came from the release fixture and the full 5-warmup,
50-sample input workload set:

```sh
cargo build --locked --release -p mado-pilot-platform-macos \
  --bin mado-pilot-macos-input-fixture
cargo run --locked -p mado-pilot-capi --example c-abi-check -- \
  --label "Apple M1 Pro macOS 26.5.2 (25F84)"
cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set input \
  --fixture-executable "$PWD/target/mado-pilot-fixtures/MadoPilotInputFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --c-executable "$PWD/target/debug/c-abi-check/macos-native-input" \
  --cpp-executable "$PWD/target/debug/c-abi-check/macos-native-input-cpp" \
  --source-revision dd0f38bc9dc209292ff946f277f442fba52b5d10 \
  --source-tree 99b26e9bc5bace43b5dc03e303ecc5c3f0f89f2a \
  --toolchain "rustc 1.97.1 (8bab26f4f 2026-07-14); C/C++ Apple clang 21.0.0" \
  --gpu-driver "Apple integrated GPU; system driver stack" \
  --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
  --os-version "macOS 26.5.2 (25F84)" \
  --display-topology "one built-in 3024x1964 Retina display at scale 2" \
  --permissions-signing "Screen Recording granted; Accessibility granted; generated fixture bundle ad-hoc signed with approved identifier"
```

Before the benchmark command, the release fixture was copied into the documented
bundle, ad-hoc signed as `dev.mado-pilot.macos-input-fixture`, and passed strict
`codesign` verification plus `--report-execution-context`. The profile-contract
tests parse the tracked artifact and enforce its hard, latency, mapped-byte,
heap, and child-resident budgets.
