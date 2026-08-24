# The C ABI

MadoPilot's C boundary is a separately versioned contract with its own
compatibility rules. This document is the part of it that does not fit in a
declaration: what a handle's lifetime is, what a `struct_size` means in each
direction, which status a caller can branch on, and how to build against the
library on each release target.

The declarations themselves live in
[`crates/bindings/capi/include/madopilot/madopilot.h`](../crates/bindings/capi/include/madopilot/madopilot.h).
The replay workflow is
[`examples/c/deterministic-slice.c`](../crates/bindings/capi/examples/c/deterministic-slice.c);
the native Windows and macOS common flows are under
[`examples/c/`](../crates/bindings/capi/examples/c/).
The production and fixture one-shot OCR flows are
[`examples/c/ocr-default.c`](../crates/bindings/capi/examples/c/ocr-default.c) and
[`examples/c/ocr-fixture.c`](../crates/bindings/capi/examples/c/ocr-fixture.c).

A C++ caller uses the header-only RAII wrapper over this contract rather than
calling the table directly; see [cpp-wrapper.md](cpp-wrapper.md). Everything
below still applies to it, because it is the same contract.

## ABI 1.3, with complete released 1.0 and 1.2 prefixes preserved

The current header declares ABI 1.3. ADR 0007 freezes ABI 1.0's 424-byte
capture/matching table; ADR 0023 freezes ABI 1.2's 592-byte input/diagnostic
table. ABI 1.3 appends one-shot OCR execution and immutable owned OCR results
through 640 bytes under ADR 0035, then the accepted default constructor for a
complete 648-byte table under
[ADR 0036](adr/0036-default-ocr-composition-and-abi-prefix.md).
The unreleased 1.1 draft remains intentionally unsupported. Within ABI major 1:

- no released numeric value changes its number;
- no released structure field moves, and none is removed;
- no released function-table entry moves, and none is removed;
- a later minor appends — to the end of a structure or the end of the table —
  and raises `MADOPILOT_ABI_MINOR`.

A different ABI major is a different library, and `madopilot_get_api` refuses it.
Use the smaller of caller `sizeof(madopilot_api_t)` and the returned
`struct_size`. ABI 1.0 and 1.2 callers negotiate 424 and 592 bytes. An ABI 1.3
caller checks the entry-specific macro: 640 bytes reaches the complete OCR
owner/accessor suffix, while
`MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_DEFAULT_OCR` requires the complete
648-byte table. The C++ wrapper requires the complete lifecycle/accessor suffix
before constructing an owner and all 648 bytes before default construction.

The released promise is executable. `tests/abi-compat/v1/` and `v1_2/` keep exact
headers and callers, compile them without the working header, link them to the
current library, negotiate only their declared extents, and run their complete
flows. Current C++ checks also negotiate complete 1.2 and partial 1.3 extents and
refuse OCR before reading a missing entry.

## Migrating from the unreleased 1.1 draft

The 1.1 header was development-only. Recompile against 1.2 or 1.3; do not copy
its numeric values, layouts, or offsets. Explicit input routes, owned receipts,
submission evidence, and bounded diagnostics are the ABI 1.2 replacement. The
frozen ABI 1.0 prefix remains compatible; there is no 1.1 alias, tombstone,
reserved slot, or negotiation profile.

## Migrating an ABI 1.2 caller to one-shot OCR

An existing 1.2 caller remains unchanged. To use OCR:

1. compile the ABI 1.3 header and require
   `MADOPILOT_API_SIZE_OCR_RESULT_TEXT_AT` before creating an OCR owner;
2. choose explicit package-backed composition or the accepted default:
   - explicit composition retains one validated package and fills
     `madopilot_ocr_request_t` with that package/model and the configured backend
     identity;
   - default composition requires
     `MADOPILOT_API_SIZE_ENGINE_CREATE_WITH_DEFAULT_OCR`, fills a standalone
     `madopilot_default_ocr_options_t` with canonical model-root/runtime views,
     and constructs through `engine_create_with_default_ocr`;
3. retain one exact source frame for the synchronous call, keep backend/model
   views exact, and name output space plus any capture-pixel region;
4. pass the same operation record across validation, mapping, backend work, and
   final commit;
5. own the returned `madopilot_ocr_result_t` independently, use fixed-width
   `ocr_result_info`, indexed region geometry/confidence, and borrowed text views,
   and release with the module entry; and
6. keep each borrowed text view only while its result owner remains retained.

For the exact integrated default, `madopilot_ocr_request_t.package` may be null
when backend/model views match the default identities returned by
`describe_build`; any explicit package model still requires its package. There
is no watcher, retry, callback, scheduling, fallback, automatic input, ambient
runtime/model search, download, or bundling. The feature-gated
`private-fixture` constructor remains outside the public header/table and absent
from release builds.

## One exported symbol

In release builds, `madopilot_get_api` is the only exported symbol. Every public
operation is a member of the immutable function table it returns.

```c
const madopilot_api_t* api = NULL;
madopilot_status_t status = madopilot_get_api(
    MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, sizeof(madopilot_api_t), &api);
```

