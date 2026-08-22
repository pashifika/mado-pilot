## Current candidate decision

Final measured candidate `dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
`109f77df9ef9f40b515245ab60a6036822ee7d78`) passed the controlled AppKit,
game-like, and native input/public-language profiles with zero correctness
failures. The affected three-display `mixed-scale` native matrix,
deterministic one-read proofs, ASan, C ABI/C++/CMake checks, and
Windows-target lint passed on predecessor `df1c45d`. The complete
`df1c45d..dec43d7` diff is exactly
`crates/mado-pilot/benches/native-phase2.rs` — a benchmark-harness file
outside product, fixture, and native-test source — so those revision-bound
native results apply to `dec43d7`. Hosted CI passed on source/test commit
`7ce1602` with pushed evidence head `705c713`.

Subsequent test-only commit `5f1fdb6177d7ec02d2f8eb841f0786432299b0c2`
tightens the minimized/off-screen qualification to require an eventual typed
zero-effect refusal, and that lifecycle row passed. Its complete
`dec43d7..5f1fdb6` diff is exactly
`crates/platform/macos/tests/native_input.rs`; no product, fixture, or
benchmark source changed, so the measured profiles and native results remain
bound to `dec43d7` and are not relabeled.

Commits `7dc5e0c` and `6deec54` tested longer child and pipe-drain allowances;
hosted Windows continued to return the contract's conservative result until job
containment completed. Final test-only commit `7ce1602` restores the production
100 ms drain bound and makes that platform outcome explicit while still
requiring primary success, reader EOF/join, and complete child-tree reaping.
No retained profile value or oracle is relabeled.

Release-level owning-process support is 14 `qualified`, 0 `rejected`, and
0 `unexecuted` pairs. Independent `single`, exact two-display non-mirrored
`same-scale`, and `mixed-scale` matrices each passed on the recorded
source/applicability chain and signed fixtures. No topology or controlled
latency result was substituted for another.

| Field | Current exact-source evidence |
|---|---|
| Product commit / tree | `dec43d7b6c91d415f2028e188e89fa289cb9c1c9` / `109f77df9ef9f40b515245ab60a6036822ee7d78` |
| Fixture source SHA-256 | `c5576c5290003c723f1d3797ab1c6032e935a9e04ab42d50ce5dc9108bc029ea` |
| Signed target / independent foreground release executable SHA-256 | `18a14299de1a2cbb6d26e1aa9f37c1708c1a25ec9a32413e4cc730f3cfa761bc` / `9e8ac146458f86d55b80d0ad719edc58e8eda15c2eb41545da55948de73b205b` |
| Benchmark executable SHA-256 | `fa4ac54a1fa9df3e25833c7923cf7b24dc5ee82a70ddff9a3aa3033a73f5ce08` |
| Input-profile C / C++ executable SHA-256 | `12d4c23d7788973efd01b5257e8352c45b73221088fd6257a7b6c08b94428819` / `4651243160c3845427011f83480ac81197a1a61cb92fa6631cf437f0d6b8da42` |
| Input-profile MadoPilot dynamic library SHA-256 | `e7f42b208522ff772fa67449f52ba7ef33350e535788a29f4876c171a5bf6898` |
| Measured `mixed-scale` topology | three online non-mirrored displays; signed-origin 1× display, 2× main display, and 2× built-in display |
| Supplemental `same-scale` topology | exactly two online non-mirrored horizontally adjacent 2× displays: main 2560×1440 logical / 5120×2880 backing at `(0,0)` and built-in 1512×982 logical / 3024×1964 backing at `(2560,170)` |
| Supplemental `single` topology | exactly one online, non-mirrored 2× built-in display: 1512×982 logical / 3024×1964 backing at `(0,0)` |
| Fixture protocol / internal shim surface / deployment target | version 11 / version 19 / macOS 26.5.2 |
| Native rows | 34 permission-independent rows and all 14 ignored interactive rows passed on the applicable signed-release matrix; independent `single`, exact two-display non-mirrored `same-scale`, and `mixed-scale` matrices each passed all 14 interactive rows plus every applicable display scenario; the tightened minimized/off-screen lifecycle row passed |
| Deterministic one-read binding | eight focused controller, geometry-source, and native-seam proofs passed; terminal `RequireUnchanged`/`UseFrameSnapshot` source/current counts are `[1, 0]`; final ordinary native authority count is one |
| AppKit controlled profile | 50 retained terminal samples; p95 `56.466375 ms` ≤ `106.34 ms`; zero correctness failures; zero allocation growth; profile SHA-256 `288106be21c9c9987a472803260e11f634c2311e521ec9eee6250926e2425fd0` |
| Controlled game-like profile | 50 retained terminal samples; p95 `56.699333 ms` ≤ `112.18 ms`; zero correctness failures; zero allocation growth; profile SHA-256 `1d4b030ad08c5a5febb9167ef5a307b7eed78794e0001ccf7436e2f3365e5b70` |
| Native input / public-language profile | six workloads, 300 retained samples; zero correctness failures; maximum allocation growth 64 bytes; profile SHA-256 `90b33b1f40286fe64d51bcde69340303faafd2145d06f9c3ed8fed5d1877598a` |
| Memory / environment gates | zero process-profile allocation growth; foreground and physical cursor unchanged; one matching fixture event per terminal sequence |
| ASan / ABI / packaging (`dec43d7`) | 254 ASan library tests passed; C ABI 1.2 table 592 bytes; frozen ABI 1.0 prefix 424 bytes and 222 layout lines held; C/C++/CMake, linkage, signing, and panic containment passed |
| Hosted CI | repository policy, branch flow, Windows x86_64, and macOS Apple Silicon passed on source/test commit `7ce1602` with pushed evidence head `705c713` |
| Detailed current procedure and outcomes | [`verification-procedure.md`](../../../rasen/changes/macos-process-directed-performance-tuning/evidence/verification-procedure.md) and [`observed-report.md`](../../../rasen/changes/macos-process-directed-performance-tuning/evidence/observed-report.md) |

The full input benchmark provisions each C/C++ sample's fresh approved
fixture outside its timed span and retains controller-owned mode-0500
executable/library pins per workload; every spawned child must match its
executable pin's live code identity.

The target and independent foreground executables were each accepted only after
their canonical paths, retained application lifetimes, and validity-checked
Security.framework identities matched the pre-launch records.

The controlled profiles are regression evidence for the named source, fixture,
route, `RequireUnchanged` geometry policy, preserving focus policy, and current
topology only. They are not real-time guarantees and establish neither
exact-window consumption nor general application, renderer, input-stack, or game
compatibility.

## Historical optimized candidates

Candidate `9e3e77d4021b792f4c4835390658aaac98e76826` (tree
`ea7881c4416ca2a330fa3097d4fa271f9a547f96`) passed its own exact-source
three-display `mixed-scale` native rows — 33 permission-independent and 11
permissioned integration rows — deterministic one-read proofs, both controlled
performance profiles, ASan, C ABI/C++/CMake checks, Windows-target lint, and
hosted CI. Its profiles recorded p95 `65.078208 ms` / `62.760084 ms`, zero
correctness failures, and maximum allocation growth 2,624 bytes, with profile
SHA-256 `3b52c2dff0e9b348e82d5dff58859a54d318c872af33872725d7ed0235580a65`
and `670a4470cd0da9d673a855f8dc5660308cc034a2fd23f83ddd3eb53f53c6594b`,
fixture source SHA-256
`fa5b1bd7577b877b7c3a42ba8ae5fd57029137bea36da4615881bd6837fbc3d0`, signed
target/foreground executable SHA-256
`51ef9a691c4118b84da9de3ac59b0b39def11ce55ccc2305c9c039d46fb74c29` /
`28a3cc788dd81b81661d911d2e92bc0b4b827b51d832f4991cb516150d53322c`, and
benchmark executable SHA-256
`bd811ac11364c7fc1ad7af3f76b59542cf55c9c1bdd5afa017c9447ac37a54cd`. The
subsequent review-driven source, fixture, and harness corrections invalidated
that evidence for the final candidate; it is historical and transfers no
pair, latency, or support result.

Its first game-like start stopped before warm-up or sampling when the fresh
capture source was rejected. It emitted no profile, left no fixture process,
and is retained as a non-qualifying setup fact. After a ten-second lifecycle
idle, the then-unchanged source, executables, topology, and gates completed
that candidate's accepted profile. Successor `df1c45d` passed the full
permissioned native matrix and both controlled process profiles, then its
public-language input run stopped before reporting when one untimed
auxiliary-window discovery exceeded a separate two-second sub-deadline; the
`dec43d7` harness correction resolved exactly that stop.

Source `a471c2d51428a25dd11e42572b73cf5e86ef7478` retains native-matrix,
deterministic, sanitizer, ABI, and hosted-CI history for that source only. The
benchmark bodies formerly attributed to it use an oracle absent from its tree
and are source/oracle-misbound, non-normative artifacts. Predecessor `28ceb2e`
was rejected after a deterministic proof showed raw point-rectangle equality
refused an unchanged fractional Retina capture. Neither predecessor supplies a
latency or support result for the current candidate.

Historical records remain fixed at immutable snapshots:

- [`8d3fa58738f201496c496159717d274e7f5c06b7`](https://github.com/pashifika/mado-pilot/blob/8d3fa58738f201496c496159717d274e7f5c06b7/docs/evidence/phase-2-native/macos-owning-process-qualification.md) for the superseded tuning history;
- [`3792e78d77c6a81f1aa8d518b36fc9a99f27d1fc`](https://github.com/pashifika/mado-pilot/blob/3792e78d77c6a81f1aa8d518b36fc9a99f27d1fc/docs/evidence/phase-2-native/macos-owning-process-qualification.md) for the superseded `9e3e77d` optimized record;
- [`b76f06fd8997b8c666b18ace6c162c3335953e55`](https://github.com/pashifika/mado-pilot/blob/b76f06fd8997b8c666b18ace6c162c3335953e55/docs/evidence/phase-2-native/macos-owning-process-qualification.md) for the pre-optimization qualified record.

## Historical pre-optimization decision

The Phase 2.2 native qualification matrix passed before the authority-timing optimization.

The Phase 2.2 native qualification matrix **passed** on source commit `a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426` and source tree `f4a707501748303adcec577df5f18fcd18f13f45`.

All fourteen in-scope operation/target/coordinate-space pairs are `qualified` under the frozen qualification plan:

- target classes: the controlled AppKit-renderer fixture window and the controlled OpenGL game-like-renderer fixture window;
- operations: `Pointer`, `Keyboard`, and `Text`;
- pointer coordinate spaces: `CapturePixels`, `FrameNormalized`, `TargetNormalized`, `TargetLogical`, and `DesktopLogical`;
- delivery: explicit `ProcessDirected` only;
- address scope: `OwningProcess`;
- compatibility: `Unknown`;
- submission evidence: `InvocationOnly`;
- focus behavior: preserve the unrelated foreground application, and honour a caller-selected `RequireFocused` predicate without activation;
- target state: the exact retained window remains open, unminimized, on-screen, and owned by the retained process lifetime.

This decision does not claim exact-window delivery, responder selection, queue admission, application consumption, visual effect, arbitrary-application compatibility, or general game compatibility. Display targets, minimized or off-screen windows, and replacement process lifetimes remain outside the qualified scope. `System` remains a separate focus-dependent route.

The revision carries both accepted review corrections. The final-gate focus authority change makes a caller-selected `RequireFocused` predicate travel in the internal process-post request and be observed inside the same bounded native operation as retained-window authority, geometry, event-post access, and process lifetime, immediately before `CGEventPostToPid`; the internal shim surface moved from 13 to 14, which also covers the earlier scroll-location signature change. The fault-conversion change reports that native focus refusal as a typed focus refusal at the public boundary and matches every native status exhaustively.

The earlier matrices at `8309a05c3e7696f3081c5afef6dd6979ea1bb084`, `850b7b26dde49035dd5759685ab6f0c7d996167f`, and `086448f4f37f060b4ce42a887bc63d20f0c240a7` are historical only; none of their pair passes was applied to this source.

## Historical pre-optimization qualified provenance

The accepted native rows were recorded on 2026-08-16 JST. Product, fixture, test, and benchmark source remained fixed at the source commit above for every accepted row. The five benchmark profiles and this report were produced afterward and do not change the measured implementation tree. The release fixture executable is byte-identical to the previous revision's, because the corrected conversion is not reachable from the fixture binary.

| Field | Observed value |
|---|---|
| Base revision | `ffb1823b68ba632b4fc8e7725361ea4596e220f0` |
| Qualified source commit | `a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426` |
| Qualified source tree | `f4a707501748303adcec577df5f18fcd18f13f45` |
| Branch / pull request | `feat/phase-2-2-macos-process-directed-delivery`; PR #36 against `dev/0.2.1` |
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
| `mixed-scale` arrangement | three online, non-mirrored displays: 3840×2160 logical/backing 1× at signed origin `(-3840,109)`, 2560×1440 logical/5120×2880 backing 2× main at `(0,0)`, and 1512×982 logical/3024×1964 backing 2× at `(2560,268)` |
| `same-scale` arrangement | two online, non-mirrored, horizontally adjacent 2× displays: 2560×1440 logical/5120×2880 backing main at `(0,0)` and 1512×982 logical/3024×1964 backing at `(2560,170)` |
| `single` arrangement | one online, non-mirrored 2× display: 2560×1440 logical/5120×2880 backing at `(0,0)`, with the built-in panel offline and the session unlocked |

Qualification executable hashes:

| Artifact role | Bytes | SHA-256 |
|---|---:|---|
| Native input integration test executable | 4,561,376 | `3a5eaacadbcd6512b5593b11a11807e2da5fbe8cb3371f5ac95f1ebbc420400c` |
| macOS unit/scenario test executable | 4,912,104 | `2f82c48c783f56c917476e8651c32b4aa3737386527c2aff7eb1c5ff20832a18` |
| native-phase2 benchmark executable | 1,378,384 | `60c499079d6632177eada2fd08f79c0d0e8a4c6e63d7a84598c8b8d6c3b9d096` |
| Release fixture executable | 599,936 | `fcecc1f820764605d5ca7fa1553d8084411b912f1fc7050230ecc402498c0cdb` |
| C common-flow executable | 34,880 | `c47e516890e5aa69130ef988e7d3e04e1cb941c413d6f820a27df02f1c10f6aa` |
| C++ common-flow executable | 327,960 | `481d470428c7bbff00a8125920f5f9bd3f79187a53c47e33f66f1a3e011a754f` |

## Historical pre-optimization command manifest

The retained pre-optimization evidence was produced by the following command groups. `$APP` and `$FOREGROUND_APP` denote repository-built fixture bundles; absolute workstation paths and private fixture records remain only in ignored raw output.

```sh
cargo build --locked --release -p mado-pilot-platform-macos \
  --features private-fixture --bin mado-pilot-macos-input-fixture
