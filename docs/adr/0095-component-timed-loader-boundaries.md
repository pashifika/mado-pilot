# ADR 0095: Bind component loader logs to retained-sample boundaries

- **Status:** Accepted — apparatus contract only
- **Date:** 2026-10-23
- **Resolves gate:** _none_; runtime eligibility and numerical profiles require separate review
- **Supersedes:** _none_; extends ADR 0094 without changing historical evidence

## Context

A representative loaded-image union establishes neither when a library was
logged nor whether logging overlapped retained samples. An endpoint list also
cannot answer that question. The macOS component lane needs source-bound stream
positions, independently of image identity, execution outcome and cleanup.

## Decision

New macOS component campaigns use `macos-dyld-observer-on-warm-v1`. They require
complete, raw-stream timing boundaries and no in-window dyld records for each
statistical row, in addition to ADR 0094's existing admission gates.

The opt-in benchmark flag `--timing-boundaries` emits these ASCII stderr records:

```text
mado-component-timing-v1[<pid>]: <workload-id> row-start
mado-component-timing-v1[<pid>]: <workload-id> samples-begin
mado-component-timing-v1[<pid>]: <workload-id> samples-end
```

`row-start` precedes setup. The sample window begins after warmups and their
checkpoint, before the first retained iteration's preparation; it ends after
the retained loop and before after-sample/lifecycle observations. Zero-warmup
rows use the same transitions. No marker is inside an individual sample clock.
Each marker fits a 256-byte stack buffer and uses one stderr write; short or
failed writes produce an ordinary benchmark failure without retry.

JSON separately declares protocol capability and invocation enablement.
Default-off runs emit no markers. Manifest-only runs declare capability but
emit no row transitions. The collector checks actual argv, output workload,
owned root PID and ordered raw records, re-deriving any retained summary.
Missing, duplicate, foreign, torn, unsupported or reordered markers cannot
represent an empty window. Every `dyld[` record counts, including duplicate
loads and informational records. Non-statistical rows retain window counts
without a zero-record requirement; complete boundaries remain mandatory.

The policy is bound to method, plan and profile identity. Incapable binaries
are refused before qualifying launch. Historical image-only bundles remain
readable with their original meaning and cannot authorize the new policy.
Historical ports require distinct source and executable identities.

## Alternatives

- Reuse only the loaded-image union: loses temporal information and repeated
  records already represented by an image identity.
- Accept a cached zero count: lets a summary conceal missing or changed raw
  evidence.
- Treat observer-on and observer-off execution as equivalent: does not account
  for startup, prehash warming, loader formatting or pipe/scheduling effects.

## Consequences

This is a conservative byte-order observation, not proof of zero loader
activity, zero logging cost or observer-off equivalence. Each role uses the
same declared observer-on runtime. Normal still means the system allocator
without work/heap accounting. Source/host/cost and observer eligibility need
prospective review before calibration; numerical profile acceptance remains a
separate decision after calibration and independent A/A control.

No SDK API, matching algorithm, sample count, public oracle, native dependency,
packaging or permission contract changes. Recorder/no-spawn constraints apply
only to the named campaign, not arbitrary consumers. Windows normal timing,
consumer startup/multiprocess closure and release qualification remain separate
obligations under ADR 0087.

## Verification

`tools/tests/test_macos_image_observer.py` covers transient/duplicate/informational
records, malformed transitions, foreign identity and unchanged image-only
history. `tools/tests/test_template_matching_efficiency.py` covers capability
admission, raw-summary disagreement, statistical versus contract-only rules,
first-failure stop and machine-readable policy binding. Native development
checks exercise default-off execution, actual marker-write failure and full
representative preflight against separately built historical/candidate roles.
These checks accept no performance budget. The external method review must
still assess coverage and observer effects that stream positions cannot prove.
