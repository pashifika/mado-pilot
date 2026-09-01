# Native template-watch current-source requalification

## Result

Rust native template watching remains `WITHHELD` for Windows WGC and macOS ScreenCaptureKit sessions. Exact integrated candidate `030398e` produced one immutable terminal-red process on each approved host; both fail-fast drivers left processes 2–5 unlaunched. Either target result independently prevents support promotion. The native APIs remain implemented; deterministic replay/OpenCV template watching remains the supported watcher boundary.

[ADR 0060](../adr/0060-current-native-template-watch-support-withheld.md) records the decision. This evidence does not replace or relabel [ADR 0057](../adr/0057-native-template-watch-rust-support.md), [ADR 0059](../adr/0059-macos-capture-start-and-fixture-cleanup.md), the historical PR #59 result, later bounded non-reproduction evidence, or any frozen benchmark section.

## Frozen candidate and authorities

| Authority | Identity |
|---|---|
| Product revision | `030398ec39f41b21d55f1f8331d07c540ed43863` |
| Product tree | `589081f739412b26c193d0304fd00da1ece46583` |
| Canonical 570-entry source inventory | `42870f9ce72391eb635fc0ac62d1025f66a0088036baba06863e347a235ed70f` |
| Frozen protocol | `6f6d91f52b5c413c791d662aa0435085ad671a39276a3acffb9b856998388266` |
| Apple qualification executable | `af3b97d17ad0cb94b4ff76d61fa99f4d2d0a22940a9d382345cfc895b3663f86` |
| Signed Apple fixture executable | `14ea7413b787a8f86f5165362af23a4470a44df94c4adc402ec7492c93435fab` |
| Apple fixture-source inventory | `6fc8910df9a1f9126281346c01c74bc02133b68b25a9068ebd3848b94aab254f` |
| Accepted Apple profile | `ef1e129aaabe65decbd0eee0d9807bf4294c11648b9764500507968e00ff4ce9` |
| Windows qualification executable | `957dc3ded250d95c8f93a6dcece8c7b160d3bcd4b283ed0a18a95be4955696bc` |
| Windows fixture executable | `29e645deb633257006c08f17469926be17141a4d8b250f0c6ef01544ad0194d6` |
| Windows fixture-source inventory | `c26c08eed7ad58b88458ca43f85e11309d5e4f4bd55deafc13dafd52c6c6eccb` |
| Accepted Windows profile | `9a8c1672d523c73b8435f4ccf27abd374ef016bc325e64c11e5f99b42e5068c9` |

The Apple preflight used an Apple M1 Pro with ten CPU cores and 32 GiB of memory, macOS 26.6.2 build 25G83 with SDK 26.5, Rust 1.97.1/LLVM 22.1.6, and OpenCV 4.14.0. The non-prompting Screen Recording probe passed. Retained topology is limited to `display_count=2` and `has_distinct_backing_scales=true`; exact display configuration and signing identifiers are not retained.

The Windows campaign used a Core i7-12700KF host with 32 GiB of memory, Windows 11 Pro 25H2 build 26200 UBR 9278 with SDK 10.0.26100.0, Rust 1.97.1/MSVC 19.44.35228, and OpenCV 4.14.0. The corrected preflight retained only `has_distinct_effective_dpi=true`; exact monitor configuration and DPI values are not retained.

## Apple formal outcome

One release executable and one private fixture were built in the frozen target directory and validated before formal execution. A single sleep-prevention lease ran the fail-fast driver.

| Process | State | Evidence |
|---|---|---|
| 1 | `terminal_red`, exit 101, 221.563179 seconds | `retained_result_mapping` recorded two fixture finalizations with control acknowledged but stop unacknowledged and bounded cleanup false; final enforcement panicked with `cleanup_failed` |
| 2–5 | Not launched | Required fail-fast suffix; no index was reused |

The process reached the fixed 24-workload order and produced a parseable diagnostic report, but a nonzero process is not green qualification evidence. The report recorded zero aggregate result-correctness, query-failure, and work-failure counts. Independent comparison against the unchanged ADR 0053 Apple profile found two additional hard failures for `retained_result_mapping`:

| Measure | Observed | Fixed limit |
|---|---:|---:|
| p95 latency | 12996.624959 ms | 7221.614 ms |
| maximum latency | 13082.141959 ms | 7343.78 ms |

Sampled allocation growth, peak live heap, and peak resident memory stayed below their accepted limits, but passing resource rows cannot override cleanup or latency failures. The cohort used zero retries, replacements, exclusions, overlap, reorder, or extra priming.

## Windows formal outcome

The corrected topology and fixture preflights passed. The formal benchmark and fixture were built once in the isolated target and identity-checked before the cohort.

| Process | State | Evidence |
|---|---|---|
| 1 | `terminal_red`, exit 101, 119.906 seconds | Typed class `privacy_violation` at workload `producer_progress_cleanup_privacy`, stage `complete`; zero report bytes |
| 2–5 | Not launched | Required fail-fast suffix; no index was reused |

The runner and extractor each ran once and returned 1 to retain the terminal-red cohort. The extractor kept only bounded failure tokens, the empty-report digest, and the stderr digest and byte count. Raw stderr remains ignored, so this Change does not infer a deeper cause from it. With no published report, no Windows measurement row can be accepted as green semantic or budget evidence. Retry, replacement, exclusion, overlap, reorder, and extra priming counts are all zero.

## Retained evidence and privacy

The allowlisted machine-readable authorities are retained in the nested planning Change `native-template-watch-current-source-requalification`. Their content digests are:

| Record | SHA-256 |
|---|---|
| Apple cohort | `a03714bd1f206c1ca6707d2c3a0eef2b99cddee7ee6facbd23d84d4bfd454138` |
| Apple validation | `d41076a62fdceca4ff08a7e1a729069a5de3455b982b5d6add8d68afea640915` |
| Windows artifact record | `d4c770f4c3618b713c4f5454d79b0e7d099f11d0d25f6edff55f54e1f5c293c2` |
| Windows artifact manifest | `0ac428d3239c3e2a7bd16d71fa23d139c52d00a81daf7ce2777e83a2850ba1dd` |
| Windows cohort | `218c420ff152c759983599b8079059ee3c6d579d2481e4803f79f75d18bb96dd` |
| Windows validation | `b53b19b62023255ccd02960a3b8acf6b4afdaae18d074151dfd6f7df0437d4df` |
| Windows cohort manifest | `6631ede15fae030a2b04e79b7c12adac26d2ab3bf0c5682e1b29ed898ed6eacd` |
| Cross-target terminal-red aggregate | `d2bbcd3b9abb93004fe583a5c003b9cf7e0a76bbf904ef8814e8c9d7887ab55b` |
| Support classification | `646bfdfcfcc22ada189231dd8c2cffbf09effa2cdba8f496945ea9929c8ddb27` |
| Prior Windows host blocker, retained as historical pre-execution observation | `bec481543df0c614dc22f42d9a10d1456115a8b07a65461205f7e962d7406049` |
| PR and protected-check deferral authority | `a4de391bd2406cd480703ee8d761dbe5c83ccb3c662396baebb12a32ebfe5881` |

Raw stdout, stderr, build products, and fixture output remain ignored. Tracked evidence contains no captured pixels or pixel hashes, template bytes or hashes, recognized or input text, titles, raw native identifiers, local paths, process inventory, signing identity, certificate identity, exact display dimensions, refresh rates, backing-scale values, or free-form native payloads.

The qualification pull request and its protected repository-policy, Windows, and
macOS checks are intentionally deferred by the operator decision recorded in
the digest-bound authority above. No protected run identity is claimed, and
their absence is not green evidence. Delivery can resume
only under fresh operator direction; it cannot change either retained
terminal-red process.

The frozen eight-file product evidence inventory remains `cf3d5e99c9a121a059f737ee0fe51297f101f3a68c833089c689d7a32c14d381`. Five planning archives were rehashed byte-identically before classification. Current documentation links to those revision-bound records; it does not rewrite them.
