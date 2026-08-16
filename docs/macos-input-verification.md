# macOS Input Adapter and Verification

The macOS platform package implements input at the Adapter boundary, and
`mado_pilot::macos_engine` wires it into the runtime and public Rust facade. The
C ABI and header-only C++ wrapper consume that same facade-owned engine. Release
acceptance consists of the explicit native checks below; ordinary test runs never
send desktop input or open the opt-in fixture window.

Current release status: **blocked**. The corrected source passes route-wide,
mixed-scale, performance, sanitizer, repository, and CI checks, but its exact
single-display and same-scale renderer rows are unexecuted. Therefore none of
the fourteen process-directed pairs is currently release-qualified. The
historical complete matrix remains provenance only.

## Capability boundary

Capabilities are operation-and-route pairs. macOS publishes two routes with
different address scopes: `System` posts `CGEvent` at the HID tap and reaches
whatever is focused, and `ProcessDirected` posts `CGEventPostToPid` to the
process that owns the retained window without focusing it.

| Discovered target | `System` | `WindowMessage` | `ProcessDirected` |
|---|---|---|---|
| Top-level window | Pointer, keyboard, text; focus required; invocation-only evidence | Unsupported | Qualification-gated pointer, keyboard, and text candidates; foreground-preserving; owning-process scope; `Unknown` compatibility; invocation-only evidence; no pair currently release-qualified |
| Additional same-process windows | Same target contract | Unsupported | Do not revoke process scope; no exact-window or responder-selection guarantee |
| Display | Pointer only; no focusable target is implied; invocation-only evidence | Unsupported | Unsupported |

There is still no exact-window input channel, and no request may obtain one: a
caller that requires `WindowMessage` fails admission with
`UnsupportedCombination` before any event. `ProcessDirected` is explicit
caller opt-in — selecting it authorizes posting to the owning process, and the
descriptor never upgrades that to an exact-window claim. A fallback plan may try
`System` only after a preceding route was refused without possible native
effect. The Adapter never substitutes system input on its own: doing so would
focus a window the caller asked not to disturb.

`ProcessDirected` publication is gated per operation pair: a pair is advertised
only when its mandatory revision-bound native rows in the
[ADR 0029](adr/0029-macos-process-directed-input.md) qualification matrix pass.
`CapabilitySupport::Unknown` is the strongest publishable compatibility claim,
and a failed or unexecuted route-wide row removes every dependent pair. The
negotiated target descriptor, not a platform guess, is the admission authority.
For the corrected source, the first rule currently yields zero advertised
release pairs because exact single-display and same-scale rows remain
unexecuted. Topic-branch binaries expose the candidate implementation for the
controlled checks below; that is not a release support statement.

Every target names `PermissionKind::InputControl` as the authorization input
needs, separately from the Screen Recording that capture needs. `InputControl`
is the MadoPilot contract name; on macOS its authority is the public,
non-prompting `CGPreflightPostEventAccess` decision. macOS may group that grant
under an Accessibility-labelled Privacy & Security pane, but the legacy
`AXIsProcessTrusted` result is not the input-post authorization source. Naming
the permission is not a claim that it is held.

Every refused or undetermined permission report names two independent execution
axes. Bundle launch is `bundled`, `unbundled`, or `unknown`; signature mode is
`unsigned`, `invalid`, structurally valid `ad-hoc`, structurally valid
`certificate-backed`, or `platform-failure`. The Adapter reads signature state
through dynamically loaded public Security.framework code-signing APIs.
Structured diagnostic records contain only reviewed enums, numbers, identifiers,
counts, and flags. They never carry the signing identifier; the dedicated
fixture prints that identifier only on its explicit evidence line.

## Coordinates

All five shared pointer spaces are advertised, because macOS capture publishes an
authoritative per-frame placement. The macOS desktop plane is one continuous space
of points with a top-left origin on the main display and signed coordinates for
anything above or to the left of it — the same plane `CGEvent` accepts, so a
resolved coordinate is posted without rounding. A Retina target's capture pixels
convert through its backing scale, and a point outside the target's own half-open
rectangle is refused rather than clamped into whatever is next to it.

A request still fails when its geometry policy cannot resolve the named frame:

- `ReprojectCurrent` reads the target's live rectangle and backing scale.
- `RequireUnchanged` accepts only a source frame retained for that target whose
  geometry fingerprint still equals the live target.
- `UseFrameSnapshot` uses a retained source transform. A revision that is no
  longer retained is unsupported rather than reconstructed from current geometry.

