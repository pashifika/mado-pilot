# Validation gates

A validation gate is a version-one decision that was deliberately left unresolved
until the evidence needed to settle it existed. Each gate records what is
undecided, what evidence resolves it, when it must be resolved, what it blocks,
its status, and where the current rule and its evidence live.

Gates keep an unresolved decision visible instead of letting an implementation
pull request settle it by accident. A gate is not permission to weaken an
architectural seam recorded in [architecture.md](architecture.md).

## Two kinds of gate

| Kind | Gates | What the evidence decides | What stays mandatory |
|---|---|---|---|
| Product contract | `G-001`–`G-012`, `G-014` | A platform floor, ownership model, boundary language, default identity or policy, public naming or ABI baseline, packaging profile, or safety ceiling | The gate's required evidence for the scope it blocks, and the resulting contract's tests |
| Empirical target | `G-013` | Workload-specific numerical latency, memory and throughput targets, and any explicitly requested performance claim | Each benchmark's correctness oracles, enforced resource limits, identity checks and owned cleanup, which are product contracts rather than targets |

An empirical target guides tuning and states what an explicit performance claim
must meet. Missing, noisy or failed numerical evidence leaves that claim
unsupported and recorded; it does not block implementation, tuning, integration
or a phase's correctness exit. It never relaxes a correctness, safety,
compatibility or support check. See [performance.md](performance.md).

## How a gate is used

Before entering or exiting a phase, check every gate whose **Due** column names
that phase. A phase must not exit while a gate that blocks its exit is open, and
a phase must not begin implementing behavior that an open product-contract gate
blocks.

Resolving a gate requires, in one change:

1. The evidence named in the gate, adequate for the gate's stated scope and
   committed or referenced from a tracked location.
2. A synchronized update to [architecture.md](architecture.md) or the owning topic
   guide, and to any affected test, policy, profile, or example.
3. An update to this registry marking the gate resolved with its current
   destination, or removing the gate when its claim was withdrawn instead.
4. A short [decision record](adr/README.md) when the resolution makes an important
   architectural or technical choice, such as a platform floor, an ownership or
   compatibility model, a backend or technology choice, a public naming rule, or
   a packaging policy. A numerical benchmark value alone is recorded in
   [tuning history](performance/tuning/README.md), not in a decision record.

A gate is never presented as resolved on the strength of a plan, an opinion, or a
successful build alone.

Qualification controls support claims, not integration. An implementation may
merge into its development branch when its implemented contracts, required
checks and reviews pass and tracked documentation states any missing or failed
qualification and withheld support. Merging never turns a failed or missing
qualification into support.

Gates resolved before source `646369fad6c922ac7b064367817337a18e572d8c` were
recorded in numbered decision records since retired from the product tree. The
**History** links below open those records at that source. They explain how a
gate was resolved; the current rule is the one stated here and in the linked
contract.

## Status of Phase 0

No gate blocks Phase 0. Phase 0 is nonetheless incomplete until every gate below
has an explicit due phase, blocking scope, and resolution rule, because that
registry is itself a Phase 0 deliverable.

## Registry

