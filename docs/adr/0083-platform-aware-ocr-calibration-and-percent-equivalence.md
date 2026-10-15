# ADR 0083: Platform-aware OCR calibration and percent equivalence

- **Status:** Accepted prospective policy; Windows numerical budgets and model admission pending
- **Date:** 2026-10-16
- **Resolves gate:** none; new-model admission under G-013 remains open
- **Supersedes:** ADR 0080's percent-sign exactness and ADR 0082's Windows reference operational allowance only

## Context

The operator requires Windows timing allowances to account for platform and
security-software differences without incremental deadline retries, and visually
indistinguishable `%` and `％` not to fail OCR quality. These are prospective
measurement policies, not changes to SDK output or model support.

The first Windows reference process at source
`b7a47943414344637bb16cc7117c8a917248da55` timed out after
`300.02602650015615` seconds. The outer process exited `2` after
`304.3387877000496` seconds. Both reported `cleanup_ok`; neither hit its output
limit. The original packet is retained at
`rasen/changes/ocr-model-support-benchmark-criteria-and-model-configuration/evidence/windows-reference-precursor-b7a4794/return-packet.zip`,
245,555 bytes, SHA-256
`fcdf857ee6fdb0da4cf612486e4815fe0a6acef4fdd3099e39e42bcd11a741ca`.

The 425 retained observations comprise 40 warmups and 385 measured samples,
with 19 complete workloads but no terminal process metrics or lifecycle rows.
All 425 report `Ok` and source correlation; that does not complete the precursor.
Original-criteria offline validation retains 23 blocking failures, 44 quality
failures and the incomplete three-process series. The quality failures compare
`次の攻撃が25%上昇` with `次の攻撃が25％上昇`.

Completed SDK elapsed spans sum to `299.5677105` seconds, including detector
inference `210.4471979` and recognizer inference `81.8929626` seconds. The
remaining `0.4583160001561737` seconds includes initialization, output and
unfinished work; it is not a measured security-software cost. No causal
attribution to security software follows from this timeout.

## Decision

Use a separately frozen, source/binary-bound Windows reference campaign with
platform-specific operational headroom. Compare only ASCII percent U+0025 and
FULLWIDTH PERCENT SIGN U+FF05 as equivalent after NFC and trim, on both observed
and expected text, in prospective benchmark quality and repeated-text stability.

### Windows reference campaign

Run exactly three fresh, serial G-004 reference processes, with the unchanged
11 images, 22 singular/grouped workloads, two warmups and twenty measured samples
per workload/process, and CPU intra/inter-op threads `1`/`1`. Retain ADR 0082's
host, runtime/resource bindings, collectors and independent receipts. Freeze the
new clean source/tree, binary digest, criteria digest, controller and commands
before any launch; do not reuse the failed process as part of the new series.

The new operational bounds are 900 seconds and 33,554,432 output bytes per
process, 15 seconds for owned cleanup, 2,000 ms per admitted interruption probe
and 3,300 seconds for the whole campaign. The outer controller reserves 3,285
seconds for primary work and 15 seconds for cleanup. Before each native launch,
require its full 900-second process allowance, 15-second cleanup allowance and
30 seconds for validation to remain within primary time.

Stop later launches on operational, input/source identity, collector or lifecycle
failure. Retain quality failures without dropping observations. This authorizes
one new campaign, not an incremental bound-increase/retry loop. Do not disable,
exclude paths from or otherwise change security software. A failed campaign
requires a separate prospective decision, not another automatic attempt.

The larger allowance is platform-specific policy headroom, not an empirical
model-performance ceiling or a claim about the timeout's cause. Derive Windows
numerical budgets only from complete observations using ADR 0082's unchanged
predeclared rules; review and commit them before final measurement. No candidate
execution, resource acquisition, rating or support promotion is authorized.
Apple's 300-second process and 1,200-second whole-precursor bounds, numerical
budget bytes and historical data remain unchanged.

### Percent comparison boundary

Apply NFC, trim, then U+FF05-to-U+0025 mapping only for benchmark comparison.
Same-implementation stability uses that same text equivalence while retaining
exact geometry, confidence, region order and group membership/order comparison.
Quality count/order, IoU at least 0.50, center deltas at most 0.025, confidence,
source correlation and lifecycle/resource gates do not change. No broad NFKC,
other width folding, case folding or fuzzy matching is permitted.

Keep `tools/ocr-models/criteria-v1.json`, its schema and criteria ID; bind the
revised semantics by its exact SHA-256 and this ADR, not by path or ID alone.
Do not rewrite raw observations, fixture authoring, original reports or their
verdicts. SDK normalization, execution/profile/catalog identities, provider and
watcher contracts, defaults and support status remain unchanged.

## Alternatives

- Keep the shared 300-second Windows allowance: prevents the observed reference
  workload from completing without establishing a Windows performance ceiling.
- Increase deadlines after each timeout or disable security software: produces
  an unfrozen campaign or changes the operating environment being evaluated.
- Rewrite expected text or use NFKC: changes authored evidence or folds unrelated
  characters beyond the operator's percent-equivalence requirement.
- Reclassify old reports under the revised criteria: loses their original
  criteria/source binding and cannot supply missing observations.

## Consequences

Maintainers must freeze a new campaign and retain the failed one separately.
The larger finite allowance increases reserved Windows execution time without
admitting a model. The original Apple 132 and Windows 44 exact-text failures
remain true under their original criteria; this decision does not rerate them.
Windows numerical budgets, a common final source, both final target reports and
a reviewed candidate-attempt disposition remain required. The catalog guide,
architecture, validation gates and contributor procedure reference this policy.

## Verification

Original offline evidence is the retained packet and raw JSONL, 2,547,511 bytes,
SHA-256 `b9ece76babcb3ac633d4b9ba6163eef4149faa8924cf45feacb5a39067467fe3`.
It establishes the incomplete timeout and original failures, not a new pass.

Before execution, focused report checks must cover percent equivalence in both
directions, NFC/trim, repeated-text stability, rejection of other character
folding, unchanged geometry/confidence/order/groups and raw-evidence integrity.
Controller checks must cover the fixed 900/3,300-second allowances, launch-time
reservation, stop-on-failure and absence of retries. Review must confirm exact
criteria/source/binary bindings and byte-identical historical evidence and Apple
budgets. These are required future checks, not claimed results. New native
observations and Windows numerical-budget acceptance remain unavailable here.