Under the two current-geometry policies, geometry is revalidated immediately
before every irreversible pointer event, so a window that moved between
resolution and delivery reports `GeometryChanged` instead of clicking what
moved into its place; a retained frame snapshot stays deliverable by design,
while target and process authority are still revalidated.

## Authorization, focus, and submission evidence

macOS does not fail a synthesized event from an unauthorized process; it
discards it, and neither `CGEventPost` nor `CGEventPostToPid` reports anything
either way. The Adapter therefore reads the public non-prompting
`CGPreflightPostEventAccess` decision before every irreversible event on both
routes, not once at open. A revocation observed mid-sequence stops submission
and the receipt carries the count already invoked. Nothing here calls a
permission-request API, opens System Settings, or presents any interface; an
unavailable or unreadable state is treated as unauthorized rather than as
permission.

The legacy `AXIsProcessTrusted` observation is no longer an authorization
input. The migration probe reads it beside the event-post preflight so
qualification evidence records both, and it can neither grant nor demote the
direct preflight; a disagreement between the two is an evidence fact to
investigate, never an admission input. On the qualified host, 100 quiet
pre-implementation samples observed both granted with zero disagreements.
Deterministic denial, revocation, symbol-unavailability, and disagreement rows
then passed fail-closed without prompting or implicit fallback.

Window liveness comes from a fresh, bounded shareable-content snapshot. PID and
window number only narrow that snapshot; the resulting logical `SCWindow` must
equal the object retained by the discovery `SCContentFilter`, and its current
frame supplies the geometry. This matters because the retained object's
`isOnScreen` and `frame` values remain unchanged after the source window closes.

Focus is a `System`-route precondition and an optional caller predicate on
`ProcessDirected`. Both use public, read-only Accessibility attributes: the
owning application must be active, its focused window must appear in its public
window list, and exactly one Accessibility window's top-left global position
and size must equal the freshly verified frame. The shareable-content and focus
observations are repeated after that join. Missing attributes, changed geometry,
or zero or multiple matches establish no focus and deliver nothing. Titles and
private Accessibility window identifiers are not read, and a same-PID
replacement that recycles numeric metadata still makes the old `TargetId`
report `TargetLost`. Every native observation is bounded by the caller's
remaining operation budget.

- `Preserve` cannot satisfy a focus-requiring system path, so a window request
  using it fails admission. It is the default meaningful policy for
  `ProcessDirected`, which imposes no route-level focus requirement and does not
  consult Accessibility under this policy.
- `RequireFocused` never activates anything. On either window route it performs
  the exact focus read above; an unfocused retained window returns
  `FocusRequired`, and unavailable Accessibility authority returns
  `NotAuthorized`, before any event is posted. On `ProcessDirected` the
  requirement travels with the request and is read again inside the same bounded
  native operation that revalidates retained-window authority, geometry,
  event-post access, and process lifetime, so a foreground change during those
  queries refuses instead of posting. A sequence-owned release never carries the
  predicate, because a target that lost the foreground is exactly when a held
  key or button most needs releasing.
- `ActivateIfRequired` asks macOS to activate the owning application only for
  `System`, then repeats the exact public focus read-back for a bounded period
  and reports `FocusRefused` if the retained window cannot be established as
  focused. It never claims to have raised one particular window, never passes
  `NSApplicationActivateIgnoringOtherApps`, and never uses Accessibility to
  move another application's windows. `ProcessDirected` does not activate:
  callers that need a focus predicate select `RequireFocused`.

A display target has no focus requirement, because nothing about a display is
focusable.

`submitted` counts complete logical events whose posting invocation returned.
Their evidence is `InvocationOnly` on both routes: it proves neither
operating-system admission nor application consumption. A text event that may
have had partial native effect before a later chunk failed is `Partial` even
when no logical event completed, never `Unexecuted`, and no fallback route is
tried after possible effect. ADR 0023 records why receipt evidence and a later
visual observation remain separate.

## Process-directed delivery

`ProcessDirected` addresses the retained window's owning process. Additional
windows, responders, queues, or handlers in that process do not revoke the
route and may receive or otherwise react to the event. The descriptor and
receipt therefore report owning-process scope and never exact-window delivery;
a caller that requires one exact consumer rejects the pair before input.

Immediately before every irreversible ordinary post, the shim re-establishes,
inside the caller's remaining budget and in this order:

1. the retained logical `SCWindow` is present and equal in a fresh
   shareable-content snapshot, with numeric window/PID metadata used only as a
   fail-closed cross-check;
2. the owning process still matches the process-lifetime token retained at
   discovery — a reused PID or restarted process cannot satisfy it;
3. the retained target window remains on screen at window layer zero with a
   finite frame, open and unminimized;
