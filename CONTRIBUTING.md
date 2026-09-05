# Contributing

Read [docs/architecture.md](docs/architecture.md) before changing package
boundaries, dependency directions, public naming, or platform support. It is the
tracked baseline that this repository is verified against.

## Toolchain and lockfile

`rust-toolchain.toml` pins Rust 1.97.1 with `rustfmt` and `clippy`, so a clean
checkout with `rustup` available selects the tested compiler automatically. That
pin is also the workspace minimum supported Rust version.

The workspace keeps one lockfile, the root `Cargo.lock`, and it is committed. No
member has its own lockfile. Run verification with `--locked` so that a check
fails instead of silently changing dependency resolution, and commit the lockfile
change in the same pull request whenever a manifest requirement changes.

## Native development prerequisites

Building the workspace needs an **OpenCV 4 development installation** and a
**libclang** the binding generator can load. `mado-pilot-backend-opencv` generates
its bindings at build time, so this is a prerequisite for `cargo build`, not only
for running the matching tests.

| Host | Install |
|---|---|
| macOS | `brew install opencv@4`. Xcode Command Line Tools supply libclang; no Homebrew LLVM and no `PKG_CONFIG_PATH` are needed. |
| Windows | Extract the official OpenCV prebuilt release, install LLVM, and set the discovery variables. |

[docs/third-party-dependencies.md](docs/third-party-dependencies.md) records the
exact versions, the Windows environment variables, why the discovery is restricted
rather than ambient, and what fails when the library is absent. This is a
development prerequisite only: source releases bundle no native dependency and
make no installable deployment-profile claim, which remains gate `G-007`.

The supported macOS native host is Apple Silicon macOS 26.6.2 (25G83), SDK
26.5; the deployment floor remains macOS 26.5.2, and earlier versions are
unsupported investigation targets rather than compatibility claims.
Individual revision-bound feature gates can still be unexecuted on that host.
`.cargo/config.toml` sets the final artifact deployment metadata to 26.5.2 and
the native build repeats that floor. The macOS native shim
`mado-pilot-platform-macos` compiles, links, and passes its tests with the **Xcode
Command Line Tools alone**, on a host where full Xcode is not installed; its only
Cargo addition is `cc`, declared as an unconditional build dependency so Cargo
resolves the edge on every host. The build script returns before compiling the shim
or emitting Apple framework link directives for a non-macOS target. That confirms on
the finished adapter what the `G-003` prototype had suggested on the same setup, and
the measurements are in [docs/evidence/g-003/](docs/evidence/g-003/README.md). Full
Xcode is still not evaluated, so this remains a positive result about the smaller
installation rather than a statement that the two are interchangeable. The shim is
compiled with `-fobjc-arc-exceptions`, which
[docs/adr/0012-macos-shim-language-and-containment.md](docs/adr/0012-macos-shim-language-and-containment.md)
records as a correctness requirement rather than a style choice: without it, an
exception unwinding out of a scope that holds a native object leaks it.

Running that adapter's capture scenarios needs one thing the build does not:
**Screen Recording granted to the process running the tests**. MadoPilot never
prompts, so on a host that has neither granted nor denied it the scenarios reach the
non-prompting refusal and print a skip naming that reason instead of passing. A green
`cargo test` on such a host — including a continuous-integration runner — is
therefore not evidence that macOS capture ran. To exercise them, grant Screen
Recording to the terminal or editor that launches `cargo test`, under System Settings
▸ Privacy & Security ▸ Screen & System Audio Recording, and restart it so the new
grant applies.

A runner reaches that skip rather than a crash for a reason worth knowing, because the
alternative is not a failure but an abort. The capture framework's shareable-content
query aborts, rather than returning a status, in a process holding no Core Graphics
window-server connection — and an abort is not an exception, so no handler contains
it. Both shim entry points that would reach that query return the non-prompting
authorization refusal first, so the query is unreachable without the grant, and a grant
implies the session the connection comes from. Keep that order if either entry point is
edited: the preflight is what stands between an unauthorized host and an abort.