One symbol means one thing to negotiate. A caller that obtained a table has, by
construction, agreed the ABI major, the minimum minor, and how much of the table
it understands; fifty exported symbols would each be a separate chance to bind
to something the negotiation would have refused.

Negotiation refuses a different major and a minimum minor newer than the library
with `MADOPILOT_STATUS_UNSUPPORTED`, and a caller-declared table size below
`MADOPILOT_API_SIZE_INFORMATION` with `MADOPILOT_STATUS_INVALID_ARGUMENT`. A
caller *larger* than the library is not an error: the returned table's
`struct_size` says how much of what the caller knows is really there, and the
caller uses the smaller of the two.

Within an ABI major a member's position is permanent. Later phases append.
Nothing is reordered, removed, repurposed, or reserved as a null slot for work
that does not exist yet.

## Handles

Every owned handle is reference counted.

| Rule | What it means |
|---|---|
| `*_retain` adds one owned reference | Null is a no-op |
| `*_release` drops one owned reference | Null is a no-op; the last one destroys the state exactly once, with the allocator that created it |
| Every other entry rejects null | "Do nothing" is not an answer to a question that has one |
| A handle must stay retained for the whole call it is passed to | Releasing the last reference concurrently with a call that has not retained one of its own is outside the contract, because no check can distinguish it from a valid pointer without racing the release it is trying to detect |
| Const access is safe from several threads at once | While each keeps a live reference |

**Releasing a parent never invalidates a separately retained child.** This is
the rule the whole design turns on, and it is what makes the following true:

- a mapping stays readable after the frame, the session, and the engine are
  released, because the mapping owns or retains the storage its byte view
  borrows;
- a prepared template outlives the package it was compiled from;
- a match result outlives the session, template, package, and engine because it
  owns the exact frame it searched;
- an OCR result outlives its frame, package, session, engine, and backend because
  it owns immutable public text, geometry, source identity, and descriptors;
- an input receipt and all of its indexed attempt values outlive the session
  and engine;
- an independently retained diagnostic reader keeps the sealed stream alive
  after engine release, and an owned batch outlives both engine and reader.

**A borrowed view is valid only while its owner is retained.** Each declaration
names the owner. Error messages borrow from errors, match template IDs borrow
from match results, target names borrow from target lists, mapping bytes borrow
from mappings, and OCR text borrows from the OCR result. Receipt attempts and
diagnostic records are value fields. Copy any borrowed view needed after final
owner release.

**A view the caller supplies is borrowed the other way, for exactly the call.**
Every input structure, every view it carries, and every view passed directly as an
argument — a template identity, a target name — must be readable for the duration
of the call and must not be modified during it, whether the library reads it once
or reads it from start to finish, which it does for
`madopilot_package_source_t.archive`. The library retains no caller memory past
the call that received it: whatever it must keep, it copies or converts into
storage of its own, so a package loaded from a caller's archive stays valid after
that archive is freed. The rule is the caller's half of the same contract the
paragraph above states for the library's half.

## Size-versioned structures

Every extensible structure begins with `uint32_t struct_size`, immediately
followed by a second 32-bit field so that no implicit padding is introduced
between them. That second field is `flags` where the structure has presence bits
and a meaningful discriminant where it has one.

A caller sets `struct_size` to `sizeof` the structure **as its own header
declares it**.

**Reading an input**, the library:

- refuses a size below the documented mandatory prefix with
  `MADOPILOT_STATUS_INVALID_ARGUMENT`, without reading past `struct_size`;
- refuses a size that ends inside a field, which would leave that field neither
  supplied nor omitted;
- refuses an array element whose `struct_size` is above the array's declared
  element stride, which would read past the extent the array declared;
- refuses a size that does not reach a field whose presence bit is set, rather
  than applying the omitted-field default to a field the caller claimed;
- applies the documented default to every field the size omits;
- ignores trailing bytes it does not recognize.

A size describes a prefix, so it has to end where a prefix can end. That is what
the three middle bullets say, and each of them is a refusal rather than an
adjustment: the library does not round a size down to the nearest field
boundary, does not clamp an element to its stride, and does not drop a presence
bit it cannot honor. Rounding down would run the request with a field the caller
supplied silently discarded — a cancellation token, or a minimum score the
caller believes is in effect. All three report
`MADOPILOT_STATUS_INVALID_ARGUMENT` with `MADOPILOT_ERROR_CATEGORY_ABI`, like
the mandatory-prefix refusal above them.

A caller that sets `struct_size` to `sizeof` the structure as its own header
declares it satisfies all three, because a released header's own size is a field
boundary, its `frame_stride` is its own element size, and a full-size prefix
covers every field a presence bit can name.

**Writing an output**, the library:

- refuses the same way;
- writes only within the declared size;
- writes back the number of bytes it actually populated, so a caller built
  against a newer header learns how much of what it knows is really there.

The pointer-length `madopilot_str_t` and `madopilot_bytes_t` primitives carry no
`struct_size`. Semantic numeric fields use fixed widths: structure/table sizes
are `uint32_t`; row strides and semantic OCR/match/package/receipt/diagnostic
counts are `uint64_t`; `size_t` is limited to addressability quantities such as
view lengths, caller array extents, accessor indexes, and negotiated table
extent. ABI 1.3 keeps those rules.

