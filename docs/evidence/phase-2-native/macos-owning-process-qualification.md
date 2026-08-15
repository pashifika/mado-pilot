# Phase 2.2 macOS owning-process observed report

## Decision

The Phase 2.2 native qualification matrix passes on source commit `a4b12ffb89e0ef5e70ddf229a258c74dbe74a9dd` and source tree `f928a7059b47ce5b8e2dbdd970317c0e5e4c1b90`.

All fourteen in-scope operation/target/coordinate-space pairs are `qualified` under the frozen qualification plan:

- target classes: the controlled AppKit-renderer fixture window and the controlled OpenGL game-like-renderer fixture window;
- operations: `Pointer`, `Keyboard`, and `Text`;
- pointer coordinate spaces: `CapturePixels`, `FrameNormalized`, `TargetNormalized`, `TargetLogical`, and `DesktopLogical`;
- delivery: explicit `ProcessDirected` only;
- address scope: `OwningProcess`;
- compatibility: `Unknown`;
- submission evidence: `InvocationOnly`;
- focus behavior: preserve the unrelated foreground application;
- target state: the exact retained window remains open, unminimized, on-screen, and owned by the retained process lifetime.

This decision does not claim exact-window delivery, responder selection, queue admission, application consumption, visual effect, arbitrary-application compatibility, or general game compatibility. Display targets, minimized or off-screen windows, and replacement process lifetimes remain outside the qualified scope. `System` remains a separate focus-dependent route.

## Qualified provenance

The final source-bound native rows were recorded on 2026-08-15 JST. Product, fixture, and qualification-oracle source remained unchanged during all accepted rows. Benchmark profiles and this report were produced afterward as evidence artifacts and do not alter the qualified implementation tree.

| Field | Observed value |
|---|---|
| Base revision | `ffb1823b68ba632b4fc8e7725361ea4596e220f0` |
| Qualified source commit | `a4b12ffb89e0ef5e70ddf229a258c74dbe74a9dd` |
| Qualified source tree | `f928a7059b47ce5b8e2dbdd970317c0e5e4c1b90` |
| Branch | `feat/phase-2-2-macos-process-directed-delivery` |
| Host | `MacBookPro18,3`; Apple M1 Pro, 10 cores, 32 GiB, `arm64` |
| OS | macOS 26.5.2, build 25F84 |
| Minimum deployment target | macOS 26.5.2 |
| SDK | macOS SDK 26.5 from Xcode Command Line Tools |
| Apple compiler | Apple clang 21.0.0 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`; `cargo 1.97.1` |
| Fixture protocol | version 9; one outstanding command; 512-byte command/result line bound; 1,024-byte ready-record bound; 1,024-event recorder bound; owned-child teardown |
| Fixture source set | SHA-256 `474d73530ed02b9060f065fb9df483d0a0e5a647ce14a76eab907b89bbd5f0c6` |
| Fixture launch/signing | bundled launch; structurally valid ad-hoc signature; approved identifier matched |
| Authorization | Screen Recording granted; non-prompting event-post access granted; required observations agreed |
| Prompting behavior | no permission request, settings opener, target activation, or target raise |
| Final standing topology | three online, non-mirrored displays: 2× main at `(0,0)`, 2× secondary at `(2560,268)`, and 1× secondary at signed origin `(-3840,109)` |

Qualification executable hashes:

| Artifact role | Bytes | SHA-256 |
|---|---:|---|
| Native input integration test executable | 4,541,104 | `04d2c35362fc1c2435b2365c7a4e5bda7ee45f33979e5c24113bd287e05f1a5e` |
| macOS unit/scenario test executable | 4,860,344 | `0f235eb767e2dae7b9da7cbfa3545d63700833ed19feccb8d4048b301a3f2094` |
| native-phase2 benchmark executable | 1,377,840 | `c8b4148a227d55a5e672afb338ebd6f491f7aab554dc5e4df02b877e9c0faef6` |
| Release fixture executable | 599,936 | `d355de10628df7f0c938e4dc72df22b3dc3666997d9e638865b8d624cd6c8ece` |
| C common-flow executable | 34,880 | `c47e516890e5aa69130ef988e7d3e04e1cb941c413d6f820a27df02f1c10f6aa` |
| C++ common-flow executable | 327,960 | `481d470428c7bbff00a8125920f5f9bd3f79187a53c47e33f66f1a3e011a754f` |

## Executed command manifest

`$APP` denotes the repository-generated target fixture bundle and `$FOREGROUND_APP` a separately launched bundle of the same qualified executable. Absolute workstation paths are omitted. The three topology blocks were run only after the live arrangement matched their exact selector.

```sh
cargo build --locked -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
# Assemble $APP and $FOREGROUND_APP, copy the debug executable, and sign both
# ad hoc with the approved identifier.
/usr/bin/codesign --verify --strict --verbose=2 "$APP"
"$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --report-execution-context