4. `CGPreflightPostEventAccess` still reports granted;
5. for a pointer event under `ReprojectCurrent` or `RequireUnchanged`, the
   current native rectangle still equals the geometry the coordinate was
   resolved against. `UseFrameSnapshot` keeps its retained transform and
   revalidates authority only.

Only then is one `CGEvent` constructed and posted to the process. The call
returns nothing, so a returned call records invocation-only evidence, closes
fallback for the sequence, and is never promoted to queue admission, target
observation, consumption, or visual success. A missing, duplicate, replaced,
minimized, or unavailable retained-target observation produces a typed refusal
before ordinary posting. Unrelated additional windows in the same process are
not ambiguity: they are part of the process scope the caller explicitly chose.

Release-only cleanup revalidates the original process lifetime, current PID,
route, and authorization before each bounded post. It deliberately does not
require ordinary target visibility, focus, or pointer geometry, because cleanup
must still release only state this sequence pressed after those ordinary gates
change. It never posts to a replacement process.

The route never activates or raises the target application or window, posts
through the system event stream, reads or moves the physical cursor, installs
an event tap, injects a helper, or uses a private Accessibility identifier.
Application effect is evaluated only by caller-selected searches on frames from
the retained capture stream whose identity is strictly newer than the source
observation; a complete receipt can coexist with no visual change, and the
caller owns its retry-or-fail policy.

## Keys, modifiers, and text

Everything but a printable character is a fixed hardware key code, transcribed
from the `kVK_` map. macOS defines no key code for F21 through F24, so those are
reported unsupported rather than posted as an undefined code. A `Key::Character`
is resolved through the active keyboard layout with `UCKeyTranslate`, and a
character the layout produces only with modifiers is unsupported: pressing the key
the caller named would deliver a different character. Callers use explicit modifier
events or `InputEvent::Text` instead.

A synthesized event carries exactly the modifiers *this sequence* is holding.
Modifier state is not merged with what the user happens to be holding, so a
sequence asking for a plain keystroke gets one. A release clears its own modifier
on the event that releases it.

Text is posted as `CGEventKeyboardSetUnicodeString` on a key event with code zero,
in chunks of at most sixteen UTF-16 units, and a chunk boundary never splits a
surrogate pair. The shared 4,096-character bound applies.

Scroll uses line units. The platform-neutral convention is positive down and
positive right; Core Graphics counts the opposite way on both axes, and the two
are reconciled in exactly one place inside the shim.

## Sequence-owned state and cleanup

Pressed buttons and keys belong only to the sequence that successfully pressed
them. On a partial stop they are released newest first under the independent shared
cleanup bound: at most 256 releases and no new release after 250 milliseconds.
Cleanup runs under a fresh context derived from the request's clock with no
cancellation, because the request's own interruption is usually why cleanup is
running.

`System` cleanup deliberately does **not** revalidate focus or geometry. A
window that stopped being frontmost is exactly when a held button matters most.
A `ProcessDirected` release revalidates the original retained process lifetime,
current PID relationship, route, authorization, deadline, and cancellation. It
does not require ordinary target visibility, focus, or pointer geometry: those
states may be why cleanup is required, and release purpose still refuses a
replacement process. Cleanup never claims that external keyboard or pointer
state was restored: a receipt reports `Incomplete` for a release the platform
refused and `Exhausted` for one that was never attempted, and the two leave a
caller with different options.

## Dedicated fixture

`mado-pilot-macos-input-fixture` creates one controlled window with:

- the exact title `MadoPilot Input Fixture [<pid>]`;
- one fixed fill colour and no other content, so a captured frame of it contains
  nothing from the user's desktop;
- an opt-in replacement mode that destroys that exact window on AppKit's main
  thread and creates a same-process, same-title successor with a deliberately
  distinct flat colour;
- a bounded report of at most 1,024 observed events, each printed as its kind and
  UTF-16 unit count and never its characters;
- the stable bundle identifier `dev.mado-pilot.macos-input-fixture` when it is run
  from a bundle;
- an opt-in `--game-like` mode that renders the same deterministic fills through
  OpenGL, dynamically loaded from the absolute system framework path, instead of
  the default AppKit background. The ready record reports the running mode as an
  explicit fact — `mode=default renderer=appkit-background` or
  `mode=game-like renderer=opengl` — and there is no silent fallback: a host
  that cannot resolve the required OpenGL symbols fails startup as unsupported
  rather than substituting the AppKit renderer.

