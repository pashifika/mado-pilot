# Private Apple Silicon OCR text-watch procedure

## Current v3 contract — verified native cohort

`ocr-text-watch-apple-v3` keeps actual SDK/query termination separate from
independently established exact window or process disappearance. It separates
controlled window disappearance, exact process-generation disappearance, SDK
notification, explicit caller close, and fresh-session capture/OCR. After loss is
established, the caller cancels and closes through existing APIs before fresh
discovery/open; an old session is never rebound and immediate loss is never
invented. The private topology is one consumer and at most two sequential owned
fixture generations, with at most two native children alive at once, all direct
`Popen` children of the same bounded supervisor. No production API, macOS adapter
behavior, background poller, restart engine, automatic recovery or platform
fallback is added.

The source-bound `86a482d074e691c74aba8280c0ad42de31fead7a` execution passed all
nine semantic rows and the separate resource/cleanup endpoints on Apple M1 Pro,
macOS27.0.1/build26A434, Retina2x, CPU bounded-v2. It distinguished window-only
loss with the original process alive from normal process-generation exit/reap,
then obtained fresh acknowledged OCR in both replacement sessions. All three
children exited0 and were reaped without force; retained original pixels stayed
identical after all three sessions and the engine closed. Both actual SDK loss
signals were `TargetLost`; this does not require that signal for every close.
The complete source/artifact/authority/image/command record is
`macos-v3-window-server-native-20261022.json` in the separately versioned Change
evidence. This is one owned cohort, not crash, accelerator, Windows or general
platform/numerical qualification. Every further execution still needs a fresh
exact binding.

The source-bound `5ddc7806` v2 attempt passed acknowledged transition, retained
producer progress, resize reset/invalidity and parent-close retention. Its
`exact-window-loss` row failed with `DeadlineExceeded` after controlled window
destruction while the fixture process remained alive. Resource/cleanup passed;
both owned children exited and were reaped without force. That failure remains
unchanged and supplies no pass for any v3 classification or rediscovery row.
The separate AppKit lifetime probe established that both window and view must be
released for the WindowServer identity to disappear. SDK notification is still
an independent observation, not a prerequisite invented for every window close.

## Historical v2 apparatus observations

The earlier observations on Apple Silicon macOS 27.0.1 reported
`ScreenCapture: Granted`. The private fixture accepts an already-Accessory
application by checking the effective policy, and the image parser retains both
owned-PID dyld delayed-state transitions without treating either as an identity.

A fixture-only startup smoke observed the owned 480x278 logical window at 2x
and an unforced, reaped exit with complete output channels. It performed no
capture or OCR. The model-free procedure suite passed 117 tests with four
platform-specific skips. Replaying the first attempt's complete loader bytes
also exposed the dynamically loaded TBB allocator outside its static manifest;
that attempt remains failed and its manifest is unchanged.

The operator separately authorized one repaired-candidate attempt, prospectively
binding the TBB allocator with the other 29 non-system images. That exact set
and all source/artifact hashes matched. The owned fixture acknowledged
`arm-blank`, but the first source checkpoint failed with
`producer-progress-absent-frame`; the remaining five rows were not run.
Resource and cleanup checks passed, including zero logical/physical OCR work
after close and unforced exit/reaping of both children.

Bounded checkpoint diagnostics distinguish acquisition, mapping and the first
color/token/state rejection. The diagnostic attempts acquired correctly sized
960x556 BGRA frames, then rejected the first yellow marker cell. ScreenCaptureKit
uses the display's color space when `colorSpaceName` is unset; capture-free
AppKit conversion confirmed that this display's conversion moves sRGB yellow
outside the original byte thresholds, while black and white retain their
neutral endpoint classification.

The marker now uses opaque black/white cells without changing the 96-bit
ownership/state challenge, source order or five-second operation deadline.
Every RGB channel must remain strictly below40 or above215, with alpha255.
The image and OCR ROI are unchanged; no runtime color conversion is introduced.
The private summary retains only counts, the last exact frame stamp and mapped
extent/stride, the first rejected cell and a closed color/opacity category.
Mapping/marker facts always belong to that last acquired frame; no pixels,
decoded token or OCR text are recorded. Capture-free replay rejects ambiguous
colors and wrong token/state, accepts only the acknowledged successor, and
preserves closed acquisition. Native scenario success requires its own run.
No permission setting or input action was performed. These observations do not
qualify ScreenCaptureKit OCR, macOS support, recognition quality or numerical
workload budgets.

