# macOS Input Adapter and Verification

The macOS platform package implements input at the Adapter boundary, and
`mado_pilot::macos_engine` wires it into the runtime and public Rust facade. The
C ABI and header-only C++ wrapper consume that same facade-owned engine. Release
acceptance consists of the explicit native checks below; ordinary test runs never
send desktop input or open the opt-in fixture window.

Current candidate release status: **0 qualified, 0 rejected, and 14 unexecuted
controlled pairs**. Final candidate
`dec43d7b6c91d415f2028e188e89fa289cb9c1c9` passed the controlled AppKit,
game-like, and native input/public-language profiles. The three-display
`mixed-scale` native rows, deterministic one-read proofs, sanitizer, and
ABI/C++/CMake checks passed on predecessor `df1c45d` and apply to `dec43d7`
because their complete diff is the benchmark harness alone. Hosted CI passed on
source/test commit `7ce1602` with evidence head `8c19a17`; the later CI-summary
successor changes documentation only.
The disconnected `single` and exact two-display non-mirrored `same-scale` rows
remain unavailable, so no release pair is promoted. Nothing
here qualifies arbitrary applications, arbitrary games, exact-window delivery,
or application consumption.

## Capability boundary

Capabilities are operation-and-route pairs. macOS publishes two routes with
different address scopes: `System` posts `CGEvent` at the HID tap and reaches
whatever is focused, and `ProcessDirected` posts `CGEventPostToPid` to the
process that owns the retained window without focusing it.

| Discovered target | `System` | `WindowMessage` | `ProcessDirected` |
|---|---|---|---|
| Top-level window | Pointer, keyboard, text; focus required; invocation-only evidence | Unsupported | Implemented pointer, keyboard, and text; foreground-preserving; honours a caller-selected `RequireFocused` predicate without activation; owning-process scope; `Unknown` compatibility; invocation-only evidence; release support is unexecuted on the optimized source |
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

`ProcessDirected` release publication is gated per operation pair: a source may
call a pair qualified only when its mandatory revision-bound native rows in the
[ADR 0029](adr/0029-macos-process-directed-input.md) qualification matrix pass.
`CapabilitySupport::Unknown` is the strongest publishable compatibility claim,
and a failed or unexecuted route-wide row blocks every dependent pair. The
negotiated target descriptor remains the runtime admission authority, but its
implemented mapping is not evidence that the current source passed the release
gate. The pre-optimization source qualified fourteen pairs. The `df1c45d`
`mixed-scale` rows apply to final candidate `dec43d7` through the
benchmark-harness-only diff, but all fourteen current
release decisions remain unexecuted because disconnected `single` and exact
two-display `same-scale` rows are unavailable.

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

- `ReprojectCurrent` reads the target's live rectangle and backing scale in Rust,
  maps from that geometry, and asks the final native gate to confirm the same
  fingerprint before posting.
- `RequireUnchanged` maps from the source-frame transform and carries that
  frame's raw screen rectangle and backing scale to the final native gate. The
  gate compares source and live raw point sizes after backing-pixel
  quantization; it does not reconstruct native bounds from an effective,
  potentially downscaled transform.
- `UseFrameSnapshot` uses the retained source transform. A revision that is no
  longer retained is unsupported rather than reconstructed from current
  geometry; the final gate still checks exact retained-window authority but does
  not reject movement.

The pre-optimization terminal `RequireUnchanged` pointer path made four fresh
ScreenCaptureKit inventory reads: route preflight, Rust live geometry, native
preparation, and native final authority. The optimized one-event terminal path
with `RequireUnchanged` or `UseFrameSnapshot`, default no-focus behavior, and no
later fallback makes one authoritative final read. A fallback-eligible route
makes one early zero-effect read plus its final read, while terminal
`ReprojectCurrent` makes one Rust live-geometry read plus the final native read.
Those are separate two-read cases; combining stronger policies can require more.
A delay-only sequence makes one early authority read because it has no native
commit gate. `RequireFocused`, fallback, `ReprojectCurrent`, cleanup, and
multi-unit sequences are outside the one-read latency claim.

At the final observation, `ReprojectCurrent` and `RequireUnchanged` refuse a
mismatch with `GeometryChanged` rather than posting into changed geometry.
`RequireUnchanged` does not remember transient movement: geometry restored to
the source-frame value before the final observation passes. `UseFrameSnapshot`
stays deliverable across movement by design while target and process authority
are still revalidated.

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
and size must equal the freshly verified frame. The shareable-content identity
and frame are read again after the Accessibility snapshot. Missing required
attributes make focus unobservable; a complete unequal or ambiguous observation
establishes that the retained window is not focused. Both outcomes deliver nothing.
Titles and private Accessibility window identifiers are not read, and a same-PID
replacement that recycles numeric metadata still makes the old `TargetId`
report `TargetLost`. Every native observation is bounded by the caller's
remaining operation budget.