cargo test --locked -p mado-pilot-platform-macos --features private-fixture
cargo run --locked -p mado-pilot-capi --example c-abi-check -- \
  --label "macOS Apple Silicon a4b12ff"

export MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE="$FOREGROUND_APP/Contents/MacOS/mado-pilot-macos-input-fixture"

# Run after configuring exactly one 2× display.
export MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY=single
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::every_attached_display_carries_same_frame_desktop_conversion -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_appkit_renderer -- \
  --ignored --exact --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_game_like_renderer -- \
  --ignored --exact --nocapture --test-threads=1

# Run after configuring exactly two horizontally adjacent 2× displays.
export MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY=same-scale
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::every_attached_display_carries_same_frame_desktop_conversion -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::horizontally_adjacent_displays_share_one_desktop_seam -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_appkit_renderer -- \
  --ignored --exact --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_game_like_renderer -- \
  --ignored --exact --nocapture --test-threads=1

# Run after restoring the recorded 2×/2×/1× signed-origin arrangement.
export MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY=mixed-scale
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::every_attached_display_carries_same_frame_desktop_conversion -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::horizontally_adjacent_displays_share_one_desktop_seam -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::mixed_scale_displays_publish_their_own_frame_geometry -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_appkit_renderer -- \
  --ignored --exact --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_game_like_renderer -- \
  --ignored --exact --nocapture --test-threads=1

for test in \
  controlled_unrelated_activity_remains_outside_appkit_process_evidence \
  controlled_unrelated_activity_remains_outside_game_like_process_evidence \
  sustained_capture_soak_keeps_process_route_isolated \
  process_directed_pointer_refuses_offscreen_and_closed_targets \
  process_directed_delivery_uses_process_authority_and_revalidates_window_state \
  owned_fixture_control_is_versioned_idempotent_and_identity_bound \
  owned_window_replacement_never_retargets_the_retained_filter \
  interactive_system_delivery_targets_only_the_exact_fixture
do
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
    --test native_input "$test" -- \
    --ignored --exact --nocapture --test-threads=1
done

cargo build --locked --release -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
# Reassemble and verify the release $APP before benchmarking.
/usr/bin/codesign --verify --strict --verbose=2 "$APP"

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set <capture|transitions|process-directed|process-directed-game-like|process-diagnostics> \
  --fixture-executable "$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --source-revision a4b12ffb89e0ef5e70ddf229a258c74dbe74a9dd \
  --source-tree f928a7059b47ce5b8e2dbdd970317c0e5e4c1b90 \
  --toolchain "rustc 1.97.1; Apple clang 21.0.0; macOS SDK 26.5" \
  --gpu-driver "Apple integrated GPU; system driver stack" \
  --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
  --os-version "macOS 26.5.2 (25F84)" \
  --display-topology "<recorded three-display mixed-scale topology>" \
  --permissions-signing "<recorded non-sensitive authorization/signing facts>"

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set input \
  --fixture-executable "$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --c-executable "<c-abi-check>/macos-native-input" \
  --cpp-executable "<c-abi-check>/macos-native-input-cpp" \
  --source-revision a4b12ffb89e0ef5e70ddf229a258c74dbe74a9dd --source-tree f928a7059b47ce5b8e2dbdd970317c0e5e4c1b90 \
  <the same recorded host, toolchain, topology, and permission options>

cargo run --locked --package mado-pilot-dependency-check
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked -p mado-pilot-testkit \
  --test benchmark_block_drift --test hard_budget_drift
