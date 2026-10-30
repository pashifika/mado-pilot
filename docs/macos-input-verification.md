# macOS Input Adapter and Verification

`mado-pilot-platform-macos` implements macOS input at the adapter boundary,
wired into the runtime and public Rust facade via `mado_pilot::macos_engine` and
exposed to C and C++ through the C ABI (`madopilot.h` / `MadoPilot::C`).

**Current candidate release status:** **14 qualified, 0 rejected, and 0 unexecuted
controlled pairs.** Final measured candidate
`dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
`109f77df9ef9f40b515245ab60a6036822ee7d78`) passed the controlled AppKit,
game-like, and native input/public-language profiles, deterministic one-read
proofs, sanitizer, and ABI/C++/CMake checks (hosted CI passed on `7ce1602` with
evidence head `705c713`), plus independent disconnected `single`, exact
two-display non-mirrored `same-scale`, and three-display `mixed-scale` native
matrices. The opt-in `AppKitBackground` pointer mode has one separately
authorized consumer workflow observation at
`acc5d98ae8cfc4958970be826a28011bc12185c9`
([below](#revision-bound-consumer-observation)). These controlled results do not
claim arbitrary application/game compatibility, exact-window delivery, or
application consumption, and the [bounded regression rows](#bounded-focus-and-raw-geometry-regressions)
require separate native execution bindings.

**Shared verification preconditions across all native macOS checks below:**

- **Ordinary `cargo test` sends no live input:** All posting checks are `--ignored`
  and require explicit opt-in (`--features private-fixture` and/or
  `MADO_PILOT_MACOS_FIXTURE_EXECUTABLE`).
- **Non-prompting permissions:** Live capture requires **Screen Recording** and
  live input requires **event-post access** (`CGPreflightPostEventAccess`, listed
  under System Settings ▸ Privacy & Security ▸ Accessibility) granted to the
  terminal or runner launching `cargo test`. MadoPilot never calls
  permission-request APIs, opens System Settings, or forces window activation to
  make a check pass.
- **No implicit fallback and separate visual oracles:** `System` is never
  substituted for `ProcessDirected` unless explicitly ordered in `DeliveryPlan`
  after a zero-effect preflight refusal. Receipt completion (`InvocationOnly`)
  and newer-frame visual postconditions are verified as independent facts.

## Capability boundary

macOS publishes two input routes with distinct address scopes: `System` posts
`CGEvent` at the HID tap to the focused target, and `ProcessDirected` posts
`CGEventPostToPid` to the owning process of the retained window without focusing
it:

| Discovered target | `System` | `WindowMessage` | `ProcessDirected` |
|---|---|---|---|
| Top-level window | Pointer, keyboard, text; focus required; `InvocationOnly` evidence | Unsupported | Qualified pointer, keyboard, text; foreground-preserving; honors `RequireFocused` without activation; `OwningProcess` scope; `Unknown` compatibility; `InvocationOnly` evidence |
| Additional same-process windows | Same target contract | Unsupported | Preserve process scope; no exact-window or responder-selection guarantee |
| Display | Pointer only; no focus requirement; `InvocationOnly` evidence | Unsupported | Unsupported |

Because macOS provides no exact-window channel, requesting `WindowMessage` fails
admission with `UnsupportedCombination` before any event. Every target names
`PermissionKind::InputControl` (`CGPreflightPostEventAccess`), distinct from
Screen Recording (`PermissionKind::ScreenCapture`). Refused or undetermined
permission reports carry two independent axes inspected via dynamically loaded
public `Security.framework` APIs: bundle launch (`bundled`, `unbundled`,
`unknown`) and signature validity (`unsigned`, `invalid`, structurally valid
`ad-hoc`, structurally valid `certificate-backed`, `platform-failure`).

## Opt-in AppKit background pointer construction

Rust callers may opt into `MacosProcessPointerMode::AppKitBackground` before
engine construction (`CoreGraphics` remains the default, including for C/C++):

```rust
use mado_pilot::{MacosConfig, MacosProcessPointerMode, NativeEngineRequest};