## Historical qualification preparation

**Task 7.3 is compiled/reviewed. Task 7.4 reached the permission gate; capture remains not-run.**
The strict Apple fixture build and Rust example passed CI and scoped verification
on the delivered `fd5468f` tree. Earlier compile failures remain recorded with
their original revisions; they were fixed without warning suppression.
Fresh task-7.4 preparation built isolated consumer/fixture bundles from unchanged
`4a9dc5b` Rust/fixture inputs and ad-hoc signed only those new bundles. Their SDK
is 27.0 and deployment minimum 26.5.2; the actual OS is macOS 26.6.2 (25G83),
not macOS 27. No model inference, fixture launch, capture, permission change,
focus action or input action is claimed. Native support and A9 remain open.
The frozen foreign/native-template controllers, evidence, grants, and binaries
are neither consumed nor changed.

The one approved attempt on `959ba687` returned `ScreenCapture: NotGranted`
(`permission-denied-or-undetermined`) on 2026-09-14. The valid ad-hoc consumer
exited 1 and was reaped without force; no fixture started and all six scenario
rows remain `not-run`. The complete 87,298-byte loader channel contains 29
approved consumer-side images; the full two-child union is not satisfied because
the fixture was never launched. This is not native success. The attempt is spent.
Resumption requires already-granted permission for the actual execution context,
outside this procedure, plus fresh applicability review and one-attempt authority.
No permission request/settings change or automatic retry was performed.

The historical `ocr-text-watch-apple-v2` procedure allowed exactly two native
children; v3 changes only that child-count and role-list restriction. Every other
channel, authority, resource and cleanup obligation below applies to both.
Historical grants, binaries and verdicts are not reused or regraded. The inert
pre-main loader probe is apparatus evidence only, not an OCR result or a failed
native qualification attempt.

## Scope and fixed oracle

One freshly approved Apple Silicon interactive desktop, its main display at
**2x**, one owned AppKit window at a time, one CPU bounded-v2 initialization,
one consumer process and two sequential fixture process generations; zero
warmups, one sample, no retries. `run.py` requires the exact approved uname array
(including node), OS version/build, CPU, SDK, source and executable identities.
It does not inherit another campaign's approval or promise support beyond the
existing adapter. The deployment floor remains macOS **26.5.2**. Non-Apple builds
refuse without simulation; native unsupported-system errors remain failures or
unexecuted prerequisites, never a fallback.

- `fixtures/ocr/g-004/hud.png`: 960x540, SHA-256
  `10f0163cf298453e55922cc6104ee3066f59a328a55e9bac6c04f24d36c9e288`.
  The OS decodes these immutable bytes; Pillow, downloaded fonts and generated
  images are unnecessary for this native lane.
- Predicate `魔導士`, confidence threshold **0.0**, finite profile confidence in
  `[0,1]`; exactly eight normalized regions, satisfying indexes exactly `[0]`.
  Region 0 must equal the literal. Its raw returned target-logical bounding box
  is compared with the independently fixed 2x oracle `(26.5,30)-(101.5,56)`:
  IoU >=0.5, center error <=12 logical units horizontally and <=6.75 vertically.
  This is source box `(53,60)-(203,112)` divided by the fixed fixture scale, not
  by the returned transform. Expectations are fixed before candidate output.
- Query ROI `(0,0)-(480,270)` in `TargetLogical`, `ClipPolicy::Reject`, output
  `TargetLogical`; effective capture ROI exactly `(0,0)-(960,540)`. The frame's
  own transform must report 2x. Every returned point must explicitly name
  `TargetLogical`; conversion must equal the raw point multiplied by the fixed
  factor 2, in addition to round-trip consistency. There is no descriptor-derived
  substitute for a missing frame transform.
