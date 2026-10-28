# Template-matching component method

This lane measures capture-free Rust/OpenCV work. It opens no capture device,
injects no input and runs no OCR. Use it for one selected mechanism or regression
question, not as application-startup, visible-UI or arbitrary-consumer evidence.
The [performance guide](../../performance.md) separates descriptive measurement,
explicit qualification and mandatory safety.

## Build modes and corpus

The current `template-matching-efficiency` executable emits protocol 4 only.
The collector defaults to protocol 4; retained protocol-1/2/3 artifacts need
their recorded collector/policy for interpretation, not conversion through the
current executable.

| Mode | Cargo feature | Evidence |
| --- | --- | --- |
| Normal | `template-matching-timing` | System allocator; no matching/mapping heap/work accounting; production-like component timing |
| Instrumented | `template-matching-benchmark` | Allocation, physical work and ownership checks; not normal-build timing |

Build modes in separate fresh Cargo target directories. Instrumentation feature
unification, including direct dependency instrumentation, rejects a normal build
at compile time. Never run later Cargo commands in a hash-pinned baseline or
candidate directory; verification uses another `CARGO_TARGET_DIR`.

For Apple Silicon, resolve the absolute Python 3.11+ executable before native
setup can change `PATH`. Keep the exact build command and Cargo JSON output:

```sh
PYTHON="$(python3 -c 'import sys; print(sys.executable)')"
CARGO_TARGET_DIR=target/template-efficiency-dev-normal \
  "$PYTHON" tools/setup-native.py -- cargo bench --locked \
  --config 'profile.bench.build-override.strip="none"' \
  --package mado-pilot --features template-matching-timing \
  --bench template-matching-efficiency --no-run --message-format=json
```

