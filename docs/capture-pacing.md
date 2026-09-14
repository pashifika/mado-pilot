# Native capture pacing and completion-paced OCR

Rust callers can select an engine default, override it per session, and inspect
what native configuration was established. The separate caller-side OCR example
waits a full cooldown after recognition and interpretation finish.

Both native adapters and the caller flow are implemented. Deterministic checks,
configuration seams and compilation are not native cadence or performance
qualification. The new both-host capture/model comparison remains unexecuted.
Existing OS support, deployment floors and historical qualification are unchanged.

## Engine and session configuration

Inside a function returning `mado_pilot::Result<()>`, select the interval and policy together:

```rust
use std::time::Duration;
use mado_pilot::{CapturePacingRequest, NativeEngineRequest, OpenRequest};

let request = NativeEngineRequest::new().with_capture_pacing(
    CapturePacingRequest::required(Duration::from_millis(200))?,
);

#[cfg(windows)]
let request = request.with_windows_config(
    mado_pilot::WindowsConfig::new().with_capture_pacing(
        CapturePacingRequest::preferred(Duration::from_millis(100))?,
    ),
);
#[cfg(target_os = "macos")]
let request = request.with_macos_config(
    mado_pilot::MacosConfig::new().with_capture_pacing(
        CapturePacingRequest::preferred(Duration::from_millis(100))?,
    ),
);

// Pass request to the target's existing native engine constructor.
// An explicit session value replaces the entire engine default.
let open = OpenRequest::new().with_capture_pacing(
    CapturePacingRequest::preferred(Duration::from_millis(60))?,
);
```

Resolution is **session > selected OS block > common engine > source default**.
The interval and required/preferred policy never merge across layers. These are
defaults, not hard limits: a session preference can replace an engine requirement.

- `CapturePacingRequest::inherit()` removes that layer's decision.
- `CapturePacingRequest::source_default()` explicitly bypasses lower layers and
  leaves native cadence untouched. It does not mean unlimited FPS.
- Repeating a setter replaces its previous value. Replacing an OS configuration
  replaces the whole block; a fresh inheriting block exposes the common default.
- Setter order across different layers does not change precedence.

All four Windows and all four macOS engine constructors use this selection,
including default OCR, explicit OCR profile and provider-policy variants.
Configuration performs no discovery, permission probe, capture or input opening.
Providers retain immutable defaults; an override or close in one session does not
change another session or engine. Existing constructors and omitted options keep
source-default behavior. `OpenRequest` remains a `Copy` value with const construction.

Zero is `InvalidArgument`. Only the selected duration receives native range
validation; an overridden large common value cannot invalidate a smaller OS value.
Selected engine overflow fails before backend initialization, and selected session
overflow fails before native capture allocation. A preference does not excuse an
invalid duration. Existing interruption admission still takes precedence.

## Inspect the configured outcome

After a successful open, `session.description().capture_pacing()` returns an
immutable `CapturePacingReport`. Use `request()`, `configured_interval()` and
`outcome()` rather than estimating a default FPS.

| `CapturePacingOutcome` | Meaning |
|---|---|
| `SourceDefault` | Native cadence was left untouched; no numerical interval is claimed. |
| `Applied` | Native configuration established the resolved request. The configured interval includes any upward representation adjustment. |
| `PreferredUnapplied(reason)` | Only a preference may open unpaced after verified capability absence. The request is retained and the configured interval is absent. |

`PacingUnsupportedReason` distinguishes `SourceCannotPace` from
`NativeControlUnavailable`. Required unavailable control returns `Unsupported`,
not a successful unapplied report. Permission, target, device, configuration,
exception, start, cancellation and deadline failures are not preferred fallback.
A readback shorter than the requested minimum is a configuration failure.
Success is published only after native start and operation commit.

Platform behavior is explicit:

| Source | Explicit minimum interval |
|---|---|
| Windows WGC | Runtime interface/property negotiation; `ceil(nanoseconds / 100)` positive signed 64-bit `TimeSpan` ticks. The last representable interval is `922337203685s + 477580700ns`. Configuration/readback precede callback registration and start. Pool recreation retains the capture session. |
| macOS ScreenCaptureKit | Dynamic getter/setter negotiation; exact positive signed 64-bit nanoseconds at `CMTime` timescale `1000000000`. The maximum is `9223372036s + 854775807ns`. Configuration/readback precede stream creation/start; retained configuration preserves the interval through dimension updates. |
| Replay / ControlledCapture | Required pacing is unsupported; preferred pacing is reported unapplied. Replay stays pull-driven and controlled publication stays caller-driven. |

The macOS scalar handshake is private ABI22; it is not the released C ABI. Public
C/C++ structures, table prefixes and default projections do not gain pacing options.
No sleep, software drop gate, queue-depth change or producer-buffer retention is
used to imitate native pacing.

## Run the caller-owned OCR flow

The complete program is
[`completion-paced-ocr.rs`](../crates/mado-pilot/examples/completion-paced-ocr.rs).
Its real flow uses only the public facade; private support drives the model-free
smoke and tests through the same consumption function.

A safe deterministic run opens no desktop capture, loads no model, and sends no input:

```sh
python3 tools/setup-native.py -- cargo run --locked -p mado-pilot \
  --example completion-paced-ocr -- --controlled-smoke
```