- Initial window: 480x278 logical / 960x556 capture pixels. The extra eight
  logical rows contain a non-text ownership/state marker, **outside the OCR
  ROI**. The image itself is not altered. Repository artwork is Apache-2.0;
  embedded Noto Sans JP rendered pixels are OFL-1.1; no font bytes are bundled.
  See `fixtures/ocr/g-004/fixture-manifest.json` for upstream provenance.

## Components and authority

`tools/ocr-text-watch/macos/fixture.m` is a private borderless AppKit fixture,
not a product GUI or adapter. Its nonactivating `NSPanel` cannot become key/main,
ignores mouse input, and runs at normal window level under
`NSApplicationActivationPolicyAccessory`. It never activates a process. Every
acknowledgement checks that the pre-launch frontmost application
is unchanged. External focus changes fail the attempt; the fixture never repairs
focus. Screen capture and Accessibility APIs are absent from this fixture.

`crates/mado-pilot/examples/ocr-text-watch-native-macos.rs` is a public Rust API
consumer. It constructs `macos_engine_with_ocr_profile` once with
`OcrProfile::BoundedDetector` and the two explicit prerequisite environment
variables below. The runner canonicalizes each path once and uses the same
absolute values for prehashing and the child's environment after changing cwd.
The consumer checks the initialized CPU/profile identity and compares
result backend identity with that bound descriptor. It probes **only**
`Engine::permission(PermissionKind::ScreenCapture, ...)`; it neither requests
Accessibility nor opens an input controller. Recognition uses
`Session::start_ocr_text_watch`, not a caller OCR polling loop.

`tools/ocr-text-watch/macos/run.py` owns the direct native `Popen` roles
`consumer`, `fixture-1`, `fixture-2`. The consumer starts first; generation 1
starts only after its already-granted non-prompting Screen Recording decision.
Generation 2 starts only after the exact predecessor has been reaped and the
consumer requests the predefined post-close phase. There are never more than
two live native children. Each role is single-use; generation directories and
nonces are distinct. Owned PIDs are the actual executable PIDs, not wrappers or
collectors. The existing `tools/native-release-profile/process_runner.py` remains
unchanged for bounded provenance commands. No shell, LaunchServices, `open`,
process-name kill, foreign controller, collector or grandchild is used.
A timeout stops only the exact owned children and remains a failure.

## Ordered scenario

The evidence directory and each `fixture-1/` / `fixture-2/` control directory
are newly created with mode 0700, with files under umask 077. Both distinct
16-character lower-case hexadecimal nonces are supplied to the consumer before
either fixture launches. All protocol rows use canonical ASCII fields separated
by one space and exactly one trailing newline; decimal identities have no signs
or leading zeroes. Commands are at most 256 bytes, acknowledgements/receipts
512 bytes, and supervisor requests128 bytes. These files share the ordinary
aggregate; generation 2 gets no separate output allowance.

| File / writer | Exact grammar |
|---|---|
| `fixture-G/command` / consumer | `<nonce> <sequence> <operation>\n` |
| `fixture-G/ack` / fixture | `<nonce> <sequence> <operation> <pid> <native-window> <logical-width> <logical-height> 2.0\n` |
| `fixture-G/fixture-owner` / supervisor after registering `Popen` | `<nonce> <generation> <pid>\n` |
| `alive-request` / consumer after successful absent-window inventory | `<nonce1> 1 <pid1> window-absent\n` |
| `fixture-1/alive` / supervisor after exact-child liveness observation | The exact `alive-request` row |
| `fixture-G/reaped` / supervisor after exact-child reap and matching exit command/ACK | `<nonce> <generation> <pid> 0\n` |
| `successor-request` / consumer after predecessor receipt and old-session close | `<nonce1> 1 <pid1> <nonce2> 2 successor-after-close\n` |

Each command sequence is accepted once; the exact persistent last command is
not a retry. Stale acknowledgements may be observed while waiting, never resent.
Malformed or mismatched nonce/generation/PID/role/phase receipts fail closed.
A fixture's own exit ACK does not prove reaping, and neither a missing reap
receipt nor a numeric PID proves liveness. The supervisor samples its retained
unreaped `Popen` handle after validating the close-window command/ACK; failure
cleanup `stop` files cannot certify process loss. Premature, duplicate or
unowned successor launches fail; the original generation cannot be resurrected.

