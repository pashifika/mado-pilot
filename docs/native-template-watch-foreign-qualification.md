# Native qualification and overhead of pull-based template queries

ABI 1.6 implements the C/C++ pull-query surface in
[ADR 0067](adr/0067-pull-template-watch-c-abi.md). Replay consumers and the
paired boundary harness exist. Target-specific eighteen-row numeric profiles
are independently accepted under ADR 0068; **final enforcement and native
C/C++ qualification remain incomplete**. Neither released Rust watcher evidence
nor a hosted green build promotes the new foreign boundary.

This protocol fixes the observable qualification rows and measurement scope.
The existing `examples/c/template-watch.c` and `examples/cpp/template-watch.cpp`
remain replay consumers. Separate `native-template-watch.c` and
`native-template-watch.cpp` consumers now use the released public boundary with
the capture-free owned-fixture controller described in
[ADR 0069](adr/0069-native-foreign-template-watch-apparatus.md). Executable
apparatus is not native support qualification.

## Separate authorities

| Evidence | Authority |
|---|---|
| Existing Rust replay/OpenCV suite and `template-watch-query` benchmark | Unchanged scheduler/query semantics |
| Existing Rust WGC/ScreenCaptureKit Lane B | Qualified Rust native boundary under ADR 0064 |
| C/Rust layout probe, frozen headers, independent C/C++ and CMake consumers | ABI compatibility, foreign ownership, replay integration |
| `template-watch-boundary` | Paired Rust/C replay costs and correctness; eighteen-row numeric ceilings independently accepted, final campaigns failed |
| Native foreign rows below | New C/C++ native boundary; candidate-bound observations exist, full qualification remains incomplete |

The accepted Rust Lane B results remain revision-bound to their recorded
sources. Carry them only through a reviewed applicability statement. A change
to runtime, capture, platform, backend, fixture, or qualification semantics
requires the affected Rust gate again. Do not rewrite historical reports,
profiles, digests, or ADRs to describe this development candidate.

On candidate `ec7d3d3b87b9828e03e1ae6215ff28a929f29af0`, the fixed five-process
numeric campaigns failed on both targets; passing central statistics did not
override maximum-latency failures. Windows C/C++ passed F1–F8 across their
warmup and three measurements, but F9 remained budget-unaccepted. Apple's
permissioned iTerm2 context captured frames, then both consumers reported F1
`observation_deadline` before any measurement cycle. Its original C++ report also
records an exit-contract infrastructure failure. These reports remain bound to
that candidate; corrected consumer exit classification does not relabel them.

Subsequent bounded diagnostics on `ae12a2a` confirmed that all 90 token cells
matched at the nominal coordinates. The blocking readiness predicate also
required zero pending and in-flight work despite completed no-match analyses
on a continuously active stream. A diagnostic-only C variant without that
extra idle condition passed warmup F1–F7, then failed F8
`nonmatched_contract_failed`. This neither changes the accepted oracle nor
qualifies the variant; the original results remain unchanged.

The successor removes only the extra idle requirement while retaining
completed/source/generation checks and bounded work. It also aligns macOS F8
with the established Rust authenticated-process-loss stimulus. A source-pinned
Apple smoke passed C F1–F8 in warmup and three measurements; C++ reached pending
readiness but its subsequent match query ended `DeadlineExceeded` during F1.
F9 remained `resource_budget_unaccepted`. This is partial smoke evidence,
not a new final qualification or numerical acceptance.

A later numeric-only probe preserved the C++ `0.99` query deadline and found
the correct marker at score `0.9824870228767395` in a separate exact-token
frame. C++ now uses the existing C/Rust/asset threshold `0.95`, with all
position, token, source and result checks retained. Its successor smoke passed
F1–F8 in warmup and three measurements. This and the earlier corrected C
cohort establish local Apple functional smoke coverage, not final qualification:
F9 is still `resource_budget_unaccepted`, and prior failed reports are frozen.

## Candidate and apparatus