`madopilot_diagnostic_record_t` has a 240-byte mandatory ABI 1.2 prefix and
appended ABI 1.3 OCR fields. A 1.2 caller receives its exact initialized prefix;
a 1.3 caller receives the opaque model instance, accepted profile
classification, requested geometry, typed outcome, timing, and source-pixel
counter too.

**One structure has two mandatory prefixes.** `madopilot_match_options_t` is the
only one the table uses in both directions. As an *input* its mandatory prefix
is eight bytes, through `flags`, because a structure that sets no presence bit
is how a caller asks for the prepared template's own defaults. As the *output*
of `result_options` the mandatory prefix is the whole structure: that report
says which thresholds the search really ran under, every field was in effect,
and a shorter one would drop an option without saying so. A caller that passes
the input prefix to `result_options` gets `MADOPILOT_STATUS_INVALID_ARGUMENT`;
pass `sizeof(madopilot_match_options_t)`. Every other structure has one prefix
that means the same thing whichever way it travels.

An array of versioned structures needs its element stride passed explicitly:
`madopilot_source_t.frame_stride` is `sizeof(madopilot_replay_frame_t)` as the
caller's header declares it. A caller built against an older header has smaller
elements, and the library cannot guess the spacing of an array it did not
declare.

ABI 1.2 has these conditional-prefix cases:

- `madopilot_input_event_t` reaches through `button` for pointer press/release,
  `key_value` for key press/release, `x` and `y` for pointer move,
  `horizontal` and `vertical` for scroll, `text` for text, and
  `delay_nanos` for delay. Fields belonging to another event kind are ignored.
- `madopilot_input_request_t` reaches through `source_frame` normally. Setting
  `MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET` requires the complete structure
  through `cleanup_timeout_nanos`.
- `madopilot_ocr_request_t` reaches through `output_space` normally. Setting
  `MADOPILOT_OCR_HAS_REGION` requires the complete structure through `region`.
The event array carries both `event_count` and `event_stride`. Its elements may
come from an older header, but every element must end on a prefix valid for its
selected kind and fit within that stride.

## Validation, and what an output looks like on failure

Before a request is validated, every independently valid output is set to its
documented failure state: an owned handle output to null, a structure through its
failure prefix, a scalar to zero. An invalid sibling output does not prevent that
initialization. On failure valid outputs stay that way, so a caller never sees a
partially initialized or stale value.

Validated before use: every pointer-length pair, every active tagged-source
field, every integer conversion, every alignment requirement, every offset,
every element stride, and every allocation-size calculation. A null pointer with
a nonzero length is rejected before the pointer is read; a null pointer with a
zero length is accepted only where the declaration documents an empty view as
meaningful.

**Every pointer parameter is required unless its declaration says otherwise**,
and a null one is `MADOPILOT_STATUS_INVALID_ARGUMENT`. The rule covers request,
source, operation, and input-policy structures as well as handles, so a null
`const madopilot_operation_t*` is refused rather than read as "no deadline" —
the way to say that is an operation whose `flags` set no bit. The two are
different requests: an absent structure declares nothing at all, while an empty
one declares which header the caller was built against and how much of the
structure it filled in. The rule applies equally to the ABI 1.2 operations:
`session_open_with_input` requires both request records, and
`engine_permission`, `engine_input_descriptor`, and `session_send_input` each
require an operation record.

Beyond `*_retain` and `*_release`, which accept null as a no-op, and the
empty-view rule above, null is accepted only where the declaration names an
optional value: `out_error`, `madopilot_operation_t.cancellation`,
`madopilot_find_request_t.frame`, `madopilot_find_request_t.options`, and
`madopilot_input_request_t.source_frame` when its pointer geometry does not
require a frame snapshot. Array pointers may be null only when their count is
zero; semantic validation still rejects an empty delivery plan.

The library does not probe arbitrary addresses. The caller remains responsible
for the validity of the addresses it passes, for the declared duration of the
call.

## Deadlines, cancellation, and activity correlation

Every potentially blocking entry takes a `madopilot_operation_t`. Its deadline
is an **absolute instant** in the library's own monotonic domain, in nanoseconds
since an origin fixed for the life of the loaded library. Read the current
instant with `clock_now` and add to it. A duration would restart at every hop;
the absolute instant names the same moment throughout the call. The origin is
not wall-clock time and must not be presented as one.

The implementation checks cancellation and deadline before admission and before
committing a successful result, so a value that loses the race is dropped rather
than published. Each contract underneath does the same. An admitted input
receipt is already the operation's terminal outcome, so a late interruption
cannot replace it with a second result.

`MADOPILOT_OPERATION_HAS_ACTIVITY_TAG` makes the nonzero `activity_tag` available
on every diagnostic record that operation produces. The value is opaque
correlation metadata, not a confidentiality boundary; callers must not place
secrets in it. A platform route may carry it in documented native observational
metadata. In particular, macOS `ProcessDirected` copies it to the Core Graphics
event-source user-data field so the addressed process can correlate an observed
event. Changing or omitting it cannot affect admission, ordering, deadline,
cancellation, identity, posting, or result semantics. It correlates observations;
it does not establish causality.