# Assemble both bundles, copy the release executable, and sign each ad hoc with
# its approved identifier.
xcrun codesign --verify --strict --verbose=2 "$APP"
xcrun codesign --verify --strict --verbose=2 "$FOREGROUND_APP"
"$APP/Contents/MacOS/mado-pilot-macos-input-fixture" --report-execution-context

export MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE="$FOREGROUND_APP/Contents/MacOS/mado-pilot-macos-input-fixture"
# Each topology block ran only after the live arrangement matched its exact
# selector, which the run validates before it opens anything.
export MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY=<single|same-scale|mixed-scale>

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
  --source-revision a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426 \
  --source-tree f4a707501748303adcec577df5f18fcd18f13f45 <recorded host options>

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

## Historical pre-optimization topology and native outcomes

| Selector / row group | Historical outcome |
|---|---|
| `mixed-scale` display-frame rows | passed: three displays published their own logical/backing geometry; the signed-origin 1×/2× seam and adjacent 2×/2× seam were exercised |
| `mixed-scale` AppKit renderer | passed: four visits closed the 2× → 2× → 1× → 2× cycle at geometry revisions 3/4/5/6 and epochs 2/2/3/4 |
| `mixed-scale` controlled OpenGL renderer | passed: the same four-visit scale/geometry cycle completed independently |
| `same-scale` AppKit renderer | passed: two visits crossed the adjacent 2×/2× seam at geometry revisions 3/4 and epoch 2 |
| `same-scale` controlled OpenGL renderer | passed: the same two-visit traversal completed independently |
| `single` AppKit renderer | passed: one visit stayed on the single 2× display at geometry revision 3 and epoch 2 |
| `single` controlled OpenGL renderer | passed: the same single-display row completed independently |