The macOS input implementation adds a second authorization with the same
non-prompting rule: **event-post access granted to the process running the
tests**, which macOS surfaces under System Settings ▸ Privacy & Security ▸
Accessibility. macOS does not fail a synthesized event from an unauthorized
process — it discards it silently — so the Adapter reads the public
`CGPreflightPostEventAccess` decision before every irreversible event on both
input routes and reports `NotAuthorized` rather than claiming a delivery. The
legacy `AXIsProcessTrusted` observation is read beside that preflight only as
paired qualification evidence; it grants nothing and demotes nothing. Screen
Recording and event-post access are separate grants and neither implies the
other.

The ordinary workspace test run **delivers no macOS input at all**. Both real
routes — focus-dependent `System` and process-scoped `ProcessDirected` — post
real events to a real process, so the automatic checks exercise the read-only
native observations and the refusals that happen before any event, and every
posting row is opt-in. The fixture binary and its private control protocol are
built only under the explicit `private-fixture` feature and are absent from the
production library. Starting the fixture window is itself opt-in, because it
takes focus:

```sh
MADO_PILOT_MACOS_FIXTURE=1 cargo test --locked \
  -p mado-pilot-platform-macos --features private-fixture --test native_input
```

Successful macOS `System` injection is the explicit user-focused check, run on
an interactive desktop with both grants in place:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$PWD/target/mado-pilot-fixtures/MadoPilotInputFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  interactive_system_delivery_targets_only_the_exact_fixture -- \
  --ignored --exact --nocapture --test-threads=1
```

It sends no click and no pointer movement, stops before input when selection is
absent or ambiguous, and refuses rather than activating anything on its own. Do
not make it pass by requesting a permission, opening System Settings, or
activating another application to force focus.

Process-directed qualification is seven explicit tests that never focus the
target fixture. They post through the production `ProcessDirected` route while
an unrelated, independently identified owned fixture stays frontmost; assert an
unchanged physical cursor and foreground; keep sustained capture active; reject
untagged same-process observation credit; and fail closed if the retained window
or original process lifetime is lost. Additional same-process windows remain
admitted because the route promises owning-process, not exact-window, scope.
After building and signing both bundles as documented in
[`docs/macos-input-verification.md`](docs/macos-input-verification.md), export
their executable paths and the one topology being qualified:

```sh
export MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$PWD/target/mado-pilot-fixtures/MadoPilotInputFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE="$PWD/target/mado-pilot-fixtures/MadoPilotForegroundFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY="<single|same-scale|mixed-scale>"
for test in \
  process_directed_delivery_qualifies_appkit_renderer \
  process_directed_delivery_qualifies_game_like_renderer \
  controlled_unrelated_activity_remains_outside_appkit_process_evidence \
  controlled_unrelated_activity_remains_outside_game_like_process_evidence \
  sustained_capture_soak_keeps_process_route_isolated \
  process_directed_pointer_refuses_offscreen_and_closed_targets \
  process_directed_delivery_uses_process_authority_and_revalidates_window_state
do
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
    --test native_input "$test" -- \
    --ignored --exact --nocapture --test-threads=1
done
```

The capability matrix, typed outcomes, privacy bounds, and bundling step are in
[docs/macos-input-verification.md](docs/macos-input-verification.md).

Run each topology selector against the exact candidate source; a pass under one
selector cannot qualify another. Measured product candidate
`dec43d7b6c91d415f2028e188e89fa289cb9c1c9` retained the complete
three-display `mixed-scale` matrix through the benchmark-harness-only
applicability diff; test-only successor `5f1fdb6` tightened and passed the
minimized/off-screen refusal row. The required disconnected `single` and exact
two-display non-mirrored `same-scale` matrices remain unavailable, so all
fourteen release decisions remain unexecuted. The complete pre-optimization
matrix and the `a471c2d` native rows are historical provenance only; the
benchmark bodies formerly attributed to `a471c2d` are source/oracle-misbound
and supply no result.

The performance claim is narrower than the qualification matrix. The one-event
terminal `RequireUnchanged` path, with default no-focus behavior and no later
fallback, makes one final inventory read. Its pre-optimization equivalent made
four: route preflight, Rust live geometry, native preparation, and native final
authority. A fallback-eligible route and terminal `ReprojectCurrent` each have
distinct two-read shapes; `RequireFocused`, combinations of stronger policies,
cleanup, and multi-unit sequences are excluded from the one-read result.

The revision-bound one-read decision is composite. Eight exact-source
controller, geometry-source, and native seam tests prove the call count.
Exact-source AppKit and controlled OpenGL benchmark rows separately prove
latency, one matching fixture event, unchanged foreground and physical cursor,
zero correctness failures, and allocation growth no greater than 4,096 bytes
without adding private timing-path instrumentation. On measured product
candidate `dec43d7`, AppKit p95 is `56.466375 ms` under `106.34 ms`;
controlled game-like p95 is `56.699333 ms` under `112.18 ms`; both profiles
have zero allocation growth. Run the exact candidate-bound commands in
[`docs/macos-input-verification.md`](docs/macos-input-verification.md#current-native-input-performance-evidence).
Each accepted benchmark retains 50 samples after five warm-ups and records
fixture source, signed fixture executable, and benchmark executable digests.
Those gates are regression evidence for the named source, host, fixture,
renderer, route, geometry policy, and focus policy only; they are not real-time
or general application/game compatibility claims.

The owned-window replacement acceptance probe sends no input, but it opens and
replaces the signed fixture window and therefore remains explicit:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$PWD/target/mado-pilot-fixtures/MadoPilotInputFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  owned_window_replacement_never_retargets_the_retained_filter -- \
  --ignored --exact --nocapture --test-threads=1
```