The nonce and 32-bit state are drawn as opaque black/white cells, outside the
OCR ROI. Unique SDK title selection locates a candidate. An independent
WindowServer query for only the acknowledged native window number must return
the PID bound to the supervisor-owned process generation. Optional SDK window
and process metadata must agree when present; their absence proves neither
ownership nor disappearance. Capture must then contain the exact marker bits
on the new session's stream. A public `TargetId` is reissued at each discovery
and its absence is not disappearance. No discovery retargets an existing session.

`target_admission` records these scalar identity observations before open.
The private consumer uses `CGWindowListCopyWindowInfo` with only
`kCGWindowListOptionIncludingWindow`, without an on-screen-only filter, and
reads only window-number and owner-PID fields. Null results, unexpected
container/types/counts, contradictory identities and late observations fail;
they are never empty-window evidence. The result is rejected after5s; a
nonreturning OS call is covered by the unchanged owned-consumer watchdog.
No image, unrelated window title, activation or permission operation is added.
Loss requires both successful fresh SDK title absence and independent original
window/owner absence, plus the separate owned-generation liveness or reap proof.

| Generation / sequence | Operation / mandatory observation |
|---|---|
| 1 / 0 | Start fully white, with ownership marker only. Ready follows AppKit display processing and pending CATransaction submission. |
| 1 / 1 | After opening and starting a two-confirmation query, `arm-blank` changes only the marker. Capture and verify the entirely white OCR ROI; record its exact source checkpoint. A separately bounded 100ms wait times out without cancelling the query. |
| 1 / 2–3 | `show` renders the fixed image. Wait for the first accepted positive (or an already committed match), then `pulse` changes only the marker. Require two distinct positive sources, both newer than checkpoint 1 in the same epoch/geometry. The retained matching pixels must carry state 2 or 3. |
| 1 / 4 | `blank`; while retaining the result, a separately cloned exact frame and its mapping, require a strictly newer state-4 frame. This is the producer-progress oracle, not just a post-close read. |
| 1 / 5–7 | Use one consecutive(2) query throughout. State 5 must establish exactly one accepted positive with no pending/logical/physical work; freeze `Completed=N`. `resize-show` preserves the identical positive ROI while changing to 520x300 logical / 1040x600 capture. Inspect the first changed completion snapshot: require exactly `N+1`, count 1, and an accepted successor epoch/geometry. After capturing state 6, recheck that no extra analysis occurred. State-7 `pulse` completes the same query; require exactly `N+2`, count 2, `first_confirmed_frame` equal to the observed successor, a newer final frame in that successor, and captured state 7. No blank reset or fresh-query substitution is accepted. |
| 1 / 8–9 | Establish blank at the larger size; start ROI `(490,280)-(510,290)` with Reject. `shrink-blank` changes to 240x140 logical. Require successor geometry and query `Failed(InvalidArgument)`, never a match. |
| 1 / 10 | Start a pending query on blank ROI `(0,0)-(100,100)`, then `close-window`. Close and release both window and view; ACK retains scalar geometry and window number only. Observe the SDK query for at most10s. Require fresh successful inventory with no owned title or original native window/process pair, then an affirmative supervisor-owned-generation liveness receipt. Cancel the query, explicitly close the old session within5s, verify physical quiescence and immutable terminal state. All this precedes replacement creation. |
| 1 / 11–13 | `open-window` must acknowledge a new native window number in the same owned process generation. Rediscover/open a fresh target/stream; capture state11 with an entirely white ROI. A new consecutive(2) query, `show`12 and `pulse`13 must produce real acknowledged OCR with the original literal, geometry, backend and source-order oracles. No old query continues. |
| 1 / 14–15 | Capture acknowledged `blank`14 and start a pending query while the replacement session exists. `exit`15 ends the owned process normally. Record SDK behavior separately; require the supervisor's exact-generation exit0 reap receipt and a fresh absent-window inventory. Cancel/close that old session, verify physical quiescence and immutable terminal state, then publish the post-close successor request. Normal exit0 is not a crash test. |
| 2 / 0–3 | Supervisor launches the successor only after the predecessor receipt and explicit request. Validate its distinct nonce/directory/owner, discover/open a fresh target and stream distinct from both previous sessions, capture `arm-blank`1, then require fresh two-confirmation OCR from `show`2 / `pulse`3. PID reuse cannot grant continuity; generation/nonce ownership and the new pixel challenge remain mandatory. |
| Final / 2:4 | Close each opened session exactly once, establish zero logical/physical OCR work and mapping reservations/barrier, and drop every session and the engine. Remap/read the original retained OCR/frame/mapping and compare every mapped byte. Release retained owners; generation2 acknowledges `exit`4 and the consumer waits for its supervisor reap receipt. Every child/channel must still exit/reap/drain boundedly. |

