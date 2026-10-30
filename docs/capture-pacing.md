# Native capture pacing and completion-paced OCR

Rust callers can configure a default native capture interval on the engine,
override it per session, and inspect the applied native outcome via
`session.description().capture_pacing()`. The companion
[`completion-paced-ocr.rs`](../crates/mado-pilot/examples/completion-paced-ocr.rs)
example demonstrates waiting a full caller-owned cooldown after one-shot OCR and
result interpretation complete.

## Engine and session configuration

Configure pacing intervals and policy (`required` or `preferred`) before opening
a session:

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

let open = OpenRequest::new().with_capture_pacing(
    CapturePacingRequest::preferred(Duration::from_millis(60))?,
);
```

Resolution rules:

- **Precedence:** `session > selected OS block (WindowsConfig / MacosConfig) > common engine > source default`.
  Interval and policy (`required` vs `preferred`) resolve as one whole unit and
  never merge across layers.
- **Reset and inheritance:** `CapturePacingRequest::inherit()` clears that
  layer's override; `CapturePacingRequest::source_default()` explicitly leaves
  native cadence untouched.
- **Validation:** Zero duration returns `InvalidArgument`. Only the winning
  layer's duration is range-checked against the target platform's native
  representation (engine defaults before backend initialization; session
  overrides before native session allocation).

## Inspect the configured outcome

`session.description().capture_pacing()` returns an immutable
`CapturePacingReport` (`request()`, `configured_interval()`, `outcome()`):

| `CapturePacingOutcome` | Meaning |
|---|---|
| `SourceDefault` | Native cadence was left untouched. |
| `Applied` | Native pacing was applied; `configured_interval()` includes any upward platform tick rounding. |
| `PreferredUnapplied(reason)` | Preferred pacing opened unpaced because the source or OS lacks pacing support (`PacingUnsupportedReason::SourceCannotPace` or `NativeControlUnavailable`). |

Required pacing fails with `Unsupported` when native control is unavailable.
Platform application boundaries:

| Source | Native representation and range |
|---|---|
| Windows WGC | Negotiates `IGraphicsCaptureSession5`, configures `ceil(ns / 100)` positive signed 64-bit `TimeSpan` ticks (max `922,337,203,685 s + 477,580,700 ns`), and reads back before callback registration and start. Retained across pool recreation. |
| macOS ScreenCaptureKit | Negotiates dynamic getter/setter over internal shim ABI 26, configures exact positive signed 64-bit nanoseconds at `CMTime` timescale `1_000_000_000` (max `9,223,372,036 s + 854,775,807 ns`), and reads back before stream start. Retained across dimension updates. |
| Replay / `ControlledCapture` | Required pacing returns `Unsupported`; preferred pacing reports `PreferredUnapplied(SourceCannotPace)`. |

Public C/C++ ABI 1.5 structures and function tables are unchanged, and no
callback sleep or software frame-drop gate is used to imitate native pacing.

## Run the caller-owned OCR flow

Run the deterministic model-free smoke check (opens no desktop capture, loads no
model, and sends no input; see
[native development prerequisites](../CONTRIBUTING.md#native-development-prerequisites)
for the Windows setup wrapper):

```sh
python3 tools/setup-native.py -- cargo run --locked -p mado-pilot \
  --example completion-paced-ocr -- --controlled-smoke
```

For an authorized live target with reviewed `G-004` model and ONNX Runtime paths
(using a dedicated `CARGO_TARGET_DIR`):

```sh
python3 tools/setup-native.py -- cargo run --locked -p mado-pilot \
  --example completion-paced-ocr -- --native window 'Authorized OCR fixture' \
  /reviewed/models /reviewed/libonnxruntime.dylib required:100 250 60000 READY
