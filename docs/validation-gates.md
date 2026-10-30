# Validation gates

A validation gate is a version-one decision left open until the evidence needed
to settle it exists. Each entry records the decision, required evidence, due
phase, blocking scope, current status, and canonical contract or evidence links.

## Two kinds of gate

| Kind | Gates | What the evidence decides | What stays mandatory |
|---|---|---|---|
| Product contract | `G-001`–`G-012`, `G-014` | Platform floors, capture ownership, shim language, default model/watcher policies, accelerator rules, public naming, C ABI layout, packaging profiles, and archive safety ceilings | The gate's required evidence for the scope it blocks, plus the resulting contract tests |
| Empirical target | `G-013` | Workload-specific latency, memory, and throughput targets, plus any explicitly requested performance claim | Correctness oracles, enforced resource limits, identity checks, and owned cleanup, which remain product contracts |

An empirical target guides tuning and defines what an explicit performance claim
requires. Missing or failed numerical evidence leaves that claim unsupported
without blocking implementation, tuning, integration, or correctness verification;
see [performance.md](performance.md) for benchmark governance and profile rules.

## How a gate is used

Check every gate whose **Due** column names a phase before entering or exiting
that phase. Resolving a gate requires, in the same change:

1. Tracked evidence adequate for the gate's stated scope.
2. Synchronized updates to [architecture.md](architecture.md) or the canonical
   topic guide, plus affected tests, policies, profiles, or examples.
3. Updating this registry with the gate's resolved status and canonical
   destination (or withdrawing the claim).
4. A short [ADR](adr/README.md) when the resolution makes a consequential
   architectural or technical choice. Adjusting a numerical benchmark target
   uses a [tuning record](performance/tuning/README.md) instead of an ADR.

Qualification governs support claims rather than branch integration: an
implementation may merge into its development branch when its contracts, tests,
and reviews pass and documentation states any open qualification or withheld
support. Gates resolved before commit `646369fad6c922ac7b064367817337a18e572d8c`
retain historical links to retired `docs/adr/NNNN-*.md` files at that commit;
the current rule is defined here and in the linked canonical contract.

## Status of Phase 0

No gate blocks Phase 0 exit, provided every version-one gate below is registered
with an explicit due phase, blocking scope, and resolution rule.

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