The fixture accepts no input packet — everything its recorder observes arrived
as ordinary macOS input — but it is driven by a private, versioned control
protocol over a controller-owned socket, so capture evidence never depends on
focus, ambient redraws, or product input. Version 10 of that newline protocol
carries one bounded command at a time, identified by a per-run nonce and a
monotonic nonzero command nonce, and an explicit result record echoing the same
identity, a bounded status, the before/after native window numbers, and a
bounded process-wide event-count summary — per-kind counts, a bounded UTF-16
unit total, and a saturation flag, never characters. Approved payload-free
commands are `transition` (fill change), `replace`, `minimize`, `restore`,
`yield-foreground`, `move`, `resize`, `open-auxiliary`, `close-auxiliary`,
`move-to-next-display` (unsupported with fewer than two displays),
`move-offscreen`, `restore-onscreen`, `reset-events`, `read-events`, `close`,
and `stop`; every window transition executes on the fixture's AppKit main
queue.
Lines are bounded, unknown or
reordered records are rejected, and EOF, repeated close, and controller
cleanup are idempotent. The protocol and event recorder are
fixture/test-only: no production artifact links them, and no fixture
acknowledgement ever counts as a product receipt or a visual result.

Selection is the fail-closed step. `select_unique_fixture` requires a window
target, the exact process-qualified title, all three operations `Supported`
over `System`, `WindowMessage` unsupported, the candidate `ProcessDirected`
contract (`Unknown` support, owning-process scope, invocation-only evidence),
and exactly one match. Zero matches and several matches both stop before input.
A check then confirms selection against the window's deterministic content —
`frame_is_fixture_content` requires the sampled region to be one flat colour
within a per-platform tolerance of the declared fill — 24 channel values on
macOS, where the current ScreenCaptureKit path was observed to convert the
fixture's fill, and 8 on Windows — because an arbitrary application window is
not one flat colour.

### Bundling, ad-hoc signing, and structurally verifying the fixture

Running the bare executable is supported and reports itself as unbundled, which is
what the evidence should then say. The reproducible OSS fixture mode below creates
only a generated artifact under `target/`, supplies the stable code-signing
identifier, and uses identity `-`. For `codesign`, `-` means ad-hoc signing: the
seal has no certificate identity and does not consult a named identity in the
user's keychain.

```sh
cargo build --locked -p mado-pilot-platform-macos --features private-fixture \
  --bin mado-pilot-macos-input-fixture
APP=target/mado-pilot-fixtures/MadoPilotInputFixture.app
FOREGROUND_APP=target/mado-pilot-fixtures/MadoPilotForegroundFixture.app
mkdir -p "$APP/Contents/MacOS" "$FOREGROUND_APP/Contents/MacOS"
cp crates/platform/macos/bundle/Info.plist "$APP/Contents/Info.plist"
cp crates/platform/macos/bundle/Info.plist "$FOREGROUND_APP/Contents/Info.plist"
/usr/bin/plutil -replace CFBundleIdentifier \
  -string dev.mado-pilot.macos-input-foreground-fixture \
  "$FOREGROUND_APP/Contents/Info.plist"
cp target/debug/mado-pilot-macos-input-fixture "$APP/Contents/MacOS/"
cp target/debug/mado-pilot-macos-input-fixture "$FOREGROUND_APP/Contents/MacOS/"
/usr/bin/codesign --force --sign - \
  --identifier dev.mado-pilot.macos-input-fixture \
  --timestamp=none "$APP"
/usr/bin/codesign --force --sign - \
  --identifier dev.mado-pilot.macos-input-foreground-fixture \
  --timestamp=none "$FOREGROUND_APP"
/usr/bin/codesign --verify --strict --verbose=2 "$APP"
/usr/bin/codesign --verify --strict --verbose=2 "$FOREGROUND_APP"
"$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --report-execution-context
```

The last command must report `launch=bundled`, `signature=ad-hoc`, and
`signing-identifier=dev.mado-pilot.macos-input-fixture`. This verification proves
the generated bundle's structural code-signature validity and the running code's
classification. It does not prove that Gatekeeper would accept a distributed
artifact, that macOS made or reused any TCC decision, that a native input route
accepted an event, or that the target application consumed one. A
certificate-backed build is a different signature mode and needs evidence from
that exact artifact rather than inheriting the ad-hoc result.

## Automated macOS checks

Run the focused checks from the repository root:

```sh
cargo check --locked -p mado-pilot-platform-macos --all-targets
cargo test --locked -p mado-pilot-input
cargo test --locked -p mado-pilot-platform-macos --lib
cargo test --locked -p mado-pilot-platform-macos --test native_input
cargo test --locked -p mado-pilot-platform-macos --test fixture_signing
cargo test --locked -p mado-pilot-platform-macos --test linkage
```