- `Preserve` cannot satisfy a focus-requiring system path, so a window request
  using it fails admission. It is the default meaningful policy for
  `ProcessDirected`, which imposes no route-level focus requirement and does not
  consult Accessibility under this policy.
- `RequireFocused` never activates anything. On either window route it performs
  the exact focus read above. A complete observation of an unfocused retained
  window returns `FocusRequired`; withheld Accessibility authorization returns
  `NotAuthorized`; and missing or inconsistent required Accessibility data
  returns `SubmissionFailed`. All refuse before any event is posted. On
  `ProcessDirected` the requirement travels with the request and is read again
  inside the same bounded native operation that revalidates retained-window
  authority, geometry, event-post access, and process lifetime, so a foreground
  change during those queries refuses instead of posting. A sequence-owned
  release never carries the predicate, because a target that lost the foreground
  is exactly when a held key or button most needs releasing.
- `ActivateIfRequired` asks macOS to activate the owning application only for
  `System`, then repeats the exact public focus read-back for a bounded period. A
  complete observation that remains unfocused reports `FocusRefused`; an
  authorization or platform observation failure retains its typed fault. It
  never claims to have raised one particular window, never passes
  `NSApplicationActivateIgnoringOtherApps`, and never uses Accessibility to move
  another application's windows. `ProcessDirected` does not activate: callers
  that need a focus predicate select `RequireFocused`.

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

The route controller requests an early retained-window authority observation
only when a later caller-ordered route could still be selected or when a
delay-only sequence has no native commit gate. That observation is a zero-effect
fallback prerequisite. A terminal route containing a native event defers mutable
window authority to the final gate; target loss can therefore surface after
route selection, and that refusal does not reopen fallback.

Each ordinary native unit then stays inside one contained shim entry. In order,
the shim:

1. checks reversible prerequisites: current non-prompting event-post access,
   original process lifetime and PID relationship, and the caller-selected focus
   predicate when `RequireFocused` applies;
2. constructs one `CGEvent`;
3. repeats the caller-selected focus predicate when `RequireFocused` applies;
4. performs one authoritative shareable-content inventory read and proves that
   the exact retained logical `SCWindow` is still present, equal, open,
   unminimized, on screen, at window layer zero, and finite;
5. applies the selected geometry policy — compares the final rectangle with the
   source frame for `RequireUnchanged`, compares it with the Rust live mapping
   for `ReprojectCurrent`, or retains snapshot mapping while still checking
   target authority for `UseFrameSnapshot`;
6. repeats authorization and original process lifetime/PID checks, then checks
   the deadline and cancellation immediately before the irreversible call;
7. enters `CGEventPostToPid`.

An event constructed before a final refusal is released in the shim's
`@finally` path and is never posted. Entering the void post call is the
possible-effect boundary; normal return increments the invoked-native-unit
count but still proves only invocation. A missing, duplicate, replaced,
minimized, off-screen, or unavailable retained target produces a typed refusal
before ordinary posting. Unrelated additional windows in the same process are
not ambiguity: they are part of the process scope the caller explicitly chose.

Release-only cleanup revalidates the original process lifetime, current PID,
route, authorization, deadline, and its independent bound before each post. It
performs no ordinary retained-window inventory, focus, visibility, or pointer
geometry check, because cleanup must still release only state this sequence
pressed after those facts change. It never posts to a replacement process.

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
A `ProcessDirected` release performs zero retained-window inventory reads. It
revalidates the original retained process lifetime, current PID relationship,
route, authorization, deadline, cancellation, and bounded release state, but
does not require ordinary target visibility, focus, or pointer geometry. Those
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
focus, ambient redraws, or product input. Version 11 of that newline protocol
carries one bounded command at a time, identified by a per-run nonce and a
monotonic nonzero command nonce, and an explicit result record echoing the same
identity, a bounded status, the before/after native window numbers, and a
bounded process-wide event-count summary — per-kind counts, a bounded UTF-16
unit total, and a saturation flag, never characters. Approved payload-free
commands are `transition` (fill change), `replace`, `minimize`, `restore`,
`yield-foreground`, `move`, `resize`, `open-auxiliary`, `close-auxiliary`,
`move-to-next-display` (unsupported with fewer than two displays),
`move-offscreen`, `restore-onscreen`, `reset-events`,
`prepare-language-flow`, `read-events`, `close`, and `stop`;
`prepare-language-flow` alone restores the declared base fill while it zeroes
the recorder, so animated language samples start from one deterministic visual
precondition without changing ordinary native qualification state. Every window
transition executes on the fixture's AppKit main queue.
Lines are bounded, unknown or
reordered records are rejected, and EOF, repeated close, and controller
cleanup are idempotent. The protocol and event recorder are
fixture/test-only: no production artifact links them, and no fixture
acknowledgement ever counts as a product receipt or a visual result.

