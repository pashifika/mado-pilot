# ADR 0067: Pull-based template-watch C ABI

- **Status:** Accepted design; implemented ABI 1.6, native foreign qualification and target budgets pending
- **Date:** 2026-09-07
- **Resolves gate:** None; the new foreign-boundary `G-013` workload remains open
- **Supersedes:** ADR 0065 only for historical release-test fixture implementation; its frozen validator and evidence remain unchanged

## Context

C and C++ need the existing maintained-session template watcher without changing
its scheduler, query authority, or Rust API. The implementation baseline is
`9ad16ae1cfea97421734f63ffd9754a0f7027052`, tree
`f354b2856b0cc02189eda2df4d0019a411d124f5`, identical to the inspected `483a612`
source. ABI 1.5 ends at 736 bytes; its exact header was frozen before integration.
This ADR does not qualify native C/C++ support or accept foreign-overhead budgets.

Direct source establishes the ownership decision:

- [`TemplateQuery`](../../crates/automation/runtime/src/watch.rs) is one owning
  Rust value whose `Drop` cancels. Poll returns value progress or
  `Arc<TemplateTerminalOutcome>`; wait checks an independent caller operation.
  The public terminal value has no final work counters.
- `TemplateWatchResult` retains `Frame` and `Arc<MatchResult>` and exposes the
  target, template, completed stability, backend, options, region, stamp, and
  transform without copying a match array.
- The synchronous C `ResultHandle` instead owns
  [`FindOutcome`](../../crates/automation/runtime/src/find.rs), whose constructor
  is `pub(crate)` and whose `MatchResult` is owned by value.
- [`FrameHandle`](../../crates/bindings/capi/src/capture.rs) needs the existing
  [`Session::mapping_observer`](../../crates/automation/runtime/src/session.rs)
  projection. That observer carries identities and weak diagnostics, not a
  strong parent reference.

Two planning assumptions require source-backed correction. ABI 1.5 has **no
`engine_close` entry and no transform accessor**: `frame_describe` reports pixel
geometry only. Final engine release invokes
[`Engine::drop`](../../crates/automation/runtime/src/engine.rs), which closes the
watcher scheduler. Also, public headers live under
`crates/bindings/capi/include/madopilot`; [CMake](../../crates/bindings/capi/CMakeLists.txt)
exports its package-relative `include` directory. The recorded filesystem
inspection found no repository-root `include` alias. This change creates none.

## Decision

Append exactly thirteen entries to ABI 1.5 and expose domain-specific query and
immutable terminal-result owners through the facade only. Share one Rust query
and one lazily materialized terminal projection; do not convert watcher results
into synchronous results or add a public Rust constructor.

The canonical declarations are in
[`madopilot.h`](../../crates/bindings/capi/include/madopilot/madopilot.h).
The pre-integration contract and independent review remain in the Change's
local planning evidence. The shipped header and Rust layout probe fix field
order, numeric tags, active fields, nullability, output reset states, and
entry/owner extents; no second public header is introduced.

### Records and compatibility

| Record | Mandatory initial prefix | Purpose |
|---|---:|---|
| `madopilot_template_scheduler_descriptor_t` | 48 bytes | All eight existing fixed scheduler limits |
| `madopilot_template_watch_options_t` | 72 bytes | Optional existing match options/region, explicit rate, stability, change policy |
| `madopilot_template_query_snapshot_t` | 160 bytes | State/id, complete optional stamp, generation, stability, nine work counts, two bounded depths |
| `madopilot_transform_snapshot_t` | 88 bytes | Exact retained frame extent, geometry, target coverage, optional placement and both scales |
| `madopilot_template_query_result_info_t` | 272 bytes | Terminal kind/status, overload, query id and all applicable matched facts |

The initial layout derivations are now exercised by the Apple Silicon C/Rust
probe; both release-target reports remain required for release qualification.
Each record's complete initial prefix is named by its `*_SIZE_V1_6` constant.
Semantic counts and nanoseconds have fixed widths, while existing pointer-view
lengths and accessor indexes retain `size_t`. Embedded records are fixed values:
only the outer output size is caller input; the library initializes nested sizes.
Future append operations cannot grow an embedded value and move later fields.

The suffix order and derived end extents are:

| Entry | End extent |
|---|---:|
| `engine_template_scheduler_descriptor` | 744 |
| `session_start_template_watch` | 752 |
| `template_query_retain` | 760 |
| `template_query_release` | 768 |
| `template_query_poll` | 776 |
| `template_query_wait` | 784 |
| `template_query_cancel` | 792 |
| `template_query_result_retain` | 800 |
| `template_query_result_release` | 808 |
| `template_query_result_info` | 816 |
| `template_query_result_match_at` | 824 |
| `template_query_result_frame` | 832 |
| `template_query_result_error` | 840 |

