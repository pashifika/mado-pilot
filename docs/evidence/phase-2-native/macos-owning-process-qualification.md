# Phase 2.2 macOS owning-process observed report

## Current decision

The corrected implementation at source commit `850b7b26dde49035dd5759685ab6f0c7d996167f` and source tree `3562f60ccaac62a778ffaf5af7ff26af19feed53` is **not yet release-qualified**.

The route-wide rows, the complete mixed-scale AppKit and controlled OpenGL renderer rows, the five Phase 2.2 performance profiles, AddressSanitizer, repository verification, and pull request CI pass on that revision. The exact `single` and `same-scale` renderer rows have not been rerun after the product correction. Because the frozen plan requires every topology for each dependent pair, the current decision is 0 `qualified`, 0 `rejected`, and 14 `unexecuted`. Draft pull request #36 remains draft.

The earlier complete matrix at `8309a05c3e7696f3081c5afef6dd6979ea1bb084` is retained below as historical evidence only. The later focus-policy and scroll-location product correction invalidated its release decision; none of its pair passes may be applied to the current source.

## Current corrected-revision provenance

The current partial rerun was recorded on 2026-08-14 JST. Product, fixture, test, and benchmark source remained fixed at the source commit above. These tracked profiles and report updates were produced afterward and do not change the measured implementation tree.

| Field | Observed value |
|---|---|
| Base revision | `ffb1823b68ba632b4fc8e7725361ea4596e220f0` |
| Corrected source commit | `850b7b26dde49035dd5759685ab6f0c7d996167f` |
| Corrected source tree | `3562f60ccaac62a778ffaf5af7ff26af19feed53` |
| Branch / pull request | `feat/phase-2-2-macos-process-directed-delivery`; draft PR #36 against `dev/0.2.1` |
| Host | `MacBookPro18,3`; Apple M1 Pro, 10 physical cores, 32 GiB, `arm64` |
| OS / SDK | macOS 26.5.2 build 25F84; macOS SDK 26.5 |
| Minimum deployment target | macOS 26.5.2 |
| Apple compiler | Apple clang 21.0.0 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`; `cargo 1.97.1` |
| Fixture protocol | version 10; one outstanding command; 512-byte command/result bound; 1,024-byte ready-record bound; 1,024-event recorder bound; owned-child teardown |
| Fixture source set | SHA-256 `51c6f991a942d30440f18a8b06e105ebb3bc15511a9e909ef696b300ba8d4c7b` |
| Fixture launch/signing | bundled launch; structurally valid ad-hoc signature; approved identifier matched |
| Authorization | Screen Recording granted; non-prompting event-post access granted; required observations agreed |
| Prompting behavior | no permission request, settings opener, target activation, or target raise |
| Current topology | three online, non-mirrored displays: 1× at signed origin, 2× main, and 2× secondary; both mixed-scale and same-scale seams are present |

Current executable hashes:

| Artifact role | Bytes | SHA-256 |
|---|---:|---|
| Native input integration test executable | 4,541,936 | `0eb323915418e59de616720e24a07830be919d569898d04c0800dcf94283c491` |
| macOS unit/scenario test executable | 4,863,416 | `cfeae7b635fab9463e584dfd69b748cde7080f25fb876cc9da94191e6fd55e65` |
| native-phase2 benchmark executable | 1,378,032 | `e92914d96bdc7cf5004243846bc3b1de4b0f31403c60107eddd6625d5c939fcd` |
| Release fixture executable | 599,936 | `34277ec97b134ddedff38930c72f9b9588b762c2828f02f05e761f40d8ae78d5` |
| C common-flow executable | 34,880 | `c47e516890e5aa69130ef988e7d3e04e1cb941c413d6f820a27df02f1c10f6aa` |
| C++ common-flow executable | 327,960 | `481d470428c7bbff00a8125920f5f9bd3f79187a53c47e33f66f1a3e011a754f` |

## Current command manifest

The retained current-revision evidence was produced by the following command groups. `$APP`, `$C`, and `$CPP` denote repository-built artifacts; absolute workstation paths and private fixture records remain only in ignored raw output.

```sh
cargo build --locked --release -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
/usr/bin/codesign --verify --strict --verbose=2 "$APP"

cargo test --locked -p mado-pilot-platform-macos \
  --features private-fixture -- --nocapture

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

# The eight other ignored route, isolation, soak, lifecycle, replacement,
# fixture-control, and explicit System checks were run one at a time with
# the same package/features/test target and exact single-threaded options.

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set <capture|transitions|process-directed|process-directed-game-like|process-diagnostics> \
  --fixture-executable "$APP" --source-revision 850b7b26dde49035dd5759685ab6f0c7d996167f \
  --source-tree 3562f60ccaac62a778ffaf5af7ff26af19feed53 <recorded host options>

cargo run --locked --package mado-pilot-dependency-check
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo deny --locked check
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
cargo clippy --locked --target x86_64-pc-windows-msvc \
  -p mado-pilot-platform-macos --all-targets -- -D warnings
MADO_PILOT_MACOS_ASAN=1 cargo test --locked \
  -p mado-pilot-platform-macos --target-dir target/asan --lib -- \
  --nocapture --test-threads=1
cargo test --locked -p mado-pilot-testkit \
  --test benchmark_block_drift --test hard_budget_drift
