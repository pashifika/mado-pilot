# Windows owned WGC OCR procedure

**Task 7.1 source/procedure preparation only. Task 7.2 is not run.** This pass
has no Windows compiler, real-model execution grant, or Windows fixture/capture
execution grant. No build, test, formatter, linter, fixture launch, capture,
permission, focus, or input operation was performed. The commands below are
prospective, not evidence of execution or native support. Existing Rust template
and historical C/C++ results do not qualify this OCR procedure.

## Fixed scope and oracle

One fresh consumer process constructs one explicit
`windows_engine_with_ocr_profile` engine with `OcrProfile::BoundedDetector`,
CPU/no fallback, `MADO_PILOT_G004_MODEL_ROOT`, and
`MADO_PILOT_ONNX_RUNTIME`. Admission must report the complete initialized
`phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v2` descriptor. Unsupported
systems, missing native dependencies, discovery refusal, and native failures
remain refusals; there is no alternate application, provider, permission probe,
elevation, foreground forcing, or input route.

The private [Win32 fixture](../tools/ocr-text-watch/windows/fixture.cpp) uses only
ordinary MSVC/Windows SDK libraries. It is not product capability, a C ABI
consumer, or a modification of existing native-template apparatus. Its owned
`WS_POPUP`/`WS_EX_NOACTIVATE`/`WS_EX_TOOLWINDOW` window is shown with
`SW_SHOWNOACTIVATE`; resize uses `SWP_NOACTIVATE`. Mouse activation is refused.
The pre-launch foreground HWND must remain unchanged at every protocol gate
and throughout the fixture message loop. No API restores or forces foreground.

Name one approved x64 Windows desktop in the authority file, with the actual
build/UBR, machine identity, compiler/SDK, and **primary SDR display** description.
The display's physical work area must fit 1088×704. The PMv2-aware fixture starts
at work-area origin + (32,32), 960×576 physical pixels, and resizes to 1024×640
without moving or scaling the image. The operator must not interact during the
run; focus changes or unexpected publications are retained failures, not retries.
No display-topology, mixed-DPI, HDR, application compatibility, or support-floor
promotion is claimed.

| Fixed input | Identity / expectation |
|---|---|
| Repository PNG | `fixtures/ocr/g-004/hud.png`, 960×540; SHA-256 `10f0163cf298453e55922cc6104ee3066f59a328a55e9bac6c04f24d36c9e288` |
| Prepared raw image | `hud.bgra`, 2,073,600 packed BGRA bytes; SHA-256 `3ca2f418f6fb083e49a679638017609e11fe825ef4a1e6f56dc0e572c74f75e6` |
| Frozen oracle document | `oracle.json` (`ocr-text-watch-replay-v2`); SHA-256 `906f787178acea1e967e3a3989f1e7c50b54b3bb6a1ec41af3e80980128a63bf`; the native path does not consume the replay manifest |
| Initial/negative image | Solid white; same 960×540 OCR region, no text |
| Requested region | Capture pixels `(0,0)-(960,540)`, `ClipPolicy::Reject`; never expanded after resize |
| Output space | Every returned point is `FrameNormalized`; independently check its projection using fixed 960×576 or 1024×640 dimensions, not only a round trip through the reported transform |
| Predicate | Literal `魔導士`, threshold `0.0` inclusive; finite profile confidence in `[0,1]`; evaluated within one region |
| Complete ordered output | `魔導士`, `Lv.42`, `HP1234/5678`, `MP98%`, `クエスト`, `[A-7]`, `次へ>`, `READY!`; exactly eight regions, satisfying indexes `[0]` |
| Selected source box | `(53,60)-(203,112)`; bounding-box IoU ≥0.5, center deltas ≤24 pixels horizontally and ≤13.5 vertically |
| Analysis | Minimum interval 2 seconds, `AnalysisAlways`; immediate first query, three consecutive observations for resize |

Source-owned artwork is Apache-2.0. The rendered Noto Sans JP pixels have OFL-1.1
provenance in the existing fixture manifest; no font bytes or new downloaded
prerequisites are bundled. The common corpus converter is separate preparation;
Pillow is **not** a dependency of this native runner or fixture.

## Controlled transitions and ownership

The [Rust public consumer](../crates/mado-pilot/examples/ocr-text-watch-native-windows.rs)
owns the exact `Child` and checks its PID, immutable HWND, unique class/title,
and discovered facade target. A fresh 128-bit run nonce, command sequence, and
state are painted as 18 grayscale cells outside the OCR ROI, at y=544–551.
This is a private render marker, not OCR text or injected input.

