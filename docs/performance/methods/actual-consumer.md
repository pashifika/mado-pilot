# Actual-consumer scalar method

This lane measures a real application invocation, not 200 interchangeable
component operations. Use a single-case development smoke for investigation.
Explicit qualification adds the fixed scalar cohort below without changing the
[observation/safety distinction](../../performance.md#observation-enforcement-and-safety).
A saved-RGBA result is not dispatch-to-visible UI or native-capture evidence.

## Development observation

`tools/consumer-performance.py` uses one source-bound case and a persistent
campaign root for the target/host. It never builds, downloads, supplies an
implicit application environment, chains stages, retries or resumes a cohort.
Plans carry explicit argv, cwd/environment, source/resource hashes and executable
identities. They must authorize execution and declare finite bounds even when
no numerical qualification is requested.

To import commands and identities from an existing development source plan:

```sh
python3 tools/consumer-performance.py prepare \
  --source-plan "$SOURCE_PLAN" --case trial-hit-roi \
  --campaign "$CAMPAIGN_DIRECTORY" --output "$NEW_PLAN"
python3 tools/consumer-performance.py smoke \
  --plan "$NEW_PLAN" --output "$NEW_EVIDENCE_DIRECTORY"
```

`SOURCE_PLAN` is an explicit local plan containing the selected consumer's
commands, observed role/executable identities, inputs and environment; it is
not an accepted profile or a path in the planning repository. `prepare` imports
those commands/identities, not old measurements as calibration. It requires an
observation-on baseline for the selected case and verifies the resulting plan.
For a newly prepared plan, retain its actual local source/artifact bindings;
do not relabel an old binary with the collector checkout's revision. `smoke`
also accepts an already prepared valid plan directly. Output files/directories
must not exist.

The twelve case IDs are `replay-{full,roi}`, `group-{full,roi}`,
`matched-{full,roi}`, `deadline-{full,roi}` and
`trial-{hit,miss}-{full,roi}`. Select exactly one. Keep its fixed RGBA input,
full/nonzero-ROI geometry, frame sequence and result oracles.

The selected configuration is the actual desktop dev-normal build:
`opt_level=0`, `debuginfo=2`, debug assertions, no benchmark accounting, and
`MADO_P050_OBSERVATIONS=1`. Observation-on is not observation-off or optimized
shipping code. Changes to configuration need their own recorded experiment;
no consumer instrumented mode is silently added.

Smoke executes one baseline invocation within 90 seconds primary, five seconds
cleanup and ten seconds retention, with 4 MiB combined stdout/stderr and a
900-second whole-phase ceiling. It needs no method review, calibration, A/A,
profile, runtime-closure approval or quiet-host reading. Optional accounting or
qualification-eligibility gaps remain visible rather than preventing otherwise
safe diagnostics, including on Windows.

Current phase reports use `consumer-phase-v2`. `execution_status` is
`completed`, `incomplete` or `failed`; `qualification_status` is `not-requested`
for smoke and `passed`/`failed` for an explicit numerical stage. The existing
`qualification` boolean stays false except for an accepted comparison. In
smoke, numerical/qualification clock findings remain in `observation_issues`;
execution, semantic and safety issues remain failures. A safely completed
observation can exit zero despite descriptive target misses or unavailable
optional accounting. Explicit `calibrate`, `control` and `compare` retain
numerical enforcement. Original raw records keep their findings; old reports
are not relabeled under the current meaning.

Campaign identity, owned-cleanup and unfinished-registration fences remain
execution barriers. Safe smoke after a numerical/inconclusive control needs
the sealed original summary and hash-bound started/raw/validation receipts
showing terminal cleanup with no unknown, semantic or identity failures. This
applies to a complete or numerical-early-stop control, not arbitrary missing
evidence. A new output-specific smoke registration neither retries calibration/
control nor clears the qualification fence. Reuse the actual campaign identity;
another output directory does not resolve unsafe state or admit a candidate.
Never overwrite, clear or reinterpret a failed control to obtain a pass.

## Units and clock boundaries

| Cases | Primary decision for explicit qualification | Independent observations |
| --- | --- | --- |
| Replay full/ROI | Non-regression | Exported startup, child preflight/workflow, cleanup and lifetime elapsed/CPU |
| Retained eight-template group full/ROI | `group_span_ns` scalar benefit | Each fixed call position/outcome, lifecycle and lifetime CPU non-regression |
| Matched query full/ROI | `query_wait_ns` scalar benefit | Complete source/result correlation, settlement and lifetime CPU non-regression |
| Deadline query full/ROI | Semantics and non-regression, not speed benefit | Application `FAIL`/`Timeout`/`deadline_exceeded`/exit 1 remains its actual outcome; retain return/settlement overrun and clean teardown |
| Fresh trial hit/miss × full/ROI | `operation_us` responsiveness and observed non-regression, not positive whole-trial benefit | Real selected OCR initialization, stages/settlement and lifetime CPU non-regression |

One invocation yields one scalar. Report six same-case cross-process values,
median, minimum and maximum for a numerical cohort, not a process-local p95.
Do not pool heterogeneous calls, outcomes, processes, roles, targets or modes.

| Observation | Actual boundary |
| --- | --- |
| Host call/query wait | Observation-on raw `control.started` ns, one call through terminal-fault retention before aggregate insertion/JS return; not isolated SDK search time |
| Retained group | First recognize start through eighth release end in one child epoch, including intervening JS/bookkeeping and fifteen preceding call-record insertions |
| Replay parent | `startup_us` ends at receipt of `ChildStarted`; `elapsed_us` ends after writer/reader/stderr joins. Polling, refresh, delivery/drain and scheduling remain included; startup can be observed during post-exit drain |
| Replay child | `preflight_us` and `workflow_us` are child-local `Instant` durations; workflow includes Node import inspection, not added parent detection delay |
| Replay cleanup | `Host::finish` after admission closure/optional cancellation through reported elapsed evaluation; excludes later fields/receipt-permit release. Cached clean results are not fresh samples or final-owner teardown |
| Trial | Primary `operation.total_duration_us` starts immediately before `recognition_trial`, ends after terminal polling, public shutdown and final drain. Input decode/hash and controller construction belong to top-level total, not the primary |
| Lifetime CPU | Owned invocation's user+system startup/descendants/teardown, denominator one invocation; not operation CPU or wall utilization, and not added to already included child CPU |

Trial parent and capability/recognition-child marks use separate epochs. Never
subtract across them, infer pre-main time from their difference, or add
overlapping stages. Observation-off integer-us aggregates cannot reconstruct
raw call records, group spans, child stages or capability identity.

Before numerical calibration bind each metric's API, unit, PID/owner,
interval/checkpoint and missing reason. Timing/CPU also need reporting quantum
and reviewed resolution or accounting allowance. Ns/us, QPC frequency or Job
100-ns source units alone establish neither accuracy nor effective resolution;
requested 1/5-ms polling sleeps are not clock resolution or detection-lag bounds.
Do not subtract polling cadence or observer cost.

POSIX root `wait4` CPU remains raw `partial` with null resolution, even after
reap; it is not a full process-tree census. Windows final zero-active Job
accounting can be complete but its effective resolution remains unavailable.
Source-closed descendant/reap evidence, actual terminal witnesses and separately
reviewed accounting margins are numerical-admission evidence, not rewritten raw
coverage or physical-accuracy claims. Windows numerical runtime/CPU admission
remains unavailable; development observations do not qualify it.

## Optional qualification

The current review identity is `consumer-method-review-v4`. v1/v2/v3 reviews
and old profiles cannot admit new numerical collection. The
`consumer-profile-v1` container binds the current review and recomputed envelope;
its unchanged name does not make older policy/context bindings transferable.

A method review binds the exact context digest, accepted reviewer/rationale,
metric contracts and hash-bound evidence for `source`, `runtime_images`,
`owned_processes`, `observer_cost`, `clocks_cpu`, `resources`, `retention` and
`exclusive_host`. Unknown or missing required evidence is not passed. The last
gate concerns owned-work scheduling and context, not a CPU-idle/transient-load
threshold. Optional host readings or their absence cannot refuse launch or
trigger quiet-host resampling.

For each case/target/configuration retain six fresh baseline calibrations,
six independent unchanged A/A pairs, then six fresh baseline/candidate pairs.
Paired order is `AB,BA,AB,BA,AB,BA`. That is 30 attempts/case (24 baseline,
six candidate), 360/target and 720/two targets for the declared configuration.
Another mode/population has a separate method, not hidden samples in these totals.
A passing calibration and A/A plus separate numerical-profile review precede
candidate collection. `profile-proposal` prints the proposal for review;
`freeze` checks accepted bindings. No candidate result widens its own envelope.

For timing/CPU metrics other than trial `operation_us`, let B be six calibration
values, q the reporting quantum, and r the maximum effective calibration
resolution or separately reviewed accounting allowance in the same unit:

- Freeze `s=max(B)-min(B)`, `a=max(s,2*r)`,
  `ceiling=q*ceil((max(B)+a)/q)` using exact ticks/lossless decimal arithmetic.
- Require `s/max(median(B),2*r)<=0.15`. Each later role independently meets the
  same noise rule with frozen r; a coarser later clock cannot enlarge it.
- Each invocation meets its ceiling and all semantic/ownership/resource oracles.
  A favorable median cannot cancel one violation.
- Absolute A/A role-median difference and each control/comparison role's
  first/second-position and chronological first/last-three median differences
  are `<=a`.
- A declared group/matched scalar benefit requires
  `median(comparison-baseline)-median(candidate)>a` strictly. Lifetime CPU and
  other endpoints independently retain their ceilings/non-regression checks;
  a separately requested lifetime-CPU benefit is not operation-local attribution.

Keep known failures alongside missing prerequisites and unrun slots. A numerical
control failure blocks that target/lane's profile/candidate qualification, not
only the one case, without blocking unrelated safe diagnostics. Identity and
cleanup applicability persist outside restartable phase owners across affected
lanes. This method supplies no full-campaign/release permission; current
[release status](../../releases/v0.5.0.md) owns its separate disposition.

## Fresh-trial responsiveness

Every `trial-*` operation uses `policy="fresh-trial-central-four-v1"` and
`population="fresh-process-first-operation"`. Bind positive exact JSON integers
`hard_ceiling_us` and `max_regression_us`; Boolean, fractional and string budgets
are invalid. Require `max_regression_us<=hard_ceiling_us`, a ceiling strictly
below `bounds.primary_seconds*1_000_000`, positive quantum/resolution, nonempty
resolution method and `2*resolution<=max_regression_us`. There are no defaults.
At least one accepted `resources` evidence item repeats the identical policy,
population and budgets under `trial_responsiveness`, with exact context/source
bindings, `complete=true` and a user-response/build/observer justification. All
such declarations must agree; this policy is invalid on nontrial metrics.

Every operation, including smoke supplied with a current review, meets the fixed
ceiling. One excess remains a numerical violation independently of any median.
In ordinary smoke it is descriptive; explicit enforcement returns non-success.
Correctness, identity and cleanup failures remain primary.

For each complete six-observation calibration or role:

1. Preserve all six original values/slots, including the first invocation.
2. Sort exact values by value and original schedule order. Remove one minimum
   and one maximum occurrence **from repeatability only**. Ties remove earliest
   minimum and latest maximum; all-equal values still exclude distinct slots.
3. Compare the central-four range with `max_regression_us`. Excess is
   inconclusive repeatability; equality passes. Missing, failed or partial
   cohorts cannot be repaired by trimming or selecting successes.
4. Retain full-six min/max/range/median and separate central-four values,
   range/mean/median plus excluded values/indexes bound to original slots/raw
   references. Full-six and central-four medians are identical; the central-four
   mean is descriptive only. Reports without a current review omit this aggregate.

With `a=max_regression_us`, keep all six balanced pairs, full-six medians and
these independent checks:

| Check | Bound and excess |
| --- | --- |
| Each role's first/second-position and first/last-three chronological median differences | Absolute difference `<=a`; excess is inconclusive order/drift |
| Unchanged A/A | Every absolute paired difference and absolute role-median difference `<=a`; excess blocks profile/candidate admission as inconclusive control |
| Baseline/candidate | Every paired `candidate-baseline<=a` and `median(candidate)-median(baseline)<=a`; excess is observed numerical non-regression violation |

Equality passes; one microsecond excess fails its bound. Faster candidates need
no minimum gain. Retain numerical violations alongside independent inconclusive
findings. Central-four screening deliberately loses full-tail sensitivity:
median/order/drift do not recover an excluded extreme's magnitude, a slow
baseline maximum can mask its paired candidate, and an isolated calibration
extreme below its ceiling can pass. This is not a warmed process, startup-free
measurement, confidence/prediction interval, causal attribution or population SLO.
No pair, CPU or resource observation is trimmed.

The scoped prospective engineering selection is `hard_ceiling_us=1000000` and
`max_regression_us=100000` solely for Apple `trial-hit-roi`, the named 32 GiB host,
dev-normal, observation-on and reviewed loader-on configuration. It needs exact
context/source-bound admission; other cases, hosts, targets, builds and modes
inherit neither amount. One second is the authoring-command response objective;
100 ms is a separately chosen engineering deterioration allowance, not derived
from historical timings or a process timeout. Inclusive 100-ms deterioration may
be noticeable. Neither amount promises imperceptibility, visible-UI latency,
observation-off equivalence or an SDK consumer limit. The original
[response-time guidance](https://www.nngroup.com/articles/response-times-3-important-limits/)
and [Apple responsiveness guidance](https://developer.apple.com/documentation/xcode/improving-app-responsiveness)
do not derive this regression/repeatability allowance or justify blocking a UI
main thread for one second.

## Resident checkpoints

Keep resident points separate from high-water, sampled maxima, requested Rust
bytes, native allocations, same-owner growth and final-owner residual.
Normal Rust counters remain unavailable. Sampled engine/supervisor RSS does not
cover Node/trial children; lifetime sampled maxima are not OS high-water.

Each selected resident metric declares positive integral `hard_ceiling_bytes`
and `comparison_allowance_floor_bytes`, floor no greater than ceiling, with
matching accepted context/source-bound `resources` evidence under
`resident_policies` and a nonempty justification. Resident contracts have no
invented clock quantum/resolution. Every invocation, including calibration,
requires positive observed bytes `<=hard_ceiling_bytes`.

Six baseline values freeze `s=max(B)-min(B)`,
`a=max(s,comparison_allowance_floor_bytes)`, fixed ceiling/floor and exact
min/max/median/spread. Require `s/median(B)<=0.15`; zero spread and fractional
medians are valid. Each six-value later role retains that repeatability rule and
first/second-position and first/last-three median differences `<=a`. A/A absolute
role-median difference is `<=a`; comparison requires
`median(candidate)<=median(baseline)+a`. A good median cannot excuse a per-row
ceiling violation. This is resource non-regression, not a latency/CPU benefit.

Only Apple `trial-hit-roi`, the bound 32 GiB host, dev-normal/observation-on has
the prospective `hard_ceiling_bytes=268435456` (256 MiB) and
`comparison_allowance_floor_bytes=1048576` (1 MiB) allocation. The endpoint is
`recognition_child_rss_bytes`, actual recognition-child PID,
`native-self-rss-v1 libc::proc_pidinfo(PROC_PIDTASKINFO).pti_resident_size`,
`unit=bytes`, `domain=resident`, `after_trial_owner_release` /
`recognition-child-process-point`. Require settled release and
`owner_released<=started_ns<=finished_ns` in that child's `control.started`
epoch. It brackets one query, not an interval maximum. Missing/short native
results, initialization or settled release, zero bytes or missing endpoints
remain unavailable, not zero-byte passes.

The ceiling is a declared `1/128` allocation of `34359738368` host physical
bytes, not an observed-RSS fit, automatic rescaling formula, RAM quota, host
capacity/concurrency guarantee or SDK limit. At low medians the 1-MiB comparison
floor can dominate 15%; it is not page size, measurement resolution or statistical
coverage. Other cases/hosts/modes need separate resource admission.

Separately admitted counted Rust endpoints retain
`max(calibration6)+max(spread,256KiB)` without supplying missing normal counters.
Growth/residual are same-process positive differences, not fresh-child variation
or per-cycle divisions. Exact cache ownership/capacity is not calibrated slack.

## Finite execution and retention

One phase is one case, target, configuration and stage. Inspect after complete
two-attempt blocks; pauses consume the same live absolute deadline. Early close
retains an incomplete phase, not a resumable cursor.

| Stage | Attempts | Work reservation | Independent total | Evidence reservation |
| --- | ---: | --- | ---: | ---: |
| Calibration | 6 | `60+6*(90+5+10)+60=750s` | 900s | 292 MiB |
| A/A control | 12 | `60+12*(90+5+10)+60=1380s` | 1680s | 580 MiB |
| Comparison | 12 | Same 1380s | 1680s | 580 MiB |

Reserve the full next 105 seconds plus pending 60-second final retention;
a pair needs 210 seconds plus that reserve. Trial internal 10-second preflight,
30-second operation and 14-second shutdown remain inside the 90-second outer
attempt. A retained driver may declare 60-second primary/five-second cleanup/
1-MiB output: preserve that declaration, require primary/output no larger than
enforced bounds and equal cleanup, and execute under the real outer bound.
These are operational reservations, not empirical latency ceilings.

Each replay/group/query selects engine and Node children; a trial selects
sequential capability and recognition children. The 1,080 selected application
process creations/target (360 roots plus 720 children) are not a complete
owned-tree cap. Platform-created members need actual identities and a finite
reviewed maximum; unexpected counts/members refuse qualification. Campaign
recorder/no-spawn restrictions do not constrain arbitrary SDK applications.

The 48-MiB/attempt content allocation includes 24 MiB escaped outer text,
64 KiB metadata, 64 KiB started receipt, 256 KiB validation, 256 KiB summary
details and two 8-MiB child sinks: 40.625 MiB plus 7.375 MiB headroom. Reserve
`48*n+4` MiB/phase plus a 64-MiB free-space margin. Account prospectively for
new closure/sink/copy bytes; sampled scans and free-space checks are not hard
filesystem quotas. Do not duplicate unaccounted raw summaries or discard evidence.

`prepare --loader [--node ABSOLUTE_BINARY]` enables actual Apple loader logging;
`discover --plan PLAN --role baseline|candidate --output NEW_DIR` invokes one
real selected command. `close-runtime --plan PLAN --discovery RECEIPT --output
NEW_FILE` reparses explicitly named raw/sink records, validates root/two named
child transports and file/cache bindings, and freezes closure without executing
or approving a method. Replay pins the actual PATH-selected Node. Foreign PIDs,
unknown images, incomplete sinks and drift refuse closure. Fresh changed
contexts require fresh applicable loader discovery, not edited origin bindings.

Source/build/runtime, observation policy, owner/checkpoint, workload, host,
configuration, collector arithmetic or schedule changes invalidate affected
qualification admission; review prospectively and obtain fresh affected closure,
calibration, control and profile. Preserve frozen Cargo roots and executables.
Hash-bound reads parse/hash one byte snapshot, not separately reopened paths.
Launch rechecks the original deadline after durable admission. Cleanup failure
fences execution even if evidence retention fails; missing provenance is not
repaired by successful cleanup. Whole-owner/OS loss is not a cleanup guarantee.

## Historical outcomes

The following immutable records are history, not current authority or permission
to continue a failed cohort:

- [Original Apple trial calibration](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0104-fresh-trial-responsiveness-and-non-regression.md#context):
  `operation_us` values `551859`, `471323`, `475221`, `470318`, `469603`, `473136`;
  range `82256`, median `472229.5`, exact range/median `164512/944459`,
  `17.418649%` (about `17.42%`). All six functional/runtime/cleanup checks passed,
  but the original 15% rule returned `NONPASS`, empty envelopes and no subsequent
  A/A/candidate/profile. This establishes no candidate regression.
- [Original resident control](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0102-actual-consumer-resident-point-policy.md#context):
  `FAILED_CONTROL_RESOURCE_ENVELOPE`; four slots ran, slot `03` failed and slots
  `04`–`11` remain not run. Successor ceilings do not regrade or resume it.
- [v3 comparison and one-shot diagnostic](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#fresh-trial-responsiveness-and-non-regression):
  comparison was inconclusive at baseline range `102878µs` against `100000µs`;
  all twelve semantic/runtime/cleanup checks passed and no candidate
  non-regression violation was reported. A separate private-source diagnostic's
  parent frame-read `539.245ms` and child main-to-emission `23.245ms` use separate
  clocks; they establish neither pre-main residual nor loader/pipe/OS causality.
- [v4 readiness refusals and retained control](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0106-observational-host-readiness.md):
  historical 89.69% and 85.82% idle failed a 90% launch threshold without starting
  their owners. The later no-CPU-probe control started, passed semantics/runtime/
  cleanup, then stopped at approximately `0.542516s` lifetime CPU above the
  frozen `0.519604s` ceiling; `596210us` operation and `150552576` resident bytes
  stayed below `1000000us` and `268435456`. Eleven slots remain unrun and the
  numerical-control fence remains. This is not an OS-causality finding.

Deferred qualification is not passing qualification. No historical failure,
profile or unrun stage is changed by ordinary diagnostic access or this method
reclassification. Native/ABI/permission/safety obligations remain separate.
