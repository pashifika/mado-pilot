# Phase 4 native template-watch qualification

This record supports the Rust native watcher boundary accepted by [ADR 0057](../adr/0057-native-template-watch-rust-support.md). It preserves the non-sensitive final aggregate; raw process streams and rejected diagnostic artifacts remain in the change evidence store and are not copied into tracked documentation.

## Identity and applicability

The five-process Apple and Windows final cohorts executed from source `367b32473eb9e053165380ad9c4877850dc6191b`, tree `fdb4c0c4997cdbe6d3cb7e72c7b471a9f6758c27`, after ADR 0053 and both target profiles were frozen. Both used `native-watch-control-v1`, OpenCV 4.14.0, the identical 24-workload semantic registry, three warmups and 20 measured samples for 16 workloads, eight single-run gates, and five fresh processes without process retry, exclusion, reorder, extra priming, or sample replacement.

The post-cohort base-merge applicability anchor is `dd07009c1af3c7ed244e393b0a0ccd6d5f930ee0`, tree `3e76c5a07b2554445bef824f524d193bcffb38a0`. The complete `367b324..dd07009` diff changes only `.github/workflows/rust.yml`, `docs/adr/0054-g004-model-download-fallback.md`, and `docs/third-party-dependencies.md`; its Git diff SHA-256 is `2c60852bcd2cae5778d3dd2032b6460c12cfbf6278a01f71eaee8abf9b393b6c`. A fresh isolated Apple benchmark build from `dd07009` reproduced the executed cohort's optimized executable byte-for-byte. The support-promotion successor that contains this record adds documentation and the standalone Rust example only: no Cargo manifest, library, fixture, benchmark, profile, budget, workload registry, or test source changes. Its exact revision/tree, repeated executable-identity proof, review, and hosted checks remain protected-delivery evidence rather than a self-referential hash in this file.

The revision-bound precursor hashes and measurements in both profile files remain attached to their actual source. Final executable hashes in this record do not replace those frozen historical fields.

## Final cohorts

| Target | Approved host | Executable SHA-256 | Fixture SHA-256 | Result |
|---|---|---|---|---|
| `aarch64-apple-darwin` | Apple M1 Pro, macOS 26.6.2 build 25G83, SDK 26.5, Rust 1.97.1/LLVM 22.1.6 | `97db3d1588957cbda7e805f8aeae713a2bb907e27de4ba4e46536b802d530f15` | `4591eb891a93e133be7f9b7f5d55007618809cc72ee99e198ec56fe92a94fdfe` | 5/5 processes and 120/120 rows passed |
| `x86_64-pc-windows-msvc` | Core i7-12700KF, qualified Windows 11 25H2 build family 26200, Rust 1.97.1/MSVC 19.44.35228 | `4d9120bc20f98c532cd9b8cdc5deb43b5ba5d8e25b76d7b7983e63fe8fdb9277` | `9bea5dab2b975a696eddcd5feb72a53664c07d9cc4a948c9e2e04bd3a85069a1` | 5/5 processes and 120/120 rows passed |

Across both targets, 480/480 sampled latency checks, 3,200 sampled measurements, 80 gate measurements, every controlled semantic signature, and every source/query/work/lifecycle/ownership/cleanup/privacy oracle passed. Sampled allocation growth maximum was zero. Maximum observed budget ratios were:

| Target | Latency | Live Rust heap | Target-native resident memory |
|---|---:|---:|---:|
| Apple Silicon | 0.629666 | 0.800002 | 0.783405 |
| Windows x64 | 0.862300 | 0.799975 | 0.799013 |

Backend accounting ended inactive and conserved `runs == successful_completions + typed_failures` in every process. Apple typed-failure counts were `[21, 44, 43, 45, 44]`; Windows counts were `[0, 0, 0, 1, 0]`. These were expected typed cancellation, deadline, availability, loss, or controlled backend outcomes, and every corresponding workload oracle passed.

The final cohorts had zero oracle, sampled-budget, warning, privacy, cleanup, hang, process-retry, exclusion, reorder, extra-priming, sample-replacement, or orphan-process findings.

## Retained rejected evidence

Rejected evidence remains revision-bound rather than being rewritten as success:

- An Apple full-load diagnostic stopped at `capability_unavailable:topology` while only one public display was available. It was never a final-cohort process. The final cohort started only after the approved independent two-display topology was available.
- A Windows strict-newer diagnostic proved that an identical visible acknowledgement can be WGC-coalesced without producing a later frame. The accepted protocol now performs a deterministic pixel transition outside the marker before each required confirmation; 5,000 controlled confirmations then passed without retry or exclusion.
- Windows final process 4 passed the frozen benchmark but was initially rejected by an evidence wrapper that incorrectly required zero typed backend failures. Its original report conserved `1014 == 1013 + 1`; the unsupported wrapper conjunct was removed, the process was not rerun or replaced, and process 5 then ran once.
- Two Apple prequalification command mistakes failed before compilation and produced no executable. The one correct isolated build produced the accepted binary before any final process started.
- A hosted macOS job stopped before compilation when the original model transport returned HTTP 502. [ADR 0054](../adr/0054-g004-model-download-fallback.md) records the exact-hash-verified transport correction; it does not alter runtime bytes or native watcher evidence.

## Privacy and withheld claims

Tracked evidence excludes captured pixels or hashes, template bytes or caller template identifiers, titles, raw native/display/window/process identifiers, credentials, OCR/input text, local paths, process inventories, signing identifiers, and unrelated desktop metadata. Exact non-sensitive source, tree, binary, fixture, host class, toolchain, process, workload, aggregate work, resource, and typed-failure facts remain reviewable.

This evidence accepts only the Windows/macOS Rust maintained-session watcher boundary. It does not qualify OCR predicates, callbacks/subscriptions, C ABI/C++, automatic input, target activation, arbitrary application/template/ROI compatibility or timing, real-time guarantees, packaging, crates.io/static artifacts, a release tag, or `v0.4.0`.