The old retained filter may report explicit `TargetLost` or remain quiescent; a
request timeout is not loss evidence. The gate fails if it publishes the
distinct successor, if the successor cannot be captured independently, or if the
retained original mapping changes. The accepted qualified-host record is
[`docs/evidence/g-001/macos-owned-window-replacement.md`](docs/evidence/g-001/macos-owned-window-replacement.md).

The minimum supported Windows boundary is Windows 11 25H2 build family 26200 on
a currently serviced x64 desktop installation, accepted by
[ADR 0019](docs/adr/0019-windows-qualified-system-and-controlled-availability.md).
Windows SDK 10.0.26100.0 is the supported build input, not the runtime floor.
Earlier Windows versions are unsupported and unqualified.

The Windows capture adapter adds no prerequisite beyond that environment. The
production adapter uses the target-gated `windows` crate for Windows Graphics
Capture, Direct3D 11, and DXGI, and needs no NuGet package, Windows App SDK,
vcpkg, vendored sample, or redistributable. The preceding `G-002` prototype used
the same native stack with the MSVC toolchain, a Windows SDK, and CMake; its
review is in
[docs/evidence/g-002/dependency-review.md](docs/evidence/g-002/dependency-review.md).
[docs/adr/0013-windows-capture-frame-detachment.md](docs/adr/0013-windows-capture-frame-detachment.md)
records the ownership decision that the production implementation follows.

The Windows input implementation adds no external prerequisite or production
helper process. It uses target-gated Win32 bindings, does not elevate, and does
not attach another thread's input queue. The ordinary workspace test run starts
only `MadoPilotInputFixture` in no-activation mode, sends no system input, and
never retains input text.

The ordinary `WindowMessage` native matrix is also ignored by default because it
temporarily activates a repository-owned foreground fixture and requires an
unlocked interactive desktop. Run it deliberately with:

```sh
cargo test --locked -p mado-pilot-platform-windows --test window_message_native ordinary_window_message_native_matrix -- --ignored --exact --nocapture --test-threads=1
```

It posts only to other exact fixture windows, sends no `System` input, does not
intentionally move the physical pointer, and attempts to restore the prior
foreground and cursor on exit.

Successful system injection is deliberately excluded from the automatic suite
because Windows requires a real foreground target. On an interactive Windows host,
run the following command and click the exact PID-qualified fixture within the
15-second prompt:

```sh
cargo test --locked -p mado-pilot-platform-windows --test native_input interactive_system_delivery_targets_only_the_exact_fixture -- --ignored --exact --nocapture --test-threads=1
```

The test sends no click, restores the previous pointer position and foreground
window when Windows permits, and fails before system input if exact target selection
or focus is unavailable. Do not make it pass through `AttachThreadInput`, elevation,
or another foreground-policy bypass. The capability matrix, typed outcomes,
privacy bounds, and focused commands are in
[docs/windows-input-verification.md](docs/windows-input-verification.md).

## Phase 2 native release matrices

Hosted CI is the first gate: open the topic pull request and let both native jobs
validate compilation, contracts, ABI negotiation, and public examples before
reserving interactive hardware. A passing hosted job is not permission, display,
GPU/device, signing, input, target-loss, or minimum-system evidence.

