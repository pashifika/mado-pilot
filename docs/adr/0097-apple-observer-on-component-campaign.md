# ADR 0097: Bound the Apple observer-on warm component campaign

- **Status:** Accepted
- **Date:** 2026-10-23
- **Resolves gate:** Apple component owner/method admission only; numerical and release gates remain open
- **Supersedes:** _none_; historical sources, evidence and verdicts remain unchanged

## Context

[ADR 0094](0094-normal-and-instrumented-component-measurement.md),
[ADR 0095](0095-component-timed-loader-boundaries.md) and
[ADR 0096](0096-component-control-and-comparison-gates.md) define the apparatus
and numerical rules, not an admitted campaign. The collection-loop deadline
does not cover the collector's admission, final evaluation or report retention.
The child output cap does not bound the serialized evidence on disk. Both
boundaries require a concrete method before calibration.

## Decision

Use protocol 3 with `template-efficiency-v3-session-prospective-r2` for the
Apple Silicon single-process, capture-free component lane under
`macos-dyld-observer-on-warm-v1`. Normal and instrumented builds retain separate
calibration, controls, profiles and claims. This accepts a method, not a launch
without current-stage admission or a measured performance result.

### Scope and admission

The campaign owner must bind the exact collector, observer, management owner,
baseline and candidate sources, frozen executables, attestations, representative
preflights, corpus, policy, environment and native backend/thread setting.
Freeze both roles in each mode's externally hash-bound method review before
calibration; later comparison must use that same review. Do not rebuild in
frozen target roots or replace historical identities with current ones.

The owner must bind the host loader manual and OS/kernel/cache/dyld identities,
same boot, power/low-power/sleep configuration and thermal observations. Declare
an exclusive operator-controlled window with no local builds, tests, other
benchmarks, added samplers, dependency maintenance or OS updates. The collector
lock excludes other collectors, not unrelated applications or filesystem
writers. A thermal report without warnings proves neither instantaneous
temperature nor absence of throttling. Admission must not silently change host
settings, permissions, allocator or native thread configuration.

Complete raw timing markers, zero in-window dyld records for statistical rows,
image reconciliation and pre/post bindings remain per-attempt requirements.
Their scope is the declared observer-on runtime, not zero loader activity or
observer-off equivalence. Prehash/cache warming, startup logging, pipe reader
scheduling, initializer tails and unobserved loader operations remain residuals.
Stable trusted files and OS/cache identity are assumptions; endpoint hashes do
not prove absence of mutation and reversion during a child lifetime.

### Six collection stages

Each mode contains 41 statistical rows at 20 warmups and 200 retained units per
process. Normal adds 13 contract rows and instrumented adds 16, each at one
warmup and three retained units. The six stages have independent clocks:

| Mode | Stage | Processes per workload | Maximum launches | Collection seconds | Owner execution seconds |
|---|---|---|---:|---:|---:|
| Normal | Calibration | Six baseline | 325 | 21,600 | 23,400 |
| Normal | A/A control | Six unchanged pairs | 650 | 43,200 | 45,000 |
| Normal | Comparison | Six baseline/candidate pairs | 650 | 43,200 | 45,000 |
| Instrumented | Calibration | Six baseline | 343 | 21,600 | 23,400 |
| Instrumented | A/A control | Six unchanged pairs | 686 | 43,200 | 45,000 |
| Instrumented | Comparison | Six baseline/candidate pairs | 686 | 43,200 | 45,000 |

Launch counts include one calibration manifest or two paired-stage manifests.
Pairs use `AB, BA, AB, BA, AB, BA`; A/A uses the identical baseline executable
under both labels. Each mode requires complete calibration before independent
control, then a separate accepted numerical-profile review before comparison.
Comparison uses fresh baseline processes, not calibration samples. The concrete
method binds serial stage order; modes do not share samples or spare time.

Each child retains the 300-second timeout, combined stdout/stderr cap of 8 MiB
and five-second cleanup allowance. Each pre/post image-binding pass reserves
120 seconds. The collector shortens child time to fit its remaining collection
clock and refuses launch without the binding/cleanup reserve. The
owner window adds 1,800 seconds for admission/evaluation, followed by a separate
65-second cleanup/retention tail. These are finite ceilings, not predictions or
a promise that every scheduled process can finish. The six collection clocks
sum to 60 hours; review and other management work are not included in that sum.