The controller loads AppKit only from its absolute system framework path, asks
`NSWorkspace` for a new application instance, and retains the exact returned
`NSRunningApplication` and PID. It accepts a control peer only when kernel
socket credentials report that PID, the harness effective user, the canonical
executable, and an audit token that still names that process lifetime. Before
trusting a ready record, the controller requires the audit-token-selected
running code identity to equal the validity-checked static identity recorded
beside the artifact SHA-256. It retains that identity through teardown. A
same-user second copy at the same path therefore cannot become the oracle merely
by learning the socket path or run nonce.

Selection is the fail-closed step. `select_unique_fixture` requires a window
target, the exact process-qualified title, all three operations `Supported`
over `System`, `WindowMessage` unsupported, the advertised `ProcessDirected`
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
TARGET_BUNDLE_ID="$(
  xcrun plutil -extract CFBundleIdentifier raw "$APP/Contents/Info.plist"
)"
FOREGROUND_BUNDLE_ID="${TARGET_BUNDLE_ID}.foreground"
xcrun plutil -replace CFBundleIdentifier \
  -string "$FOREGROUND_BUNDLE_ID" \
  "$FOREGROUND_APP/Contents/Info.plist"
cp target/debug/mado-pilot-macos-input-fixture "$APP/Contents/MacOS/"
cp target/debug/mado-pilot-macos-input-fixture "$FOREGROUND_APP/Contents/MacOS/"
xcrun codesign --force --sign - \
  --identifier "$TARGET_BUNDLE_ID" \
  --timestamp=none "$APP"
xcrun codesign --force --sign - \
  --identifier "$FOREGROUND_BUNDLE_ID" \
  --timestamp=none "$FOREGROUND_APP"
xcrun codesign --verify --strict --verbose=2 "$APP"
xcrun codesign --verify --strict --verbose=2 "$FOREGROUND_APP"
"$APP/Contents/MacOS/mado-pilot-macos-input-fixture" \
  --report-execution-context
```

The last command must report `launch=bundled`, `signature=ad-hoc`, and a
`signing-identifier` equal to `$TARGET_BUNDLE_ID`. This verification proves
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
Its rows are superseded.

The corrected pre-optimization source commit
`a1eee9c14a0bd9a1ba92a5ceeff53d378c33f426` (tree
`f4a707501748303adcec577df5f18fcd18f13f45`) uses internal shim surface
version 14 and fixture protocol version 10. Its route-wide and complete
`single`, `same-scale`, and `mixed-scale` AppKit/OpenGL rows passed, qualifying
fourteen controlled pairs on that revision only.

Review invalidated the first optimized candidate `28ceb2e`: its final
`RequireUnchanged` check compared the source transform's normalized logical
size with raw `SCWindow.frame` size. A deterministic native seam reproduced an
unchanged 320.4-point Retina window whose 641-pixel capture normalizes to 320.5
points and proved the raw rectangle equality rejected it. Source commit
`a471c2d51428a25dd11e42572b73cf5e86ef7478` (tree
`3f5ada8d116b527a8644be4d804f91341bc1e296`) instead compared exact desktop
origin and backing scale plus integral capture extent and redacted fixture
process identities. Its native rows remain historical; its retained benchmark
bodies are source/oracle-misbound because their commanded-transition oracle is
absent from that tree.

Candidate `9e3e77d4021b792f4c4835390658aaac98e76826` (tree
`ea7881c4416ca2a330fa3097d4fa271f9a547f96`) passed its own earlier
exact-source `mixed-scale` rows and controlled profiles; later review-driven
source, fixture, and harness corrections invalidated that evidence and it is
historical. Predecessor `df1c45d` then passed the signed-release
`mixed-scale` matrix — 34 permission-independent rows, all 14 ignored
interactive rows, and three topology scenarios. Final measured candidate
`dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
`109f77df9ef9f40b515245ab60a6036822ee7d78`) carries same-frame raw screen
bounds and backing scale, uses private shim surface version 19 and fixture
protocol version 11, and differs from `df1c45d` only in
`crates/mado-pilot/benches/native-phase2.rs`, so the `df1c45d` native matrix
applies to it. The `single` and `same-scale` rows did not run, so every
release pair remains `unexecuted`.