The complete table is `736 + 13 × 8 = 840` bytes. The old
provider-descriptor extent becomes `offsetof` the first watcher entry, preserving
736 rather than following a growing `sizeof(table)`. Prior 424/592/648/720/736-byte
prefixes, old numeric values, and unsupported ABI 1.1 remain unchanged.

A direct C caller checks every invoked entry and returned owner's lifecycle.
Query shared lifecycle alone ends at 768; result shared lifecycle ends at 808.
Both complete C++ owners require through 840, including transitive frame,
mapping, and error lifecycles from the old prefix. Check caller-known **and**
library-reported extents before reading pointers, then reject missing required
entries before a creating call. Descriptor-only access requires 744.

### Request and observable outcomes

Start receives session, prepared template, options, and query-lifetime operation
separately. It borrows caller storage only for that call. Existing matching
option precedence and find-request CapturePixels/clip conversion remain the one
validation implementation. An inactive region is ignored; inactive stability
count/duration must be zero. Zero interval is unrestricted. Immediate,
consecutive-positive-count, and positive-duration stability use existing Rust
constructors. Change policy is explicit: analysis-always is 0 and exact-RGBA is 1.
An all-zero options payload does not silently mean the Rust exact-RGBA default.

The query considers current maintained state once, then strictly newer frames.
No-match remains pending. Pending snapshots contain all nine saturating work
counts; `pending_count` and `in_flight_count` are each 0..1 per query. The
scheduler's limit of two analyses is engine-wide, not per query. Optional stamp
presence covers all four identity components. Polling cannot create observations
or advance confirmed stability. Terminal snapshots retain only state/id and
clear pending fields; those zeros are not final accounting.

| Observation | Call status | Primary output |
|---|---|---|
| Poll pending | `OK` | Value snapshot, null result |
| Poll/wait observes any query terminal | `OK` | Owned immutable terminal result |
| Only caller-wait authority ends | `CANCELLED` / `DEADLINE_EXCEEDED` | Null result; query unchanged |
| Valid cancel, including after a match | `OK` | Authoritative winner, not necessarily Cancelled |
| Invalid start or runtime admission refusal | Existing typed failure | Null query |
| Non-Matched frame/match access or invalid index | `INVALID_ARGUMENT` | Null frame/reset match |
| Error accessor on Failed | `OK` | Independently owned structured error |
| Error accessor on any other terminal | `OK` | Null error |

Terminal tags distinguish Matched, Cancelled, DeadlineExceeded, SessionClosed,
SchedulerClosed, TargetLost, Overloaded, and Failed. Their status projection is
respectively `OK`, `CANCELLED`, `DEADLINE_EXCEEDED`, `CLOSED`, `CLOSED`,
`TARGET_LOST`, `LIMIT_EXCEEDED`, and the retained failure status. QueueExpired is
the sole overload reason; start-time capacity refusal is not a terminal result.

A matched result exposes effective options, backend/template identities, target,
positive match count, full source stamp, effective region, confirmed count/span,
and frame-time transform. Placement presence does not imply target and desktop
scales are equal. Non-Matched fields are explicitly inactive/reset. Strings
borrow the result owner; exact frame access shares storage through the ordinary
frame/mapping API. Adding transform values to result info closes the missing
foreign observation without a fourteenth entry or a conversion subsystem.

### Errors, outputs, and authority

Preflight every independently valid output before conversion or mutation, even
when another output is invalid. Invalid output storage is never written. Legal
prefixes receive their documented reset state; unknown trailing caller bytes
remain untouched. Required pointers cannot be null; retain/release accept null.
Call `out_error` is optional. Result-error's required `out_failure` is terminal
data, not optional call-error storage. Outputs cannot overlap inputs or each
other. Pointer validity/type/lifetime are caller preconditions, not a fabricated
opaque-handle liveness check. Existing panic containment publishes no partial
owner, but cannot undo cancellation that already won.

[`Error`](../../crates/automation/core/src/status.rs) actually contains status
and redacted detail, not a backend identifier, asset subcode, or platform error
chain. Preserve those two fields; use the draft's explicit status-to-category
mapping, leave backend/asset presence unset and asset codes Unknown. Do not
invent missing provenance or parse messages. Error access may allocate owned
error storage; ordinary poll/wait do not eagerly construct it or emit new logs.

Wait always delegates to Rust before consulting/materializing a terminal cache.
An already-interrupted wait operation is checked before an existing terminal;
there is no extra C commit check after Rust returns a terminal. Similarly, Rust
owns start admission/publication and terminal competition. A C cache cannot
bypass those operations or introduce new deadline/close/cancel precedence.

### Lifecycle and concurrency

- C query retain shares one payload containing one Rust `TemplateQuery` and the
  existing non-owning `MappingObserver`. Non-final release does not cancel; final
  release drops that Rust owner. Release/cancel is not a general drain fence.
- Cache at most one terminal projection retaining query id and
  `Arc<TemplateTerminalOutcome>`. Pending poll allocates no result storage.
  Later terminal observations share the projection, not a copied match array.
  Hold no cache synchronization across Rust poll/wait/cancel, parent close,
  backend/mapping work, or destruction that can cancel or drain.