## Statuses, owned errors, and admitted receipts

A caller branches on `madopilot_status_t`. The message an error handle carries is
diagnostic and is never required for control flow.

| Status | Meaning |
|---|---|
| `MADOPILOT_STATUS_OK` | Every required output is populated |
| `MADOPILOT_STATUS_INVALID_ARGUMENT` | Malformed, out of range, or naming something unknown |
| `MADOPILOT_STATUS_UNSUPPORTED` | Well-formed, but this build cannot satisfy it |
| `MADOPILOT_STATUS_CANCELLED` | The token was set before the result committed |
| `MADOPILOT_STATUS_DEADLINE_EXCEEDED` | The deadline passed before the result committed |
| `MADOPILOT_STATUS_CLOSED` | The session starts no further work |
| `MADOPILOT_STATUS_TARGET_LOST` | The capture target no longer exists |
| `MADOPILOT_STATUS_LIMIT_EXCEEDED` | A configured or implementation limit |
| `MADOPILOT_STATUS_CAPTURE_FAILED` | Capture could not produce the frame |
| `MADOPILOT_STATUS_ASSET_INVALID` | A package broke a rule that makes it trustworthy |
| `MADOPILOT_STATUS_VISION_FAILED` | The backend was unavailable or could not finish |
| `MADOPILOT_STATUS_INTERNAL` | An invariant the library owns did not hold |
| `MADOPILOT_STATUS_INTERNAL_PANIC` | A Rust panic was contained at the boundary |
| `MADOPILOT_STATUS_INPUT_FAILED` | Input was refused before admission; no terminal receipt exists |

`MADOPILOT_STATUS_INTERNAL_PANIC` is the boundary's own status and has no Rust
counterpart. `MADOPILOT_STATUS_INPUT_FAILED` is intentionally pre-admission:
once a sequence is admitted, its outcome is receipt data rather than a second
fallible return channel.

**Package loading carries more than a status.** Every other operation in the
facade reports a status plus diagnostic text. Package loading reports which rule
was broken *and* how far loading had got, deliberately: a bad content hash and an
unsafe entry path are both `MADOPILOT_STATUS_ASSET_INVALID` and are not the same
problem. `madopilot_error_detail_t` therefore carries `asset_fault` and
`asset_stage` alongside the status, flagged by
`MADOPILOT_ERROR_HAS_ASSET_DETAIL`. Flattening them into one status here would
have thrown away detail the Rust layer took care to keep.

A related line worth knowing: asking a **loaded** package for a template identity
it never declared is `MADOPILOT_STATUS_INVALID_ARGUMENT`, not
`MADOPILOT_STATUS_ASSET_INVALID`. A package that loaded is valid; the mistake is
the caller's. The error's category is still `MADOPILOT_ERROR_CATEGORY_ASSET`,
because the mistake is about the package's contents. That refusal **does** carry
the fault pair — `MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` at
`MADOPILOT_ASSET_STAGE_COMMIT` — because `template_prepare_from_package`
resolves the identity against the package before asking the backend for
anything. It names no backend, because none ran. The status says whose mistake
it was and the fault pair says which one; see
[ADR 0007](adr/0007-phase-1-c-abi-freeze.md), decision 4.

**A caller-supplied C region is in capture pixels.** Any other space is
`MADOPILOT_STATUS_INVALID_ARGUMENT` with `MADOPILOT_ERROR_CATEGORY_ABI`; this
applies to map, find, and OCR request regions. The C table has no general
coordinate-conversion entry, so callers convert before the request. The Rust
facade is broader because it can resolve other spaces through the immutable frame
transform; the C surface intentionally stays narrower.

`madopilot_pixel_rect_t.space` is still read in the other direction: on a
rectangle the library writes, it names whichever space that rectangle was
measured in.

`MADOPILOT_SPACE_TARGET_NORMALIZED` and `MADOPILOT_SPACE_FRAME_NORMALIZED` are
two bits over one set of numbers in the capture prefix. A frame covers exactly
here, so a target-normalized coordinate and a frame-normalized one address the
same point; a session advertises the target-normalized bit when its source
declares that its frames cover the target, and never as a claim that some other
extent applies. The first phase that captures a sub-region of a target makes the
two differ, and that is when the distinction begins to carry information. See
[ADR 0009](adr/0009-phase-1-normalized-coordinate-spaces.md).

There is no global, thread-local, or engine-wide last-error slot. A failure
belongs to the call that produced it, and a slot would make two threads' failures
each other's business. `out_error` may be null, and then only the status is
reported.

A rejected output argument is described like any other invalid argument. An entry
initializes every valid output before it validates anything, so a caller's stale
error handle never survives a call; when `out_error` itself passed validation, the
entry then reports through it which output was null or misaligned. Only a call
whose `out_error` is the rejected output gets the status alone, because there is
then nowhere to put the message.

## One-shot OCR and immutable results