Before a campaign, record full source commit/tree, branch, clean-worktree state,
canonical header digests, the built library and each consumer/fixture executable
path, SHA-256 and size, compiler invocations, target, OS build, hardware,
OpenCV version, Rust/C/C++ toolchains, SDK and deployment minimum, features,
and accepted-budget revision. Record the target C/Rust layout report for the
five new records and the complete 840-byte table.

Build pinned artifacts into a dedicated `CARGO_TARGET_DIR`. Any later Cargo
invocation that can rebuild the same package uses a different root, including
private-fixture, smoke, profile, and sanitizer purposes. Rehash every campaign
artifact at the end. A changed digest invalidates apparatus provenance; it does
not silently replace the original artifact or rescue a passing row.

Use the existing token-driven owned-fixture methods from
[the Rust native guide](native-template-watch.md) and
[ADR 0064](adr/0064-token-driven-native-template-watch-qualification-v2.md):

- **Windows:** approved interactive x64 desktop, repository-owned window with
  exact authenticated identity, existing visual-command acknowledgements, and
  the approved mixed-DPI topology for the geometry-transition row.
- **macOS:** approved permissioned Apple Silicon Retina host, existing
  controlled ad-hoc-signed fixture bundle and authenticated launch lifetime.
  Read Screen Recording separately from input permission through non-prompting
  public probes. Never request permission, inject input, activate another
  application, or infer permission from signing metadata.
- Derive scale-exact token and marker geometry from the observed target/frame
  transform. Supply a validated marker asset package to the public preparation
  API. Only the orchestrator changes fixture state.
- Establish readiness by decoding the acknowledged token and checking the
  physical marker's visible/absent state in the same frame acquired by the
  **consumer's own session**. Contradictory or ambiguous marker cells are not
  readiness. A sleep, successful call, unrelated capture, or observer-session
  token is not readiness evidence.
- Readiness may observe bounded pending/in-flight work after an accepted
  no-match completion; it does not require capture or analysis to become idle.
- F8 loss uses owned-window close on Windows and authenticated fixture-process
  finalization on macOS, under the existing request deadline. Window-only
  ScreenCaptureKit quiescence is not a synthetic target-loss event.
- Keep screenshots and raw frame dumps transient in private scratch storage;
  retained reports contain bounded identities, geometry, statuses, and counters,
  not images, arbitrary window titles, paths, or sensitive payloads.

The Windows ordinary fixture constructs the background, marker and token from
one command snapshot in a reusable off-screen scene, then publishes through one
checked client-area copy. Visual and geometry success acknowledgements require
the requested paint generation, target window and snapshot to have succeeded.
The scene is bounded to 16,384 pixels per dimension and 128 MiB; a clipped
marker/token scene, failed draw or failed publication is a failed control
outcome. A refused resource release is reported and blocks further allocation
by that scene owner. Resize replaces storage; target destruction releases it.
These apparatus bounds are not F9 resource budgets. One final copy reduces
application-created partial scenes; it does **not** guarantee compositor or WGC
atomicity, nor does it prove the cause of historical token failures.

C and C++ observation mapping slices remain within the original absolute
observation deadline. A slice expiry may continue observation; expiry while
mapping the retained result frame is terminal. C++ result correlation requires
the exact expected query ID and the admitted stream/epoch/geometry watermark
with no earlier sequence, in addition to result/frame/mapping agreement. These
checks do not relax exact result-token validation or authorize a new campaign.

Use `tools/setup-native.py -- COMMAND` for every OpenCV-dependent child; a prior
check-only setup does not configure the parent shell. Observe fixture/process
cleanup and resource return against a recorded baseline. Missing host access,
permission, topology, executable mode, or accepted budget is an explicit
unexecuted prerequisite, never a skipped pass.

### Executable apparatus

Use separate absolute target roots for the library/consumers, controller,
boundary precursors and validation. Complete all builds before pinning bytes.
For the library/consumer root:

