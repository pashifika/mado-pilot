# Architecture decisions

An ADR records an important architectural or technical choice, the constraints
that motivated it, and the consequences of choosing it over credible alternatives.
It is a short rationale record, not a specification, runbook or tuning ledger.
Current contracts live in [architecture](../architecture.md) and the
[topic guides](../README.md); update those in the same change.

## When to write one

Write an ADR when changing a consequential commitment: package boundaries or
dependency direction, public naming, ownership or compatibility policy, platform
or native backend selection, supported toolchain/OS policy, or a security/trust boundary.
The test is whether a future maintainer needs the reason and trade-off to avoid
repeating or accidentally undoing the decision—not the size of the diff.

Routine implementation, dependency updates within an existing policy, empirical
budget adjustments, measurements and gate bookkeeping do not need an ADR by
themselves. A gate closure needs its outcome and evidence in the relevant guide
or registry; an accompanying architectural choice may warrant a separate ADR.
Safety, ownership, bounded resource use and compatibility obligations remain
mandatory even when a numerical performance target changes.

| Material | Current home |
| --- | --- |
| Important decision and rationale | A short ADR here |
| Current behavior and public contract | Architecture or a topic guide |
| Repeatable verification instructions | Topic guide or `docs/verification/` |
| Measurement definitions and qualification rules | [Performance methods](../performance.md) |
| Tuning reason, evidence and observed effect | [Tuning history](../performance/tuning/README.md) |
| Original trial outputs and failures | Revision-bound evidence |

## Record format and numbering

Use [template.md](template.md). Aim for a page; link detailed procedures and raw
evidence instead of embedding them. Record status and date as reader information,
not an executable approval token. Ordinary review is sufficient; no separate
admission workflow is introduced.

IDs 0001–0107 are retired, including unused 0067–0069. Start new records at 0108,
use the next unused four-digit ID, and never reuse an ID. A superseding decision
links its predecessor and updates the current contract; readers must not traverse
a status chain to discover supported behavior.

## Current decisions

| ADR | Decision |
| --- | --- |
| [0108](0108-windows-component-loader-observation.md) | Bounded observer-on Windows component measurement; held-file correlation, not full backing identity |
| [0109](0109-purpose-driven-performance-metrics.md) | Matching latency, physical work and resources drive performance decisions; CPU remains diagnostic |

## Legacy history

The former ADR corpus and template are preserved byte-for-byte in the independent
planning repository at
`rasen/legacy-adrs/646369fad6c922ac7b064367817337a18e572d8c/`, with an index and
section dispositions. That archive is historical reference only. A product-only
checkout does not need it. The same original records remain accessible at the
[immutable source revision](https://github.com/pashifika/mado-pilot/tree/646369fad6c922ac7b064367817337a18e572d8c/docs/adr).

Released notes, frozen profiles and evidence can contain source-era ADR links.
Interpret them at their recorded revision; do not rewrite their bytes or use them
as current execution prerequisites.