Deterministic tests cover the route capability matrix for both routes,
retained-object descriptor gating, focus outcomes, Retina and signed
multi-display mapping, invocation-only evidence, exact retained-target
authority scripts (missing, one, duplicate metadata, replacement, PID reuse,
owner restart and exit), additional same-process windows that preserve process
scope, preflight revocation and disagreement representation, partial sequences,
fallback closure, cleanup completeness and exhaustion, target loss,
cancellation and deadline races, close, bounded diagnostic
loss/ordering/privacy, control protocol parsing and bounds, all five signature
classifications, identifier redaction, and separation of bundle launch from
signature mode. The fixture-signing integration check constructs and ad-hoc
signs a generated temporary bundle, runs structural verification, and exercises
only the fixture's metadata-reporting mode. The linkage check proves the eager
framework list is unchanged and that no fixture control or event-recording
symbol enters a production shim consumer. The submission cases run against the
driver seam rather than the desktop, because a live host cannot be made to
revoke an authorization or refuse a release on cue.

The native integration test **submits nothing**. It exercises discovery, the
input provider surface, and refusals that precede any event. Starting the
fixture window takes focus and needs the fixture binary, which only the
explicit `private-fixture` feature builds, so it is doubly opt-in:

```sh
MADO_PILOT_MACOS_FIXTURE=1 cargo test --locked \
  -p mado-pilot-platform-macos --features private-fixture --test native_input
```

An explicitly configured `MADO_PILOT_MACOS_FIXTURE_EXECUTABLE` selects an
already built bundle and needs no feature flag.

## Explicit owned-window replacement check

Build and verify the generated signed bundle above, then run this on the
permissioned qualified host. It sends no input and needs Screen Recording, but
no input authorization:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  owned_window_replacement_never_retargets_the_retained_filter -- \
  --ignored --exact --nocapture --test-threads=1
```

The old retained filter may report explicit `TargetLost` or remain quiescent;
individual frame-request timeouts do not establish loss. The check rejects any
successor-colour frame from that filter, independently captures the successor as
a negative control, and verifies that the retained original mapping did not
change. The accepted result was rerun at commit
`a1faf04505c8471deb4de8c136fddcc7f76105e7` and is retained in
[`evidence/g-001/macos-owned-window-replacement.md`](evidence/g-001/macos-owned-window-replacement.md).

## Explicit system-input check

Run this only on an interactive Apple Silicon desktop with Screen Recording and
event-post access granted to the process that launches `cargo test`. macOS may
surface the latter grant in an Accessibility-labelled privacy pane; that UI
grouping does not make `AXIsProcessTrusted` the Adapter's authorization source.
The non-prompting probe output, not the pane label, is the execution truth.
Build and verify the generated signed bundle above first, then deliberately
select that exact executable:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --test native_input \
  interactive_system_delivery_targets_only_the_exact_fixture -- \
  --ignored --exact --nocapture --test-threads=1
```

The check starts the fixture and requires its ready line to report a bundled,
structurally valid signature with the stable signing identifier before selecting
it exactly once. Before it opens an input controller, it opens capture for that
exact `TargetId`, waits for one frame, maps it to BGRA8, and passes the mapped bytes
through `frame_is_fixture_content`. It deliberately keeps capture open through
the input sequence: ScreenCaptureKit adds a same-owner auxiliary window while
streaming, and the regression proves that read-only focus authority still names
the exact retained fixture. Launch/signature, capture, mapping, or predicate
failure stops before the first `RequireFocused` probe.
It then waits 15 seconds: click that exact fixture window when prompted. The probe
activates nothing, and every attempt before the exact retained window is focused
submits zero events. Only then does it send Enter down and up and the fixed text
`system-probe`. It sends no click and no pointer movement, and closes capture
after input verification. The fixture event queue is deliberately delimited
after focus is established, so ordinary mouse-enter or operator focus events are
not attributed to that later submission sequence.

If the fixture is not focused in time, selection is ambiguous, deterministic
content cannot be captured and mapped, the pixels do not match, or event-post
access is absent, the check stops before further system input. Do not replace
any failure with a permission request, a settings prompt, or an activation
intended to take focus from the user.

## Explicit process-directed checks

Seven ignored native tests qualify the process route against the owned fixture.
All need Screen Recording and event-post access granted to the process that
launches `cargo test`, an interactive desktop, the `private-fixture` feature,
the structurally verified signed target bundle, and the separately identified
foreground bundle. Each renderer matrix first launches its target alone, proves
`RequireFocused` succeeds while the retained window is already focused without
the input request changing foreground, and terminates that process. It then
launches the independent foreground bundle before an inactive target; the
refusal subrow proves `RequireFocused` returns zero-effect `FocusRequired`.
Ordinary posting rows keep that unrelated bundle frontmost. Export both
executable paths before running the rows:

```sh
export MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_FOREGROUND_FIXTURE_EXECUTABLE="$FOREGROUND_APP/Contents/MacOS/mado-pilot-macos-input-fixture"
export MADO_PILOT_MACOS_QUALIFICATION_TOPOLOGY="<single|same-scale|mixed-scale>"
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_appkit_renderer -- \
  --ignored --exact --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_delivery_qualifies_game_like_renderer -- \
  --ignored --exact --nocapture --test-threads=1
```

These two tests run the positive production-route matrix independently against
the default AppKit renderer and the `mode=game-like renderer=opengl` fixture.
Each keeps desktop-independent capture active for the frozen dwell, opens an
additional ordinary same-process window, and proves that the
owning-process capability remains advertised. It independently exercises
pointer move, press, drag, release, and scroll in every public pointer coordinate
space; current and retained geometry across move, resize, and each live display;
printable, modifier-chord, Enter, F1, and arrow keys; BMP, surrogate-pair,
native-chunk-boundary, and maximum-length Unicode text; cancellation after a
pressed modifier with bounded release; repeated input/capture close; foreground
and physical-cursor preservation; target-process observation; unrelated-process
non-observation; and strictly newer controlled visual transitions. Receipts,
fixture observations, and visual results remain separate facts.
The scroll oracle fingerprints the resolved global desktop location together
with both wheel deltas. A row therefore fails if Core Graphics inherits a stale
or ambient cursor location instead of the sequence's last pointer position.

The topology selector is mandatory and fail-closed. `single` accepts exactly one
2× display; `same-scale` accepts exactly two horizontally adjacent 2× displays;
`mixed-scale` accepts a horizontally adjacent 2×/1× seam with a signed desktop
origin. The test opens every live display long enough to compare authoritative
logical size, backing extent, scale, placement, and frame identity, then requires
the retained fixture window to visit the selected topology. A passing row under
one selector cannot be reported for another.

The controlled unrelated-activity row brackets a private redraw and one real
`System` keyboard action to the owned frontmost fixture with identical
process-directed target sequences:

```sh
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  controlled_unrelated_activity_remains_outside_appkit_process_evidence -- \
  --ignored --exact --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  controlled_unrelated_activity_remains_outside_game_like_process_evidence -- \
  --ignored --exact --nocapture --test-threads=1
```

The two rows require exact target/foreground event separation, rejection of an
untagged same-payload post to the target process, unchanged process receipts,
unchanged target pixels for unrelated activity, one separately observed
foreground transition, foreground preservation, and physical-cursor invariance
in each process-directed posting window.

The bounded soak keeps each renderer's capture stream active for at least 60
seconds while two spaced process-directed sequences run under the unrelated
foreground fixture:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  sustained_capture_soak_keeps_process_route_isolated -- \
  --ignored --exact --nocapture --test-threads=1
```

The retained-authority row exercises additional-window, minimize/restore,
movement, resize, replacement, and terminal target-loss transitions:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  process_directed_delivery_uses_process_authority_and_revalidates_window_state -- \
  --ignored --exact --nocapture --test-threads=1
```

This test keeps capture active with an additional ordinary same-process window
and proves that process scope still admits input. It then closes that window,
minimizes and restores the retained target, moves and resizes it, and replaces
the retained logical window. Minimized and replacement states produce typed
zero-effect refusals; restore, move, and resize re-establish authority without
activating the target; the stale retained identity never retargets the
replacement process/window. Foreground identity, physical cursor, bounded event
summaries, and repeated close are checked throughout.

The off-screen cleanup row moves each retained renderer target outside every
display after a process-directed key press, proves the matching release still
uses the original process/source without ordinary visibility admission, and
then proves a new pointer event refuses both the off-screen and closed target
before posting:

```sh
cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input process_directed_pointer_refuses_offscreen_and_closed_targets -- \
  --ignored --exact --nocapture --test-threads=1
```

The fixture-control lifecycle row independently proves protocol version,
run/command nonce binding, rejection of stale or reordered commands, identity
continuity across replacement, and idempotent teardown:

```sh
MADO_PILOT_MACOS_FIXTURE_EXECUTABLE="$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  cargo test --locked -p mado-pilot-platform-macos --features private-fixture \
  --test native_input \
  owned_fixture_control_is_versioned_idempotent_and_identity_bound -- \
  --ignored --exact --nocapture --test-threads=1
```

Each run binds its evidence to the exact source revision and tree, fixture hash,
host, authorization state, and live display topology recorded with the Change.