### Numerical and resource gates

For each statistical timing metric, freeze the calibration spread `S`, maximum
observed resolution `R`, allowance `A = max(S, 2 * R)` and per-process ceiling
`ceil(max(calibration6) + A)`. Apple CPU resolution uses at least the 1,000-ns
accounting floor. Every calibration/control/comparison role must satisfy
`spread / max(median(role6), 2 * R) <= 0.15`; later roles cannot enlarge `R`.
A/A additionally requires between-role, first/second-position and first/last
three-process median differences to be at most `A`. These are engineering
rejection rules, not confidence intervals, clock-accuracy or power guarantees.

Every later process must meet its frozen timing and resource ceilings. Both
normal selection and repeated-preparation goals require
`median(comparison-baseline6) - median(candidate6) > A` for each selected row's
p95 and summed retained-sample CPU. Equality passes ceilings/control gates but
fails benefit. Process p95 is nearest-rank 190 of 200; six-process medians do not
pool samples. Turnover hit/miss strata and their overlapping primary cycle are
graded separately, never added. Instrumented work/ownership reductions explain
mechanisms and cannot substitute for either normal benefit goal.

CPU accounting covers the entire process, including native workers and the
controlled query producer's `query.poll()` / `yield_now()` loop. It is not
isolated SDK-worker CPU. A CPU repeatability failure does not establish which
thread or mechanism caused it.

Resource ceilings are `max(calibration6) + max(spread, fixed_slack)` plus only
the applicable RSS-peak live allowance. RSS slack is 1 MiB; instrumented Rust
heap slack is 256 KiB. RSS peak adds zero for selection, 32 MiB for other rows,
and 64 MiB only for the two-independent-backend lifecycle rows. Growth and
residual receive no live-cache or per-cycle exemption. Each mode uses its own
calibration; normal heap/accounting fields remain unavailable.

RSS checkpoints and high-water observations cover the child only through the
observed checkpoint, not its later reporting-to-exit peak or management-owner
memory. Rust heap accounting is not all native allocation. Endpoint growth and
residual are finite differences, not leak rates. Footprint and ownership
checkpoints do not acquire additional numerical ceilings by being retained.
The later profile review must assess the actual derived envelopes and their
resource scope; this ADR approves no resulting byte or timing envelope.

### Evidence storage admission

The maximum 3,340 child launches yield at most 28,017,950,720 raw pipe bytes.
This is not a disk limit: replacement decoding/JSON escaping, full before/after
identities, observer receipts, started files, summaries and copied runtime
bundles add content. Ordinary campaign identities are not the compact
protocol-3 preflight identity references.

The source-bound planning review gives 353,885,294,385 bytes of collector
regular-file content across the six stages, including one additional 1-GiB
failure allowance per stage. This is a conditional content envelope for the
reviewed trusted inputs, not measured usage or a portable quota. The final
method must add actual fixed plan/profile/review bytes and separately bound
management receipts, interrupted-call output and any additional copies. The
owner must not duplicate the full report on stdout; output routing is part of
the admitted storage calculation.

Before each stage, the campaign owner must check fresh free bytes on the actual
evidence filesystem against the remaining admitted content and explicitly
reviewed filesystem/OS operating headroom. Count partial files, allocation
rounding and management artifacts; logical bytes do not include APFS metadata,
copy-on-write/snapshots, swap or unrelated writes. Retain a post-stage inventory
of lengths/hashes and compare every new file and the total with the admitted
bound before the next stage. Do not delete or truncate evidence to fit it.

A free-space check and post-stage size comparison provide admission and later
detection, not reservation or live quota enforcement. Another writer can cause
`ENOSPC` below the campaign's content bound. A hard shared-filesystem guarantee
requires a separate real reservation/quota arrangement; none is supplied here.
No new live storage watcher is proposed for this trusted finite campaign.

### Finite management owner

