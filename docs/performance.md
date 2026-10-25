# Benchmark profiles and budgets

Later phases of MadoPilot introduce performance-sensitive behavior: capture,
frame mapping, template matching, OCR, watcher scheduling, and acceleration. Each
of those phases must be able to show that its behavior is both correct and within
an agreed cost before the phase exits.

This document defines the format that evidence takes. Setting a numeric budget
for a workload is gate [`G-013`](validation-gates.md#g-013), which is resolved
per workload and target rather than once. Phase 1 is resolved. ADR 0021
invalidated the three historical macOS Phase 2 native profiles after source and
correctness-oracle drift. ADRs 0024–0032 accept the current target-specific
Phase 2 diagnostic, native input, controlled owning-process, production capture,
transition, and corrected dual-4K profiles while preserving each historical
source identity.

[ADR 0037](adr/0037-phase-3-ocr-performance-budgets.md) accepts target-specific
Apple M1 Pro and Core i7-12700KF default-OCR profiles. Each uses five precursor
processes and a separate five-process post-budget run for cold accepted-model
startup, warm full-frame and bounded-region OCR, empty results, exact source
correlation, allocation growth, observed mapped bytes/inference/session/result
counts, cleanup, close, and target-native resident high-water. A separate native
contract test covers cancellation, late-publication suppression, recovery, and
close races without mixing that instrumentation into latency samples. Hosted
Windows Server smoke enforces hard correctness and growth only; its timing and
resident observations define no release-host budget.
Phase 4 keeps three authorities separate. Required Lane A owns deterministic
replay/OpenCV scheduler, query, ownership, and accounting semantics. Required
target-specific Lane B owns the compact WGC and ScreenCaptureKit semantic and
cleanup contracts. Optional Lane C owns statistical latency, resource, topology,
and endurance evidence; it is not a release gate unless an explicit policy says
otherwise. A correctness, cleanup, bounded-resource, or privacy failure remains
terminal regardless of lane.

Nothing in this document is itself a measured result. The numbers live in the
profiles under [benchmarks/](benchmarks/), each naming the host it was measured
on, and the example this document references records no measurement at all.

The Rust OCR text watcher has complete [Apple](benchmarks/ocr-text-watch-aarch64-apple-darwin.toml)
and [Windows](benchmarks/ocr-text-watch-x86_64-pc-windows-msvc.toml) precursors.
[ADR0075](adr/0075-ocr-text-watch-workload-profiles.md) accepts target-specific
ceilings before final qualification. Its Python-owned format version2
profiles extend the legacy version1 convention below without changing historical
profiles or assigning their hard-budget constants to another executor.

The [measurement scopes](adr/0071-ocr-watch-observable-measurement-scopes.md)
separate actual view traffic, logical extents, native process memory, logical
close and physical quiescence. Controlled percentiles stay process-local;
resources include warmups. Real startup intervals retain their actual clock
boundaries and exclude the collector from native-memory observations. The
source-bound runner validates each required metric and accepted profile/ADR,
then combines numerical comparisons with independent semantic/identity/cleanup
gates. Complete precursor observations alone do not qualify a numerical budget.

Current [pacing evidence](capture-pacing.md#owned-native-verification-apparatus),
[release-profile tools](evidence/native-release-profiles/README.md#current-apparatus-safeguards)
and [OCR measurement tools](ocr-model-catalog.md#repeatable-rust-measurements)
enforce final receipt authority, retained-snapshot accounting, pinned compiler
identity and interruption checkpoints. These are apparatus safeguards, not a new
native or statistical campaign. Historical schemas, exact evidence bytes,
source/binary hashes, measured values, failed attempts, accepted ceilings and
source applicability remain unchanged. Tool checks alone do not qualify current
capture/input behavior, a new host or a broader release claim.

## Rust template-matching work efficiency

`template-matching-efficiency` is a capture-free Rust benchmark. Its current
protocol-4 executable keeps the protocol-3 corpus: 57 instrumented rows, of which
54 also run in normal mode.
It preserves the 21 component/selection/ownership guards and adds 36 public
Session, query and lifecycle rows. `--list` prints the first-party generated
corpus manifest and hashes; `--smoke` runs one warmup and three samples. No row
opens a capture device, injects input or runs OCR.

OpenCV accepts source RGBA8 and BGRA8 directly, with BGRA8 as its default.
`BackendDescriptor::format()` names that default; use `accepts_format` for
capabilities and the actual mapping descriptor for byte provenance. Full-frame
CPU-owned RGBA avoids the channel-swap mapping allocation; full-frame BGRA keeps
its existing path. Cropped regions still require packed mapping storage, and
native CPU readback is not eliminated. Both formats convert to owned three-channel
BGR with unchanged alpha exclusion, scoring, suppression and caller options.

Exact-request reuse distinguishes independent equal-stamp frames. Compatible
newer RGBA mappings can now reach existing `ExactRgba` unchanged admission;
BGRA, format transitions and `AnalysisAlways` require analysis. Unchanged
pixels do not confirm consecutive or duration stability.

The `match-report` example emits `defaultPixelFormat` and `acceptedPixelFormats`
instead of `pixelFormat`; retained reports are not rewritten. The deterministic
slice's derived mapped-byte measure counts logical searched pixels, not row
padding, copy traffic or native allocations.

Live cropped/channel-converted mappings reuse only weakly indexed, still-owned
pixels. Backend clones separately share at most two immutable BGR images and
33,554,432 retained payload bytes; this does not retain mapping reservations.
Independent backends each have that allowance. Expired sources are pruned on
access, and final backend ownership releases retained images; closing a session
alone need not flush a surviving backend.

Score selection owns no second dense map. The one-result path scans once;
multi-result extraction keeps a row-bounded heap and repairs stale winners.
A live maximum stays at the heap root; only stale roots are repaired or removed.
This avoids unchanged pop/push cycles. For `KeepAll`, each eligible row guarantees
one result: reserve from the smaller of that row count and the requested limit,
rounded up to a power of two. This skips result-vector growth steps without
reserving for empty rows. No-hit searches allocate no candidate storage;
`DropOverlapping` still grows with actual survivors. Reservation can move result
allocation earlier in an interrupted call. Dominated raw values bypass
normalization. A scan stops at its first finite public score `1.0`: clamping
prevents any later offset from outranking that canonical maximum. Only the
visited prefix contributes to scan accounting and the 4096-offset checkpoint
bound. Exact checkpoint positions can change; the caller's final commit remains
required. The exact threshold applies to each proven maximum. Different
templates still execute independently.
These mechanisms remove named work, not establish a universal latency or memory
improvement. [ADR 0086](adr/0086-template-matching-preparation-reuse-and-selection-work.md)
records identity, lifetime and numerical acceptance rules.

An Apple Silicon baseline build uses a fresh directory:

```sh
CARGO_TARGET_DIR=target/template-efficiency-baseline \
  python3 tools/setup-native.py -- cargo bench --locked \
  --config 'profile.bench.build-override.strip="none"' \
  --package mado-pilot --features template-matching-benchmark \
  --bench template-matching-efficiency --no-run
```

Windows uses its native x64 MSVC/OpenCV environment and a separate build directory.
Never reuse a hash-bound baseline or candidate directory for later Cargo checks;
verification has its own `CARGO_TARGET_DIR`.

### Normal and instrumented builds

The separate `template-matching-timing` feature builds normal release code with
the system allocator and no matching/mapping work counters. Invoke its emitted
executable with `--protocol-version 4`, its default and only accepted value. The
collector also parses protocol-1/2/3 executable output for retained campaigns,
which require their recorded collector and policy for revalidation; current
executables cannot reproduce those protocols.
Cargo feature unification that enables OpenCV or capture instrumentation makes a
normal benchmark fail at compile time, including direct dependency features.

Protocols 3 and 4 record the compiled `build_mode`, allocator label and an explicit
`unavailable` declaration. Both modes run 41 statistical rows; normal adds 13
public-result contracts and instrumented adds 16 contracts. Normal's 26 unobserved
heap/work/owner fields are `null`, not zero. The format-transition and two watcher
counter-oracle rows remain instrumented-only; requesting one from a normal
executable is an error. Both modes share semantic fixtures and public-result
oracles. Instrumented owner/work proof does not establish a normal-build resource
or timing verdict.

Build each mode in its own Cargo directory. The build command above selects
instrumented mode; substitute `template-matching-timing` and a distinct
directory for normal mode. The existing host build-dependency strip override
avoids [Rust issue 157750](https://github.com/rust-lang/rust/issues/157750) on
macOS 27 without changing measured-code optimization, allocator or deployment
minimum. Keep that configuration in each build receipt.

The component protocols observe `cv::currentParallelFramework` and `cv::getNumThreads` before
and after work through a benchmark-only, exception-contained C++ boundary. It
does not set limits. The value is the library-reported parallel-region setting,
not actual worker utilization. Unavailable, unsupported GCD, invalid or changed
observations refuse success while retaining their reasons and workload JSON.
Build information is not substituted for the active backend.
Roles must share the same backend identifier and thread setting; collection and
retained comparison validation refuse differences between roles.

### Protocol-3 Session corpus

New rows request `0.9/8/DropOverlapping` on packed RGBA and BGRA, with full-frame
and nonzero-ROI cases. These are explicit workload options, not inferred consumer
defaults. Hits name the planted template, bounds and complete source-frame
identity; misses return an empty collection without an invented absent score.
Existing component guards keep their input bytes, options and timing boundaries.

| Family | Semantic unit and measured boundary |
|---|---|
| Retained hit/miss and `64×48` patch sensitivity | One live `Session::find_template` call through bounded scalar projection and outcome release; the frame/backend persist across units. |
| Eight-template groups | Eight sequential public searches on full, same-ROI or distinct-ROI keys. The compatible-owner variant separately records mapping creation/release outside the group interval. |
| New-frame turnover | Exactly `M,H0,M,H1`; one publication/acquisition-to-final-release cycle plus separate hit/miss search strata. Their overlapping times are never added or treated as independent processes. |
| Matched query | Fresh provider/engine/session/query with shared row backend/templates; producer release through actual wait, terminal projection and producer join. Public completed-work receipts gate `M → H0 → H1`; the third frame supplies two confirmed hits. |
| Deadline query | Publish `M`, prove completed no-match, then require the real query-lifetime deadline. Caller-wait interruption is not that terminal result. |
| Owner lifecycles | Complete Session-close/retained-value/refused-search script, or real Matcher calls through A, A-clone and independent B followed by B reuse after A release. |

Query setup and close/retained-terminal/query-result/runtime release are separately
timed outside primary. Lifecycle contracts time their complete owner script.
Owner checkpoints preserve RSS, peak RSS, private bytes and footprint where
available, without inventing a calibrated per-checkpoint envelope. Instrumented
independent-backend checkpoints require `49,766,400 → 24,883,200 → 0` BGR payload
bytes for the prepared-cache role; only that row permits two simultaneous
32-MiB cache allowances. The uncached role reports zero cache payload, not zero
temporary native allocation. Growth and final-owner residual receive no extra
allowance.

Logical fixture identity includes inputs, options, row order and timing/owner
contracts, not helper source hashes or implementation-specific work counts.
Each role separately binds the exact 11-source `apparatus` manifest to its
executable and attestation; it must remain unchanged within that role.

| Accounting contract | Physical work and ownership |
|---|---|
| `prepared-cache-row-winner-v1` | Direct RGBA/BGRA preparation, bounded prepared-image reuse and row-winner selection. |
| `uncached-bgra-repeated-scan-v1` | Historical BGRA mapping, per-call BGR conversion and repeated response scans. Cache gauges remain zero; an active reader holds its real local BGR matrix without an eviction claim. |

Both contracts validate mapped-view bytes separately from mapping-copy and BGR
conversion bytes. Common-manifest comparison removes only admitted role-specific
physical metadata; inputs, public oracles and timing/owner boundaries must agree.
The new rows add regression/sensitivity coverage; they do not waive the existing
selection/preparation improvement requirements or establish consumer performance.

### Component collection

The source-bound collector separates these stages:

| Command | Evidence and admission |
|---|---|
| `attest` | Bounded compiled-mode/manifest, source/toolchain probes and loaded OpenCV endpoint identities; not full runtime closure |
| `runtime-preflight` | macOS-only representative discovery over every selected path; retains raw loader records, file bindings, timed-region markers and a frozen union; not acceptance timing |
| `calibrate` | Six baseline processes under an accepted, hash-bound source/host/cost/observer-eligibility review and runtime prerequisites |
| `control` | Independent balanced six-pair unchanged A/A cohort; incomplete calibration prerequisites prevent launch and a passing report |
| `freeze` | Separate accepted numerical review binds the exact method, calibration/control receipts and envelopes before candidate data |
| `compare` | Six balanced baseline/candidate pairs against that immutable profile; normal p95 and CPU benefits remain joint requirements |

Preflight and each later observation bind the executable, declared dependencies,
compiled mode, host/environment, helper/collector sources and effective native
thread setting. Each observed run reserves its pre/post binding time separately
from the child deadline. Raw output, failed outcomes and unrun schedule entries
remain evidence; successful cleanup does not repair missing provenance.

New macOS campaigns bind `macos-dyld-observer-on-warm-v1` in their method review,
plan and profile. The benchmark's opt-in `--timing-boundaries` emits
`mado-component-timing-v1` records for `row-start`, `samples-begin` and
`samples-end`, naming the actual process and workload. Each bounded record uses
one stderr write outside individual sample clocks. Default-off invocations emit
no markers; manifest output declares capability and actual enablement without
emitting row markers.

The collector re-derives ordered boundaries from retained stderr and the owned
root PID, counting every `dyld[` record, including duplicate and informational
records. Statistical rows require complete markers and zero records inside the
enclosing retained-sample window; non-statistical rows retain their counts but
still require complete boundaries. Missing, torn, foreign or reordered markers
are not empty windows. A stored zero cannot override the raw stream.
Incapable binaries and image-only historical bundles cannot authorize this policy.
Historical readers retain their original, narrower meaning.

Zero in-window log records establishes their location in the retained stream,
not zero loader activity, zero observer cost or observer-off equivalence.
Startup, prehash warming, pipe processing and scheduling effects still require
the external method review. All roles use the same declared observer-on runtime;
normal remains the system-allocator/no-accounting build mode.
[ADR 0095](adr/0095-component-timed-loader-boundaries.md) records this evidence
contract without accepting a numerical profile.

Prospective numerical policy revisions `template-efficiency-v2-prospective-r2`
and `template-efficiency-v3-session-prospective-r2` require the absolute
between-role A/A median difference to stay within the frozen allowance for
every graded timing metric. Control and comparison independently enforce the
15% cohort noise cap for each role, including turnover hit/miss strata, using
the metric's frozen maximum calibration resolution and existing target CPU
floor. Later role observations cannot enlarge that allowance. Equality passes
both gates. Known failures remain failures when another role, metric or clock
observation is incomplete; a passing median benefit never overrides them.
Sampling, ceilings, benefit allowances and resource budgets are unchanged.
Existing numerical evidence remains bound to its original collector and policy
and cannot authorize the revised policy. New campaigns require fresh
collector-bound preflights, method review, calibration, control and numerical review.
See [ADR 0096](adr/0096-component-control-and-comparison-gates.md).

The shared owned-process runner adds lifetime `cpu` accounting without changing
timeout, output, root-exit or cleanup decisions or their deadlines. POSIX collects
the root's owning `wait4` usage at reap and reports
`root-and-native-descendant-rollup` as `partial`, even when final. Windows uses
the existing Job user/kernel totals, including nested jobs; `complete` requires
observing zero active processes. Unavailable usage remains `unknown` with null
values. Microsecond and 100-ns source representation units do not establish
resolution or accuracy: `accounting_resolution_seconds` remains `null`. This
accounting does not establish full POSIX-tree coverage or performance qualification.

Each runner receipt also has `process_accounting`. It is `null` outside Windows
or when launch did not complete. Windows retains raw `total`, `active` and
`terminated` counters from the same existing cleanup query as CPU, before closing
the anonymous owned Job. The receipt's `process_id` binds its root; `source`,
`scope` and `checkpoint` identify the native aggregate observation, not a PID list.
Nested Job totals are not added again and OS-created members are not excluded.
[`TotalProcesses`](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information)
includes lifetime associations, including failed limit-violation associations;
`terminated` counts limit violations, not all exits or `total - active`.

`query_succeeded` describes the last attempted query. A later query failure
preserves an earlier successful snapshot as `partial`. Without an observation,
status is `unknown` and counters are null, never an invented empty Job. `complete`/`final`
requires a successful zero-active observation without an accounting failure.
These facts do not change timeout, nonzero exit or cleanup verdicts. They do not
establish a process-generation/role census, a process cap, image closure or
qualification, and cannot reconstruct counters omitted by historical receipts.

Protocol-3/4 runtime preflight stores the complete binary/source identity once in
`plan.binary`. Successful before/after file checks retain its canonical-JSON
SHA256 as `plan_binary_sha256`; unobserved boundaries remain null. Admission
checks both references and the full retained proof within the unchanged 64 MiB
bundle ceiling. A fully verified `exported-git-tree-v1` binding is admissible;
ordinary checkout provenance still requires a clean status. Protocol-2 preflight
and other campaign receipt formats retain their existing identities.

Attempt reports distinguish execution, cleanup, file identity, runtime provenance
and semantic failures. Post-run file identity is retained before receipt validation.
Diagnostic smoke may continue after a fully bound row failure; that failure
never enters accepted scalar cohorts. Unsafe cleanup, identity or
runtime-provenance failure stops later launches; failed manifest discovery also
stops sampling. Current Apple timing-policy campaigns additionally stop on
execution or semantic failure. Historical receipts retain their recorded
collector and policy interpretation.

For macOS preflight, use the same absolute native Python path on both sides of
setup so child `PATH` staging cannot select another interpreter:

```sh
"$NATIVE_PYTHON" tools/setup-native.py -- "$NATIVE_PYTHON" \
  tools/template-matching-efficiency.py runtime-preflight \
  --protocol-version 4 --build-mode normal --target aarch64-apple-darwin \
  --binary "$NORMAL_BINARY" --build-metadata "$BUILD_METADATA" \
  --output "$NEW_PREFLIGHT_DIRECTORY"
```

The macOS method records loader loads/reloads, not unload chronology or manual
mappings. It rejects missing/foreign/unrecognized records and uses observed
shared-cache membership rather than a system-path exemption. Its loader UUID
and pre/post on-disk bindings require a reviewed stable-file/OS scope; they are
not proof against arbitrary concurrent replacement or same-UUID mutation.
Its coverage review binds the host's `dyld(1)` contract and OS identity; the
public dyld source reference is not asserted to match the running loader.
Formatting, stderr writes and possible pipe/scheduling contention are observer
effects requiring prospective assessment, not costs subtracted from samples.
The Windows held-file debugger observer is preflight-only: it is not admitted
for normal timing. Windows normal qualification therefore remains closed.

The macOS native observer suite uses owned loader/dylib fixtures for transient
reloads, vanished-image refusal and foreign-process rejection. CI enables these
fixtures; run them locally with a C compiler available:

```sh
MADO_MACOS_IMAGE_OBSERVER_SMOKE=1 python3 -B -m unittest discover \
  -s tools/tests -p test_macos_image_observer.py -v
```

Native discovery, semantic smoke and CI are not observer-cost or performance
qualification. The external method review must explicitly accept the selected
observer's coverage and perturbation assumptions; getters may initialize native
state, and logging/prehashing can affect execution and caches. Numerical budgets
need their own subsequent review. [ADR 0094](adr/0094-normal-and-instrumented-component-measurement.md)
defines these apparatus boundaries without changing historical format-1 profiles
or accepting a target-local performance claim.

Recorder and process-coverage restrictions apply only to the named benchmark
campaign's claims, not to library consumers' target, window or child-process
choices. Applications select their own scope and own consumer-created children.
The SDK remains responsible for its documented parameter contracts and the
resources and workers it creates. No application is required to adopt these
recorders, and component evidence does not guarantee arbitrary consumer behavior.

### Incremental collection

`collection_steps(args, mode)` uses the ordinary parser's namespace and yields
after each saved attempt, launching at most one benchmark process per advance.
Each checkpoint names the attempt, its raw file and the original absolute
campaign deadline. A management owner can inspect the retained prefix before
advancing again; pauses consume that same deadline.

Keep the iterator inside `contextlib.closing`. It holds the exclusive collector
lock until exhaustion or close. Early close preserves every completed or started
slot; `report` classifies the missing suffix as incomplete. There is no persisted
resume, retry or replacement-sample API. Exhaustion returns the final report in
`StopIteration.value`; `collect` and the CLI still consume the full schedule.

For numerical collection, a reviewed, source-bound management owner starts its
absolute whole-command clock before admission and covers inspection, evaluation
and retention. The collector clock covers collection; the existing process
helpers bound individual owned invocations. Neither supplies the outer
management deadline by itself.
Checkpoints do not change workload counts, numerical rules, native ownership or
method-review requirements, and are not crash-durability acknowledgments. A new
collector identity requires fresh applicable method/source binding before any
qualification campaign.

External profile reviews, comparison admission and retained reports verify the
digest of the exact JSON bytes they parse. Comparison retains that same verified
snapshot. Replacing a file between digest verification and parsing cannot
authorize different content; a mismatch refuses the profile before use.
Historical campaigns remain bound to their recorded collector revision.

Cohort plans and started receipts also bind interpretation and digest to one
read snapshot. Plan creation hashes the exact bytes written. Malformed started
receipts retain their read-byte digest as failed evidence without discarding
other attempts; non-object plans are validation failures.

### Apple observer-on component campaign

[ADR 0097](adr/0097-apple-observer-on-component-campaign.md) accepts the
source-bound method after actual management-owner smoke and concrete initial
host/storage admission. It covers only the Apple Silicon single-process,
capture-free warm component lane under `macos-dyld-observer-on-warm-v1`, with
separate normal and instrumented calibration, A/A control and comparison:
six collection stages. Every stage requires fresh admission.
Each mode requires its own accepted numerical-profile review after calibration
and control, before comparison. Normal p95/CPU benefit and instrumented mechanism
evidence remain separate; no measurement or qualification is accepted here.

The fixed protocol-3 schedule retains 41 statistical rows at 20/200 warmups and
samples, with 13 normal or 16 instrumented contract rows at 1/3. Maximum launches
are 325/650/650 and 343/686/686, including manifests. Each stage retains the
300-second child, 8-MiB combined raw-output, five-second cleanup and 120-second
per-pass image-binding limits. Independent collection clocks are 21,600 seconds
for calibration and 43,200 for control/comparison. Whole-owner execution
windows are 23,400/45,000 seconds with a separate 65-second cleanup/retention
tail; these bounds promise neither completion nor complete persistence.

The private owner calls the exact collector in-process and reuses its
owned-process runner, deferring deadline interruption while native ownership and
cleanup are active. It must retain late/failing child receipts without retry or
success promotion. Whole-owner/OS loss and non-returning OS primitives remain
outside the cleanup guarantee. Apple timing-policy prerequisite failures stop
later launches; known failures and unrun slots remain evidence.

Admission must count serialized evidence, fixed metadata and owner receipts,
not just raw pipe bytes. Fresh free-space checks and post-stage inventories are
not filesystem reservations or quotas. Operator-controlled host exclusivity
does not exclude unrelated writers; the collector lock is not a host lock.
RSS high-water is observed only at apparatus checkpoints, not necessarily at
the final process-lifetime peak; these observations exclude management-owner
memory. Rust heap accounting does not cover all native allocations.

Final hash-bound method artifacts and subsequent profile reviews govern each
campaign. Historical evidence remains immutable. Startup, observer-off, Windows
normal observation, actual-consumer qualification and the whole performance
Change remain incomplete; campaign no-spawn assumptions impose no SDK or
consumer restriction.

The admitted normal calibration has no accepted numerical profile. Its first
stored CPU-repeatability failure is `single-small96x64/cpu_ns`; evaluation stops
there, without declaring other rows clean. An independent offline audit also
finds CPU-noise failures for `session-owner8-bgra-roi` and `query-match-rgba-roi`,
without rerunning or regrading the campaign. Successful process cleanup and
immutable input bindings do not validate noisy timings. A/A control and candidate
comparison remain unrun; no product regression or benefit follows from this
calibration failure.

### Workload-scoped component cohorts

[ADR 0098](adr/0098-workload-scoped-component-cohorts.md) refines collection
granularity for newly bound protocol-3 scopes, not ADR 0097's historical method
or failed calibration. `tools/template-matching-efficiency.py` accepts
`--workload` under protocol 3 or 4 on `calibrate`, `control` and `compare` to select
one complete workload. Omitting `--workload` retains the exact ordered full corpus:
54 normal or 57 instrumented rows. Arbitrary subsets, duplicate IDs, wrong-mode
workloads and older-protocol non-smoke selections are refused. Smoke is unchanged.

Under protocol 3, each statistical workload retains 20 warmups and 200 retained
units per process; protocol 4 uses its frozen sampling plan (below). Both keep
contract workloads at 1/3. A selected calibration runs one manifest and six
baseline processes. Control runs two manifests and six independent unchanged A/A
pairs; comparison runs two manifests and six fresh baseline/candidate pairs.
Both paired stages retain `AB, BA, AB, BA, AB, BA`. There is no automatic stage
chain, cohort aggregation, retry, resume or replacement sampling.

The source/hash-bound method review fixes all three `stage_limits` before
collection. For a single workload, only `campaign_timeout_seconds` may be lower
than the existing full-stage ceiling: a positive finite integer at most 21,600
seconds for calibration or 43,200 for control/comparison. The accepted review
chooses case-specific clocks from workload cost, not a blanket ten-minute limit
or universal 900-second policy. All other limits equal `execution_limits` for the
exact schedule, retaining the 300-second child, 8-MiB combined output, five-second
cleanup and 120-second per-pass image-binding bounds. Pauses consume the same
live absolute stage deadline; no enlargement or reset is permitted. Management
execution, serialized storage and retention still require separate admission.

`plan.workloads`, profile `workloads`, the external numerical-profile review and
comparison must bind the same exact scope. The partial `method_review` binding
retains the validated `stage_limits`; `plan.execution_limits` must equal the
selected stage's limits. Changed scope, cap or profile envelope coverage refuses
before native launch. Raw slots, failures, observer/source gates, noise/A/A gates,
frozen allowances and per-process timing/resource ceilings remain mandatory.

Protocol-3/4 reports expose `workload_coverage.required`, `.selected` and `.omitted`
as workload ID lists and `.complete` as a boolean against the full mode-specific
corpus. `status` describes only the declared cohort. A partial comparison may
report `status: "passed"` while `performance_qualified` remains `false`;
`targets[target]` and the applicable `normal_timing_verdict` or
`heap_and_work_verdict` remain `incomplete`, not whole-corpus `passed`.
Known failures remain `failed`; unavailable observations remain unavailable.
Omitted rows do not become missing attempts within a complete selected cohort.

Both normal selection and repeated-preparation goals still require joint p95
and CPU benefit above frozen allowances: summed retained-sample process CPU under
protocol 3, or the retained-loop bracket under protocol 4.
Instrumented work/ownership evidence cannot replace either goal. Whole-target,
Change and release claims still require every required case, per-process/resource
non-regression, both targets and build modes, and separate actual-consumer
evidence. Neither a selected profile nor a passing scoped report supplies that
qualification or authorizes a new native launch.

### Protocol-4 retained-loop CPU and frozen sampling

[ADR 0100](adr/0100-retained-loop-cpu-and-prospective-sampling.md) defines
prospective protocol 4. Its current policy is
`template-efficiency-v4-retained-loop-control-budget-prospective-r2`, specified
by [ADR 0103](adr/0103-component-control-error-allocation.md). Corpus, oracles,
timing boundaries, Session strata and the build-mode split are unchanged from
protocol 3.

Every observed row adds `retained_loop_cpu`, a single current-process CPU
bracket:

- It reads CPU once immediately after `samples-begin` and once after the last
  retained sample is stored, before `samples-end`.
- Its scope, `current-process-retained-loop-including-preparation-oracles-and-storage`,
  covers per-iteration preparation, the measured operation, public-result oracles
  and sample storage on all process threads. Setup, warmups, post-warmup memory
  observation, markers and serialization are excluded.
- macOS reads `getrusage(RUSAGE_SELF)` user plus system time
  (`source_unit_ns: 1000`). Windows reads current-process `GetProcessTimes`
  kernel plus user time (`source_unit_ns: 100`). These units are source
  representation, not resolution or accuracy.
- A missing or reversed bracket is unavailable with a reason, never zero, and its
  observed row exits nonzero.

Normal protocol-4 grading uses nearest-rank process p95 and
`retained_loop_cpu_ns`; turnover rows also grade stratum p95. `metrics.cpu_ns`
keeps its summed-sample meaning as a diagnostic, and the bracket may not fall
below it. The calibration noise and per-process ceilings remain those of ADR 0096,
as do the existing CPU accounting floors. Each comparison role must also follow
`AB, BA` positions and pass the first/second-position and first/last-three median
checks. Instrumented protocol-4 timing and CPU are descriptive. Their resource
ceilings and exact work/ownership checks remain graded and cannot set
`performance_qualified`.

For each normal timing metric independently, the empirical allowance is
`e=max(spread(calibration6),2*clock_allowance)` and the per-process ceiling stays
`max(calibration6)+e`. The control allowance is
`c=max(e,ceil(median(calibration6)*15/100))`; it bounds unchanged A/A role bias
and within-role order/drift in control and comparison. Benefit rows require
strict improvement above `b=max(e,c)` jointly on p95 and retained-loop CPU.
The three allowances are stored separately and recomputed with exact
half-nanosecond median arithmetic; `c` and `b` never enlarge a ceiling.

The 15% is an engineering sensitivity contract, not statistical coverage.
When a role's median does not exceed the calibration median, its order/drift
checks are implied by the noise gate and add no independent detection.
Effects at or below `b` are not claimable. The protocol-4 policy change requires
fresh source-bound preflight, method, calibration, unchanged A/A and profile
review. Earlier policy artifacts and failed controls are not converted or
regraded. The prospective policy alone admits no numerical execution.

The benchmark accepts paired `--warmups W --samples N` only for one statistical
`--workload`, with W in `1..=1000` and N in `1..=40000`. Default and `--list`
manifests keep the logical reference counts: 20/200 for statistical rows and 1/3
for contract rows. Smoke remains 1/3, and records name the counts actually run.

Protocol-4 `calibrate`, `control` and `compare` require a pinned plan. The plan is
UTF-8 JSON of at most 64 KiB that maps exactly the selected workloads to
`{"warmups": W, "samples": N}`; contract rows must be `{"warmups": 1, "samples": 3}`.
For one selected workload:

```sh
"$PYTHON" tools/setup-native.py -- "$PYTHON" tools/template-matching-efficiency.py calibrate \
  --protocol-version 4 --build-mode normal --target "$TARGET" \
  --workload single-small96x64 \
  --sampling-plan "$SAMPLING_PLAN" --sampling-plan-sha256 "$SAMPLING_PLAN_SHA256" \
  --method-review "$METHOD_REVIEW" --method-review-sha256 "$METHOD_REVIEW_SHA256" \
  --baseline "$NORMAL_BINARY" --baseline-metadata "$BUILD_METADATA" \
  --exclusive-host --output "$NEW_CALIBRATION_DIRECTORY"
```

Smoke refuses a plan. The method review, calibration, control, numerical-profile
review, frozen profile and comparison must carry the identical embedded plan, and
changed counts refuse before collection.

Parser bounds are not adopted counts, cost proofs or output guarantees. Each count
is a reviewed prospective input fixed before calibration. The 300-second child,
8-MiB output and five-second cleanup bounds still fail closed, and ADR 0098 stage
clocks still come from the method review. Automatic count selection, adaptive
stopping, resampling, retry, post-result changes and CPU normalization across
counts are not permitted.

Protocol-1/2/3 evidence, including ADR 0097's failed calibration, is not
converted or regraded. A protocol-4 campaign needs its own method review,
collector-bound preflight, calibration, control and profile review. Windows normal
image closure remains unresolved, and both joint p95/CPU goals remain required.

A development smoke on the Apple Silicon host invoked the current normal and
instrumented executables directly through the owned-process runner:

- Their manifests listed 54 and 57 rows.
- Each mode ran three selected rows with timing boundaries and loader logging:
  `single-small96x64` at 2/7, `session-retained-rgba-roi-hit` at 2/5 and contract
  `query-deadline-rgba-roi` at 1/3.
- All six rows had zero oracle failures, valid `getrusage` brackets, three
  same-process boundary markers and zero in-window loader records.
- Unpaired counts and `--samples 40001` exited 1 before benchmark work.
- All ten processes cleaned up.

This is development evidence only. It used no collector sampling plan and is not
whole-corpus collection, complete image-file closure or qualification.

### p050 host observations

[ADR 0106](adr/0106-observational-host-readiness.md) makes CPU idle percentages,
transient host-process CPU and OS background activity optional descriptive
observations for new p050 scopes. Neither their values nor their unavailability
may refuse launch, trigger quiet resampling or start replacement campaign loops.
Do not intentionally overlap owned builds, tests or independent timing work.
Source/binary/resource identity, actual ownership and cleanup, correctness,
deadlines, output/storage bounds and campaign fences remain mandatory.
Required owned-invocation CPU accounting is not an optional host observation.

Uncertain OS-sensitive empirical numerical qualification is not a prerequisite
to development, implementation closure or release-decision progress. The
numerical rules below remain unchanged for optional qualification reports and
performance claims; replacement calibration/controls or tuning until pass are
not required to close implementation work. Deterministic correctness, safety,
provenance, compatibility and required CI remain mandatory.

Successor plans and receipts must name `ADR 0106` as their host-readiness policy
separately from their numerical method/profile. Historical 90% readiness
refusals, including 89.69% and 85.82% idle, retain their original evidence and
unrun stages; they are not consumer failures or regraded passes.
The current trial-v4 continuation retains its collector/context/method and passing
calibration bytes. An explicit ADR 0106 applicability supplement supersedes only
the `exclusive_host` gate's outer CPU/process-readiness requirement and accompanies
the new result. Reuse the bounded owner with one new stage plan, deadlines and
output path under the existing owner root; do not add a CPU probe or recalibrate
merely to remove this prelaunch predicate. Numerical grading remains unchanged.

Numerical variability, benefit and inconclusive outcomes retain their
source-bound rules and failure fences; do not rerun a failed control or bypass
its fence for implementation closure. Removing these gates establishes neither
accepted performance nor release approval or all-target benefit.
`P050-OS-NOISE` in the independent planning repository's `rasen/backlog.md` owns
unresolved noise causality, OS-independent repeatability and numerical
qualification follow-up. Deferral does not permit unsupported performance claims
or require tuning until pass.

The trial-v4 control under the unchanged numerical method stopped after its first
invocation exceeded the frozen lifetime-CPU ceiling; semantics, runtime checks
and cleanup passed. Eleven slots remain unrun and the failed control fence is
retained without numerical retry. This outcome is distinct from the current
deferred release gate and establishes no OS-noise cause; ADR 0106 records its
scope and evidence.

### Actual-consumer scalar method

[ADR 0099](adr/0099-actual-consumer-scalar-measurement.md) defines the scalar
method. [ADR 0105](adr/0105-central-four-fresh-trial-repeatability.md) selects
`consumer-method-review-v4` for new admission and succeeds ADR 0104's full-six
trial repeatability screen. The collector rejects v1/v2/v3 reviews rather than
reinterpreting historical receipts or profiles. Method implementation alone
accepts no contextual method or numerical profile.
Preserve the actual desktop-selected dev profile (`opt_level=0`, `debuginfo=2`,
debug assertions), with benchmark accounting absent and
`MADO_P050_OBSERVATIONS=1`. This is distinct from component release timing and
from observation-off. Current source/binary/lock/resource substitutions must be
bound independently for baseline and candidate; an embedded revision constant
does not identify the actual linked producer.

Keep all twelve fixed cases and their exact RGBA, full/nonzero-ROI, retained
observation, frame-sequence and result oracles:

| Cases | Primary decision | Required independent observations |
| --- | --- | --- |
| Replay full / ROI | Non-regression | Exported startup, child preflight/workflow, successful cleanup and whole-invocation elapsed/CPU |
| Retained eight-template group full / ROI | Group-span scalar benefit | Each fixed call position/outcome, lifecycle and whole-invocation CPU non-regression |
| Matched query wait full / ROI | Caller-wait scalar benefit | Source/result correlation, settlement and whole-invocation CPU non-regression |
| Deadline query wait full / ROI | Semantics and non-regression, not speed benefit | Expected application `FAIL`/`Timeout`/`deadline_exceeded`/exit1 remains a failure; retain actual return/settlement overrun and clean teardown |
| Fresh saved-RGBA trial hit / miss × full / ROI | Fixed operation responsiveness and observed non-regression, not positive benefit | Mandatory `operation_us`, real selected OCR initialization, stage/settlement observations and whole-invocation CPU non-regression |

These cases neither replace both component joint p95/CPU goals nor add a
requirement to improve unrelated initialization in every consumer case.
Saved-RGBA completion does not measure dispatch-to-visible UI completion;
unestablished UI/native-capture boundaries remain not run.

#### Observable units and clocks

Each scalar-primary invocation supplies one group, wait or trial-operation
scalar, not a process-local latency distribution. Report six same-case values, their
median, minimum and maximum. Do not label the six-process maximum a process-local
p95 or pool heterogeneous calls, roles, outcomes, targets or configurations.

| Observation | Actual boundary and availability |
| --- | --- |
| Host call / query wait | Observation-on raw `control.started` ns; one call through terminal-fault retention, before aggregate insertion and JS return. Internal SDK search timing is not inferred. Off has integer-us aggregates, not equivalent raw call records. |
| Retained group | Observation-on first recognize start through eighth release end, one child epoch. Includes intervening JS/bookkeeping and fifteen preceding call-record insertions. Off has no group-span observation. |
| Replay parent endpoints | Exported `startup_us` records receipt of `ChildStarted`; `elapsed_us` ends after writer/reader/stderr joins. Include actual polling, refresh, delivery/drain and scheduling delay. Startup can be recorded during post-exit drain. Internal `exit_us` is not exported; failure-only `containment_us` is not a success endpoint. |
| Replay child endpoints | `preflight_us` and `workflow_us` are child-local `Instant` durations. Workflow includes Node import inspection; parent detection delay is not added. |
| Replay cleanup | Child `Host::finish` starts after admission closure/optional cancellation and ends at evaluation of the reported `elapsed_us`. Includes child-local polling, but excludes later result fields and receipt-permit release. Cached clean results are not new samples; incomplete cleanup fails its oracle. This is not final-owner teardown. |
| Trial operation / stages | Primary `operation.total_duration_us` starts immediately before `recognition_trial` and ends after terminal polling, public shutdown and final drain. Input decoding/hash and controller construction are excluded; top-level `total_duration_us` includes them and is not the primary. Observation-on child initialization/matching/projection/settlement/close/release marks and per-child parent marks retain separate epochs; never subtract across them. Off lacks these marks and capability-child identity. |
| Lifetime CPU | Final owned-invocation user+system CPU includes its declared startup, descendants and teardown, denominator one invocation. Require complete coverage and independent non-regression. It is not operation CPU, wall utilization or a sum with already-included child CPU. |

Bind each metric's API, stored unit, PID/owner, interval or checkpoint and
missing reason before calibration. Timing/CPU additionally bind reporting
quantum and reviewed resolution or accounting allowance. Resident-byte
contracts do not invent timer resolution.
Ns/us representation, QPC frequency and Job100ns units do not alone prove
resolution or accuracy. Requested5ms or1ms polling sleeps are neither clock
resolution nor upper detection-lag bounds. Boundary work remains in its observed
interval; no cadence subtraction, inferred event time or overlapping-stage sum
is permitted. Apple raw `posix-wait4` coverage remains partial; Windows final
Job coverage is complete but its accounting resolution is unavailable. Raw
records alone do not supply numerical CPU admission; source-closed coverage
and a separately reviewed accounting allowance remain distinct evidence.

#### Scalar calibration, controls and comparison

For each case/target/configuration, retain six fresh baseline calibrations, six
unchanged-baseline A/A pairs and six B/C pairs. Paired stages use balanced
`AB,BA,AB,BA,AB,BA` order. This is30 attempts per case (24 baseline, six
candidate),360 per target and720 across the two targets for the selected
dev-normal/observation-on configuration. Off or another build mode needs a
separate prospective contract and is not hidden in those totals.

Resident metrics use the separate two-axis resource rule below. Trial
`operation_us` uses the fixed responsiveness policy below, not this percentage
noise rule or sample-derived ceiling. For every other graded timing/CPU metric,
let `B` be its six baseline values, `q` its reporting quantum and `r` the maximum
effective calibration resolution in the same unit, or its separately reviewed
accounting allowance. Use exact ticks or a lossless decimal representation:

- Freeze `s=max(B)-min(B)`, `a=max(s,2*r)` and
  `ceiling=q*ceil((max(B)+a)/q)`.
- Reject calibration when `s/max(median(B),2*r)>0.15`.
- Every control/comparison invocation must meet its own ceiling and all semantic,
  ownership and resource oracles. A good median cannot cancel one failure.
- Each role independently meets the same noise rule with frozen `r`. The absolute
  A/A role-median difference must be `<=a`. Within each A/A and comparison role,
  both the absolute first/second-position median difference and the absolute
  chronological first/last-three median difference must be `<=a`.
- A declared scalar benefit requires `median(Bcomparison)-median(Ccomparison)>a`.
  Lifetime CPU and separately selected endpoints independently meet non-regression
  rules. An additional lifetime-CPU benefit requires its own median difference
  above its allowance; it is not operation-local CPU attribution.

Unknown required coverage/resolution is unavailable, not zero. A later coarser
clock cannot enlarge frozen allowances. Valid calibration, controls and explicit
profile acceptance precede candidate collection; no retries, replaced samples,
post-result ceilings or historical regrading are allowed. The sole aggregate
exception is the prospective trial central-four range below; every raw
observation remains retained and all other policies keep their original inputs.

Identity/cleanup failure persists at the target/host campaign-management tier,
outside restartable phase owners. Every new phase checks that fence; all lanes
sharing affected identity/host conditions lose applicability until an explicit
source-bound cleanup/applicability disposition. Control failure blocks that
target/lane's candidate collection, not only one case. Keep known failures
visible alongside incomplete prerequisites and unrun work.

#### Fresh-trial responsiveness and non-regression

Every `trial-*` operation contract requires
`policy="fresh-trial-central-four-v1"` and
`population="fresh-process-first-operation"`. Declare `hard_ceiling_us` and
`max_regression_us` as positive exact JSON integers; Boolean, floating-point
and string budgets are invalid. Require
`max_regression_us <= hard_ceiling_us`, with `hard_ceiling_us` strictly below
`bounds.primary_seconds * 1_000_000`. Keep positive `quantum` and `resolution`,
a nonempty `resolution_method`, and `2*resolution <= max_regression_us`.
These fields have no numeric defaults. The timeout bound is only a sanity check,
not a source for either user-response budget.

At least one accepted, hash-bound `resources` evidence item must contain
`trial_responsiveness` with the identical policy, population and two budgets,
plus a nonempty justification. Every such declaration must agree and retain
the exact `context_sha256`, matching source-file bindings, `gate="resources"`
and `complete=true`. Reject this trial-only policy on nontrial contracts or
evidence. The justification must relate both budgets to intended user response,
build and observation configuration, not fit the failed cohort or reuse an
execution timeout. Missing, malformed or mismatched evidence blocks numerical
collection before a target starts.

Every operation observation must be `<= hard_ceiling_us`: smoke when a review
is supplied, calibration, both control roles and both comparison roles.
One excess is numerical `NONPASS`, independently of every other row or median.
Correctness, runtime identity and cleanup failures remain primary and retained.

Keep exactly six complete fresh calibration values, including the first.
Sort exact `operation_us` values by value and original schedule order; exclude
one minimum and one maximum occurrence from repeatability only. Ties choose
the earliest minimum and latest maximum; all-equal values still exclude two
distinct occurrences. The first invocation is not automatically excluded.
Missing, failed or partial observations cannot form a replacement cohort.

The trial envelope retains policy, population, both budgets, clock metadata
and full-six minimum, median, maximum and spread. It separately reports the
central-four values, spread, mean and median, with excluded values/indexes.
Statistics bind these indexes to the role's original values and complete slot
objects; the phase retains the corresponding raw references. The central-four
and full-six medians are identical. The central-four mean is descriptive only.
Reports without a current method review omit the successor aggregate.

The ceiling is exactly `hard_ceiling_us` and allowance exactly
`max_regression_us`; neither is derived from calibration or rounded upwards.
Central-four spread above that allowance is inconclusive repeatability, not a
candidate regression. No percentage noise gate applies to this endpoint.

Passing control and comparison require all six balanced pairs in
`AB,BA,AB,BA,AB,BA` order without pooling, replacement or exclusion. With
`a=max_regression_us`, apply these exact checks:

| Scope | Required bound | Excess |
| --- | --- | --- |
| Each role separately | Central-four range `<= a` | Inconclusive repeatability |
| Each role's three first-position and three second-position values | Absolute median difference `<= a` | Inconclusive order effect |
| Each role's chronological first three and last three values | Absolute median difference `<= a` | Inconclusive drift |
| Unchanged A/A | Every corresponding pair's absolute difference and the absolute role-median difference `<= a` | Inconclusive control; profile/candidate admission blocked |
| Baseline/candidate comparison | Every pair's `candidate-baseline <= a` and `median(candidate)-median(baseline) <= a` | Numerical observed non-regression-budget violation |

Equality passes; an excess of one microsecond fails the corresponding bound.
Faster candidates need no minimum gain. Retain known per-row or comparative
numerical violations alongside any independent inconclusive findings; neither
can erase the other. These are observed finite-cohort decisions, not causal SDK
regression proof, confidence/prediction intervals, clock-accuracy evidence or
population-wide guarantees. The same fixed tolerance governs the central-four
screen, controls and non-regression; no empirical widening is allowed.

This screen is weaker than full-six repeatability. The ceiling and pair checks
remain, but median/order/drift checks do not recover extreme magnitude. A slow
baseline maximum may mask its comparison partner, while an isolated calibration
extreme below its ceiling can pass. ADR 0105's sensitivity table records these
limits. A slow first operation may still fail the ceiling or A/A pair check.
The method is neither startup-free nor a warmed-process measurement.

The existing `consumer-profile-v1` proposal/freeze path binds the accepted
method and fully recomputes the envelope. Policy, population, budget or clock
mismatch, substituted old reviews and edited envelopes invalidate admission.
Hash-bound method-review, gate and runtime-closure reads use `read_reference`:
the byte-bounded snapshot supplies both the SHA256 check and the parsed value,
with stable file identity checked across the read. A later pathname replacement
cannot substitute different bytes for that verified snapshot.
Unchanged A/A must pass before profile acceptance or candidate collection;
the campaign control fence, bounded stages and no-automatic-retry rule remain.
This policy authorizes neither a full campaign nor a 15%-to-20% substitution.
Lifetime CPU admission, other timing/CPU envelopes, resident policy,
`group_span_ns` and matched `query_wait_ns` benefits, and component obligations
are unchanged.

[ADR 0104](adr/0104-fresh-trial-responsiveness-and-non-regression.md) accepts
`hard_ceiling_us=1000000` and `max_regression_us=100000` as prospective engineering
limits solely for Apple `trial-hit-roi`, `fresh-process-first-operation`, on the
approved 32 GiB host, `dev-normal`, `MADO_P050_OBSERVATIONS=1` and the reviewed
loader-on configuration. Other cases, targets, hosts, builds and observation
modes inherit neither amount. The ADR's primary UX sources motivate one second
for continuity of an authoring command and a separately chosen 100 ms deterioration
allowance; neither historical runtimes nor process timeouts determine them.

Inclusive bounds permit exactly 100 ms deterioration, which can be noticeable and
a large percentage at a low baseline. UX guidance does not derive that regression
allowance or its use for repeatability/controls. The operation observes no
displayed result and excludes input decoding/hash and controller construction;
these limits are not a user/product SLO, imperceptibility guarantee, visible-UI
measurement or release/observation-off equivalence.

The selected Apple context has an accepted v3 method, six fresh baseline
calibration invocations, twelve unchanged A/A invocations and one independently
accepted numerical profile. Its subsequent twelve-invocation baseline/candidate
comparison is **inconclusive**: baseline operation range is102878µs, exceeding
the fixed100000µs allowance. All twelve semantic/runtime/cleanup checks pass,
and no candidate non-regression violation is reported. The profile does not turn
this failed comparison into acceptance; no retry, warmup exclusion or budget
change is applied. Other cases and targets retain their separate admission gates.
The observed configuration remains dev-normal, observation-on and loader-on,
not an optimized shipping build or observation-off equivalence.

A separate one-shot diagnostic uses newly built private sources to split child
startup from parent transport handling. Original semantics and cleanup pass.
The capability notification spends539.245ms in the parent's frame read, while
the child's own main-entry-to-emission interval is23.245ms. These are separate
clock domains, not a measured pre-main residual. Loader, scheduling and pipe
causality remain unestablished; this diagnostic neither reproduces the frozen
binary's latency nor changes the profile, comparison or qualification status.

The original Apple six-value
calibration remains `NONPASS`: range `82256 us`, median `472229.5 us`, and
range/median `17.418649%` (about `17.42%`) against its original 15% rule. Keep
all six values, empty envelopes, source/build bindings and unrun A/A/candidate
stages unchanged. That baseline failure establishes no candidate regression
and is not regraded under v3.

#### Closed execution and resource reservations

A phase is one case, target, configuration and cohort stage, with inspection
after each complete two-attempt block. Pauses consume its original absolute
deadline; no disk resume, automatic stage chain or clock reset is permitted.

| Stage | Attempts | Work reservation | Independent total | Evidence reservation |
| --- | ---: | --- | ---: | ---: |
| Baseline calibration | 6 | `60+6*(90+5+10)+60=750s` | 900s | 292MiB |
| Unchanged A/A | 12 | `60+12*(90+5+10)+60=1380s` | 1680s | 580MiB |
| Comparison | 12 | Same1380s | 1680s | 580MiB |

Each attempt reserves90s primary,5s cleanup,10s retention and4MiB combined
stdout/stderr. Descendants share its absolute end. Admission reserves the full
next105s plus pending60s final retention; a pair needs210s plus that reserve.
These are scope-specific operational bounds, not latency ceilings or a universal
ten-minute rule. Trial's internal10s preflight,30s operation and14s shutdown stay
inside90s. Three bounds stay separate:

- the retained trial driver's embedded declaration (60s primary, 5s cleanup,
  1MiB output);
- the collector-enforced per-attempt envelope above;
- the stage reservation and independent total in the table.

Each attempt retains the declaration unchanged. Declared primary and output must
be `<=` the enforced values, and cleanup must be equal. The one real enforced
bound governs execution, so rebuilding the driver only to equalize caps is
unnecessary.

One replay/group/query invocation selects an engine and Node child; a trial
selects sequential capability and recognition children. The resulting1080
application-process creations per target (360 roots plus720 selected children)
are not a complete owned-tree cap.
Platform-created owned members still require identities and a finite reviewed
maximum. Unexpected members or counts refuse collection; SDK applications are
not required to adopt these campaign restrictions.

The prospective48MiB per-attempt evidence allocation includes at most24MiB
escaped outer text (`6*4MiB`),64KiB raw metadata,64KiB started receipt,256KiB
validation,256KiB duplicated summary details and two8MiB child sinks:
40.625MiB plus7.375MiB headroom. Reserve `48*n+4`MiB per phase and64MiB free-space
safety margin. New closure records/sinks must fit before admission or receive a
prospective bound revision. No second raw-copy summary, packaging copy or
unaccounted output is allowed. Sampled storage scans are not hard quotas.

Keep logical BGR, requested Rust bytes, native allocation domains, RSS/OS
high-water/private commit, same-owner growth and final-owner residual separate.
Normal consumer Rust counters remain unavailable; instrumented checkpoints and
independent controls apply only to a separately admitted configuration. Sampled
engine/supervisor RSS does not cover Node or trial children. Only accepted steady
window refreshes have clock brackets; lifetime sampled maxima have no bracket
for every contributing refresh and are not OS high-water.

[ADR 0102](adr/0102-actual-consumer-resident-point-policy.md) records the
historical v2 split between fixed resident checkpoint ceilings and calibrated
comparison allowances; v3 carries that policy unchanged. Its prospective Apple
`trial-hit-roi`, dev-normal/observation-on allocation is 256MiB at the recognition
child's post-owner-release point on the bound32GiB host, with a1MiB comparison
floor. The fixed allocation is an engineering choice, not an observed-RSS fit,
scaling formula, host-capacity guarantee or SDK consumer limit. Other hosts,
observation-off, sampled replay RSS and other cases need their own source-bound
resource admission.

A `consumer-method-review-v4` binds each selected resident metric's positive
integer `hard_ceiling_bytes` and `comparison_allowance_floor_bytes`, with floor
no larger than ceiling, to matching context/source-bound `resources` evidence
and a nonempty justification. Missing policies refuse admission; no old-method
fallback exists. Existing `resident_policies` handling is unchanged. Every
invocation, including calibration, requires available positive integer bytes
no larger than its fixed ceiling. For six baseline values, freeze
`s=max(B)-min(B)`, `a=max(s,comparison_allowance_floor_bytes)`, the fixed ceiling,
floor, minimum, maximum, spread and exact median. Reject
`s/median(B)>0.15`; zero spread and fractional medians are valid.

Resident control/comparison roles each require six observations with
`spread/median<=0.15`. Each role's first/second-position and first/last-three
median differences must be `<=a`. Unchanged A/A additionally requires absolute
role-median difference `<=a`; comparison requires
`median(candidate)<=median(baseline)+a`. The per-row resident gate is the fixed
ceiling, while these balanced checks carry comparison sensitivity.
`a<=max(0.15*median(B),1MiB)` for this allocation; at low medians the floor can
dominate the percentage. The floor is comparison slack, not page size,
measurement resolution or a confidence interval.

All timing/CPU ceilings and benefit obligations remain independent.
Separately admitted counted requested-Rust-byte endpoints retain
`max(calibration6)+max(spread,256KiB)`; this admits no missing normal counter.
Growth/residual use same-process positive differences, not fresh-child
variation or division by cycles. Exact cache capacity/ownership contracts are
not calibrated allowances. A resident point proves neither high-water usage,
all-native leak freedom nor zero final-owner residual.

Original resource envelopes, failed controls and campaign fences remain final.
A new context requires fresh runtime closure, method/resource review,
calibration, unchanged A/A and an accepted profile before candidate collection.
The new directory or policy does not bypass a fence or rebind old observations.

`tools/consumer-performance.py` implements one source-bound case per phase with
`prepare`, `smoke`, `calibrate`, `control`, `freeze`, `compare`,
`profile-proposal`, `report` and `inspect`. Calibration has six fresh invocations;
control and comparison each have six balanced pairs. Campaign registrations and
failure fences survive the phase owner. Failed control cannot launch a candidate,
and neither an interrupted phase nor a different output directory permits retry.

For Apple loader-on plans, `discover` launches one real invocation and retains
the root and two named child transports. `close-runtime` reparses explicitly
named raw/sink records, rechecks file/cache identity and freezes their closure
without launching a process or accepting a method. Replay also pins the actual
PATH-selected Node executable. Unknown images, foreign PIDs, incomplete sinks
and identity drift refuse closure. Launch rechecks the original deadline after
durable admission; a cleanup failure fences the campaign even if raw retention
fails.

CPU eligibility additionally requires a source-bound descendant-creation and
reap certificate, selected native-path evidence and actual terminal witnesses.
A separately reviewed accounting margin never changes `record.cpu`'s partial
status, reason or null resolution and is not a physical-accuracy claim. The
collector does not approve those documents itself. Its direct Apple trial
development smoke and loader-on trial/replay/deadline discoveries passed their
contracts. The deadline application retains `FAIL`/`Timeout` and exit 1.
These are not calibration, observer-cost acceptance or consumer qualification.

**Fresh v3 numerical collection remains blocked** pending current role/image
closure, complete owned counts, observer eligibility/cost, clocks/CPU coverage,
named resource checkpoints and headroom, justified trial responsiveness and
regression budgets, byte-accounted retention and cross-phase enforcement.
A hash-bound method review must admit every gate before numerical phases
start; earlier context-specific acceptance does not transfer. Windows numerical
admission remains unavailable. No v3 consumer profile, candidate benefit,
UI result, whole-Change completion or whole-release qualification is accepted.


### Historical format-1 collector


`tools/template-matching-efficiency.py` retains JSON format-1 interpretation,
separate from the legacy TOML format below. Use `--protocol-version 1` with a
compatible retained executable; current binaries cannot reproduce that protocol.
`policy --protocol-version 1` prints its fixed sampling and envelope formulas. `calibrate`
retains five fresh baseline processes per row; `freeze` records a reviewed,
hash-bound numerical profile; `compare` runs a separate alternating five-baseline,
five-candidate cohort per row. Performance rows use 20 warmups and 200 retained
samples; lifecycle-only rows retain their explicit short plan. Every process
retains its output, status, source/binary/dependency identities and raw samples.
Build metadata is an operator attestation, not a reproducible-build proof.

Each process must meet its own correctness, p95, aggregate-CPU and resource
ceilings. Improvement rows additionally require median p95 and CPU reductions
beyond their frozen variability/accounting allowance. CPU accounting units are
not a claim of effective clock accuracy. Peak resources include setup and
warmup; same-owner-state growth and final-owner residual have separate ceilings.
The 32 MiB live prepared-image allowance applies once to native RSS peak, never
to Rust heap, steady growth or final-owner residual. Evicted BGR payload awaiting
its last owner is reported separately from cache-held bytes. The runner preserves
supplied thread settings rather than tuning them.

Comparison reports retain known per-process violations alongside missing
observations, with original process indices; missing values cannot qualify a
benefit. Started attempts without a result remain `incomplete`, distinct from
`not-run` work. Recorded interruptions and unreadable attempt records are `failed`.

Hosted CI executes the current executable's normal and instrumented semantic plans
on both targets. Smoke, missing resource observations, a missing target, or a
failed envelope cannot establish performance acceptance. Existing historical
profiles and their source/binary identities are unchanged.
Revalidation uses the campaign's recorded collector revision. A collector change
does not regrade earlier campaigns or replace their source hashes.

The complete Apple comparison of implementation `560d868` is **failed**, despite
zero oracle failures. RSS/timing guards and repeated-preparation benefit remain
unresolved performance findings. The current scalar scan passes separate bounded
selection diagnostics and public smoke, not a replacement qualification cohort.
Windows native performance is **not run**. Neither target has performance
qualification for this optimization.

[ADR 0087](adr/0087-v0-5-0-release-performance-qualification.md) assigns remaining
statistical calibration, comparison and adjustment to the selected v0.5.0 release
candidate. Development integration still requires correctness, operation/source
authority, finite resource ownership, affected compatibility checks and CI.
Release preparation reviews any prospective method/profile before candidate
acceptance data, distinguishes actual consumer timing from component replay,
and requires reruns or reviewed applicability after relevant source changes.
This scheduling decision changes no frozen ceiling, failed verdict or historical
source identity; a completed development checklist is not performance acceptance.

### Semantic smoke and direct-format compatibility

Historical corpus `template-efficiency-integer-v2-direct-formats` has 21 workload
rows, including non-statistical `contract-roi-format-transition` with full,
padded and nonzero-ROI RGBA/BGRA inputs plus forced-BGRA public-mapping controls.
These cases and their physical-work oracles remain in the successor corpus;
historical manifests retain their original interpretation. Persistent hits and a
real paused production reader exercise
mapping/source/result release and active eviction. The active reader completes
before final backend release; these are separate checkpoints, not RSS-return
or all-native-allocation guarantees.

Use Python 3.11 or newer. Resolve its absolute executable before native setup,
which may prepend a different Python to `PATH`. Build in a fresh root, retaining
the command and Cargo JSON output:

```sh
PYTHON="$(python3 -c 'import sys; print(sys.executable)')"
CARGO_TARGET_DIR=target/template-efficiency-smoke-instrumented \
  "$PYTHON" tools/setup-native.py -- cargo bench --locked \
  --config 'profile.bench.build-override.strip="none"' \
  --package mado-pilot --features template-matching-benchmark \
  --bench template-matching-efficiency --no-run --message-format=json
```

Select `BINARY` from Cargo's `compiler-artifact.executable`, not a newest-file
guess. `BUILD_ARGV_JSON` is the executed build command as a JSON array; `TARGET`
is the actual supported host target and `EVIDENCE` is a new output directory.
Windows uses its configured MSVC/OpenCV environment and `python`.

```sh
"$PYTHON" tools/template-matching-efficiency.py policy --protocol-version 4 --build-mode instrumented
"$PYTHON" tools/setup-native.py -- "$PYTHON" tools/template-matching-efficiency.py attest \
  --protocol-version 4 --build-mode instrumented --target "$TARGET" \
  --binary "$BINARY" --build-argv-json "$BUILD_ARGV_JSON" \
  --output "$EVIDENCE/build-metadata.json"
"$PYTHON" tools/setup-native.py -- "$PYTHON" tools/template-matching-efficiency.py smoke \
  --protocol-version 4 --build-mode instrumented --target "$TARGET" --baseline "$BINARY" \
  --baseline-metadata "$EVIDENCE/build-metadata.json" \
  --output "$EVIDENCE/campaign"
```

For a measurement-only historical export, add both `--source-root "$EXPORT"`
and `--source-revision "$MEASUREMENT_COMMIT"` to `attest`. The full commit must
exist in the collector's repository. `exported-git-tree-v1` uses real commit,
tree and recursive Git-tree probes, then verifies the complete export before
and after attestation against Git blob identities, sizes and SHA256. It does
not synthesize checkout `HEAD` or a clean status. The original historical tree
and executable remain unchanged; the port has its own commit and build roots.

Export checks allow at most 20,000 entries, 64 MiB per file, 512 MiB total and
60 seconds per pass. Regular files must match; only the exact allowlisted
repository-context symlink may be omitted. POSIX checks executable bits;
Windows records Git modes as metadata and verifies regular-file bytes without
claiming POSIX-mode equality. File identity and hard-link counts use full
no-follow filesystem metadata, not Windows directory-entry cache fields.
Pathname and descriptor timestamps are checked against their own before/after
snapshots: Windows creation-time compatibility can give the two APIs different
`ctime` meanings. Identity, size, timestamp drift, Git blobs and SHA256 remain
validated. The lockfile and compiled helper-source manifest remain independently
bound.

The collector uses one warmup and three retained iterations per row, a 90-second
owned-child deadline, a 900-second campaign deadline, 8 MiB bounded child output
and five-second cleanup. No failed sample is retried or replaced. It retains
source/tree, dirty status, binary, command, corpus and raw process records.
`attest` observes loaded OpenCV image paths and hashes; it does not attest the
complete transitive native-library closure or prove reproducible builds.
Recorded executable and dependency paths use the producer target's path syntax,
independent of the host reviewing the packet. Windows path aliases are compared
case-insensitively; raw observed paths remain in the evidence.
Work reduction establishes neither elapsed benefit nor release-host statistical
qualification.

The 2026-10-21 local Apple Silicon smoke of source `e788b59` passed all 21 rows
with one warmup and three retained iterations. Full-frame RGBA/BGRA mapped
without copying; explicit forced-BGRA mapping and nonzero ROI controls retained
their declared copies. This is bounded semantic/work evidence, not a timing
comparison or Windows execution claim.

## Where benchmark files live

A phase commits one file per **run** under `docs/benchmarks/`, named
`<phase>-<workload-set>-<target>.toml`. A measured file holds exactly one
`[profile]` and one `[[measurement]]` block per workload, each with its own
budgets, so that a budget is never separated from the conditions under which it
was measured. A run that could not be performed uses one explicit
`[measurements]` gap record with `performed = false` and carries no budget.

One file per run rather than one per workload, because the profile describes the
run and not the workload: a set of eight workloads measured together on one host
shares one fixture hash, one target, one machine, one build, and one sample
count. Eight files would carry eight copies of that and eight chances for them to
disagree. What must never be shared across a file is the *target* — a budget is
valid only for the target in its profile — which is why the target is in the
name.

The format example is
[benchmarks/example-synthetic.toml](benchmarks/example-synthetic.toml). It is a
format demonstration only: it is marked non-normative, it records no measurement,
and its numbers are illustrative.

## Format version

Every file declares `format_version`. The current version is `1`. A change that
alters the meaning of an existing field increments the version and records the
migration in an architecture decision record; adding an optional field does not.

## Benchmark

A file opens with the `[benchmark]` block, which says what was run and what the
file is for. The harness prints this block and a committed profile carries the
same keys with two answers changed, so that a recorded profile is the harness's
output with budgets added rather than a document assembled beside it.

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | The identifier the file is filed under, matching its name. |
| `workload` | yes | One sentence naming what the set of workloads covers. |
| `phase` | yes | The phase that introduced them. |
| `status` | yes | `measured` for a recorded run, `harness-output` for the printer's own output, `format-example` for the format demonstration. |
| `normative` | yes | Whether anything gates on this file. Harness output and the format example do not. |
| `measurements_recorded` | yes | Whether the numbers are readings. False only for a file whose numbers are illustrative. |

`crates/support/testkit/tests/benchmark_block_drift.rs` compares the harness's
keys against every committed profile's, because a key that only one of them
carries is a file no reader can turn into the other.

## Profile

A profile records the conditions that make a measurement reproducible. A budget
without a profile is not evidence, because the same number can pass or fail
depending on the fixture, the host, and the queue policy.

| Field | Required | Meaning |
|---|---|---|
| `fixture` | yes | Tracked path of the recorded frame sequence, template set, or model input the workload replays. |
| `fixture_sha256` | yes | Hash of the fixture, so a silent fixture change invalidates the evidence. |
| `release_target` | yes | Target triple the measurement ran on. A measurement is valid only for its own target. |
| `hardware` | yes | Host machine identification, including CPU, memory, and any accelerator used. |
| `os_version` | yes | Operating-system version string of the host. |
| `build_profile` | yes | Cargo profile and any feature selection that affects code generation. |
| `warmup_iterations` | yes | Iterations discarded before sampling, so first-run costs are separated from steady state. |
| `sample_count` | yes | Number of retained samples. |
| `correctness_oracle` | yes | How each sample's output was checked against the expected result. A latency number without a correctness check is not acceptable evidence. |
| `queue_policy` | yes | Queue depth and drop policy in effect, because latency and stale-work counts are meaningless without it. |
| `notes` | no | Anything else a reader needs in order to reproduce the run. |

## Measures

A budget names one measure. The version-one vocabulary is:

| Measure | Unit | Notes |
|---|---|---|
| `capture_to_result_latency_p50` | milliseconds | End-to-end latency percentile from frame capture to committed result. |
| `capture_to_result_latency_p95` | milliseconds | As above, at the 95th percentile. |
| `peak_memory` | bytes | Peak resident memory during the run. |
| `peak_resident_bytes` | bytes | High-water resident set of the measured native benchmark process or a separately measured child process, reported by the native operating system. Optional because workloads without a native process observation do not emit it. |
| `steady_memory` | bytes | Resident memory in steady state. |
| `mapped_bytes_per_result` | bytes | Frame bytes mapped into CPU memory per produced result, full-frame or region of interest. |
| `stale_work_ratio` | ratio | Share of scheduled work that was dropped, coalesced, superseded, rejected, queue-expired, or discarded as stale. |
| `model_load_time` | milliseconds | Time to load and initialize an OCR model, including provider selection. |
| `startup_time` | milliseconds | Time from process start to a usable session. |
| `cold_load_p95` | milliseconds | Worst accepted first backend open across the fresh qualification processes; recorded as a target-specific startup regression gate. |
| `first_close_max` | milliseconds | Slowest first close after the complete workload set in a fresh process. |
| `reopen_close_max` | milliseconds | Slowest accepted-model reopen-and-close cycle in a fresh process. |
| `active_cancellation_max` | milliseconds | Longest interval from cancellation after observed native-run start to the cancelled call returning. |
| `retained_result_max` | milliseconds | Longest separately measured grouped operation whose immutable result is then checked after parent teardown. |
| `max_detector_tensor_bytes` | bytes | Largest detector input tensor admitted by the selected profile across the run. |
| `result_correctness` | count | Retained samples whose output disagreed with the correctness oracle. A hard gate, never a tuned ceiling. |
| `memory_growth` | bytes | Signed change in resident memory across the sampled run, so a decrease is negative. A hard gate: unbounded growth is a defect, not a slow result, and its predicate bounds growth rather than demanding an exact zero. |
| `latency_p50` | milliseconds | Median of the per-iteration samples for one workload. |
| `latency_p95` | milliseconds | The 95th percentile of the same samples. Distinct from `capture_to_result_latency_p95`, which is end-to-end from capture to committed result rather than one operation. |
| `latency_max` | milliseconds | Slowest retained per-iteration sample for one workload. Frozen scenario bounds use it so one outlier cannot hide behind passing percentiles. |
| `iteration_span_ms` | milliseconds | One clock reading across the whole sampled run, divided by the sample count. It covers everything an iteration does, including the correctness check, so it is an upper bound on the operation rather than a reading of it. Use it where a per-iteration percentile is not expressible; see below. |
| `peak_allocated_bytes` | bytes | High-water mark of live heap bytes during the sampled run, above what was live before the workload's fixture existed. |
| `steady_allocated_bytes` | bytes | Live heap bytes when the sampled run finished, above the same baseline, with the fixture still alive. |
| `allocated_growth_bytes` | bytes | Signed change in live heap bytes across the sampled run alone. A hard gate, on the same terms as `memory_growth`. |
| `copied_bytes_per_result` | bytes | Producer-surface bytes copied while producing one retained sample. The report keeps the largest sample, so unexpected duplicate copies remain visible. |
| `cpu_ns` | nanoseconds | Sum of retained sample process-CPU deltas, with accounting resolution recorded separately; diagnostic only under protocol 4. |
| `retained_loop_cpu_ns` | nanoseconds | Protocol 4: one current-process CPU bracket over all retained iterations, including per-iteration preparation, oracles and sample storage. Source units are not accuracy. |
| `score_copied_bytes` | bytes | Physically duplicated score-map storage observed by the opt-in selector instrumentation. |
| `score_offsets_scanned` | count | Score offsets inspected by production selection, including row repairs. |
| `mapping_copied_bytes` | bytes | Actual packed CPU mapping copies; distinct from logical mapped-view extents. |
| `bgr_converted_bytes` | bytes | Packed BGR payload produced by completed region conversions. |
| `bgr_retained_bytes` | bytes | Current cache-owned BGR payload across observed backend instances, excluding active evicted owners. |
| `allocations` | count | Successful Rust allocation/reallocation calls within the named observation interval. |
| `detached_textures_peak` | count | Maximum simultaneously live Adapter-owned detached textures during one workload. |
| `staging_textures_peak` | count | Maximum simultaneously live CPU-readable staging textures during one workload. |
| `gpu_resources_peak` | count | Maximum simultaneously live producer, detached, and staging textures during one workload. |

A phase that needs a measure outside this list adds it here in the same change,
with its unit and its meaning.

### Why some names carry their unit and others do not

The suffix is not decoration and it is not applied evenly, so the rule is worth
stating rather than inferring. A name carries its unit when the quantity would be
ambiguous without it — `iteration_span_ms` is a duration and `_ms` says which
one, `peak_allocated_bytes` counts bytes and `_bytes` separates it from a byte
*rate* — and omits it when the `Unit` column above is the only answer the measure
can have. `latency_p95` is milliseconds because every latency here is.

Nine vocabulary names differ from the key a profile records the value under,
which is the one place a reader can be caught out:

| Vocabulary name | Recorded as |
|---|---|
| `latency_p50` | `latency_p50_ms` |
| `latency_p95` | `latency_p95_ms` |
| `latency_max` | `latency_max_ms` |
| `memory_growth` | `allocated_growth_bytes`, when the measure is live heap rather than resident memory |
| `cold_load_p95` | `cold_load_p95_ms` |
| `first_close_max` | `first_close_max_ms` |
| `reopen_close_max` | `reopen_close_max_ms` |
| `active_cancellation_max` | `active_cancellation_max_ms` |
| `retained_result_max` | `retained_result_max_ms` |

A budget's `measure` may name either form; committed profiles use the recorded
key everywhere except `latency_p50`, `latency_p95`, and `latency_max`, where
they use the vocabulary name. Renaming to one convention would move every
committed profile, the harness that prints them, and the drift test that
compares the two, so the mapping is documented instead.

### Live heap bytes and resident memory are different measures

`peak_memory`, `steady_memory`, `memory_growth`, and
`peak_resident_bytes` are resident-memory measures.
`peak_allocated_bytes`, `steady_allocated_bytes`, and `allocated_growth_bytes`
are live heap bytes, counted by a global allocator the benchmark installs. They
are separate entries rather than a redefinition of the first group, because a
budget written against one of them does not mean the same thing against the
other and a reader must never have to guess which was measured.

Phase 1 uses the heap measures. Resident memory is read through a different
platform API on each release target, it moves with allocator and
operating-system behaviour that no change to this project can affect, and on a
workload of that size the noise is larger than the signal. Phase 2 adds
`peak_resident_bytes` only for fresh C and C++ child processes whose loaded
native footprint is invisible to the Rust harness allocator; their
`peak_allocated_bytes` remains explicitly harness-side. Live heap growth still
answers whether a repeated in-process operation gives back what it took.

### When a per-iteration latency cannot be measured

An operation faster than the host clock can express reports zero, and a
percentile over zeros is not a budget. On `x86_64-pc-windows-msvc`,
`map_full_frame` measures exactly zero: a mapping whose requested format already
matches the frame's is shared rather than copied, so the operation is a
reference-count increment.

A workload in that position is bounded three ways instead, and a latency ceiling
is not one of them:

- `mapped_bytes_per_result`, which is exact, target-independent, and bounds the
  behaviour that actually costs something;
- `iteration_span_ms`, which recovers a number the clock can express by reading
  it once across hundreds of iterations;
- the hard memory and correctness gates, which apply to every workload.

Recording that a measurement was zero is not the same as recording that a
workload is free. A profile says which of these applies and why.

## Budget kinds

A budget is one of three kinds. A file may contain any mix of them.

### `hard`

A pass or fail requirement that is not a number to be tuned. Correctness and
bounded resource use are hard gates: a run that violates one has produced an
incorrect result, not a slow one. A hard budget states a `requirement` in prose
and a `predicate` that the benchmark harness evaluates.

A hard budget failure fails the phase. It is never relaxed to accommodate a
measurement; the behavior is fixed instead.

Name a hard budget after what it gates — `result_correctness` or `memory_growth` —
rather than after a latency measure. A correctness gate attached to a latency
measure reads as a latency budget and invites a later phase to tune it.

### `absolute`

A numeric ceiling or floor for a measure, with a `unit`, a `limit`, and a
`direction` of `at_most` or `at_least`. Use an absolute budget when the acceptable
cost is a property of the product rather than of a particular machine.

### `relative`

A ratio against a named baseline profile, with `baseline_id` and `max_ratio`. Use a
relative budget when the absolute cost depends on the host but a regression is
still unacceptable — for example, when a change must not make a measure more than
ten percent worse than the recorded baseline on the same target.

A relative budget requires that the baseline it names exists in a tracked
benchmark file for the same release target and fixture hash.

### Which side evaluates a budget

The kind decides that, and the two answers are different because the two kinds
claim different things. A hard budget is a structural property that holds on any
host, so the benchmark harness evaluates its predicate in process on every run —
both the run that produces timings and the reduced run the ordinary test
sequence performs — and a violation fails that run. An absolute or relative
budget is a per-target regression ceiling measured on named hardware, so only a
run on that hardware can evaluate it: whoever performs the run compares it
against the committed profile for the matching release target.

Continuous integration therefore reports correctness and bounded memory on both
release targets and evaluates no timing ceiling. That is deliberate: a hosted
runner's timings vary by more than a regression ceiling is worth, so a ceiling
evaluated there would teach a reader to re-run rather than to investigate.

## Where a budget attaches

A `[[budget]]` at the top level of a file applies to every measurement in it.
A `[[measurement.budget]]` applies to the one measurement it sits under. Use the
first for the gates that are true of the whole run — correctness and bounded
memory — and the second for anything whose number depends on the workload.

A workload that omits a measure's budget is stating that the measure does not
bound it. That is a decision the file should explain, not a gap: a latency
ceiling left out because the operation is faster than the clock says something
different from one left out because nobody looked.

## Rules that apply to every budget

- A budget is set, raised, or lowered only with a measurement recorded in the same
  change, plus an architecture decision record explaining the number.
- A budget is valid only for the release target in its profile. A result from one
  target never satisfies a budget for the other.
- A higher capture rate is not an improvement when it increases stale work,
  increases memory, or produces incorrect results. Correctness and bounded memory
  outrank throughput.
- The correctness oracle is reported alongside every latency and throughput
  number. A benchmark that does not check its output is reported as a timing
  experiment, not as evidence.
- When a run is not performed, the file says so explicitly rather than omitting the
  measurement section.

## Phase 0 status

Phase 0 delivers this format and the synthetic example. It sets no numeric budget,
records no measurement, and makes no performance claim. Each later phase populates
the format for the workloads it introduces, under gate
[`G-013`](validation-gates.md#g-013).

## Phase 1 status

Phase 1 delivers the harness, the correctness oracles, and **the first numeric
budgets this project has set**. The Phase 1 workloads of
[`G-013`](validation-gates.md#g-013) are resolved by
[ADR 0008](adr/0008-phase-1-performance-budgets.md); the gate stays open for
every later phase.

There are two benchmarks, each committed for both release targets:

| Benchmark | Profiles | Covers |
|---|---|---|
| `deterministic-slice` | [aarch64](benchmarks/phase-1-deterministic-slice-aarch64-apple-darwin.toml), [x86_64](benchmarks/phase-1-deterministic-slice-x86_64-pc-windows-msvc.toml) | The eight-operation Rust workflow |
| `c-boundary` | [aarch64](benchmarks/phase-1-c-boundary-aarch64-apple-darwin.toml), [x86_64](benchmarks/phase-1-c-boundary-x86_64-pc-windows-msvc.toml) | What the C ABI costs, against the same work through the facade |

Every profile was measured on a named host, two hundred samples per workload
after twenty warm-up iterations, every sample checked against its oracle, zero
oracle failures anywhere. Each benchmark's two profiles share a fixture hash.

ADR 0011's replay-reservation change is measured in all four profiles above at
the same reviewed executable source state, `8d7973e`. Each benchmark ran twice
on each named native host with the second run recorded. Every budget passes,
fixture hashes match within each benchmark pair, and correctness and allocated
growth are zero everywhere.

On Windows, `replay_open` p95 is `0.000900 ms` under the unchanged `0.003 ms`
ceiling and peak live heap is `94,702 bytes`, down from the regressed `119,062
bytes`. On Apple Silicon, p95 is `0.000750 ms` under the unchanged `0.0025 ms`
ceiling and peak live heap is `95,118 bytes`, back at the pre-copy shape. The
paired C-boundary runs also remain inside every retained ceiling.

The refreshed profiles explain deterministic allocation changes since ADR 0008.
In particular, the archive-file workload now carries the immutable snapshot
copy accepted by ADR 0010, while small asset, vision, and C-boundary allocation
shifts remain far below the existing ceilings. Timing variation is recorded as
measured rather than presented as a deterministic effect.

A refreshed profile keeps the ceilings ADR 0008 set. Each rationale therefore
names the baseline value the ceiling was derived from rather than the current
measurement, so a refresh cannot quietly relax a budget by re-deriving it.

Across those four files, thirteen workloads are measured, all thirteen are
covered by the two file-level hard gates, eleven carry a per-measurement
ceiling, and two are deliberate unbudgeted controls. The two controls are
`engine_create_rust` and `match_warm_rust`: each exists so its C counterpart can
be compared against it in one process, and the Rust workflow's own ceilings are
in the `deterministic-slice` profile for the same target, so a second set here
would be the same claim measured twice and free to disagree with itself.

The scaffolding both share — the sampling loop, the allocation accounting, and
the report — is `mado_pilot_testkit::bench_harness`, so the format described
above has one printer rather than two that can drift.

### The Rust workflow

A bench target of the `mado-pilot` package, at
`crates/mado-pilot/benches/deterministic-slice.rs`, covering eight workloads:

| Workload | What it measures | Correctness oracle |
|---|---|---|
| `replay_open` | Discovering targets and opening a session | The session reports the source's own extent and pixel format |
| `map_full_frame` | Mapping a whole frame to CPU-readable bytes | The mapping covers the whole frame and reports its exact source identity |
| `map_region_of_interest` | Mapping one region of a held frame | The mapping covers the requested region and no more |
| `load_package_directory` | Loading and validating a directory package | The package declares its six tracked templates |
| `load_package_memory` | The same package, described in caller-owned memory | The committed package equals the one the directory commits |
| `load_package_archive` | The same package, as a ZIP archive | As above |
| `prepare_and_match_cold` | Compiling a template and searching with it | Both planted copies are found at their planted offsets, scoring `1.0` within `1e-5` |
| `match_warm` | Searching with an already-compiled template | As above |

The three loading workloads use the `G-014` tiny package rather than the slice
package, because it is the only package tracked in more than one form. Same
bytes, same six templates, three containers: that is what makes the three
numbers comparable, and `mado-pilot-assets` already asserts that all three
commit the same package, so a fixture that drifted fails a test before it
reaches a benchmark.

Every sample is checked against its oracle, including in the run that produces
timings, because a latency number whose output was never checked is a timing
experiment rather than evidence.

The harness has two modes so the oracles can run far more often than the
timings. `cargo test --locked --workspace --all-targets` executes it with three
samples per workload and fails on any oracle violation, which puts the whole
workflow's correctness into the ordinary verification sequence. A timing run is
explicit, and states its host because the program does not guess one:

```sh
cargo bench --locked --package mado-pilot --bench deterministic-slice -- \
    --hardware "Core i7-12700KF, 20 threads, 32 GiB" \
    --os-version "Windows 11 Pro 10.0.26200 build 26200"
```

The release target is the one condition the program does state for itself, and
it selects rather than detects it: `std::env::consts` can report the
architecture and the operating system but not the vendor or the ABI, and a
triple assembled from the parts that are available would be a guess printed
where a measurement condition belongs. Anything that is not one of the two
declared release targets says so.

Turning a run into tracked evidence means adding the budgets and committing it
under `docs/benchmarks/`, in the same change that sets them.

### The C boundary

A bench target of the `mado-pilot-capi` package, at
`crates/bindings/capi/benches/c-boundary.rs`. Each workload that has a Rust
equivalent is measured twice, in one process and one run, so the difference
between a pair is the boundary rather than the conditions.

| Workload | What it measures |
|---|---|
| `negotiate_table` | `madopilot_get_api`, the whole of what a C caller pays before it holds anything |
| `engine_create_c` / `_rust` | Building an engine over the deterministic scene |
| `match_warm_c` / `_rust` | One search with a prepared template, and reading every match back |

The answer is that the boundary costs a fixed amount per entry rather than a
proportion of the work: negotiation does not register on either host's clock,
engine creation costs a sub-microsecond constant more, and a warm match costs
0.1% more on `aarch64-apple-darwin` and 3.4% more on `x86_64-pc-windows-msvc`
across four table calls. What makes the boundary material is therefore the
number of crossings, not the size of the work behind them; see
[ADR 0008](adr/0008-phase-1-performance-budgets.md).

Dynamic loading is not covered. The benchmark links the library, so the
`LoadLibrary` or `dlopen` a real C host performs once at startup is outside
every measured window.

### The finding the budgets are shaped around

On `x86_64-pc-windows-msvc`, `map_full_frame` measures **zero**. The mapping is
shared rather than copied when the requested format already matches the frame's,
so the operation is a reference-count increment, and the Windows monotonic
clock's granularity is coarser than that. Its median is exactly `0.000000` and
its 95th percentile is exactly one 100-nanosecond tick.

That workload therefore has no latency budget on either target. It is bounded by
`mapped_bytes_per_result`, which is exact and target-independent, and by
`iteration_span_ms`, which recovers a number by reading the clock once across
two hundred iterations. `negotiate_table` is bounded the same way for the same
reason, and so is any later operation whose fast path is a pointer copy.

## Phase 2.2 diagnostic performance

The common diagnostic hot paths have one benchmark at
`crates/automation/runtime/benches/diagnostic-overhead.rs`. Its ten workloads
measure one-event input submission, retained-frame acquisition/mapping, and
explicit close/drain with diagnostics `Off`, `Normal`, and `Debug`, plus a
four-slot debug queue under input pressure. Every sample proves its frame,
mapping, receipt, or close result is unchanged, validates retained categories
and order, checks exact loss counts, and records mapped bytes where applicable.

The benchmark's smoke plan runs under
`cargo test --locked --workspace --all-targets`, so both release-target CI jobs
enforce zero oracle failures and bounded allocation growth. A full profile is
run with the target's named host metadata:

```sh
cargo bench --locked --package mado-pilot-runtime \
    --bench diagnostic-overhead -- \
    --hardware "<named host hardware>" \
    --os-version "<named host OS and build>"
```

[ADR 0024](adr/0024-input-diagnostic-performance-budgets.md) accepts the
[aarch64 profile](benchmarks/phase-2-input-diagnostic-overhead-aarch64-apple-darwin.toml).
[ADR 0026](adr/0026-windows-native-and-diagnostic-performance-budgets.md)
replaces the Windows timing gap with the measured
[x86_64 profile](benchmarks/phase-2-input-diagnostic-overhead-x86_64-pc-windows-msvc.toml).
Both profiles retain 200 samples after 20 warmups for every workload, report
zero oracle failures and allocation growth, cap live Rust heap at 32 KiB, and
keep the capture mapping exact at 3,072 bytes.

On the Apple M1 Pro, p95 `Normal` input diagnostics add `0.000042 ms` over
`Off`, `Debug` adds `0.000125 ms`, debug capture/mapping adds `0.000126 ms`,
and debug close/drain adds `0.000209 ms`. On the Windows Core i7-12700KF,
the corresponding post-review differences are `0.000200 ms`, `0.000200 ms`,
`0.000200 ms`, and `0.000300 ms`. Four submissions against capacity four
retain all four normal terminal records, report all eight discarded debug
records, and still return four complete receipts on both targets.

## Phase 2 native performance status

The records in this section remain bound to their named source revisions.
Under [ADR 0089](adr/0089-direct-c-abi-consumption.md), the current benchmark's
input set runs Rust and C only; its official-wrapper C++ executable option,
workloads and oracles are retired. Historical C++ rows are reproducible with
their original source, not relabeled as direct-C-ABI C++ or C measurements.
This cutover records no replacement profile or qualification.

Phase 2's affected [`G-013`](validation-gates.md#g-013) production and
target-specific native profiles are accepted. Windows final-source Phase 1
reruns pass on the exact exit candidate; Apple Silicon runs remain attributed
to `d8336be` and apply by reviewed complete diff. Both keep their unchanged
ceilings.
[ADR 0021](adr/0021-invalidate-phase-2-native-performance-evidence.md)
invalidated the three macOS profiles originally accepted by
[ADR 0020](adr/0020-phase-2-native-performance-budgets.md). ADR 0025 replaced
the macOS input profile, while ADR 0026 accepts the controlled Windows native
profiles:

| Workload set | Target | Profile | Current status |
|---|---|---|---|
| Capture | macOS | [aarch64](benchmarks/phase-2-native-capture-aarch64-apple-darwin.toml) | historical, non-normative after source and oracle repairs |
| Capture | Windows | [x86_64](benchmarks/phase-2-native-capture-x86_64-pc-windows-msvc.toml) | measured and normative under ADR 0026 |
| Transitions | macOS | [aarch64](benchmarks/phase-2-native-transitions-aarch64-apple-darwin.toml) | historical, non-normative because it names the superseded tree |
| Transitions | Windows | [x86_64](benchmarks/phase-2-native-transitions-x86_64-pc-windows-msvc.toml) | measured and normative under ADR 0026 |
| Input and public languages | macOS | [aarch64](benchmarks/phase-2-native-input-aarch64-apple-darwin.toml) | measured and normative under ADR 0025 |
| Input and public languages | Windows | [x86_64](benchmarks/phase-2-native-input-x86_64-pc-windows-msvc.toml) | measured and normative under ADR 0026, extended by ADR 0028 |

The two historical macOS files retain their old samples, environment metadata,
and former budget blocks with `normative = false`; they do not gate current
source. The macOS input requalification at final candidate `dec43d7` retained
300 correct samples with maximum allocation growth 64 bytes under the
4,096-byte hard gate. The harness provisions each C/C++ sample's fresh
approved fixture outside its timed span and retains controller-owned
mode-0500 executable/library pins per workload, so one sample cannot change
the next sample's identity, lifecycle, or visual precondition.

The post-review Windows run is bound to source commit
`6873d4b05a13fd15cb3ffd961892b1153f606d78`, implementation tree
`2483269ee071d14adfe14f829d318a4c59337f85`, on the named Core i7-12700KF /
RTX 4080 host. Its retained 600 capture, 80 transition, and 300
input/public-language samples all satisfy their exact oracles, report zero
allocation growth, and pass the unchanged ADR 0026 ceilings. Capture p95 ranges
from `0.002500 ms` for latest acquisition to `31.546700 ms` for
stimulus-to-frame. Transition p95 ranges from `2.530000 ms` for close to
`112.344000 ms` for first-frame open. Input p95 is `0.366000 ms` for a Rust
receipt, `116.048000 ms` for the Rust common flow, `15.717900 ms` for C process
loading, `15.343200 ms` for C++ process loading, and below `285 ms` for either
public-language common flow.

[ADR 0028](adr/0028-windows-window-message-performance-budgets.md) fixed the
production `WindowMessage` ceilings from the decision-setting `b72a95f`
profile. That run rejected the pre-measurement 64 KiB maximum-sequence
hypothesis by 1,275 bytes and retained a measured 256 KiB regression ceiling.

The post-review input/public-language profile was regenerated at source
`223925d52d24045ddadbc97c751d79d75a94ad7c`, tree
`ae009ae7f8b917ae13c2ebd02cdea92696d009b9`; its
[raw output](evidence/phase-2-performance/native-phase2-input-window-message-223925d.log)
retains 50 samples after five warmups. One-unit submission measured
`0.3134 ms` p50 and `0.5124 ms` p95; positioning plus a two-unit
primary-button event measured `0.7904 ms` p50 and `1.0026 ms` p95. The maximum
256-event sequence measured `73.7495 ms` p50, `78.8125 ms` p95, 66,647 bytes
of aggregate Rust heap, and zero post-warmup growth. Every workload satisfied
its oracle and the unchanged ADR 0026/0028 budgets.

The first Windows transition and language runs were rejected rather than
recorded. They proved benchmark apparatus defects: the resize fixture stopped
publishing before WGC pool recreation stabilized, one workload reused a
1,024-event fixture for 2,050 redacted summaries, child processes lacked the
known Cargo-profile DLL path, and the C++ oracle expected macOS evidence on
Windows. ADR 0026 records the probes and the bounded repairs. Production capture
and input semantics were not changed to make a benchmark pass.

The Phase 1 profiles historically passed all applicable comparisons at their
recorded source revisions. Windows reruns them on the exact exit candidate;
Apple Silicon runs remain attributed to `d8336be` and apply by reviewed complete
diff. Neither target moves its committed ceilings. The accepted Phase 2.2
controlled-stimulus lineage below
supplies the current macOS capture, transition, and owning-process-route
measurements without treating them as comparable to the invalidated
input-stimulus lineage. ADRs 0030, 0031, and 0032 separately accept the complete
macOS, Windows 1280×720, and corrected Windows dual-4K production-capture
lineages.

## Phase 2.2 macOS process-directed and controlled-stimulus lineage

The macOS process-directed route changes what the `native-phase2` bench can
truthfully measure, in two ways. Capture and transition stimulus moves from
focus-dependent product input to acknowledged fixture-private commands, so the
`capture` and `transitions` workload sets keep their CLI switches but start a
new macOS profile lineage: a controlled-stimulus sample is not comparable with
an input-stimulus sample, and the historical files above stay unchanged. And
the route itself gains its own workload sets — `process-directed`,
`process-directed-game-like`, and `process-diagnostics` — covering fixture
command acknowledgement (`fixture_command_acknowledgement`), acknowledged
stimulus to a strictly newer frame (`controlled_stimulus_to_frame`), static
retained-latest and strictly-newer-expiry behavior, discovery/open with
retained process authority (`discovery_open_retained_authority`), the
per-event authority/preflight/post path (`event_authority_preflight_post`),
release cleanup and session close, and diagnostics `Off`/`Normal`/`Debug` and
overflow around process-directed events.

The macOS-only `resize-allocation` workload set is the focused regression seam
for allocation retained across controlled geometry changes. It measures the
fixture resize command separately from `resize_recreation`, uses five warm-ups
and fifty retained samples, and applies the repository hard correctness and
4,096-byte growth gates without defining an independent latency ceiling. It is
non-normative diagnostic evidence and does not replace the complete
`transitions` or production-capture acceptance profiles.

The original five revision-bound profiles were measured on the approved Apple
Silicon host at corrected pre-optimization source commit
`a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426` (implementation tree
`f4a707501748303adcec577df5f18fcd18f13f45`). The controlled-capture,
controlled-transition, and process-diagnostic records remain bound to that
source. Benchmark bodies formerly attributed to `a471c2d` use a visual oracle
absent from its source tree and remain rejected, non-normative evidence.

The authority-timing-sensitive current profiles are measured at final source
`dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
`109f77df9ef9f40b515245ab60a6036822ee7d78`):

- `phase-2-2-process-directed-appkit-aarch64-apple-darwin`;
- `phase-2-2-process-directed-game-like-aarch64-apple-darwin`.

AppKit terminal p95 is `56.466375 ms` under the frozen `106.34 ms` ceiling;
controlled game-like p95 is `56.699333 ms` under `112.18 ms`. Both profiles
record zero correctness failures, one matching fixture event per terminal
sequence, unchanged foreground and physical cursor, and zero post-warm-up
allocation growth in every workload.

The one-read gate is composite rather than a field in either benchmark row.
Eight revision-bound controller, geometry-source, and native seam tests prove
terminal route ordering, source-transform reuse without a Rust live query, one
final ordinary authority call, and zero retained-window authority calls for
cleanup. The measured rows independently prove latency, fixture observation,
foreground/cursor stability, correctness, memory, and executable provenance
without adding a private inventory counter to the timed path.

The read count has a narrow scope. Before optimization, the same terminal
`RequireUnchanged` pointer profile made four fresh inventory reads — route
preflight, Rust live geometry, native preparation, and native final authority —
and measured `212.674625 ms` / `224.368667 ms` p95. The optimized terminal
one-event `RequireUnchanged` or `UseFrameSnapshot` path with default no-focus
behavior and no later fallback has a deterministic one-final-read shape. A
fallback-eligible route makes one early read plus the final read; terminal
`ReprojectCurrent` makes one live-geometry read plus the final read. These are
separate two-read paths. `RequireFocused`, cleanup, and multi-unit sequences are
excluded from the one-read call-count scope.

The committed files under `docs/benchmarks/` carry the exact measurements,
profile metadata, executable hash, and in-process budgets. Every retained
process-directed profile binds its result to one full Git commit id, tree id,
fixture-source SHA-256 digest, and fixture-executable SHA-256 digest. The commit
and tree in its opening provenance comment must exactly match the values in
`[profile].notes`; `benchmark_block_drift` verifies those equalities and digest
shapes for every committed process-directed profile. A newly generated native
report also carries `benchmark_executable_sha256`, which must be retained when
that report becomes a current profile. The
[macOS input verification guide](macos-input-verification.md#current-native-input-performance-evidence)
provides the runnable benchmark commands, and the public
[qualification record](evidence/phase-2-native/macos-owning-process-qualification.md#current-candidate-decision)
records their outcomes and provenance. Game-like samples carry the fixture's
explicit `mode=game-like renderer=opengl` fact, so they establish no result for
another renderer, game, application, input stack, or anti-cheat system.

These passing profiles are controlled-fixture regression evidence, not a
real-time latency promise or evidence of exact-window delivery, application
consumption, `RequireFocused`, `ReprojectCurrent`, or fallback performance.
Timing did not replace topology qualification: independent `single`, exact
two-display non-mirrored `same-scale`, and `mixed-scale` matrices each passed.
All fourteen controlled pair decisions are release-qualified.

## Phase 2 macOS production-capture acceptance

[ADR 0030](adr/0030-macos-production-capture-performance-budgets.md) accepts the
production-capture and production-transition profiles at measured source
`d182300cd8710891ded6cba17184c44d6d58a114`, tree
`c570343d334a5c77415e6a885ef8821c731b0ad5`, on the approved exactly-two-display
mixed-scale Apple Silicon host. The two profiles retain 1,150 samples across
eight workloads with zero correctness failures and zero allocation growth while
enforcing latency, live-heap, mapped-byte, correctness, and growth budgets.

Natural `publication_age`, strictly-newer acquisition, latest acquisition,
BGRA8 mapping, and retained-pressure recovery are recorded separately from
fresh-session startup, controlled resize recreation, and close drain. Mapped
rows carry exactly one 1280x904 BGRA8 frame (`4,628,480` bytes). The deliberate
retained-pressure row reports a stale ratio near `0.835` only after filling the
finite retained budget and proving blocked/resumed publication; ordinary
production publication and acquisition report zero stale work.

The resize investigation did not establish an unbounded general capture leak.
It identified the documented 64-revision macOS source-geometry history growing
in power-of-two allocation steps and briefly retaining capacity 128 because
retirement followed insertion. The implementation now keeps a small history
for fixed-geometry targets, reserves the complete bound only on the first
geometry change, and retires before insertion at the bound. The exact
production resize row changed from `9,856` bytes to zero post-warm-up growth.

ADR 0030's target-specific p50, p95, and maximum ceilings follow the established
three-times-measurement policy. They are regression gates for this fixture,
host, topology, and operation shape, not real-time or application/game
compatibility claims. Production capture uses a 32 MiB peak live Rust heap
ceiling; transitions use 16 MiB. Every row remains subject to zero correctness
failures and at most 4,096 bytes of allocation growth.

## Phase 2 Windows ownership prototype

The G-002 prototype resolves a correctness and ownership gate; it does not set
numeric product budgets. Its accepted result is recorded in
[ADR 0013](adr/0013-windows-capture-frame-detachment.md) and
[evidence/g-002/](evidence/g-002/).

The selected two-frame WGC pool with callback detachment and lease-aware private
textures delivered every 600-frame candidate sample. Across the updated
MSVC/SDK confirmation, the maximum post-warm-up arrival gap was 63.671 ms. That
figure belongs to the rejected `copy-blind-2` control at pool three; the selected
`copy-leased` rows stayed at or below 63.259 ms and the two-4K rows at or below
62.784 ms. The private default-texture peak was 33 under a retained-frame bound
of 40, and every final resource count was zero. Across all accepted workloads, sequence
regressions were zero and maximum sequence stall was 22.613 ms. The lifecycle
suite's maximum callback drain was 0.037 ms, complete close was 527.302 ms, and
complete reset through `StartCapture()` return was 650.355 ms. Those values
demonstrate margin against the
prototype's 500 ms progress and 2 second close/reset correctness gates; they
are measurements, not ceilings for production code.

The required full-frame GPU copy is a consequence of ownership. Each 1280×720
detached matrix row copied 2,654,208,000 bytes and mapped 3,391,488,000 bytes.
Each 600-frame 3840×2160 display row copied 23,887,872,000 bytes and mapped
30,523,392,000 bytes. A later implementation cannot remove that copy by
publishing a WGC surface or by reusing leased content; it may optimize scheduling
or representation only while the ADR's detachment and lifetime tests still pass.

## Phase 2 Windows 1280x720 production-capture acceptance

[ADR 0031](adr/0031-windows-1280-production-capture-performance-budgets.md)
accepts separate 1280×720 capture and transition profiles. Shared-marker capture
source `f50285a`, tree `4c2f23f`, retained 600 samples across four workloads
with zero correctness failures, zero allocation growth, and every unchanged
budget enforced. Repaired transition source `7c31752`, tree `4e99487`, reran all
five lifecycle workloads with zero correctness failures and every unchanged
latency, mapping, memory, growth, and cleanup gate enforced.

The capture profile records arrival, one frame-stamp-correlated callback copy,
latest acquisition, and BGRA8 mapping separately. The shared-marker run reports
one exact 3,686,400-byte callback copy, two detached textures, one staging
texture, five total producer/detached/staging textures, zero steady stale work,
a 7,417,467-byte live Rust heap peak, and a 66,506,752-byte resident peak.
Resource counts are nonzero upper bounds; valid lower counts pass.

ADR 0031 follows the three-times-measurement policy for target-specific p50,
p95, and maximum ceilings. Both profiles use a 32 MiB live Rust heap ceiling,
a 256 MiB native process resident ceiling, zero correctness failures, and at
most 4,096 bytes growth. They are regression gates for the named host, fixture,
topology, and operation shape, not game or real-time guarantees.

## Phase 2 Windows dual-4K production-capture acceptance

[ADR 0032](adr/0032-windows-dual-4k-production-capture-performance-budgets.md)
accepts the corrected mixed-DPI dual-4K profile at shared-predicate source
`f50285a`, tree `4c2f23f`. The stationary pair retains 600 strictly newer
samples per display while sharing each capture/mapping interaction. A distinct
no-warm-up workload retains 300 frame pairs while moving the controlled fixture
across the signed seam.

The final stationary rows report zero correctness failures, zero growth, exact
66,355,200-byte mappings, a 199,065,600-byte six-surface copy interval, seven
detached textures, one staging texture, twelve total resources, a `0.455782313`
stale ratio, 99,582,727 bytes live Rust heap, and 219,213,824 bytes resident.
The moving row reports zero correctness failures, 320 bytes growth, a
six-surface copy interval, six/one/eleven resources, a `0.474145486` stale
ratio, 99,576,732 bytes heap, and 288,911,360 bytes resident.

Two corrected-marker precursor runs establish the moving 125/175/225 ms
p50/p95/maximum ceilings through ADR 0032's three-times/readable-rounding
policy. Both requested marker positions and each frame's coherent post-baseline
stream/epoch/sequence callback record must match under one absolute deadline.
ADR 0032 retains 384 MiB heap, 1 GiB resident, copy, texture, and stale-work
ceilings for all three workloads. Windows production-capture `G-013` is
complete. Windows Phase 1 reruns pass on the exact exit candidate; Apple Silicon
runs remain attributed to `d8336be` and apply by reviewed complete diff.
The complete workload and correctness obligations are in
[windows-capture-contract-tests.md](windows-capture-contract-tests.md).

## Phase 3 accepted CPU OCR performance

The Phase 3 benchmark is
`crates/backend/onnx/benches/onnx-cpu.rs`. It exercises only the accepted
`g-004-rapidocr-ppocrv4-det-v6-rec-small-v1` model through controlled ONNX
Runtime 1.29.0 API 17, CPU provider, one admitted inference, one session pair,
two sessions, one thread per session axis, disabled CPU arena, and recognition
batch ceiling six.

Every process opens through the same default path a Rust/C/C++ caller uses.
Three warmups precede twenty retained samples for each workload. With twenty
samples, nearest-rank p95 is the nineteenth value rather than the maximum; the
independent maximum ceiling still catches a single slow operation:

| Workload | Oracle and accounting |
|---|---|
| `onnx_cpu_hud_full` | exact eight NFC strings/count/order; fixture-derived geometry; finite same-host-stable confidence; exact source/backend/model/effective-region correlation; 2,073,600 observed mapped bytes; one observed detector plus two recognizer tensor runs |
| `onnx_cpu_hud_region` | exact bounded text/count; the same geometry/confidence/source rules; 64,800 observed mapped bytes; one observed detector plus one recognizer tensor run |
| `onnx_cpu_blank` | exact empty result/source correlation; 16,384 observed mapped bytes; one observed detector and zero recognizer tensor runs |

No operation is discarded after execution. A failed warmup aborts immediately;
retained incorrect count must be zero, and every workload must remain within the
shared 4,096-byte live-heap growth gate. The native ignored contract test
separately proves cancellation after native-run admission, termination and
`Cancelled` within 250 ms, no late publication, recovery, close-race safety, and
idempotent repeated close; instrumentation from that proof is not mixed into
latency percentiles.

The first smoke attempt rejected a draft exact-geometry assumption for bounded
recognition. Independent crop preprocessing legitimately produced a different
quadrilateral while passing the pre-existing fixture thresholds, so the
performance oracle was corrected before precursor measurement. The failed
attempt remains recorded; no measured failure was excluded and no target result
selected a threshold.

[`phase-3-ocr-aarch64-apple-darwin.toml`](benchmarks/phase-3-ocr-aarch64-apple-darwin.toml)
is normative under ADR 0037. Five fresh precursor processes at source `b83f23f`,
tree `7dfe9c2`, retained 50 samples per workload. Every warmup/sample passed and
every workload ended with zero heap growth. Worst precursor observations were
476.576/299.674/135.640 ms p95 for full/bounded/empty, 86.988 ms cold open,
1.116 ms first close, 63.569 ms reopen-close, and 548,487,168 bytes RSS.

The earlier post-budget run at source `192f3d2`, tree `45abefc`, remains valid
for latency, correctness, allocation growth, and external RSS observations. An
independent review found that mapped/tensor/session values were synthesized and
the RSS ceiling was manually compared rather than executable; those resource
acceptance claims do not transfer.

Review-fixed source `e41fbbd`, tree `9fbc47e`, instruments the actual mapped
`CpuMapping` bytes, detector/recognizer session accesses, and opened session
topology, and enforces macOS `getrusage` peak RSS in-process. Five fresh
processes retained 100 samples per workload. Worst full/bounded/empty p95 was
472.623/301.455/184.057 ms and maximum 478.703/309.750/185.290 ms; cold open
87.377 ms, close 1.163 ms, reopen-close 64.929 ms, RSS 516,833,280 bytes, and
worst attributable Rust heap 14,149,992 bytes. Every oracle/resource gate passed
with zero growth.

[`phase-3-ocr-x86_64-pc-windows-msvc.toml`](benchmarks/phase-3-ocr-x86_64-pc-windows-msvc.toml)
is the matching normative Windows profile. Precursor source `6b5f3c1`, tree
`0a9fc22`, executable SHA-256
`f3c13157807c9617fb03039b9689cd53ccc138701d9c412180c03fc28800a316`
ran in five fresh processes on the approved Windows 11 Pro 25H2 build-family-26200
Core i7-12700KF host. All 100 retained samples per workload passed with zero
growth. Worst full/bounded/empty p95 was 810.143/581.789/320.843 ms and maximum
849.570/586.828/359.929 ms; cold open was 182.002 ms, first close 5.652 ms,
reopen-close 161.748 ms, and `GetProcessMemoryInfo` peak RSS 242,667,520 bytes.
The full-frame process-5 and empty-result process-4 slow rows remain in the
profile derivation rather than being excluded.

Separate final source `f2d3f29`, tree `b9c8fb5`, executable SHA-256
`f0292ce5fafc106ddff2ffc4ef1066543e780dfa0ca8958eb6784e891d187310`
ran five more fresh processes. Every target budget and hard oracle passed without
retry or exclusion. Worst full/bounded/empty p95 was
717.487/579.638/275.385 ms and maximum 724.006/581.872/290.567 ms; cold open was
177.195 ms, first close 6.081 ms, reopen-close 160.820 ms, and peak RSS
242,810,880 bytes.

The first Apple resource-instrumented process failed the former 140 ms cold
ceiling at 141.686 ms; an unchanged repeat reported 90.075 ms. ADR 0037 retains
the failure and sets 175 ms from both observations. A separate ten-sample run produced one
785.460 ms full-frame outlier, making nearest-rank p95 equal the maximum and
exceeding 750 ms even though the 900 ms maximum held. The final twenty-sample
policy makes p95 the nineteenth sample while the independent maximum remains
enforced; it changes no latency ceiling.

Accepted Apple ceilings are 600/750/900 ms p50/p95/maximum for full-frame,
375/450/600 ms bounded, and 175/210/300 ms empty. Cold open is capped at 175 ms,
first close 2 ms, reopen-close 100 ms, live Rust heap 20 MiB, and resident
high-water 768 MiB.

Accepted Windows ceilings are 900/1,000/1,200 ms p50/p95/maximum for full-frame,
725/750/850 ms bounded, and 350/425/500 ms empty. Cold open is capped at 250 ms,
first close 10 ms, reopen-close 225 ms, live Rust heap 20 MiB, and resident
high-water 320 MiB. These are derived only from the approved Windows desktop,
not Apple or hosted Windows Server.

Both profiles cap input/output tensor bytes at 256 MiB and enforce exact mapped
bytes plus detector/recognizer/session/result counts. Producer-surface copied
bytes are not applicable to immutable CPU replay inputs and have no ceiling.
These are regression ceilings for the named hosts and fixture, not
arbitrary-resolution, 4K, multi-region, real-time, renderer, application, or
game guarantees. Phase 3 default-OCR `G-013` is complete on both release
targets. The Phase 4 sections below record the deterministic template-query
qualification; the later provider section records the independent CoreML
rejection and explicit CUDA qualification.

## Phase 4 change-detection correctness and cost boundary

Accepted [ADR 0050](adr/0050-change-detection-default.md) resolves correctness
before watcher timing. The frozen `G-005` matrix has nine transitions: six
must-detect and three compatible unchanged rows. `exact-rgba-v1` produces zero
false skips, admits six analyses, skips three, and inspects 72 pixels. Every
predeclared changed-pixel threshold or fixed-grid candidate admits less work only
by skipping at least one mandatory change and is rejected.

The product detector is stateless and allocation-free per comparison. It reads
already mapped RGBA8 rows, ignores non-pixel row padding, stops at the first
difference, and fails safe to analysis across unsupported format, identity,
epoch, geometry, region, transform, descriptor, ordering, byte-extent, or
arithmetic incompatibility. The offline evaluator may allocate bounded report
data because it is test/evidence support and is never called from capture or
watcher runtime paths.

This gate establishes no wall-clock, heap/RSS, native-capture, backend, or
scheduler budget. Both hosted targets must reproduce the same target-neutral
report, but hosted timing would not qualify a watcher workload. Exact-source
replay/OpenCV watcher correctness and target-specific latency/resource ceilings
belong to `phase-4-template-watch-query-qualification`.

## Phase 4 deterministic template-query performance

[ADR 0051](adr/0051-template-watch-query-replay-budgets.md) and focused
[ADR 0052](adr/0052-windows-template-watch-roi-maximum-budget.md) accept the
independently reviewed Rust replay/OpenCV profile on Apple M1 Pro/macOS 26.5.2
and Core i7-12700KF/Windows 11 25H2. Each target ran five fresh sequential
optimized processes, three warmups, and 20 retained samples for each of eleven
workloads. Every retained sample first enforced exact source/match/state,
observed backend calls/completions/mapped request bytes, work/lifecycle facts,
and producer progress; incorrect output, silent loss, stale commit, starvation,
unbounded work, or lifecycle failure fails qualification regardless of time.

The accepted p95/maximum ceilings in milliseconds are:

| Workload | Apple p95/max | Windows p95/max |
|---|---:|---:|
| Current-frame match | 0.743 / 0.933 | 0.896 / 1.006 |
| Appearance stability | 2.135 / 2.270 | 2.509 / 2.606 |
| Disappearance/reset | 2.652 / 2.994 | 3.387 / 3.446 |
| ROI match | 0.313 / 0.370 | 0.331 / 0.933 |
| Duration stability | 2.710 / 2.850 | 2.468 / 2.911 |
| Exact coalescing | 0.266 / 0.309 | 0.550 / 0.715 |
| Saturation/latest-wins | 60.634 / 86.239 | 94.648 / 123.699 |
| Two-session fairness | 30.338 / 30.722 | 45.971 / 57.311 |
| Cancel in flight | 0.254 / 0.271 | 2.458 / 2.749 |
| Close and retained result | 0.437 / 0.467 | 0.992 / 1.317 |

`engine_session_startup` measures public replay/OpenCV engine construction plus
discover/open/close and retains p50/p95/maximum observations. Its numeric latency
ceiling is explicitly withheld because no pre-remediation precursor predeclared
one; exact identity, zero mapping/work, RSS, live heap, growth, and lifecycle
remain hard gates.

The corresponding p50 ceilings and complete workload budgets are
revision-bound in
`docs/benchmarks/phase-4-template-watch-query-remediated-aarch64-apple-darwin.toml`
and
`docs/benchmarks/phase-4-template-watch-query-remediated-x86_64-pc-windows-msvc.toml`.
Apple/Windows peak RSS ceilings are 69,206,016/19,922,944 bytes; both targets
cap live Rust heap at 245,760 bytes and post-warmup growth at 4,096 bytes.
Mapped bytes and backend/query/publication/work counts are exact rather than
upper bounds. Immutable replay and controlled CPU frames own no producer surface,
so copied producer-surface bytes are not applicable.

The first remediated Windows cohort retained one 0.7458 ms ROI maximum failure
against the former 0.348 ms bound. ADR 0052 changes only that maximum to the
evidence-derived 0.933 ms successor ceiling; all five fresh successor processes
pass. The predecessor profiles and every rejected cohort remain historical
evidence and contribute no successor samples.

These are regression ceilings for the named hosts, fixtures, OpenCV 4.14.0,
fixed scheduler descriptor, and exact workload semantics. They do not qualify
arbitrary templates/ROIs, native capture or application timing, OCR predicates,
callbacks, C/C++, automatic input, real-time behavior, packaging, or a release.

## v0.3.1 bounded-detector candidate performance

`crates/backend/onnx/benches/bounded-detector.rs` compares released native G-004
and the explicit bounded candidate on identical 4K, wide, extreme-wide,
960×540, odd, dense, boundary-region, and 4K blank inputs. Every iteration
checks text/count/order, fixture geometry, same-host confidence, complete source
and profile identity, final detector dimensions/bytes, one direct resize,
detector/recognizer runs, mapped bytes, one-pair/two-session topology,
cancellation, and at most 4,096 post-warm live Rust growth. The bounded
candidate additionally enforces at most 20 MiB attributable live Rust heap.
Native arbitrary-4K peak is recorded as comparator work under its released
256 MiB tensor ceiling; it is not judged by the new profile's peak limit.

ADR 0039 separates three phases. `smoke` is the target-independent hosted gate.
`precursor` runs three warmups and 20 retained samples in five fresh processes,
enforces all hard correctness/resource rules, and records timing/RSS without a
numeric verdict. The current executable refuses `--qualify`;
`enforce-budgets` is added to a fresh executable only after both approved
targets have precursor evidence and a final budget ADR. Result count does not
select a cost class: the 3840×2160 blank workload maps and detects 4K input
despite returning no regions.

The first Apple run at source `2782564`, executable SHA-256
`3a4bb04477b14092c9b3b34153275819684790d072aa1659e44183bafbd1f8b4`,
remains rejected. All correctness/resource rows passed, but the procedure
incorrectly applied the released 64×64 empty-result latency row to the 4K blank
workload, and one 2.697 ms close exceeded the earlier revision's 2 ms final
budget. No failed process was removed or relabeled.

Reviewed rectangular-candidate precursor source `cff5338`, executable SHA-256
`c28257dea60a86fe58d9fe9549670f6615004d3553c2f0bbe0a996a40ef9575d`,
ran five fresh processes per profile on the approved Apple M1 Pro. Every
bounded process passed. The native comparator preserved five false executable
verdicts because the precursor still evaluated its arbitrary-4K peak against
the bounded-only 20 MiB ceiling. Those false verdicts are retained rather than
relabeled. All 1,600 retained native/bounded samples otherwise passed with zero
oracle failures and zero live Rust growth. Worst per-process observations were:

| Workload | Bounded detector | Native detector | Bounded p95 | Native p95 |
|---|---:|---:|---:|---:|
| 4K HUD | 1312×736 / 11,587,584 bytes | 3840×2176 / 100,270,080 bytes | 490.454 ms | 2,347.264 ms |
| Wide menu | 1312×320 / 5,038,080 bytes | 2944×736 / 26,001,408 bytes | 339.375 ms | 787.027 ms |
| Extreme-wide status | 1312×160 / 2,519,040 bytes | 5888×736 / 52,002,816 bytes | 230.009 ms | 1,282.602 ms |
| 960×540 HUD | 1312×736 / 11,587,584 bytes | 1312×736 / 11,587,584 bytes | 479.206 ms | 480.257 ms |
| Dense tooltip | 1312×640 / 10,076,160 bytes | 1472×736 / 13,000,704 bytes | 599.615 ms | 673.754 ms |
| 4K blank | 1312×736 / 11,587,584 bytes | 3840×2176 / 100,270,080 bytes | 242.689 ms | 2,102.128 ms |

Bounded attributable live Rust peak was 12,695,400 bytes and bounded peak RSS
was 586,645,504 bytes. The native comparator's attributable peak reached
108,627,472 bytes under its released 256 MiB tensor ceiling, and its peak RSS
was 2,454,126,592 bytes. Reference-size work is intentionally unchanged. One
odd-size bounded process retained a slower p95 despite equal detector
dimensions, so no general speedup is inferred from the profile name.

The exact Apple process rows and retained native false verdicts are Change
evidence. The matching approved Windows `cff5338` matrix also passed all five
bounded correctness/resource verdicts, but ADR 0039's unchanged formula rejected
the rectangular candidate. Margin-derived p50/p95 ceilings exceeded fixed caps
for 4K HUD, 960×540 HUD, odd HUD, and dense tooltip; dense maximum also exceeded
its cap. Cold derived 275 ms above 250 ms. Observed RSS derived 384 MiB above
320 MiB, although that harness retained all eight source frames and the row is
not reused as one-operation memory evidence. No cap or expected result was
relaxed.

ADR 0040 replaces the unreleased `bounded-v1` tuple. Candidate v2 preserves
1312×736 reference/odd detector pixels, but after an oversized desired detector
first fits the 1312×736 rectangle, it applies a second 6 MiB aspect-preserving
tensor fit when needed. Fixed workload dimensions become 960×512 for 4K,
1024×480 for dense tooltip, and remain 1312×320, 1312×160, 1312×736, and
576×736 for the other declared shapes. The benchmark constructs and drops one
source fixture per workload so those frames are not simultaneously live. OS RSS
is process-lifetime high-water: schema-v3 workload fields are explicitly named
`process_peak_resident_bytes_after_workload`, and only final report-level RSS
sets the process budget.

Exact candidate-v2 source `ce658b3`, executable SHA-256
`dea9cdfbbb66ba75cb490fc3359efa6ca599786726157a520544cf39b38f81eb`,
ran five fresh bounded and five fresh native schema-v3 processes on the approved
Apple M1 Pro with alternating pair order. All ten raw verdicts and all 1,600
retained samples passed with zero oracle failure/growth and complete RSS rows.
Bounded worst p95 was 372.100 ms for 4K HUD, 476.275 ms for reference HUD,
474.544 ms for dense tooltip, and 122.430 ms for 4K blank; peak heap was
12,695,336 bytes and final process RSS was 464,519,168 bytes. Every
formula-derived Apple candidate fits the unchanged caps.

Exact Windows executable
`54a10f73970e24e126b4863853c0949610206bb7135579477883ec669ea0b5ed`
ran the same alternating five bounded/five native schema-v3 matrix on the
approved Core i7-12700KF. All ten raw verdicts and all 1,600 retained samples
passed. Bounded worst p95 was 530.211 ms for 4K HUD, 718.100 ms for reference
HUD, 591.838 ms for dense tooltip, and 244.941 ms for 4K blank; peak heap was
12,695,272 bytes and final process RSS was 228,741,120 bytes. Every
formula-derived Windows candidate fits the unchanged caps.

ADR 0041 accepts the exact target budgets in
`phase-3-1-bounded-ocr-aarch64-apple-darwin.toml` and
`phase-3-1-bounded-ocr-x86_64-pc-windows-msvc.toml`. Strict final source
`33cd36b` rejects missing prerequisites, unknown/duplicate modes, and ambiguous
profiles before work; independent fix review returned no findings.

Apple executable
`7e48921dfeaa7b0f3a4bb33b9e927eea9e50d75422c570adb6443fd4f32cf190`
and Windows executable
`aefdfa9cd6a023049b532f650a5493191994b22b3c07b582097ca1146a58d5e4`
each passed five fresh bounded `--enforce-budgets` processes without retry or
exclusion. Worst final process RSS was 464,666,624 bytes on Apple and
229,089,280 bytes on Windows; all workload latency/heap/growth rows passed.
The explicit profile is qualified for the named target/runtime/model/fixture
boundaries. It remains non-default and is not a real-time or arbitrary-workload
guarantee.

## v0.3.1 integrated grouped-zone qualification

The accepted bounded-v2 singular profiles above remain revision-bound regression
gates. Integrated exact source `180c1b1`, tree `479e410`, added the fixed
one-/three-/eight-zone, dense, and empty rows without changing those profiles.
The approved Apple M1 Pro executable
`6ce1df5bba8bc555fa961af366b0386333e6baeebd7c9483b1be9da39f16c792`
and Windows Core i7-12700KF executable
`b34b99eb7dcb3870edbd768055428be655e1e45ad125400d3b999bfb4da23398`
each ran one native comparator and five fresh alternating 3+20 integrated
processes without retry or exclusion.

Every row passed exact text/count/order/geometry/source/profile,
source-envelope/zones/memberships, detector/recognizer work, mapping, heap,
growth, cancellation, retained-result independence, startup, close, and cleanup
oracles. Both targets reported zero incorrect retained samples, zero call
failures, zero post-warm growth, and identical deterministic resource signatures.
Duplicate, one-pixel-different, adjacent, slight-overlap, and complete-overlap
layouts remain bounded safety cases with no quality or latency support claim.

[ADR 0044](adr/0044-integrated-zone-ocr-target-budgets.md) accepts the
predeclared 1.25-times/25-ms formula results:

| Workload | Apple p50 / p95 / maximum | Windows p50 / p95 / maximum |
|---|---:|---:|
| full-frame one zone | 600 / 600 / 625 ms | 900 / 900 / 900 ms |
| three sparse zones | 375 / 375 / 375 ms | 525 / 525 / 525 ms |
| eight distinct zones | 450 / 450 / 475 ms | 600 / 600 / 600 ms |
| dense unique candidates | 600 / 675 / 700 ms | 725 / 750 / 750 ms |
| empty 4K result | 175 / 175 / 175 ms | 300 / 325 / 325 ms |

ADR 0041 close, 20 MiB attributable live Rust peak, 4 KiB growth,
11,587,584-byte detector tensor, and target final-RSS ceilings remain unchanged.
Active native cancellation-to-return is capped at 25 ms on each target.
Retained one-zone result completion is capped at 625 ms on Apple and 900 ms on
Windows. Each fixed grouped row additionally requires one mapping/resize/detector
run, exact mapped and detector bytes, expected zero/one/two recognizer runs,
exact selected/ignored/unique/membership/result accounting, one cleanup, and no
retained parent resource.

The first post-budget Apple process at exact source `7835884` is rejected and
retained. Every workload/lifecycle/resource row passed, but its 149.832 ms first
backend open exceeded the inherited 125 ms startup ceiling. The same executable
then measured 84.408–91.766 ms in six explicitly non-qualification warm-cache
diagnostics. After an authorized macOS disk-buffer-cache purge, the next
diagnostic reproduced the failure at 130.844 ms. This proves the historical
process-cold ADR 0041 profile did not control cache state.

[ADR 0045](adr/0045-integrated-ocr-cache-cold-startup-budget.md) therefore sets
only the new Apple integrated startup ceiling to 200 ms: 1.25 times the retained
149.832 ms observation, rounded upward to 25 ms. Windows stays at 250 ms.
Historical ADR 0041 constants and profiles remain unchanged, and the benchmark
does not prime the runtime/model before timing.

The new
`phase-3-1-integrated-zone-ocr-aarch64-apple-darwin.toml` and
`phase-3-1-integrated-zone-ocr-x86_64-pc-windows-msvc.toml` profiles record this
separate lineage rather than editing either ADR 0041 profile. Corrected exact
source `1ad2031`, tree `c06a969`, produced Apple executable
`9d9941e3a14c7acdea71c8c28c4af215a77a7f143d4cc793a53ef2d1d4d3e1da`
and Windows executable
`cce21732cc7afb5b9c4903b95f732c42e725e7c92591845f6a39482be0ea5504`.
Each passed five fresh alternating integrated `--enforce-budgets` processes
without retry or exclusion.

Apple worst final cold/close/reopen/RSS/cancellation/retained-result observations
were 86.591 ms, 1.043 ms, 62.963 ms, 469,565,440 bytes, 6.516 ms, and
479.335 ms. Windows observed 193.091 ms, 5.341 ms, 152.954 ms, 231,067,648
bytes, 4.664 ms, and 711.576 ms. Every singular/grouped latency, exact resource,
ownership, cancellation, cleanup, heap, and growth gate passed. Hosted CI also
passed both release targets on that exact source; hosted timing and RSS remain
non-qualifying.

## v0.3.1 ONNX execution-provider qualification

ADR 0046 fixes provider selection and measurement independently from the
native/bounded preprocessing profile. The schema-v5
`bounded-detector` benchmark records requested/active provider, initialization
fallback, runtime profile, startup/lifecycle/RSS, and the unchanged eight
singular plus five grouped quality/resource rows. Provider registration is not
accepted as placement: qualification-only ORT profiles are finalized on session
drop and retained privately, while tracked evidence contains only redacted
unique CUDA/CoreML/CPU node counts.

CoreML registration and placement succeeded on the approved Apple M1 Pro:
detector assignment was 6 CoreML / 8 CPU unique nodes and recognizer assignment
was 27 CoreML / 44 CPU. The fixed smoke gate nevertheless failed every warmup
and retained `bounded_menu_wide` and `bounded_status_extreme_wide` row on
geometry, then exceeded the 250 ms active-cancellation return bound. The
identical CPU smoke passed all thirteen workloads and cancellation gates.
[ADR 0047](adr/0047-coreml-ocr-provider-qualification.md) therefore rejects
CoreML support and assigns no CoreML performance budget. No failed row is
discarded or converted into a latency result.

The approved Windows CUDA precursor used exact base `83ce7e3` plus patch
SHA-256 `90e9d2ab4fd3b151d56cc1a9ddc08ecfd76120f667e5e41b57e23e5fa24a6135`
and executable SHA-256
`29b5be55a43d6eb958ff3932dd320782296041847c4748e86d90efb7c31c4d8a`.
The controlled ORT 1.29/CUDA 13/cuDNN 9/NVRTC root produced empty provider
stderr. Placement was 328 CUDA / 0 CPU detector nodes and 181 CUDA / 2 CPU
recognizer nodes.

Five fresh CPU and five fresh CUDA precursor processes, each with three warmups
and 20 retained samples, passed in the fixed alternating order. Same-index
CPU-over-CUDA speedup ranges were 4.326–21.048× p50, 4.115–19.662× p95, and
4.053–19.279× maximum. Exact final enforcement retained two stopped sequences
rather than excluding their tails: one CUDA dense-group p95/maximum and one
interleaved fresh-CPU fallback eight/dense-group p95/maximum. Their
approximately 1.25× rounded corrections change only the new provider profiles;
all historical standalone CPU profile bytes and budgets remain unchanged.

The terminal patch
`40e5ec9f1720fbfa5ab0943a3738ccc4a719c68d96532201c6740cca41c6152b`
built executable
`25a0ce6e51bfad48110e118807491d82b3870ac608491f25e86f207244f1c1c7`.
All five required-CUDA and five preferred-CUDA/missing-root CPU-fallback
processes passed 249/249 checks in the fixed order with zero retry, exclusion,
priming, overlap, incorrect retained sample, growth failure, or NVRTC
diagnostic.

| Measure | Fallback CPU median | CUDA median | CUDA final worst | CUDA ceiling |
|---|---:|---:|---:|---:|
| Cold open | 179.724 ms | 323.479 ms | 339.682 ms | 425 ms |
| First close | 4.908 ms | 15.701 ms | 17.774 ms | 25 ms |
| Reopen-close | 149.894 ms | 132.928 ms | 134.340 ms | 175 ms |
| Active cancellation | 4.441 ms | 1.683 ms | 2.133 ms | 25 ms |
| Retained result | 712.658 ms | 128.416 ms | 147.014 ms | 175 ms |
| Process peak RSS | 230,543,360 B | 1,121,656,832 B | 1,140,326,400 B | 1,426,063,360 B |

Representative CUDA p50/p95/maximum ceilings are 150/150/175 ms for 4K HUD,
150/175/175 ms for dense tooltip, 175/200/200 ms for one full-frame group, and
150/525/1,100 ms for dense grouped work. The separate mixed-provider fallback
profile keeps the standalone CPU p50 limits and records the retained grouped
tail ceilings. The three exact schema-v5 profiles and testkit registries enforce
every workload and scalar ceiling.

Across precursor and terminal evidence, host-wide VRAM was 1,811–1,899 MiB
during fallback CPU processes and 2,432–2,700 MiB during CUDA processes. The
conservative cross-process maximum difference was 889 MiB under the configured
1 GiB provider-arena cap. These host totals are not process attribution.

[ADR 0048](adr/0048-cuda-ocr-provider-qualification.md) accepts explicit CUDA
and rejects automatic preference: median CUDA process RSS is about 4.9 times
CPU, above the predeclared 1.5× automatic-selection bound. The latency benefit
does not relax that memory rule. `AutoPreferAccelerator` therefore remains CPU
on both release targets.

### ADR 0049 successor CUDA remediation

ADR 0049 preserves the ADR 0048 rows above as revision-bound history and adds
three `phase-3-1-cuda-remediated-*` profiles from exact successor tree
`896e037d962610c4abd7a4a7b143d1ae9c90f549`. The internal CUDA recognizer output
limit is 128 MiB. Ordinary accepted widths retain batches of six; six
4096-pixel candidates form `[3, 3]`, with maximum estimated output 114,954,240
bytes and actual extracted output checked against the same 134,217,728-byte
limit.

Adaptive batching passed its five-baseline/five-candidate/five-CPU comparison:
maximum workload-median `B/A` was 1.004, geometric mean was 0.972, and minimum
workload-median CPU-over-candidate speedup was 4.288×. This accepts a
memory-safety bound, not an ordinary RSS reduction. `SameAsRequested` arena
extension reduced median RSS only 0.016% and worst RSS 0.106%; restricted cuDNN
workspace stopped on an absolute CPU-reference latency failure. Both were
removed. Memory-stage attribution opened loader investigation at median
211,193,856 bytes, but PE imports proved incomplete for known dynamic NVRTC
requirements, so no preload was removed.

The final five required-CUDA and five preferred-CUDA/missing-root fallback
processes passed all eight singular and five grouped workloads with three
warmups and 20 retained samples:

| Measure | Fallback CPU median | CUDA median | CUDA worst | Ceiling |
|---|---:|---:|---:|---:|
| Cold open | 181.636 ms | 335.084 ms | 347.779 ms | 425 ms |
| First close | 4.618 ms | 15.745 ms | 16.472 ms | 25 ms |
| Reopen-close | 146.005 ms | 133.675 ms | 134.299 ms | 175 ms |
| Active cancellation | 4.290 ms | 3.729 ms | 4.112 ms | 25 ms |
| Retained result | 692.142 ms | 135.861 ms | 137.496 ms | 175 ms |
| Process peak RSS | 229,404,672 B | 1,119,711,232 B | 1,120,923,648 B | 1,426,063,360 B |

Minimum workload-median p95 CPU-over-CUDA speedup was 3.995×. Host-wide
CUDA-minus-fallback VRAM maxima were 663–671 MiB; these remain temporal host
observations. CUDA median RSS was 4.881× fallback CPU, so automatic selection
remains CPU. One retained predecessor process disproved only the inherited
`zone_empty_4k` 25 ms maximum; its successor p50/p95/maximum ceilings are
25/25/50 ms under the existing 1.25-times/next-25-ms rule.

Placement remains 328 CUDA / 0 CPU detector nodes and 181 CUDA / 2 CPU
recognizer nodes. One pre-runtime canonical-path apparatus stop is retained; its
authorized canonical-spelling replacement changed no source, executable,
directory contents, workload arguments, or product environment and reproduced
the exact counts.

## OCR model rule comparison

[ADR 0084](adr/0084-ocr-model-choices-and-test-results.md) and the
[model maintainer procedure](ocr-model-catalog.md) define independent results for a
suite: unchanged G-004 v3 plus the frozen `compact-ui-v1` inputs, actual Rust
singular/grouped recognition, three fresh processes per profile, two warmups and
twenty retained samples per workload. Reference and candidate run serially with
one intra/inter-op CPU thread.

Each named rule reports passed, failed, not run or incomplete. The relative timing
rule requires both p50/p95 at most 80% of the reference for every workload in the
same target/provider/cohort, independently of quality. It supplies no aggregate
rating or model-selection gate. Empirical timing/RSS ceilings are distinct from
enforced execution bounds, source correlation and owned lifecycle contracts.

Both final comparisons at `cf2763a` completed under the frozen Apple/Windows
policies. The candidate retained text and negative-image failures despite passing
the numerical checks. The original nonadmission verdicts remain unchanged; the
later per-rule view names the original source and its evaluator revision rather
than claiming a fresh run. Missing approved hosts, frozen bounds, originals or
collectors cannot produce passing evidence. Hosted CI is not a performance
cohort. Historical artifacts, hashes, ceilings and adverse rows remain intact.