- **Decision:** Minimum deployment floors are Windows 11 25H2 build family 26200 (serviced x64 desktop) and Apple Silicon macOS 26.5.2 (25F84, SDK 26.5). Earlier versions are unsupported.
- **Due / Blocks:** Before Phase 2 exit; blocks Windows and macOS support claims and release.
- **Status & canonical contract:** Resolved. See [Release targets](architecture.md#release-targets) and [OS support policy](architecture.md#os-support-policy) for deployment floors versus verification hosts.
- **Evidence & history:** [Windows minimum system](evidence/g-001/windows-minimum-system.md) (build `26200.9168`, SDK `10.0.26100.0`, sources `834a58f`/`c6ff39a`) and [macOS owned-window replacement](evidence/g-001/macos-owned-window-replacement.md) (plus permissioned ASan and 4,097-frame cross-scale movement verification); historical [ADR 0019](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0019-windows-qualified-system-and-controlled-availability.md) and [ADR 0014](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0014-macos-qualified-host-and-frame-placement.md).

## G-002

- **Decision:** The Windows Graphics Capture producer pool holds two frames; callbacks copy each publishable frame into an adapter-owned D3D11 texture and release the WGC frame immediately, reusing textures only after all public-frame, mapping, and backend leases drop.
- **Due / Blocks:** Before Phase 2 implementation; blocks Windows capture ownership.
- **Status & canonical contract:** Resolved. See [Windows native capture ownership](architecture.md#windows-native-capture-ownership) for the runtime contract and [windows-capture-contract-tests.md](windows-capture-contract-tests.md) for the contract test suite.
- **Evidence & history:** [evidence/g-002/README.md](evidence/g-002/README.md) (four candidates evaluated across pool sizes 2–4, resize, 100 close races, device reset, and dual-4K displays); historical [ADR 0013](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0013-windows-capture-frame-detachment.md).

## G-003

- **Decision:** The macOS shim uses Objective-C with ARC and `-fobjc-arc-exceptions` behind one internal C-callable boundary, avoiding Objective-C++ and `libc++` linkage requirements while preventing unwind leaks.
- **Due / Blocks:** Before Phase 2 implementation; blocks macOS shim implementation.
- **Status & canonical contract:** Resolved in `mado-pilot-platform-macos`. See [The macOS native boundary](architecture.md#the-macos-native-boundary) for containment, `@autoreleasepool`, callback fencing, and controlled ScreenCaptureKit loading.
- **Evidence & history:** [evidence/g-003/README.md](evidence/g-003/README.md) (18 exception and ownership cases across three compilation modes); historical [ADR 0012](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0012-macos-shim-language-and-containment.md).

## G-004

- **Decision:** The default OCR profile is `g-004-rapidocr-ppocrv4-det-v6-rec-small-v1` (RapidOCR v3.9.2 PP-OCRv4 mobile detector + PP-OCRv6 small recognizer) with pinned digests, vocabulary, normalization, ordering, and caller-provided deployment paths.
- **Due / Blocks:** Before Phase 3 implementation; blocks default OCR profile identity.
- **Status & canonical contract:** Resolved. See [OCR recognition](architecture.md#ocr-recognition) for runtime semantics and [the G-004 dependency record](third-party-dependencies.md#g-004-accepted-ocr-profile) for model provenance.
- **Evidence & history:** [evidence/g-004/](evidence/g-004/) (only `ppocrv4-det-v6-rec-small` passed all 42 regions and deterministic cross-target checks); historical [ADR 0033](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0033-default-ocr-model-profile.md).

## G-005

- **Decision:** Default template-watcher change detection uses `exact-rgba-v1` over compatible mapped RGBA8 regions with `analysis-always-v1` as the explicit fail-safe; threshold and grid-sampling candidates remain offline evaluation-only.
- **Due / Blocks:** Before Phase 4 watcher implementation; blocks default watcher policy.
- **Status & canonical contract:** Resolved. See [Bounded template-presence queries](architecture.md#bounded-template-presence-queries) for watcher semantics and [native template watching](native-template-watch.md#qualification-v2-protocol-and-migration) for native qualification rules.
- **Evidence & history:** [evidence/g-005/](evidence/g-005/) (zero false skips across nine transitions, reproduced byte-for-byte on both targets at `3bb191d`); historical [ADR 0050](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0050-change-detection-default.md).

## G-006

- **Decision:** Existing and automatic OCR constructors default to CPU on both targets. Explicit Windows CUDA is supported for its qualified boundary; CoreML is not release-qualified. `Preferred` policy may fall back to a fresh CPU pair only during engine initialization; `Required` policy fails closed; runtime inference never switches providers.
- **Due / Blocks:** Before Phase 5 implementation; blocks acceleration defaults and accelerator support claims.
- **Status & canonical contract:** Resolved for v0.3.1 OCR providers in [OCR recognition](architecture.md#ocr-recognition); remains open for future accelerators, graphs, targets, or automatic defaults (which require an ADR and target-native correctness, memory, cancellation, and performance evidence).
- **Evidence & history:** Historical ADRs [0046](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0046-onnx-accelerator-provider-policy.md)–[0049](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0049-cuda-ocr-memory-remediation.md).

## G-007

- **Unresolved decision:** Which native deployment profiles can be published with complete dependency ownership, controlled library loading, and license/notice compliance.
- **Due / Blocks:** Before Phase 5 implementation; blocks release packaging.
- **Status & canonical references:** Open. The repository is source-only and redistributes no native libraries or models; developer setup via `tools/setup-native.py` ([native development prerequisite ownership](architecture.md#native-development-prerequisite-ownership), [OpenCV policy](third-party-dependencies.md#opencv)) and the frozen [five-candidate investigation](evidence/native-release-profiles/README.md) do not close this gate. Resolution requires clean-system prototypes on both targets, license review, and an ADR.

## G-008

- **Unresolved decision:** Whether static library linkage is feasible and license-compliant for advertised dependency combinations.
- **Due / Blocks:** Before Phase 5 exit; blocks static-library artifact claims only (does not block shared-library or source releases).
- **Status:** Open. Resolution requires link/license evidence and an ADR recording supported combinations or withdrawing the static claim.

## G-009

- **Decision:** Reviewed `0.x` public Rust names in `mado-pilot` allow additive extensions while treating renames, removals, or semantic changes as breaking changes requiring a version bump and ADR; full API stability begins at 1.0.
- **Due / Blocks:** Before Phase 1 exit; blocks Rust API stability promises.
- **Status & canonical contract:** Resolved in [Public naming baseline](architecture.md#public-naming-baseline); historical [ADR 0006](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0006-public-rust-names-and-compatibility-policy.md).

## G-010

- **Decision:** C ABI major 1, minor 0 froze 13 status codes, a 40-byte mandatory table prefix ending at `status_text`, and 23 fixed-layout structures, with later minor versions appending only.
- **Due / Blocks:** Before Phase 1 exit; blocks the C ABI compatibility baseline and frozen-prefix claims.
- **Status & canonical contract:** Resolved. See [Released prefixes and compatibility](c-abi.md#released-prefixes-and-compatibility) for ABI 1.0/1.2/1.3/1.4/1.5 rules, [evidence/c-abi/](evidence/c-abi/) for layout reports, and `crates/bindings/capi/tests/abi-compat/` for frozen caller verification; historical [ADR 0007](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0007-phase-1-c-abi-freeze.md).

## G-011

- **Unresolved decision:** How native-frame extension tables are discovered, allocated, and version-negotiated in the C ABI.
- **Due / Blocks:** Future roadmap; does not block version one ([The opaque frame-storage seam](architecture.md#the-opaque-frame-storage-seam)).
- **Status:** Deferred. Activating it requires a portable prototype and an ADR.

## G-012

- **Unresolved decision:** Published Cargo feature defaults and C build profiles, backed by cross-target feature-matrix builds and binary-size measurements.
- **Due / Blocks:** Before Phase 5 implementation; blocks the published release capability matrix.
- **Status:** Open. `tools/setup-native.py` configures development builds only, and the frozen [five-candidate protocol](evidence/native-release-profiles/protocol.json) does not close this gate. Resolution requires an ADR and a published build-profile capability table.

## G-013

- **Decision:** Workload-specific numerical latency, memory, mapped-byte, stale-work, model-load, and startup targets, plus the evidence required for explicit performance claims.
- **Due / Blocks:** Due when a workload is introduced or a performance claim is made; blocks only that explicit performance claim or opt-in `--enforce-budgets` check.
- **Status & canonical references:** Open per workload. Enforced correctness and resource-bound checks remain mandatory everywhere, while empirical targets live in [benchmark profiles](benchmarks/) and prospective adjustments are recorded in [tuning history](performance/tuning/README.md). See [performance.md](performance.md) for current measurement methods and historical summaries across:
  - **Phase 1–4 & v0.3.1 profiles:** Frozen under [benchmarks/](benchmarks/) with original verdicts (historical ADRs [0008](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0008-phase-1-performance-budgets.md), [0020](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0020-phase-2-native-performance-budgets.md)–[0032](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0032-windows-dual-4k-production-capture-performance-budgets.md), [0037](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0037-phase-3-ocr-performance-budgets.md)–[0045](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0045-integrated-ocr-cache-cold-startup-budget.md), [0049](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0049-cuda-ocr-memory-remediation.md), [0051](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0051-template-watch-query-replay-budgets.md)–[0053](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0053-native-template-watch-budgets.md)).
  - **OCR text queries & model catalog:** Format-3 workload profiles in [OCR text queries](ocr-text-watch.md#model-free-workload-apparatus) (succeeding read-only format-2 profiles from historical [ADR 0075](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0075-ocr-text-watch-workload-profiles.md)) and model evaluations in [the OCR model catalog](ocr-model-catalog.md#final-comparison-at-cf2763a).
  - **v0.5.0 component & consumer evaluations:** Summarized in [performance.md](performance.md#historical-results-and-applicability) and [tuning history](performance/tuning/README.md).

## G-014

- **Decision:** Asset archives use ZIP (`Stored`/`Deflated`) with strict UTF-8 JSON manifests and six enforced safety ceilings: 4 MiB manifest, 4,096 entries, 64 MiB per entry, 512 MiB total uncompressed, 256 MiB total compressed, and 64× compression ratio (callers may lower limits only).
- **Due / Blocks:** Before Phase 1 implementation; blocks version-one archive loading.
- **Status & canonical contract:** Resolved in [Safety ceilings](architecture.md#safety-ceilings). Raising any ceiling is a resource-contract change rather than empirical tuning.
- **Evidence & history:** [evidence/g-014/](evidence/g-014/) and [../fixtures/assets/g-014](../fixtures/assets/g-014/) (24 adversarial fixtures verified by `g014_conformance.rs`); historical [ADR 0001](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0001-asset-archive-container-and-safety-ceilings.md).