let request = NativeEngineRequest::new().with_macos_config(
    MacosConfig::new()
        .with_process_pointer_mode(MacosProcessPointerMode::AppKitBackground),
);
```

This mode affects only `ProcessDirected` mouse moves, drags, presses, and
releases:

- **Construction and `Command` modifier flag:** `NSEvent` supplies the authorized
  window number, bounded event number, click count, and pressure; the resulting
  `CGEvent` receives the sequence-private source, activity tag, and
  window-relative location via runtime-resolved `CGEventSetWindowLocation`.
  **Every pointer event ORs `Command` into its modifier flags** alongside
  sequence-held modifiers without posting a Command key, which can alter click or
  drag behavior in the target app.
- **Refusal and cleanup:** Ordinary pointer events refuse with
  `UnsupportedCombination` (without fallback) if the target is frontmost,
  background state is unobservable, or the construction symbol is unavailable.
  Bounded cleanup releases owned buttons using the retained pointer position and
  last authorized bounds even if the window later became frontmost or hidden.
  Non-posting unit tests run via
  `cargo test --locked -p mado-pilot-platform-macos --lib appkit_pointer`.

### Revision-bound consumer observation

On macOS `27.0` arm64, MadoMata's unchanged character-guide workflow passed once
using published revision `acc5d98ae8cfc4958970be826a28011bc12185c9`, Rust
`1.98.1`, explicit `AppKitBackground`, and a 50 ms press/release hold (30.867 s,
10 logical clicks, 40 SDK events including 10 delays, complete cleanup, and
unchanged foreground and physical cursor across 2,498 passive samples, with
independent OCR and template postconditions). This single-application workflow
observation does not replace the 14-pair controlled qualification matrix or
upgrade route compatibility from `Unknown`.

## Coordinates

macOS capture publishes placement in the top-left-origin Core Graphics global
point plane (with signed coordinates above/left of the main display), matching
`CGEvent` directly:

- `ReprojectCurrent`: Reads live bounds and backing scale in Rust (1 read) and
  re-verifies at the final native commit gate (2 reads total).
- `RequireUnchanged`: Projects from the source-frame transform and verifies
  `NativeBounds::capture_equivalent_to` against the source frame's raw
  `SCStreamFrameInfoScreenRect` origin, raw backing scale, and quantized
  backing-pixel extent (preserving fractional raw bounds even on downscaled
  effective-1/raw-2 frames). On a terminal no-fallback single-event route with
  default focus policy, this executes in **one** final native inventory read.
- `UseFrameSnapshot`: Projects from the retained 64-revision per-stream geometry
  ledger (`UnsupportedCoordinate` if evicted) and revalidates window/process
  authority at the final gate while tolerating window movement (1 read on the
  terminal no-fallback path).

Fallback-eligible routes perform one early zero-effect authority read plus the
final native read; delay-only sequences perform one early read.

## Authorization, focus, and submission evidence

- **Per-event `CGPreflightPostEventAccess`:** Because macOS silently drops
  unauthorized `CGEvent` posts, the adapter re-reads `CGPreflightPostEventAccess`
  before every irreversible event on both routes. Revocation mid-sequence stops
  submission and reports the count already invoked. Legacy `AXIsProcessTrusted`
  is read beside preflight only as paired qualification evidence (100 quiet
  samples on the qualified host showed zero disagreements).
- **Focus policies:**
  - `Preserve`: Default for `ProcessDirected` (no Accessibility query); rejected
    at admission for `System` window targets.
  - `RequireFocused`: Never activates. Uses public Accessibility
    (`kAXPositionAttribute` / `kAXSizeAttribute`) to verify that the owning app
    is active and its focused window matches the freshly verified retained
    `SCWindow` 1-to-1 (`FocusRequired` if unfocused, `NotAuthorized` if
    Accessibility is withheld, `SubmissionFailed` if attributes are missing).
  - `ActivateIfRequired`: Supported on `System` only; activates the owning app
    (without `NSApplicationActivateIgnoringOtherApps`) and repeats the bounded
    Accessibility read-back (`FocusRefused` if still unfocused).
- **Submission evidence:** Both routes report `InvocationOnly` (`submitted`
  counts complete logical events whose post call returned; partial text chunks
  report `Partial` and close fallback).

## Process-directed delivery

`ProcessDirected` addresses the retained window's owning process
(`InputAddressScope::OwningProcess`). For each ordinary native unit, the
contained Objective-C shim entry executes in order:

1. Preflights `CGPreflightPostEventAccess`, original `NSRunningApplication`
   lifetime/PID, and early `RequireFocused` when selected.
2. Constructs the `CGEvent`.
3. Performs one bounded shareable-content read and verifies the retained logical
   `SCWindow` is present, equal, open, unminimized, on-screen, layer-zero, and
   finite.
4. Re-verifies `RequireFocused` against the returned bounds when selected.
5. Applies `RequireUnchanged`, `ReprojectCurrent`, or `UseFrameSnapshot` geometry
   validation.
6. Re-checks authorization, process lifetime, deadline, and cancellation.
7. Invokes `CGEventPostToPid` (releasing unposted events in `@finally` on any
   prior refusal).

Non-move pointer events without a prior sequence pointer position read the
current physical cursor and refuse with `UnsupportedCoordinate` unless it lies
inside the active target bounds.

## Keys, modifiers, and text

Hardware keys map to `kVK_*` codes (`F21`–`F24` return `Unsupported`).
`Key::Character` resolves via `UCKeyTranslate` on the active layout and refuses
characters that require modifier chords (use explicit modifiers or
`InputEvent::Text` instead). Synthesized events carry only the modifiers held by
the current sequence. `InputEvent::Text` posts `CGEventKeyboardSetUnicodeString`
on key code `0` in surrogate-safe chunks of at most `16` UTF-16 units (up to
`4,096` scalars). Scroll deltas convert positive-down/right into Core Graphics
line units inside the shim.

## Sequence-owned state and cleanup

Pressed keys and buttons are released newest-first under the sequence's
`CleanupBudget` (`256` releases, `250 ms` fresh clock context without
cancellation). Neither `System` nor `ProcessDirected` cleanup re-checks focus,
visibility, or window inventory (`ProcessDirected` revalidates only the original
process lifetime, PID, route, authorization, and cleanup budget), ensuring held
state releases even if the target window moved, minimized, or closed.

## Dedicated fixture

`mado-pilot-macos-input-fixture` (`dev.mado-pilot.macos-input-fixture`) creates a
deterministic window (`MadoPilot Input Fixture [<pid>]`) with a flat fill color,
an optional `--game-like` OpenGL renderer (`mode=game-like renderer=opengl`), an
owned-window replacement mode, and a bounded 1,024-event recorder reporting only
event kinds and UTF-16 unit counts (never characters). A private version-11
socket control protocol (authenticated by kernel peer PID, UID, canonical path,
audit token, and `SecCode` identity) drives window transitions (`transition`,
`replace`, `minimize`, `restore`, `move`, `resize`, `open-auxiliary`,
`move-to-next-display`, `move-offscreen`, `prepare-language-flow`, etc.).
`select_unique_fixture` requires an exact single match and verifies flat-fill
pixels within a 24-channel tolerance on macOS.

### Bundling, ad-hoc signing, and structurally verifying the fixture

Build and ad-hoc sign (`--sign -`) both the target and foreground fixture
bundles under `target/mado-pilot-fixtures/`:

```sh
cargo build --locked -p mado-pilot-platform-macos --features private-fixture \
  --bin mado-pilot-macos-input-fixture