Each renderer test executed its fixed matrix of five pointer coordinate spaces, seven pointer sequences over the topology's authoritative geometry stages, five keyboard cases, three Unicode text cases, cancellation/cleanup, caller-selected `RequireFocused` success and inactive refusal, foreground/cursor invariance, independent process observation, and strictly newer controlled visual confirmation.

Retained frame sequences identify capture progress only and claim no input-caused visual effect:

| Selector | AppKit visits | Controlled OpenGL visits |
|---|---|---|
| `mixed-scale` | 1,178 / 3,462 / 1,103 / 1,174 | 1,180 / 3,443 / 1,090 / 1,173 |
| `same-scale` | 1,152 / 3,396 | 1,168 / 3,497 |
| `single` | 1,195 | 1,142 |

The default private-fixture suite recorded 253 passes and ten deliberately ignored interactive rows; all ten ignored rows were executed separately on the qualified revision, including the operator-clicked `System` route row. The complete workspace recorded 1216 passes and 11 ignored opt-in tests. Doctests recorded 8 passes and 1 ignored example. The dedicated native-shim AddressSanitizer run recorded 188 passes with no sanitizer finding. PR #36 passed repository policy, branch flow, Windows, and macOS CI jobs on this revision.

### Route-wide decisions

| Row | Decision | Observed basis |
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