`ACK sequence pid hwnd width height state paint_count` is emitted only after
synchronous `WM_PAINT` completion, successful GDI drawing, `GdiFlush`, and `DwmFlush`.
Acknowledgement is not assumed to be WGC delivery: the consumer separately maps
an exact captured frame, checks every OCR-ROI RGB byte against the immutable
raw input (white for blank), and checks every marker cell. Alpha is not an
oracle because native GDI/WGC alpha representation is not source RGB identity.
A match must expose this same pixel state and the same frame/transform in its
OCR result. Neither a post-open frame alone nor a preexisting text coincidence
gets credit.

1. Start an immediate query while the owned window is blank. Issue a blank
   marker update after open; observe and record its exact source checkpoint and
   accepted zero-confirmation analysis **before** sending `show`. Inspect the
   match against the acknowledged image/marker; require a strictly newer stamp
   in the same stream, epoch, and geometry, exactly eight regions, predicate,
   confidence, geometry, and satisfying indexes. Retain the outcome after the
   query is dropped; cancellation after match must return the same terminal.
2. Publish and observe a newer blank frame while the first result is retained.
   Start the three-observation query, establish its blank checkpoint, show text,
   and observe exactly one accepted positive. Record that accepted source before
   resize. Resize **without changing or blanking the text**. Require a newer
   same-stream source with changed geometry and exactly one accepted successor
   positive, not two: a negative observation cannot supply the reset oracle.
   Two separately acknowledged marker ticks produce counts two then three.
   The final result's first-confirmed source must belong to the successor epoch
   and geometry. Every result uses its own output extent, never old geometry.
3. Observe a newer blank frame, start and observe a pending negative query, then
   command destruction of that exact HWND while its owning process stays alive.
   `IsWindow` must be false and the query must yield `TargetLost`, not timeout,
   `SessionClosed`, or match. Repeated cancel preserves that terminal.
4. Observe physical OCR quiescence separately from logical terminal success.
   Explicitly close the session, drop session and engine, and remap both retained
   source frames. Text, confidence, quadrilaterals, complete source correlation,
   provider facts, and acknowledged pixels must remain unchanged. Release both
   results, then command fixture exit and observe the child and reply reader end.

Checkpoint acquisition is solely fixture/source evidence, not a host OCR polling
implementation. Each checkpoint admits at most 128 strictly newer frames under
one 20-second context. Transitional frames are recorded; mapping/capture errors
are never swallowed or retried. Missing frames, excess/early confirmations,
unsupported resize, mismatched output, deadline, overload, and unexpected
terminals remain mandatory failures of this **fixed** procedure. It does not
quietly accept a smaller scenario subset or switch to a different resize oracle.

## Prospective endpoints and accounting

| Endpoint | Fixed bound |
|---|---|
| Engine initialization | 60 seconds |
| Discover, session open, fixture readiness, each ACK | 10 seconds each |
| Each source checkpoint / analysis progress / caller wait / retained mapping | 20 seconds each |
| Each query lifetime | 90 seconds; independent caller waits cannot extend it |
| Physical OCR quiescence, explicit session close, engine release | 5 seconds each |
| Fixture command input / reply storage | ≤16 commands; ≤31 input bytes/line; 192 reply bytes/line; one queued reply |
| Graceful fixture process exit and reply-reader drain | 5 seconds each, after quit acknowledgement |
| Whole consumer tree | 300 seconds, ≤1 MiB combined stdout/stderr; owned cleanup ≤5 seconds |
| Samples | One process, one fixture, one pass through all scenarios; no warmup/replacement/retry |

The runner reuses the unchanged ordinary native-setup
[`process_runner.py`](../tools/native-release-profile/process_runner.py): its
Windows Job owns the consumer before resume, prohibits breakaway, and owns the
fixture descendant. Only retained child/Job handles are terminated. The fixture
also has a 300-second lifetime ceiling. A stuck native call, synchronous write,
destructor, or reader is contained by the outer Job deadline; forced termination
never counts as graceful cleanup.

Evidence separates **semantic**, **resource**, and **cleanup**. Resource facts
include actual current/peak working set via `GetProcessMemoryInfo`, logical and
physical OCR occupancy, mapping/cache observations, and the actual two retained
source layouts plus result logical extents. Logical quiescence alone is not
physical cleanup. RSS/latency are observations, not accepted G-013 budgets;
no p95, heap, growth, or performance qualification is implied. A semantic match
with failed cleanup remains semantic evidence and overall failure.

## Build and prepare on the approved Windows host — not executed here

