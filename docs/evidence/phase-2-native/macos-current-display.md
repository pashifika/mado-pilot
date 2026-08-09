# Phase 2 macOS current-display native matrix

This record covers the Phase 2 current-display acceptance matrix on the approved
Apple Silicon host. The shared external-display topology remains a separate
matrix and is not inferred from this run.

## Reviewed source and environment

| Fact | Value |
|---|---|
| Commit | `a1faf04505c8471deb4de8c136fddcc7f76105e7` |
| Tree | `a6a3edd6e627eadc9da76785c861136d669e8b05` |
| Target | `aarch64-apple-darwin` |
| Host | Apple M1 Pro, 10 CPU cores, 32 GiB |
| Operating system | macOS 26.5.2 (`25F84`) |
| SDK / compiler | macOS SDK 26.5 / Apple Clang 21.0.0 (`clang-2100.1.1.101`) |
| Rust / CMake / OpenCV | Rust 1.97.1 (`8bab26f4f`) / CMake 4.4.2 / OpenCV 4.14.0 |
| Display topology | One built-in 3024×1964 Retina display |
| Authorization | Screen Recording and Accessibility granted to the process running the checks |
| Fixture | Generated `MadoPilotInputFixture.app`, bundled and structurally valid ad-hoc, signing identifier `dev.mado-pilot.macos-input-fixture` |

This is the performance-harness source revision. The complete intervening diff
from the earlier `9057154` matrix was reviewed before this rerun: the production
change replaces snapshot `SCWindow.frame` input geometry with a retained-filter
liveness fence around an exact PID/window-number Core Graphics lookup. Fixture,
example-reporting, benchmark, and documentation changes make up the rest.

## Results

The current-display matrix passed:

| Surface | Result | Observable coverage |
|---|---|---|
| Adapter library under AddressSanitizer | 160/160 passed; no sanitizer finding | Live ScreenCaptureKit display/window capture, callback and native-exception containment, retained frames and mappings after close, producer progress, bounded native-object counts, concurrent mapping, cancellation/deadline/close races, target-loss status mapping, Retina placement, input admission/cleanup, and privacy redaction; peak 8 live native objects over 257 publications |
| Adapter library, ordinary build | 160/160 passed | Same contract and live scenarios without instrumentation; the producer run published 254 frames while the live native-object peak remained 8, equal to the first-frame baseline |
| Native integration | 9 passed; 2 explicit interactive tests ignored by the ordinary run | Discovery, capability and permission refusals, deterministic fixture selection/content, protocol bounds, and non-prompting behavior |
| Fixture signing and linkage | 4/4 passed | Structural ad-hoc signature classification, stable identifier, deployment metadata, and controlled framework linkage |
| Owned-window replacement | 1/1 passed in 15.89 seconds | The retained original filter never published the distinct same-process successor; a fresh session captured it; the retained original mapping stayed unchanged |
| Explicit Adapter input | 1/1 passed in 5.29 seconds | Exact process-qualified selection, captured-content confirmation, focus-required system delivery, four-event receipt, observed key down/up, explicit close |
| Public Rust facade | Complete | Native engine construction, two granted permission records, exact discovery, 1280×904 BGRA8 capture and 4,628,480-byte mapping, six delivered system-input events, close |
| C ABI 1.1 | Complete | 480-byte table negotiation, two granted permission records, exact discovery, native capture/mapping, six-event immutable receipt, 93,306,880-byte child resident high-water report, close |
| Header-only C++ wrapper | Complete | Same facade-owned native workflow through RAII ownership, with the same 480-byte ABI 1.1 table, six-event receipt, and 93,388,800-byte child resident high-water report |

The C/C++ binaries and `libmadopilot.dylib` were rebuilt from the reviewed source
through the repository's `c-abi-check` driver before the full native workflows.
That rebuild also passed the complete C/C++ consumer project, ownership suite,
ABI layout checks, and both frozen-header compatibility consumers.

The earlier `9057154` run first exposed and corrected a test-oracle defect:
launch or manual focus can enqueue an ordinary pointer event before delivery,
but the test attributed every queued event to the later sequence. That revision
ended the observation interval after focus. The `a1faf04` rerun preserves that
oracle and additionally exercises the live-window geometry lookup introduced
for deterministic resize: all ordinary, AddressSanitizer, replacement, explicit
input, Rust, C, and C++ rows above passed without changing their acceptance
conditions.

Detailed replacement evidence is retained in
[`../g-001/macos-owned-window-replacement.md`](../g-001/macos-owned-window-replacement.md).

## Commands

The recorded matrix used these repository commands, with the generated fixture
bundle built, copied, signed, and strictly verified first:

```sh
MADO_PILOT_MACOS_ASAN=1 cargo test --locked \
  -p mado-pilot-platform-macos --target-dir target/asan --lib -- \
  --nocapture --test-threads=1

cargo test --locked -p mado-pilot-platform-macos --lib -- \
  --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --test native_input -- \
  --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos \
  --test fixture_signing --test linkage -- --nocapture

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  interactive_system_delivery_targets_only_the_exact_fixture -- \
  --ignored --exact --nocapture --test-threads=1

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  owned_window_replacement_never_retargets_the_retained_filter -- \
  --ignored --exact --nocapture --test-threads=1

cargo run --locked -p mado-pilot --example macos-native-input -- \
  "MadoPilot Input Fixture [<pid>]"
cargo run --locked -p mado-pilot-capi --example c-abi-check -- \
  --label "macos-current"
target/debug/c-abi-check/macos-native-input \
  "MadoPilot Input Fixture [<pid>]"
target/debug/c-abi-check/macos-native-input-cpp \
  "MadoPilot Input Fixture [<pid>]"
```

## Applicability and exclusions

This run qualifies one current Retina display. The signed-origin scenario noted
that this topology had no window left of the main display, and the display-seam
and mixed-scale scenarios explicitly noted that only one display was attached.
Those are honest topology exclusions, not passes for the shared external-display
matrix. The previously accepted two-display cross-scale movement evidence remains
bound to ADR 0014; the final shared external-display matrix must still run on the
release candidate.

ScreenCaptureKit emitted no explicit terminal event for the destroyed owned
window. The bounded replacement oracle therefore proves non-retargeting, not a
claim that every destruction produces `TargetLost`. Request deadlines remained
request deadlines.

No captured pixels, pixel hashes, typed characters, credentials, raw display
identifiers, process paths, or unrelated desktop metadata are retained. Fixture
output was bounded and reported event kinds and UTF-16 unit counts rather than
text. This distilled record omits the fixture's ephemeral process/window numbers
and process-qualified title; target and display references are per-run derived
values only.