No route-wide row failed.

### Historical pre-optimization pair decisions

| Operation | Target class | Coordinate space | Decision | Reason |
|---|---|---|---|---|
| `Pointer` | AppKit renderer | `CapturePixels` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | AppKit renderer | `FrameNormalized` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | AppKit renderer | `TargetNormalized` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | AppKit renderer | `TargetLogical` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | AppKit renderer | `DesktopLogical` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Keyboard` | AppKit renderer | `not applicable` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground invariance, physical-cursor invariance, and independent process observation. |
| `Text` | AppKit renderer | `not applicable` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground invariance, physical-cursor invariance, and independent process observation. |
| `Pointer` | OpenGL game-like renderer | `CapturePixels` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | OpenGL game-like renderer | `FrameNormalized` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | OpenGL game-like renderer | `TargetNormalized` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | OpenGL game-like renderer | `TargetLogical` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Pointer` | OpenGL game-like renderer | `DesktopLogical` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground and physical-cursor invariance and independent process observation. |
| `Keyboard` | OpenGL game-like renderer | `not applicable` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground invariance, physical-cursor invariance, and independent process observation. |
| `Text` | OpenGL game-like renderer | `not applicable` | `qualified` | Its `single`, `same-scale`, and `mixed-scale` renderer rows each passed on this revision under sustained capture, with foreground invariance, physical-cursor invariance, and independent process observation. |

