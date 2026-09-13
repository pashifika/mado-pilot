# Private Apple Silicon OCR text-watch procedure

**Task 7.3 preparation only. Task 7.4 is not run.** No signing, model inference,
fixture launch, capture, permission change, focus action, or input action has run.
Main's first strict compile-only attempt failed on deprecated `[self.window flushWindow]`;
that call is removed without warning suppression. A later Rust example compile-only
check passed with two unreachable match-arm and three helper-visibility warnings;
those sources are corrected without suppression. The latest resize-oracle delta
has not been rebuilt by this worker.
Source review or compilation is not native qualification; native support and A9 remain open.
The frozen foreign/native-template controllers, evidence, grants, and binaries
are neither consumed nor changed.

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

`tools/ocr-text-watch/macos/run.py` owns exactly the consumer and fixture children.
It launches the consumer first, then the fixture only after the consumer reports
an already-granted non-prompting Screen Recording decision. It uses the existing
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
grace, then 2s termination and 2s kill/reap. No close retry. Child output files
are individually hard-capped at 64 KiB with `RLIMIT_FSIZE`, total private live
output at 256 KiB; core dumps are disabled. Provenance subprocesses have their
own 10s/64 KiB limits (the bounded source diff permits 2 MiB). These are safety
endpoints, **not** accepted task-8 latency/RSS budgets.

| Observation | Classification |
|---|---|
| `Granted` | May attempt capture, not a success result. |
| `NotGranted` | `denied-or-undetermined`, not-run. The public non-prompting boolean cannot distinguish an explicit TCC denial from no decision; no private TCC database is consulted. |
| `Unknown` | `undetermined`, not-run. |
| `Unavailable` / unsupported host or adapter | Explicit unsupported prerequisite/outcome; never pass/skip inflation. |
| No owned source/checkpoint or no acknowledged text frame by its bound | Producer-progress/semantic failure once native execution starts, not a skip. |
| Correct match, failed close/drain/exit | Semantic success may remain recorded; resource/cleanup fail independently; overall does not pass. |
| Unreached scenario | `not-run`; earlier executed failures remain failed. |

Private `consumer.report` contains source stamps, backend/provider descriptors,
work observations, actual retained/mapped extents and row verdicts. `record.json`
adds exact commands/outputs, host, approval/build-record hashes, clean Git head/tree,
consumer/fixture/model/runtime/source hashes, valid signing metadata, actual
dyld-loaded image paths/hashes, exit status and cleanup facts. A bounded approved
non-system image manifest is checked before launch and against the exact observed
image set after exit. System shared-cache libraries bind to OS build. Missing,
additional or changed developer-owned images fail resource admission.
Ordinary output contains only closed verdict/reason labels, never
recognized text, pixels/hashes, model/runtime paths or signing identifiers.
No RSS ceiling, native thread-unload fence, arbitrary retained-clone memory bound,
or task-8 qualification is claimed.

## Explicit later build steps — not executed

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

## Fresh authority and later execution — not authorized here

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