`session_recognize` initializes both outputs before reading inputs. The session,
exact frame, request views, and operation record are borrowed for the
synchronous call. An explicit request also borrows its package, which resolves a
complete validated model/profile identity. The accepted default may pass a null
package only with the exact backend/model identity retained by the engine.
Backend ID/version and model must exactly match the session. A foreign stream,
missing backend, unknown model, missing required package, malformed view, invalid
region, deadline, cancellation, close, or backend fault returns one typed status
with no partial result.

Success returns one opaque immutable `madopilot_ocr_result_t`. `ocr_result_info`
reports complete source identity, effective region, output space, fixed-width
count, and borrowed backend/model/profile views. `ocr_result_region_at` reports
four points and finite confidence; `ocr_result_text_at` returns normalized text
borrowed from the result owner. Retain/release are atomic, const access is
concurrent, indexes are checked, and releasing every parent leaves the result
unchanged.

Runtime performs final deadline/cancellation/close arbitration before the C
boundary publishes the handle. A backend panic is contained as
`MADOPILOT_STATUS_INTERNAL_PANIC`; result and error outputs remain initialized
and no unwind crosses C.

`madopilot_default_ocr_options_t` is a separate size-versioned record containing
only model-root and runtime-path views. It was not appended to the frozen
`madopilot_engine_options_t`: that structure remains size 16, alignment 4 on both
release targets. `engine_create_with_default_ocr` validates the controlled
runtime first, then the two fixed accepted model paths, and returns no engine on
failure. It never changes the behavior of `engine_create`.

## Native capabilities and non-prompting permissions

ABI 1.2 makes capability checks explicit before a caller opens anything.
`engine_capabilities` reports whether the configured source can submit input and
whether it can read permission state.
`target_list_input_capability` reports one target and operation/route pair:
compatibility support, exact address scope, focus requirement, accepted pointer
spaces, related permission, and strongest submission evidence.
`engine_input_descriptor` re-reads live capability before open;
`session_input_descriptor` reports the immutable policy the session accepted.
Its `known_pairs`, `supported_pairs`, and `unknown_pairs` keep attemptability
separate from positive application-compatibility evidence.

`engine_permission` is a probe, never a request. It presents no UI and calls no
permission-request API. macOS reports Screen Recording and input-control —
the public non-prompting event-post-access preflight — separately. Only
`MADOPILOT_PERMISSION_STATE_GRANTED` is authorization; `UNKNOWN`,
`NOT_GRANTED`, and `UNAVAILABLE` promise no operation will succeed.
The optional diagnostic is redacted, and its string views borrow from the
retained engine. Windows advertises no readable permission mechanism:
`MADOPILOT_ENGINE_READS_PERMISSIONS` is clear and `engine_permission` returns
`MADOPILOT_STATUS_UNSUPPORTED` with an initialized output and owned error.

The older `madopilot_target_t` and `madopilot_session_info_t` records grew only
at their tails. A 1.0 caller still receives its old prefix. A 1.2 caller can read
target kind, capture support and permission, session target identity, and
whether input was established.

Target and stream identities cross this boundary as `uint64_t` scalars that
directly project the engine's own identity ordinals; the boundary keeps no
second registry mapping them, so nothing grows with discovery or stream
lifetime. They are engine-scoped, not globally comparable: two engines may
hand out the same numbers, and a value correlates targets, sessions, frame
stamps, receipts, and diagnostic records only within the engine that issued it.

## Input admission, submission evidence, and receipts

Input operation and route are separate axes. Pair masks name the nine exact
combinations across pointer/keyboard/text and `SYSTEM`, `WINDOW_MESSAGE`, and
`PROCESS_DIRECTED`; capability for one pair never implies another. Routes say
how the native API addresses work:

| Route | Address scope | What the route name claims |
|---|---|---|
| `SYSTEM` | focused system or platform target | submission through a system input mechanism |
| `WINDOW_MESSAGE` | one exact retained window | a message addressed to that window |
| `PROCESS_DIRECTED` | one owning process | a process-scoped transport |

The separate evidence value states the strongest observed transport fact:
`INVOCATION_ONLY`, `SYSTEM_INPUT_ADMISSION`, `TARGET_QUEUE_ADMISSION`, or
`TARGET_PROTOCOL_ACKNOWLEDGEMENT`. None of these alone claims game/application
consumption or visual change.

`session_open_with_input` keeps input policy separate from the frozen
`madopilot_open_request_t`. A required policy fails without opening when its
pairs cannot be established. An optional policy may open capture-only, visible
through `madopilot_session_info_t.accepts_input` and the session descriptor.

`madopilot_input_request_t` supplies a bounded event array, ordered route plan,
explicit focus and geometry policies, optional source frame, and optional
cleanup bounds. The plan is the only fallback authority: the library never
substitutes a route the caller omitted. Event text, arrays, and source-frame
reference are borrowed for the call; the frame remains retained through
`session_send_input`.

A delay-only sequence names no operation kind but still travels an explicit
route: preflight derives its submission evidence from that route's actual
first attemptable operation pair, and a route with no attemptable pair is
refused rather than granted invented evidence.

