# Windows Ordinary Background Input Qualification

## Revision binding and preconditions

- Change: `phase-2-1-windows-background-input-qualification`
- Active version branch: `dev/0.2.1`
- Topic branch: `test/phase-2-1-windows-background-input-qualification`
- Source commit before probe work: `bf135ee9c62239d557ca4a8144a42cab4f8e5259`
- Source tree before probe work: `c1cb94978f8c841682641951c68b2aabbb9683ad`
- Source commit date: `2026-08-09T19:35:19+09:00`
- Prerequisite PR: [#30](https://github.com/pashifika/mado-pilot/pull/30), merged into `dev/0.2.0` at `bf135ee9c62239d557ca4a8144a42cab4f8e5259` on `2026-08-09T10:35:19Z`
- PR #30 required checks: `Validate branch flow`, `Repository policy`, `Windows x86_64-pc-windows-msvc`, and `macOS aarch64-apple-darwin` all passed before merge.
- Native evidence harness prerequisite: the PR #30 merge commit is the current `dev/0.2.1` source commit, so the harness is present in the qualification base.

The source revision above is the unmodified qualification baseline. The probe revision, executable hashes, fixture identities, host record, commands, bounded outputs, raw-log hashes, and final decision are recorded below as the gate advances.

## Frozen qualification gate

The matrix below was frozen before fixture or probe implementation on
`2026-08-09`. `UNRUN` is a result, not a passing skip. Changing a row, oracle, or
acceptance rule after observing probe output requires a new probe revision and an
applicability note covering every earlier result.

### Decision rules

1. Every global gate `G-01` through `G-09` and every mandatory authority,
   lifecycle, integrity, safety, and host row must execute and pass for a go
   decision. A failed or unexecuted global row mechanically produces no-go.
2. Only after all global gates pass may an operation/delivery/consumer pair be
   accepted independently. Failure of one pair never inherits success from
   another operation, delivery model, window relationship, or consumer.
3. `PostMessageW` success is only queue admission.
   `SendMessageTimeoutW` success is only bounded dispatch return. Fixture
   observation is separate, and neither is ordinary-application consumption.
4. The selected exact `HWND` is the authority. Class, title, geometry, PID,
   thread, or an earlier match never authorizes retargeting.
5. No row may activate the target, move the real cursor, call `SendInput`, attach
   input queues, elevate, change a message filter, install a hook, inject code,
   broadcast, post a thread message, or fall back to another delivery model.
6. A safety stop settles the current row truthfully, records possible current
   message effect when applicable, and prevents every later ordinary-input row.
7. Production descriptors remain system-only throughout this Change. A go result
   authorizes only a separate `input-control` proposal.

### Result vocabulary

| Token | Meaning |
|---|---|
| `NO_CALL` | Preflight refused before an irreversible Windows message call |
| `QUEUE_ACCEPTED` | `PostMessageW` admitted one message to the destination queue |
| `QUEUE_REFUSED` | `PostMessageW` returned failure and its supported error was recorded |
| `DISPATCH_RETURNED` | `SendMessageTimeoutW` returned within its bound |
| `DISPATCH_TIMED_OUT` | Bounded synchronous dispatch timed out; current-message effect remains possible |
| `DISPATCH_REFUSED` | Synchronous dispatch was refused before target observation |
| `OBSERVED_EXACT` | The selected owned fixture window observed the classified message |
| `OBSERVED_WRONG_WINDOW` | A sibling, unintended child, replacement, or other owned target observed it |
| `OBSERVED_FOREGROUND` | The unrelated foreground fixture observed target-directed input |
| `OBSERVATION_TIMEOUT` | No owned oracle observed the message before the row bound |
| `UNRUN` | A precondition or approved host state was unavailable; never a pass |

### Owned fixture identities

| ID | Process and purpose | Required behavior |
|---|---|---|
| `F-ORDINARY` | Qualification-only ordinary Win32 target process | Top-level, sibling, and child windows; duplicate metadata; replacement and reparent controls; approved fill states; bounded per-window legacy-message summaries with text reduced to UTF-16 unit counts |
| `F-GAME` | Qualification-only game-like system-framework process | Selectable legacy-message, Raw Input, and asynchronous key/mouse-state-polling consumers; no DirectInput, XInput, raw-HID, anti-cheat, hook, or injected-helper claim |
| `F-FOREGROUND` | Qualification-only unrelated foreground process | Remains foreground and records bounded legacy, raw, and polling counters; any target-directed observation is a global safety failure |
| `F-ACK` | Existing `MadoPilotInputFixture` | Positive control only: class-qualified, versioned `WM_COPYDATA` and explicit acknowledgement; never evidence for an ordinary target |

Fixture executable hashes, process revisions, class identities, and bounded
startup records are filled in when built. Raw handles, PIDs, titles, and text are
not retained.

### Approved host and topology rows

The only approved physical host is the repository's Windows 11 workstation:
Core i7-12700KF, 32 GiB RAM, and RTX 4080. Each run must additionally record the
exact serviced Windows build, desktop SKU, architecture, non-sensitive GPU driver
version, keyboard layout identifier, caller/target integrity relationship, and
redacted display geometry. GitHub-hosted Windows Server is not this host.

| ID | Host state | Mandatory outcome |
|---|---|---|
| `H-01` | Approved host, ordinary single-display topology | Execute all topology-independent global and operation rows |
| `H-02` | Approved host, same-DPI multi-display topology when attached | Execute client/screen conversion and unchanged-cursor rows; otherwise `UNRUN` and global no-go |
| `H-03` | Approved host, deliberately mixed-DPI displays | Execute per-monitor pointer rows; otherwise `UNRUN` and global no-go |
| `H-04` | Approved host, secondary display at a signed virtual-desktop origin | Execute signed-origin pointer rows; otherwise `UNRUN` and global no-go |
| `H-05` | GitHub-hosted Windows Server CI | Compile and deterministic-contract evidence only; never satisfies `H-01` through `H-04`, interactive foreground, UIPI, or physical-display rows |

### Global gates

| ID | Gate | Pass rule | Initial status |
|---|---|---|---|
| `G-01` | Documented and isolated mechanism | Only exact-`HWND` `PostMessageW` and bounded `SendMessageTimeoutW`; no forbidden mechanism or production link/export | `UNRUN` |
| `G-02` | Exact target authority | Every call retains owner lifetime and revalidates exact window, process, thread, root relationship, class/current metadata, integrity, deadline, and cancellation; replacement/reuse ambiguity makes zero calls | `UNRUN` |
| `G-03` | Foreground preservation | `F-FOREGROUND` remains foreground, target remains inactive, real cursor is unchanged, and foreground/sibling/unintended-child counters stay zero | `UNRUN` |
| `G-04` | Truthful outcomes and receipts | Queue, dispatch, exact observation, wrong-window observation, foreground observation, timeout, partial effect, and cleanup remain distinguishable without calling a generic result consumption | `UNRUN` |
| `G-05` | Integrity and UIPI | Equal-integrity rows behave as classified; higher-integrity/UIPI rows fail closed without fallback, elevation, or filter change | `UNRUN` |
| `G-06` | Finite lifecycle | One outstanding input message per row; bounded queue, dispatch, observation, cancellation, teardown, partial state, and release-only cleanup; late observations cannot change settlement | `UNRUN` |
| `G-07` | Stable public eligibility | One non-fixture predicate is evaluable at descriptor time and immediately before every call without private application knowledge or class/title/geometry whitelisting | `UNRUN` |
| `G-08` | Complete revision-bound matrix | Every mandatory row is executed on the approved source, probe, fixture, host, integrity, layout, and display state | `UNRUN` |
| `G-09` | Disposable and private evidence | Probe code is absent from final production packages and retained output contains only approved bounded metadata | `UNRUN` |

### Exact-authority, integrity, and lifecycle rows

Every ambiguity/refusal row must report zero irreversible calls. Positive rows
must target only the selected window and still satisfy `G-03`.

| ID | Condition | Frozen expected classification |
|---|---|---|
| `A-01` | One live inactive top-level target | Exact target may advance to its named delivery model |
| `A-02` | Zero candidate windows | `NO_CALL` |
| `A-03` | Multiple candidates | `NO_CALL` |
| `A-04` | Selected sibling beside same-process sibling | Only selected sibling may observe |
| `A-05` | Explicitly selected child under retained root | Only selected child may observe |
| `A-06` | Duplicate class, title, and geometry | Retained identity selects one; metadata-only lookup is `NO_CALL` |
| `A-07` | Selected child reparented after discovery | `NO_CALL` |
| `A-08` | Selected window destroyed before call | `NO_CALL` |
| `A-09` | Same-process replacement with duplicate metadata | `NO_CALL`; replacement receives zero messages |
| `A-10` | Owner exits and restarts | `NO_CALL`; successor receives zero messages |
| `A-11` | Stale retained owner identity or creation time | `NO_CALL` |
| `A-12` | Bounded destroy/recreate handle-reuse stress | Any unclosed authority interval fails `G-02`; reused successor receives zero messages |
| `A-13` | Cancellation observable before call | `NO_CALL` with cancellation |
| `A-14` | Absolute deadline expired before call | `NO_CALL` with deadline |
| `A-15` | Equal-integrity caller and target | Named delivery model may run after all other checks |
| `A-16` | Higher-integrity target or UIPI refusal | Refusal without fallback; no unsupported optimistic success |
| `A-17` | Destination queue quota condition | One qualified post is classified `QUEUE_REFUSED`; no retry or flood |
| `A-18` | Hung synchronous target | Bounded `DISPATCH_TIMED_OUT`; possible current effect retained; no retry |
| `A-19` | Consumer incompatible with selected operation | `NO_CALL` when incompatibility is knowable; otherwise admission and non-observation remain separate |

### Operation, delivery, and consumer matrix

`POST` and `SEND` are mandatory independent rows in every non-`N/A` cell. A
legacy-message observation in `F-GAME` does not authorize Raw Input or polling.
Raw Input rows must never fabricate `WM_INPUT` data or handles. Polling rows
observe actual asynchronous key/button state and therefore are expected not to
change from legacy posting; the observation, not that expectation, is retained.

| Operation row | Ordinary top-level | Selected sibling | Selected child | Game legacy | Game Raw Input | Game state polling |
|---|---|---|---|---|---|---|
| `K-01` one key-down message | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` classify | `POST`, `SEND` classify |
| `K-02` balanced key down/up | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` classify | `POST`, `SEND` classify |
| `K-03` modifier sequence | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` classify | `POST`, `SEND` classify |
| `T-01` direct character message | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `N/A`: no fabricated raw text | `N/A`: no physical-state text model |
| `P-01` client pointer move | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` classify | `POST`, `SEND` classify |
| `P-02` balanced button press/release | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` classify | `POST`, `SEND` classify |
| `P-03` vertical and horizontal wheel | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` classify | `POST`, `SEND` classify |
| `P-04` press/move/release drag | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` | `POST`, `SEND` classify | `POST`, `SEND` classify |

Keyboard classifications retain repeat count, scan code, extended, previous-state,
and transition flags, plus whether the consumer depends on layout, accelerator,
focus, `GetKeyState`, or `GetAsyncKeyState`. Text retains only UTF-16 unit count
and makes no layout-composition, dead-key, IME, keyboard, or Raw Input claim.
Pointer classifications retain only approved client/screen coordinates and
dependencies on cursor, hit test, button state, capture, DPI, and foreground.

### Protocol and terminal-race rows

| ID | Condition | Frozen pass rule |
|---|---|---|
| `R-01` | Malformed or truncated fixture record | Refused without state mutation |
| `R-02` | Oversized fixture record | Refused before allocation beyond the documented bound |
| `R-03` | Duplicate command identifier | Refused or replayed idempotently; never executed twice |
| `R-04` | Out-of-order command identifier | Refused without changing the expected identifier |
| `R-05` | Bounded observation queue overflow | Explicit overflow count; no unbounded growth |
| `R-06` | Fixture child exits | Bounded target-loss settlement and worker join |
| `R-07` | Cancellation/deadline before each irreversible call | `NO_CALL` |
| `R-08` | Cancellation/deadline after call before commit | Immutable terminal result with possible current effect recorded |
| `R-09` | Late observation after settlement | Diagnostic only; no result mutation or second message |
| `R-10` | Target closes after observed press | Exact partial count and bounded release-only cleanup; no metadata retarget |
| `R-11` | Worker teardown and repeated cleanup | Idempotent, bounded, and no outstanding worker or owned pressed state |

### Safety stop conditions

Stop all later ordinary-input rows immediately after any of the following:

- the selected target becomes foreground or `F-FOREGROUND` loses foreground;
- the foreground, sibling, unintended child, replacement, or restarted owner
  observes a target-directed message;
- the real cursor moves;
- exact authority, integrity, owner lifetime, deadline, or cancellation cannot be
  revalidated immediately before a call;
- one row would require retry, fallback, elevation, message-filter change, queue
  attachment, hook, injection, broadcast, thread messages, or system input;
- a bounded queue, timeout, memory, teardown, cleanup, or privacy rule fails.

### Privacy fields and CI boundary

Tracked evidence may retain commit/tree identities, probe and executable hashes,
generic fixture identities, Windows/SKU/build, architecture, toolchain/SDK,
generic CPU/RAM/GPU/driver, keyboard layout identifier, integrity relationship,
redacted display count/resolution/scale/origin, row identifiers, operation kinds,
UTF-16 unit counts, bounded counters, durations, status classes, command lines,
and raw-log hashes.

Tracked evidence must not retain characters, captured pixels or pixel hashes,
raw handles, PIDs, window titles, process paths or inventories, user/profile
paths, credentials, tokens, unrelated desktop metadata, or unbounded logs.
Detailed raw output belongs only in the Change ephemera directory.

Hosted CI may compile the qualification target and run deterministic construction,
classifier, protocol, authority-fake, timeout-fake, cancellation, settlement, and
package-isolation checks. It cannot satisfy approved-host identity, real inactive
foreground preservation, higher-integrity/UIPI behavior, physical cursor
preservation, real queue/dispatch observation, interactive target loss, mixed-DPI
placement, signed display origin, or the final executable/host binding. Those
rows remain `UNRUN` until executed on the approved physical host and cause no-go
if still unexecuted at decision time.

## Decision destination and blocked production phase

The next available focused decision number is reserved as
`docs/adr/0022-windows-ordinary-background-input-qualification.md`. That ADR is
the sole destination for the final tested authority, delivery, integrity,
receipt, consumer, lifecycle, and go/no-go conclusions.

A later production Change that modifies `input-control` remains blocked. It has
no implementation authorization from this qualification branch. An incomplete,
failed, or partially unexecuted global matrix blocks both that production Change
and every ordinary-window background-input release claim. Only a complete go
decision may open a separately reviewed production proposal; a no-go decision
closes the phase while preserving system-only ordinary descriptors and the
existing acknowledged fixture capability.

## Baseline production and documented contracts

### Current production capability boundary

The source baseline was checked directly against
`crates/platform/windows/src/input.rs`, `native_input.rs`, and
`fixture_protocol.rs`.

| Discovered target | Current descriptor |
|---|---|
| Ordinary top-level window | Pointer, keyboard, and text through `System`; focus required; no `BackgroundTarget` pair |
| Class `MadoPilotInputFixture` | The same focus-required `System` pairs plus pointer, keyboard, and text through fixture-only `BackgroundTarget` |
| Display | Pointer through `System` only; no keyboard, text, or background pair |

No qualification result changes this table. The class check is exact and the
ordinary-message probe is not reachable from `input_capability`,
`NativeInputDriver`, the facade, ABI, C++ wrapper, or a released example.

### Existing acknowledged fixture and receipt behavior

The dedicated fixture uses protocol version 1 and a synchronous `WM_COPYDATA`
packet tagged `0x4d50_4946`. The sender checks operation interruption, target
liveness, exact fixture class, target kind, packet size, remaining deadline, and
integrity before `SendMessageTimeoutW`. It uses
`SMTO_ABORTIFHUNG | SMTO_BLOCK` and caps each call at the lesser of the remaining
operation deadline and 100 ms.

The receiver validates the tag, non-null payload, exact bounded packet shape,
version, scalar fields, key/button codes, UTF-16, and text size. It returns the
fixture-specific acknowledgement `0x4d50_414b` only after accepting the event.
The code retains a rolling maximum of 1,024 summaries containing only event kind
and UTF-16 unit count. The previous verification text said 256; the source
constant and receiver both enforce 1,024, so that documentation is corrected by
this Change rather than treating the stale prose as evidence.

`InputReceipt.delivered` counts complete logical events. A fixture call that
returns the expected acknowledgement increments that count. A call that may have
entered dispatch but does not return the acknowledgement is a partial receipt
even when zero logical events completed. A failure proven before dispatch is
unexecuted. No fallback runs after a possible effect. Sequence-owned pressed
state receives only bounded newest-first release cleanup, and the receipt records
released/owed cleanup separately.

This acknowledgement is an application-private protocol. A generic legacy
message `LRESULT` has no equivalent meaning and cannot be mapped to fixture
consumption.

### Documented ordinary-message mechanisms

- [`PostMessageW`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-postmessagew)
  places a message in the queue of the thread that created one non-null,
  non-broadcast `HWND` and returns before processing. Nonzero means queue
  admission only. Zero requires immediate `GetLastError`; UIPI reports
  `ERROR_ACCESS_DENIED`, and the documented 10,000-message queue limit reports
  `ERROR_NOT_ENOUGH_QUOTA`. No qualification parameter contains a pointer.
- [`SendMessageTimeoutW`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw)
  invokes the selected window procedure and, across threads, waits for return or
  the bound. Qualification uses `SMTO_ABORTIFHUNG | SMTO_BLOCK |
  SMTO_ERRORONEXIT`, one exact `HWND`, and a cleared last-error value. Nonzero
  means dispatch returned; the output is only that message's `LRESULT`. Zero can
  mean timeout or failure, and a still-zero last error is only generic failure.
  Same-queue direct dispatch is forbidden by the separate-process fixture model
  because the documented timeout would otherwise be ignored.
- Both APIs marshal only system messages below `WM_USER`. The qualification
  input rows use system-defined legacy messages with scalar parameters; the
  fixture's private control protocol performs its own bounded marshalling and is
  never confused with an ordinary input row.
- Starting with Windows Vista, message posting is subject to UIPI: a process may
  post only to queues at lesser or equal integrity. Qualification never elevates,
  changes a target's message filter, or converts an access refusal into another
  delivery attempt.
- [Raw Input](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input)
  is a distinct registered-device model. The consumer registers a top-level
  collection and receives `WM_INPUT` carrying an operating-system `HRAWINPUT`
  record; `RIDEV_INPUTSINK` is the consumer's background choice. A posted legacy
  keyboard or pointer message does not create that record and the probe never
  fabricates one.

### Documented operation fields and coordinates

- [`WM_KEYDOWN`](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-keydown)
  carries the virtual key in `wParam`; `lParam` contains repeat count in bits
  0–15, scan code in 16–23, extended-key in 24, context in 29, previous-state in
  30, and transition in 31. Down, repeat, and up rows construct and classify
  those fields independently. Posted messages are not assumed to change
  `GetKeyState` or `GetAsyncKeyState`.
- [`WM_CHAR`](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-char)
  is normally produced when `TranslateMessage` translates a key message, has no
  one-to-one key relationship, and carries UTF-16 code units for a Unicode
  window. Direct text posting is therefore classified separately and makes no
  keyboard-layout composition, dead-key, IME, key-state, or Raw Input claim.
- [`WM_MOUSEMOVE`](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-mousemove)
  and button messages such as
  [`WM_LBUTTONDOWN`](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-lbuttondown)
  use signed client-relative coordinates in `lParam` and key/button-state bits
  in `wParam`. Real delivery normally depends on cursor hit testing or capture;
  posting the message does not establish either.
- [`WM_MOUSEWHEEL`](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-mousewheel)
  uses signed screen coordinates in `lParam`, key/button state in the low word of
  `wParam`, and signed wheel delta in the high word. The fixture records the
  exact supplied convention and never moves the real cursor.

These documented contracts are the probe oracle. A fixture observation can show
that one owned message loop handled the supplied fields; it cannot strengthen
the operating-system return value or generalize to an uninstrumented
application.


## Executed probe revision

The approved-host run completed at `2026-08-09T20:50:20+09:00` against base
commit `bf135ee9c62239d557ca4a8144a42cab4f8e5259`. The probe was not committed as
production code. This source bundle and the built executable bind the result:

| Artifact | SHA-256 |
|---|---|
| `main.rs` | `efa7fb1e1bd2ff2e0ccdabda447b831eda700e0daf07b07f7591b8e9323d814d` |
| `model.rs` | `570d6735063f0060d4de36880c16cb30b37015ac2a4be914194770f46a18eb2f` |
| `native.rs` | `9002c7855f1a35cb2b324c9695e267c7f0544c5fe59044312f52c02554bdc995` |
| Qualification executable | `368a29e65a72f42e53182e2675275d197f04f9d0b54c828bcfcc8ddcb25c7559` |
| Untracked raw run in Change ephemera | `623008a536eed6cd8d69cf73166e9c2597e1c339abea7a635ba9cbec43db9be6` |

The executable was built and run with:

```text
cargo run --quiet --locked -p mado-pilot-platform-windows \
  --bin mado-pilot-windows-background-input-qualification -- \
  run --output .rasen/changes/phase-2-1-windows-background-input-qualification/ephemera/qualification-raw.log
```

`rustc 1.97.1 (8bab26f4f 2026-07-14)`, host
`x86_64-pc-windows-msvc`, and LLVM 22.1.6 compiled the revision. The probe was a
qualification-only binary target. It was not imported by the Windows library,
input provider, facade, C ABI, C++ wrapper, example, or acknowledged fixture
protocol.

## Approved-host record

The physical workstation matched the approved Core i7-12700KF, 32 GiB RAM, and
NVIDIA GeForce RTX 4080 identity. It ran Windows 11 Pro, edition identifier
`Professional`, display version `25H2`, build `26200.8973`, and NVIDIA driver
`32.0.15.9186`. The target architecture was `x86_64`. The fixture thread
reported keyboard-layout identifier `0x4110411`. Caller and target both ran at
integrity value `8192` (medium integrity).

Two 3840 by 2160 displays were attached, both at DPI 144. One display had a
signed virtual-desktop origin. Consequently `H-02` and `H-04` passed. `H-01`
was `UNRUN` because the host was not in a single-display topology, `H-03` was
`UNRUN` because the attached displays were not deliberately mixed-DPI, and
`H-05` remained `UNRUN` because this was the physical-host run rather than
hosted CI.

The unrelated `F-FOREGROUND` process owned the foreground authority before any
qualification input row. Windows foreground policy was intermittent under the
non-interactive runner, so setup granted that owned child foreground permission
and, when needed, used a bounded, balanced temporary input-queue attachment.
This setup named only `F-FOREGROUND`, detached before the first row, and never
activated an input target. No row attached queues, moved the real cursor,
called `SendInput`, elevated, changed a message filter, installed a hook,
injected code, broadcast a message, or posted a thread message.

## Executed results

### Global gates

| Gate | Result | Evidence |
|---|---|---|
| `G-01` | `PASS` | Only scalar system messages to one retained exact `HWND` used `PostMessageW` or bounded `SendMessageTimeoutW`; the probe had no production link. |
| `G-02` | `UNRUN` | Destroy/recreate stress completed 4,096 bounded attempts without observing actual `HWND` reuse. All observed replacement, owner-exit, and relationship changes made zero calls, but the mandatory reuse case was not executed. |
| `G-03` | `PASS` | Every completed row retained the owned foreground authority, an inactive target, unchanged real cursor, zero unrelated foreground observations, and zero wrong-window observations. |
| `G-04` | `FAIL` | Queue admission and generic dispatch return cannot prove ordinary-application consumption using the existing receipt contract. A private fixture oracle is required to distinguish observation. |
| `G-05` | `UNRUN` | Equal-integrity behavior ran. No owned higher-integrity target was available, and the probe correctly did not elevate or change a message filter. |
| `G-06` | `PASS` | Queue fill, dispatch, observation, cancellation, worker join, target loss, cleanup, and late-effect handling were bounded; terminal settlement remained immutable. |
| `G-07` | `FAIL` | No stable public predicate can identify an ordinary non-fixture consumer compatible with a legacy keyboard, text, or pointer message at descriptor and immediate pre-call time. |
| `G-08` | `FAIL` | Mandatory `G-02`, `G-05`, `H-01`, `H-03`, and hosted-CI rows remained unexecuted in this revision, in addition to the substantive `G-04` and `G-07` failures. |
| `G-09` | `PASS` | Post-decision cleanup deleted the qualification target. Package metadata exposes only the Windows library, the existing input fixture, and the three existing integration-test targets; linked-string inspection retains `MadoPilotInputFixture`/`fixture_protocol.rs` and finds no qualification identifier. |

The frozen rule therefore produces `NO_GO`; no discretionary weighting or
partial pair success can override it.

### Authority, refusal, and lifecycle

- `A-02`, `A-03`, `A-07` through `A-11`, `A-13`, `A-14`, and `A-19`
  refused before an input call under their named zero/multiple-target,
  relationship, destroyed-window, replacement, owner-exit, stale-owner,
  cancellation, deadline, and known-incompatible-consumer conditions.
- `A-04` and `A-05` delivered only to the selected sibling or child. Every
  parent, sibling, child, replacement, and restarted-owner counter required to
  remain zero did so.
- `A-06` retained one exact identity while a duplicate metadata lookup was
  ambiguous and made no call.
- `A-12` did not observe actual handle reuse in 4,096 attempts. The public
  retained authority and private fixture generation both refused the destroyed
  target; the replacement counters remained zero. The row stays `UNRUN` rather
  than converting absence of reuse into a pass.
- `A-17` filled the destination queue with 10,000 admitted `WM_NULL` messages.
  The next qualified post was `QUEUE_REFUSED` with
  `ERROR_NOT_ENOUGH_QUOTA` (`1816`), with no retry, target effect, foreground
  effect, or cursor movement.
- `A-18` returned `DISPATCH_TIMED_OUT` with `ERROR_TIMEOUT` (`1460`) and
  `possible_current_effect=true`. The fixture observed the message after the
  timeout; that late observation remained diagnostic and did not mutate the
  committed terminal result or trigger a retry.
- `R-01` through `R-05` refused malformed, oversized, duplicate, and
  out-of-order control records and retained only 256 observations plus an
  explicit overflow count. `R-06` joined the exited child. `R-07` made zero
  calls after cancellation or deadline expiry. `R-08` and `R-09` preserved one
  immutable timed-out settlement. `R-10` recorded one observed press, then
  target loss and one cleanup item still owed without retargeting. `R-11`
  joined teardown with no outstanding worker and bounded repeated cleanup.

### Operation and consumer observations

The operation matrix and physical-display subset made 212 classified calls:
106 admitted posts and 106 bounded dispatch returns. Of those, 148 were exact
legacy observations and 64 were observation timeouts. All 212 reported matching
message fields, no wrong-window observation, no foreground observation, and an
unchanged real cursor. The two raw/polling text cells remained explicitly
not-applicable.

All ordinary top-level, selected-sibling, selected-child, game-legacy, and
display-subset calls were observed exactly by the intended legacy-message
oracle. For game Raw Input and state-polling modes, all 64 legacy-message calls
entered the target window procedure but produced zero Raw Input or asynchronous
polling-consumer change before the observation bound. That result rejects the
assumption that legacy message delivery is interchangeable with either
consumer model. The fixture intentionally did not claim DirectInput, XInput,
raw HID, anti-cheat, or any real third-party game.

Pointer rows preserved the supplied signed coordinate convention across both
same-DPI displays, including the display at a signed virtual origin. Keyboard
rows matched message identifier, repeat, scan code, extended, previous-state,
and transition fields. Text rows retained and matched only a UTF-16 unit count;
no character, composition, IME, or dead-key content was retained.

## Receipt and eligibility conclusion

The observations disprove the design assumption that successful ordinary
window-message transport can be represented as generic delivered input.
`PostMessageW` nonzero established queue admission only. `SendMessageTimeoutW`
nonzero established bounded window-procedure return only. Its `LRESULT` was not
an application acknowledgement. The hung row additionally demonstrated that a
timeout can precede a real current-message effect.

The probe could distinguish those states only because it owned a private,
instrumented consumer and queried its bounded counters. An arbitrary ordinary
window exposes no equivalent acknowledgement. The current
`InputReceipt.delivered` field counts complete logical events and therefore
cannot truthfully encode a generic admitted post or unacknowledged dispatch as
delivered. Reusing `partial` would preserve possible effect after a timeout but
would still not establish application consumption.

Likewise, the probe's private `Legacy`, `RawInput`, and `Polling` modes allowed
pre-call compatibility classification only because the fixture disclosed its
implementation. Production discovery has no public non-fixture descriptor
predicate with equivalent knowledge. Class, title, geometry, PID, or observed
message-loop transport cannot supply that predicate.

Adding richer admission and observation fields to the receipt would not repair
the missing public eligibility predicate, and whitelisting applications would
violate the frozen gate. This is a capability-boundary failure, not a transport
implementation bug.

## Decision

Decision: `NO_GO`.

Ordinary Windows targets remain system-only. The acknowledged
`MadoPilotInputFixture` background capability remains unchanged. No
`input-control` production proposal, alias, feature flag, compatibility shim,
optimistic capability, or ordinary-message fallback is authorized.

### Post-decision isolation verification

The three qualification-only source files were deleted. `cargo metadata
--locked --no-deps --format-version 1` now reports only
`mado_pilot_platform_windows`, `mado-pilot-windows-input-fixture`,
`loader_imports`, `native_capture`, and `native_input` as package targets.
Linked-string inspection of the rebuilt fixture executable found
`MadoPilotInputFixture` and `fixture_protocol.rs`, and found none of
`MadoPilotQualification`, `background-input-qualification`, or
`background_input_qualification`.

The focused
`input::tests::target_classes_advertise_only_the_verified_delivery_matrix`
regression passed after cleanup. It confirms that every ordinary-window
operation remains `System`-only, every fixture operation retains its existing
`System` and acknowledged `BackgroundTarget` pairs, and displays remain
pointer-only through `System`. This later `G-09` pass cannot reverse the no-go:
`G-04`, `G-07`, and `G-08` still fail.

Focused post-cleanup regressions passed:

- the Windows package reported 75 library tests, the optional-import test, the
  native capture test, and the dedicated background fixture test passing; the
  one user-focused system-injection test remained intentionally ignored;
- `mado-pilot-input` reported 68 passing contract tests;
- `mado-pilot-capi --all-targets` reported 134 passing Rust, facade, header,
  layout, prefix, partial-receipt, and panic-containment tests;
- the native MSVC C/C++ check agreed on all 322 current layout lines, exercised
  the acknowledged fixture through both languages, compiled and ran both frozen
  headers, and built and ran all CMake consumers.

The first native CMake rerun exposed a verifier defect rather than a product
failure: Rust passed Windows paths with backslashes in untyped CMake definitions,
and supported CMake 3.29.5 rejected the resulting `\W` escape. The verifier now
passes CMake-style forward-slash paths; the complete native check then passed.

## Final verification record

The retained production source revision remains
`bf135ee9c62239d557ca4a8144a42cab4f8e5259`: the disposable probe never linked
from or modified a production module. The post-decision C/C++ verifier repair is
bound separately as
`crates/bindings/capi/examples/c-abi-check.rs` SHA-256
`3e995d48b9164369dc121eab5cafce4732fd149aa49174639294104a0f5e8822`.
The measured probe-source and executable hashes remain in the executed-revision
table above even though those disposable files no longer exist.

| Command | Complete observed outcome |
|---|---|
| `cargo fmt --all --check` | Passed with no formatter output. |
| `cargo run --locked --package mado-pilot-dependency-check` | Passed: 16 workspace packages and 38 internal dependency edges; workspace metadata remained version 0.2.0, edition 2024, Rust 1.97.1, Apache-2.0. |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Passed with no warning. |
| `cargo test --locked --workspace --all-targets` | Passed: 986 tests across 63 suites; two explicit interactive tests ignored; zero failures. |
| `cargo test --locked --workspace --doc` | Passed: eight doctests across 16 suites; one documented ignore; zero failures. |
| `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps` | Passed: the facade index and 17 other workspace documentation outputs were generated with no warning. |
| `cargo deny --locked check` | Advisories, bans, licenses, and sources passed. The accepted duplicate-version warning names `syn` 2.0.119 through `windows` and 3.0.3 through `serde`; this Change added neither dependency. |
| `cargo package --locked -p mado-pilot-platform-windows --list --allow-dirty` | Passed: 19 package paths; the retained fixture binary and protocol are present and no qualification path is present. The package is non-publishable, so no release archive was produced. |
| Native MSVC `cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label local-windows-x86_64 --windows-native-fixture` after both prerequisite builds | Passed with MSVC 19.44.35228, CMake 3.29.5, and Rust 1.97.1: 322 current layout lines agreed; C and C++ each completed acknowledged native fixture input; both frozen headers compiled, linked, negotiated, and ran; every CMake consumer built and ran. |
| `cargo run --locked --release --package mado-pilot-backend-opencv --example match-report -- --label local-windows-x86_64` | Passed against OpenCV 4.14.0 and completed all seven report fixtures. Scores remained diagnostic and were not used as this qualification's acceptance rule. |
| `rasen validate phase-2-1-windows-background-input-qualification --strict --json --no-interactive` | Passed: one change validated, zero issues, zero failures. The validated scope contains the no-go qualification only and no unimplemented production capability. |

Package metadata and linked-string inspection additionally reported exactly the
retained Windows library, fixture, and three integration-test targets; the
rebuilt fixture contained its acknowledged class/protocol identifiers and no
qualification identifier. This is the final probe-removal proof for `G-09`.

Not run on this host:

- macOS compile, native, AddressSanitizer, input, C ABI, C++, and package checks
  require the qualified Apple Silicon release host and were not represented as
  Windows evidence;
- the Windows interactive system-input test remained ignored because it
  requires a user to focus the exact fixture and sends real system input;
- the post-decision ordinary-message probe cannot be rerun after its required
  deletion. Its higher-integrity/UIPI, actual handle-reuse, single-display,
  mixed-DPI, and hosted-CI rows remain explicitly `UNRUN`; the measured physical
  host supplied only the recorded dual-4K same-DPI and signed-origin rows;
- hosted required checks remain pending until this branch is pushed to a pull
  request. No local result is labeled as hosted-CI evidence.

The result needs no product performance budget: it was decided by authority,
receipt semantics, public eligibility, and mandatory unexecuted rows rather
than latency or throughput. The tracked summary contains no raw handles, PIDs,
titles, process paths, characters, pixels, user/profile paths, credentials,
tokens, or unrelated desktop metadata. Detailed per-call output remains only
in ignored Change ephemera and is bound above by hash.