```

## Display topology outcomes

All topologies were non-mirrored. The display-frame and renderer rows were run separately for each exact selector; one topology was never substituted for another.

| Selector | Live public geometry | Renderer window frame summary | Outcome |
|---|---|---|---|
| `single` | one display: 1512×982 logical / 3024×1964 backing at `(0,0)`, 2× | AppKit: sequence 1121, geometry 3; OpenGL: sequence 1100, geometry 3; both 688×484 logical / 1376×968 backing at 2× | passed |
| `same-scale` | 2560×1440 logical / 5120×2880 backing at `(0,0)` and 1512×982 logical / 3024×1964 backing at `(2560,170)`; both 2× and horizontally adjacent | AppKit: sequence 1137, geometry 3 → sequence 3427, geometry 4; OpenGL: sequence 1136, geometry 3 → sequence 3416, geometry 4 | passed |
| `mixed-scale` | 3840×2160 logical/backing at signed origin `(-3840,109)`, 1×; 2560×1440 logical / 5120×2880 backing at `(0,0)`, 2×; 1512×982 logical / 3024×1964 backing at `(2560,268)`, 2× | AppKit visits: sequence 1172/3503/1153/1190, geometry 3/4/5/6, epoch 2/2/3/4; OpenGL visits: sequence 1210/3551/1166/1177 with the same geometry/epoch progression; both closed the 2×→2×→1×→2× cycle | passed |

The three-display mixed-scale arrangement was restored after qualification. Final display-frame scenarios passed in that restored arrangement, and no fixture process remained after cleanup.

## Bounded native row outcomes

The default `private-fixture` macOS suite recorded 249 passing tests and ten interactive tests ignored by default. All ten ignored tests were then executed explicitly. Each renderer matrix completed under each mandatory topology selector on the clean qualified source.

The positive renderer matrix contained these exact bounded row counts, derived from the fixed test matrix:

- 1,610 pointer scenario rows: two renderer classes × five coordinate spaces × seven pointer sequences (`move`, leading seam endpoint, trailing seam endpoint, primary drag, secondary click, middle click, scroll) × 5/7/11 authoritative geometry stages for single/same-scale/mixed-scale topology;
- 20 stale-geometry zero-effect refusal rows across movement, resize, and inter-display transitions;
- 30 keyboard rows: five fixed-key/layout/chord cases × two renderers × three topology runs;
- 18 Unicode text rows: three BMP/surrogate/chunk-boundary/maximum-bound cases × two renderers × three topology runs;
- six cancellation-after-possible-effect rows, each reporting one owed and one completed release without fallback;
- fourteen recorded renderer topology visits: two single-display visits, four same-scale visits, and eight mixed-scale closed-cycle visits.

Every positive pointer row recorded an unchanged physical cursor, exact process-wide event kinds and payload digest, zero unrelated-process observations, preserved foreground identity, `InvocationOnly` receipt accounting, and caller-controlled strictly newer frame evidence. Geometry-changing stages exercised both `RequireUnchanged` and `ReprojectCurrent`; stale source frames were refused with zero possible effect.

The controlled unrelated-activity row passed for both renderer classes. Per renderer it recorded two process-directed sequences and four target logical events while the unrelated foreground fixture separately recorded two real `System` logical events and one private visual transition. Target visual matches from that unrelated activity remained zero; process receipts, cursor state, and foreground ownership remained unchanged.

The sustained-capture row passed for both renderer classes. Each retained stream remained active for at least 60 seconds across two spaced process-directed sequences while an additional same-process ordinary window existed. No unrelated frame, repeated content, fixture acknowledgement, or ambient redraw was accepted as product input or capture success. The separate off-screen/closed-target row refused both target-loss states before posting and proved the restored exact target recovered without retargeting.

## Route-wide decisions

| Row | Decision | Observed basis |
|---|---|---|
| RW-01 | passed | Controlled absolute-framework symbol loading, unavailable-symbol typing, shim version/layout checks, package isolation, fixture-source exclusion, and production linkage tests passed. |
| RW-02 | passed | Fresh validation joined the retained logical window, original process lifetime, current ownership relation, and eligible target state before every ordinary post. Mutable metadata alone never established authority. |
| RW-03 | passed | Replacement, exit/relaunch, stale identity, and commit-boundary reuse scenarios were refused before posting; the retained capture filter never retargeted. |
| RW-04 | passed | Every renderer row held capture for the 10-second admission dwell; both renderer soak rows held capture for at least 60 seconds. |
| RW-05 | passed | Admission and posting passed with the primary window alone and with one deterministic additional ordinary same-process window. Window count and capture UI were not authority. |
| RW-06 | passed | The unrelated fixture remained frontmost through ordinary and cleanup events; target activation and raise counts remained zero. |
| RW-07 | passed | Live non-prompting event-post preflight was granted. Deterministic grant, denial, revocation, unavailability, and disagreement orderings failed closed; no permission request or settings navigation occurred. |
| RW-08 | passed | Receipts reported `ProcessDirected`, `OwningProcess`, `InvocationOnly`, and the exact invoked logical prefix without claiming admission, exact-window delivery, consumption, or visual effect. |
| RW-09 | passed | Required process-directed attempts never substituted `System`; deterministic ordered-fallback rows closed fallback after possible native effect. The interactive `System` check remained separate. |
| RW-10 | passed | Deterministic boundary tests and live cancellation rows preserved the terminal winner and prevented later ordinary posts or result mutation. |
| RW-11 | passed | Sequence-owned private sources, newest-first bounded cleanup, conservative possible-effect accounting, close/drain, and repeated close passed. Cleanup retained exact key/button identity and possible drag destination. |
| RW-12 | passed | Owned-child identity, run nonce, provider/target identity, process lifetime, malformed/duplicate/stale record refusal, audit-token teardown, and idempotent controller close passed. Fixture acknowledgements never counted as product evidence. |
| RW-13 | passed | Off/Normal/Debug/overflow diagnostics remained bounded and privacy-safe with exact loss accounting. Tracked evidence passed the privacy review below. |
| RW-14 | passed | Native pointer/size/range validation, Objective-C exception and Rust panic containment, queue pressure, diagnostic overflow, and allocation bounds passed. |

No route-wide row is failed or unexecuted in the accepted matrix.

## Pair decisions

A pointer decision covers move, button, drag, and scroll behavior independently; the repeated reasons in the table are deliberate because the release gate is per exact pair.

| Operation | Target class | Coordinate space | Decision | Reason |
|---|---|---|---|---|
| `Pointer` | AppKit renderer | `CapturePixels` | `qualified` | RW-01–RW-14 passed; all pointer sequences, both geometry policies, stale refusal, movement, resize, single/2×-same/2×-to-1× topology, signed-origin, seam, cursor, foreground, unrelated-process, receipt, and frame rows passed. |
| `Pointer` | AppKit renderer | `FrameNormalized` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space. |
| `Pointer` | AppKit renderer | `TargetNormalized` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space. |
| `Pointer` | AppKit renderer | `TargetLogical` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space. |
| `Pointer` | AppKit renderer | `DesktopLogical` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space, including signed desktop origins and both seams. |
| `Keyboard` | AppKit renderer | not applicable | `qualified` | RW-01–RW-14, fixed/layout/chord ordering, inactive-target observation, foreground preservation, partial cancellation, cleanup, target loss, and strictly newer frame rows passed. |
| `Text` | AppKit renderer | not applicable | `qualified` | RW-01–RW-14, BMP/surrogate/chunk-boundary/maximum-bound, partial native-unit accounting, revocation/cancellation/deadline, privacy, and strictly newer frame rows passed. |
| `Pointer` | OpenGL game-like renderer | `CapturePixels` | `qualified` | The same complete matrix passed for the controlled game-like fixture only; no arbitrary-game claim is inferred. |
| `Pointer` | OpenGL game-like renderer | `FrameNormalized` | `qualified` | The same complete matrix passed for this exact controlled target and space only. |
| `Pointer` | OpenGL game-like renderer | `TargetNormalized` | `qualified` | The same complete matrix passed for this exact controlled target and space only. |
| `Pointer` | OpenGL game-like renderer | `TargetLogical` | `qualified` | The same complete matrix passed for this exact controlled target and space only. |
| `Pointer` | OpenGL game-like renderer | `DesktopLogical` | `qualified` | The same complete matrix passed for this exact controlled target and space only, including signed origins and both seams. |
| `Keyboard` | OpenGL game-like renderer | not applicable | `qualified` | The complete keyboard and route-wide matrix passed for the controlled game-like fixture only. |
| `Text` | OpenGL game-like renderer | not applicable | `qualified` | The complete text and route-wide matrix passed for the controlled game-like fixture only. |

Pair totals: 14 `qualified`, 0 `rejected`, 0 `unexecuted`. Targets and states excluded by the frozen plan are unsupported scope, not inferred pair passes.

## Performance outcomes

All 2,700 retained Phase 2.2 benchmark samples across twenty-four workloads satisfied their workload correctness oracle. Every refreshed profile passed its frozen p50, p95, hard-maximum, mapped-byte, stale-work, peak-heap, allocation-growth, and diagnostic-capacity gates. `result_correctness` was zero for every workload, and post-warmup allocation growth was zero throughout this run.

| Profile / workload | p50 ms | p95 ms | max ms | Peak live Rust bytes |
|---|---:|---:|---:|---:|
| capture / `fixture_command_acknowledgement` | 0.183542 | 0.256208 | 1.513083 | 36,933 |
| capture / `controlled_stimulus_to_frame` | 16.673500 | 17.437166 | 19.015917 | 9,300,418 |
| capture / `static_latest_retained` | 0.000125 | 0.000167 | 0.000250 | 4,671,474 |
| capture / `static_newer_repeated_pixels` | 17.367458 | 34.821459 | 39.915375 | 9,300,418 |
| capture / `latest_acquisition` | 0.000791 | 0.001000 | 0.001375 | 9,300,418 |
| capture / `cpu_map_bgra8` | 0.183000 | 0.201166 | 0.268625 | 9,300,418 |
| transitions / `resize_recreation` | 51.764500 | 60.338791 | 62.029792 | 8,329 |
| transitions / `open_first_frame` | 99.399000 | 107.193875 | 110.687083 | 5,335,291 |
| transitions / `retained_pressure_resume` | 3.758875 | 16.887041 | 18.114666 | 4,671,945 |
| transitions / `close_drain` | 55.802709 | 65.890667 | 67.838875 | 7,259 |
| AppKit / `discovery_open_retained_authority` | 322.378167 | 395.168875 | 419.255000 | 51,195 |
| AppKit / `event_authority_preflight_post` | 206.456125 | 213.065250 | 215.856541 | 4,673,362 |
| AppKit / `release_cleanup` | 1.722458 | 2.190792 | 2.462459 | 4,673,386 |
| AppKit / `session_close` | 57.619833 | 63.784750 | 65.616167 | 43,619 |
| AppKit / `fixture_controller_close` | 54.012375 | 58.990042 | 65.185459 | 37,401 |
| game-like / `discovery_open_retained_authority` | 316.509667 | 408.370834 | 440.772917 | 51,195 |
| game-like / `event_authority_preflight_post` | 206.814958 | 212.258917 | 215.157042 | 4,673,362 |
| game-like / `release_cleanup` | 1.762291 | 2.654000 | 2.725458 | 4,673,386 |
| game-like / `session_close` | 57.908250 | 65.329875 | 67.543917 | 43,619 |
| game-like / `fixture_controller_close` | 20.820125 | 31.409500 | 34.344541 | 37,413 |
| diagnostics / `event_diagnostics_off` | 206.393375 | 211.258041 | 215.148667 | 4,673,482 |
| diagnostics / `event_diagnostics_normal` | 206.294375 | 213.003083 | 219.834416 | 4,684,194 |
| diagnostics / `event_diagnostics_debug` | 207.135166 | 212.853375 | 216.599750 | 4,683,890 |
| diagnostics / `event_diagnostic_overflow` | 209.330125 | 214.299917 | 219.164000 | 4,674,314 |

The controlled capture profile retained the exact 4,628,480-byte BGRA8 mapping bound. Its acquisition workloads recorded a `0.000000000` stale-work ratio. The controlled transition profile recorded the expected `0.836601307` retained-pressure stale-work ratio and passed its frozen `0.95` ceiling. Process-directed profiles stayed below the frozen 16 MiB peak-live-heap limit.

The separately accepted ADR 0025 native-input/public-language profile was also refreshed on the same source with 300 retained samples. Its Rust common flow passed; fresh C and C++ process-load checks passed; and the explicit C and C++ `ProcessDirected` common flows completed with p95 values of 1812.596500 ms and 1974.578792 ms respectively. Receipt evidence and owned-fixture event observation remained separate in both language consumers.

Committed profile hashes:

| Profile | SHA-256 |
|---|---|
| `phase-2-2-controlled-capture-aarch64-apple-darwin.toml` | `6c82867e2cb2568af692835f418c0a6dbc666152feec2bed75cddefd12d5c7e9` |
| `phase-2-2-controlled-transitions-aarch64-apple-darwin.toml` | `11dd0617da73859fc48dd2bcbdbc818266d3426333fd894cd9e52051d6bf4407` |
| `phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml` | `364b2202830a14702a69b6ecca46b6c376a8c5e978bd1c468ee5405392efb7da` |
| `phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml` | `18f0c1602a5f6f95565d2976bb60e6b0e5301998ae6fe12011ff70b3965650d2` |
| `phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml` | `d761af47e804c50c3a73539466ea2ca9af171242c7031194273fb8f60e7dfb8f` |

## Failed, excluded, and superseded attempts

The accepted matrix has no failed or unexecuted pair row. Earlier complete matrices on source commits `8dd70810d60c06b298c806ffce16720d0a07e4c2` and `b1059cf6239042107bd62373eb65211117beaab9` were invalidated by later product, fixture, and oracle changes and contribute no final decision.

During the first final-revision benchmark attempt, the operator disclosed possible host interaction. All six outputs from that attempt were excluded before any profile promotion. After the operator reported no further activity, every workload set was rerun unattended and captured as complete raw output. One intermediate filtered game-like benchmark attempt failed before sampling with `configured capture source is invalid`; the final raw isolated run passed and is the only promoted measurement.

An additional back-to-back mixed-scale game-like renderer rerun on the final source returned `Unexecuted` with zero submitted events after its five-second operation deadline. It reported no possible effect and contributes to no pair result. After fixture cleanup and a ten-second idle interval, the same isolated full renderer matrix passed. The excluded deadline attempt remains hash-bound below rather than hidden.

Excluded attempt hashes:

| Excluded raw artifact | Bytes | SHA-256 |
|---|---:|---|
| `capture-possible-operator-interaction-attempt.log` | 5,158 | `04020e21f40270f4fed051f62271a39a6263af25d8e20c9543407b4146ed7a28` |
| `transitions-possible-operator-interaction-attempt.log` | 4,029 | `b7498b0140fd807b36037a2c4babd5a24e57f1e9b92cbaeeb136f7c3e756c7f9` |
| `input-possible-operator-interaction-attempt.log` | 5,262 | `ba46b7230f86b0bc073c888c6cd427ddf27e5eca5f075fe8518baa3293ff9cef` |
| `process-directed-possible-operator-interaction-attempt.log` | 5,112 | `ec533a8322474724e14505fa3d0d64f29101491e3177e0e0eaffb5ce1c4a448c` |
| `process-directed-game-like-possible-operator-interaction-attempt.log` | 5,125 | `d58adfe7cb3dc79027422c46d1e4a7aab8c359dd86236868c7fae1cfbaecfff7` |
| `process-diagnostics-possible-operator-interaction-attempt.log` | 4,129 | `ff42f477b966039396e2f60e8e28d79872f26945bfdda15b852ef821046a0e5e` |
| `process-directed-game-like-unattended-attempt1.log` | 583 | `9591c948e39219ddce5a35b11c78733e9193d0bc60c5ca85eb18c86ca23255c1` |
| `mixed-game-like-renderer-matrix-deadline-attempt.log` | 93,174 | `d30a155efbff5d178a9bb96ee60faf348805e76c5c10e09f51167418109b7764` |

## Raw-output provenance

Accepted raw logs remain ignored under `.rasen/changes/phase-2-2-macos-owning-process-delivery/ephemera/qualification-final-a4b12ff/`. They contain native identifiers and fixture-private records and must not be committed. The tracked binding is limited to file size and SHA-256.

| Accepted raw artifact | Bytes | SHA-256 |
|---|---:|---|
| `source-commit.log` | 66 | `f7c82ca8a4fbf3f82d80f280f2c22fa9c2bae366efbfda83da8d1bd99f4e40fc` |
| `source-tree.log` | 66 | `70b23de943f2153ea7ab481a3a868241606c77757f51780009cc35a4d071e584` |
| `host-status.log` | 90 | `969d0d6df53615ba27cc8a31ffc221ddf6429195992e872be71d01afb69c12e9` |
| `sdk.log` | 30 | `b0594ffe6b4818051966ad4e173420477863c282e17b427ce43476d6c7b1c309` |
| `rustc.log` | 61 | `a4824ad4a769e76236f8ee4f2cc26a4d1081187de1513261e19d8e92a34a0f8b` |
| `cargo.log` | 61 | `a278cac6e8410e251ab18613144ca74413b8d8926b0bba8b27f566b016cd430c` |
| `cc.log` | 184 | `0f0f5071e4ce01a4e5ce4c26b1671b7837b458493e259f42d04dff856bcda6ac` |
| `hardware.log` | 53 | `4027c1933fed7318378b4eb29a9b1a311cba991c2de2827ca6eeba06d7cfd110` |
| `displays.log` | 508 | `82afdfbc44d60370a988c1e33b3f639892d09a19d62145c6b495fe3caad9728b` |
| `single-host-baseline.log` | 1,925 | `d9a1f6cd706bac4cfcf3ecefe7a1641a1a9124c234cde16e441616ec1439e242` |
| `same-scale-host-baseline.log` | 2,612 | `3edc5b5b4045657e9822339a96fa365fdccb09cfa04d9a818379026aea33f11c` |
| `restored-mixed-host-baseline.log` | 3,362 | `45437379987c244805aa66f30ae5529641c5bc387b50beee9a56f6dc398443e7` |
| `fixture-build-debug.log` | 97 | `1fc299b7cc5e6b68bcae7735af90c6cfdb6d8101170c251df5f06292f35ba7f5` |
| `fixture-debug-verification.log` | 534 | `ad1f5ba7e7e01e8a69379bd151063504251c9c5875c23c4a59720b5b96a117fa` |
| `deterministic-macos.log` | 26,524 | `2818705bdb7ce862d4c624b6333762769bbdd01e62de2306079395207ac75494` |
| `c-abi-check.log` | 24,668 | `7e821749607c6bc4fcd6f7fafd163a56a31c3b732c7953a663bb3019444c13e6` |
| `single-display-conversion.log` | 1,716 | `1e25903157f924c106cf21892e031696589eabfa9f8b09ef58d73b9141ce30f4` |
| `same-scale-display-conversion.log` | 1,642 | `2f0c24f3485fc7caba7dd646f5d3a81caf61dbed10814f2667cf0457e39e2dfe` |
| `same-scale-display-seam.log` | 1,747 | `ffbe0f664ab38f62f85c374e328ca07315d3cadc428a9d949a8a4b9ff77eadaa` |
| `mixed-display-conversion.log` | 1,617 | `4934ea901110b28bf8d7e1fe68aacdb387551cb6a14c080645a69b76bba8aa50` |
| `mixed-display-seam.log` | 1,659 | `744c1feda4efedcb5629a6a5ee293b734a90d06e3aefb73318a1734eb53c3bb4` |
| `mixed-display-scale.log` | 1,610 | `b9fba20e73c52378748110ae65221628c3bd0660e05d94ff8d202cad1a428cfe` |
| `restored-mixed-display-conversion.log` | 1,642 | `04d24030e695a23c73e91563dbb34a75ec38c96d487049488bac5e28880a589f` |
| `restored-mixed-display-seam.log` | 1,684 | `f9081d7cbce0ba0a2d24ef99c8a812ed8f9d7abcd0da7d20474bdc372c3174e1` |
| `restored-mixed-display-scale.log` | 1,635 | `e81f0d10d27b9a58685857548539d9d83c4276b6f4df153c08957682508fd01e` |
| `single-appkit-renderer-matrix.log` | 44,261 | `d282372d983ff9c499bf14b2fa7e3766d5c8e42990281a26b95be17714a07f0e` |
| `single-game-like-renderer-matrix.log` | 44,245 | `6a879bb75c2b828396b2ab0d25497a27054c8f3ec95800b3283e0901928dd9b6` |
| `same-scale-appkit-renderer-matrix.log` | 61,225 | `b695e1cf54ede485b24e647f1aa2c62b5217d564a7076cd7d93daabd27a0b579` |
| `same-scale-game-like-renderer-matrix.log` | 61,199 | `102e22c1bc9794cee2eedd33964c3a980cc653a7ffc4ab05d1c0d815bb91ea35` |
| `mixed-appkit-renderer-matrix.log` | 94,876 | `7aba1bb0ade90a92e48ca93f96381f3f3d7d20e72ff41b909241922450220474` |
| `mixed-game-like-renderer-matrix.log` | 94,830 | `02e6816c59b151ae5c5d0487fc158170b3ebd250516ad506171a111413c2446e` |
| `unrelated-appkit.log` | 2,354 | `7b1fee7f87de24550cb2d8c97a478286988c1eebdaf31b6f01f9ba671d990ae8` |
| `unrelated-game-like.log` | 2,338 | `d2490d602bfb5dfa20acc11a99e0d7ec947564be5f98d26a12fac1c7988b5503` |
| `sustained-capture-soak.log` | 2,154 | `4db780b0b8ebdfd144fb8a6f8e05e9815784096527db48232d3f428f2b858ce1` |
| `offscreen-cleanup.log` | 2,494 | `0354cf9ae3029cb1f7ed34afee91967038c520ef097a868646059f88d8157684` |
| `retained-authority-lifecycle.log` | 3,283 | `dd4646c00a85ada5c2321094750b74fcde693edacc27f004f9758541638a8e9f` |
| `fixture-control.log` | 1,401 | `3d228efe5c08730092d86f656fb9b77c0f79469352e87c852f63d4e8dcc9ff0f` |
| `retained-filter-replacement.log` | 646 | `a1f567e81a5f1d951708e33efa31c7ad37217708167918c51f382ea7e862e7e1` |
| `system-route.log` | 526 | `ed1e6ce46bcf364f5091145a417abbf4083abee3237fd1beac0fdccbed60e053` |
| `fixture-build-release.log` | 87 | `2a497654e77e53515cdbee79aba1cd119c3e80bc45f80cef6d6439dadb1eb9ee` |
| `fixture-release-verification.log` | 534 | `780a966d47c8ea99f15cdd4d2c18fc3afc2af652100657531f162b8edd60ca1a` |
| `dependency-check.log` | 3,276 | `8b7a14ea0f1e9316a1140b7d3cbbb48d27c37fe5f9ba980c7f8bb49877650c8b` |
| `cargo-fmt.log` | 36 | `31908f9d1f394e141d5a965a10109ad013c1d87ab6301208db1cb7ae1fe599e7` |
| `cargo-clippy.log` | 1,268 | `c4127ae6c32f81a3c5d8a880e3c378d6fc9c7d40caa3537256c5fe7401f33c0c` |
| `cargo-test-workspace.log` | 108,424 | `fca205781b76c3c952f8496f8238efcc767d137afe90dbf2ae2ddacfb47c9854` |
| `capture.log` | 5,218 | `ed881b0d5851f3ab5caad32f041cb2347fa089e07704b3fcf53b863b6e4fe89c` |
| `transitions.log` | 4,092 | `b0f822d02b07c8f8bee62849551e59d9d4a84a05786185d0a5ec8d9f1c886cb9` |
| `process-directed.log` | 5,161 | `cfdbbd58f4116ea588dada57eed1b7d83d6bd4f73a1695b1bdffafdfd5aa2e99` |
| `process-directed-game-like.log` | 5,168 | `bf83511271801d56a579b41fa92e997c42c0c85cd7ab60ce7a3149bdb1020ced` |
| `process-diagnostics.log` | 4,183 | `356e7d77c45b58ee96208f6e07cdee4686e595a877ae62a5a3c73ade95a80270` |
| `input.log` | 5,580 | `9a307d066517fa5899d08e8bdb841b5d1fb898ca05c7c2bbb1b9b2d005c6dfb7` |
| `fixture-process-final.log` | 61 | `02c723f42cf515a06f51e362b8999a0d90c61b12f7c0b8965be20ee1bf2907ea` |

## Privacy review

This tracked report contains no captured pixels, recognized or submitted text, window titles, application names, native process identifiers, native window numbers, signing identifiers, raw authorization values, credentials, fixture-private payloads, unrelated foreground identity, or process inventory. Display geometry, internal frame stamps, bounded event/sample counts, typed outcomes, artifact sizes, and hashes are retained because they are required qualification facts.

## Strict release gate

Every route-wide row and every mandatory accepted row for each of the fourteen exact pairs passed on the qualified source revision. The single-display, same-scale, and mixed-scale results are independently recorded. Therefore ADR 0029 remains Accepted for these controlled pair contracts, and public descriptors and documentation may advertise them with `Unknown` compatibility, `OwningProcess` address scope, `InvocationOnly` evidence, foreground-preserving behavior, and explicit caller opt-in.

No support statement may broaden this result to exact-window delivery, arbitrary applications, arbitrary games, display targets, minimized/off-screen targets, application consumption, or visual success. Any later product, fixture, qualification-oracle, budget, or mandatory-documentation change invalidates the affected acceptance rows and requires a new revision-bound report.