Select `BINARY` from Cargo's `compiler-artifact.executable`, not a newest-file
guess. `BUILD_ARGV_JSON` below is that actual build command as a JSON array;
`TARGET` is the executing host target. Windows uses its configured x64
MSVC/OpenCV environment and native Python. For instrumented work substitute
`template-matching-benchmark`, `--build-mode instrumented` and a distinct target
root. The host build-dependency strip override works around
[Rust issue 157750](https://github.com/rust-lang/rust/issues/157750) on macOS 27;
record it without changing measured-code optimization or the deployment floor.

Both modes share fixtures and public-result oracles. The corpus has 41
statistical rows plus 13 normal or 16 instrumented contract rows: 54/57 total.
Normal's unobserved 26 heap/work/owner fields are `null`, not zero. The
format-transition and two watcher-counter oracle rows are instrumented-only;
normal cannot select them. `--list` prints the generated manifest and hashes.

The Session rows use explicit `0.9/8/DropOverlapping` requests on packed RGBA and
BGRA with full-frame and nonzero ROI inputs; these are not inferred defaults.

| Family | Unit and boundary |
| --- | --- |
| Retained hit/miss and `64×48` patch sensitivity | One `Session::find_template` through bounded scalar projection and outcome release; frame/backend persist |
| Eight-template groups | Eight sequential searches on full, same-ROI or distinct-ROI keys; compatible-owner creation/release is separately observed |
| Turnover | Exactly `M,H0,M,H1`; publication/acquisition through final release, with separate hit/miss strata whose overlapping times are not added |
| Matched query | Fresh provider/engine/session/query, shared row backend/templates; producer release through actual wait, terminal projection and producer join; completed-work receipts gate `M → H0 → H1` |
| Deadline query | Publish `M`, prove completed no-match, require the real query-lifetime deadline; caller-wait interruption is not that terminal result |
| Owner lifecycle | Complete close/retained-value/refused-search script, or A/A-clone/independent-B calls and B reuse after A release |

Query setup and close/retained-terminal/result/runtime release are outside the
primary wait interval. Lifecycle rows time the whole owner script. Logical
fixture identity includes input, options, order and timing/owner contracts;
role-specific apparatus hashes and physical counters remain separate.

The two physical contracts are `prepared-cache-row-winner-v1` and
`uncached-bgra-repeated-scan-v1`. They share logical/public-result oracles, not
physical work. Logical mapped-view, mapping-copy and BGR-conversion bytes remain
separate. Zero uncached-role cache payload does not imply zero temporary native
allocation. Independent prepared backends each retain their own two-image,
33,554,432-byte allowance. The instrumented independent-backend row requires
`49,766,400 → 24,883,200 → 0` BGR payload bytes; it does not widen a per-backend
bound or final-owner residual.

## Development observation

Attest the chosen prebuilt executable, then run one bounded row:

```sh
"$PYTHON" tools/setup-native.py -- "$PYTHON" tools/template-matching-efficiency.py attest \
  --protocol-version 4 --build-mode normal --target "$TARGET" \
  --binary "$BINARY" --build-argv-json "$BUILD_ARGV_JSON" \
  --output "$NEW_BUILD_METADATA"
"$PYTHON" tools/setup-native.py -- "$PYTHON" tools/template-matching-efficiency.py smoke \
  --protocol-version 4 --build-mode normal --target "$TARGET" \
  --baseline "$BINARY" --baseline-metadata "$NEW_BUILD_METADATA" \
  --workload single-small96x64 --output "$NEW_EVIDENCE_DIRECTORY"
```

The output paths must be new. Smoke needs neither a method review, profile,
calibration/control pass, exclusive-host assertion, sampling plan nor approved
runtime closure. It runs manifest discovery and one fresh process for the
selected row, with one warmup and three retained units. Omitting `--workload`
runs the mode's full smoke corpus; a focused row is sufficient for investigation.

Each smoke child has a 90-second deadline, 8 MiB combined stdout/stderr and
five-second cleanup; the collection deadline is 900 seconds. Source/tree,
checkout state, executable/dependencies, argv, corpus and raw process outcomes
remain attributable. Attestation observes loaded OpenCV endpoints, not complete
transitive runtime closure or reproducible-build equivalence.

New protocol-4 plans identify reports as
`report_format: component-measurement-report-v2`, independently of producer
`format_version`; `policy_id` and `metric_roles` distinguish graded outcomes
from CPU diagnostics. `execution_status` is `completed`, `incomplete` or `failed`;
`qualification_status` is `not-requested` for smoke, otherwise the declared
scope's `passed`/`incomplete`/`failed` result. Protocol-1/2/3 report-v1 artifacts
retain their original interpretation.

A fully bound row failure stays a failure even if diagnostic collection can
continue. Failed manifest discovery, identity/runtime-provenance drift or unsafe
cleanup stops later launches. Numerical qualification-control failure alone
cannot bar a separately labeled safe smoke. Immutable `control_evidence` keeps
that failed control ineligible for profile/compare admission; there is no
persistent numerical-control lock. Do not retry or replace its cohort.

The existing temporary-directory lock
`mado-template-matching-efficiency.lock` records PID, host and evidence directory.
Unknown launch/cleanup, binary/dependency identity or runtime-provenance failure
retains it across collector exit; a new output directory cannot bypass it.
Recovery requires explicit inspection of the recorded host/PID and started/
attempt receipts, confirmed owned-child cleanup and source identity, then
deliberate lock removal. A dead PID alone does not prove cleanup or authorize
automatic stale-lock removal. A safely closed prefix between attempts, or a
numerical/semantic-only failure with confirmed cleanup, releases the lock.

## CPU and timing boundaries

Protocol 4 declares `mado-component-sampling-v1` and
`mado-component-retained-loop-cpu-v1`. A row's `retained_loop_cpu` contains
`protocol`, `source`, `source_unit_ns`, `start_ns`, `end_ns`, `cpu_ns`,
`sample_count`, `reason` and `scope`.

The CPU bracket starts immediately after `samples-begin`, before the first
retained iteration, and ends after the last sample is stored, before
`samples-end`. Its scope is
`current-process-retained-loop-including-preparation-oracles-and-storage`:
all process threads, including preparation and public-result oracles, but not
setup, warmups, post-warmup memory observation, markers or serialization.
macOS uses `getrusage(RUSAGE_SELF)` user+system (`source_unit_ns: 1000`);
Windows uses current-process `GetProcessTimes` kernel+user (`source_unit_ns: 100`).
Representation units are not accuracy or effective resolution.

`sample_count` equals the actual retained count; available
`cpu_ns=end_ns-start_ns` is nonnegative and no smaller than the contained
summed-operation CPU. Missing or reversed endpoints retain null CPU and an
explicit reason, never zero. A structurally valid unavailable bracket can
complete either smoke or full producer execution with exit 0 when other required
checks pass. Native protocol stays 4; CPU availability does not qualify or
disqualify latency. Malformed brackets, hidden availability (increasing endpoints
with null CPU), wrong source/count or invalid arithmetic remain semantic failures
in every mode, as do oracle, owner and native-thread-setting failures. Changed
producer apparatus requires a fresh build and attestation; the smoke CLI is
unchanged. `metrics.cpu_ns`, `retained_loop_cpu_ns` and turnover-stratum operation
CPU remain diagnostics with their original boundaries; no stratum loop bracket
is inferred. Retain available values, null reasons and resolution rather than
fabricating complete accounting or zero-cost work.

Windows and macOS CI exercise the producer's actual exit predicate in both build
modes before building and attesting the smoke executable. Those tests distinguish
valid unavailable CPU, malformed accounting and semantic failure; their Cargo
target directories are separate from the attested binaries.

Normal grading uses nearest-rank process p95, `ceil(0.95*N)`; turnover also grades
each stratum p95 independently. Median and CPU are descriptive. Instrumented
timing/CPU are descriptive; resource and exact work/ownership checks remain
enforced. Neither CPU variation nor a missing valid CPU observation enters
latency envelopes, noise/control tests or benefit decisions. Instrumented
evidence cannot substitute for a normal benefit or set `performance_qualified`.

## Runtime image evidence

The benchmark observes `cv::currentParallelFramework` and `cv::getNumThreads`
before/after work through a benchmark-only exception-contained C++ boundary.
This is the library-reported parallel-region setting, not actual worker use.
It sets no thread limit. Missing, unsupported GCD, invalid, changed or unequal
role settings refuse success with reasons and available JSON. Build information
is not a substitute for the active backend.

`runtime-preflight` is representative discovery, not acceptance timing. On
macOS it uses child-only `DYLD_PRINT_LIBRARIES`, the actual root PID, raw records,
Mach-O UUIDs, file identities/hashes and observed shared-cache membership.
Unknown, truncated, foreign or unbound records refuse closure; path prefixes do
not establish cache membership. The observer's OS/cache identity and installed
loader documentation matter; a public dyld source tag need not match the host.
Unload chronology and manual mappings are outside this method. Windows component
observation uses the bounded method below; historical debugger discovery is not
a normal timing observer and supplies no certificate for this method.

Preflight retains separate identity, provenance, execution, cleanup and semantic
findings. A semantic failure or owned nonzero exit remains unsuccessful and
produces no qualifying bundle; the original runner receipt is preserved.
Read-only `validate_discovery` can establish raw image/process identity
independently of exit, but does not establish a successful closure. Its
`preceding` receipts retain the existing union checks: host/system/dyld/file
identities must match preceding successful attempts even if the latest child
exits nonzero. Host/process identity mismatch remains fenced.
`validate_closure` still requires exit 0. Releasing the collection lock requires
complete provenance, before/after attested identity and confirmed cleanup;
failed provenance, identity or unknown cleanup retains it.

Preflight and measurement use the same target-specific keys for attested image
coverage: Windows normalizes separators, the `\\?\` prefix and case; macOS
preserves exact paths. Original path strings and receipts remain unchanged.
File/hash identity, observed-image provenance and cleanup checks still apply.

Attempt records and the final preflight bundle use compact JSON without changing
values, raw stdout/stderr strings or canonical identity encoding. The64MiB
retained-attempt and bundle limits apply to stored bytes; oversized evidence
remains incomplete. Existing formatted receipts remain readable and unchanged.

For macOS preflight, use the same absolute Python on both sides of setup:

```sh
"$PYTHON" tools/setup-native.py -- "$PYTHON" tools/template-matching-efficiency.py runtime-preflight \
  --protocol-version 4 --build-mode normal --target aarch64-apple-darwin \
  --binary "$BINARY" --build-metadata "$NEW_BUILD_METADATA" \
  --output "$NEW_PREFLIGHT_DIRECTORY"
```

Explicit Apple qualification uses `macos-dyld-observer-on-warm-v1` in the method,
plan and profile. Opt-in `--timing-boundaries` emits these stderr transitions
outside individual sample clocks:

```text
mado-component-timing-v1[<pid>]: <workload-id> row-start
mado-component-timing-v1[<pid>]: <workload-id> samples-begin
mado-component-timing-v1[<pid>]: <workload-id> samples-end
```

`row-start` precedes setup; the sample window follows warmups/checkpoints and
includes retained preparation through storage, ending before after-sample and
lifecycle observations. A bounded single write emits each marker; a failed/short
write fails without retry. Default-off and manifest-only runs emit no row markers.
The collector checks argv, output row, owned PID and ordered raw stream, not a
cached zero count. Missing, duplicate, foreign, torn or reordered markers cannot
represent an empty window. Every `dyld[` record counts, including duplicate and
informational records. Statistical rows require zero in-window records; contract
rows retain counts but still require complete boundaries.

This proves byte-order positions, not no loader activity, zero observer cost or
observer-off equivalence. Prehash/cache warming, initialization, startup logging,
pipe scheduling and stable-file/OS assumptions need prospective review. Endpoint
hashes cannot prove absence of mutation-and-reversion during child execution.

### Windows bounded observer

On `x86_64-pc-windows-msvc`, protocol4 supports the explicit
`windows-ldr-observer-on-warm-v1` method. The benchmark flag
`--windows-loader-observer` requires `--timing-boundaries` and one workload
unless listing. Default-off and list runs never register the observer. Other
targets refuse the flag. The SDK and its capture/input adapters are unchanged.

The manifest publishes the actual callback RVA and three static ranges; these
addresses belong to each executable, not the common baseline/candidate workload
contract. Windows includes the observer as its twelfth apparatus source; Apple
retains eleven. Manifest row observations are `null`, not fixture semantics.
The collector inspects the exact executable's bounded PE function/unwind data
and callback instructions before preflight and checks the proof again against
the actual executable before numerical admission. Indirect transfers, calls,
unknown instructions, out-of-function branches and unbound RIP-relative anchors
refuse admission. This gate does not prove callback memory/loop semantics or
loader identity; those still require source and method review.

The process-lifetime callback records only into fixed, pre-touched storage.
Initial, begin and end cuts each contain two censuses and mapped-name/PE/open-file
bridges. Their cumulative bridge limit is 129, allowing at most 43 unchanged
modules across all three cuts, with 128 modules per census and 256 events.
Exhaustion, pending publication, ambiguous instance identity or failed native
cleanup refuses success. Failure serialization preserves the last captured
counter; it cannot heal a pending observation by reading again.

Discovery binds observed files for prospective use. Measurement opens every
closure file before launching the child, with read sharing only, and retains
the handles through child termination. FileIdInfo128, final NT path, SHA256 and
non-address PE fields are read from those same handles before and after.
Mapped names and PE fields correlate each observed instance to its held file;
only relocation-sensitive `ImageBase` is excluded from PE equality, with both
raw values retained. Unknown or unbridged instances fail statistical and
contract rows alike. Only statistical rows require zero callback events between
the source-ordered window counters. Stderr markers bind PID/row/boundaries;
their byte order is not used to order a separate loader-event stream.

Full runtime preflight exercises every actual mode workload. Windows does not
add a redundant manifest discovery; numerical manifest children remain owned,
attested metadata-only runs and cannot claim runtime observation. The child
environment is retained privately in full with case-insensitive key identity.
The collector keeps its environment allowlist and canonicalizes Windows names
before selection, so `SystemRoot`/`SYSTEMROOT` produce one unchanged value.
The observer still refuses duplicate aliases in externally supplied mappings.
No debugger, second supervisor, quiet-host probe or observer-cost subtraction is
introduced.

This is conditional observed-window and file/header correlation, not
backing-object, running-code-byte or KnownDLL membership proof. Registration
history is absent and earlier effects may persist. Delayed callbacks,
non-atomic census, lack of final callback drain, serialization-to-exit activity
and observer-on costs remain limitations. Pre-touch does not guarantee
residency. Fresh source/build/linked proof and an applicable external method
review remain prerequisites for any numerical cohort; a smoke pass is not
qualification. See [ADR0108](../../adr/0108-windows-component-loader-observation.md).

## Optional qualification

Use the following stages only for an explicitly scoped performance claim:

| Stage | Required evidence |
| --- | --- |
| `attest` / `runtime-preflight` | Exact source/build/mode/dependency and representative runtime identities; discovery does not approve eligibility |
| `calibrate` | Externally hash-bound source/host/cost/observer method review; six fresh baseline processes per selected workload |
| `control` | Complete calibration plus six independent unchanged A/A pairs |
| `freeze` | Separate numerical-profile review binding exact method, cohorts, recomputed envelopes and scope |
| `compare` | Six fresh baseline/candidate pairs against that immutable profile |

Normal/instrumented modes have separate samples, controls, profiles and claims.
Both roles and their native backend/thread setting are fixed before calibration.
`calibrate`, `control` and `compare` can select one complete workload via one
`--workload`; without it they select the exact full mode corpus. Arbitrary
subsets, duplicates and wrong-mode rows are refused. Paired order is
`AB, BA, AB, BA, AB, BA`. There is no automatic chaining, aggregation, adaptive
stopping, sample replacement, retry or persisted resume.

Protocol-4 numerical stages require `--sampling-plan PATH` and
`--sampling-plan-sha256 SHA`. The at-most-64-KiB UTF-8 JSON maps exactly selected
workloads to `{"warmups": W, "samples": N}`. Statistical bounds are W `1..=1000`,
N `1..=40000`; contract rows are exactly 1/3. The benchmark accepts custom paired
`--warmups`/`--samples` only with one statistical row, never `--list`, `--smoke`
or a contract row. Logical reference manifests retain 20/200 and 1/3; smoke is
1/3. Parser limits do not adopt counts or prove cost/output bounds. The exact
prospective plan, raw bytes and hash bind method, calibration, control, profile
and comparison. Changed counts require fresh calibration/control; do not normalize
CPU across counts or choose counts after seeing the qualification result.

The current policy is
`template-efficiency-v4-latency-work-resources-v1`. For each graded normal
wall-time metric independently, use six calibration values B:

- `m=median(B)`, `s=max(B)-min(B)`, `r=max(calibration clock resolution)`.
- Empirical allowance `e=max(s,2r)`; per-process ceiling `max(B)+e`.
- Control allowance `c=max(e,ceil(m*15/100))`; benefit allowance `b=max(e,c)`.
- Calibration and each later role require
  `spread/max(median(role),2r)<=0.15`, with frozen calibration r for later roles.
- Absolute A/A role-median bias, and each control/comparison role's
  first/second-position and chronological first/last-three median differences,
  must be `<=c`. Timing ceilings and noise remain independent.
- Improvement rows require `median(comparison-baseline)-median(candidate)>b`
  strictly for operation p95. Guard rows, including `single-small96x64`, carry
  no benefit obligation. Turnover stratum p95 uses its own allowance.
  No metric, count or overlapping interval is pooled. CPU has no frozen
  envelope, threshold or benefit obligation.

Keep exact half-nanosecond medians. For sorted `v0..v5`,
`ceil(m*15/100)=((v2+v3)*3+39)//40`; compare doubled medians rather than round
first. Store/recompute empirical, control and benefit allowances separately;
caller-edited envelopes cannot authorize a profile. Equality passes rejection
bounds but not strict benefit. The 15% allowance is engineering sensitivity,
not statistical coverage or a false-failure probability. Order/drift add no
independent detection when implied by noise at a role median no larger than
calibration; effects at or below b remain unclaimable.

Resource ceilings are `max(calibration6)+max(spread,fixed_slack)` plus only the
applicable RSS-peak live allowance. RSS slack is 1 MiB; instrumented Rust heap
slack is 256 KiB. RSS peak adds zero for selection, 32 MiB for other rows, and
64 MiB only for the two-independent-backend lifecycle. Growth and residual get
no live-cache/per-cycle allowance. Checkpoint RSS/peak/private/footprint are not
all-native allocation or necessarily the process's reporting-to-exit peak;
management-owner memory is excluded. Endpoint differences are not leak rates.

`rss_growth_bytes` is `max(0, rss_steady_after_bytes - rss_steady_before_bytes)`;
`rss_residual_bytes` is `max(0, rss_after_drop_bytes - rss_before_bytes)`, not
absolute post-drop RSS or unreleased owned memory. The `steady` labels identify
post-warmup endpoints, not proven stationarity. Both differences cover the
benchmark process, including its retained observations. Session rows accumulate
per-unit JSON between the endpoints; unit and owner reports remain allocated
after fixture teardown. Counted Rust heap also includes this harness storage.
RSS does not replace the instrumented ownership oracles.

Each frozen resource ceiling applies independently: lower absolute RSS does not
offset a growth-limit failure. Interpret such a failure as nonconformance to the
fixed apparatus/count budget, not as proof of product-memory growth or its cause.

`workload_coverage` records required/selected/omitted IDs and `complete`.
A complete selected cohort may pass its `status` while whole-target verdicts
remain `incomplete` and `performance_qualified=false`. Known failures remain
failed. A whole-target or all-target claim still needs every required case,
both selected normal latency goals, per-process/resource checks, both
build modes/targets and separate actual-consumer evidence. Physical-work changes
explain mechanisms but do not replace latency evidence. The Windows bounded
observer alone grants no numerical or general image-closure claim, and Apple
evidence cannot qualify Windows. The current
[release disposition](../../releases/v0.5.0.md) is not changed by a scoped pass.

The policy and report identities are prospective. Existing joint p95/CPU
profiles and failed cohorts retain their original collector and verdict;
they cannot admit a new-policy comparison. New calibration, A/A and profile
bindings are required for a new numerical claim, not for ordinary safe smoke.

## Finite execution and retention

Qualification retains the 300-second child, 8-MiB combined output, five-second
cleanup and 120-second per-pass image-binding bounds. The bound method review
fixes each stage's `execution_limits`: selected-workload collection clocks may
be lower, never above 21,600 seconds calibration or 43,200 control/comparison.
Other limits equal the exact schedule. One live absolute deadline includes
pauses; no later extension, reset or spare-time transfer is allowed.

A source-bound management owner must separately cover admission, inspection,
evaluation, serialization and retention. The collection clock is not a
whole-command deadline; raw output caps are not disk quotas. Bind serialized
content, fixed metadata, observer/owner receipts, interrupted-call output,
partial-file headroom and filesystem operating space before launch. Recheck free
space and inventory retained lengths/hashes; these checks detect shortfall but
reserve no storage and cannot prevent unrelated writers or `ENOSPC`. Never delete
or truncate evidence to fit an admitted bound.

The owner reuses the collector and existing process runner, preserving its
unreaped group-leader/Job ownership through termination, cleanup and reap.
Expiry during ownership must not throw through launch/setup/cleanup; late child
success does not repair an expired stage. Retain primary, cleanup, provenance
and retention outcomes separately. Whole-owner/OS loss and uninterruptible OS
operations remain residuals, not an external-kill cleanup guarantee.

`collection_steps` yields after each saved attempt, at most one child per
advance. Keep it in `contextlib.closing`; safe exhaustion/close releases its
exclusive lock, while an unresolved safety failure retains it as described
above. Early close preserves the started/completed prefix; `report` marks the
suffix incomplete. Checkpoints are not crash-durability acknowledgments. Plan/profile/
started-receipt verification hashes the same bounded bytes it parses; new plans
hash the bytes written. Do not reopen a pathname to substitute content after
verification. Successful cleanup does not repair missing provenance.

A measurement-only export has its own actual commit/tree, never the collector's
`HEAD`. Paired `attest --source-root/--source-revision` verifies regular bytes,
Git blobs/modes, sizes, hashes and stable no-follow identity before/after.
Bounds are 20,000 entries, 64 MiB/file, 512 MiB total and 60 seconds/pass;
Windows records Git modes without claiming POSIX executable-bit equality.
Ordinary checkout provenance still records/requires the applicable checkout
state. Original historical sources/executables are not overwritten.

## Historical outcomes

These are source-bound history, not current authority or instructions to resume:

- [Protocol-3 Apple campaign](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0097-apple-observer-on-component-campaign.md#verification):
  all 325 baseline normal processes cleaned up; the first stored failure was
  `single-small96x64/cpu_ns`. Offline review also found CPU-noise failures for
  `session-owner8-bgra-roi` and `query-match-rgba-roi`. No passing A/A, numerical
  profile or candidate comparison follows from that calibration.
- [First protocol-4 control](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0103-component-control-error-allocation.md#context):
  `component-owner-p4-v1` failed with
  `A/A retained_loop_cpu_ns drift exceeds frozen control tolerance` under its
  original policy. Later allowances do not regrade it.
- [Earlier format-1 comparison and development smokes](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/performance.md#historical-format-1-collector):
  implementation `560d868` failed its Apple comparison despite zero oracle
  failures; Windows performance was not run. Selected semantic/work smokes are
  not replacement numerical cohorts.

Recorder/no-spawn restrictions belong to their declared campaign, not SDK
consumers. Applications choose their targets, windows and children; the SDK
retains responsibility for its own contracts, workers and resources.
