# ADR 0069: Native foreign template-watch apparatus

- **Status:** Implemented apparatus; native qualification and numeric acceptance pending
- **Date:** 2026-09-07
- **Resolves gate:** None; native C/C++ qualification and foreign `G-013` acceptance remain pending
- **Related:** ADR 0064, ADR 0067

## Context

ABI 1.6 has complete pull-query owners but its examples at source `9db705be`
exercise replay. Native qualification must run independently compiled C and C++
consumers against the loaded public library, not infer their behavior from the
Rust fixture controller's capture session.

The raw C frame API reports dimensions and identity, but no transform accessor.
Only a matched template-query result exposes the retained exact-frame transform.
The native fixture's marker and token geometry must not be guessed from a
separate capture session or a desktop coordinate assumption.

## Decision

Keep ABI 1.6 and the production watcher unchanged. Add separate native C and C++
consumers, leaving replay examples intact. The C++ consumer uses the released
wrapper's query/result/frame/session owners. Ordinary `c-abi-check` and CMake
execute only their non-prompting `--check` admission mode; capture remains an
explicit approved-host operation.

Reuse the existing Rust native benchmark's private `NativeFixture` controller,
platform transports, `VisualToken` codec, PNG encoder and explicit finalization.
The controller never opens a capture session for a consumer or transfers an
engine-local target ordinal. The only capture and watcher observations used by
a row belong to that consumer's own public session.

### Exact-frame metadata bootstrap

A declared setup stage prepares a tiny 3-by-2 repository marker, starts a bounded
immediate public query with minimum score zero and one result, and retains its
actual matched frame and transform. This query supplies metadata only: its match
is not an F3 result or stability observation. Its failure remains a failure; no
retry or fabricated transform rescues it.

The consumer then compares all ninety token cells in its own exact frame with
the acknowledged absent token and prepares a scale-exact marker package through
the public asset loader. F2 pending and all subsequent qualified matches use the
real marker policy. An explicit resize/movement setup stage may obtain a new
metadata-only result; each probe has a distinct `bootstrap` source-stamp record.
Before a post-resize/movement probe, raw consumer frames must already show the
new epoch/geometry (and changed dimensions for resize). This prevents an immediate
metadata probe from committing a still-maintained old frame. New-engine setup
in F7/F8 uses its own declared probe, never another engine's geometry ordinal.

Alternatives rejected:

- Adding another ABI suffix only for this apparatus changes the consumer contract
  being qualified unnecessarily.
- Fixture-only dimensions or an observer-session transform do not establish the
  consumer frame's coordinate authority.
- Calling private Rust capture functions from C++ would bypass the surface under
  qualification.

### Identity and stimulus

The controller authenticates its exact retained fixture process and target.
Windows validates the live owned process and HWND before control; macOS retains
its authenticated application launch and existing signed bundle unchanged.
Consumer discovery selects exactly one Window with the full controlled title in
its own engine, then proves exact acknowledged token progress on its own session.

Existing tokens and titles are deterministic sequences. They are not secrets or
cryptographic authentication. This is a controlled, non-adversarial owned-fixture
qualification, not a claim about hostile title collisions or arbitrary windows.

Only the controller mutates the fixture. Geometry commands require a subsequent
fresh visual token and observed source-generation transition. Windows movement
requires different effective DPI; Apple requires movement and new geometry with
exact observed scale, even when both screens use the same Retina scale. Missing
topology is `UNEXECUTED`, never a passed or product-unsupported row.

### Bounded control and retained evidence

One synchronous request is outstanding over consumer stdout/stdin. Framing uses
ASCII verbs/numbers, UTF-8 path/title payloads without CR/LF/NUL, C-locale decimal
numbers and explicit CRLF normalization. Complete wire lines, including LF/CRLF,
are bounded to 2,048 bytes and reader queues to 64 events. The first byte starts
the five-second request/reply deadline; incomplete lines cannot restart it.
Product operations retain separate absolute five-second authorities; fixture
acknowledgements retain two seconds and lifecycle retains ten seconds. Token
observation slices are 25 ms; five-ms poll pacing is not readiness evidence.
EOF, overflow and process loss remain bounded; an unbounded reader join is not cleanup.

The control operations are `LOADED`, `VISUAL`, `GEOMETRY`, `ASSET`, `RESIZE`,
`MOVE`, `DESTROY`, `FACT`, `ROW` and `DONE`. Only a strict bounded projection of
numeric facts and fixed statuses enters the retained JSON ledger. Paths, titles,
raw stderr and pixels are not report fields. F1–F9 must appear exactly once in
order; required numeric facts cannot be replaced by a fixed `PASS` string.
There are at most 64 facts per row and 512 records per process. Outer containment
is 120 seconds per row and one absolute 600-second authority for the complete
controller cohort. Consumer work between requests uses those outer bounds,
not an undeclared five-second aggregate limit over
several independent public operations. The limits do not extend any individual
operation, request/reply, acknowledgement or cleanup deadline.

