# Phase 4 native template-watch qualification

This is a historical and current rejected qualification record. It preserves non-sensitive measurements and the reasons [ADR 0057](../adr/0057-native-template-watch-rust-support.md) withholds native watcher support; raw process streams and diagnostic artifacts remain in the change evidence store.

## Identity and applicability

The Apple and Windows cohorts executed from source `367b32473eb9e053165380ad9c4877850dc6191b`, tree `fdb4c0c4997cdbe6d3cb7e72c7b471a9f6758c27`, after ADR 0053 and both target profiles were frozen. Both recorded the identical 24-workload registry and fixed process/sample policy.

Independent review later found that neither cohort exercised a live query's `SchedulerClosed` outcome after engine destruction. The Windows topology row also selected the fixture's current monitor and did not cross the approved DPI boundary. The former post-cohort applicability proof cannot repair missing executed semantics, so the `367b324` results remain revision-bound diagnostics.

Replacement source `f16591f` corrected the Windows scale oracle and retained the engine-close semantics. Its fresh Windows cohort passed 5/5 and 120/120 rows, including the different-monitor 144-to-120-DPI transition. Its fresh Apple cohort passed processes 1–4 and serialized 96 green rows, then process 5 terminated red during the `retained_result_mapping` warmup with `producer_stalled` after ScreenCaptureKit suspended the newly opened stream. Fifty focused runs and one immediate full-load diagnostic did not reproduce the suspension, but neither result replaces the terminal-red qualification process. Native support remains withheld.

The revision-bound precursor hashes and measurements in both profile files remain attached to their actual source. Final executable hashes in this record do not replace those frozen historical fields.

Corrective successor `7139a68773b2049f8eb0faa83d9f3c356db4afca`
serializes backend generation admission per query and bounds Windows fixture
output before allocation. Those runtime and harness changes make this
`f16591f` host evidence inapplicable to the current implementation. The
corrective successor has no replacement native cohort; support remains
withheld.

## Replacement-source outcome

| Target | Approved host | Executable SHA-256 | Fixture SHA-256 | Result |
|---|---|---|---|---|
| `aarch64-apple-darwin` | Apple M1 Pro, macOS 26.6.2 build 25G83, SDK 26.5, Rust 1.97.1/LLVM 22.1.6 | `5cb47e861a4b8eca2315bdb1238003be5385956095197a86e04965ea40a7e793` | `4591eb891a93e133be7f9b7f5d55007618809cc72ee99e198ec56fe92a94fdfe` | Processes 1–4 green; process 5 terminal red at `retained_result_mapping`; rejected |
| `x86_64-pc-windows-msvc` | Core i7-12700KF, Windows 11 Pro 25H2 build 26200.9168, Rust 1.97.1/MSVC 19.44.35228 | `b339cb820701e3de66c5a670a422a8314bf55af8ab8e291193df02d5adcbb759` | `af2e4e86b0a115471490f2f5fa7c1f55c1a8572945b68097afb1b0f4330f69fc` | 5/5 processes and 120/120 rows green; retained as historical target evidence, cross-target rejected |

The Apple process 1–4 maximum retained ratios were 0.635668 latency,
0.320064 live Rust heap, and 0.362330 target-native RSS. The Windows maximum
ratios were 0.873045 latency, 0.799975 heap, and 0.798921 RSS. Every serialized
row passed its correctness and target-specific budget, and backend accounting
conserved with no active work. The Apple process 5 failure occurred before
report serialization, so no missing measurements are inferred.

The qualification protocol retained five sequential Apple process launches and
five sequential Windows processes with zero retry, exclusion, filtering,
reorder, extra priming, overlap, sample replacement, or process replacement.
The Apple terminal red prevents a cross-target acceptance aggregate.

## Rejected historical cohorts

| Target | Approved host | Executable SHA-256 | Fixture SHA-256 | Result |
|---|---|---|---|---|
| `aarch64-apple-darwin` | Apple M1 Pro, macOS 26.6.2 build 25G83, SDK 26.5, Rust 1.97.1/LLVM 22.1.6 | `97db3d1588957cbda7e805f8aeae713a2bb907e27de4ba4e46536b802d530f15` | `4591eb891a93e133be7f9b7f5d55007618809cc72ee99e198ec56fe92a94fdfe` | 5/5 processes recorded; rejected after semantic review |
| `x86_64-pc-windows-msvc` | Core i7-12700KF, qualified Windows 11 25H2 build family 26200, Rust 1.97.1/MSVC 19.44.35228 | `4d9120bc20f98c532cd9b8cdc5deb43b5ba5d8e25b76d7b7983e63fe8fdb9277` | `9bea5dab2b975a696eddcd5feb72a53664c07d9cc4a948c9e2e04bd3a85069a1` | 5/5 processes recorded; rejected after semantic review |

