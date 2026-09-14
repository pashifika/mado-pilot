# Private Apple Silicon OCR text-watch procedure

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

The prospective `ocr-text-watch-apple-v2` channel contract follows
[ADR 0076](adr/0076-bound-native-ocr-evidence-channels.md). Its
[verification obligations](adr/0076-bound-native-ocr-evidence-channels.md#verification)
must pass before native admission. The inert pre-main loader probe is apparatus
evidence only, not an OCR result or a failed native qualification attempt.

## Scope and fixed oracle

One freshly approved Apple Silicon interactive desktop, its main display at
**2x**, one private AppKit window, one CPU bounded-v2 initialization, one consumer
process and one fixture process; no warmups or retries. `run.py` requires the
exact approved uname array (including node), OS version/build, CPU, SDK, source and executable identities. It
does not inherit another campaign's host approval or promise support beyond the
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

`tools/ocr-text-watch/macos/run.py` owns exactly two direct native `Popen`
children: consumer first, then fixture only after the consumer reports an
already-granted non-prompting Screen Recording decision. Their owned PIDs remain
the actual executable PIDs, not wrappers or collectors. It uses the existing
`tools/native-release-profile/process_runner.py` unchanged for bounded provenance
commands, and a small private supervisor for the two interacting children.
It does not invoke a shell, LaunchServices, `open`, process-name kill, or a foreign
controller. No child spawns another process. A timeout stops only those exact
owned children and remains a failure.

## Ordered scenario

The private directory is newly created with mode 0700, files under umask 077.
A single atomic command file carries `<nonce> <sequence> <operation>`; the fixture
accepts each sequence once. Acknowledgements carry nonce, sequence, operation,
owned PID/window number, logical size and rendered scale, at most 512 bytes.
Commands are at most 256 bytes. Stale acknowledgements are observed, not resent.
The 64-bit per-run nonce and 32-bit state are drawn as blue/yellow cells; capture
must contain those exact marker bits. Unique title selection only locates a
candidate: the retained public target/stream plus the pixel challenge establish
the owned source. No later discovery retargets the session.

| Sequence | Operation / mandatory observation |
|---|---|
| 0 | Start fully white, with ownership marker only. Ready follows AppKit display processing and pending CATransaction submission. |
| 1 | After opening and starting a two-confirmation query, `arm-blank` changes only the marker. Capture and verify the entirely white OCR ROI; record its exact source checkpoint. A separately bounded 100ms wait times out without cancelling the query. |
| 2–3 | `show` renders the fixed image. Wait for the first accepted positive (or an already committed match), then `pulse` changes only the marker. Require two distinct positive sources, both newer than checkpoint 1 in the same epoch/geometry. The retained matching pixels must carry state 2 or 3. |
| 4 | `blank`; while retaining the result, a separately cloned exact frame and its mapping, require a strictly newer state-4 frame. This is the producer-progress oracle, not just a post-close read. |
| 5–7 | Use one consecutive(2) query throughout. State 5 must establish exactly one accepted positive with no pending/logical/physical work; freeze `Completed=N`. `resize-show` preserves the identical positive ROI while changing to 520x300 logical / 1040x600 capture. Inspect the first changed completion snapshot: require exactly `N+1`, count 1, and an accepted successor epoch/geometry. After capturing state 6, recheck that no extra analysis occurred. State-7 `pulse` completes the same query; require exactly `N+2`, count 2, `first_confirmed_frame` equal to the observed successor, a newer final frame in that successor, and captured state 7. No blank reset or fresh-query substitution is accepted. |
| 8–9 | Establish blank at the larger size; start ROI `(490,280)-(510,290)` with Reject. `shrink-blank` changes to 240x140 logical. Require successor geometry and query `Failed(InvalidArgument)`, never a match. |
| 10 | Start a pending query on blank ROI `(0,0)-(100,100)`, then `close-window`. The fixture process remains alive. Require exactly `TargetLost`; a timeout, `SessionClosed`, stop or absent-source observation is not relabelled. |
| 11 | Close the session, establish zero logical/physical OCR work and mapping reservations/barrier, drop session/engine, then remap/read the original retained result and compare every mapped byte. Release retained owners and acknowledge fixture exit. The supervisor independently requires both children to exit and be reaped. |

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
mapping, session close and physical drain 5s; semantic wait 15s; query lifetime
100s; consumer wall clock 200s; fixture independent fuse 210s. Child exit gets 5s
grace, then 2s termination and 2s kill/reap. No close retry. Core dumps remain
disabled with `RLIMIT_CORE=0`; inherited `RLIMIT_FSIZE` is unchanged, never raised
or replaced with an evidence-channel limit. This is not a sandbox for arbitrary
native regular-file writes. Provenance subprocesses retain their own 10s/64 KiB
limits (the bounded source diff permits 2 MiB).

The complete authority `bounds` must equal `run.py::BOUNDS`, including these
byte limits; process counts remain one consumer and one fixture, with zero
warmups and one sample:

| `BOUNDS` key | Bytes | Scope |
|---|---|---|
| `each_output_file_bytes` | `65536` | Each ordinary stdout/stderr file; the existing 64 KiB `consumer.report` admission limit also remains. |
| `total_output_bytes` | `262144` | All ordinary/control/report output, including `record.json`; exclude only the two named native-image files below. |
| `each_native_image_file_bytes` | `1048576` | Each separate native-image file. |
| `total_native_image_bytes` | `2097152` | Both native-image files together. |

Each child has separate anonymous stdout, stderr and native-image pipes, drained
with nonblocking parent reads into exclusive mode-0600 private files.
`DYLD_PRINT_LIBRARIES=1` and `DYLD_PRINT_TO_FILE=/dev/fd/N` direct loader output
to that child's inherited image descriptor. `consumer.native-images` and
`fixture.native-images` contain the complete loader streams, including system
images; neither consumes the ordinary-output budget. No other file is excluded.
This extends [ADR 0074](adr/0074-isolate-darwin-loader-image-budget.md)'s pipe
mechanism inside the existing two-child supervisor, without importing its
replay collector topology or changing replay evidence.

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
work observations, actual retained/mapped extents and row verdicts. `record.json`
uses `procedure: "ocr-text-watch-apple-v2"` and adds exact commands/outputs,
host, approval/build-record hashes, clean Git head/tree,
consumer/fixture/model/runtime/source hashes, valid signing metadata, actual
dyld-loaded image paths/hashes, exit status and cleanup facts. Private channel
facts record observed/retained bytes, EOF/completeness and failures without
replacing the existing semantic/resource/cleanup/status fields.
Only complete `consumer.native-images` and `fixture.native-images` supply
dependency-image records. Strict decoding and row parsing reject malformed
records or an incomplete final row; ordinary stderr is never an image fallback.
The exact owned-PID `move loaded to delayed: <basename>` diagnostic is retained
but supplies no image identity; other non-image syntax is rejected.
The bounded approved canonical non-system manifest is checked before launch,
against the exact observed union after both channels finish, and against disk
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

## Fresh authority and bounded execution

`OCR_APPLE_AUTHORITY` must name an independently reviewed private JSON document,
not a generated consent flag. Required fields: `approved: true`, `task: "7.4"`,
`target: "aarch64-apple-darwin"`; `host` with the complete recorded `uname` array
(`system`, `node`, `release`, `version`, `machine`, `processor`), the lower-case
`machine_uuid` from `IOPlatformUUID`, and exact `product_version`, `build`, `cpu`,
`sdk`; the exact `BOUNDS` object. The bounded read-only I/O Registry identity
stays in private evidence; it is neither a permission probe nor a permission grant.
`source_head`, `source_tree`, `consumer_sha256`, `fixture_sha256`,
`runtime_sha256`, `detector_sha256`, `recognizer_sha256`, and
`build_record_sha256`; `procedure_sha256` maps the root-relative `Cargo.lock`,
`rust-toolchain.toml`, Rust consumer, fixture, runner, this document and generic
process-runner paths to their SHA-256 values. The candidate must be committed
and clean, including no untracked product/procedure source.

The v2 bounds and updated procedure hashes require independent exact-binding
review; this document does not grant execution or permission authority.

`native_images` is a finite map of canonical absolute paths to approved SHA-256
values for the exact union of non-system dyld images loaded by both children.
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