For correlated process rows, the fixture reads `kCGEventSourceUserData` from
each observed `CGEvent` and accepts event, payload-digest, or visual credit only
when that nonzero value exactly matches the row token installed over the
authenticated private control channel. The unrelated-source row posts the same
keyboard payload to the same process without that tag and proves it receives no
event or visual credit before the production sequence runs.

The historical complete qualification ran at source commit
`8309a05c3e7696f3081c5afef6dd6979ea1bb084` (tree
`27fe879e0c4bb55fe4850d9a50737b568936cc10`) under fixture protocol version 9.
Its AppKit and game-like rows passed `single`, `same-scale`, and `mixed-scale`
separately, but the later product correction invalidated every pair decision.

The corrected source commit `a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426`
(tree `f4a707501748303adcec577df5f18fcd18f13f45`) uses internal shim surface
version 14 and fixture protocol version 10. Its route-wide and complete
mixed-scale AppKit/OpenGL rows pass, while exact
`single` and `same-scale` renderer rows remain unexecuted. The privacy-reviewed
[observed report](evidence/phase-2-native/macos-owning-process-qualification.md)
records both the current partial decision and the superseded complete run,
including commands, artifact and raw-output hashes, bounded outcomes, excluded
attempts, and all fourteen current `unexecuted` pair decisions. Current raw logs
remain ignored under the Change's
`ephemera/qualification-final-a1eee9c/` evidence root.

## Explicit facade check

The checks above exercise the Adapter directly. The same route through the
public Rust facade — engine construction with bounded `Debug` diagnostics,
non-prompting authorization reads, discovery, capture, mapping, a frame-bound
explicit `ProcessDirected` sequence under a shared activity tag, immutable
invocation-only receipt inspection, a strictly newer expected-condition search,
diagnostic drain, and explicit close — is
`crates/mado-pilot/examples/macos-native-input.rs`. Start the fixture with its
opt-in animation, then name its exact window title:

```sh
cargo run --locked --package mado-pilot-platform-macos \
  --features private-fixture \
  --bin mado-pilot-macos-input-fixture -- --animate-on-input
cargo run --locked --package mado-pilot --example macos-native-input -- \
  "MadoPilot Input Fixture [<pid>]"
```

It matches the title exactly, refuses zero or more than one match, and refuses
before discovery when either authorization is missing. It requires
`ProcessDirected` with `FocusPolicy::Preserve`, asserts the candidate contract
(`Unknown` support, owning-process scope, invocation-only evidence), and never
activates or focuses the selected window; the fixture's input-driven animation
is what turns a received event into new pixels. Point it only at the fixture —
it posts real input to the owning process of whatever window carries that exact
title. A complete receipt is not treated as success until an independent
strictly newer frame has the expected changed fill. Drained records carry only
the documented metadata for the shared activity tag.

## Explicit C and C++ boundary checks

The ABI check compiles and runs both native common-flow examples in `--check`
mode. That mode creates the real macOS engine, verifies the ABI 1.2 route,
permission, and diagnostic records without prompting, and stops before discovery
or input:

```sh
cargo run --locked --package mado-pilot-capi --example c-abi-check -- \
  --label "<host>"
```

After starting the dedicated fixture in `--animate-on-input` mode, the same
binaries exercise engine construction, both permission reads, exact discovery,
capture, mapping, a frame-bound explicit process-directed request, immutable
receipt and attempt inspection, a strictly newer expected-condition search,
diagnostic drain, and explicit close through C and then through the header-only
C++ wrapper:

```sh
target/debug/c-abi-check/macos-native-input \
  "MadoPilot Input Fixture [<pid>]"
target/debug/c-abi-check/macos-native-input-cpp \
  "MadoPilot Input Fixture [<pid>]"
```

Full mode selects the ABI 1.2 process-directed operation and delivery values
explicitly — pointer and keyboard in the C flow; pointer, keyboard, and text in
the C++ flow — asserts process scope, `Unknown` compatibility, and
invocation-only evidence, permits no other route, and never activates the
fixture, then independently evaluates a newer frame. Apply the same Screen
Recording, event-post-access, exact-title, and owned-window restrictions as the
Rust facade check. `--check` remains the only mode run unattended.

## Redaction review

Production input and diagnostic code emits no event, key, text, window title,
signing identifier, desktop content, captured bytes, native free-form message,
platform namespace, or backend name. Process-directed records additionally
never carry a PID, a native window number, a raw authorization value, or a
per-process window inventory beyond the bounded eligible-candidate count.
Permission records select only reviewed static launch/signature
classifications. The fixture prints only its own deterministic title, its
process and window numbers, its separate launch/signature modes, its mode and
renderer facts, its signing identifier, and per-event kind and unit counts. The
Objective-C fixture reads an event's characters solely to take their length and
never copies them out of that block. Interactive evidence may record
capability, route, submission counts and evidence, typed faults, cleanup
counts, and diagnostic loss counts; it must not record input text or unrelated
desktop payload.