Across both targets, the harness reported 480 sampled latency checks, 3,200 sampled measurements, 80 gate measurements, and zero failures under the then-current oracles. Those numbers do not establish the unexercised engine-close and cross-DPI topology contracts and are not support evidence. Recorded maximum budget ratios were:

| Target | Latency | Live Rust heap | Target-native resident memory |
|---|---:|---:|---:|
| Apple Silicon | 0.629666 | 0.800002 | 0.783405 |
| Windows x64 | 0.862300 | 0.799975 | 0.799013 |

Backend accounting ended inactive and conserved `runs == successful_completions + typed_failures` in every process. Apple typed-failure counts were `[21, 44, 43, 45, 44]`; Windows counts were `[0, 0, 0, 1, 0]`. These were expected typed cancellation, deadline, availability, loss, or controlled backend outcomes, and every corresponding workload oracle passed.

The reports contained no recorded oracle, sampled-budget, warning, privacy, cleanup, hang, process-retry, exclusion, reorder, extra-priming, sample-replacement, or orphan-process finding. Independent review still rejected the cohorts because their semantic gates did not cover the required behaviors.

## Retained rejected evidence

Rejected evidence remains revision-bound rather than being rewritten as success:

- An Apple full-load diagnostic stopped at `capability_unavailable:topology` while only one public display was available. It was never a final-cohort process. The final cohort started only after the approved independent two-display topology was available.
- A Windows strict-newer diagnostic proved that an identical visible acknowledgement can be WGC-coalesced without producing a later frame. The accepted protocol now performs a deterministic pixel transition outside the marker before each required confirmation; 5,000 controlled confirmations then passed without retry or exclusion.
- Windows final process 4 passed the frozen benchmark but was initially rejected by an evidence wrapper that incorrectly required zero typed backend failures. Its original report conserved `1014 == 1013 + 1`; the unsupported wrapper conjunct was removed, the process was not rerun or replaced, and process 5 then ran once.
- Two Apple prequalification command mistakes failed before compilation and produced no executable. The one correct isolated build produced the historical cohort binary before any final process started.
- A hosted macOS job stopped before compilation when the original model transport returned HTTP 502. [ADR 0054](../adr/0054-g004-model-download-fallback.md) records the exact-hash-verified transport correction; it does not alter runtime bytes or native watcher evidence.
- Independent review rejected both final cohorts because `session_engine_close` closed its only query before engine destruction.
- The Windows controller selected the fixture's current monitor under the approved left-of-primary topology, so its five topology rows did not prove a cross-display DPI transition.
- Corrected Apple attempts that stopped on retained native-session ownership, confirmed user input, or a real-clock transient race remain rejected; the fresh `c363826` cohort started only after those causes were corrected.
- The corrected `c363826` Windows final attempt proved the required different-monitor 144-to-120-DPI apparatus, then stopped at `semantic_oracle_failed:window_topology_scale`. The gate compared the invariant physical-desktop scale instead of the per-target effective-DPI scale; its first process is retained as terminal-red evidence and processes 2–5 were not launched.
- The `f16591f` Windows replacement cohort passed all five fresh processes and the corrected per-target scale oracle without reusing its disposable focused diagnostic.
- The `f16591f` Apple replacement cohort passed processes 1–4, then process 5 stopped at `producer_stalled` after a ScreenCaptureKit suspension. Post-failure diagnostics remain non-qualification evidence and do not replace that process.

## Privacy and withheld claims

Tracked evidence excludes captured pixels or hashes, template bytes or caller template identifiers, titles, raw native/display/window/process identifiers, credentials, OCR/input text, local paths, process inventories, signing identifiers, and unrelated desktop metadata. Exact non-sensitive source, tree, binary, fixture, host class, toolchain, process, workload, aggregate work, resource, and typed-failure facts remain reviewable.

This evidence does not accept the Windows/macOS native Rust watcher boundary. OCR predicates, callbacks/subscriptions, C ABI/C++, automatic input, target activation, arbitrary application/template/ROI compatibility or timing, real-time guarantees, packaging, crates.io/static artifacts, a release tag, and `v0.4.0` also remain unqualified or unavailable.
