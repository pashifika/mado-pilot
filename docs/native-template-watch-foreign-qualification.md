# Native qualification and overhead of pull-based template queries

ABI 1.6 implements the C/C++ pull-query surface in
[ADR 0067](adr/0067-pull-template-watch-c-abi.md). Replay consumers and the
paired boundary smoke harness exist. **Native C/C++ qualification and new
numeric overhead budgets remain unaccepted.** Neither the released Rust
watcher evidence nor a hosted green build promotes the new foreign boundary.

This protocol fixes the observable qualification rows and measurement scope.
The current `examples/c/template-watch.c` and `examples/cpp/template-watch.cpp`
are replay consumers, not native qualification runners. A native campaign
must first implement and review their controlled native flow and its owned
fixture orchestration. There is no executable native foreign command to run
at this revision.

## Separate authorities

| Evidence | Authority |
|---|---|
| Existing Rust replay/OpenCV suite and `template-watch-query` benchmark | Unchanged scheduler/query semantics |
| Existing Rust WGC/ScreenCaptureKit Lane B | Qualified Rust native boundary under ADR 0064 |
| C/Rust layout probe, frozen headers, independent C/C++ and CMake consumers | ABI compatibility, foreign ownership, replay integration |
| `template-watch-boundary` | Paired Rust/C replay observation costs and correctness; no accepted numeric ceilings |
| Native foreign rows below | New C/C++ native boundary; currently unexecuted |

The accepted Rust Lane B results remain revision-bound to their recorded
sources. Carry them only through a reviewed applicability statement. A change
to runtime, capture, platform, backend, fixture, or qualification semantics
requires the affected Rust gate again. Do not rewrite historical reports,
profiles, digests, or ADRs to describe this development candidate.

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
- Establish readiness by decoding the acknowledged token from a frame acquired
  by the **consumer's own session**. A sleep, successful call, unrelated capture,
  or observer-session token is not readiness evidence.
- Keep screenshots and raw frame dumps transient in private scratch storage;
  retained reports contain bounded identities, geometry, statuses, and counters,
  not images, arbitrary window titles, paths, or sensitive payloads.

Use `tools/setup-native.py -- COMMAND` for every OpenCV-dependent child; a prior
check-only setup does not configure the parent shell. Observe fixture/process
cleanup and resource return against a recorded baseline. Missing host access,
permission, topology, executable mode, or accepted budget is an explicit
unexecuted prerequisite, never a skipped pass.

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

Smoke uses one warmup and three retained samples. Profile uses twenty warmups
and two hundred retained samples. A sample contains 32 observations except the
first-projection rows, which contain one. Backend completion and fixture setup
are outside caller latency/allocation windows. Matched setup uses a bounded
public wait, not a sleep or an unstimulated poll.

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
- The current harness supplies correctness and structural allocation gates only.
  It accepts no latency/heap ceiling, makes no real-time claim, and reuses none
  of the historical Rust watcher numbers.

## Acceptance and rejection

`G-013` remains open for these new workloads. Before final-candidate enforcement:

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