```

## Current topology and native outcomes

| Selector / row group | Current corrected-revision outcome |
|---|---|
| `mixed-scale` display-frame rows | passed: three displays published their own logical/backing geometry; the signed-origin 1×/2× seam and adjacent 2×/2× seam were exercised |
| `mixed-scale` AppKit renderer | passed: four visits closed the 2× → 2× → 1× → 2× cycle at geometry revisions 3/4/5/6 and epochs 2/2/3/4 |
| `mixed-scale` controlled OpenGL renderer | passed: the same four-visit scale/geometry cycle completed independently |
| `single` renderer rows | **unexecuted**: the corrected source has not been run with exactly one 2× display online |
| `same-scale` renderer rows | **unexecuted**: the corrected source has not been run with exactly two horizontally adjacent 2× displays online |

Each completed mixed-scale renderer test executed its fixed matrix of five pointer coordinate spaces, seven pointer sequences over eleven authoritative geometry stages, five keyboard cases, three Unicode text cases, cancellation/cleanup, caller-selected `RequireFocused`, foreground/cursor invariance, independent process observation, and strictly newer controlled visual confirmation. AppKit frame sequences for the four visits were 1,632 / 4,521 / 1,296 / 1,640; controlled OpenGL sequences were 1,644 / 4,483 / 1,260 / 1,599. These values identify capture progress only; they do not claim input-caused visual effect.

The default private-fixture suite recorded 250 passes and ten deliberately ignored interactive rows; all ten ignored rows were executed separately on the corrected revision. The complete workspace recorded 1,213 passes and eleven ignored opt-in tests. Doctests recorded eight passes and one ignored example. The dedicated native-shim AddressSanitizer run recorded 185 passes with no sanitizer finding. Draft PR #36 then passed repository policy, branch flow, Windows, and macOS CI jobs.

### Route-wide decisions

| Row | Decision | Current observed basis |
|---|---|
| RW-01 | passed | controlled symbol loading, availability, version/layout, package isolation, production linkage, C ABI, and C++ checks passed |
| RW-02 | passed | retained logical window, original process lifetime, current ownership, and eligible state were revalidated before ordinary posts |
| RW-03 | passed | replacement, close/relaunch, stale identity, and retained-filter refusal rows passed |
| RW-04 | passed | renderer admission dwell and the bounded sustained-capture soak completed |
| RW-05 | passed | primary-only and additional same-process window states remained process-scoped |
| RW-06 | passed | unrelated foreground preservation and no target activation/raise passed |
| RW-07 | passed | non-prompting grant, denial, revocation, unavailable-symbol, and disagreement behavior failed closed |
| RW-08 | passed | receipts remained `ProcessDirected`, `OwningProcess`, and `InvocationOnly` with exact submitted prefixes |
| RW-09 | passed | no implicit fallback; explicit fallback closed after possible native effect |
| RW-10 | passed | deadline and cancellation arbitration prevented later ordinary posts or result mutation |
| RW-11 | passed | sequence state, newest-first cleanup, close/drain, and repeated close remained bounded |
| RW-12 | passed | owned-child identity, nonce, protocol bounds, replacement, and teardown checks passed |
| RW-13 | passed | diagnostics modes and overflow remained bounded and privacy-safe with exact loss accounting |
| RW-14 | passed | pointer/range validation, exception/panic containment, queue pressure, allocation gates, and 185 ASan tests passed |

No route-wide row failed. Missing topology rows still block every final pair decision.

### Current pair decisions

| Operation | Target class | Coordinate space | Decision | Reason |
|---|---|---|---|---|
| `Pointer` | AppKit renderer | `CapturePixels` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `FrameNormalized` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `TargetNormalized` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `TargetLogical` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `DesktopLogical` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Keyboard` | AppKit renderer | `not applicable` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Text` | AppKit renderer | `not applicable` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `CapturePixels` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `FrameNormalized` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `TargetNormalized` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `TargetLogical` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `DesktopLogical` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Keyboard` | OpenGL game-like renderer | `not applicable` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |
| `Text` | OpenGL game-like renderer | `not applicable` | `unexecuted` | The mixed-scale row passed, but exact `single` and `same-scale` renderer rows have not run on the corrected source; another topology cannot substitute. |

Pair totals: 0 `qualified`, 0 `rejected`, 14 `unexecuted`. The implementation remains reviewable on the draft branch, but no current-source release support claim follows from this partial matrix.

## Current performance outcomes

The five Phase 2.2 profiles were remeasured on the corrected source. All 2,700 retained samples across twenty-four workloads satisfied their correctness oracle and frozen latency, hard-maximum, mapped-byte, stale-work, heap, allocation-growth, and diagnostics-capacity gates. `result_correctness` was zero for every workload. The AppKit discovery workload recorded 2,624 bytes of post-warmup allocation growth, below the frozen 4,096-byte hard limit; every other workload recorded zero growth.

| Profile / workload | p50 ms | p95 ms | max ms | Peak live Rust bytes |
|---|---:|---:|---:|---:|
| capture / `fixture_command_acknowledgement` | 0.180208 | 0.228917 | 1.581459 | 36,933 |
| capture / `controlled_stimulus_to_frame` | 16.519417 | 18.612042 | 23.689584 | 9,301,070 |
| capture / `static_latest_retained` | 0.000125 | 0.000167 | 0.000167 | 4,672,126 |
| capture / `static_newer_repeated_pixels` | 17.049084 | 18.326166 | 25.142125 | 9,301,070 |
| capture / `latest_acquisition` | 0.000750 | 0.001917 | 0.059750 | 9,301,070 |
| capture / `cpu_map_bgra8` | 0.191917 | 0.473375 | 2.101416 | 9,301,070 |
| transitions / `resize_recreation` | 51.805417 | 53.924083 | 54.518834 | 8,981 |
| transitions / `open_first_frame` | 104.268833 | 114.179083 | 116.080958 | 5,335,943 |
| transitions / `retained_pressure_resume` | 3.858083 | 17.724750 | 18.894625 | 4,672,597 |
| transitions / `close_drain` | 59.643875 | 71.861083 | 75.761333 | 7,911 |
| AppKit / `discovery_open_retained_authority` | 330.969333 | 410.645500 | 417.327750 | 58,659 |
| AppKit / `event_authority_preflight_post` | 227.123417 | 238.558125 | 239.806750 | 4,674,014 |
| AppKit / `release_cleanup` | 1.603792 | 2.790292 | 2.874916 | 4,674,342 |
| AppKit / `session_close` | 65.687625 | 73.898000 | 75.259667 | 44,271 |
| AppKit / `fixture_controller_close` | 53.358250 | 64.509666 | 79.935875 | 37,401 |
| game-like / `discovery_open_retained_authority` | 333.255500 | 428.402416 | 440.226083 | 53,403 |
| game-like / `event_authority_preflight_post` | 226.660875 | 231.926375 | 237.528000 | 4,674,014 |
| game-like / `release_cleanup` | 1.691041 | 2.729875 | 2.860459 | 4,674,038 |
| game-like / `session_close` | 63.423750 | 74.152250 | 82.509500 | 44,271 |
| game-like / `fixture_controller_close` | 20.715333 | 33.702750 | 46.463416 | 37,413 |
| diagnostics / `event_diagnostics_off` | 225.912417 | 229.972625 | 232.025958 | 4,674,438 |
| diagnostics / `event_diagnostics_normal` | 225.600333 | 230.721292 | 236.434584 | 4,684,542 |
| diagnostics / `event_diagnostics_debug` | 226.301250 | 230.556625 | 234.786958 | 4,684,846 |
| diagnostics / `event_diagnostic_overflow` | 227.567709 | 231.767750 | 235.045125 | 4,674,966 |

