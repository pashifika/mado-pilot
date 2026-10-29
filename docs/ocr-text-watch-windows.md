# Windows owned WGC OCR procedure

**The fixed Windows native procedure passed on its recorded cohort.**
Source `fd98dba26ae5eb3375ab2111ae9f272258e39e99`, with
`benchmark-instrumentation` enabled, passed acknowledged OCR transition,
retained-result producer progress, resize confirmation reset, explicit native
`TargetLost`, and retention after parent close. Resource and cleanup endpoints
also passed. The single execution used Windows build26300.9711 and the selected
secondary DELL `DISPLAY1`, WCG/HDR-user-off,150%, CPU bounded-v2.

At resize, native `ContentSize` was1024×640 while the transition texture was
960×576. That clipped frame was not published as full-size content. A subsequent
1024×640 frame passed the exact source-pixel/marker check in50ms, on the same
public stream with epoch1 and geometry revision1.

This is one instrumented cohort, not a causal fix or repeatability result.
Metadata observation can affect scheduling; feature-off native execution and
broader application/display compatibility are unmeasured. `FrameRequest` waits
for actual publication; it does not generate a successor after `Recreate`.
No automatic restart, repaint, fallback, input or permission change is added.

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
build/UBR, machine identity, compiler/SDK, and an explicitly selected display.
Both preparation and execution require `--display-device` with the exact GDI
device name, such as `\\.\DISPLAY1`. The runner binds that name in
`identities.display_device`; an authority for a different device is refused.
The runner overrides `MADO_PILOT_OCR_WGC_DISPLAY_DEVICE` in the consumer's child
environment. The unchanged Rust consumer inherits it into the fixture.
The PMv2-aware fixture requires exactly one matching `MONITORINFOEXW.szDevice`
from `EnumDisplayMonitors`. Missing, ambiguous, or failed enumeration is a refusal:
there is no primary-monitor or other-display fallback.

The selected display's authoritative physical work area must fit 1088×704.
The fixture starts at that work-area origin + (32,32), 960×576 physical pixels,
and resizes to 1024×640 without moving or scaling the image. Negative desktop
origins are valid. The operator must not interact during the run; focus changes
or unexpected publications are retained failures, not retries.

The new prospective cohort is **DELL U2725QE / `\\.\DISPLAY1` / HDR user-off /
active WCG / 150% / 3840×2160**, with observed physical work area 3840×2088.
It is not the historical primary-SDR cohort, and WCG is not relabeled SDR.
Record current monitor identity, physical work rectangle, scaling, active color
mode and HDR user state in the approved `display_scope`, using separately retained
readiness metadata. The device selector does not certify those properties.
WCG may change captured pixels: the exact RGB, marker, output and geometry
oracles below remain mandatory, with no conversion workaround or threshold
relaxation. No display-topology, mixed-DPI, HDR/WCG, application compatibility,
or support-floor promotion is claimed.

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
   and observe exactly one accepted positive. Freeze its accepted source and
   `Completed=C` from that same progress snapshot before resize. Resize **without
   changing or blanking the text**. Require newer same-stream successor geometry,
   exactly one positive and `Completed=C+1`: a hidden negative or extra accepted
   analysis fails even if it preceded the captured successor checkpoint.
   Two separately acknowledged marker ticks must produce counts two then three,
   with exactly `C+2` then `C+3` completions. Check the frozen terminal counter and
   require `first_confirmed_frame` to equal the observed first successor positive.
   Keep rejected progress in private evidence. Every result uses its own output
   extent, never old geometry.
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

Each checkpoint also writes and flushes one bounded `source_checkpoint` scalar
summary after its acquisition loop returns, including on an API deadline/error.
It records the requested ACK/state/extent and prior stamp; the last phase;
acquisition attempts, successes and errors; successful maps and map errors;
size, expected-image-pixel and marker rejection counts; the last acquired stamp
and extent; the last mapping `(width,height,stride)`; the last pixel-check category;
elapsed milliseconds and matched/failed outcome. Mapping/check observations are
cleared for each newly acquired frame so they never describe a predecessor.
The existing semantic error remains authoritative; a diagnostic write/flush
failure cannot replace it and emits only a closed I/O-error category to stderr.
A diagnostic I/O failure after a successful checkpoint still fails the procedure.

Rejection counts describe the original first-failing check order: extent, exact
960×540 RGB image/blank region, then exact nonce/ACK/state marker. A zero marker
count does not clear the marker if image comparison failed first. No raw pixels,
captured images or recognized text are added to these summaries or ordinary logs.
There is no extra capture/map call, repaint, tick, retry or deadline extension.
An acquisition deadline with zero acquired frames distinguishes absent public
successor delivery from a mapping failure, but cannot by itself identify which
native publication/recreation branch ran. These are private diagnostic facts,
not a resize fix, a relaxed oracle or a Windows qualification claim.

The existing `benchmark-instrumentation` feature also exposes fixed process-wide
callback progress counters through `CaptureMetricsSnapshot`. The private Windows
consumer's existing dev-dependency enables it; no runtime API or Cargo feature
change is required. Each `source_checkpoint` summary includes absolute
`capture_before` (before the checkpoint context) and `capture_after` (after the
loop returns, including errors). The existing `engine_release_ms` record includes
`capture_after_close`, taken after session close and engine drop. The consumer
does not reset counters or introduce another wait, acquisition, map or repaint.

The counters distinguish handler entry, admission refusal, missing weak owner or
sender, transition-lock contention, successful native frame acquisition, resize
detection, actual frame-pool `Recreate` begin/success, geometry-fence refusal,
descriptor/lease/copy refusal, storage-publication attempt/success/refusal and
frame-processing faults. Descriptor, lease and copy refusal each includes both
an error and an unusable/unavailable result; no error is swallowed. A recreate
completion counts the API's success before owner/transition bookkeeping. A
callback fault includes an existing late `SessionClosed` error ignored during
teardown; it is not necessarily a newly terminal session failure.

