## Current decision

The corrected implementation at source commit `086448f4f37f060b4ce42a887bc63d20f0c240a7` and source tree `c3c4e9969448cc34e89018f699d055f747c7427e` is **not yet release-qualified**.

The route-wide rows, the complete mixed-scale AppKit and controlled OpenGL renderer rows, the five Phase 2.2 performance profiles, AddressSanitizer, repository verification, and pull request CI pass on that revision. The exact `single` and `same-scale` renderer rows have not been run on it. Because the frozen plan requires every topology for each dependent pair, the current decision is 0 `qualified`, 0 `rejected`, and 14 `unexecuted`. Draft pull request #36 remains draft.

This revision adds the final-gate focus authority correction: a caller-selected `RequireFocused` predicate now travels in the internal process-post request and is observed inside the same bounded native operation as retained-window authority, geometry, event-post access, and process lifetime, immediately before `CGEventPostToPid`. The internal shim surface moved from 13 to 14, which also covers the earlier scroll-location signature change. Both earlier matrices at `8309a05c3e7696f3081c5afef6dd6979ea1bb084` and `850b7b26dde49035dd5759685ab6f0c7d996167f` are historical only; none of their pair passes may be applied to this source.

## Current corrected-revision provenance

The current partial rerun was recorded on 2026-08-16 JST. Product, fixture, test, and benchmark source remained fixed at the source commit above. The five benchmark profiles and this report were produced afterward and do not change the measured implementation tree; the recorded working-tree status counts exactly those five profile files.

| Field | Observed value |
|---|---|
| Base revision | `ffb1823b68ba632b4fc8e7725361ea4596e220f0` |
| Corrected source commit | `086448f4f37f060b4ce42a887bc63d20f0c240a7` |
| Corrected source tree | `c3c4e9969448cc34e89018f699d055f747c7427e` |
| Branch / pull request | `feat/phase-2-2-macos-process-directed-delivery`; draft PR #36 against `dev/0.2.1` |
| Host | `MacBookPro18,3`; Apple M1 Pro, 10 physical cores, 32 GiB, `arm64` |
| OS / SDK | macOS 26.5.2 build 25F84; macOS SDK 26.5 |
| Minimum deployment target | macOS 26.5.2 |
| Apple compiler | Apple clang 21.0.0 |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`; `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Internal shim surface | version 14; size-versioned process-post request with caller focus requirement and focus-result report field |
| Fixture protocol | version 10; one outstanding command; 512-byte command/result bound; 1,024-byte ready-record bound; 1,024-event recorder bound; owned-child teardown |
| Fixture source set | SHA-256 `51c6f991a942d30440f18a8b06e105ebb3bc15511a9e909ef696b300ba8d4c7b` |
| Fixture launch/signing | bundled launch; structurally valid ad-hoc signature; approved identifiers matched for both roles |
| Authorization | Screen Recording granted; non-prompting event-post access granted; required observations agreed |
| Prompting behavior | no permission request, settings opener, target activation, or target raise |
| Current topology | three online, non-mirrored displays: 3840×2160 logical/backing 1× at signed origin `(-3840,109)`, 2560×1440 logical/5120×2880 backing 2× main at `(0,0)`, and 1512×982 logical/3024×1964 backing 2× at `(2560,268)`; both mixed-scale and same-scale seams are present |

Current executable hashes:

| Artifact role | Bytes | SHA-256 |
|---|---:|---|
| Native input integration test executable | 4,561,376 | `66d6b4c8af1644c54d5329223f32f019180374b337e9a931b5cdd249808203a3` |
| macOS unit/scenario test executable | 4,911,288 | `294bfba642edd2ce5791e16fea397f092df789d0607ead737b018c210808aa2b` |
| native-phase2 benchmark executable | 1,378,384 | `4723ef6a558072ed93af993d6d9f75656b2d965b9608885b790ed1a96b7e6812` |
| Release fixture executable | 599,936 | `fcecc1f820764605d5ca7fa1553d8084411b912f1fc7050230ecc402498c0cdb` |
| C common-flow executable | 34,880 | `c47e516890e5aa69130ef988e7d3e04e1cb941c413d6f820a27df02f1c10f6aa` |
| C++ common-flow executable | 327,960 | `481d470428c7bbff00a8125920f5f9bd3f79187a53c47e33f66f1a3e011a754f` |

## Current command manifest