```sh
export CARGO_TARGET_DIR="$PWD/target/native-foreign-consumers"
python3 tools/setup-native.py -- cargo build --locked --release --package mado-pilot-capi --lib --example c-abi-check
python3 tools/setup-native.py -- cargo run --locked --release --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

`c-abi-check` compiles both new consumers and runs only their non-prompting
`--check` admission mode, alongside the existing ABI/C++/CMake checks. That mode
never opens a capture session and reports no native row pass.

In the independent controller root:

```sh
export CARGO_TARGET_DIR="$PWD/target/native-foreign-controller"
python3 tools/setup-native.py -- cargo build --locked --release --package mado-pilot --features native-template-watch-qualification --bench native-template-watch
```

Record the actual controller and consumer executable paths. Run each consumer
sequentially against its own authenticated fixture lifetime:

```sh
python3 tools/setup-native.py -- <absolute-controller> \
  --foreign-consumer=<absolute-consumer> \
  --fixture-executable=<absolute-approved-fixture-executable> \
  --foreign-artifact-dir=<absolute-new-attempt-directory> \
  --foreign-library=<absolute-actually-loaded-library>
```

On Windows use the explicit OpenCV/libclang setup options from
`CONTRIBUTING.md`, and build the existing window-message fixture in a separate
root. Do not pass `--activate` or `--fail-stage`. On Apple retain the already
approved fixture bundle; do not replace, rebuild or re-sign it to change a
permission outcome. Both foreign commands use absolute `--key=value` arguments;
the attempt directory must not exist and its parent must already exist.

`LOADED` identifies the module containing the actual `madopilot_get_api`
address. Pin that module, not merely an import library or preferred search path.
For a Cargo-built Apple library, its install name may point into `release/deps`
rather than the top-level hard link; record and verify the actual module.

Each invocation writes immutable root `before.json` and `report.json` using
`madopilot.native-foreign-watch.cohort.v2`. The fixed cohort contains one full
warmup and three full measurements in the same controller process, with fresh
fixture/consumer owners and unchanged artifact pins. Each launched cycle writes
its own complete F1–F9 ledger and cleanup facts under `warmup/` or
`measurement-001/` through `measurement-003/`, using
`madopilot.native-foreign-watch.cycle.v2`. Unlaunched cycles are explicitly
unexecuted, not missing successes. Exit codes are `0` for complete acceptance,
`1` for a nonpassing cohort and `2` for apparatus failure. Native F9 resource
ceilings remain unaccepted and prevent complete acceptance.

The consumer process exits zero after a complete ledger and acknowledged `DONE`,
including semantic `FAIL` rows. Nonzero consumer exit indicates an incomplete
exchange or unreported initialization failure. The controller derives cohort
status from the ledger and its own finalization; consumer exit zero is not a
native PASS. C and C++ use the same contract.

Cold controller counts are initialization observations, not a leak-free claim.
After an eligible full warmup, every measured before/after and final observation
must satisfy the same fixed post-warmup baseline. No reset, allowance, additional
warmup or wait-until-stable is permitted. Warmup semantic or cleanup failure
prevents later launches; the sole F9 budget-unaccepted gate stays visibly
unexecuted while allowing otherwise clean planned measurements.

The transport bounds a complete wire line, including LF/CRLF, to 2048 bytes,
uses a finite 64-event reader queue and permits 64 facts per row / 512 records
per consumer. Consumer work between requests uses the 120-second row bound
and the same absolute 600-second cohort authority across all four cycles.
Filesystem/native calls are not preempted; startup overruns permit only bounded
cleanup, not later binding or consumer launch. The first request byte starts a fixed
five-second request/reply deadline; partial bytes never restart it. Individual
public operations retain their own five-second absolute deadlines.

## Common native public flow

Run each row through both the negotiated C table and the C++ wrapper. C++ may
not reach private Rust functions or bypass its query/result owners. Reuse the
existing Lane B operation and fixture deadlines; bounded polling must not
manufacture observations or extend authority after cancellation/deadline.

| Row | Required consumer observation |
|---|---|
| F1 Admission | ABI 1.6 extent and lifecycle guards, fixed scheduler descriptor, native capabilities, separate permission facts, exactly one owned target, successful maintained-session open or the observed typed refusal |
| F2 Readiness and pending | Consumer-session frame decodes the acknowledged absent-marker token; query remains Pending after a completed no-match analysis with zero confirmed stability |
| F3 Match correlation | Acknowledged marker appearance produces Matched; target, full stream/epoch/sequence/geometry stamp, effective region, match bounds, backend/options, transform, exact result frame, and mapped frame agree; caller request/template/package storage was released after start |
| F4 Resize | New acknowledged geometry/epoch replaces old authority; no old-geometry result commits; the resulting source frame and transform agree |
| F5 Movement/scaling | Controlled movement and scale/geometry transition preserve capture-pixel result interpretation; scale-exact marker assets and source identity refer to the new geometry, with no desktop-coordinate guess |
| F6 Independent authority | Caller-wait cancellation and deadline return call failure without cancelling the query; query-lifetime deadline and explicit cancel produce immutable terminals; a C++ intermediate clone release does not cancel shared work |
| F7 Retained ownership | Terminal result, indexed borrowed views, and independently retained exact frame remain valid after query/session/engine teardown; a fresh session makes acknowledged progress while the old frame/mapping is retained |
| F8 Lifecycle terminals | Explicit session close, final engine release, and owned target destruction produce SessionClosed, SchedulerClosed, and TargetLost respectively; non-Matched match/frame access is refused with initialized outputs |
| F9 Cleanup | Close/release is balanced and idempotent where documented; no owned process, handle, native buffer, or retained allocation remains beyond its reported baseline/bound |

F3 must decode the stimulus from the retained result's exact frame, not merely
from a later frame. The result must stay readable after all parents are gone.
F4/F5 cannot pass on a status-only observation. On macOS, permission-refusal
behavior is recorded only when that state is actually observed; do not revoke
user permission or fabricate a refusal. Unavailable Windows topology leaves
F5 unexecuted without weakening the other rows.

Raw public frames have no transform accessor. Each declared metadata setup
uses one immediate public probe result and its exact retained frame/transform,
then compares all ninety acknowledged token cells in the consumer session.
Bootstrap facts are separate from qualification match evidence. A post-change
probe starts only after raw frames already show new epoch/geometry (and resize
dimensions). Stream/query ids are nonzero; epoch, sequence and geometry
revision are zero-based and are never adjusted by the apparatus.

F1 includes one complete native warmup and three complete fresh lifecycles in
the same consumer process, fixture and loaded module. It records an all-owners-
released baseline and one post-release resource sample per measured lifecycle.
F9 records the final pre-exit sample. Windows reports private commit, working
set and handle count; Apple reports physical footprint, resident bytes and Mach
port-name count without retaining port values. These are not GPU-byte or exact
native live-object measurements. Until new target-specific native precursors and
independent ceiling acceptance exist, effective F9 is
`UNEXECUTED resource_budget_unaccepted`, even when consumer cleanup succeeds.

Deterministic tests retain authority for scheduler saturation/fairness,
coalescing, queue expiry, controlled late backend results, invalid prefixes and
outputs, panic containment, concurrency, and final-reference resource release.
A short native smoke run does not replace those tests or prove timing budgets.

## Independent foreign-overhead profile

The implemented paired harness uses real replay pixels, the released asset
loader, OpenCV matching, the Rust facade, and the negotiated C table. It adds no
native capture or private runtime hook.

```sh
python3 tools/setup-native.py -- cargo test --locked --package mado-pilot-capi --bench template-watch-boundary
python3 tools/setup-native.py -- cargo bench --locked --package mado-pilot-capi --bench template-watch-boundary -- --hardware "<hardware>" --os-version "<OS build>"
```

Use separate target roots for smoke and profile builds. Preserve the emitted
TOML and raw samples with their source/executable/fixture provenance before
selecting numeric ceilings.

| Paired workload stems (`_rust`, `_c`) | Measured boundary |
|---|---|
| `pending_poll` | Settled pending value snapshot |
| `first_closed_terminal_poll` | First SessionClosed projection after a public close committed outside the timed window |
| `retained_terminal_poll` | Repeated terminal observation after initial projection |
| `terminal_info_match_read` | Retained immutable info and indexed match reads |
| `caller_wait_cancelled` | Independently cancelled caller wait, with query still pending |
| `query_clone_release` | Shared query reference acquisition/release |
| `result_clone_release` | Shared terminal reference acquisition/release |
| `exact_frame_after_parents` | Exact retained frame mapping/read after all parents are gone |
| `create_cancel_release` | Complete replay engine/session/package/template/query setup, settled pending, cancellation, result/query release, and final caller teardown |

Smoke uses one warmup and three retained samples. Profile uses twenty warmups
and two hundred retained samples. A sample contains 32 observations except the
first-projection and full-lifecycle rows, which contain one. Backend completion
and fixture setup are outside the original observation windows; the full
lifecycle pair deliberately includes its complete startup-through-close window.
Matched setup uses a bounded public wait, not a sleep or an unstimulated poll.

The report retains per-sample caller latency, allocation/deallocation calls,
process-wide Rust live-byte observations, heap peak/growth, and correctness.
Pending poll, retained terminal poll, and info/match reads enforce zero caller
allocation calls. First projection and caller-error materialization are
separate workloads and are not declared allocation-free.

Measurement limits are part of the result:

- The caller counter observes Rust `GlobalAlloc` on the calling thread, including
  allocate/free activity hidden by a zero net-byte delta. It does not measure
  foreign `malloc`, an independently loaded library's allocator, or RSS.
- Process-wide Rust heap observations include fixture/setup and worker activity;
  they are not a caller-only retained-storage measurement.
- Exact-frame rows report the actually readable RGBA view length. That is not
  authoritative incremental mapped bytes or a native producer-pool measurement.
- Correctness and structural allocation gates remain unconditional. Numeric
  enforcement is opt-in and fails closed unless a complete independently
  accepted target profile matches. [ADR 0068](adr/0068-template-watch-boundary-budgets.md)
  independently accepts all eighteen rows on both targets, including each
  lifecycle sample's signed live-byte delta. Final-candidate enforcement remains
  pending. No historical Rust ceiling is inherited.

## Acceptance and rejection

`G-013` remains open. Precursor measurement and numeric review are complete;
the requirements below still distinguish profile acceptance from final and
native qualification:

1. Capture repeated precursor profiles on **both** approved release targets,
   preserving all raw samples and exact provenance. Separate stable boundary
   cost from fixture/worker effects and explain noise or unavailable metrics.
2. Independently review the paired deltas and choose explicit target/workload
   latency, allocation, and memory bounds in a focused budget ADR. Do not derive
   new ceilings from the final candidate being judged or widen them after a miss.
3. Add machine-readable profiles and their enforcement using the existing
   benchmark conventions. Validate metric scope and deterministic oracles;
   absent measurement is not zero.
4. Build final candidates in isolated roots, run accepted-budget enforcement on
   both targets, and execute F1–F9 through both foreign consumers. Hosted ABI
   green, native semantic evidence, and statistical acceptance stay distinct.

A row is `PASS` only when every required observation and cleanup succeeds.
Product mismatch is `FAIL`; apparatus failure is `INFRA`; unsupported public
capability is `UNSUPPORTED`; missing prerequisites are `UNEXECUTED` with a
specific reason. None counts as a pass. Retain failed runs and source identities;
no retry, recapture, token replacement, deadline extension, or oracle weakening
may relabel the original row.

Publish native C/C++ support only after independent review accepts the complete
candidate ledger, both target layouts, common-flow rows, and new overhead
budgets. Until then, leave the Rust support table and historical benchmark
status unchanged. OCR watching, callbacks, automatic input, packaging, and
arbitrary-application timing remain outside this change.