The privacy-reviewed
[repository evidence](evidence/phase-2-native/macos-owning-process-qualification.md)
keeps immutable historical records separate from the current decision. The
tuning Change's
[observed report](../rasen/changes/macos-process-directed-performance-tuning/evidence/observed-report.md)
records exact commands, hashes, bounded outcomes, the stopped pre-sample
attempt, and raw-log exclusion.

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
`ProcessDirected` with `FocusPolicy::Preserve`, asserts the advertised contract
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
is normative under
[ADR 0025](adr/0025-macos-native-input-performance-budgets.md) and measured
at final candidate `dec43d7`. Its six workloads retain 300 correct samples
with maximum allocation growth 64 bytes under the 4,096-byte hard gate and
exact frame-mapping and fixture-event oracles. The harness provisions each C
and C++ sample's fresh approved fixture outside its timed span and retains
controller-owned mode-0500 executable/library pins per workload, so one
sample cannot change the next sample's identity, lifecycle, or visual
precondition.

This profile requalifies input and public-language performance only. It does not
replace the full current-display, shared-display, retained-frame, or
AddressSanitizer acceptance matrices named below.

The process-directed route, fixture-controlled capture stimulus, and their
budgets form a separate profile lineage whose pre-measurement ceilings were
frozen in [ADR 0029](adr/0029-macos-process-directed-input.md). Controlled
capture, controlled transitions, and process diagnostics remain bound to their
named pre-optimization revision. The two authority-timing-sensitive current
profiles are measured and normative at source
`dec43d7b6c91d415f2028e188e89fa289cb9c1c9` (tree
`109f77df9ef9f40b515245ab60a6036822ee7d78`):

- [`phase-2-2-process-directed-appkit`](benchmarks/phase-2-2-process-directed-appkit-aarch64-apple-darwin.toml);
- [`phase-2-2-process-directed-game-like`](benchmarks/phase-2-2-process-directed-game-like-aarch64-apple-darwin.toml).

After setting `MADO_PILOT_MACOS_FIXTURE_EXECUTABLE` to the exact signed release
fixture, reproduce either profile with:

```sh
for workload in process-directed process-directed-game-like
do
  cargo bench --locked -p mado-pilot --bench native-phase2 -- \
    --workload-set "$workload" \
    --fixture-executable "$MADO_PILOT_MACOS_FIXTURE_EXECUTABLE" \
    --source-revision dec43d7b6c91d415f2028e188e89fa289cb9c1c9 \
    --source-tree 109f77df9ef9f40b515245ab60a6036822ee7d78 \
    --toolchain "rustc 1.97.1; cargo 1.97.1; Apple clang 21.0.0; macOS SDK 26.5" \
    --gpu-driver "Apple integrated GPU; system driver stack" \
    --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
    --os-version "macOS 26.5.2 (25F84)" \
    --deployment-target "macOS 26.5.2" \
    --display-topology "three online non-mirrored displays; signed-origin display 3840x2160 1x; main display 2560x1440 logical / 5120x2880 backing 2x; built-in display 1512x982 logical / 3024x1964 backing 2x; mixed-scale validator authoritative" \
    --permissions-signing "Screen Recording granted; event-post access granted; target and foreground bundles structurally ad-hoc signed"
done
```

Each current profile retained 50 samples after five warm-ups per workload and
recorded fixture source, signed fixture executable, and benchmark executable
digests. AppKit terminal p95 was `56.466375 ms` under `106.34 ms`; controlled
game-like p95 was `56.699333 ms` under `112.18 ms`. Both recorded zero
correctness failures, one matching fixture event per terminal sequence,
unchanged foreground and physical cursor, and zero post-warm-up allocation
growth in every workload.

The one-read gate is the revision-bound conjunction of controller,
geometry-source, and native seam tests with those exact-source rows. It applies
to the terminal `RequireUnchanged`, default-focus-policy, one-pointer-event
path. `RequireFocused`, `ReprojectCurrent`, ordered fallback, cleanup, and
multi-unit sequences retain distinct observation shapes.

The benchmark bodies formerly attributed to `a471c2d` are source/oracle-
misbound and supply no result. Current performance and `mixed-scale` evidence do
not substitute for unavailable `single` or exact two-display `same-scale`
topology rows, so all fourteen release decisions remain unexecuted. These are
controlled regression ceilings, not user-facing latency promises or evidence
of general game or anti-cheat compatibility.

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