The private POSIX owner imports the exact collector and calls its
existing parser and `collect`, preserving ordinary attempts and reports. Every
native launch reuses the existing `process_runner`; there is no independently
killed collector child, new process supervisor or benchmark sampler.

Bind absolute same-host monotonic execution and retention deadlines in an
exclusively created plan. Plan parsing, input verification, import, evaluation
and final report hashing consume that execution budget; admission cannot restart
it. A main-thread one-shot `SIGALRM` interrupts non-child work. Inside the
runner's ownership region it only marks expiry, without throwing through
`Popen`, reader setup or cleanup. Clamp child time to remaining execution time
minus its unchanged cleanup reserve and check expiry at launch entry.

Preserve the runner's unreaped group-leader fence through termination, cleanup
and reap. After the runner returns, expiry still fails the stage; retain the
actual bounded child receipt separately if the collector could not receive it.
Never rerun the command or reinterpret a late child success as stage success.
Keep primary outcome, cleanup, retention and unknown facts separate. Refuse a
foreign thread or active timer; restore timer/handler/hooks on ordinary exit
without changing the benchmark child's signal mask or environment.

Whole-owner/OS loss, an uninterruptible OS primitive and storage failure remain
residuals. Killing an outer process group does not establish cleanup of a
benchmark launched in a separate session. This design is not an external-kill
cleanup guarantee, and expiry cannot guarantee complete evidence persistence.
No timeout, failed cleanup, write failure, missing provenance or Apple timing
prerequisite failure authorizes another stage. Preserve failures and unrun slots;
no automatic retry, sample removal/replacement or deadline extension is allowed.

## Alternatives

- Treat collection clocks and raw caps as command/disk bounds: omits admission,
  final evaluation, serialization and retention.
- Kill an outer collector process: does not own the separately sessioned native
  benchmark's cleanup fence.
- Add continuous resource/storage monitoring: changes the observed runtime;
  finite trusted-input admission avoids that extra perturbation but is not a
  hard host-wide enforcement mechanism.

## Consequences

Recorder and no-spawn assumptions apply only to this campaign. Applications own
their selected targets, windows and consumer-created children. The SDK remains
responsible for its documented parameter contracts, resources and workers; it
acquires no no-spawn restriction or requirement to adopt these recorders.

Final hash-bound method artifacts and later profile reviews are authoritative
for each admitted campaign. This decision grants no startup, observer-off,
Windows, actual-consumer or whole-release claim. The whole performance Change,
Windows normal observation and consumer qualification remain incomplete under
[ADR 0087](0087-v0-5-0-release-performance-qualification.md).

## Verification

The retained native owner proof passes twelve finite scenarios: normal return,
descendant timeout/group cleanup, non-child expiry, expired-launch refusal,
output cap, nonzero exit, launch failure, late completion, SIGTERM handling,
retention failure/expiry and expired-plan refusal. Frozen-baseline direct
collector and observer-sibling integrations also pass. Returned native cleanup
is complete, and owner/support/frozen input hashes remain unchanged.

Independent source lifecycle and method-admission reviews report no findings.
The accepted method binds those reviews/proofs, both frozen roles, the unchanged
policy and conditional storage envelope. Initial admission observes AC power,
unchanged boot, low-power/sleep settings and sufficient free capacity; every
later stage requires fresh admission. These observations do not establish a
200-sample numerical result, future availability, continuous thermal/worker
invariance or whole-owner-loss recovery.

Exact identities and raw receipts are retained under the owning Change in the
separate Rasen repository, in
`evidence/numerical-admission-verification-20261023.md`.
Calibration, independent A/A and numerical-profile review remain separate gates.

The baseline-only normal calibration completes its 325 benchmark processes with
successful cleanup. Its first stored CPU-repeatability failure is
`single-small96x64/cpu_ns`; evaluation stops there, without declaring other rows
clean. An independent offline audit also finds CPU-noise failures for
`session-owner8-bgra-roi` and `query-match-rgba-roi`, without rerunning or
regrading the campaign. Complete raw evidence and frozen input identities remain
retained; no A/A control, numerical-profile acceptance or candidate comparison
has run. Method acceptance does not promote that failed calibration or establish
product regression or benefit.