`sdk_observation` records `target-lost` or `none-within-bound` separately from
independent disappearance and cleanup. The latter requires that the query still
be pending at the caller's 10s observation boundary; it is not a fabricated SDK
loss. The query's own absolute 100s lifetime is unchanged. A query lifetime
deadline, arbitrary SDK/backend error, unexpected terminal state, hidden/minimized
window, discovery failure, no frame, or absent notification alone cannot certify
native loss. Rejected observations record `signal=failed` with a separate typed
status or closed rule label. Cancellation after the observation may race with
real `TargetLost`; only either legitimate winner is accepted, and its exact
terminal object must remain immutable across close.

The nine rows are `acknowledged-transition`, `retained-producer-progress`,
`resize-reset`, `resize-invalidity`, `window-loss-classification`,
`same-process-rediscovery`, `process-loss-classification`,
`new-process-rediscovery`, and `parent-close-retention`. There is no current alias
for the historical `exact-window-loss` row.

Acknowledgement follows `displayIfNeeded` returning and `CATransaction` flushing
pending transactions. It does not assert GPU, WindowServer, or ScreenCaptureKit
completion. The independently captured marker, immutable blank checkpoint,
exact frame ordering and fixed OCR result supply that missing proof.
A post-open frame or coincidental text is insufficient. Resize failure cannot be
hidden by a negative analysis: `Completed` includes both positive and negative
analyses, and the first changed snapshot is accepted once or rejected immediately.
An early terminal, count zero, or extra completion fails; polling does not wait
until counts happen to fit. Deterministic mid-inference generation/race proofs
remain task 6.1 rather than this native row.

## Prospective bounds and verdicts

The exact machine-readable limits are `BOUNDS` in `run.py`: construction 45s;
permission/launch stage 65s; fixture readiness 10s; each control, frame checkpoint,
mapping, session close and physical drain 5s; semantic wait 15s maximum, including
the 10s SDK-loss observation; query lifetime 100s; consumer wall clock 200s; each
fixture's independent fuse 210s. Child exit gets 5s grace, then 2s termination and
2s kill/reap. No close retry. Core dumps remain
disabled with `RLIMIT_CORE=0`; inherited `RLIMIT_FSIZE` is unchanged, never raised
or replaced with an evidence-channel limit. This is not a sandbox for arbitrary
native regular-file writes. Provenance subprocesses retain their own 10s/64KiB
limits.

The complete authority `bounds` must equal `run.py::BOUNDS`, including these
byte limits and `consumer_processes: 1`, `fixture_processes: 2`,
`concurrent_native_children: 2`, `warmups: 0`, `samples: 1`:

| `BOUNDS` key | Bytes | Scope |
|---|---|---|
| `each_output_file_bytes` | `65536` | Each ordinary stdout/stderr file; the existing 64 KiB `consumer.report` admission limit also remains. |
| `total_output_bytes` | `262144` | All ordinary/control/report output across both generations, including `record.json`; exclude only the three named native-image files below. |
| `each_native_image_file_bytes` | `1048576` | Each separate native-image file. |
| `total_native_image_bytes` | `2097152` | All three native-image files together. |