The ABI publishes every fixed input ceiling rather than requiring discovery by
rejection:

| Contract | Published ceiling |
|---|---:|
| Events in one sequence | `MADOPILOT_INPUT_MAX_EVENTS` = 256; a descriptor may be lower |
| Text in one event | 4,096 Unicode scalar values and 16,384 UTF-8 bytes |
| One delay | 5,000,000,000 ns |
| Either scroll component | absolute value at most 120; both cannot be zero |
| Function-key number | inclusive 1 through 24 |
| Explicit cleanup | 256 releases and 250,000,000 ns |

`MADOPILOT_KEY_CHARACTER` accepts one non-control Unicode scalar value. Values at
either exact ceiling remain valid.

A refusal before admission returns `MADOPILOT_STATUS_INPUT_FAILED`, leaves the
receipt handle null, and may return an owned error. After admission, a normal
return reports `MADOPILOT_STATUS_OK` with exactly one immutable owned receipt:

- `COMPLETE` means every complete logical event reached its route's submission
  threshold;
- `UNEXECUTED` means no native unit may have had an effect;
- `PARTIAL` records a stopped sequence, including a possible partial native
  effect before any complete logical event was submitted.

`input_receipt_info` reports target, outcome, attempt count, selected route and
address scope, submitted count, optional last-submitted index, evidence, typed
fault, fallback, partial-native-effect flag, and cleanup accounting. Indexed
`input_receipt_attempt_at` values preserve each refused or attempted route in
order. Semantic receipt and attempt counts — attempts, submitted, optional
last-submitted, cleanup released and owed — are `uint64_t`; the attempt
accessor's `size_t` index and output count are addressability, not semantics.
Presence flags distinguish absent values from valid zeroes. `NOT_NEEDED`
and `COMPLETE` are the only cleanup values proving no sequence-owned state
remains held; treat `INCOMPLETE`, `EXHAUSTED`, and unknown later values
conservatively.

A contained boundary panic leaves the receipt handle null but cannot prove no
native input took effect, so the caller must not automatically retry.
Windows advertises exact-window `WindowMessage` for ordinary retained
top-level windows as unknown-but-attemptable with target-queue-admission
evidence. The dedicated fixture raises the same route to supported with
target-protocol acknowledgement. Both remain separate from Windows system
routes. The macOS implementation reports system routes plus process-directed
pairs with owning-process scope, unknown compatibility, and invocation-only
evidence for retained top-level windows. Final candidate `dec43d7` passed the
controlled profiles; independent `single`, exact two-display non-mirrored
`same-scale`, and `mixed-scale` matrices passed for all fourteen controlled
pairs. Their release decision is 14 qualified, 0 rejected, and 0 unexecuted.
Additional windows in
the same process do not revoke that scope, and no exact-window route exists on
macOS. The negotiated capability report, not a platform guess in the caller,
decides what may be admitted.

## Bounded diagnostic stream

Diagnostics are configured when the engine is created with
`engine_create_with_options`. `OFF` requires capacity zero, allocates no queue,
and issues no diagnostic operation or template identities. `NORMAL` and `DEBUG`
require capacity `1..=65,536`; a larger value is
`MADOPILOT_STATUS_LIMIT_EXCEEDED`. The frozen ABI 1.0 `engine_create` entry
remains equivalent to `OFF`.

One enabled engine exposes one independently retainable reader through
`engine_take_diagnostic_reader`; a second take is rejected. Producers never
block or call host code. A full queue or contended lock discards the record and
increments an exact normal/debug loss count. `diagnostic_reader_drain` returns:

- `BATCH` with an immutable owned batch whenever records or loss counts exist;
- `OPEN_EMPTY` when no data exists and the engine can still produce records;
- `END_OF_STREAM` after engine close or final release seals production and all
  retained data and losses have been drained.

A batch can be loss-only. Its records are indexed in strict increasing
engine-local sequence order. Every record carries a monotonic observation
timestamp, checked operation identity, optional caller activity tag, level,
operation kind, and one closed typed payload. Timestamp proximity is not
causality; sequence is the total commit order.

The record schema is privacy-reviewed and fixed-width. OCR records may carry an
opaque library-issued model-instance ID, accepted G-004 classification, exact
source frame, requested/effective geometry, output space, typed outcome,
result count, elapsed nanoseconds, and source-pixel count. Admission and terminal
records remain independent observations under the caller's opaque activity tag;
they do not claim prior input caused a frame or that later input succeeded.

Records contain no pixels/hashes, recognized text, vocabulary, key/event
payload, window title, platform namespace, backend/runtime name, caller
asset/model identity, model digest/path/bytes, signing identifier, credential,
or native/free-form backend message. Full or contended streams never block OCR;
they preserve exact normal/debug loss counts. Draining remains self-silent.

## Panic containment

Every exported symbol and every table entry contains a Rust panic before it can
cross into C. A contained panic returns `MADOPILOT_STATUS_INTERNAL_PANIC`, leaves
every valid output in its failure state, releases whatever the unwinding call had
allocated, and does not poison unrelated handles. A later call can therefore
run, but repeating a side-effecting call is not necessarily safe: the panic may
have happened after a native input effect. Selected native macOS exceptions are
contained inside the Objective-C shim before control returns to Rust; neither
exception nor panic crosses the C boundary.

