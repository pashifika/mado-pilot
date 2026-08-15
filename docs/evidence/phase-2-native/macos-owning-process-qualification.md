# Phase 2.2 macOS owning-process observed report

## Decision

The Phase 2.2 native qualification matrix passes on source commit `b1059cf6239042107bd62373eb65211117beaab9` and source tree `0fffe5587d29bed8c6908e7b852617be8b381c13`.

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

The native rows were recorded on 2026-08-15 JST. Product and fixture source were unchanged during the qualifying runs; tracked benchmark profiles and this report were produced afterward as evidence artifacts.

| Field | Observed value |
|---|---|
| Base revision | `ffb1823b68ba632b4fc8e7725361ea4596e220f0` |
| Qualified source commit | `b1059cf6239042107bd62373eb65211117beaab9` |
| Qualified source tree | `0fffe5587d29bed8c6908e7b852617be8b381c13` |
| Branch | `feat/phase-2-2-macos-owning-process-delivery` |
| Host | `MacBookPro18,3`; Apple M1 Pro, 10 cores, 32 GiB, `arm64` |
| OS | macOS 26.5.2, build 25F84 |
| Minimum deployment target | macOS 26.5.2 |
| SDK | macOS SDK 26.5 from Xcode Command Line Tools |
| Apple compiler | Apple clang 21.0.0 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`; `cargo 1.97.1` |
| Fixture protocol | version 5; one outstanding command; 1,024-byte line bound; owned-child teardown |
| Fixture launch/signing | bundled launch; structurally valid ad-hoc signature; approved identifier matched |
| Authorization | Screen Recording granted; non-prompting event-post access granted; required observations agreed |
| Prompting behavior | no permission request, settings opener, target activation, or target raise |
| Final standing topology | three online, non-mirrored displays; 2× main, 2× secondary, and 1× signed-origin secondary |

Qualification executable hashes:

| Artifact role | Bytes | SHA-256 |
|---|---:|---|
| Native input integration test executable | 4,064,704 | `fcc0e5612b07b7156b1168b085d6d9e2608412c6f1f2e9791d3f7ca681766610` |
| macOS unit/scenario test executable | 4,541,224 | `48f644f467a2767f9be40d69213bc2dfebdf1f514ab441ee996c8b621f7e5cf2` |
| `native-phase2` benchmark executable | 1,332,320 | `08a53f48be2ff04557f5f16148e1106d4ef8e04edb48de48907914d2efd750f7` |
| Release fixture executable | 598,464 | `1360206fc656a55385bde2c7d5f36badc869a8966731786ebd5f0b80ff47e7a2` |
| Fixture source set | — | `2252287e99fc2f14645326eb76b8c3b4a9ba4bb53f81fdf196f172d9ee3ac851` |

## Executed command manifest

`$APP` below denotes the repository-generated fixture bundle. Absolute workstation paths were normalized; every other option is the executed option.

```sh
cargo build --locked --release -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
# Assemble $APP, copy the release executable, and sign ad hoc with the approved identifier.
/usr/bin/codesign --verify --strict --verbose=2 "$APP"

cargo test --locked -p mado-pilot-platform-macos --features private-fixture

cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::every_attached_display_carries_same_frame_desktop_conversion -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::horizontally_adjacent_displays_share_one_desktop_seam -- \
  --exact --nocapture
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  scenarios::mixed_scale_displays_publish_their_own_frame_geometry -- \
  --exact --nocapture

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY="<single|same-scale|mixed-scale>" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  process_directed_delivery_qualifies_default_and_game_like_renderers -- \
  --ignored --exact --nocapture --test-threads=1

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  process_directed_delivery_uses_process_authority_and_revalidates_window_state -- \
  --ignored --exact --nocapture --test-threads=1

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  controlled_unrelated_activity_remains_outside_process_evidence -- \
  --ignored --exact --nocapture --test-threads=1

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  sustained_capture_soak_keeps_process_route_isolated -- \
  --ignored --exact --nocapture --test-threads=1

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  owned_fixture_control_is_versioned_idempotent_and_identity_bound -- \
  --ignored --exact --nocapture --test-threads=1

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  owned_window_replacement_never_retargets_the_retained_filter -- \
  --ignored --exact --nocapture --test-threads=1

MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  interactive_system_delivery_targets_only_the_exact_fixture -- \
  --ignored --exact --nocapture --test-threads=1

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set <capture|transitions|process-directed|process-directed-game-like|process-diagnostics> \
  --fixture-executable "$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --source-revision b1059cf6239042107bd62373eb65211117beaab9 \
  --source-tree 0fffe5587d29bed8c6908e7b852617be8b381c13 \
  --toolchain "rustc 1.97.1; Apple clang 21.0.0; macOS SDK 26.5" \
  --gpu-driver "Apple integrated GPU; system driver stack" \
  --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
  --os-version "macOS 26.5.2 (25F84)" \
  --display-topology "<recorded three-display mixed-scale topology>" \
  --permissions-signing "<recorded non-sensitive authorization/signing facts>"

cargo test --locked -p mado-pilot-testkit \
  --test benchmark_block_drift --test hard_budget_drift
```

## Display topology outcomes

All topologies were non-mirrored. The display rows and renderer rows were run separately for each exact selector; one topology was never substituted for another.

| Selector | Live public geometry | Renderer window frame summary | Outcome |
|---|---|---|---|
| `single` | one display: logical 1512×982, backing 3024×1964, origin `(0,0)`, scale 2×; display frame stream `(1,1)`, epoch 0, sequence 0, geometry 0 | AppKit stream `(3,1)`: epoch 2, sequence 698, geometry 3; OpenGL stream `(2,1)`: epoch 2, sequence 708, geometry 3; both 688×484 logical / 1376×968 backing at 2× | passed |
| `same-scale` | display A: 2560×1440 logical / 5120×2880 backing at `(0,0)`; display B: 1512×982 logical / 3024×1964 backing at `(2560,170)`; both 2× and joined at the horizontal seam; display streams `(1,2)` and `(1,1)`, epoch 0, sequence 0, geometry 0 | AppKit stream `(3,1)`: geometry 3→4, sequence 731→2176; OpenGL stream `(2,1)`: geometry 3→4, sequence 725→2140; both visits remained 2× | passed |
| `mixed-scale` | 1× display: 3840×2160 logical/backing at signed origin `(-3840,109)`; 2× main: 2560×1440 logical / 5120×2880 backing at `(0,0)`; 2× secondary: 1512×982 logical / 3024×1964 backing at `(2560,268)`; display streams `(1,3)`, `(1,2)`, and `(1,1)`, epoch 0, sequence 0, geometry 0 | AppKit stream `(3,1)`: geometry 3→4→5, sequence 721→2168→1427 with epoch 2→2→3; OpenGL stream `(2,1)`: geometry 3→4→5, sequence 751→2212→1432 with epoch 2→2→3; each crossed 2×/2× and 2×/1× seams | passed |

The original three-display mixed-scale arrangement was restored after qualification. No fixture process remained after cleanup.

## Bounded native row outcomes

The default `private-fixture` macOS suite recorded 214 passing tests and seven interactive tests ignored by default. All seven ignored tests were then executed explicitly. The renderer test ran once successfully on the clean qualified revision for each mandatory topology; earlier excluded attempts are recorded separately below.

The positive renderer matrix contained these exact bounded row counts, derived from the fixed test matrix:

- 1,050 pointer scenario rows: two renderer classes × five coordinate spaces × five pointer sequences (`move`, primary drag, secondary click, middle click, scroll) × 5/7/9 authoritative geometry stages for single/same-scale/mixed-scale topology;
- 18 stale-geometry zero-effect refusal rows across movement, resize, and inter-display transitions;
- 30 keyboard rows: five fixed-key/layout/chord cases × two renderers × three topology runs;
- 18 Unicode text rows: three BMP/surrogate/chunk-boundary/maximum-bound cases × two renderers × three topology runs;
- six cancellation-after-possible-effect rows, each reporting one owed and one completed release without fallback;
- twelve recorded renderer topology visits: two single-display visits, four same-scale visits, and six mixed-scale visits.

Every positive pointer row recorded an unchanged physical cursor, exact process-wide event kinds, zero unrelated-process observations, preserved foreground identity, `InvocationOnly` receipt accounting, and caller-controlled strictly newer frame evidence. Geometry-changing stages exercised both `RequireUnchanged` and `ReprojectCurrent`; stale source frames were refused with zero possible effect.

The controlled unrelated-activity row passed for both renderer classes. Per renderer it recorded two process-directed sequences and four target logical events while the unrelated foreground fixture separately recorded two real `System` logical events and one private visual transition. Target visual matches from that unrelated activity remained zero; process receipts, cursor state, and foreground ownership remained unchanged.

The sustained-capture row passed for both renderer classes. Each retained stream remained active for at least 60 seconds across two spaced process-directed sequences while an additional same-process ordinary window existed. No unrelated frame, repeated content, fixture acknowledgement, or ambient redraw was accepted as product input or capture success.

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
| RW-11 | passed | Sequence-owned private sources, newest-first bounded cleanup, partial-effect accounting, cleanup failure typing, close/drain, and repeated close passed. |
| RW-12 | passed | Owned-child identity, run nonce, provider/target identity, process lifetime, malformed/duplicate/stale record refusal, and idempotent controller teardown passed. Fixture acknowledgements never counted as product evidence. |
| RW-13 | passed | Off/Normal/Debug/overflow diagnostics remained bounded and privacy-safe with exact loss accounting. Tracked evidence passed the privacy review below. |
| RW-14 | passed | Native pointer/size/range validation, Objective-C exception and Rust panic containment, queue pressure, diagnostic overflow, and allocation bounds passed. |

No route-wide row is failed or unexecuted on the qualified revision.

## Pair decisions

A pointer decision covers move, button, drag, and scroll behavior independently; the repeated reasons in the table are deliberate because the release gate is per exact pair.

| Operation | Target class | Coordinate space | Decision | Reason |
|---|---|---|---|---|
| `Pointer` | AppKit renderer | `CapturePixels` | `qualified` | RW-01–RW-14 passed; all pointer sequences, both geometry policies, stale refusal, movement, resize, single/2×-same/2×-to-1× topology, signed-origin, seam, cursor, foreground, unrelated-process, receipt, and frame rows passed. |
| `Pointer` | AppKit renderer | `FrameNormalized` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space. |
| `Pointer` | AppKit renderer | `TargetNormalized` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space. |
| `Pointer` | AppKit renderer | `TargetLogical` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space. |
| `Pointer` | AppKit renderer | `DesktopLogical` | `qualified` | Same complete route-wide and pointer matrix passed for this exact space, including signed desktop origins and both seams. |
| `Keyboard` | AppKit renderer | not applicable | `qualified` | RW-01–RW-14, fixed/layout/chord ordering, inactive-target observation, foreground preservation, partial cancellation, cleanup, target-loss, and strictly newer frame rows passed. |
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

All 2,700 retained benchmark samples satisfied their workload correctness oracle. Every committed profile passed its frozen p50, p95, hard-maximum, mapped-byte, stale-work, peak-heap, allocation-growth, and diagnostic-capacity gates. `result_correctness` was zero for every workload. Post-warmup allocation growth was 2,624 bytes for each renderer's discovery/authority workload and zero for every other workload, within the frozen 4,096-byte ceiling.

| Profile / workload | p50 ms | p95 ms | max ms | Peak live Rust bytes |
|---|---:|---:|---:|---:|
| capture / `fixture_command_acknowledgement` | 0.161209 | 0.208667 | 1.582708 | 36,775 |
| capture / `controlled_stimulus_to_frame` | 16.515042 | 17.521792 | 18.654458 | 9,301,169 |
| capture / `static_latest_retained` | 0.000125 | 0.000167 | 0.000958 | 4,672,225 |
| capture / `static_newer_repeated_pixels` | 17.190208 | 19.919250 | 24.748084 | 9,301,169 |
| capture / `latest_acquisition` | 0.000500 | 0.000666 | 0.001125 | 9,301,473 |
| capture / `cpu_map_bgra8` | 0.187708 | 0.204125 | 0.229375 | 9,301,169 |
| transitions / `resize_recreation` | 51.460583 | 52.916083 | 54.095250 | 9,130 |
| transitions / `open_first_frame` | 103.262875 | 113.477667 | 118.326084 | 5,336,114 |
| transitions / `retained_pressure_resume` | 3.269750 | 16.618500 | 19.168375 | 4,672,695 |
| transitions / `close_drain` | 60.519916 | 71.387750 | 77.966958 | 8,082 |
| AppKit / `discovery_open_retained_authority` | 323.165792 | 386.449750 | 395.665458 | 59,203 |
| AppKit / `event_authority_preflight_post` | 161.651125 | 165.862500 | 166.511375 | 9,302,266 |
| AppKit / `release_cleanup` | 54.879625 | 57.400417 | 59.049334 | 9,302,644 |
| AppKit / `session_close` | 58.711500 | 69.374500 | 70.884833 | 44,449 |
| AppKit / `fixture_controller_close` | 0.000041 | 0.000042 | 0.000167 | 34,256 |
| game-like / `discovery_open_retained_authority` | 321.844125 | 358.041583 | 386.030833 | 59,203 |
| game-like / `event_authority_preflight_post` | 161.653333 | 165.512708 | 166.741583 | 9,302,266 |
| game-like / `release_cleanup` | 54.185375 | 58.213084 | 63.393042 | 9,302,644 |
| game-like / `session_close` | 60.345667 | 74.783417 | 77.797042 | 44,417 |
| game-like / `fixture_controller_close` | 0.000041 | 0.000042 | 0.000125 | 34,256 |
| diagnostics / `event_diagnostics_off` | 159.394459 | 165.798083 | 170.064417 | 4,674,650 |
| diagnostics / `event_diagnostics_normal` | 159.622375 | 165.047625 | 168.632750 | 4,684,818 |
| diagnostics / `event_diagnostics_debug` | 160.103625 | 165.193625 | 169.715458 | 4,685,154 |
| diagnostics / `event_diagnostic_overflow` | 162.511791 | 167.470750 | 169.508500 | 4,677,674 |

The controlled capture profile retained the exact 4,628,480-byte BGRA8 mapping bound. Its `latest_acquisition` workload recorded a `0.004975124` stale-work ratio. The controlled transition profile recorded the expected `0.835526316` retained-pressure stale-work ratio and passed its frozen `0.95` ceiling. Process-directed profiles stayed below the frozen 16 MiB peak-live-heap limit.

Committed profile hashes:

| Profile | SHA-256 |
|---|---|
| `phase-2-2-controlled-capture-aarch64-apple-darwin.toml` | `5385e193a07e1884ff861529315845232bd728fcbc296a140e475b84e4606101` |
| `phase-2-2-controlled-transitions-aarch64-apple-darwin.toml` | `e7b89ac2995b01ed45d863fa8227693cbdc23264df7a9cfa8c1eb0b9d58da0cc` |
| `phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml` | `3e03d9edaf827b173d169cbeab43dc50eaf149a8cae81dd354edb7db5a2425ee` |
| `phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml` | `dab54f8ff034f3a469cc0cc60b3e1d8340f4974fd20d26150b8e4d7472283c50` |
| `phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml` | `78c7f55f9ada27c93076e70b65dc5a5239c8c437ae41b1f610d5e8aa89abd4f9` |

## Failed, excluded, and skipped attempts

No mandatory row failed or remained unexecuted on the final qualified revision. Every final-revision deterministic, display, renderer, authority, lifecycle, replacement, unrelated-activity, sustained-capture, System-route isolation, benchmark, and budget-check command completed successfully.

Earlier candidate runs on superseded source revisions included one unsolicited-pointer-input stop, two clean-worktree prerequisite exclusions, and one pre-sampling capture-open setup failure. They remain separate ignored diagnostic ephemera and contribute to no final-revision row, profile, or pair decision. The complete matrix was rerun after the final code-only commit; no equivalent failure, timeout, stale commit, dropped result, allocation-growth breach, or budget breach occurred.

The repository's `window_movement` integration tests also passed on the final revision. Mandatory movement, resize, signed-origin, and inter-display transitions ran through the owned fixture commands in every renderer/topology matrix above.

## Raw-output provenance

Raw logs remain ignored under `.rasen/changes/phase-2-2-macos-owning-process-delivery/ephemera/qualification-final-b1059cf/`. They contain native identifiers and fixture-private records and must not be committed. The tracked binding is limited to file size and SHA-256.

| Raw artifact | Bytes | SHA-256 |
|---|---:|---|
| `deterministic-macos.log` | 21,949 | `b6ca95e424fb7d320a61699a4831d25e86157de293714d94935f01a652df317e` |
| `fixture-control.log` | 4,870 | `9316ee75820899555a0633934ec5e1f48b0be73f268d2d8ac78ec3906e631df5` |
| `mixed-display-conversion.log` | 1,617 | `56330e30b6f912b25b974482a5e183838dbbd6e89d2ee7b99fab2eed64de7eba` |
| `mixed-display-scale.log` | 1,610 | `bd51d1b060fb9efe68074a9dc5676397b5e8bd448cabebafa31071e630c35627` |
| `mixed-display-seam.log` | 1,659 | `409219517feb5d0fa67527e00cfcd978a059f44bad8f61f87d15212437b704dc` |
| `mixed-renderer-matrix.log` | 487,592 | `d321c1ed9e7bbfb9ec4a233d83fd9545a1d2a0d833638a136cacd16eefcbb3fb` |
| `mixed-scale-placement.log` | 161 | `f457454f4819c7070b4d133d31ebac75579e9b754e398dc1e5c338fcd8cf32a8` |
| `mixed-scale-topology.log` | 483 | `cf403b9a65bee78b8b4f2f744d1241aa74c011d73ffcf0ed8cb08793f35b32ac` |
| `phase-2-2-controlled-capture-aarch64-apple-darwin.toml` | 5,038 | `4edcf09b377438398a1bfb147500965aaf4c758ca4a17484b6fc552fc13c13d8` |
| `phase-2-2-controlled-transitions-aarch64-apple-darwin.toml` | 3,914 | `3458912c2449200eb942ebb3acf6d801e25226e7579417b0675841b3e246c37f` |
| `phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml` | 4,932 | `127e5dc2b7ed7385ba8cfa5bb1cf30597f86052370869fd42123b25fd8c21ba6` |
| `phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml` | 4,056 | `c035344e329528dc90b2b954c2a39bbcfa1eefcf287eca0819286b9851ce95d8` |
| `phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml` | 4,939 | `be0b6cad2fbba7b0026f1b35a40c240628714ee0289e6fa9cb6bf00311dfcaac` |
| `retained-authority.log` | 10,444 | `6845b7c067cdfa8b0b4235496e58e18ea0094c9c568da297b8f99a12f6c4fdf3` |
| `retained-filter-replacement.log` | 868 | `66fa9f056c14ac367c9f0eb7d072e883e512f1068f49b9ef30b1473d975517ed` |
| `same-scale-display-conversion.log` | 1,617 | `14139e72bba813348afd077055d2eeb1b0afe817b2ef135e70f27f479356b961` |
| `same-scale-display-seam.log` | 1,722 | `75bb6a0e06768400ecd33618c118ce143ac5e3059be05dfa4413ef7d4fc32afe` |
| `same-scale-placement.log` | 104 | `052fe9537bce1208d19984beac6728e0c5ecbeb26adef5f89b9dbe07792ca3ec` |
| `same-scale-renderer-matrix.log` | 383,826 | `f7bd308ca58e05028dd9af3c32c6ceb0cf22cb7601fd554c84209a5e3d7847a0` |
| `same-scale-topology.log` | 326 | `10eb225a74c5e7633a72dc1586f1fc7d7b630f2df9bac16ef5aa197038b90d20` |
| `single-display-conversion.log` | 1,691 | `2a3c4e5c9c126cd73d3293bff987f4f90e9c5037dad8d59e29f8f417b342dfd4` |
| `single-placement.log` | 49 | `04d6263aab3b2498ff0ae7386405b9c683d5804ba24f06b344007f5348f26166` |
| `single-renderer-matrix.log` | 280,050 | `5798cf18e0ff370c48abc6d91b62237bd92d63e6ab1f0e081b879413c72cd793` |
| `single-topology.log` | 168 | `4916452bca1bf573ddd26900090d2ba3a7a4f88845617fea595b3c7fa9c590dd` |
| `sustained-capture-soak.log` | 8,500 | `a2197e604fb75eb8591f1e0c3cac045d76fb4aa02afc8286a19c4ed0ae99b241` |
| `system-route.log` | 716 | `301bc41fbb3f12134d8ccefc01ad500ce206272f94ca313dae4c3b2556a19d53` |
| `unrelated-activity.log` | 12,236 | `59b1c45525ba507f7e8f50bd8cef6ddb217b2265f3253739ddc55ec7c6e53dbe` |

## Privacy review

This tracked report contains no captured pixels, recognized or submitted text, window titles, application names, native process identifiers, native window numbers, signing identifiers, raw authorization values, credentials, fixture-private payloads, unrelated foreground identity, or process inventory. Display geometry, internal frame stamps, bounded event/sample counts, typed outcomes, artifact sizes, and hashes are retained because they are required qualification facts.

## Strict release gate

Every route-wide row and every mandatory row for each of the fourteen exact pairs passed on the qualified source revision. The single-display, same-scale, and mixed-scale results are independently recorded. Therefore ADR 0029 may resolve to Accepted for these controlled pair contracts, and public descriptors and documentation may advertise them with `Unknown` compatibility, `OwningProcess` address scope, `InvocationOnly` evidence, foreground-preserving behavior, and explicit caller opt-in.

No support statement may broaden this result to exact-window delivery, arbitrary applications, arbitrary games, display targets, minimized/off-screen targets, application consumption, or visual success. Any later product, fixture, qualification-oracle, budget, or mandatory-documentation change invalidates the affected acceptance rows and requires a new revision-bound report.