The controlled capture profile retained the exact 4,628,480-byte BGRA8 mapping bound and a zero stale-work ratio for acquisition workloads. The transition profile recorded its measured retained-pressure ratio below the frozen 0.95 ceiling. Both process-directed renderer profiles remained below the frozen 16 MiB live-Rust-heap ceiling. These are controlled-fixture regression measurements, not real-time guarantees or general game compatibility evidence.

The separately versioned ADR 0025 native-input/public-language profile remains bound to the historical `8309a05` source. One corrected-source refresh attempt stopped before the first retained sample because the fixture controller found non-empty pending event state at reset. No measurement from that attempt was accepted, and the historical input profile is not represented as current-source evidence.

Current committed profile hashes:

| Profile | SHA-256 |
|---|---|
| `phase-2-2-controlled-capture-aarch64-apple-darwin.toml` | `ed51d9e3fbbfdb23ef79ee229209a8704de2f4457ac459f69d4334f3610b08c0` |
| `phase-2-2-controlled-transitions-aarch64-apple-darwin.toml` | `c517e805ab3dfe2f945e2753f65998cf3e15e5272ed0a3c2853d373d46619d9b` |
| `phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml` | `0aecdb9bb8413a69b50f0921a3a97a85a209c8fd58769963e432dd5d67192ee8` |
| `phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml` | `22dcfcc6e3aada8232bdf2d2a8fa9ecfa570c74970831c0c121db0d87d350040` |
| `phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml` | `9a5cd686e6826b1c681b4a485a7cbfd3b09ddb3c497303c0f10f7c45bb12c2a7` |

## Current raw-output provenance

Accepted current-revision raw logs remain ignored under `.rasen/changes/phase-2-2-macos-owning-process-delivery/ephemera/qualification-final-850b7b2/`. They may contain native identifiers or fixture-private records and must not be committed. The tracked binding is limited to file size and SHA-256.