Bind every retained run to the candidate commit and tree, or add a review that
covers the complete intervening diff. The accepted release matrices are:

- Windows single-display 1280×720 evidence on the approved Windows 11 desktop
  and exactly two non-mirrored 3840×2160 displays for mixed-DPI, signed-origin,
  movement, capture, mapping, pointer-input, device-reset/removal, target-loss,
  Rust, C, and C++ cases. Shared-display availability is an operational
  prerequisite, not a timing oracle.
- macOS exactly two online non-mirrored displays with different effective
  scales on the qualified Apple Silicon macOS 26.6.2 host, covering signed
  placement, movement, capture, mapping, pointer-input, target-loss, Rust, C,
  and C++ cases regardless of physical connection method.

Use dedicated fixtures. Evidence may retain approved host/toolchain metadata,
typed outcomes, timings, counts, and source identities; it must not retain
captured pixels, pixel hashes, input text, credentials, unrelated window titles,
process paths, or desktop metadata. Record an unavailable host or topology as an
explicit evidence gap. Never turn absence into a skip that passes the release
claim.

## Native template-watch qualification lanes

Native template-watch verification has four independent jobs. Do not infer one
job's authority from another:

1. Required Lane A owns deterministic replay/OpenCV scheduler and query
   semantics. Both hosted release-target jobs run
   `cargo bench --locked --package mado-pilot --bench template-watch-query`.
   This short plan enforces every semantic, accounting, mapped-byte, and bounded
   growth oracle, but does not apply target-specific statistical ceilings.
2. Required Windows Lane B owns the compact WGC integration contract on the
   approved interactive mixed-DPI host.
3. Required macOS Lane B owns the same compact ScreenCaptureKit integration
   contract on the approved permissioned Retina host.
4. Optional Lane C owns statistical, topology, resource, provenance, and
   endurance evidence. Invoke the replay or native executable with
   `--lane-c-evidence` and its exact source, tree, executable, fixture, process,
   host, toolchain, and backend arguments. Add `--enforce-budgets` only in this
   lane. A Lane C result cannot replace or reinterpret Lane B.

Each Lane B host builds once:

```sh
cargo bench --locked --package mado-pilot --bench native-template-watch \
  --features native-template-watch-qualification --no-run
```

Record the exact source tree and executable/fixture digests, then invoke the
emitted executable once with `--native-contract`. Exit `0` is `PASS`, `1` is
product `FAIL`, `2` is `INFRA`, and `3` is `UNSUPPORTED`; only `PASS` satisfies
the required host job. Preserve the single JSON report even when it is red.

Hosted Windows and macOS jobs compile both native modes but do not execute Lane
B: hosted macOS has no Screen Recording grant, and neither hosted runner owns
the approved interactive fixture topology. A green hosted job is therefore Lane
A plus compile/contract evidence, not WGC or ScreenCaptureKit semantic evidence.
Lane B session open starts no readiness timer by itself. Startup begins after
accepted open and ends when the acknowledged absent token is decoded; watcher
time begins at the visible-token acknowledgement and ends at the correlated
terminal outcome; teardown begins at explicit close/finalize and ends only at
the native and fixture resource baseline. No interval includes fixture launch,
and no hidden retry, sleep, deadline extension, or replacement sample is
permitted.

## Verification

Run this sequence from the repository root before opening a pull request. The
steps are ordered so that the cheapest structural failure is reported first, and
each step returns a non-zero status with an actionable diagnostic when its policy
is violated.