## Current native input performance evidence

The revision-bound native input and public-language profile at
[`benchmarks/phase-2-native-input-aarch64-apple-darwin.toml`](benchmarks/phase-2-native-input-aarch64-apple-darwin.toml)
is measured and normative under
[ADR 0025](adr/0025-macos-native-input-performance-budgets.md). Its six
workloads retain 300 correct samples with zero allocation growth and exact
frame-mapping and fixture-event oracles. Each C and C++ sample provisions a
fresh approved fixture before its timed span so one sample cannot change the
next sample's visual precondition.

This profile requalifies input and public-language performance only. It does not
replace the full current-display, shared-display, retained-frame, or
AddressSanitizer acceptance matrices named below.

The process-directed route, the fixture-controlled capture stimulus, and their
budgets are a separate profile lineage whose pre-measurement ceilings were
frozen in the [ADR 0029](adr/0029-macos-process-directed-input.md)
qualification plan. That lineage was remeasured at corrected source commit
`a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426` (tree
`f4a707501748303adcec577df5f18fcd18f13f45`) in five revision-bound
`aarch64-apple-darwin` profiles:
[`phase-2-2-controlled-capture`](benchmarks/phase-2-2-controlled-capture-aarch64-apple-darwin.toml),
[`phase-2-2-controlled-transitions`](benchmarks/phase-2-2-controlled-transitions-aarch64-apple-darwin.toml),
[`phase-2-2-process-directed-appkit`](benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml),
[`phase-2-2-process-directed-game-like`](benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml),
and
[`phase-2-2-process-directed-diagnostics`](benchmarks/phase-2-2-process-directed-diagnostics-aarch64-apple-darwin.toml).
Their twenty-four workloads retain 2,700 passing samples in total, and every
correctness oracle and frozen budget passed. Allocation growth is zero for every
workload. The per-workload measurements, budgets, and artifact hashes are
bound in the privacy-reviewed
[observed report](evidence/phase-2-native/macos-owning-process-qualification.md)
rather than duplicated here. These profiles are regression ceilings for
controlled AppKit/OpenGL fixtures on the qualified host, not user-facing
latency promises. The corrected event-authority/preflight/post workloads
measured about 210–224 ms at p95, so this release makes no real-time input
latency claim. The `game-like` label identifies the controlled OpenGL renderer;
it does not establish general game compatibility. Receipts claim neither
exact-window delivery nor application consumption. Passing performance profiles
do not substitute for missing topology rows and therefore do not qualify a
process-directed pair.

## Historical Phase 2 evidence

The one-display qualified-host matrix for commit
`a1faf04505c8471deb4de8c136fddcc7f76105e7`, including the AddressSanitizer
run and full Rust, C, and C++ flows, is retained in
[`evidence/phase-2-native/macos-current-display.md`](evidence/phase-2-native/macos-current-display.md).
ADR 0021 invalidated its former acceptance status after source, oracle, profile,
and toolchain drift. It is historical evidence, not authorization for the
current release candidate. The exact owned-window replacement oracle remains
retained separately in
[`evidence/g-001/macos-owned-window-replacement.md`](evidence/g-001/macos-owned-window-replacement.md).
Revision-bound current-display and shared-display matrices remain release gaps.

## Frameworks

Input adds no crate and no eager framework. `CGEvent`, `CGWindowList`, and the
legacy Accessibility observation come from frameworks the build script already
declares, and the process route's `CGEventPostToPid` and
`CGPreflightPostEventAccess` entry points are resolved by symbol from the
absolute CoreGraphics framework path on first use, so a host that cannot supply
them reports a typed unsupported result instead of failing a load. AppKit — for
application activation — HIToolbox — for the keyboard-layout lookup — and
Security.framework — for public code-signature inspection — are opened from
their absolute system paths on first use, exactly as ScreenCaptureKit is, so a
headless library adds a load command for none and the operation that needed one
reports an explicit unavailable/platform result where it cannot run.
`crates/platform/macos/tests/linkage.rs` asserts the eager list is unchanged
and that no fixture control or event-recording symbol reaches a production shim
consumer. The fixture's window, control protocol, event recorder, and game-like
OpenGL renderer are compiled into a separate archive that no released artifact
links; the fixture alone loads OpenGL, from the absolute system framework path,
and only in `--game-like` mode.