| Accepted raw artifact | Bytes | SHA-256 |
|---|---:|---|
| `benchmark-profile-gates.log` | 1,342 | `8c0888f19c92bd633c5ab8e80543b76afb30a6fca251a80135c385d52b2e3ea4` |
| `c-abi-check.log` | 24,780 | `2b8bd9e5c295b37142cf3675e23f958e383e07c23054628c3480f31b83c7779c` |
| `capture.log` | 5,385 | `6644765964ec3630040c788b0ce59cdc0674ff21eec8b1e386f7a9a553394b9d` |
| `cargo-clippy-macos-cross-target.log` | 203 | `88985a441e66489ff4ca622ef121571c7dd446f42f48d4c6c2c0ff02e870ea9f` |
| `cargo-clippy.log` | 436 | `6d2b772f2a926d0a1d03b0555264153c275a8821a4dcd1dd5c471aeb092b792d` |
| `cargo-deny.log` | 2,057 | `c52db9d8b7907af482ad0c236a578a66ba23dbb2807fc447ce3b29f1d683f96c` |
| `cargo-doc.log` | 804 | `665f8940da11cc67583f6daea9137473587dda17a2f17ffc1af84d9f43df2694` |
| `cargo-fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `cargo-test-doc.log` | 3,467 | `250b1899a9ac7ab53da90c283a81d5907d6556967d0bead054e3d0f656d506df` |
| `cargo-test-macos-asan.log` | 17,585 | `b9737d9c684bfe43597b7269c3639cbc2ae0fecab63fa0b2d5352217ac5c1fae` |
| `cargo-test-private-fixture.log` | 26,692 | `b3dc1746009e94012b2466a9c1ab0301c3ab95c00a12c972fc4352d8854e2b0f` |
| `cargo-test-workspace.log` | 112,531 | `7dc52a7af9671bc2210583998f384372e52bad987cd4bb8fbfe88234714e1103` |
| `cargo.log` | 36 | `1aedaa9cce116919d39b1eb303a58865771582bc24fc259aa8e26cb212e0f59e` |
| `cc.log` | 159 | `76a924f59997fb65051981eb13105e0a8c151f16cd9c3e45e0f7c27ecefdb839` |
| `ci-pr36.log` | 720 | `16fbd352c8b5f832df1b211e8734e43a8319b55ab64d58432a903e5e33915bc7` |
| `dependency-check.log` | 3,276 | `8b7a14ea0f1e9316a1140b7d3cbbb48d27c37fe5f9ba980c7f8bb49877650c8b` |
| `displays.log` | 483 | `cf403b9a65bee78b8b4f2f744d1241aa74c011d73ffcf0ed8cb08793f35b32ac` |
| `fixture-build-release.log` | 193 | `9683b8ee0ed558ad0a07521134821adc1716de0e0c49e04a0aa0c77e0b664336` |
| `fixture-control.log` | 1,401 | `7f87d299a7f6f2eb8d7e5a5b830d391f4382774f915d93fe6f095b58f456a36a` |
| `fixture-release-verification.log` | 102 | `1d287c10fba4ec2962f9778ea542c78aa084ca1d1ae52d95e6274a30b798b9e7` |
| `hardware.log` | 125 | `faa5bb19ecee5335724e399a8ad08c6903823d9aa020dcb52a1a4d35ffd63dc9` |
| `host-status.log` | 80 | `11712fda5b3f02979c4d7bed188e46545cc0204a2174d0452653e23369f30ad9` |
| `mixed-appkit-renderer-matrix.log` | 95,470 | `4bf6e43627e064d7cfd9f06620a052cdc92a73633f81730e0c75de993a69d075` |
| `mixed-display-conversion.log` | 1,617 | `e468145a5706da8fd51a1b80f4dbf5d46830b489301e3e05a7f3602d44cd4be5` |
| `mixed-display-scale.log` | 1,610 | `7abcdc3157215b6e4f67b93328d1c5f561deb8d3307e1b60b2aed391cbc78f93` |
| `mixed-display-seam.log` | 1,659 | `5e7ff1d2a5b843531b916b129e7c86208aa9f6694ebddfee2be49a35f0999b83` |
| `mixed-game-like-renderer-matrix.log` | 95,284 | `63c1f70e58115fc0ec810f79af5378971091a071045806da7a4c9f16b0ecf8d4` |
| `mixed-host-baseline.log` | 435 | `cd4c7bdf7a0d1515f70f612d3713ab0b361e5665806e3ff1f94f0fa1e7358bb7` |
| `offscreen-cleanup.log` | 2,496 | `7a3b2ba7bbe0e33a88f9d1be55b952b1f6b2aee2477fee40c15b1ca8012cf47a` |
| `os.log` | 65 | `9a27ea4f1e11f7b9a4f7573d8729db037a10ce2e457f97cb73f646a96a550b16` |
| `process-diagnostics.log` | 4,107 | `6b046d287ae9ac943642ea63384b3934e42fa6261b4385118247606bf0c77ec1` |
| `process-directed-game-like.log` | 5,092 | `096e934f7ed2396c3b37d65475f2e252ff01234686831ac8416378713ec61e65` |
| `process-directed.log` | 5,088 | `0b843190f884a984c8c5ae3bc81369000293713a64482b62147861420dde1a8b` |
| `retained-authority-lifecycle.log` | 3,284 | `30d408a318adffa729853b9c39105ecb6d44721bfcca99b94c4fbda6eea7a713` |
| `retained-filter-replacement.log` | 620 | `3ffa068c380eb254de5e996f8ed9492548f827b71d8d120da40f00168bcdde0d` |
| `rustc.log` | 192 | `e794146d99fad54562fdcbd4a7c8b22b0240d2b0312ff6d879d67939b3131850` |
| `sdk.log` | 5 | `25205f2e2f02dc71036ee827e19c49b893b231a3d1af240e35a3ac55aa8cdcb6` |
| `source-commit.log` | 41 | `4fd171b4d855851449e489e8dd9d539a498c06709e9532bc7a5280a87294ccea` |
| `source-tree.log` | 41 | `caf230cec40e48be5ea10ba719f044d1c3991c3b80223df27ca3b1d5b1bb205c` |
| `sustained-capture-soak.log` | 2,155 | `d820ae6cc4f48fea74c9782454edda9ae97017eeb537cd4bc37a6f51824364d1` |
| `system-route.log` | 501 | `ef5431ba62a0bbf3b3abafedd3b4582604b4118438fc646dbc0d92148cdffd3a` |
| `transitions.log` | 4,018 | `04b4b3a38aad1515b348afd462cd1b20f426c8a4f8651185d7688872d22cb15c` |
| `unrelated-appkit.log` | 2,355 | `dbd3a485699d33aa491ef53a76948053d00cb49ef435139ee16a5722313db9be` |
| `unrelated-game-like.log` | 2,339 | `f4c08a8b894e3d0e637592de76e22c3da6ff570280286b1abc1d7653f22f70e2` |

Stopped attempt:

| Excluded raw artifact | Bytes | SHA-256 | Reason |
|---|---:|---|---|
| `input-stopped-unexpected-event-attempt.log` | 454 | `c02cf37383d933da25519b69f301c4481c791b9928aa270c733af23d2d5e4093` | stopped before measurement because reset found non-empty pending fixture event state; contributes no performance or pair decision |

## Current privacy review

This current tracked section contains no captured pixels, recognized or submitted text, window titles, application names, native process identifiers, native window numbers, signing identifiers, raw authorization values, credentials, fixture-private payloads, unrelated foreground identity, or process inventory. Display geometry, frame stamps, bounded event/sample counts, typed outcomes, artifact sizes, and hashes are retained because they are required qualification facts.

## Current strict release gate

The gate is **blocked**, not passed: exact corrected-revision `single` and `same-scale` renderer rows are unexecuted. ADR 0029 remains the accepted owning-process design, but its release publication evidence is suspended for the current source. Public release documentation must not claim any of the fourteen pairs as currently qualified until both topology rows pass and the report is rebound again. Draft PR #36 must remain draft.

## Superseded complete-run record

The remainder of this document is the complete historical record for source `8309a05c3e7696f3081c5afef6dd6979ea1bb084` and tree `27fe879e0c4bb55fe4850d9a50737b568936cc10`. It was valid before the later product correction. Its pair decisions, profile hashes, raw manifest, and strict-gate conclusion are retained for provenance only and do not describe current source.

### Decision

The Phase 2.2 native qualification matrix passes on source commit `8309a05c3e7696f3081c5afef6dd6979ea1bb084` and source tree `27fe879e0c4bb55fe4850d9a50737b568936cc10`.

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

### Qualified provenance

The final source-bound native rows were recorded on 2026-08-16 JST. Product, fixture, and qualification source remained unchanged during all accepted rows. Benchmark profiles and this report were produced afterward as evidence artifacts and do not alter the qualified implementation tree.

| Field | Observed value |
|---|---|
| Base revision | `ffb1823b68ba632b4fc8e7725361ea4596e220f0` |
| Qualified source commit | `8309a05c3e7696f3081c5afef6dd6979ea1bb084` |
| Qualified source tree | `27fe879e0c4bb55fe4850d9a50737b568936cc10` |
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
| native-phase2 benchmark executable | 1,377,840 | `fef853d80bee20276a3dfa7037ff4a2338a47510f1514bd5cfabd7a177df4056` |
| Release fixture executable | 599,936 | `d355de10628df7f0c938e4dc72df22b3dc3666997d9e638865b8d624cd6c8ece` |
| C common-flow executable | 34,880 | `c47e516890e5aa69130ef988e7d3e04e1cb941c413d6f820a27df02f1c10f6aa` |
| C++ common-flow executable | 327,960 | `481d470428c7bbff00a8125920f5f9bd3f79187a53c47e33f66f1a3e011a754f` |

### Executed command manifest

`$APP` denotes the repository-generated target fixture bundle and `$FOREGROUND_APP` a separately launched role of the same qualified executable. Absolute workstation paths are omitted. Each topology block ran only after the live arrangement matched its exact selector.

```sh
cargo build --locked -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
# Assemble $APP, copy the debug executable, and sign it ad hoc with the
# approved identifier.
/usr/bin/codesign --verify --strict --verbose=2 "$APP"
"$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --report-execution-context