Pair totals: 14 `qualified`, 0 `rejected`, 0 `unexecuted`.

## Historical pre-optimization performance outcomes

The five Phase 2.2 profiles were measured on the qualified source under the `mixed-scale` arrangement, which their committed notes record. All 2,700 retained samples across twenty-four workloads satisfied their correctness oracle and frozen latency, hard-maximum, mapped-byte, stale-work, heap, allocation-growth, and diagnostics-capacity gates. `result_correctness` was zero for every workload, and every workload recorded zero post-warmup allocation growth.

| Profile / workload | p50 ms | p95 ms | max ms | Peak live Rust bytes |
|---|---:|---:|---:|---:|
| capture / `fixture_command_acknowledgement` | 0.175541 | 0.248958 | 1.413042 | 36,933 |
| capture / `controlled_stimulus_to_frame` | 16.604166 | 17.434709 | 18.569125 | 9,300,405 |
| capture / `static_latest_retained` | 0.000125 | 0.000167 | 0.000167 | 4,671,461 |
| capture / `static_newer_repeated_pixels` | 17.482458 | 35.539792 | 55.728583 | 9,300,405 |
| capture / `latest_acquisition` | 0.000500 | 0.000750 | 0.001292 | 9,300,405 |
| capture / `cpu_map_bgra8` | 0.185125 | 0.206250 | 0.247417 | 9,300,405 |
| transitions / `resize_recreation` | 52.555834 | 61.353917 | 69.669959 | 8,316 |
| transitions / `open_first_frame` | 98.200417 | 109.305875 | 115.946083 | 5,335,278 |
| transitions / `retained_pressure_resume` | 3.743334 | 17.520750 | 17.898917 | 4,671,932 |
| transitions / `close_drain` | 65.151709 | 74.761417 | 79.895917 | 7,246 |
| AppKit / `discovery_open_retained_authority` | 311.514042 | 387.611417 | 395.485833 | 51,143 |
| AppKit / `event_authority_preflight_post` | 207.483584 | 212.674625 | 213.684958 | 4,673,349 |
| AppKit / `release_cleanup` | 1.629375 | 2.452042 | 2.867166 | 4,673,373 |
| AppKit / `session_close` | 62.575042 | 72.372375 | 74.834708 | 43,606 |
| AppKit / `fixture_controller_close` | 32.183208 | 54.893166 | 56.674875 | 37,401 |
| game-like / `discovery_open_retained_authority` | 315.287375 | 371.193583 | 389.543875 | 51,143 |
| game-like / `event_authority_preflight_post` | 207.938166 | 224.368667 | 231.991584 | 4,673,349 |
| game-like / `release_cleanup` | 1.741334 | 2.671834 | 2.892750 | 4,673,373 |
| game-like / `session_close` | 64.954500 | 75.559042 | 78.040875 | 43,606 |
| game-like / `fixture_controller_close` | 20.868334 | 33.520208 | 35.951292 | 37,413 |
| diagnostics / `event_diagnostics_off` | 204.886500 | 211.246083 | 216.556541 | 4,673,469 |
| diagnostics / `event_diagnostics_normal` | 204.548666 | 210.178084 | 213.286416 | 4,683,877 |
| diagnostics / `event_diagnostics_debug` | 204.995584 | 211.942083 | 232.486083 | 4,683,877 |
| diagnostics / `event_diagnostic_overflow` | 207.117459 | 212.318916 | 223.287667 | 4,674,301 |