```

Under one shared `OperationContext` (plus an independent 5-second cleanup budget),
each iteration:

1. Acquires the initial latest frame (or `FrameRequest::newer_than(last_checked)`
   on subsequent iterations).
2. Runs one-shot OCR on that exact frame and completes caller interpretation.
3. Returns immediately if the stop literal matched; otherwise drops frame/result
   owners and waits the full post-completion cooldown (`<= 2 ms` cooperative
   polling slices) before step 1 repeats.

## Freshness, geometry and terminal limits

Frames published during OCR or cooldown remain eligible if the source then stays
idle; bursts collapse to the newest frame (`LatestWins`), and unchanged stamps
wait under the operation deadline without re-running OCR. Terminal capture states
(`Closed`, `TargetLost`) take precedence over retained latest frames and abort
in-flight one-shot OCR commitment before interpretation or cooldown runs.

## Performance and verification boundary

Native capture pacing (producer cadence), caller completion cooldown (post-OCR
wait), and `OcrTextAnalysisRate` (watcher scheduler admission) are independent
controls. Configured intervals do not guarantee observed FPS or application
frame-time reductions.

## Owned native verification apparatus

[`crates/mado-pilot/examples/capture-pacing-native.rs`](../crates/mado-pilot/examples/capture-pacing-native.rs)
and [`tools/capture-pacing/run.py`](../tools/capture-pacing/run.py) execute the
private 6-case verification suite (`semantic`, `capture-off`, `baseline`,
`cooldown-only`, `native-only`, `combined`) against an owned non-activating
fixture (`100 ms` native pacing, `250 ms` cooldown, `2 s` warm-up, `6 s` nominal
measurement; `180 s` semantic / `120 s` comparison deadlines + `10 s` cleanup;
`<= 60 s` startup, `<= 2 GiB` consumer peak RSS, `<= 256 MiB` fixture peak RSS,
`<= 5 s` OCR p95, `<= 8 MiB` padded mapping):

```sh
python3 -B tools/capture-pacing/run.py --host-snapshot
python3 -B tools/capture-pacing/run.py --authority .rasen/approved-pacing-run.json
python3 -B tools/capture-pacing/report.py \
  --input .rasen/approved-pacing-run/records.json \
  --run-receipt .rasen/approved-pacing-run/run.json \
  --output .rasen/approved-pacing-report.json
```

Runner receipts and derived reports use schema 3 (with consumer schema 2
recording `missed_poll_deadlines` separately from dropped samples, and authority
schema 1 binding exact source, host, binary, model/runtime, and static loader
inputs; offline reporter exit codes are `0` = `pass`, `1` = published non-pass
or `legacy-inspection`, `2` = input/alias/publication refusal).

## Accepted owned-workload qualification

Both release targets passed the `semantic` case and all five comparison cases on
owned fixtures without input injection or permission changes:

- **Windows (`x86_64-pc-windows-msvc`):** Windows 11 Pro 25H2 (`26200.9445`),
  Core i7-12700KF; binaries and supervisor at
  `2db68d000c595ed7deb37153424579ab23b363f9` (protocol rev 3; raw archive SHA-256
  `a375ef9c77b7e6fe82309d32b45d6a9efbd315d2b35acd445dcb009d4463e025`).
- **macOS (`aarch64-apple-darwin`):** macOS `26.6.2` (`25G83`), Apple M1 Pro, 2×
  display; binaries at `2db68d000c595ed7deb37153424579ab23b363f9`,
  supervisor/reporting at `01adac8387e80e0471200ec2ea93377491093fd9` (protocol
  rev 4; raw archive SHA-256
  `91708052834e432dae32f9126f1cc0d66c16231c9f415165677f8f5cc7a60510`).

| Host | Case | CPU cores | Peak RSS MiB | OCR/s | OCR p95 ms | Native copy MiB/s |
|---|---|---:|---:|---:|---:|---:|
| Windows | `capture-off` | 0.0286 | 104.68 | 0 | — | — |
| Windows | `baseline` | 1.0629 | 254.54 | 1.417 | 724.622 | 124.231 |
| Windows | `cooldown-only` | 0.7756 | 254.04 | 1.043 | 709.617 | 124.514 |
| Windows | `native-only` | 1.0565 | 253.59 | 1.428 | 711.985 | 20.757 |
| Windows | `combined` | 0.7595 | 254.05 | 1.037 | 723.182 | 20.943 |
| macOS | `capture-off` | 0.0029 | 114.39 | 0 | — | — |
| macOS | `baseline` | 1.0161 | 366.52 | 2.032 | 494.077 | — |
| macOS | `cooldown-only` | 0.6839 | 360.02 | 1.342 | 497.526 | — |
| macOS | `native-only` | 1.0049 | 366.95 | 2.034 | 494.145 | — |
| macOS | `combined` | 0.6664 | 364.28 | 1.337 | 502.000 | — |

Relative to `baseline`, `combined` reduced process CPU cores to `0.7145×` on
Windows and `0.6558×` on macOS, and reduced Windows callback-copy throughput to
`0.1686×` (macOS native copy bytes and GPU metrics are unavailable; fixture
render-interval p95 was `16.218–16.572 ms` on Windows and `19.348–20.880 ms` on
macOS).