APP=target/mado-pilot-fixtures/MadoPilotInputFixture.app
FOREGROUND_APP=target/mado-pilot-fixtures/MadoPilotForegroundFixture.app
mkdir -p "$APP/Contents/MacOS" "$FOREGROUND_APP/Contents/MacOS"
cp crates/platform/macos/bundle/Info.plist "$APP/Contents/Info.plist"
cp crates/platform/macos/bundle/Info.plist "$FOREGROUND_APP/Contents/Info.plist"
TARGET_BUNDLE_ID="$(xcrun plutil -extract CFBundleIdentifier raw "$APP/Contents/Info.plist")"
FOREGROUND_BUNDLE_ID="${TARGET_BUNDLE_ID}.foreground"
xcrun plutil -replace CFBundleIdentifier -string "$FOREGROUND_BUNDLE_ID" "$FOREGROUND_APP/Contents/Info.plist"
cp target/debug/mado-pilot-macos-input-fixture "$APP/Contents/MacOS/"
cp target/debug/mado-pilot-macos-input-fixture "$FOREGROUND_APP/Contents/MacOS/"
xcrun codesign --force --sign - --identifier "$TARGET_BUNDLE_ID" --timestamp=none "$APP"
xcrun codesign --force --sign - --identifier "$FOREGROUND_BUNDLE_ID" --timestamp=none "$FOREGROUND_APP"
xcrun codesign --verify --strict --verbose=2 "$APP"
xcrun codesign --verify --strict --verbose=2 "$FOREGROUND_APP"
"$APP/Contents/MacOS/mado-pilot-macos-input-fixture" --report-execution-context
```

The final command must print `launch=bundled`, `signature=ad-hoc`, and
`signing-identifier=dev.mado-pilot.macos-input-fixture`.

## Automated macOS checks

Run the non-posting deterministic suite from the repository root:

```sh
cargo check --locked -p mado-pilot-platform-macos --all-targets
cargo test --locked -p mado-pilot-input
cargo test --locked -p mado-pilot-platform-macos --lib
cargo test --locked -p mado-pilot-platform-macos --test native_input
cargo test --locked -p mado-pilot-platform-macos --test fixture_signing
cargo test --locked -p mado-pilot-platform-macos --test linkage
```

To also launch the fixture window for non-posting provider/refusal checks:

```sh
MADO_PILOT_MACOS_FIXTURE=1 cargo test --locked \
  -p mado-pilot-platform-macos --features private-fixture --test native_input