```sh
# 1. Workspace architecture
cargo run --locked --package mado-pilot-dependency-check

# 2. Formatting
cargo fmt --all --check

# 3. Lints, with warnings promoted to failures
cargo clippy --locked --workspace --all-targets -- -D warnings

# 4. Tests
cargo test --locked --workspace --all-targets

# 5. Documentation examples. `--all-targets` above deliberately excludes doctests,
#    so they need their own run.
cargo test --locked --workspace --doc

# 6. Documentation, with rustdoc warnings promoted to failures
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps

# 7. Dependency licenses, advisories, sources, and duplicate versions
cargo deny --locked check

# 8. C/C++ ABI 1.5, provider feature compilation, ownership, frozen
#    1.0/1.2/1.3/1.4 callers, current examples, and CMake consumers. The
#    private-fixture run additionally compiles/runs local OCR fixtures; its
#    constructor is absent from release builds and the public table.
cargo build --locked --package mado-pilot-capi
cargo check --locked --package mado-pilot-capi --all-targets --all-features
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
cargo run --locked --package mado-pilot --example ocr-fixture
cargo build --locked --package mado-pilot-capi --features private-fixture
cargo run --locked --package mado-pilot-capi --features private-fixture \
  --example c-abi-check -- --label "<host>"

# 9. macOS host only: the macOS adapter as the *other* release target sees it.
#    Step 3 above cannot reach that configuration here, because on this host the
#    macOS adapter compiles under its own target.
cargo clippy --locked --target x86_64-pc-windows-msvc \
  -p mado-pilot-platform-macos --all-targets -- -D warnings

# 10. macOS host only: the capture scenarios with the native shim instrumented.
#     Run on the qualified 26.6.2 (25G83) Apple Silicon host. Needs Screen
#     Recording granted, and fails rather than skips without it.
MADO_PILOT_MACOS_ASAN=1 cargo test --locked \
  -p mado-pilot-platform-macos --target-dir target/asan --lib -- --test-threads=1
```

The opt-in `--release-scope` check validates the immutable v0.4.0 source-release
tree, not later development changes. Run it separately from a checkout of the
source-release candidate. Its historical tree/blob identities remain frozen;
never refresh them to make a feature branch pass. See
[ADR 0065](docs/adr/0065-historical-source-release-gate-scope.md).

Both platform adapters are workspace members on both targets, so step 3 lints each of
them with the *other* platform's code compiled away — which is a configuration neither
adapter's own host ever produces. The coverage is asymmetric, and step 9 is what closes
the half a macOS host would otherwise leave to a continuous-integration job:

- a **macOS** host's step 3 lints the Windows adapter gated away, and step 9 adds the
  macOS adapter gated away;
- a **Windows** host's step 3 lints the macOS adapter gated away, so it needs no step 9.
  The mirrored command does not exist: targeting `aarch64-apple-darwin` from Windows
  would run the shim's build script, which needs a macOS toolchain.

Step 10 looks for a defect none of the steps above it can see. The macOS ownership
scenarios assert that a live native object *count* returns to its baseline, and a count
cannot observe an access after a free — which is how a confirmed use-after-free in the
native session's lifetime passed 72 green cases. `MADO_PILOT_MACOS_ASAN=1` compiles the
Objective-C shim under AddressSanitizer and links the sanitizer runtime into that
package's test binaries; with the variable unset the build is unchanged, and the runtime
is never part of a released artifact. The Xcode Command Line Tools ship it, so the step
needs nothing installed that the shim did not already need.

The same run is also the permissioned coordinate/selection acceptance gate. It
compares frame-attached `screenRect` origin, logical size, and effective scale with
the qualified host's display inventory and checks that a fresh discovery does not
terminate an already-open retained filter. A run that fails because Screen Recording
is not granted is useful denial evidence, but passes neither that live gate nor the
manual window move/resize/loss probe in
`crates/platform/macos/tests/window_movement.rs`. The separate owned-window
replacement command above is the release oracle that proves a retained filter
never captures a successor after its selected window is destroyed; its accepted
qualified-host result is retained with `G-001`.

Every part of that command is load-bearing rather than a matter of taste:

- **the single package**, because the runtime is attached with
  `cargo::rustc-link-arg`, which Cargo applies to the emitting package's own binaries
  and tests and does not propagate to a consumer. A wider build instruments the shim
  and then fails to link everything downstream of it;
- **the separate target directory**, so the instrumented archive never reaches a link
  that steps 3 through 8 read from — the mixed-cache hazard the section below
  describes, with a linker error instead of a puzzle;
- **one test thread**, because the sanitizer halts on its first violation and aborts
  the process, which is every test thread at once. One run therefore names one defect,
  and fixing the group it belongs to is an iteration rather than a single report.

Only the shim is instrumented, so the step observes any access the shim makes to freed
memory and not a freed Rust allocation that Rust dereferences. Covering both sides needs
`-Zsanitizer=address` on nightly, which the toolchain pin above rules out.