For Windows, use the documented x64 MSVC setup form with explicit OpenCV/libclang
roots around the same Cargo command. See [native setup](../CONTRIBUTING.md#native-development-prerequisites).

Native execution is a separate, explicit action. After authorizing the target and
reviewing the G-004 model/runtime paths, the argument order is:

```text
completion-paced-ocr --native <window|display> <exact-target-name> <model-root> <runtime-file> <source-default|required:MS|preferred:MS> <cooldown-ms> <operation-ms> <exact-stop-text>
```

For example, a separately authorized macOS invocation is:

```sh
python3 tools/setup-native.py -- cargo run --locked -p mado-pilot \
  --example completion-paced-ocr -- --native window 'Authorized OCR fixture' \
  /reviewed/models /reviewed/libonnxruntime.dylib required:100 250 60000 READY
```

Windows uses its reviewed runtime DLL path. Selection must match exactly one
kind/name in discovery; there is no first-target, substring or foreground fallback.
The issued target identity governs subsequent operations. Nothing runs without an
explicit mode. Input and permission requests are absent. Ordinary output contains
no target name, recognized text, pixels or model paths.

One absolute operation context covers setup and every iteration:

1. Acquire the initial latest frame without a preliminary cooldown.
2. Recognize that exact frame and finish bounded caller interpretation.
3. If interpretation stops, return without another cooldown or acquisition.
4. Otherwise release unnecessary result/frame/view/mapping owners, retaining the
   complete checked stamp, then wait the full additional cooldown.
5. Acquire `FrameRequest::newer_than(last_checked)` and repeat.

A long OCR or interpretation step never consumes the additional cooldown. The
local helper uses the operation clock and at most 2ms requested sleep slices,
clipped by the cooldown and remaining deadline. It checks interruption before
waiting and at completion; cancellation retains precedence over deadline expiry.
Polling is cooperative: OS wakeup delay is additional, and arbitrary caller code
or native OCR cannot be physically preempted by this helper. Callers embedding the
loop can attach their cancellation token to the same `OperationContext`.

Cleanup uses an independent, bounded five-second authority in the same clock
domain. Work and cleanup failures are both retained. Logical close does not prove
physical termination of native backend work.

## Freshness, geometry and terminal limits

A publication received during OCR or cooldown remains eligible when the live
source then stays idle. Its capture time need not follow cooldown completion.
Bursts collapse to the newest eligible observation, not a FIFO backlog. Sequence
gaps and identical pixels under a newer same-stream stamp are valid. An unchanged
stamp waits under the operation deadline instead of triggering repeated OCR.

Ordering uses the complete same-stream epoch/sequence contract. Recognition uses
each successor frame's own transform; an old invalid pixel ROI receives the
existing typed geometry failure. These are complete pixel observations, not codec
I-frames.

Actual terminal state precedes retained latest state during acquisition. `Closed`
and `TargetLost` are preserved; there is no post-terminal final-frame drain or
parsing of diagnostic text to distinguish closed causes. Capture loss during
already-admitted one-shot OCR may still allow its exact held-frame result to
commit. That result does not establish that the target is still live; the next
acquisition observes loss. Session close has its existing stronger commit fence.

## Performance and verification boundary

Native capture pacing, caller completion cooldown and watcher backend-admission
rate are three distinct controls. Neither this example nor this configuration
changes `OcrTextAnalysisRate` or template-watcher scheduling.

Longer intervals can miss transient visual states and increase detection latency.
A configured interval is not measured FPS, manufactured periodic output,
proportional CPU/GPU savings or a promise of unchanged application frame times.

Before a native comparison, bind the exact owned workload, hosts, source/binaries,
native dependencies, intervals, duration, cleanup bounds, metric acquisition and
pass/fail budgets under a separate execution grant. Compare capture-off,
source-default, cooldown-only, native-only and combined cases. Separate callback/
copy work, OCR admissions/time, frame age/detection latency, scoped memory,
CPU/GPU and application frame times. Unavailable metrics are unavailable, not zero.
No historical ceiling or failed campaign is rewritten by this feature.

Local deterministic checks cover precedence/report invariants, replay/controlled
behavior, completion timing/freshness/ownership/interruption, and the actual
call-local macOS configuration/start/resize/close seam plus C/Rust layout
agreement. Windows cross-compilation checks types and cfg, not native execution.
Both hosted target jobs run the deterministic example; native pacing qualification
and the five-case performance comparison remain separate incomplete gates.
See [ADR0077](adr/0077-native-capture-pacing-and-completion-cooldown.md).

## Owned native verification apparatus

[`capture-pacing-native.rs`](../crates/mado-pilot/examples/capture-pacing-native.rs)
and [`tools/capture-pacing/run.py`](../tools/capture-pacing/run.py) are private
verification tools, not a second application API. They reuse the caller example's
exact-frame recognition and full-cooldown helper. Hosted CI runs only model-free
checks and compiles the GDI/AppKit fixtures; it does not launch this campaign.

Prepare new consumer/fixture binaries in separate `.rasen/` build directories.
Never rebuild a hash-bound candidate in place. The run supervisor never builds,
downloads, changes permissions, injects input, activates a window or retries.
Its six fixed cases are `semantic`, `capture-off`, `baseline`, `cooldown-only`,
`native-only`, and `combined`; `baseline` uses native source-default pacing.

Before approving a run:

- Obtain `python3 -B tools/capture-pacing/run.py --host-snapshot` on that host.
  This reads OS/build, architecture, CPU, memory, processor count and a hashed
  machine identity; it performs no discovery, capture or model initialization.
- Bind source commit/tree and canonical project root, that exact host object,
  consumer, fixture, CPU runtime, G-004 detector/recognizer, and fixture image.
  Each file binding has `path`, `bytes`, and `sha256`. Windows uses the verified
  raw BGRA conversion of G-004 `hud.png`; macOS uses the PNG.
- Bind `native_inspector` (`dumpbin` or `otool`) and `native_inputs`, including
  transitive non-OS imports. Use explicit canonical `loader_directories` with
  only bound DLL/dylib candidates. The supervisor also checks candidates beside
  the executables/runtime and, on Windows, in the working directory.
  This is static import and lookup-path evidence, **not actual loaded-image
  enumeration**.
- Set authority schema1, purpose `capture-pacing-native`, `approved: true`,
  target, `attempts: 1`, the exact ordered `cases`, six distinct nonzero
  16-digit hexadecimal `nonces`, and a nonexistent `.rasen/` `output_root`.
  Set `native_interval_ns: 100000000`, `cooldown_ns: 250000000`,
  `warmup_seconds: 2`, `measurement_seconds: 6`, `model_root`,
  `input_authorized: false`, and `permission_changes_authorized: false`.

Run only the reviewed authority:

```sh
python3 -B tools/capture-pacing/run.py --authority .rasen/approved-pacing-run.json
```

The fixture is an owned nonactivating child identified by exact title, PID and
nonce pixels outside the OCR ROI. Every captured marker bit is checked before
OCR. Windows requires the fixture's physical geometry to fit the work area;
macOS requires a 2x main screen. Unsupported geometry or changed foreground
state fails rather than moving focus or changing display settings.
macOS probes current Screen Recording access before starting the fixture or
CPU model; denial is `not-run`, not successful zero-resource measurement.

The native idle/burst oracle authenticates the final publication, releases its
frame/mapping owners, waits the full cooldown, then validates the **first**
acquisition against that witness. It cannot drain old frames until a desired
answer appears. Publication specifically during OCR/cooldown and coalescing
remain deterministic controlled-source proof. Native static comparison preserves
the OCR ROI, not the whole frame: the private footer marker changes.

Every comparison initializes the same CPU model, including capture-off. After
two seconds of warmup, measurement admits work for nominally six seconds and
finishes the admitted OCR/interpretation/full cooldown. Rates use actual elapsed
time. Consumer deadlines are 180 seconds for semantic and 120 for comparison,
with a separate ten-second outer cleanup allowance and a 1MiB output cap.
Startup is limited to 60 seconds, process peak RSS to 2GiB, fixture peak RSS to
256MiB, and OCR p95 to five seconds. A padded mapping is limited to 8MiB;
distinct retained logical layouts to 24MiB in semantic and 8MiB in comparison.
Sample capacities are 1024 consumer / 2048 process sampler / 4096 fixture.
Capacity failures and fixture render-sample loss prevent qualification.
Capturing comparisons require three committed OCR samples; capture-off admits none.

`records.json` retains actual process, consumer, fixture and cleanup evidence.
`run.json` distinguishes observed failure, incomplete evidence, unexecuted work
and success. First non-pass stops later cases without fabricating samples.
Missing optional CPU/GPU/copy metrics withhold those savings claims; missing
hard-gate memory evidence cannot establish qualification. macOS callback counts
cover closed-session lifetimes, not just the measurement window. Fixture draw
intervals are application-requested work, not display FPS or general game
performance. Source, host and bound native inputs are checked again after the run.

Consumer and aggregate report schema 2 use `missed_poll_deadlines` for process
polls delayed across their nominal 100ms deadlines. The count and maximum gap
remain visible. A delayed poll is not a dropped stored observation: cumulative
CPU, OS peak RSS and coherent copy intervals retain their own validity checks.
Do not infer ideal polling coverage or continuous instantaneous peaks from
sampled maxima. Required memory evidence and all other hard gates still apply.
The authority and fixture-stat schemas remain version 1.

Schema-one reports and their failures are immutable and are not rescored by the
new analyzer. The corrected accounting and complete-frame Windows presentation
belong to separately bound new runs; they do not change historical product
budgets. See the schema-two apparatus decision in [ADR0077](adr/0077-native-capture-pacing-and-completion-cooldown.md#qualification-apparatus-correction-report-schema-2).

Protocol revision 3 also keeps the macOS fixture's preexisting `Prohibited`
activation policy rather than promoting a CLI process to `Accessory`. Marker
decoding requires a 96-channel blue/yellow dominance margin, not exact primary
colors, and still checks all ownership bits. On Windows, resize acknowledges the
last of four bounded renders, with at least 125ms between each completion and the
next render, so capture can pass through pool recreation. The resize case does not claim a solitary final repaint remains
available when no subsequent publication occurs.