```

## Explicit owned-window replacement check

Requires Screen Recording (sends no input). Verifies that destroying the
retained fixture window and creating a same-process, same-title successor never
retargets the old `SCContentFilter` and that retained mappings stay unchanged
(accepted record at `a1faf04505c8471deb4de8c136fddcc7f76105e7` in
[`evidence/g-001/macos-owned-window-replacement.md`](evidence/g-001/macos-owned-window-replacement.md)):

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  owned_window_replacement_never_retargets_the_retained_filter -- \
  --ignored --exact --nocapture --test-threads=1
```

## Explicit system-input check

Requires Screen Recording, event-post access, and clicking the fixture window
within the 15-second prompt while ScreenCaptureKit streaming is active (sends
Escape up, Enter down/up, and `system-probe` with no click or pointer move):

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  interactive_system_delivery_targets_only_the_exact_fixture -- \
  --ignored --exact --nocapture --test-threads=1
```

## Bounded focus and raw-geometry regressions

These three ignored rows test `ProcessDirected` `RequireFocused`, `System`
`RequireUnchanged` raw-geometry refusal across `Move`/`Resize`, and
`effective-1/raw-2` frame handling on a `mixed-scale` topology (`CONTENT_WAIT = 5s`;
separate native execution authority required):

```sh
cargo test --locked -p mado-pilot-platform-macos --features private-fixture --test native_input \
  process_directed_require_focused_accepts_only_the_focused_fixture -- \
  --ignored --exact --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --features private-fixture --test native_input \
  system_require_unchanged_accepts_raw_geometry_and_refuses_real_changes -- \
  --ignored --exact --nocapture --test-threads=1
MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY=mixed-scale \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture --test native_input \
  system_require_unchanged_accepts_effective_one_raw_two_frame -- \
  --ignored --exact --nocapture --test-threads=1
```

## Explicit process-directed checks

Seven ignored tests qualify `ProcessDirected` while `MadoPilotForegroundFixture`
remains frontmost (`MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY` selects `single`
1×2x display, `same-scale` 2×2x displays, or `mixed-scale` 2x/1x signed-origin
seam, and `kCGEventSourceUserData` correlates row tokens):

```sh
export MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE="$FOREGROUND_APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY="<single|same-scale|mixed-scale>"
for test in \
  process_directed_delivery_qualifies_appkit_renderer \
  process_directed_delivery_qualifies_game_like_renderer \
  controlled_unrelated_activity_remains_outside_appkit_process_evidence \
  controlled_unrelated_activity_remains_outside_game_like_process_evidence \
  sustained_capture_soak_keeps_process_route_isolated \
  process_directed_pointer_refuses_offscreen_and_closed_targets \
  process_directed_delivery_uses_process_authority_and_revalidates_window_state \
  owned_fixture_control_is_versioned_idempotent_and_identity_bound
do
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
    --test native_input "$test" -- \
    --ignored --exact --nocapture --test-threads=1
done
```

Revision lineage for the qualification record in
[`evidence/phase-2-native/macos-owning-process-qualification.md`](evidence/phase-2-native/macos-owning-process-qualification.md):

- Historical superseded runs: `8309a05c3e7696f3081c5afef6dd6979ea1bb084` (tree
  `27fe879e0c4bb55fe4850d9a50737b568936cc10`, protocol v9), pre-optimization
  `a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426` (tree `f4a707501748303adcec577df5f18fcd18f13f45`,
  shim v14 / protocol v10), invalidated `28ceb2e` (fractional Retina point
  mismatch), `a471c2d51428a25dd11e42572b73cf5e86ef7478` (tree
  `3f5ada8d116b527a8644be4d804f91341bc1e296`, benchmark bodies oracle-misbound),
  and `9e3e77d4021b792f4c4835390658aaac98e76826` (tree
  `ea7881c4416ca2a330fa3097d4fa271f9a547f96`).
- Current accepted candidate: `df1c45d` and benchmark-harness-only successor
  `dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
  `109f77df9ef9f40b515245ab60a6036822ee7d78`, shim v19 / protocol v11), with
  passing `single`, `same-scale`, and `mixed-scale` matrices qualifying all 14
  controlled pairs.