Each child has separate anonymous stdout, stderr and native-image pipes, drained
with nonblocking parent reads into exclusive mode-0600 private files.
`DYLD_PRINT_LIBRARIES=1` and `DYLD_PRINT_TO_FILE=/dev/fd/N` direct loader output
to that child's inherited image descriptor. `consumer.native-images`,
`fixture-1.native-images` and `fixture-2.native-images` contain complete loader
streams, including system images; only these three files are excluded from
ordinary-output accounting. No collector process or new process-wide file-size
limit is used: `RLIMIT_CORE=0` and inherited `RLIMIT_FSIZE` stay unchanged and
are never raised. Ordinary stdout/stderr keep their own per-file and total
budgets, and loader bytes, including system images, count toward the separate
native-image budgets in `BOUNDS`. No replay collector or changed replay evidence
is involved.

The supervisor bounds work per loop and drains every channel during execution
and graceful/terminate/kill cleanup, including final exit tails. It never waits
on a child without servicing its pipes. Every started exact child must still be
reaped and every endpoint closed after a pump or storage failure. Completeness
requires EOF and successful bounded collection; saturation, incomplete or
malformed image observation, and collection failure remain failures, never a
silently truncated pass. Successful cleanup cannot replace the first failure.
After child cleanup and evidence validation, interruption admission closes before
the final bounded record snapshot. Every earlier accepted signal is retained;
signals after this commit point do not invalidate completed work. Publication
and storage failures still fail the command rather than producing success.
These are apparatus safety endpoints, **not** changes to any accepted `G-013`
ceiling, task-8 qualification, deployment floor or numeric floor.

| Observation | Classification |
|---|---|
| `Granted` | May attempt capture, not a success result. |
| `NotGranted` | `denied-or-undetermined`, not-run. The public non-prompting boolean cannot distinguish an explicit TCC denial from no decision; no private TCC database is consulted. |
| `Unknown` | `undetermined`, not-run. |
| `Unavailable` / unsupported host or adapter | Explicit unsupported prerequisite/outcome; never pass/skip inflation. |
| No owned source/checkpoint or no acknowledged text frame by its bound | Producer-progress/semantic failure once native execution starts, not a skip. |
| Correct match, failed close/drain/exit | Semantic success may remain recorded; resource/cleanup fail independently; overall does not pass. |
| Channel saturation, incomplete/malformed image observation or collection failure | Failed apparatus/resource admission; no partial-image-set pass or automatic retry. |
| Unreached scenario | `not-run`; earlier executed failures remain failed. |

Private `consumer.report` contains source stamps, backend/provider descriptors,
work observations, actual retained/mapped extents and row verdicts.
Each source checkpoint adds at most one `source_checkpoint` summary on return,
within the same report/output budgets. `acquired=0` distinguishes an acquisition
refusal with no observed frame from marker rejection; `phase=map` identifies a
mapping failure. Rejection counters describe the first failed marker check on
each rejected frame, not every incorrect cell. They cannot establish the cause
of an unrecognized color or change the scenario verdict.
`record.json` uses `procedure: "ocr-text-watch-apple-v3"` and adds exact
commands/outputs, host, approval/build-record hashes, clean Git head/tree,
consumer/fixture/model/runtime/source hashes, valid signing metadata, actual
dyld-loaded image paths/hashes, exit status and cleanup facts. Private channel
facts record observed/retained bytes, EOF/completeness and failures without
replacing the existing semantic/resource/cleanup/status fields.
Only complete role-specific `*.native-images` files in the declared roster supply
dependency-image records. Strict decoding and row parsing reject malformed
records or an incomplete final row; ordinary stderr is never an image fallback.
The exact owned-PID `move loaded to delayed: <basename>` and
`move delayed to loaded: <basename>` diagnostics are retained but supply no
image identity; other non-image syntax is rejected.
The bounded approved canonical non-system manifest is checked before launch,
against the exact observed union after all three channels finish, and against disk
hashes again after exit. System shared-cache libraries bind to OS build, but
their raw loader bytes still count toward the image budgets. Missing, additional,
noncanonical or changed developer-owned images fail resource admission.
Ordinary consumer error interpretation still reads `consumer.stderr`.
Ordinary output contains only closed verdict/reason labels, never
recognized text, pixels/hashes, model/runtime paths or signing identifiers.
No RSS ceiling, native thread-unload fence, arbitrary retained-clone memory bound,
or task-8 qualification is claimed.