- Result owners do not retain query/session/engine/thread owners. Frame creation
  receives the saved mapping observer, which keeps no parent open. A result,
  exact frame, or mapping remains safe after all parents are released; borrowed
  views last only as long as their explicitly named result/error/mapping owner.
- Each concurrent caller keeps its own reference throughout poll/wait/cancel or
  read. Final release racing an unretained call remains outside the ABI contract.
  Callers synchronize shared output buffers.
- Explicit `session_close` establishes SessionClosed authority and refuses new
  starts even when its drain is interrupted. Releasing a session reference is
  not close. Final `engine_release` destroys the engine and establishes scheduler
  shutdown; pending queries observe SchedulerClosed unless prior authority won.
  A retained session cannot restart a closed scheduler. No new engine-close API
  or parent registry is introduced.
- An existing terminal winner is immutable. Before a winner exists, the runtime's
  established session-close and scheduler-close authority can defeat a proposed
  cancellation. The C/C++ adapter does not reorder the race.
- C++ owners remain move-only, explicitly clonable, and exception-free for
  MadoPilot failures. Pending facts are values; string-bearing access is
  lvalue-only with `BorrowedStr`. A request projection owns its call-local options
  backing and rebinds interior pointers after each supported copy/move, including
  assignment and any still-usable moved-from projection.

## Alternatives

- **Reuse the synchronous result owner:** would require a new public
  `FindOutcome` constructor and/or copying its match array. It also conflates
  terminal query failure with completed one-shot matching. Rejected in favor of
  direct access to the already shared terminal value.
- **Return terminal failure as call failure:** loses whether the query ended or
  only one caller stopped waiting. Rejected; terminal status is data.
- **Copy results into caller buffers or create one owner per poll:** complicates
  post-parent lifetime and permits repeated allocations/history. Rejected;
  pending snapshots are fixed values and terminal projection is shared once.
- **Retain the session to construct later frames:** changes parent teardown and
  retains unrelated resources. Rejected; use the existing mapping observer.
- **Expose callbacks or another scheduler:** adds unrelated dispatch, locking,
  cancellation, and drain contracts. Rejected; this is pull-only facade access.

## Consequences

Old callers need no migration. New callers must negotiate the complete ownership
surface and distinguish call errors from terminal outcomes. The added result-info
payload is larger because complete source and transform facts are observable,
but it copies only fixed values/views, never match arrays or pixels. New records
and slots become immutable within ABI major 1 once released.

Canonical C/C++ headers, boundary/layout reporting, consumers and ownership docs
must be synchronized during integration. No Rust watcher semantics, provider,
permission behavior, native defaults, packaging or product release changes here.
Historical Rust/native measurements retain their original source authority.

Two release-validator tests exposed a stale fixture dependency: they copied the
current C/C++ headers into a synthetic `v0.4.0` observation. ABI 1.6 correctly
failed that old release's no-watcher rule. The fixture now uses the frozen ABI
1.5 C header and a minimal C++ lexical-scope input; exact release source remains
bound by the unchanged required tree/blob identities. This narrowly replaces
ADR 0065's test-byte-preservation statement, not its gate or historical evidence.
The validator, release body, old ADR, hashes, and rejection oracles stay unchanged.

## Verification

The initial draft was derived from direct source and available LSP definitions;
partial/stale graph positions were supplemented by source reads. Independent
contract review preceded implementation. Local Apple Silicon integration
exercises the C/Rust layout probe, all five frozen-header consumers, current
C/C++ examples, CMake consumers, deterministic lifecycle/race tests, and paired
boundary allocation/correctness smoke. These are not native foreign evidence.

The independent C++ review found a test-apparatus defect: allocation-failure
injection affected backend workers. Caller-local state fixes it; a single-point
mutation removing `thread_local` makes the named caller-versus-worker regression
fail, while the corrected version passes. Product behavior did not change.

Complete release qualification still requires these evidence gates:

1. Independent contract review, then Rust/C size/alignment/offset measurements
   on both release targets, including nested layouts and all table extents.
2. Frozen 1.0/1.2/1.3/1.4/1.5 consumers; malformed prefixes/active fields/outputs;
   Failed detail and wrong-outcome/index failure states.
3. Deterministic independent wait interruption, terminal/cancel/close races,
   retained-thread lifetimes, parent teardown, exact frame/stamp/transform and
   mapping survival, and C++ short-table/lifecycle/copy-move checks.
4. Pending allocation and repeated-terminal bounded-storage evidence, with new
   target-specific `G-013` foreign-overhead budgets accepted before enforcement.
5. Protected PR hosted checks before approved native C and C++ flows on Windows
   and macOS. CI proves neither interactive capture permission nor native
   qualification. Unexecuted native rows remain unexecuted; this ADR closes no
   packaging, static-artifact, provider, or release gate.