Containment requires an unwinding panic profile, and the crate refuses to build
without one. `catch_unwind` catches nothing under an aborting profile: a panic
ends the host process instead of the entry returning
`MADOPILOT_STATUS_INTERNAL_PANIC`, so the library's documented behaviour would be
false while the build succeeded. `mado-pilot-capi` therefore carries a
`#[cfg(panic = "abort")] compile_error!`, and `-C panic=abort` or a profile
`panic = "abort"` fails the build rather than producing that library.

One case is outside what a crate can check: `panic_immediate_abort` is a `std`
feature selected through `-Z build-std`, and a dependent crate's `cfg` cannot see
it. A build that enables it produces a library whose panic containment does not
work, and nothing here will say so. Do not enable it for a build that advertises
this ABI.

## What ABI 1.2 does not contain

The 1.2 table ends at bounded diagnostic batch access. There is no entry for OCR
model loading or recognition, watchers, callbacks, callback unregistration,
acceleration selection, release packaging, or platform-native frame extensions,
and none is reserved as a null slot. A later minor appends only implemented
contracts.

There is also no action, retry, wait-for-effect, or general coordinate-conversion
entry. Caller-supplied map and find regions must already be in capture pixels.
Input pointer events carry their own coordinate space and geometry policy into
`session_send_input`. A caller that wants to establish post-input visual state
acquires a strictly newer frame and searches it as a separate operation; neither
the receipt nor diagnostics synthesize that causal conclusion.

## Building against the library

### Artifacts

`mado-pilot-capi` builds as a `cdylib` plus an ordinary Rust `lib`.

| Target | Built | Reserved release name |
|---|---|---|
| `aarch64-apple-darwin` | `libmadopilot.dylib` | `libmadopilot.1.dylib` |
| `x86_64-pc-windows-msvc` | `madopilot.dll`, `madopilot.dll.lib` | `madopilot-1.dll`, `madopilot-1.lib` |

The ABI-major decorated names in the right column are what a release ships, so
that an incompatible ABI is a different library rather than a silent breakage.
Applying them is a release-packaging step — an install name on macOS, a linked
file name and matching import library on Windows — and is not implemented yet.
What is built today is the undecorated development artifact.