## Fresh native candidate build and signing — separate authority required

Use ordinary installed Command Line Tools, the pinned Rust toolchain, Python
3.13+, and the repository's existing OpenCV/libclang prerequisites. No dependency
is downloaded. Save the command/output transcript privately as `OCR_APPLE_BUILD_RECORD`.
Use a **new** root for each build purpose; never overwrite a frozen/qualified root.

```sh
umask 077
mkdir -p target
BUILD="$(mktemp -d "$PWD/target/ocr-text-watch-apple-build.XXXXXX")"
FIXTURE_APP="$BUILD/OcrOwnedFixture.app"
CONSUMER_APP="$BUILD/OcrPublicConsumer.app"
mkdir -p "$FIXTURE_APP/Contents/MacOS" "$CONSUMER_APP/Contents/MacOS"
xcrun clang -Wall -Wextra -Werror -arch arm64 -mmacosx-version-min=26.5.2 -fobjc-arc \
  -fobjc-arc-exceptions -framework AppKit -framework QuartzCore \
  tools/ocr-text-watch/macos/fixture.m \
  -o "$FIXTURE_APP/Contents/MacOS/ocr-owned-fixture"
python3 tools/setup-native.py -- cargo build --locked -p mado-pilot \
  --example ocr-text-watch-native-macos --target aarch64-apple-darwin \
  --target-dir "$BUILD/cargo"
cp "$BUILD/cargo/aarch64-apple-darwin/debug/examples/ocr-text-watch-native-macos" \
  "$CONSUMER_APP/Contents/MacOS/ocr-public-consumer"
plutil -create xml1 "$FIXTURE_APP/Contents/Info.plist"
plutil -insert CFBundleIdentifier -string dev.madopilot.private.ocr-fixture "$FIXTURE_APP/Contents/Info.plist"
plutil -insert CFBundleExecutable -string ocr-owned-fixture "$FIXTURE_APP/Contents/Info.plist"
plutil -insert CFBundlePackageType -string APPL "$FIXTURE_APP/Contents/Info.plist"
plutil -insert LSUIElement -bool YES "$FIXTURE_APP/Contents/Info.plist"
plutil -create xml1 "$CONSUMER_APP/Contents/Info.plist"
plutil -insert CFBundleIdentifier -string dev.madopilot.private.ocr-consumer "$CONSUMER_APP/Contents/Info.plist"
plutil -insert CFBundleExecutable -string ocr-public-consumer "$CONSUMER_APP/Contents/Info.plist"
plutil -insert CFBundlePackageType -string APPL "$CONSUMER_APP/Contents/Info.plist"
plutil -insert LSBackgroundOnly -bool YES "$CONSUMER_APP/Contents/Info.plist"
codesign --sign - "$FIXTURE_APP"
codesign --sign - "$CONSUMER_APP"
```

No Cargo feature/dependency or shared-manifest change is required for this
example. Main/CI owns later native binding compilation and integration checks.
The consumer depends on the new public OCR watch/result/progress/observation
accessors agreed with the runtime owner. Do not add private adapter hooks to
make this procedure pass.

## Parent verification before native admission

Verification is pending for this v3 candidate. The parent runs the existing
model-free apparatus suite, the macOS example's capture-free Rust tests, and the
strict fixture build in separate fresh output directories. These checks do not
launch AppKit or exercise ScreenCaptureKit:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/ocr-text-watch/tests -p 'test_native_macos_runner.py'
python3 tools/setup-native.py -- cargo test --locked -p mado-pilot \
  --example ocr-text-watch-native-macos --target aarch64-apple-darwin \
  --target-dir "$OCR_APPLE_CONTRACT_TARGET"