The controlled capture profile retained the exact 4,628,480-byte BGRA8 mapping bound and a zero stale-work ratio for acquisition workloads. The transition profile recorded its measured retained-pressure ratio below the frozen 0.95 ceiling. Both process-directed renderer profiles remained below the frozen 16 MiB live-Rust-heap ceiling. The authority/preflight/post workloads measured roughly 210–224 ms p95 across the three process-directed profiles, below the frozen 750 ms budget. These are controlled-fixture regression measurements, not real-time guarantees or general game compatibility evidence; the focus predicate is not part of that workload, because the benchmark posts under the default preserving policy.

The separately versioned ADR 0025 native-input/public-language profile remains bound to the historical `8309a05` source and is not represented as current-source evidence.

Historical as-measured profile hashes:

The two renderer profile files were later overwritten by corrected optimized
measurements and are not reproduced by the current checkout. These digests bind
only the historical pre-optimization output named by this section.

| Profile | SHA-256 |
|---|---|
| `phase-2-2-controlled-capture-aarch64-apple-darwin.toml` | `fc8c4f052e34f04faed07bcb43fb37cac287b8aed8b2162cecb9c1798c8bff23` |
| `phase-2-2-controlled-transitions-aarch64-apple-darwin.toml` | `29cab8e052b924ecf6033412c02fe7ae5c3bf9b8bdfb151b087575026805b7af` |
| `phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml` | `4d9dfb104630208537af519ee03048dac03ff96b2415f85d4dd165da0be49caa` |
| `phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml` | `2c5c56a7c72c4146e304f1e136f694b27333212e7ff86784c0bd3819b51e9eb8` |
| `phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml` | `900e333f68adf9de5924e057fe82ece783aef422c24ccfdce35e1a038bb4248b` |

## Historical pre-optimization derivative provenance

Raw session logs are not part of the tracked evidence chain and are not named,
sized, or hashed here. Some contain workstation paths, native identifiers, or
fixture-private records; hashing those payloads would not make their provenance
safe. The durable record is limited to privacy-sanitized derivatives:

| Evidence class | Retained derivative |
|---|---|
| Source and environment | exact product commit/tree, public toolchain and host versions, topology class, and fixture source/executable digests |
| Native qualification | public scenario names, aggregate pass/fail counts, pair decisions, and cleanup outcome |
| Benchmarks | committed profile digests, bounded measurement tables, and hard-gate decisions |
| Repository verification | command class, aggregate suite counts, public ABI extents, and pass/fail outcome |
| Stopped attempt | setup failed before the first retained sample; zero submitted events; fixtures stopped; no result promoted |

