# macOS owned-window replacement evidence

This record closes the remaining macOS acceptance item in
[`G-001`](../../validation-gates.md#g-001): a capture session that retained the
original `SCContentFilter` must never begin publishing a same-process replacement
window after the selected window is destroyed.

## Reviewed source and host

| Fact | Value |
|---|---|
| Commit | `a1faf04505c8471deb4de8c136fddcc7f76105e7` |
| Tree | `a6a3edd6e627eadc9da76785c861136d669e8b05` |
| Target | `aarch64-apple-darwin` |
| Host | Apple M1 Pro, 10 CPU cores, 32 GiB |
| Operating system | macOS 26.5.2 (`25F84`) |
| SDK / compiler | macOS SDK 26.5 / Apple Clang 21.0.0 (`clang-2100.1.1.101`) |
| Rust | 1.97.1 (`8bab26f4f`) |
| Fixture | Generated `MadoPilotInputFixture.app`, bundled and structurally valid ad-hoc, signing identifier `dev.mado-pilot.macos-input-fixture` |
| Authorization | Screen Recording granted; the probe sends no input and requires no Accessibility decision |
| Display topology | One built-in 3024×1964 Retina display |

The earlier accepted run was bound to `9057154`. This rerun follows the complete
intervening-diff review recorded in
[`../phase-2-native/macos-current-display.md`](../phase-2-native/macos-current-display.md)
and binds the oracle to the performance-harness revision.

## Oracle

The opt-in fixture creates one deterministic flat-colour window. Five seconds
after its ready callback, on AppKit's main thread, it closes that exact `NSWindow`
and creates a same-process, same-title successor with a deliberately distinct
flat colour. The probe:

1. discovers the original exactly once, opens it through the public provider,
   captures and maps its declared colour, and retains that mapping;
2. requires the fixture's bounded replacement callback to report success with
   non-zero old and new WindowServer numbers;
3. observes the original session for ten seconds, mapping every admitted newer
   frame and rejecting any successor-colour pixels;
4. accepts only an explicit `TargetLost` terminal status or bounded quiescence;
   per ADR 0014, request timeouts are not converted into inferred target loss;
5. discovers and opens the successor independently, captures its distinct colour,
   and confirms the retained original mapping is unchanged; and
6. closes both sessions and reaps the fixture process.

The successor capture is the negative control: it proves the replacement was
live and capturable while the old retained filter was under observation. Merely
seeing no old frames would not establish that.

## Command and result

The fixture binary was built from the reviewed commit, copied into the generated
bundle, ad-hoc signed with the stable identifier, and passed strict `codesign`
verification. The acceptance command was:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$ROOT/target/mado-pilot-fixtures/MadoPilotInputFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked --package mado-pilot-platform-macos --test native_input \
  owned_window_replacement_never_retargets_the_retained_filter -- \
  --ignored --exact --nocapture --test-threads=1
```

Observed result: one test passed in 15.89 seconds. The fixture reported a
successful replacement with distinct non-zero window numbers. The retained
filter returned only bounded frame-request timeouts for the full ten-second
observation, so the Adapter correctly inferred no terminal outcome. It published
no successor-colour frame. A fresh session captured the successor's distinct
colour, and the retained original mapping remained unchanged.

This result closes the owned-window destroy/replacement oracle. It does not claim
that every destroyed window must produce an explicit ScreenCaptureKit loss event,
does not qualify another macOS version or display topology, and is not a
performance measurement.

## Supporting regression checks

The same source passed these focused checks before the acceptance rerun:

- `MADO_PILOT_MACOS_ASAN=1 cargo test --locked --package mado-pilot-platform-macos --target-dir target/asan --lib -- --nocapture --test-threads=1` — 160 passed with no sanitizer finding.
- `cargo test --locked --package mado-pilot-platform-macos --lib -- --nocapture --test-threads=1` — 160 passed, including 27 live capture lifecycle scenarios.
- `cargo test --locked --package mado-pilot-platform-macos --test native_input -- --nocapture --test-threads=1` — nine passed and two explicit interactive probes remained ignored.
- `cargo test --locked --package mado-pilot-platform-macos --test fixture_signing --test linkage -- --nocapture` — four passed.
- The focused system-delivery probe — one passed in 5.29 seconds.

## Privacy

The local test output contained only the repository-owned fixture's structured
launch/signature classifications and fixture-owned process/window numbers. This
record retains neither those ephemeral numbers nor the process-qualified title.
No captured pixels, pixel hashes, input text, credentials, raw display
identifiers, process paths, or unrelated desktop metadata are retained.