## Explicit facade check

Exercises `mado_pilot::macos_engine` (`Debug` diagnostics, permission preflights,
discovery, capture, mapping, `ProcessDirected` with `FocusPolicy::Preserve` and
activity tag, receipt check, strictly newer changed-fill search, and close):

```sh
cargo run --locked --package mado-pilot-platform-macos \
  --features private-fixture \
  --bin mado-pilot-macos-input-fixture -- --animate-on-input
cargo run --locked --package mado-pilot --example macos-native-input -- \
  "MadoPilot Input Fixture [<pid>]"
```

## Explicit C ABI boundary checks

Non-input C ABI check (`--check`, default on macOS):

```sh
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

With the fixture running in `--animate-on-input` mode, run the 4-event default
and 5-event `--extended-sequence` C consumers:

```sh
target/debug/c-abi-check/macos-native-input "MadoPilot Input Fixture [<pid>]"
target/debug/c-abi-check/macos-native-input --extended-sequence "MadoPilot Input Fixture [<pid>]"
```

## Redaction review

Production input and diagnostic records exclude event payloads, keys, text,
window titles, signing identifiers, PIDs, native window numbers, raw
authorization values, and captured pixels (see
[Bounded diagnostic observation](architecture.md#bounded-diagnostic-observation)).
The fixture prints its signing identifier and bounded per-kind / UTF-16 unit
counts only on its own test stdout and never retains typed characters.

## Current native input performance evidence

Measured at final candidate `dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
`109f77df9ef9f40b515245ab60a6036822ee7d78`):

- [`benchmarks/phase-2-native-input-aarch64-apple-darwin.toml`](benchmarks/phase-2-native-input-aarch64-apple-darwin.toml):
  6 workloads, 300 samples, max allocation growth `64 B` (`<= 4,096 B` hard gate).
- [`benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml`](benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml)
  and [`benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml`](benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml):
  50 samples after 5 warm-ups per workload; AppKit terminal p95 `56.466375 ms`
  (budget `106.34 ms`), OpenGL game-like p95 `56.699333 ms` (budget `112.18 ms`),
  zero oracle failures, and `0 B` post-warm-up allocation growth.

Reproduce either `phase-2-2` profile with:

```sh
for workload in process-directed process-directed-game-like
do
  cargo bench --locked -p mado-pilot --bench native-phase2 -- \
    --workload-set "$workload" \
    --fixture-executable "$MADO_PILOT_MACOS_FIXTURE_EXECUTABLE" \
    --source-revision dec43d7b6c91d415f2028e188e89fa289cb9c1c9 \
    --source-tree 109f77df9ef9f40b515245ab60a6036822ee7d78 \
    --toolchain "rustc 1.97.1; cargo 1.97.1; Apple clang 21.0.0; macOS SDK 26.5" \
    --gpu-driver "Apple integrated GPU; system driver stack" \
    --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
    --os-version "macOS 26.5.2 (25F84)" \
    --deployment-target "macOS 26.5.2" \
    --display-topology "three online non-mirrored displays; signed-origin display 3840x2160 1x; main display 2560x1440 logical / 5120x2880 backing 2x; built-in display 1512x982 logical / 3024x1964 backing 2x; mixed-scale validator authoritative" \
    --permissions-signing "Screen Recording granted; event-post access granted; target and foreground bundles structurally ad-hoc signed"
done
```

## Historical Phase 2 evidence

The single-display matrix for commit `a1faf04505c8471deb4de8c136fddcc7f76105e7`
is retained as historical provenance in
[`evidence/phase-2-native/macos-current-display.md`](evidence/phase-2-native/macos-current-display.md),
and the owned-window replacement record is in
[`evidence/g-001/macos-owned-window-replacement.md`](evidence/g-001/macos-owned-window-replacement.md).

## Frameworks

macOS input adds no Cargo dependency and no eager framework link:
`CGEventPostToPid`, `CGPreflightPostEventAccess`, and `CGEventSetWindowLocation`
resolve by symbol from `CoreGraphics.framework`, while `AppKit`, `HIToolbox`
(`Carbon.framework`), and `Security.framework` open from absolute system paths on
first use (`crates/platform/macos/tests/linkage.rs` verifies the eager link list
and absence of fixture symbols in production artifacts). Only the opt-in fixture
binary loads `OpenGL.framework` when `--game-like` is selected.