cargo test --locked -p mado-pilot-platform-macos \
  --features private-fixture -- --nocapture
cargo run --locked -p mado-pilot-capi --example c-abi-check -- \
  --label "macOS Apple Silicon 8309a05"

export MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture"

# The standing mixed-scale topology ran first.
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

# Restore the recorded 2×/2×/1× signed-origin arrangement and rerun all
# public display-frame scenarios.
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

cargo build --locked --release -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
# Reassemble and verify the release $APP before benchmarking.
/usr/bin/codesign --verify --strict --verbose=2 "$APP"

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set <capture|transitions|process-directed|process-directed-game-like|process-diagnostics> \
  --fixture-executable "$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --source-revision 8309a05c3e7696f3081c5afef6dd6979ea1bb084 \
  --source-tree 27fe879e0c4bb55fe4850d9a50737b568936cc10 \
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
  --source-revision 8309a05c3e7696f3081c5afef6dd6979ea1bb084 --source-tree 27fe879e0c4bb55fe4850d9a50737b568936cc10 \
  <the same recorded host, toolchain, topology, and permission options>

cargo run --locked --package mado-pilot-dependency-check
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo deny --locked check
cargo clippy --locked --target x86_64-pc-windows-msvc \
  -p mado-pilot-platform-macos --all-targets -- -D warnings
MADO_PILOT_MACOS_ASAN=1 cargo test --locked \
  -p mado-pilot-platform-macos --target-dir target/asan --lib -- \
  --test-threads=1
cargo test --locked -p mado-pilot-testkit \
  --test benchmark_block_drift --test hard_budget_drift
