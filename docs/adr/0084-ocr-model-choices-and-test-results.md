# ADR 0084: OCR model choices and test results

- **Status:** Accepted; implemented
- **Date:** 2026-10-16
- **Resolves gate:** none; replaces this Change's model-quality admission policy
- **Supersedes:** ADR 0080's quality/performance-based model admission and aggregate rating policy; its execution and resource contracts remain in force

## Context

The final Rust comparisons at `cf2763a` completed on both release targets.
The new PP-OCRv6 pair passed the recorded numerical, identity, collector and
lifecycle checks, but failed several text and negative-image rules. The
[source-bound results](../ocr-model-catalog.md#final-comparison-at-cf2763a)
retain the exact observations and original nonadmission verdicts.

The operator wants model choices with statements such as "passed this test
rule", not an SDK-imposed all-rules-pass condition for choosing a model.
A model's suitability for a caller's images cannot be decided by this corpus
alone. At the time of this decision, accepted-only enumeration and construction
still implemented the old policy.

## Decision

`mado-pilot-ocr`, the ONNX adapter and the Rust facade expose reviewed, implemented
catalog configurations as ordinary model choices independently of quality or
numerical benchmark results. Versioned, target-scoped test results describe what
passed, failed, was not run or remained incomplete; they do not authorize model
selection or supply a universal recommendation.

- Keep tests, exact resources, frozen rules, all samples and attributable reports.
  Failed text, region, geometry, negative-image or benchmark-ceiling checks remain
  visible findings, not reasons to hide a model or reject its selection.
- Keep technical validation: supported algorithms/providers, resource hashes and
  containment, graph/vocabulary consistency, finite execution bounds, source
  correlation, deadlines and owned cleanup. A typed runtime/contract failure is
  not waived by a benchmark result. Conversely, an empirical RSS/latency ceiling
  is not the same as an enforced tensor limit or an owned process deadline.
- Scope each result to its rule/version, corpus/workload, execution identity,
  measured source, target/provider and host/runtime. Missing Windows evidence
  does not erase an Apple result or impose a cross-target quality gate on choice.
- Replace aggregate Reference/Fast/Balanced/Unrated judgments in the active
  interface with factual rule outcomes and measurements. A comparison reference
  is a test role. The existing 80% timing condition can be reported as a timing
  rule independently of accuracy; no new threshold is introduced here.
- Preserve the native G-004 default and every saved execution identity. Neither
  a faster result nor a failed rule changes a selection or causes fallback.
  Manifest v1/v2, C/C++, provider capabilities and bounded-v2 text-watch scope
  are unchanged; an interface unable to express the resource set still refuses.

## Alternatives

- Retry or tune until every existing rule passes: rejects the requested product
  behavior and repeats work without answering which model suits the caller.
- Mark the old campaign successful or weaken its rules: corrupts evidence.
- Add an "experimental" opt-in around the same quality filter: preserves the
  unwanted gate under another name.
- Add arbitrary runtime catalogs or a benchmark service: unnecessary; reuse the
  strict compiled catalog, existing runner and read-only metadata interface.

## Consequences

Hosts list implemented choices and present applicable per-rule results,
including failures and missing coverage. A passing row says only that the named
rule passed in its recorded scope. The SDK does not choose the user's tolerance
for recognition errors or speed/memory trade-offs.

The cutover removes quality-dependent catalog status, selection filters and
aggregate admission decisions rather than adding compatibility aliases.
`ocr_profiles()` / `ocr_profile(id)` replace accepted-only lookup; generated
`evaluations` replace benchmark ratings. Resource validation stays closed and
bounded. Hosts migrate the metadata interface without a new quality filter.

Old reports, criteria, raw text, receipts, executable hashes and verdicts remain
unchanged. Any new view derived from retained observations names the original
measurement source and the new evaluation-tool revision; it is not a fresh run
or a retroactive pass. This decision authorizes no native retry, new acquisition,
security change or reuse of a spent execution grant.

## Verification

Verification covers ordinary selection despite failed or missing test evidence,
mixed rule outcomes, incomplete/invalid evidence, independent target scopes,
unchanged defaults and typed technical refusals. The `ocr-profiles` example
exercises the public metadata/configuration path without models or inference.
`tools/ocr-models/run.py evaluate` distinguishes successful evaluation from
passing all tests; quality/numerical failures alone do not make it fail.

The [derived result view](../benchmarks/ocr-models/cf2763a-evaluation-v1.json)
retains all original quality findings and separately reports numerical outcomes.
It identifies measured source `cf2763a` and the content-addressed evaluator
revision. Original measurements and verdicts were not repeated or rewritten.
