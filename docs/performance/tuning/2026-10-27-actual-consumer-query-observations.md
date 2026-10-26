# Actual-consumer query observations

## Question and source applicability

The actual consumer can release its caller observation before waiting, return a correlated newer-frame match, and report an explicit deadline without acquiring a result. These observations extend the [group record](2026-10-26-actual-consumer-group-observation.md); they do not qualify latency, CPU or the final release source.

Apple executes one fresh baseline/current pair for each ROI case through the PR116 loader-off collector. Windows uses the already-completed corresponding pairs; no Windows native execution or current-collector verification is inferred. Both targets retain effective producers `beb064f0c652648f92df5bfb65aaf05384a18b8d` and `dd320044cdd390ee662117c69e865391f72ed996`, dev/observation-on with normal allocator. Raw `build.engine_revision` remains the authored Cargo Git pin `f61a57f661c0314ea0a1d860d9298fbff86d903d`; original local source graphs, path patches and Cargo receipts establish the effective producer identities.

The query-specific source uses authored0.9/8 defaults, ROI64/48/320/180 and `DropOverlapping`. Both producers' query-wait path calls recognition with those defaults. The later PR114 `KeepAll` row-heap optimization is inapplicable to these requests; this is not binary/timing equivalence or verification of a final release build.

## Observable contracts

Both cases first confirm no match on frame2, create one query and release the caller observation before one wait. The outcomes are intentionally different:

| Case | Requested wait | Native exit | Application result | Engine-child exit | Query result |
|---|---:|---:|---|---:|---|
| `matched-roi` |5000ms|0|PASS|0|Frame3, bounds83/66/12/10; retained identity and handles released|
| `deadline-roi` |1ms|1|FAIL / Timeout / deadline_exceeded|0|No acquired result; terminal cleanup completes|

Owner/collector exit0 means the selected contract and cleanup were verified. It does not convert deadline application failure into success, a successful terminal no-match, stream exhaustion or an outer timeout. A1ms requested deadline does not guarantee scheduling and error delivery within1ms.

All four Apple invocations and all four retained Windows invocations preserve the expected outcomes and known cleanup. Matched runs record17 host calls; deadline runs record12, with no dropped records. Original registration/seal evidence and native outcomes remain unchanged.

## Observed wait intervals

Each interval is the single `query_wait` call's end minus start on its own process-relative clock. It excludes startup/preflight and is not whole operation time. No cross-process clock arithmetic is used.

| Target/case | Baseline ns | Current ns |
|---|---:|---:|
| Apple matched |15040375|8235459|
| Apple deadline |2664792|3631084|
| Windows retained matched |12211400|6040700|
| Windows retained deadline |2261700|2691500|

These are individual development observations, not a statistical speedup or regression verdict. Deadline waits exceed the requested1ms in both roles on both targets; no quiet-host gate, retry or regrading was used.

## Resource and stage tradeoffs

Adverse observations remain part of the result:

| Observation | Current minus baseline |
|---|---:|
| Apple matched child sampled RSS |+7389184B|
| Apple matched supervisor sampled RSS |+98304B|
| Apple deadline child sampled RSS |+753664B|
| Apple deadline supervisor sampled RSS |+114688B|
| Apple deadline startup |+2143us|
| Apple deadline preflight |+6395us|
| Apple deadline whole elapsed |+277us|
| Windows matched raw Job CPU |+0.046875s|
| Windows matched startup |+1195us|
| Windows deadline raw Job CPU |+0.0625s|
| Windows deadline child sampled RSS |+147456B|

All Windows deadline application stages increase in the retained pair. Complete stage values and deltas are in the projections. Raw CPU observations are not reconciled comparable lifetime CPU: Apple remains partial (`descendant_rollup_unproven`), and Windows complete Job accounting still has unavailable effective resolution/closure. Reconciled CPU remains null. Incomplete steady-state RSS windows remain unavailable; sampled maxima are not OS high-water, allocation-domain measurements or leak evidence.

## Execution and evidence boundaries

Apple reuses each case's original campaign root and sealed discovery history. Two prepare and four smoke stages run once with separate finite clocks, six owned processor metadata calls and four actual consumer root invocations. Every stage's owner/outer succeeds with known cleanup; each baseline is inspected before its candidate, and matched is inspected before deadline. Actual report entry points preserve every original cohort file hash. These counts are owner call accounting, not a descendant PID-role census.

Windows retains the original raw receipts, management outcomes, post-write seals and673 content/1520 metadata-only guard distinction. Old provisional inspection and final post-write completion are not conflated. No historical process identity, liveness or clock is adopted by the Apple scope.

Public evidence:

- [Apple projection](../../evidence/p050-actual-consumer-query/apple-observation.json): fresh role/source bindings, wait outcomes, resource values and private receipt hashes.
- [Windows projection](../../evidence/p050-actual-consumer-query/windows-observation.json): retained archive references, query-specific source assessment, original outcomes and adverse values.

Private paths, environment values, PIDs and arbitrary logs are excluded. Original environment-bearing receipts remain local; their bytes are not asserted as receiver-verified evidence. Native-thread getters are endpoint observations, not invariant worker counts. No normal-image closure, qualified p95/CPU, capture/input permission, OCR inference, UI responsiveness, archive, tag or release is claimed. Saved-RGBA trial initialization, delivery and settlement are separate from these query waits.