The consumer reports the actual module containing `madopilot_get_api` through
`LOADED`. The controller verifies its canonical path and bytes against the pinned
library; a loader-directory preference alone is insufficient. Before/after
identities cover runner, consumer, library and fixture. Each build purpose uses
a separate target root; later Cargo invocations never rebuild pinned artifacts.

Consumer F9 reports releases and a pre-exit process-resource observation. The
controller separately checks fixture finalization, exact process containment,
reader/output drain and its own resource baseline.
An apparatus failure overrides consumer `PASS`; `DONE` or exit zero cannot rescue
missing rows, cleanup debt or changed artifact bytes. Original failed attempts
remain immutable.

### Controller steady-state resource boundary

Candidate001's cold controller baseline increased by five Windows handles
after otherwise completed teardown. Separate process/thread probes also
showed first-use growth, while three same-controller diagnostic lifecycles
recorded 239→244, 244→244 and 244→244. The last diagnostic lifecycle
separately failed F5 token correlation. These observations do not identify
the retained objects or prove them harmless; their original INFRA/FAIL
ledgers remain immutable.

The successor contract measures steady-state controller resource restoration:
exactly one complete warmup followed by three complete measured lifecycles
in the same controller process, using unchanged pinned artifacts and a fresh
owned fixture/consumer per cycle. Cold before/after counts remain diagnostic;
this does not claim cold-start leak freedom.

Warmup enforces the same full F1–F9 semantic, explicit-owner cleanup,
containment, identity and deadline checks. It has its own retained ledger.
Any refusal, missing row, semantic failure or infrastructure failure prevents
later launches. The sole F9 `resource_budget_unaccepted` gate remains visibly
`UNEXECUTED`; it does not prevent eligibility when all nine consumer oracles
and explicit cleanup checks pass, and it never becomes `PASS`.

Establish the fixed baseline once after eligible warmup and complete child,
pipe and worker teardown. Windows requires each measured before/after count
and the cohort final count to equal that same baseline. Apple retains its
cumulative fixture-cleanup predicate against the fixed baseline: no active
or newly exhausted cleanup, and equal scheduled/completed deltas.
Keep the four artifact-file owners in the same retained state across these
boundaries. No per-cycle baseline reset, allowance, extra warmup, sleep or
wait-until-stable is permitted. A later failure stops further launches and
leaves their cycles explicitly unexecuted.

Root reports use `madopilot.native-foreign-watch.cohort.v2`; each launched
cycle retains a complete `madopilot.native-foreign-watch.cycle.v2` report.
The root lists warmup and all three planned measurements, relative report
paths, eligibility, the fixed/final baseline and aggregate outcome. Each
cycle identifies its role and whether resource equality is enforced.
An unlaunched cycle has no fabricated report or result.

The 600-second authority is shared, not restarted for each cycle. Reserve
the existing launch allowance before entry, propagate that same authority
through the private startup helpers, and recheck it after identity reads,
immediately before and after OS launch, and before startup success. The
handshake uses the earlier of its existing deadline and the cohort deadline;
macOS launch and handshake remain separate bounded stages. Binding, consumer
launch and control actions recheck the same authority. Filesystem and native
calls are not preempted; an overrun cannot authorize another normal action
or successful admission. Only bounded cleanup may continue.
Existing per-operation, row, framing and finalization bounds remain intact.

### Performance authority

Native readiness records time from accepted consumer session open through exact
absent-token observation, excluding fixture launch. Native operation bounds are
not capture-rate guarantees or a statistical latency budget. Foreign boundary
profiles and independently reviewed numeric ceilings remain separate; unavailable
RSS or incremental mapped-byte instrumentation is not reported as zero.

F1 includes exactly one complete native warmup followed by three complete native
lifecycles in the same consumer process, fixture and loaded module. Each cycle
opens its own public session, obtains acknowledged token progress, matches and
maps the exact result, and releases every public owner. The baseline follows
warmup with all owners released; each measured cycle has the same post-release
boundary while the protocol pipes remain open. Four separate bootstrap source
records distinguish this prelude from qualifying F2/F3 facts.

The consumer measures Windows private commit, current working set and process
handle count, or Apple physical footprint, current resident bytes and Mach port
name count. Returned Mach arrays are released immediately; port names are not
retained. These are process-resource bounds, not complete GPU-byte or live-object
instrumentation. Missing measurements cannot become zero.

F9 records a final sample before consumer exit, after all public owners are gone.
New native precursor processes and independent target-specific ceiling acceptance
are required before these resource observations can qualify F9. Until then its
effective outcome is `UNEXECUTED resource_budget_unaccepted`, even when consumer
release oracles pass. This prevents process-exit reclamation from masquerading
as bounded in-process lifetime behavior.

## Verification and support

Implementation requires deterministic malformed-protocol/ledger/deadline tests,
ordinary two-target public checks, independent apparatus review, and actual C and
C++ F1–F9 results on each approved host. Screen Recording refusal is retained
without requesting permission and does not qualify capture. Existing authorized
applications are neither replaced nor re-signed. No System input, forced focus,
privilege escalation, release, packaging or support promotion is authorized by
this apparatus design alone.