The complete historical tracked record is fixed at immutable snapshot
[`b76f06fd8997b8c666b18ace6c162c3335953e55`](https://github.com/pashifika/mado-pilot/blob/b76f06fd8997b8c666b18ace6c162c3335953e55/docs/evidence/phase-2-native/macos-owning-process-qualification.md).

## Historical pre-optimization privacy review

This tracked record contains no captured pixels, recognized or submitted text,
window titles, application names, native process identifiers, native window
numbers, signing identifiers, raw authorization values, credentials,
fixture-private payloads, unrelated foreground identity, absolute workstation
paths, or process inventory.

## Historical pre-optimization strict release gate

The gate **passed** for the fourteen pairs above and for nothing else. No row failed and no mandatory row was unexecuted: each renderer pair passed its own `single`, `same-scale`, and `mixed-scale` rows on that revision, and no topology's evidence was substituted for another's.

Nothing here qualifies exact-window delivery, arbitrary applications, arbitrary games, display targets, minimized or off-screen targets, other-Space targets, application consumption, or visual success. Any later product, fixture, qualification, budget, or mandatory-documentation change invalidates the affected acceptance rows and requires a new revision-bound report.

## Superseded complete-run record

The remainder of this document is the complete historical record for source `8309a05c3e7696f3081c5afef6dd6979ea1bb084` and tree `27fe879e0c4bb55fe4850d9a50737b568936cc10`. It was valid before the later product correction. Its pair decisions, profile hashes, privacy-sanitized derivatives, and strict-gate conclusion are retained for provenance only and do not describe current source.

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
xcrun codesign --verify --strict --verbose=2 "$APP"
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
xcrun codesign --verify --strict --verbose=2 "$APP"

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set <capture|transitions|process-directed|process-directed-game-like|process-diagnostics> \
  --fixture-executable "$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --source-revision 8309a05c3e7696f3081c5afef6dd6979ea1bb084 \
  --source-tree 27fe879e0c4bb55fe4850d9a50737b568936cc10 \
  --toolchain "rustc 1.97.1; Apple clang 21.0.0; macOS SDK 26.5" \
  --gpu-driver "Apple integrated GPU; system driver stack" \
  --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
  --os-version "macOS 26.5.2 (25F84)" \
  --deployment-target "macOS 26.5.2" \
  --display-topology "<recorded three-display mixed-scale topology>" \
  --permissions-signing "<recorded non-sensitive authorization/signing facts>"

cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set input \
  --fixture-executable "$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --c-executable "<c-abi-check>/macos-native-input" \
  --cpp-executable "<c-abi-check>/macos-native-input-cpp" \
  --library "<cargo-profile>/deps/libmadopilot.dylib" \
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

Committed profile hashes, as those five Phase 2.2 files stood for this historical
run. The tracked files were later overwritten by the corrected-revision reruns,
so a current checkout no longer reproduces these digests; the current-source
digests are in the current record above.

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

The first same-scale AppKit renderer attempt on the final source reached its
five-second operation deadline after 208 seconds. Its receipt was `Unexecuted`
with zero submitted events, so it had no possible native effect and contributes
to no pair result. The owned fixtures were confirmed stopped, a ten-second idle
interval elapsed, and the same isolated full renderer matrix then passed. No
failed output was overwritten or promoted, and no raw artifact identifier or
digest is retained.

### Privacy-sanitized derivative provenance

Raw session logs are excluded from the tracked evidence chain because they
contain native identifiers and fixture-private records. Durable evidence retains
only the named source/tree, public environment facts, fixture digests, scenario
and workload names, aggregate counts, pair decisions, bounded profile tables,
public ABI extents, and pass/fail outcomes.

### Privacy review

This tracked report contains no captured pixels, recognized or submitted text,
window titles, application names, native process identifiers, native window
numbers, signing identifiers, raw authorization values, credentials,
fixture-private payloads, unrelated foreground identity, absolute workstation
paths, or process inventory.

### Strict release gate

Every route-wide row and every mandatory accepted row for each of the fourteen exact pairs passed on that historical source revision. The single-display, same-scale, and mixed-scale results were independently recorded. ADR 0029's acceptance conditions were therefore satisfied for those controlled pair contracts on that revision, and descriptors and documentation for that source could advertise them with `Unknown` compatibility, `OwningProcess` address scope, `InvocationOnly` evidence, foreground-preserving behavior, and explicit caller opt-in. The optimized-source decision at the start of this report supersedes that release status.

No support statement may broaden this result to exact-window delivery, arbitrary applications, arbitrary games, display targets, minimized/off-screen targets, application consumption, or visual success. Any later product, fixture, qualification, budget, or mandatory-documentation change invalidates the affected acceptance rows and requires a new revision-bound report.
