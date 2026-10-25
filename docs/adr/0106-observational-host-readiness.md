# ADR 0106: Observational host readiness

- **Status:** Accepted — launch and release-decision policy; numerical qualification deferred
- **Date:** 2026-10-25
- **Resolves gate:** none
- **Supersedes:** p050's CPU-idle/transient host-load launch gates and empirical numerical qualification as a prerequisite to implementation closure or release-decision progress; no historical outcome

## Context

The retained Apple trial-v4 A/A scopes refused launch at 89.69% and 85.82% CPU
idle against a 90% minimum. Their owners never started; control, profile and
comparison stages remain not-run. These are historical readiness refusals, not
consumer failures or evidence of background-load causality.

The independent planning repository retains both scopes in the performance
Change's `evidence/numerical-admission-verification-20261023.md`, sections
“Fresh trial-v4 calibration and A/A readiness” and “Trial-v4 A/A control
continuation”, with locators 086 and 087. The operator's 2026-10-25 decision
removes uncertain OS-sensitive gates rather than seeking a quiet OS or requiring
empirical qualification to pass before development and release decisions advance.

## Decision

For new p050 scopes, CPU idle percentages, transient host-process CPU and OS
background activity are optional descriptive observations, never launch gates.
Their values or unavailability must not refuse launch, trigger quiet resampling,
or initiate replacement campaigns or loops until a measurement passes.

Uncertain OS-sensitive empirical numerical qualification is not a prerequisite
to development, implementation closure or release-decision progress.
[ADR 0087](0087-v0-5-0-release-performance-qualification.md)'s remaining p050
qualification work receives an explicit deferred disposition in `P050-OS-NOISE`,
not a passing verdict. Keep the collector's numerical rules unchanged for
optional qualification reports and any supported performance claims. Do not
require replacement calibration, controls or tuning until pass to close
implementation work. Required correctness, provenance, safety, compatibility
and CI obligations remain separate.

Do not intentionally overlap owned builds, tests or independent timing work with
numerical collection. Retain source, binary and resource identity, actual process
ownership and cleanup, correctness, absolute deadlines and finite output/storage
bounds. Optional host observations do not replace required owned-invocation CPU
accounting or other metric admission. Identity, cleanup and control fences remain
in force; this policy is not authority to bypass them or to run a new campaign.

Successor execution plans and retained receipts must identify `ADR 0106` as their
host-readiness policy, separately from the numerical method/profile identity.
Preserve historical 90% refusals, raw observations, source/binary bindings,
hashes and unrun slots without reinterpretation. ADRs 0099, 0104 and 0105 and all
source-bound numerical outcomes remain unchanged.

For the trial-v4 continuation, retain the collector, context, method and
passing calibration bytes. An explicit prospective applicability supplement
supersedes only the `exclusive_host` gate's outer CPU/process-readiness
requirement, not numerical grading, metrics or actual ownership checks. Name
that supplement beside the new result; do not report the old readiness as passed.
Removing this non-measurement predicate requires no new calibration.

The continuation uses the unchanged bounded `run_consumer_stage.py` owner with
one new stage plan, new deadlines and a new output path under the existing owner
root. Preserve all old plans, deadlines, refusals and outputs. No CPU probe or
orchestration framework is added; runtime/binary hash checks remain the original
owner/collector's responsibility. A terminal numerical failure remains a failure
under that unchanged method, with its unrun slots and control fence retained.
Do not rerun the control or bypass its fence to obtain implementation closure.

## Alternatives

- **Keep or lower the idle threshold:** still makes an uncertain OS observation
  determine launch admission without establishing measurement validity.
- **Resample until quiet or repeat campaigns until pass:** selects execution
  conditions or results after observation and obscures retained failures.
- **Treat removal as a numerical pass:** confuses launch or release-decision
  authority with evidence of repeatability, benefit or performance qualification.

## Consequences

The [performance method](../performance.md#p050-host-observations) and
[release status](../releases/v0.5.0.md#performance-qualification-status) distinguish
this policy from historical refusals and optional numerical qualification.
No SDK/runtime behavior, measurement population, numerical budget or accepted
profile changes. Variability, inconclusive results and failed benefit checks
remain factual under their source-bound rules; host noise is not a proven
explanation or a numerical waiver. Removing these gates neither establishes
accepted performance nor approves a release or an all-target benefit claim.

Unresolved OS-noise causality, OS-independent repeatability and numerical
qualification follow-up belong to stable item `P050-OS-NOISE` in the independent
planning repository's `rasen/backlog.md`. Its unresolved status does not block
implementation closure or release-decision progress. Archiving the performance
Change does not resolve it or permit unsupported performance claims.

## Verification

Execution review must inspect the actual launch predicate and retained plan/
receipt policy identity: no CPU-idle or transient host-load threshold, no required
availability of those observations, and no quiet-resampling or retry loop.
Review must separately retain owned-work scheduling, identity, cleanup,
correctness and bounded-execution obligations. A missing optional observation
must remain unavailable, never an invented zero or evidence of an idle host.

The Apple trial-v4 control launched through the original owner without a CPU
probe. Its first invocation passed semantics, runtime checks and cleanup, but
lifetime CPU was approximately `0.542516s`, above the frozen `0.519604s` ceiling.
Operation time `596210us` stayed below `1000000us`; resident bytes `150552576`
stayed below `268435456`. Raw CPU coverage remains partial with null resolution;
grading retains the existing source-conditioned accounting admission.
The collector stopped with eleven slots unrun and its failed control fence
retained. This is a failure under the original numerical method, not a new
readiness refusal or evidence of OS causality. No numerical retry follows.

The performance Change's `evidence/numerical-admission-verification-20261023.md`
section “Observational host policy and retained control outcome” owns this
source-bound result. The current release-decision disposition is deferred
numerical qualification, not a passing control or permission to bypass its fence.

Historical locators 086 and 087 retain their original decisions. Release review
must distinguish deferred qualification from passing evidence, preserve failed
numerical reports and withhold unsupported benefits. Optional future
qualification needs separately authorized source-bound evidence; this decision
does not create an automatic enforcement mechanism or authorize a retry.