No `staticlib` is produced. Gate [`G-008`](validation-gates.md#g-008) has not
recorded which static dependency combinations are supported, and emitting the
artifact would advertise a claim the project has not made.

### Prerequisites

The library links OpenCV dynamically, so a host that runs a C program against it
needs the same OpenCV a Rust host does; see
[third-party-dependencies.md](third-party-dependencies.md). An absent OpenCV
stops the process at load time, before any MadoPilot code runs, so it is not
reachable as a status — recorded against [`G-007`](validation-gates.md#g-007).

Compiling the examples additionally needs a C and C++ toolchain, and the check
needs **CMake 3.22 or later** for the consumer-project step. All of them are the
release target's own, and neither CI runner nor either verification host
installs anything extra for them:

| Target | Compiler | Flags used by the check |
|---|---|---|
| `aarch64-apple-darwin` | Xcode Command Line Tools `cc`, `c++` | `-std=c11` / `-std=c++17`, `-Wall -Wextra` |
| `x86_64-pc-windows-msvc` | MSVC `cl` | `/std:c11` / `/std:c++17 /EHsc`, `/W3` |

Set `CC`, `CXX`, or `CMAKE` to choose a different one.

**On Windows, run the check from a Developer Command Prompt**, or call
`vcvars64.bat` first. `cl` is not on `PATH` in a plain shell even when Visual
Studio is installed, and it needs `INCLUDE` and `LIB` set to find the C runtime
headers and import libraries. `c-abi-check` says so when it cannot launch the
compiler. The Windows CI job discovers the install path with `vswhere` and calls
`vcvars64.bat` itself, so nothing has to be hard-coded there either. That same
environment sets `VSINSTALLDIR`, through which the check finds the CMake Visual
Studio ships when none is on `PATH`.

One Windows-specific trap is worth knowing because it looks like a missing file:
MSVC cannot open a source or include path in the `\\?\C:\...` extended-length
form that `std::fs::canonicalize` returns, and reports **C1083** as though the
file did not exist. `c-abi-check` strips that prefix from every path it hands to
a compiler.

### Compiling

```sh
cargo build --locked --package mado-pilot-capi
cc -std=c11 -I crates/bindings/capi/include \
   -I crates/bindings/capi/examples \
   -o deterministic-slice \
   crates/bindings/capi/examples/c/deterministic-slice.c \
   target/debug/libmadopilot.dylib -Wl,-rpath,target/debug
./deterministic-slice --package fixtures/assets/phase1-slice
```

On Windows, link `target\debug\madopilot.dll.lib` and put `target\debug` on
`PATH` before running.

The second include directory is for `deterministic-scene.h`, which holds the
deterministic scene both the C and C++ examples build their replay frame from. A
program of your own needs only the first.

CMake targets are available as well, and are what a C++ consumer uses; see
[cpp-wrapper.md](cpp-wrapper.md#building-against-it).

The native C examples have two modes. `--check` creates the real platform
engine, verifies capability reporting, and reads only non-prompting permission
state; it stops before discovery and sends no input. That remains the default on
macOS and for an ordinary local run:

```sh
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

On Windows, `--windows-native-fixture` launches both already-built repository
fixtures, obtains their exact titles, runs the C and C++ common flows once
against the ordinary contract and once against the acknowledged contract, and
terminates both fixtures:

```bat
cargo build --locked --package mado-pilot-platform-windows --bin mado-pilot-windows-input-fixture --bin mado-pilot-windows-window-message-fixture
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>" --windows-native-fixture
```

Each flow covers discovery, capture, mapping, one bounded pointer/keyboard
sequence, immutable receipt and attempt inspection, a strictly newer
visual-condition search, diagnostic drain, and explicit session close. The
ordinary run requires `MADOPILOT_CAPABILITY_UNKNOWN` and
`MADOPILOT_SUBMISSION_EVIDENCE_TARGET_QUEUE_ADMISSION`; the dedicated fixture
requires `MADOPILOT_CAPABILITY_SUPPORTED` and
`MADOPILOT_SUBMISSION_EVIDENCE_TARGET_PROTOCOL_ACKNOWLEDGEMENT`. Both preserve
focus, permit no system-input fallback, and print no title, captured bytes, or
typed text.

When a caller owns the fixture lifecycle, pass `--ordinary "<full title>"` or
`--acknowledged "<full title>"` directly to `windows-native-input.exe`; the C++
counterpart accepts the same flags.

## How the header is verified

The header is hand-written and tracked, not generated; the reasoning is in
[ADR 0004](adr/0004-c-header-authorship-and-abi-verification.md). Its agreement
with the Rust `#[repr(C)]` definitions is proved rather than asserted:

```sh
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
cargo run --locked --package mado-pilot-capi --features private-fixture \
  --example c-abi-check -- --label "<host>"
```

The checker compares every C and Rust size/alignment/offset, compiles and runs
the deterministic/native consumers, and then compiles frozen ABI 1.0 and 1.2
callers only against their own headers. Those callers negotiate 424 and 592
bytes and run unchanged against ABI 1.3. Current C++ checks negotiate complete
1.2 and partial 1.3 extents and refuse OCR/default construction before a missing
function pointer is read.

The ordinary checker also compiles and runs the production C and C++ default OCR
examples with reviewed prerequisites, requires identical normalized
backend/model/count output, and runs the independent CMake consumer. The
`private-fixture` mode separately runs deterministic C/C++ OCR against the
fixture-only constructor and validates content-redacted diagnostics. Both paths
exercise ownership, repeated close, and immutable result independence without
turning the fixture symbol into a release entry.

The same command continues into the C++ surface: compile-time and runtime
ownership tests, replay/native/default examples, and the independent CMake
consumer project. See
[cpp-wrapper.md](cpp-wrapper.md#how-the-wrapper-is-verified).

The invariants that need no C compiler — versioned prefixes, per-event required
fields, fixed numeric values, thin handles, table order, invalid-input failure
states, handle lifetimes, concurrent reads and input serialization, diagnostic
ordering/loss/privacy/lifetime, panic containment, and the absence of deferred
surface — run under `cargo test`.

Both native CI jobs run the complete boundary check on every pull request. The
Ubuntu repository-policy job deliberately compiles no product package.

## What the freezes recorded

[ADR 0007](adr/0007-phase-1-c-abi-freeze.md) records the complete ABI 1.0
prefix: numeric values, the forty-byte mandatory table prefix, every structure's
layout and required prefix, output-state and ownership rules, and the
Rust-error-to-C-status mapping. Its per-field reports are under
[evidence/c-abi/](evidence/c-abi/).

[ADR 0017](adr/0017-c-abi-1-1-native-input-prefix.md) records the
superseded, unreleased ABI 1.1 draft. ADR 0023 removed its declarations and
executable caller from the current tree; repository history retains the
development record without turning it into a compatibility target.

[ADR 0023](adr/0023-input-submission-observation-and-abi-1-2.md) records ABI 1.2:
the native route capability and submission-evidence vocabulary, owned receipt
and attempt access, operation activity tags, bounded diagnostics, the complete
592-byte table, and the deliberate rejection of minimum minor 1.

[ADR 0035](adr/0035-ocr-public-surfaces-and-private-fixture-boundary.md) records
ABI 1.3's six-entry OCR suffix, immutable result/view ownership, the 640-byte
owner/accessor extent, redacted diagnostic append, and isolation of the local
fixture constructor. [ADR 0036](adr/0036-default-ocr-composition-and-abi-prefix.md)
records the standalone default options, preserved engine-options layout, and
`engine_create_with_default_ocr` at offset 640 completing the table at 648 bytes.

Each released header gets an immutable fixture. New coverage goes in the next
fixture rather than editing an old caller; the rule is in
[`tests/abi-compat/README.md`](../crates/bindings/capi/tests/abi-compat/README.md).