Use an ordinary x64 MSVC developer PowerShell with the existing repository
prerequisites from `CONTRIBUTING.md`. The product source must be a clean reviewed
candidate containing this procedure. Use new paths **outside the product tree**
for artifacts, authority, and private evidence. Never reuse a frozen or hash-pinned
Cargo root. These commands neither stage nor commit anything:

```powershell
$work = Join-Path $env:TEMP ('ocr-wgc-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$env:CARGO_TARGET_DIR = Join-Path $work 'cargo-native-example'
$fixture = Join-Path $work 'ocr-text-watch-fixture.exe'
$corpus = Join-Path $work 'corpus'
$env:MADO_PILOT_G004_MODEL_ROOT = 'C:\reviewed\g004-model'
$env:MADO_PILOT_ONNX_RUNTIME = 'C:\reviewed\onnxruntime.dll'
$opencv = 'C:\native\opencv'
$clang = 'C:\Program Files\LLVM\bin'

# Separate immutable pixel preparation; existing pinned Pillow 12.3.0 is needed only here.
python tools/ocr-text-watch/replay_corpus.py --output $corpus
if ($LASTEXITCODE -ne 0) { throw 'corpus preparation failed' }
cl.exe /nologo /std:c++17 /EHsc /W4 /DUNICODE /D_UNICODE tools\ocr-text-watch\windows\fixture.cpp "/Fe:$fixture" "/Fo:$work\fixture.obj" /link user32.lib gdi32.lib dwmapi.lib /SUBSYSTEM:CONSOLE
if ($LASTEXITCODE -ne 0) { throw 'fixture compilation failed' }
python tools/setup-native.py --opencv-root $opencv --libclang-path $clang -- cargo build --locked --target x86_64-pc-windows-msvc --package mado-pilot --example ocr-text-watch-native-windows
if ($LASTEXITCODE -ne 0) { throw 'consumer compilation failed' }
$consumer = Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-msvc\debug\examples\ocr-text-watch-native-windows.exe'
python tools/setup-native.py --opencv-root $opencv --libclang-path $clang -- python tools/ocr-text-watch/windows/run.py --consumer $consumer --fixture $fixture --corpus $corpus --prepare "$work\prospective.json"
if ($LASTEXITCODE -ne 0) { throw 'identity preparation failed' }
```

`--prepare` reads/hashes inputs and runs bounded Git identity commands; it does
not load OCR or launch a fixture/capture session. Review its immutable identities
and complete a **separate** approved file with `authorization` exactly
`owned-fixture-wgc-real-cpu-ocr`, a fresh `authority_id`, the named `host_id`, the
actual `planning_revision`, verbatim successful `build_commands`, and the
approved `display_scope`. Preserve all generated identities/limits and the
original prospective file. Missing authority remains `not-run`; a preparation
file with null authority is intentionally not executable. Record controlled
OpenCV/libclang installation and DLL identities with the build evidence, in
addition to the model/runtime hashes the runner captures automatically.
The child receives the same canonical absolute model/runtime paths that were
hashed; changing its working directory cannot select different prerequisites.

## One authorized execution — task 7.2, not authorized by this document

Only after separate real-CPU OCR and owned Windows fixture/WGC authority:

```powershell
python tools/setup-native.py --opencv-root $opencv --libclang-path $clang -- python tools/ocr-text-watch/windows/run.py --consumer $consumer --fixture $fixture --corpus $corpus --authority "$work\approved.json" --evidence "$work\execution-1"
if ($LASTEXITCODE -ne 0) { throw 'native procedure failed or was refused; retain this attempt without retry' }
```

`before.json`, `consumer.txt`, and `evidence.json` are private requested evidence.
They preserve actual command, source commit/tree, planning/authority identity,
host/build/SDK, executable and script hashes, exact model/runtime paths and file
hashes, nonce/HWND/PID, source checkpoints, recognized output, observed resource
facts, child output/exit, and cleanup. Successful execution rechecks identities
without rebuilding. Ordinary runner output contains only closed status facts;
never publish these private files as ordinary logs. Existing output directories
and files are not overwritten. A failed or partial attempt is never relabeled by
a later run, and host/build success does not inherit any old native qualification.

Main/CI owns later Windows compilation and validation. No manifest change is
needed: Cargo auto-discovers the example and the existing Windows dev-dependency
already selects the required `windows` 0.62.2 SDK bindings. Runtime integration
must provide the agreed `OcrTextQuery::progress`, accepted/source/confirmation
accessors, retained result/index/first-confirmed/provider/extent accessors, and
`Engine::ocr_text_observation`. Native success remains unverified until the
reviewed finite command above actually completes under fresh authority.