| ID | Decision | Kind | Due | Blocks | Current destination | Status |
|---|---|---|---|---|---|---|
| [`G-001`](#g-001) | Minimum Windows and macOS versions | Contract | Before Phase 2 exit | Windows and macOS support claims | [Release targets](architecture.md#release-targets) | Resolved |
| [`G-002`](#g-002) | Windows capture producer-pool and frame-detachment strategy | Contract | Before Phase 2 implementation | Windows capture ownership | [Windows native capture ownership](architecture.md#windows-native-capture-ownership) | Resolved |
| [`G-003`](#g-003) | macOS shim language | Contract | Before Phase 2 implementation | macOS shim implementation | [The macOS native boundary](architecture.md#the-macos-native-boundary) | Resolved |
| [`G-004`](#g-004) | Default OCR model profile | Contract | Before Phase 3 implementation | Default OCR profile identity | [OCR recognition](architecture.md#ocr-recognition) | Resolved |
| [`G-005`](#g-005) | Default change-detection algorithm and threshold | Contract | Before Phase 4 watcher implementation | Default watcher policy | [Bounded template-presence queries](architecture.md#bounded-template-presence-queries) | Resolved |
| [`G-006`](#g-006) | Acceleration candidates and provider ordering | Contract | Before Phase 5 implementation | Acceleration defaults and accelerator support claims | [OCR recognition](architecture.md#ocr-recognition) | Open per future accelerator; v0.3.1 OCR providers resolved |
| [`G-007`](#g-007) | Native dependency bundling profiles | Contract | Before Phase 5 implementation | Release packaging | [Third-party dependencies](third-party-dependencies.md) | Open |
| [`G-008`](#g-008) | Static-library feasibility | Contract | Before Phase 5 exit | Static artifact claim only | [C ABI](c-abi.md) | Open |
| [`G-009`](#g-009) | Stable public Rust item names | Contract | Before Phase 1 exit | Rust stability promise | [Public naming baseline](architecture.md#public-naming-baseline) | Resolved |
| [`G-010`](#g-010) | Version-one C ABI status, prefix, and layout | Contract | Before Phase 1 exit | ABI compatibility baseline | [Released prefixes](c-abi.md#released-prefixes-and-compatibility) | Resolved |
| [`G-011`](#g-011) | Native-frame extension discovery | Contract | Future roadmap | Does not block version one | [The opaque frame-storage seam](architecture.md#the-opaque-frame-storage-seam) | Deferred |
| [`G-012`](#g-012) | Published Cargo and C build profiles | Contract | Before Phase 5 implementation | Release capability matrix | [Third-party dependencies](third-party-dependencies.md) | Open |
| [`G-013`](#g-013) | Numeric benchmark targets | Empirical | When a workload is introduced or a performance claim is made | The corresponding explicit performance claim or opt-in enforcement only | [Performance](performance.md) and [benchmark profiles](benchmarks/) | Open per workload |
| [`G-014`](#g-014) | Archive safety ceilings | Contract | Before Phase 1 implementation | Version-one archive loading | [Safety ceilings](architecture.md#safety-ceilings) | Resolved |

## G-001

**Decision.** Windows 11 25H2 build family 26200 on a currently serviced x64
desktop installation is the Windows floor. Apple Silicon macOS 26.5.2 (25F84),
SDK 26.5, is the macOS floor. Earlier versions are unsupported.

**Required evidence.** Per target: build, process load, SDK, API availability,
unsupported-capability behavior, and native Rust, C and C++ flows. On macOS,
deployment metadata, controlled linkage, permissioned live capture placement,
cross-scale movement and owned-window replacement.

**Due.** Before Phase 2 exit. **Blocks.** The Windows and macOS support claims and
release.

**Status.** Resolved. The current floors, verification hosts and OS support policy
are in [Release targets](architecture.md#release-targets).

**Evidence.** [Windows minimum system](evidence/g-001/windows-minimum-system.md):
Pro build `26200.9168`, SDK `10.0.26100.0`, source `834a58f` passed build,
lazy-import, native Rust, product-DLL, C/C++, frozen ABI 1.0, ownership, fixture
and CMake rows; repair source `c6ff39a` loaded Rust, C and C++ with one
WinRT-D3D export suppressed and reported typed `Unsupported` discovery. On macOS,
a 2026-08-01 permissioned ASan run passed 95 library tests; the repaired
movement probe passed 2/2 over 4,097 frames, 30 cross-scale moves advancing
epochs 0 through 30; a post-repair ASan build passed 101 library tests; and the
[owned-window replacement probe](evidence/g-001/macos-owned-window-replacement.md)
closed the replacement item.

**History.** [ADR 0019](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0019-windows-qualified-system-and-controlled-availability.md)
and [ADR 0014](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0014-macos-qualified-host-and-frame-placement.md)
(historical).

## G-002

**Decision.** The Windows Graphics Capture producer pool holds two frames. The
callback copies each publishable frame into an Adapter-owned D3D11 texture and
releases the WGC frame before publication. A private texture is reused only after
every public-frame, mapping and backend lease releases it.

**Required evidence.** A retained-frame stress prototype showing that published
frames do not pin producer slots capture progress needs, and that native resources
outlive in-flight mapping and backend work.

**Due.** Before Phase 2 implementation. **Blocks.** Windows capture ownership. It
does not by itself make native Windows capture an implemented capability.

**Status.** Resolved. The production contract is
[Windows native capture ownership](architecture.md#windows-native-capture-ownership);
its acceptance suite is [windows-capture-contract-tests.md](windows-capture-contract-tests.md).

**Evidence.** The [G-002 record](evidence/g-002/README.md) compared four ownership
candidates at pool sizes two to four with retained mapping, backend lifetime,
resize, 100 close races, target close, injected reset and two 4K displays. Direct
WGC retention stalled every producer pool and blind texture reuse changed retained
content; lease-aware detachment at pool size two passed every hard gate.

**History.** [ADR 0013](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0013-windows-capture-frame-detachment.md)
(historical).

## G-003

**Decision.** The macOS shim is Objective-C with Automatic Reference Counting,
compiled with `-fobjc-arc-exceptions`, behind one internal C-callable surface with
the containment rules in [The macOS native boundary](architecture.md#the-macos-native-boundary).

**Required evidence.** A prototype covering exception behavior across the language
boundary, object ownership, and build integration on Apple Silicon.

**Due.** Before Phase 2 implementation. **Blocks.** macOS shim implementation.

**Status.** Resolved and implemented in `mado-pilot-platform-macos`, whose tests
enforce the containment, ownership, autorelease, fence, teardown, panic and
linkage rules.

**Evidence.** [evidence/g-003/](evidence/g-003/README.md) built one file as
Objective-C, Objective-C with `-fobjc-arc-exceptions`, and Objective-C++, and ran
eighteen cases on each. All three contained every injected exception. Without the
flag, ARC leaks the retained object on an exception's unwind edge; with it,
Objective-C matched Objective-C++ without requiring libc++ in every consuming
process.

**History.** [ADR 0012](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0012-macos-shim-language-and-containment.md)
(historical).

## G-004

**Decision.** The default OCR profile is
`g-004-rapidocr-ppocrv4-det-v6-rec-small-v1`: RapidOCR v3.9.2's PP-OCRv4 mobile
detector plus PP-OCRv6 small recognizer, with exact bytes, digests, vocabulary,
language, preprocessing/decoder, normalization, ordering, confidence, license and
caller-provided deployment metadata.

**Required evidence.** The immutable v3 quality fixture under the independently
reviewed v5 evaluator reproduces exact text, region count, source-relative
geometry, ordering, confidence validity and stable outcomes on both release
targets, with matching fixture, tool, code, session, vocabulary and license
identities.

**Due.** Before Phase 3 implementation. **Blocks.** The default OCR profile
identity. Backend, packaging, public-surface and support work are separate.

**Status.** Resolved. The profile and its loading boundary are in
[OCR recognition](architecture.md#ocr-recognition) and
[the G-004 dependency record](third-party-dependencies.md#g-004-accepted-ocr-profile).

**Evidence.** [evidence/g-004/](evidence/g-004/): four fresh v5 candidate
processes per release target; only `ppocrv4-det-v6-rec-small` passes all 42
regions and every identity row, and every deterministic candidate and image gate
matches across targets. Later approved-host reruns of the integrated backend
reproduced every accepted and rejected category.

**History.** [ADR 0033](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0033-default-ocr-model-profile.md)
(historical).

## G-005

**Decision.** `exact-rgba-v1` for compatible already-mapped RGBA8 regions, with
`analysis-always-v1` as the explicit fail-safe. Changed-pixel thresholds and
fixed-grid sampling are evaluation-only, not runtime options.

**Required evidence.** Repository-owned Apache-2.0 sequences freezing nine ordered
transitions, a pre-observation candidate/threshold contract, and a canonical
report showing which candidate admits the least work with zero false skips.

**Due.** Before Phase 4 watcher implementation. **Blocks.** The default watcher
policy, not the offline evaluator or the closed policy descriptor seam.

**Status.** Resolved. The watcher's use of the policy is in
[Bounded template-presence queries](architecture.md#bounded-template-presence-queries).
Any future false skip or behavior-affecting fixture, report or policy drift
removes current authority until an affected-host rerun or reviewed complete-diff
applicability restores it.

**Evidence.** [evidence/g-005/](evidence/g-005/): exact RGBA reports zero false
skips, six admitted analyses and three compatible skips; every lower-work
candidate has at least one false skip. At revision `3bb191d`, protected run
`33036793853` reproduced canonical report v2 byte for byte on both release
targets, and independent review returned CLEAN.

Native watcher support is a separate support decision, documented in
[native template watching](native-template-watch.md#qualification-v2-protocol-and-migration).

**History.** [ADR 0050](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0050-change-detection-default.md)
(historical).

## G-006

**Decision for v0.3.1 OCR.** Provider selection is independent of the model and
preprocessing profile. Existing and automatic constructors use CPU on both
release targets. Explicit Windows CUDA is supported only for the qualified
source, target, runtime and dependency boundary; CoreML is not release-qualified.
Preferred policy may construct one fresh CPU pair only before engine publication;
required policy publishes no engine on failure. No inference, cancellation,
deadline, device or native error switches provider.

**Required evidence for a later claim.** Target-native correctness, placement,
memory, cancellation, cleanup and fallback evidence for the claimed accelerator,
plus performance evidence appropriate to any benefit the claim states.

**Due.** Before Phase 5 implementation. **Blocks.** Acceleration defaults and
accelerator support claims.

**Status.** The v0.3.1 OCR provider row is resolved; the current policy is in
[OCR recognition](architecture.md#ocr-recognition). The gate stays open for
future accelerator kinds, graphs, targets or automatic defaults. Changing
provider ordering or an automatic default is a technology choice and needs a
decision record.

**History.** CoreML was rejected after geometry and cancellation hard-gate
failures; explicit CUDA passed the successor five-CUDA/five-fallback matrix, and
automatic selection stayed CPU because CUDA resident memory exceeded the fixed
ratio rule. See ADRs
[0046](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0046-onnx-accelerator-provider-policy.md),
[0047](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0047-coreml-ocr-provider-qualification.md),
[0048](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0048-cuda-ocr-provider-qualification.md)
and
[0049](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0049-cuda-ocr-memory-remediation.md)
(historical).

## G-007

**Unresolved decision.** Which broader native deployment profiles can be
published, with complete dependency ownership, controlled loading, and
license/notice obligations.

**Required evidence.** Clean-system package prototypes on both release targets and
a license review of every redistributed artifact. Development setup or hosted CI
cannot supply this proof.

**Due.** Before Phase 5 implementation. **Blocks.** Release packaging.

**Status.** Open. The current source-only project redistributes no native
libraries or models. Developers own OpenCV, libclang and their compiler/SDK and
configure them with `tools/setup-native.py`; see
[native development prerequisite ownership](architecture.md#native-development-prerequisite-ownership).
That development policy does not close this gate.

**History.** The original
[five-candidate protocol and results](evidence/native-release-profiles/README.md)
remain frozen, including development-host inventories, failed attempts and the
direct-linked missing-OpenCV pre-entry failure. They are not passing release
evidence. OpenCV 4.14.0 was a development prerequisite, not a release profile;
see [the dependency policy](third-party-dependencies.md#opencv).

**Resolution.** A decision record for the profile matrix, controlled library
search paths, and license and notice obligations, with updates to
[third-party-dependencies.md](third-party-dependencies.md) and packaging
documentation.

## G-008

**Unresolved decision.** Whether a static library is feasible for each advertised
dependency combination.

**Required evidence.** Link results and license review for each combination that a
static artifact would advertise.

**Due.** Before Phase 5 exit. **Blocks.** The static artifact claim only. It does
not block the shared-library release.

**Status.** Open.

**Resolution.** A decision record that either records the supported static
combinations with their evidence, or withdraws the static artifact claim.

## G-009

**Decision.** The reviewed public Rust item names of the facade are the `0.x`
baseline, reviewed rather than stable. Adding a name is free. Renaming or
removing one is a breaking change that needs a decision record and a version
bump. Changing what a name means while keeping it is refused. The stability
promise begins at 1.0.

**Required evidence.** The Phase 1 vertical slice with working Rust examples, and
an interface review of the names they exercise.

**Due.** Before Phase 1 exit. **Blocks.** Any Rust API stability promise.

**Status.** Resolved. The policy is in
[Public naming baseline](architecture.md#public-naming-baseline).

**History.** The review renamed six items, added four, aligned closed-session
search status across Rust and C, and kept `StreamId`/`TargetId` opaque and
`REQUIRED_BACKEND` a `&str` with recorded reasons. See
[ADR 0006](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0006-public-rust-names-and-compatibility-policy.md)
(historical).

## G-010

**Decision.** C ABI major 1, minor 0 is frozen: thirteen status values with fixed
numbers, a forty-byte mandatory table prefix ending at `status_text`, twenty-three
structures with fixed sizes, alignments and offsets, and a Rust-to-C status
mapping with one arm per `Status`. `MADOPILOT_STATUS_INTERNAL_PANIC` is the only
C-only value, and a `Status` added to Rust later reports as
`MADOPILOT_STATUS_INTERNAL`.

**Required evidence.** The Phase 1 C ABI exercised by C and C++ examples, with
owned-handle lifecycle, structure-size negotiation and error-ownership tests.

**Due.** Before Phase 1 exit. **Blocks.** The ABI compatibility baseline and every
later old-header-prefix compatibility claim.

**Status.** Resolved. Released prefixes and their compatibility rules are in
[the C ABI guide](c-abi.md#released-prefixes-and-compatibility); per-field reports
are in [evidence/c-abi/](evidence/c-abi/). The frozen caller at
`crates/bindings/capi/tests/abi-compat/v1/` compiles against the 1.0 header and
runs against the current library.

**History.** [ADR 0007](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0007-phase-1-c-abi-freeze.md)
(historical).

## G-011

**Unresolved decision.** How a native-frame extension table is discovered and
allocated in the C ABI.

**Required evidence.** A portable extension prototype covering discovery,
allocation, and version negotiation.

**Due.** Future roadmap. **Blocks.** Nothing in version one.

**Status.** Deferred. Activating it is an ABI decision and needs a decision record.

**Resolution.** A decision record for the prototype and negotiation rule, plus a
scope update that moves the feature out of the future roadmap.

## G-012

**Unresolved decision.** The published Cargo feature defaults and the C build
profiles.

**Required evidence.** A feature-matrix build across both release targets with
binary-size measurements for each profile.

**Due.** Before Phase 5 implementation. **Blocks.** The release capability matrix.

**Status.** Open. `tools/setup-native.py` configures the existing development
build; it selects no Cargo/C profile and changes no CPU default, provider scope,
Rust API or ABI 1.5 contract. A passing setup or build is not the published
feature, capability and size matrix. The frozen
[five-candidate protocol](evidence/native-release-profiles/protocol.json) and its
results do not close this gate or `G-007`.

**Resolution.** A decision record for the matrix and chosen defaults, plus a
build-profile capability table published with the release.

## G-013

**Decision.** The numerical latency, memory, mapped-byte, stale-work, model-load
and startup targets for each performance-sensitive workload, and the evidence an
explicit performance claim must meet.

**Required evidence.** For a target: scoped measurements recorded with the profile
format in [performance.md](performance.md) on the release target the workload runs
on. For a performance claim: evidence appropriate to the claim's stated workload,
host, source and statistic.

**Due.** When a workload is introduced, and before a performance claim is made.

**Blocks.** Only the corresponding explicit performance claim or opt-in
`--enforce-budgets` result. It does not block implementation, tuning, integration
or a phase's correctness exit.

**Status.** Open per workload. Each workload's hard checks — zero oracle failures,
bounded allocation growth, enforced resource and mapping limits, identity
bindings and owned cleanup — remain mandatory wherever its benchmark runs.
Numerical targets live in [benchmark profiles](benchmarks/). Changing a target is
a tuning change recorded in [tuning history](performance/tuning/README.md); it
applies prospectively and never regrades an earlier run.

**History.** These outcomes keep their original sources and verdicts.

- Phase 1: thirteen workloads, 200 samples after 20 warmups on Apple M1 Pro and
  Core i7-12700KF, zero oracle failures and zero growth; latency ceilings are
  regression ceilings at three times the measured value
  ([ADR 0008](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0008-phase-1-performance-budgets.md);
  `benchmarks/phase-1-*`).
- Phase 2: diagnostic, native input and public-language, Windows controlled,
  1280×720 and dual-4K capture, and macOS production capture profiles were
  accepted with their own sources; the earlier macOS `native-phase2`
  capture/transition profiles were invalidated and stay `normative = false`
  (ADRs
  [0020](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0020-phase-2-native-performance-budgets.md),
  [0021](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0021-invalidate-phase-2-native-performance-evidence.md),
  [0024](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0024-input-diagnostic-performance-budgets.md),
  [0025](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0025-macos-native-input-performance-budgets.md),
  [0026](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0026-windows-native-and-diagnostic-performance-budgets.md),
  [0028](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0028-windows-window-message-performance-budgets.md),
  [0030](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0030-macos-production-capture-performance-budgets.md),
  [0031](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0031-windows-1280-production-capture-performance-budgets.md),
  [0032](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0032-windows-dual-4k-production-capture-performance-budgets.md)).
- Phase 3 default OCR: five final processes per target passed their own target
  ceilings
  ([ADR 0037](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0037-phase-3-ocr-performance-budgets.md)).
- v0.3.1 bounded-detector, grouped-zone and provider OCR: the rectangular
  candidate was rejected; candidate v2 singular and grouped target budgets were
  accepted; the Apple integrated cache-cold startup row was corrected after a
  retained failure; the CUDA profiles stay distinct from CPU (ADRs
  [0038](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0038-bounded-detector-sampling-profile.md)–[0041](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0041-bounded-detector-target-budgets.md),
  [0044](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0044-integrated-zone-ocr-target-budgets.md),
  [0045](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0045-integrated-ocr-cache-cold-startup-budget.md),
  [0049](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0049-cuda-ocr-memory-remediation.md)).
- Phase 4 template watching: replay/OpenCV and native regression profiles were
  accepted with startup timing and single-run gate timing withheld
  (ADRs
  [0051](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0051-template-watch-query-replay-budgets.md)–[0053](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0053-native-template-watch-budgets.md)).
  Native watcher support rests on semantic contracts, not these profiles.
- Rust OCR text queries: target-specific ceilings for the fixed CPU bounded-v2
  workloads were accepted from complete precursors; final enforcement recorded
  Apple passes and a Windows real-workload failure
  ([ADR 0075](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0075-ocr-text-watch-workload-profiles.md)).
  Their format-2 profiles are read-only history; current format-3 profiles are
  described in [OCR text queries](ocr-text-watch.md#model-free-workload-apparatus).
- OCR model comparisons: both final comparisons at `cf2763a` completed with
  candidate quality failures and no numerical or operational failure; see
  [the OCR model catalog](ocr-model-catalog.md#final-comparison-at-cf2763a).
- v0.5.0 component and actual-consumer measurement history is summarized in
  [performance.md](performance.md).

## G-014

**Decision.** The asset archive container is ZIP restricted to `Stored` and
`Deflated`; the manifest is strict UTF-8 JSON. Six implementation ceilings bound
loading: 4 MiB manifest, 4,096 entries, 64 MiB per entry, 512 MiB total
uncompressed, 256 MiB total compressed, and a compression ratio of 64. A caller
may lower a limit and may not raise one.

**Required evidence.** Adversarial fixtures covering traversal, links, special
files, duplicate normalized entries, and decompression bombs, each rejected
deterministically at the chosen ceilings.

**Due.** Before Phase 1 implementation. **Blocks.** Version-one archive loading.

**Status.** Resolved. The ceilings and staged enforcement are in
[Safety ceilings](architecture.md#safety-ceilings). Raising a ceiling is a
resource-contract change, not tuning.

**Evidence.** [evidence/g-014](evidence/g-014/) and
[../fixtures/assets/g-014](../fixtures/assets/g-014/). The conformance suite
asserts the failure category and refusing stage for every tracked adversarial
fixture on both release targets.

**History.** [ADR 0001](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0001-asset-archive-container-and-safety-ceilings.md)
(historical).