Callback progress increment hooks perform only one relaxed atomic increment when enabled and
are no-ops with the feature off. They add no allocation, I/O, clock read, lock or
retry and do not change callback admission, pool recreation, frame refusal,
resource ownership or publication decisions. Existing timing instrumentation is
unchanged. Counter increments wrap modulo 2^64; only the existing quiescent
profile reset clears them. These independent atomic loads are **not a coherent
transaction**: concurrent callbacks and other capture sessions in the process
can contribute, and a snapshot may straddle an increment. Post-close does not
make unrelated sessions quiescent. The checkpoint starts after the fixture ACK,
so resize callbacks may already be included in `capture_before`. Keep absolute
observations; do not assert callback-to-ACK identity or exact event ordering from
subtraction, equality between counters, or a zero interval difference.

Interpretation remains hypothesis-driven: H1 predicts completed recreation with
no subsequent callback progress; H2 predicts callback notification lost at
transition-lock contention; H3 predicts later progress ending at an existing
geometry, descriptor, resource/copy or publication refusal. These counters can
narrow those boundaries but do not alone prove which callback caused the missing
public successor. No images, raw pixel values or recognized text are added.

A separate feature-gated resize metadata probe runs after the existing producer
reservation succeeds and before the original transition frame is closed. It
records that frame's native `ContentSize` width/height and, through
`Surface` → `IDirect3DDxgiInterfaceAccess` → `ID3D11Texture2D::GetDesc`, the
texture width/height and raw DXGI format. All temporary COM references are
released before the original frame drop and `Recreate`, on success or failure.
The feature-off build contains neither this probe nor its call. The enabled
probe has a fixed number of metadata calls and scalar atomic writes: no pixel
copy/map/read, new frame acquisition, clock, callback I/O, wait, retry or repaint.
Native metadata calls can still perturb scheduling; no new latency guarantee is
claimed.

`resize_surface_successes` counts completed descriptor reads, not usable images
or oracle passes; `resize_surface_failures` counts diagnostic failures separately
from capture faults. The latest scalar fields are `resize_content_width/height`,
`resize_texture_width/height/format`, `resize_surface_status` and
`resize_surface_hresult`. Status values are 0 unmeasured/reset, 1 descriptor read,
2 `Surface` failure, 3 interface-access cast failure and 4 texture-interface
failure. On failure, texture fields are zeroed and the original failing HRESULT
is recorded; no diagnostic error is propagated into the capture operation.
Success/reset records HRESULT zero. Fields remain process-wide and
nontransactional, so concurrent probes can mix observations; neither status nor
counter equality proves a coherent tuple. Existing checkpoint/close summaries
print these fields without a consumer change.

The distinction matters because [Microsoft's frame-pool guidance](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture)
states that surfaces have the pool allocation size, larger content is clipped,
and `Recreate` discards queued frames. `ContentSize` alone therefore does not
prove that a growth-transition frame contains the full new geometry. A complete
descriptor is only a necessary sizing/format observation, not pixel validity,
freshness or a marker/OCR result. This probe neither preserves/publishes that
frame nor restarts a producer, and changes no existing capture/error/resource
ordering or qualification oracle.

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

## Build and prepare a fresh candidate on the approved Windows host

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
$displayDevice = '\\.\DISPLAY1' # Explicit operator-selected device; never inferred from primary.
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
python tools/setup-native.py --opencv-root $opencv --libclang-path $clang -- python tools/ocr-text-watch/windows/run.py --consumer $consumer --fixture $fixture --corpus $corpus --display-device $displayDevice --prepare "$work\prospective.json"
if ($LASTEXITCODE -ne 0) { throw 'identity preparation failed' }
```

`--prepare` reads/hashes inputs and runs bounded Git identity commands; it does
not load OCR or launch a fixture/capture session. Review its immutable identities
and complete a **separate** approved file with `authorization` exactly
`owned-fixture-wgc-real-cpu-ocr`, a fresh `authority_id`, the named `host_id`, the
actual `planning_revision`, verbatim successful `build_commands`, and the
approved `display_scope` describing the same selected device and current cohort.
Preserve `identities.display_device`, all other generated identities/limits, and
the original prospective file. A missing selector, a different selector at
execution, or an old authority lacking the selected-device binding is not
compatible with this procedure. Missing authority remains `not-run`; a
preparation file with null authority is intentionally not executable. Record
controlled OpenCV/libclang installation and DLL identities with the build
evidence, in addition to the model/runtime hashes the runner captures automatically.
The child receives the same canonical absolute model/runtime paths that were
hashed; changing its working directory cannot select different prerequisites.

## One authorized execution — task 7.2, not authorized by this document

Only after separate real-CPU OCR and owned Windows fixture/WGC authority:

```powershell
python tools/setup-native.py --opencv-root $opencv --libclang-path $clang -- python tools/ocr-text-watch/windows/run.py --consumer $consumer --fixture $fixture --corpus $corpus --display-device $displayDevice --authority "$work\approved.json" --evidence "$work\execution-1"
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

Cargo auto-discovers the example; the existing Windows dev-dependency supplies
the required `windows` 0.62.2 SDK bindings. The compiled runtime provides
`OcrTextQuery::progress`, accepted/source/confirmation
accessors, retained result/index/first-confirmed/provider/extent accessors, and
`Engine::ocr_text_observation`. Native success remains unverified until the
reviewed finite command above actually completes under fresh authority.