rasen validate phase-2-2-macos-owning-process-delivery --strict --json
```

### Display topology outcomes

All topologies were non-mirrored. Display-frame and renderer rows ran separately for each exact selector; one topology was never substituted for another.

| Selector | Live public geometry | Renderer window frame summary | Outcome |
|---|---|---|---|
| `single` | one display: 1512×982 logical / 3024×1964 backing at `(0,0)`, 2× | AppKit: sequence 1080, geometry 3; OpenGL: sequence 1095, geometry 3; both 688×484 logical / 1376×968 backing at 2× | passed |
| `same-scale` | 2560×1440 logical / 5120×2880 backing at `(0,0)` and 1512×982 logical / 3024×1964 backing at `(2560,170)`; both 2× and horizontally adjacent | AppKit: sequence 1170, geometry 3 → sequence 3447, geometry 4; OpenGL: sequence 1131, geometry 3 → sequence 3346, geometry 4 | passed after one zero-effect deadline retry |
| `mixed-scale` | 3840×2160 logical/backing at signed origin `(-3840,109)`, 1×; 2560×1440 logical / 5120×2880 backing at `(0,0)`, 2×; 1512×982 logical / 3024×1964 backing at `(2560,268)`, 2× | AppKit visits: sequence 1110/3353/1123/1142, geometry 3/4/5/6, epoch 2/2/3/4; OpenGL visits: sequence 1135/3380/1125/1118 with the same geometry/epoch progression; both closed the 2×→2×→1×→2× cycle | passed |

The three-display mixed-scale arrangement was restored after qualification. Final display-frame scenarios passed in that restored arrangement, and no fixture process remained after cleanup.

### Bounded native row outcomes

The default `private-fixture` macOS suite recorded 249 passing tests and ten interactive tests ignored by default. All ten ignored tests were then executed explicitly. Each renderer matrix completed under each mandatory topology selector on the qualified source. The complete workspace recorded 1,212 passing tests and eleven ignored opt-in tests; doctests recorded eight passes and one ignored example; the dedicated ASAN macOS library run recorded 184 passes.

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

### Route-wide decisions

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
| RW-14 | passed | Native pointer/size/range validation, Objective-C exception and Rust panic containment, queue pressure, diagnostic overflow, allocation bounds, and the 184-test ASAN run passed. |

No route-wide row is failed or unexecuted in the accepted matrix.

### Pair decisions

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

### Performance outcomes

All 2,700 retained Phase 2.2 benchmark samples across twenty-four workloads satisfied their workload correctness oracle. Every refreshed profile passed its frozen p50, p95, hard-maximum, mapped-byte, stale-work, peak-heap, allocation-growth, and diagnostic-capacity gates. `result_correctness` was zero for every workload, and post-warmup allocation growth was zero throughout this run.

| Profile / workload | p50 ms | p95 ms | max ms | Peak live Rust bytes |
|---|---:|---:|---:|---:|
| capture / `fixture_command_acknowledgement` | 0.174834 | 0.247125 | 1.517500 | 36,933 |
| capture / `controlled_stimulus_to_frame` | 16.675708 | 17.456125 | 18.408250 | 9,300,417 |
| capture / `static_latest_retained` | 0.000125 | 0.000167 | 0.000250 | 4,671,473 |
| capture / `static_newer_repeated_pixels` | 17.681583 | 36.310166 | 70.723583 | 9,300,417 |
| capture / `latest_acquisition` | 0.000500 | 0.000709 | 0.003250 | 9,300,417 |
| capture / `cpu_map_bgra8` | 0.185208 | 0.200333 | 0.242125 | 9,300,417 |
| transitions / `resize_recreation` | 51.864000 | 65.297417 | 68.836708 | 8,328 |
| transitions / `open_first_frame` | 103.177458 | 111.884583 | 116.651958 | 5,335,290 |
| transitions / `retained_pressure_resume` | 3.372792 | 17.096834 | 17.683000 | 4,671,943 |
| transitions / `close_drain` | 55.533500 | 68.959166 | 70.590334 | 7,258 |
| AppKit / `discovery_open_retained_authority` | 338.025417 | 363.706833 | 402.802417 | 51,187 |
| AppKit / `event_authority_preflight_post` | 223.885334 | 228.749375 | 232.166125 | 4,673,360 |
| AppKit / `release_cleanup` | 1.670959 | 2.538083 | 2.833375 | 4,673,384 |
| AppKit / `session_close` | 61.321916 | 69.602833 | 71.502958 | 43,617 |
| AppKit / `fixture_controller_close` | 53.454667 | 64.851708 | 69.106458 | 37,398 |
| game-like / `discovery_open_retained_authority` | 336.899958 | 390.206208 | 399.066417 | 51,187 |
| game-like / `event_authority_preflight_post` | 224.161417 | 230.348167 | 233.705125 | 4,673,360 |
| game-like / `release_cleanup` | 1.696125 | 2.670750 | 2.861458 | 4,673,384 |
| game-like / `session_close` | 60.453625 | 70.455542 | 78.822292 | 43,617 |
| game-like / `fixture_controller_close` | 20.588917 | 23.784250 | 34.664667 | 37,410 |
| diagnostics / `event_diagnostics_off` | 224.030250 | 229.019834 | 297.820208 | 4,673,480 |
| diagnostics / `event_diagnostics_normal` | 223.253667 | 228.602625 | 236.941833 | 4,683,888 |
| diagnostics / `event_diagnostics_debug` | 223.155583 | 227.883541 | 230.188000 | 4,683,888 |
| diagnostics / `event_diagnostic_overflow` | 225.369959 | 229.631625 | 241.537792 | 4,674,312 |

The controlled capture profile retained the exact 4,628,480-byte BGRA8 mapping bound. Its acquisition workloads recorded a `0.000000000` stale-work ratio. The controlled transition profile recorded a `0.834983498` retained-pressure stale-work ratio and passed its frozen `0.95` ceiling. Process-directed profiles stayed below the frozen 16 MiB peak-live-heap limit.

The separately accepted ADR 0025 native-input/public-language profile was also refreshed on the same source with 300 retained samples. Its Rust common flow passed; fresh C and C++ process-load checks passed; and the explicit C and C++ `ProcessDirected` common flows completed with p95 values of 1842.148958 ms and 2056.563250 ms respectively. Receipt evidence and owned-fixture event observation remained separate in both language consumers. Its process-load resident-memory ceilings and all shared latency, mapping, heap, and correctness budgets passed.

Committed profile hashes:

| Profile | SHA-256 |
|---|---|
| `phase-2-2-controlled-capture-aarch64-apple-darwin.toml` | `f59f42fb003bb9c145a8bdf3006cd0c71cb9ea0d6dc5a502cb4341e37cbaae4b` |
| `phase-2-2-controlled-transitions-aarch64-apple-darwin.toml` | `c58ac65fd08bcb7f8d8df864cf0baf185259cebf71c8c9c0dbb07a9a8b056dd7` |
| `phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml` | `d98201b5cafc5dc6cff6c3518280c0ab94e4d9864806ff9eaee617bdde64cad0` |
| `phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml` | `ac4e9cb130f15cba5dbaaa04cdebd8498335d5d2efb8db6b79c6f820c1092705` |
| `phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml` | `e5863b5c93a8242de6d754cd1f008ded675fb63bd8611e025a2346c38d0d67d6` |
| `phase-2-native-input-aarch64-apple-darwin.toml` | `248fa715f7ac3a3b3c50fcf6ddfdf2718000c4fe2ac8f7028370b5f74daf267b` |

### Failed, excluded, and superseded attempts

The accepted matrix has no failed or unexecuted pair row. Complete matrices on source commits `8dd70810d60c06b298c806ffce16720d0a07e4c2`, `b1059cf6239042107bd62373eb65211117beaab9`, and `a4b12ffb89e0ef5e70ddf229a258c74dbe74a9dd` were invalidated by later product, fixture, qualification, evidence, or benchmark-oracle changes and contribute no final decision. The final rerun was required even though the last source change only replaced an equivalent benchmark mapping `let...else` with `?`; the frozen plan binds qualification to an exact revision.

The first same-scale AppKit renderer attempt on the final source reached its five-second operation deadline after 208 seconds. Its receipt was `Unexecuted` with zero submitted events, so it had no possible native effect and contributes to no pair result. The raw output was retained, the owned fixtures were confirmed stopped, a ten-second idle interval elapsed, and the same isolated full renderer matrix then passed. No failed output was overwritten or promoted.

Excluded attempt hashes:

| Excluded raw artifact | Bytes | SHA-256 |
|---|---:|---|
| `same-scale-appkit-renderer-matrix-deadline-attempt.log` | 59,536 | `ccc945bf91b71c0ff7291bf00fccb4e8acd935dcf689a8827354b2fa01f4bc4d` |

### Raw-output provenance

Accepted raw logs remain ignored under `.rasen/changes/phase-2-2-macos-owning-process-delivery/ephemera/qualification-final-8309a05/`. They contain native identifiers and fixture-private records and must not be committed. The tracked binding is limited to file size and SHA-256.

| Accepted raw artifact | Bytes | SHA-256 |
|---|---:|---|
| `source-commit.log` | 66 | `c763b5ed9c862aac3bd4caad058445f861e53dada3964c821e7ec0d106057700` |
| `source-tree.log` | 66 | `7348f51c292d804dee8663fb75cc102cbcaedd75d37ba9fd3e8d249f5e4d4141` |
| `host-status.log` | 105 | `075cce1c47922989e2518568b95912fd93aa55f13b0709800db1716ef1b85b69` |
| `rustc.log` | 61 | `df42bc3daf5b3d7cc0c6cb898c89b82f835ae233c1b13e1eb4316863f3398fbd` |
| `cargo.log` | 61 | `a278cac6e8410e251ab18613144ca74413b8d8926b0bba8b27f566b016cd430c` |
| `cc.log` | 184 | `0f0f5071e4ce01a4e5ce4c26b1671b7837b458493e259f42d04dff856bcda6ac` |
| `hardware.log` | 622 | `403dde7a3e891a294b448a9014569ff1e5871e2440df5078d0e30b6539163bf0` |
| `sdk.log` | 30 | `b0594ffe6b4818051966ad4e173420477863c282e17b427ce43476d6c7b1c309` |
| `displays.log` | 2,721 | `ac897e7fbe3fa40efe517d78ac54fe1f259336202067abbea85365dd23bfabe6` |
| `mixed-host-baseline.log` | 460 | `dfcee14003efe55e3a422411f84cca14488dfd0b3f8a49f6f6cde8d11e06390a` |
| `fixture-build-debug.log` | 97 | `eef5190aa50f234a560f41b58147dea7f191b4886d686022b8696179d34d34f0` |
| `fixture-debug-verification.log` | 369 | `2a5ed7f73edc6efec7f6e6393daaa76be5da950c62722fe71f0d260278e5c26e` |
| `deterministic-macos.log` | 26,606 | `a842411d5623d1a5a218472dedfb60c40374f3fd4b5929ed6071452386c381c5` |
| `c-abi-check.log` | 24,788 | `0fa13f443da3de9c2f1225d54156ad17f90ec92b90f645cd12d2127e04387187` |
| `mixed-display-conversion.log` | 1,617 | `5836910dee6e4524f7e57183e3df42488388ca89e3bf9e2528ead1cde6f9ad40` |
| `mixed-display-seam.log` | 1,659 | `c1b850fa4ac05121b464c62b13a45917b0ccd9aacf6eb8b00c978271e6931e02` |
| `mixed-display-scale.log` | 1,610 | `3257ecb941574bf99a5109aada097a97722a104aeaaf4388d94b9cdf78d1ca95` |
| `mixed-appkit-renderer-matrix.log` | 94,849 | `da11c96d05c6deb720aa4cd9c93657325cda4accf4ca8209cc58a09b33521445` |
| `mixed-game-like-renderer-matrix.log` | 94,803 | `5cf71ea2b85f18499a7adcea8b49b4a2b826c8a4a2e485a892de2190af186fd4` |
| `unrelated-appkit.log` | 2,354 | `b06cb1476c7b09a45c9f94833004cfcc2d7dd9faa63048c026304638af51a665` |
| `unrelated-game-like.log` | 2,338 | `6d20228253f41ad0a7d2120feb892ef0ff46d0d30b5d606ba6ea7ef914258a09` |
| `sustained-capture-soak.log` | 2,154 | `f240707de952ee0e28e6d2b876c465936ab2a5b4635543f1d393e235b4d75d77` |
| `offscreen-cleanup.log` | 2,494 | `1278c3b6a0b62369d5c0ce62c50cd2277d3f479b7640645a36c3792057cac784` |
| `retained-authority-lifecycle.log` | 3,283 | `f0b34c985ab30bb0d04b882a7828aff72b71ce966af1f849174176f98c5b14b6` |
| `fixture-control.log` | 1,401 | `2996726a2825515246a02fd667327b103df9e594742b2329b14c7f280976cff9` |
| `retained-filter-replacement.log` | 646 | `3767b29d5c4af16a738490ee601f10b72e559147faa8774e48074913deb9aa1d` |
| `system-route.log` | 526 | `aa0af0004211f95aee1395c87872cd20d73aa92d7d016b6be268b3bfba778f45` |
| `single-host-baseline.log` | 175 | `7465f411a1038f598a0fd951ede4c134e74988ea69fd02fe4731b299d0e0310b` |
| `single-display-conversion.log` | 1,691 | `f27c8c97ec59a2243fa1ba0aafdc5e5c90113693a8253768483581ea8612c942` |
| `single-appkit-renderer-matrix.log` | 44,234 | `e16248aff97e9b94fde5d50bc38afd816daadd82cc82b1b28eaf011428f7d38c` |
| `single-game-like-renderer-matrix.log` | 44,218 | `58caaba98cc376d39d953085323d9b7a7b766104ccc7a67c8d58c69820c5d485` |
| `same-scale-host-baseline.log` | 318 | `fc77243622d0107a45cf195a72bd9013ecd2934d524c5213fb781e59c3c7ceab` |
| `same-scale-display-conversion.log` | 1,617 | `fbe75878e8f7e6014ad88cb64b8630cb38ba3cb98820d1573d5ee0a0292d8379` |
| `same-scale-display-seam.log` | 1,722 | `deb5840d7a5ec61d9b78ed8d0c4a741ef8f630eb45add476bc6a01b34d4c8e04` |
| `same-scale-appkit-renderer-matrix.log` | 61,198 | `ce80df0a99f768a2ab2d950399ac54f4ccbf69d4003603dadde6fc8c91104bf4` |
| `same-scale-game-like-renderer-matrix.log` | 61,172 | `43702638e6c579baa8b11cf78d0d6968a8f42917aea18b7d5e2fb7a0a562dff9` |
| `restored-mixed-host-baseline.log` | 460 | `69ed4ea698d1805ca8b652d5fd8afdb9cce6b14d2226ca63eab83a51e008015b` |
| `restored-mixed-display-conversion.log` | 1,617 | `5836910dee6e4524f7e57183e3df42488388ca89e3bf9e2528ead1cde6f9ad40` |
| `restored-mixed-display-seam.log` | 1,659 | `c1b850fa4ac05121b464c62b13a45917b0ccd9aacf6eb8b00c978271e6931e02` |
| `restored-mixed-display-scale.log` | 1,610 | `597cc02de05239e8c0dbe0bd58071b7054fd7e43a0b05126c437d69982671a23` |
| `fixture-build-release.log` | 87 | `7e95865620916ddfb379fc9afac8ac0db7394da1b1e0b49f5c2476a1691fac50` |
| `fixture-release-verification.log` | 369 | `93a463da8dbb7c5e7785d2564ec0fca5cd3cb8e195f986d6ffa12acd95e58994` |
| `capture.log` | 5,218 | `71b6fc2dcb79ce40a16ab474fe2d9082dc1b0d833023ebad4329a3c7fe7f734c` |
| `transitions.log` | 4,094 | `69b8ce7b6dd77b2d0949ca6e976e0a70a724b1d76b0a80b87f505a44f00cd638` |
| `process-directed.log` | 5,161 | `0d6af1e3a4ae95bf3521dde0a6cc30319b52a5dddbde06342eab334226061697` |
| `process-directed-game-like.log` | 5,168 | `dbc865ac0209a79522ebbcbcf9e4d2d8ab2800344a1c99ad66382f45ac04aec4` |
| `process-diagnostics.log` | 4,183 | `10577a7a6cfdd98789e992ba8da70ac65eb98e28fc0577349707ad236df82a16` |
| `input.log` | 5,580 | `95809c334a765491b2dd33e523e075c9e672dbeb3170aa9bb93b383d0264ca1e` |
| `dependency-check.log` | 3,276 | `8b7a14ea0f1e9316a1140b7d3cbbb48d27c37fe5f9ba980c7f8bb49877650c8b` |
| `cargo-fmt.log` | 36 | `7a65ac0e89615c8f5b5efd327e2ee1111be27c07054e4f1690a70d9f8e282b1d` |
| `cargo-clippy.log` | 222 | `8c98463f16ab03f01b07ec11ed471abba050d583a45e808cb24d401a3199ac0f` |
| `cargo-test-workspace.log` | 112,438 | `1d4f480baa1f280302bf563ea19771af44f745a5d3f5c82a920b2ccf565d7d08` |
| `cargo-test-doc.log` | 3,467 | `5a1a2cc58bc836f6d3979bdd6d73433c37b4f3e4c1ac726a6d34e9cf753312a6` |
| `cargo-doc.log` | 829 | `c35521990fa03f7a72e1e218e7edfb554551e9ed81f9b756212c5f0dac0230be` |
| `cargo-deny.log` | 2,082 | `76771058c8a54401a329cf02aea57383bec7129726af67ed6141159892626aa9` |
| `cargo-clippy-macos-cross-target.log` | 228 | `9ca64fd66edd80e6b370dd8dcb1adbb258f7bbb73452e594f98f017a8a75c540` |
| `cargo-test-macos-asan.log` | 17,268 | `65ec1f1c31496786bee4f48f59770d383a2ba220d7d081a0e5f6bcaa64ee473b` |
| `benchmark-profile-gates.log` | 1,467 | `ad5219ce01fff39dc632a2d04f561d2111f76bf7c92c9807eb34e0b3485fcae3` |
| `git-diff-check.log` | 36 | `eaf22227b66af4ffbe44c0fe205fe50ecab5ff99cd24cf2340321c7ba5c265b5` |
| `fixture-process-final.log` | 70 | `2a29eb907c14ad607e7011d2e32608f5ee97fc0e151e949c324b744b94a04b54` |
| `rasen-validate.log` | 544 | `c5811f049c3710081e6d8ed03bcf5cf134e580a1e2a5a4dc4b68d25a66101686` |

### Privacy review

This tracked report contains no captured pixels, recognized or submitted text, window titles, application names, native process identifiers, native window numbers, signing identifiers, raw authorization values, credentials, fixture-private payloads, unrelated foreground identity, or process inventory. Display geometry, internal frame stamps, bounded event/sample counts, typed outcomes, artifact sizes, and hashes are retained because they are required qualification facts.

### Strict release gate

Every route-wide row and every mandatory accepted row for each of the fourteen exact pairs passed on the qualified source revision. The single-display, same-scale, and mixed-scale results are independently recorded. Therefore ADR 0029 remains Accepted for these controlled pair contracts, and public descriptors and documentation may advertise them with `Unknown` compatibility, `OwningProcess` address scope, `InvocationOnly` evidence, foreground-preserving behavior, and explicit caller opt-in.

No support statement may broaden this result to exact-window delivery, arbitrary applications, arbitrary games, display targets, minimized/off-screen targets, application consumption, or visual success. Any later product, fixture, qualification, budget, or mandatory-documentation change invalidates the affected acceptance rows and requires a new revision-bound report.