```

`OCR_APPLE_CONTRACT_TARGET` must be a fresh contract-only Cargo target directory,
not the candidate or any hash-pinned qualification directory. Build the native
candidate using the separate build/signing procedure above, retain its exact
hashes, and do not rebuild it with test/lint commands. Parent-owned native
execution then requires the fresh authority below and must retain all nine row
verdicts, independent SDK observations, generation receipts, retained-byte proof
and complete role-specific channel/exit facts. No test-double or source review
substitutes for those native observations.

## Fresh authority and bounded execution

`OCR_APPLE_AUTHORITY` must name an independently reviewed private JSON document,
not a generated consent flag. Required fields: `approved: true`, `task: "7.4"`,
`procedure: "ocr-text-watch-apple-v3"`,
`roles: ["consumer", "fixture-1", "fixture-2"]` in that order, and
`target: "aarch64-apple-darwin"`; `host` with the complete recorded `uname` array
(`system`, `node`, `release`, `version`, `machine`, `processor`), the lower-case
`machine_uuid` from `IOPlatformUUID`, and exact `product_version`, `build`, `cpu`,
`sdk`; the exact `BOUNDS` object. The bounded read-only I/O Registry identity
stays in private evidence; it is neither a permission probe nor a permission grant.
`source_head`, `source_tree`, `consumer_sha256`, `fixture_sha256`,
`runtime_sha256`, `detector_sha256`, `recognizer_sha256`, and
`build_record_sha256`; `procedure_sha256` maps the root-relative `Cargo.lock`,
`rust-toolchain.toml`, Rust consumer, fixture, runner, this document and generic
process-runner paths to their SHA-256 values. The candidate must be committed and
clean, including
no untracked product/procedure source.

The v3 procedure, role roster, complete bounds and updated procedure hashes require
independent exact-binding review. Construct a fresh authority with its own bounds
object; never mutate or copy authority from a spent v2 attempt. This document
does not grant execution or permission authority.

`native_images` is a finite map of canonical absolute paths to approved SHA-256
values for the exact union of non-system dyld images loaded by all three roles.
It includes the consumer, fixture, ONNX Runtime, OpenCV and their developer-owned
dependencies, with at most 128 entries. Resolve this dependency set during
reviewed preparation, not from the qualification output. Every entry is hashed
before launch and rechecked afterward; the observed set must match exactly.
The two executable and runtime entries must also match their separately approved
artifact hashes. System shared-cache images are excluded from this map and
remain bound to the exact approved OS build. A new or missing image is a failure,
not permission to extend the manifest after seeing the run.
Fix the binary, source, model, runtime, fixture, host, oracle and bounds **before** approving.
A changed candidate or failed first attempt requires new applicability review,
not a rewritten approval or silent retry. No example approval is prefilled.

Only after that authority exists, with Screen Recording already granted to the
actual execution context outside this procedure:

```sh
: "${MADO_PILOT_G004_MODEL_ROOT:?explicit absolute reviewed model root required}"
: "${MADO_PILOT_ONNX_RUNTIME:?explicit absolute reviewed ONNX Runtime 1.29.0 file required}"
: "${OCR_APPLE_AUTHORITY:?fresh reviewed task-7.4 authority required}"
: "${OCR_APPLE_BUILD_RECORD:?private build transcript required}"
: "${OCR_APPLE_EVIDENCE:?new private evidence directory path required}"
python3 tools/setup-native.py -- python3 tools/ocr-text-watch/macos/run.py \
  --consumer "$CONSUMER_APP/Contents/MacOS/ocr-public-consumer" \
  --fixture "$FIXTURE_APP/Contents/MacOS/ocr-owned-fixture" \
  --authority-file "$OCR_APPLE_AUTHORITY" --build-record "$OCR_APPLE_BUILD_RECORD" \
  --evidence "$OCR_APPLE_EVIDENCE"
```

The evidence parent directory must already exist; the run directory must not.
The supervisor preserves all outputs and never removes or replaces an attempt.
A killed child counts as failed cleanup even if OS reclamation succeeds. If the
OS cannot reap an exact owned child within the declared bound, keep the record
failed and report that exact PID; do not run a broad cleanup command.