The retained current-revision evidence was produced by the following command groups. `$APP` and `$FOREGROUND_APP` denote repository-built fixture bundles; absolute workstation paths and private fixture records remain only in ignored raw output.

```sh
cargo build --locked --release -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
# Assemble both bundles, copy the release executable, and sign each ad hoc with
# its approved identifier.
/usr/bin/codesign --verify --strict --verbose=2 "$APP"
/usr/bin/codesign --verify --strict --verbose=2 "$FOREGROUND_APP"
"$APP/Contents/MacOS/mado-pilot-macos-input-fixture" --report-execution-context

export MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE="$FOREGROUND_APP/Contents/MacOS/mado-pilot-macos-input-fixture"
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

# Each of the ten ignored rows ran alone with the same package, features, and
# test target: the two renderer matrices, both unrelated-activity rows, the
# sustained-capture soak, the off-screen/closed pointer row, the retained
# authority lifecycle row, the fixture-control row, the retained-filter
# replacement row, and the operator-clicked System route row.
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input <row> -- --ignored --exact --nocapture --test-threads=1

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set <capture|transitions|process-directed|process-directed-game-like|process-diagnostics> \
  --fixture-executable "$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --source-revision 086448f4f37f060b4ce42a887bc63d20f0c240a7 \
  --source-tree c3c4e9969448cc34e89018f699d055f747c7427e <recorded host options>

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
  -p mado-pilot-platform-macos --target-dir target/asan --lib -- --test-threads=1
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

Each completed mixed-scale renderer test executed its fixed matrix of five pointer coordinate spaces, seven pointer sequences over eleven authoritative geometry stages, five keyboard cases, three Unicode text cases, cancellation/cleanup, caller-selected `RequireFocused` success and inactive refusal, foreground/cursor invariance, independent process observation, and strictly newer controlled visual confirmation. AppKit frame sequences for the four visits were 1,152 / 3,335 / 1,028 / 1,157; controlled OpenGL sequences were 1,220 / 3,528 / 1,113 / 1,203. These values identify capture progress only; they do not claim input-caused visual effect.

The default private-fixture suite recorded 252 passes and ten deliberately ignored interactive rows; all ten ignored rows were executed separately on the corrected revision, including the operator-clicked `System` route row. The complete workspace recorded 1215 passes and 11 ignored opt-in tests. Doctests recorded 8 passes and 1 ignored example. The dedicated native-shim AddressSanitizer run recorded 187 passes with no sanitizer finding. Draft PR #36 passed repository policy, branch flow, Windows, and macOS CI jobs on this revision.

### Route-wide decisions

| Row | Decision | Current observed basis |
|---|---|---|
| RW-01 | passed | controlled symbol loading, availability, internal version 14/layout, package isolation, production linkage, C ABI, and C++ checks passed |
| RW-02 | passed | retained logical window, original process lifetime, current ownership, and eligible state were revalidated before ordinary posts |
| RW-03 | passed | replacement, close/relaunch, stale identity, and retained-filter refusal rows passed |
| RW-04 | passed | renderer admission dwell and the bounded sustained-capture soak completed |
| RW-05 | passed | primary-only and additional same-process window states remained process-scoped |
| RW-06 | passed | unrelated foreground preservation and no target activation/raise passed, including under caller-selected `RequireFocused` |
| RW-07 | passed | non-prompting grant, denial, revocation, unavailable-symbol, and disagreement behavior failed closed; an unobservable focus predicate reported the authorization answer |
| RW-08 | passed | receipts remained `ProcessDirected`, `OwningProcess`, and `InvocationOnly` with exact submitted prefixes |
| RW-09 | passed | no implicit fallback; explicit fallback closed after possible native effect |
| RW-10 | passed | deadline and cancellation arbitration prevented later ordinary posts or result mutation |
| RW-11 | passed | sequence state, newest-first cleanup, close/drain, and repeated close remained bounded; cleanup required no focus predicate |
| RW-12 | passed | owned-child identity, nonce, protocol bounds, replacement, and teardown checks passed |
| RW-13 | passed | diagnostics modes and overflow remained bounded and privacy-safe with exact loss accounting |
| RW-14 | passed | pointer/range validation, focus-gate refusal rows, exception/panic containment, queue pressure, allocation gates, and 187 ASan tests passed |

No route-wide row failed. Missing topology rows still block every final pair decision.

### Current pair decisions

| Operation | Target class | Coordinate space | Decision | Reason |
|---|---|---|---|---|
| `Pointer` | AppKit renderer | `CapturePixels` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `FrameNormalized` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `TargetNormalized` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `TargetLogical` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | AppKit renderer | `DesktopLogical` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Keyboard` | AppKit renderer | `not applicable` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Text` | AppKit renderer | `not applicable` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `CapturePixels` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `FrameNormalized` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `TargetNormalized` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `TargetLogical` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Pointer` | OpenGL game-like renderer | `DesktopLogical` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Keyboard` | OpenGL game-like renderer | `not applicable` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |
| `Text` | OpenGL game-like renderer | `not applicable` | `unexecuted` | The mixed-scale row passed on the corrected source, but its exact `single` and `same-scale` renderer rows have not run; another topology cannot substitute. |

Pair totals: 0 `qualified`, 0 `rejected`, 14 `unexecuted`. The implementation remains reviewable on the draft branch, but no current-source release support claim follows from this partial matrix.

## Current performance outcomes

The five Phase 2.2 profiles were remeasured on the corrected source. All 2,700 retained samples across twenty-four workloads satisfied their correctness oracle and frozen latency, hard-maximum, mapped-byte, stale-work, heap, allocation-growth, and diagnostics-capacity gates. `result_correctness` was zero for every workload, and every workload recorded zero post-warmup allocation growth.

| Profile / workload | p50 ms | p95 ms | max ms | Peak live Rust bytes |
|---|---:|---:|---:|---:|
| capture / `fixture_command_acknowledgement` | 0.169917 | 0.219959 | 1.489458 | 36,933 |
| capture / `controlled_stimulus_to_frame` | 16.553000 | 17.283542 | 22.173500 | 9,300,383 |
| capture / `static_latest_retained` | 0.000125 | 0.000125 | 0.000208 | 4,671,439 |
| capture / `static_newer_repeated_pixels` | 18.395584 | 36.464041 | 53.894458 | 9,300,383 |
| capture / `latest_acquisition` | 0.000625 | 0.000875 | 0.001000 | 9,300,383 |
| capture / `cpu_map_bgra8` | 0.187167 | 0.207791 | 0.256833 | 9,300,383 |
| transitions / `resize_recreation` | 51.709708 | 71.010167 | 88.733208 | 8,294 |
| transitions / `open_first_frame` | 97.363500 | 106.136750 | 113.065250 | 5,335,256 |
| transitions / `retained_pressure_resume` | 2.852083 | 16.288125 | 18.403459 | 4,671,910 |
| transitions / `close_drain` | 59.905916 | 73.023250 | 77.693375 | 7,224 |
| AppKit / `discovery_open_retained_authority` | 314.669416 | 352.811667 | 374.007875 | 51,055 |
| AppKit / `event_authority_preflight_post` | 204.121666 | 208.971250 | 210.760584 | 4,673,327 |
| AppKit / `release_cleanup` | 1.723625 | 2.747000 | 2.773875 | 4,673,351 |
| AppKit / `session_close` | 64.088292 | 73.663833 | 77.099417 | 43,584 |
| AppKit / `fixture_controller_close` | 51.541125 | 55.919708 | 59.933916 | 37,401 |
| game-like / `discovery_open_retained_authority` | 313.802875 | 371.752958 | 379.814625 | 51,055 |
| game-like / `event_authority_preflight_post` | 204.882500 | 211.230792 | 216.500209 | 4,673,327 |
| game-like / `release_cleanup` | 1.763708 | 2.611416 | 2.703167 | 4,673,351 |
| game-like / `session_close` | 65.680542 | 70.784375 | 74.851250 | 43,584 |
| game-like / `fixture_controller_close` | 20.915083 | 26.439875 | 33.242208 | 37,413 |
| diagnostics / `event_diagnostics_off` | 204.396125 | 209.158334 | 213.124042 | 4,673,447 |
| diagnostics / `event_diagnostics_normal` | 203.103209 | 208.690958 | 215.558541 | 4,683,855 |
| diagnostics / `event_diagnostics_debug` | 203.078959 | 209.701792 | 227.195292 | 4,683,855 |
| diagnostics / `event_diagnostic_overflow` | 206.405709 | 211.661625 | 226.198125 | 4,674,279 |

The controlled capture profile retained the exact 4,628,480-byte BGRA8 mapping bound and a zero stale-work ratio for acquisition workloads. The transition profile recorded its measured retained-pressure ratio below the frozen 0.95 ceiling. Both process-directed renderer profiles remained below the frozen 16 MiB live-Rust-heap ceiling. The authority/preflight/post workload measured roughly 204–212 ms p95 across the three process-directed profiles, below the frozen 750 ms budget. These are controlled-fixture regression measurements, not real-time guarantees or general game compatibility evidence; the focus predicate is not part of that workload, because the benchmark posts under the default preserving policy.

The separately versioned ADR 0025 native-input/public-language profile remains bound to the historical `8309a05` source. One earlier corrected-source refresh attempt stopped before the first retained sample because the fixture controller found non-empty pending event state at reset. No measurement from that attempt was accepted, and the historical input profile is not represented as current-source evidence.

Current committed profile hashes:

| Profile | SHA-256 |
|---|---|
| `phase-2-2-controlled-capture-aarch64-apple-darwin.toml` | `1586a75702b7de82326e45087567e6c5ac8a7316196054790b8c66a409b2fcac` |
| `phase-2-2-controlled-transitions-aarch64-apple-darwin.toml` | `05f8d23299581d9a2ab65e9bb080cf613a3b1647a85dc5654ad1bbd7ccc22979` |
| `phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml` | `2204caeb8437ab9923595ba870788641a00fff915c24f37be76b1e16bdb294c3` |
| `phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml` | `15eb753ca8340edb915958aa1fc7d98cb7bf2663a23b17e591e064107229f144` |
| `phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml` | `c7c7d8be05885f028f7f43e9ffe8c0fcc95e9d9b50ea7feeeafff65fd818137a` |

## Current raw-output provenance

Accepted current-revision raw logs remain ignored under `.rasen/changes/phase-2-2-macos-owning-process-delivery/ephemera/qualification-final-086448f/`. They may contain native identifiers or fixture-private records and must not be committed. The tracked binding is limited to file size and SHA-256.

| Accepted raw artifact | Bytes | SHA-256 |
|---|---:|---|
| `benchmark-profile-gates.log` | 1,342 | `e8b752d78e19df81cb485253ab33d09d900e0be494dcd5ee92d384729173ffdb` |
| `c-abi-check.log` | 24,782 | `d8f9e581ae76170dc9c8cb6175d3bc463b34e6069c55722ff7c38aab15359c7e` |
| `capture.log` | 4,991 | `3c1dcbde345f531512a4164778d6c7bf187fbf7efcf84d198ad51ba06af63748` |
| `cargo-clippy-macos-cross-target.log` | 72 | `59639588a6f1826f129cbcd121edc07279dab94786f5fa5bde6b3439b2e845cc` |
| `cargo-clippy.log` | 197 | `ccf32af401aea84583ef30870e141c8b27922ba28967023b621c4c8da27dcf36` |
| `cargo-deny.log` | 2,057 | `56d153a74401bff3d2f1ec6b638b5e4dd59f823921fec4e2c09a04bbefa0115f` |
| `cargo-doc.log` | 198 | `b8e562e3ce48b3ab2d15fd4b60c649f708f8b9861cd6319f7e4d52977acb90c4` |
| `cargo-fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `cargo-test-doc.log` | 3,467 | `aadc39a0f7d9a022bcc0bf2ebdd3328e5fcb135b57587e480ff822def17e7a53` |
| `cargo-test-macos-asan.log` | 17,411 | `60fc132e7c83a3da3b83f1166c41efdd2458f03655ee2ee455ab2e596bb8cd17` |
| `cargo-test-private-fixture.log` | 25,835 | `3ce79ff599f885d6cb08effda139853b62073734f3d6fb394e06fec3f985e273` |
| `cargo-test-workspace.log` | 112,601 | `94f9bfd30c226be6fead0d994bfa5c0f45c059e053d6036451e6b1625d668307` |
| `cargo.log` | 192 | `53d3dc8115abfdc08d1d740fe5c1ac16528c4a9ba91c92f98bf77db672cb771b` |
| `cc.log` | 159 | `76a924f59997fb65051981eb13105e0a8c151f16cd9c3e45e0f7c27ecefdb839` |
| `ci-pr36.log` | 464 | `629fb584c473f065b1fa68b389c01210ef165297dc59dc94758d7c459a613428` |
| `dependency-check.log` | 3,276 | `066169d29508bfc697be756d09a689debf75dd9a8411ae10b07a4ff92771befd` |
| `displays.log` | 927 | `825d943436d53865e0cc5dd3fb8f2eb0157e56ee5781dea25b24a6b53ed625bc` |
| `fixture-build-release.log` | 62 | `222a02ed477d249a21359df8cae975864a62d848f4739cadc5b00b6190a963c6` |
| `fixture-control.log` | 1,401 | `158d3a6a01fbf39caf966b1cd40ad3e66ce386ab9a5b619b3dff1590325ad9c1` |
| `fixture-release-verification.log` | 430 | `a5fdf19fafd4915f420adf7e8a218fa9a9293deae2628b02eeda96622faef67d` |
| `hardware.log` | 43 | `aaa406159e8d01e903ad19b8d9e86d6a055f2d4a88090c2004c92f8da09afcbe` |
| `host-status.log` | 9 | `30179a803d48be87dfe0857a82b26d66e8c866dcbdc7c8b50aa3d24dff84f81e` |
| `mixed-appkit-renderer-matrix.log` | 95,339 | `5ded39905a0a4cfa5a0aacc6bd582114b60e6856a6b3baa9ab3a510d8c629ba1` |
| `mixed-display-conversion.log` | 1,617 | `bc61056b9b90c36641dd3287e51505851bbc18837f256b63371bfefd43e9616c` |
| `mixed-display-scale.log` | 1,610 | `9f432fc571c0b4e64a4d142f7afd8da3c7aab29e762832966010da07810be25f` |
| `mixed-display-seam.log` | 1,659 | `4f5bd08f8f21666852863801fd7f27d1d08b44141cf9d36673b7c387123b4562` |
| `mixed-game-like-renderer-matrix.log` | 95,284 | `aefd49478a7c03edbc18fd2ad218c3136c6c33411da3711c661000355d5ff282` |
| `offscreen-cleanup.log` | 2,496 | `25794a561134af993a647a92ef6192062493ac9036d93097fae23d1fe0e7fe45` |
| `os.log` | 65 | `9a27ea4f1e11f7b9a4f7573d8729db037a10ce2e457f97cb73f646a96a550b16` |
| `process-diagnostics.log` | 3,955 | `3bd9120031c63dc5709625fb5287ab18c8735aedf6618b6c9bdcb00094d29b0c` |
| `process-directed-game-like.log` | 4,941 | `314bf543f10c22b44b67cd2430b54b66d6c557eff8e0b453fbc7c79bf5d0cf04` |
| `process-directed.log` | 4,934 | `37e8e792e5c27551ee4fa19e87ca5095850088ab8aaf22760a4a9818c23847ea` |
| `retained-authority-lifecycle.log` | 3,284 | `750bf70ec1549eb352140c4f9deeb97ee3f431f5728f30d6d4cfe4883efb3073` |
| `retained-filter-replacement.log` | 620 | `37df3f03670da64adee11e83184237cd26a1dac5b770207ceab0c00346f0156c` |
| `rustc.log` | 72 | `cdd372f49763b7ad6faf8210706bc4f7212e18ebbd1abee8ab874ebbd588c4a3` |
| `sdk.log` | 5 | `25205f2e2f02dc71036ee827e19c49b893b231a3d1af240e35a3ac55aa8cdcb6` |
| `source-commit.log` | 41 | `0c826cd856f23c485ffcd43230303f213e1d2ef2841b786012295a201e7a0fa7` |
| `source-tree.log` | 41 | `dfb8fd7590804a0385b475490e87b46d17eb104225dbb764147a38d4570e304b` |
| `sustained-capture-soak.log` | 2,155 | `ad32d67f235c4aa9780d4aa44783c11414c6857087f7814be7cc0d4a986afb96` |
| `system-route.log` | 501 | `8d7a191b1bfacdfacd584ba8ad9d793ec9157d8505588b5e9652090ca9d2a2bd` |
| `transitions.log` | 3,865 | `4c938ed44847f5a35874c9f067bd66c59557b094ed705d0164c21d26e70ff288` |
| `unrelated-appkit.log` | 2,355 | `7ae8077f345a2c74c7dda143be302ee18865e147811fbab3d85ca9349b5994c4` |
| `unrelated-game-like.log` | 2,339 | `8bea9177e60ac8db0325e39ab28fea40a9796913396431d1a7bc8ac0034434bf` |

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