This step is deliberately not a continuous-integration check. A runner has granted no
Screen Recording, so the capture scenarios cannot run there — and because a sanitizer
run whose scenarios never captured reports nothing at all, a skip and a pass would be
the same line of output. A scenario that cannot reach a capture therefore *fails* under
this build rather than skipping, which is the same rule the replay symlink tests follow,
and which is why the step belongs to the macOS verification host alongside step 9 rather
than to a job that can never satisfy it.

One gap in that rule is deliberate and has to be read alongside it. A scenario whose
subject is a *window* moves on when no window matches it or when every match is idle,
and passes having noted that — because an idle window is a legitimate absence of subject
matter rather than a host that cannot capture, and the desktop decides which it is. So a
green step 10 means the display-based scenarios captured; it does not by itself mean the
window-based ones did. Read the run's notes when the window paths are what a change
touched.

Step 9 names one package rather than the workspace because `--workspace` at another
target fails in `opencv`'s build script, which looks for an installation for the target
it is building for. It needs no more than that package: crate-level `#[cfg]` gating of a
whole file is what produces this failure, and the two platform adapters are the only
places the workspace uses it. Elsewhere a target is selected with `cfg!()` inside an
expression, which strips no items.

The failure this reaches is never in the gated code. A crate-level `#![cfg(…)]` strips
the whole file, and when the crate's `//!` documentation sits inside what is stripped,
the crate root is left undocumented and `missing_docs` fails the lint. Every
target-gated test file in both adapters therefore opens with the guard before the gate:

```rust
#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! …
```

Keep it there. Without it a file passes only while its documentation happens to sit
above the gate — an ordering no reader can be expected to preserve, and one that has
already broken a continuous-integration job once for each adapter.

Step 4 needs one thing of a Windows host: the privilege to create a symbolic
link. Two tests in `mado-pilot-adapter-replay` prove that a linked path component
cannot reach outside a replay source, and they are the only coverage of that
rule. A host that cannot create the link has proven nothing, so they fail rather
than return early — a skipped test and a passing one are the same line of output,
and a green suite has to mean the case ran. The failure names the requirement and
carries `Os { code: 1314 }`, which is `ERROR_PRIVILEGE_NOT_HELD`.

Turn on **Settings → System → For developers → Developer Mode**. It takes effect
in a console opened afterwards, with no reboot, and `cmd` confirms it:

```bat
reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock" /v AllowDevelopmentWithoutDevLicense
```

`0x1` is on. An elevated prompt also holds the privilege, but do not verify from
one: `cargo` writes `target/` as whoever ran it, and mixing elevated and ordinary
runs in one working copy produces permission failures later that look like
anything but their cause.

Running with `-- --skip a_symlinked_` gets the rest of the suite through on a host
without the privilege. That run is not a verification: it is exactly the silent
gap the two tests were changed to expose, so record it as what it is.

Step 8 is the only check in this repository that is not `cargo` alone. It needs a
C compiler, a C++ compiler, and CMake 3.22 or later. On both release targets the
compilers are the ones the platform already has — MSVC on Windows, the Xcode
Command Line Tools on macOS — and both CI runners and both verification hosts
already have a CMake. Set `CC`, `CXX`, or `CMAKE` to choose a different one.

[docs/c-abi.md](docs/c-abi.md) records what it compiles and why the header is
verified this way rather than generated;
[docs/cpp-wrapper.md](docs/cpp-wrapper.md) records the C++ half.

On Windows, run step 8 from a Developer Command Prompt. `cl` is not on `PATH`
otherwise, and the same environment sets `VSINSTALLDIR`, through which the check
finds the CMake that Visual Studio ships when none is on `PATH`.

Step 6 sets an environment variable, which each Windows shell spells its own way.
The Developer Command Prompt the paragraph above asks for is `cmd`, so that form
comes first:

```bat
set "RUSTDOCFLAGS=-D warnings"
cargo doc --locked --workspace --no-deps
set "RUSTDOCFLAGS="
```

Quote the whole assignment. `set RUSTDOCFLAGS="-D warnings"` puts the quotation
marks *inside* the value, and rustdoc is then passed an argument it does not
recognize. The third line clears it again: `set` outlives the command, and a
`RUSTDOCFLAGS` left behind applies to every later `cargo doc` in that window.

On Windows PowerShell, step 6 is:

```powershell
$env:RUSTDOCFLAGS = "-D warnings"; cargo doc --locked --workspace --no-deps
```

Step 7 needs `cargo-deny`, which is not part of the toolchain, and needs network
access because it fetches the RustSec advisory database:

```sh
cargo install --locked cargo-deny
```

No step modifies a tracked file. `cargo fmt --all` without `--check` applies the
formatting that step 2 verifies.

[docs/third-party-dependencies.md](docs/third-party-dependencies.md) records the
dependency policy that step 7 enforces, including the review required before a
native library or model file is added.

### When a step contradicts the one before it

A build cache can go stale in a way that makes one step disagree with another,
and the disagreement is the symptom worth recognising rather than debugging.
Observed on the Windows verification host: step 6 reported that a trait was
missing methods its callers use, on a working tree where step 3 and step 4 had
both just passed over the same source. `cargo doc` had checked the dependents
against metadata for two crates that was thirteen commits old.

`cargo clippy` runs through `RUSTC_WORKSPACE_WRAPPER` and writes its check
artifacts under a fingerprint of its own, so a green step 3 does not refresh what
step 6 consumes. When one step reports an API that the source plainly does not
have, do not read the error: rebuild what it read.

```sh
cargo clean -p <the crates the error names>
```

Then re-run from step 1. A cache that was wrong once about one artifact kind was
not necessarily right about the others, and a verification is worth only as much
as the freshness of what it compiled — so if a run needed this, say so alongside
its result, or `cargo clean` and take the run again.

## Branch strategy

`main` contains released, production-ready code. Development for each release
is collected in a version branch named `dev/x.y.z`, where `x.y.z` is a semantic
version without a leading `v` (for example, `dev/0.2.0`).

Use short-lived topic branches for individual changes:

- `feat/<name>` for features
- `fix/<name>` for bug fixes
- `docs/<name>` for documentation
- `refactor/<name>` for refactoring
- `test/<name>` for tests
- `chore/<name>` for maintenance
- `ci/<name>` for CI changes
- `build/<name>` for build changes
- `perf/<name>` for performance changes

Branch names after the prefix must contain lowercase letters, numbers, `.`,
`_`, or `-`.

## Pull request flow

1. Create `dev/x.y.z` from `main` for the next release.
2. Create each topic branch from that `dev/x.y.z` branch.
3. Open a pull request from the topic branch into the same `dev/x.y.z` branch.
4. When the release is ready, open a pull request from `dev/x.y.z` into `main`.
5. After merging into `main`, tag the release as `vx.y.z`.

Emergency fixes follow the same flow: create the next patch-version
`dev/x.y.z` branch from `main`, then merge a `fix/<name>` branch into it.

Pull requests directly from topic branches into `main`, or from one version
development branch into another, are rejected by the branch policy check.

## Release checklist

A version branch is not a release by itself. Use this order so that the permanent
tag, public notes, and native verification identify one final source revision:

1. On a short-lived topic branch from `dev/x.y.z`, add or update the canonical
   `docs/releases/vx.y.z.md` notes. Confirm that the workspace version, public
   behavior, prerequisites, compatibility, artifact list, limitations, security
   boundaries, and retained evidence all agree.
2. Run the complete [verification](#verification) sequence. Retained benchmark
   or ABI evidence from an earlier revision needs a reviewed applicability record
   covering the complete intervening diff; otherwise rerun it on its named native
   host.
3. Merge the release-readiness topic branch to `dev/x.y.z` through its protected
   pull request and wait for every required check.
4. Open the release pull request from `dev/x.y.z` to `main`. Verify that its head
   contains only the intended release history and that its checks pass.
5. Merge the release pull request, record the full resulting `main` commit id,
   and verify that `origin/main` contains the accepted development history. The
   required Rust statuses are the release pull-request checks from step 4; they
   are not rerun only because the protected merge created a new `main` commit.
6. Create an annotated `vx.y.z` tag at that verified `main` commit and push that
   tag. Never tag the version-branch head before the release merge.
7. Publish the release-provider record using the tracked
   `docs/releases/vx.y.z.md` file verbatim as its body. Verify the tag, body,
   source archives, and public URL.
8. Create the next `dev/x.y.z` branch from the released `main` commit before
   accepting implementation for that version.

A published version tag is immutable: never move, delete, or reuse it to replace
a release. Correct a source or documentation defect in a later semantic version.
A release-provider metadata correction may clarify the record only when it does
not change what the tagged source contains or claims.

## Protected branches

The repository rulesets protect `main` and all `dev/*` branches:

- Changes must be submitted through a pull request.
- The branch policy check must pass.
- Review conversations must be resolved.
- Merge commits, squash merging, and rebasing are allowed.
- Force pushes are blocked.

Deletion is blocked for `main`. Merged topic and `dev/*` branches are deleted
automatically; deletion remains allowed for `dev/*` so completed release
branches do not accumulate.

The required approval count is currently zero because a pull request author
cannot approve their own pull request. Increase it to one or more when another
maintainer is available to review changes.

## Required status checks

`.github/rulesets/*.json` are tracked exports that describe the intended state of
the branch rulesets. They are not applied automatically, so editing them does not
change the live repository configuration.

The stable required pull-request check names are:

| Check | Workflow | What it verifies |
|---|---|---|
| `Validate branch flow` | `branch-policy.yml` | The source and target branches follow the pull request flow. |
| `Repository policy` | `rust.yml` | Package inventory, dependency directions, formatting, and dependency policy. |
| `Windows x86_64-pc-windows-msvc` | `rust.yml` | Native `windows-2025` inventory, lint, test, doctest, documentation, C/C++ checks, the required Lane A semantic run, and compilation of Lane B/C. It does not execute approved-host WGC Lane B. |
| `macOS aarch64-apple-darwin` | `rust.yml` | Native `macos-26` Apple Silicon inventory, lint, test, doctest, documentation, C/C++ checks, the required Lane A semantic run, and compilation of Lane B/C. It has no Screen Recording authority and does not execute ScreenCaptureKit Lane B. |

Every pull request targeting `main` or `dev/**` runs these checks without a path
filter. A `dev/**` push normally runs Repository policy and both native jobs
again after integration. Before those push jobs start, a lightweight
`Select dev push checks` job looks for an open `dev/x.y.z` → `main` release PR
in this repository whose head branch and commit exactly match the push.
Only a confirmed match suppresses the three duplicate push jobs; draft release
PRs count because their PR checks also run. A closed PR, another repository,
another branch, or a different head commit does not suppress verification.
API errors, unreadable responses, or a failed selector leave push checks enabled.

The lookup needs only `pull-requests: read`, scoped to the selector job in
addition to its checkout permission. Pull-request jobs do not perform this lookup
and retain their existing permissions and required names. Push job names append
` (dev push)`, so their skipped statuses cannot satisfy a required PR check.
The small selector run and skipped push jobs remain visible in Actions; this
avoids duplicate builds, not every extra workflow entry.

The decision uses the PR state observed by the selector, not an atomic snapshot
with PR creation or closure. Opening a release PR after the selector has decided
to run does not cancel that push run; closing it after a skip does not replay
the skipped push. Subsequent pushes without a covering release PR run normally.
Superseded PR runs still cancel only earlier runs of that same PR. `main` pushes
remain disabled; release-PR checks remain the release merge gate.

The `Repository policy` job builds no product package, which is why
documentation, lints, tests, and the C and C++ boundary are verified only in the
two native jobs. Building any product package needs OpenCV and a loadable
libclang, because `mado-pilot-backend-opencv` generates its bindings at build
time, and that installation exists on the two release targets rather than on a
host that is neither. Of the verification sequence above, steps 3 through 6,
step 8, Lane A execution, and Lane B/C compilation therefore run only in the two
native jobs; steps 2 and 7 run only in the repository-policy job, and step 1 runs
in all three. The separate approved-host Lane B jobs are support evidence, not
GitHub required-status names.

A check is activated as a live required status only after that check has produced
its first successful run on the branch it will guard, and only with separate
maintainer authorization. Enabling a required check that has never reported turns
every open pull request into a blocked pull request.

The native jobs name `windows-2025` and `macos-26` rather than a `-latest` label, so
moving to a new operating-system version is a reviewed change. Those labels still
pin only an OS version: GitHub migrates the images behind them on its own schedule,
so the toolchain and the exact image contents are not frozen by the label. The
`Repository policy` job deliberately uses the moving `ubuntu-latest`, because it
verifies host-independent policy rather than a release target.
