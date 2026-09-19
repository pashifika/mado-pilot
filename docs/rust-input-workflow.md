# Independent Rust observation-to-input workflow

[`examples/rust-input-workflow`](../examples/rust-input-workflow/) is a standalone
Cargo application, not a product workspace member. It consumes only the public
`mado-pilot` facade and composes existing OCR queries and input contracts. No
runtime, C ABI, C++ wrapper or automatic library action is added.

This is a source-only, pre-1.0 integration example. Compilation and model-free
smoke are not evidence that a real application accepts input or reaches its
expected state. An operational run needs the operator's selected consuming
project, application, exact target, benign action, route/focus policy, visible
postcondition and finite capture/input authority. Do not substitute a fixture or
an arbitrary application for that selection.

## Build a genuinely external application

Install Rust **1.97.1**, Python **3.13+**, and the existing
[native prerequisites](../CONTRIBUTING.md#native-development-prerequisites).
The consumer owns its `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` and
`.cargo/config.toml`. A dependency's toolchain and Cargo configuration do not
configure its caller. The consumer explicitly sets macOS deployment minimum
**26.5.2**; Windows uses native x64 MSVC and the selected Windows SDK.

The proof also runs formatting and lint checks; install their toolchain components:

```sh
rustup component add --toolchain 1.97.1 rustfmt clippy
```

The tracked dependency is a path to this checkout's `crates/mado-pilot`. Select a
specific source revision before copying it. The proof tool copies the consumer
physically outside the checkout, replaces only that path with the selected
checkout's absolute path, and runs Cargo from the copied directory. It retains
its own lockfile and uses fresh Cargo build state; old qualification roots are
never reused. The destination must not already exist.

On Apple Silicon, from the product checkout:

```sh
python3 tools/check-external-rust-consumer.py \
  --work-root "$HOME/mado-input-consumer-proof"
```

On Windows, from an x64 MSVC developer command prompt:

```bat
python tools\check-external-rust-consumer.py --work-root "%USERPROFILE%\mado-input-consumer-proof" --opencv-root C:\native\opencv --libclang-path "C:\Program Files\LLVM\bin"
```

Use a new destination for each attempt. Retain failed attempts as failures rather
than replacing their output. The procedure reuses `tools/setup-native.py` from
an external working directory, retains commands and outputs under
`<work-root>/evidence`, and checks dependency resolution, compilation, model-free
execution and (on macOS) deployment metadata. It does not acquire native
libraries or models, prompt for permission, open capture or send input. Cargo may
fetch Rust dependencies into the new Cargo home.

OpenCV 4 and its shared dependencies must be loadable even for `--help` and
`--smoke`: they are linked into the executable. Build setup also needs libclang
and the native compiler/SDK. Keep the setup command's loader environment when
launching the application. For manual Cargo invocations, run from the copied
consumer directory and use the selected checkout's absolute setup path:

```sh
python3 /absolute/path/to/mado-pilot/tools/setup-native.py -- \
  cargo +1.97.1 build --locked
```

The Windows equivalent requires the same explicit OpenCV/libclang options and
MSVC context shown above. Setup affects only its child; it does not modify the
calling shell. The path dependency still reads the selected source checkout;
keep that checkout unchanged throughout a run and retain its commit/tree and any
working-tree changes with the build record.

A revision-pinned Git dependency is an alternative source selection:

```toml
mado-pilot = { git = "https://github.com/pashifika/mado-pilot", rev = "FULL_SOURCE_COMMIT" }
```

Replace `FULL_SOURCE_COMMIT` with the intended full commit, generate the
consumer's own lockfile for that route, then build with `--locked`. The Git route
has not been exercised by this Change; it is not covered by path-route evidence.
There is no crates.io package or installable dependency bundle.

## Configure a selected workflow

Create a private JSON file after selecting the real application. This
observation-only shape illustrates the required fields; replace the title,
absolute native paths, region and literal with your own authorized values.
Coordinates are capture pixels, not desktop coordinates.

```json
{
  "target": {"window_title": "Exact operator-selected window title"},
  "ocr": {
    "model_root": "/absolute/path/to/model-root",
    "runtime_path": "/absolute/path/to/libonnxruntime.dylib"
  },
  "before": {
    "roi": {"x": 0, "y": 0, "width": 640, "height": 360},
    "literal": "Operator-selected expected text",
    "minimum_confidence": 0.8
  },
  "action": {"kind": "observe"},
  "budgets": {
    "workflow_ms": 60000,
    "wait_ms": 10000,
    "postcondition_ms": 10000,
    "close_ms": 5000
  },
  "analysis_interval_ms": 200,
  "capture_pacing": {"mode": "preferred", "minimum_interval_ms": 100}
}
```

On Windows, use JSON-escaped absolute Windows paths and `onnxruntime.dll`.
Omitting `capture_pacing` preserves source defaults; explicit
`{"mode":"source_default"}` does the same. A `required` request refuses unavailable
native pacing; `preferred` may report unapplied capability instead.

For a separately authorized action, replace `action` with one of these recipes
and add both `input` and `after`:

```json
{
  "action": {"kind": "click"},
  "input": {"route": "system", "focus": "require_focused"},
  "after": {
    "roi": {"x": 0, "y": 0, "width": 640, "height": 360},
    "literal": "Operator-selected postcondition",
    "minimum_confidence": 0.8
  }
}
```

The snippet shows replacement/additional fields, not a complete second config.
A click requires one unambiguous satisfying OCR region and targets its center.
Use `{"kind":"text","text":"Host-selected text"}` for bounded text, or
`{"kind":"chord","key":"a","modifiers":["control"]}` for a key chord.
Modifiers are `shift`, `control`, `alt`, or `meta`; an empty modifier list
requests one key press/release. Choose the actual route/key meaning for the
selected application and platform. Neither this sample nor its coordinates
authorizes executing an action.

The consumer's admission bounds are explicit, not measured performance budgets:

| Field | Limit |
|---|---|
| Configuration / exact title / literal / input text | 64 KiB / 512 / 256 / 1,024 UTF-8 bytes |
| OCR paths | Absolute, at most 4,096 OS-string bytes each |
| ROI | Positive dimensions at most 2,048 each, area at most 1,048,576 pixels; origins at most 16,384 |
| Workflow | 1–120,000 ms, including model initialization |
| Independent wait / postcondition | Positive and no larger than the workflow budget; never extend its absolute deadline |
| Close / watcher admission / explicit capture interval | 1–10,000 ms each |
| Recipe / cleanup | At most 10 events; at most 10 releases within 250 ms |

The chord accepts one printable non-whitespace character, or `Enter`, `Tab`,
`Backspace`, `Delete`, `Escape`, `Space`, `ArrowUp`, `ArrowDown`, `ArrowLeft`,
`ArrowRight`, `Home`, `End`, `PageUp`, or `PageDown`. Duplicate modifiers refuse.
Observation-only mode omits `input`/`after` and refuses `--allow-input`.
Postcondition observation may re-arm at most 32 times after obtaining a frame
newer than the pre-input checkpoint; this never authorizes another input submission.

From the copied consumer directory, only after capture authority is established:

```sh
python3 /absolute/path/to/mado-pilot/tools/setup-native.py -- \
  cargo +1.97.1 run --locked -- --config /private/workflow.json --allow-capture
```

For an action configuration, append `--allow-input` only after separately
authorizing that exact action. Without input consent, an action configuration
refuses rather than silently acting or downgrading to observation. For Windows,
pass the native setup options as in the build procedure. Keep execution under
the finite budgets in the configuration.

## Authority and lifetime

`--help` describes the application interface. `--smoke` exercises caller decision
boundaries without an engine, target discovery, models, desktop permissions,
capture or input. Ordinary startup without a valid configuration refuses.

Operational mode requires `--config PATH --allow-capture`. Input additionally
requires `--allow-input`; an OCR match alone is never consent. Observation-only
mode opens no input capability. The JSON configuration is bounded to 64 KiB and
rejects unknown fields. Keep configuration files private: target names, literals,
input text and model/runtime paths are sensitive caller data.

The application initializes the explicit CPU bounded-detector OCR profile once
and maintains one session. Supply the reviewed ONNX Runtime **1.29.0** and model
files yourself, following the [OCR dependency guide](third-party-dependencies.md).
No download, provider substitution or ambient runtime search is performed.

One finite workflow authority covers initialization, discovery, observation,
input and postcondition observation. Query lifetime and caller wait remain
separate: ending a wait does not silently alter a query. Abandoned query ownership
is explicitly cancelled/dropped. Session close has a separate bounded cleanup
context; its outcome is retained alongside the primary outcome. Logical close
is not proof that native work has physically quiesced.

Capture pacing reuses the existing engine/session configuration and reports what
was applied. OCR watcher rate controls backend admission, not a cooldown after
recognition or caller interpretation. See [capture pacing](capture-pacing.md).

## Actions and platform routes

The application demonstrates one bounded action per invocation: click, text, or
keyboard/chord. Click coordinates and the complete source stamp come from the
same retained OCR result. The explicit unchanged-geometry policy refuses known
geometry changes; it does not prove that visual content stayed unchanged between
recognition and input. This visual time-of-check/time-of-use gap is the caller's
risk decision, not an atomic observe-and-act transaction.

Each action uses typed events, balanced sequence-owned presses/releases and a
bounded cleanup policy. Target selection is exact and unique. Target loss never
retargets another window. The application does not activate targets, switch
geometry policy, or retry uncertain input.

| Platform / route | Submission meaning |
|---|---|
| Windows `System` | Explicit focus policy and system-stream admission, not application consumption. |
| Windows `WindowMessage` | Exact retained-window address and queue admission. Ordinary application compatibility remains `Unknown`. |
| macOS `System` | Event-post permission and explicit focus policy; invocation evidence, not application effect. |
| macOS `ProcessDirected` | Owning-process invocation, not exact-window/responder delivery. Ordinary compatibility remains `Unknown`. |

Unsupported operation/route combinations refuse. No generic “background” route
or automatic `System` fallback is selected. The example uses one required route.
A consuming host can use the existing `DeliveryPlan::ordered` API only with its
own explicit route list; fallback is legal only after a proven zero-effect
refusal. A submitted prefix or possible partial native effect prohibits replay.

macOS Screen Recording and event-post/Accessibility permission are separate
non-prompting decisions. Neither grant implies the other. Missing grants do not
become permission requests or OS-settings changes. Windows integrity/UIPI and
focus failures remain observable; the consumer does not elevate.

## Submission and postcondition are different evidence

The immutable receipt retains complete, partial or unexecuted outcome, visited
routes, address scope, native submission evidence, submitted prefix, possible
partial effect, terminal fault and exact cleanup owed/released counts. An
incomplete or exhausted cleanup is not successful completion. Postcondition
failure never discards that receipt or triggers another input submission.

The consumer records a pre-input source checkpoint and accepts a postcondition
only from a strictly newer frame of the retained stream. It never recycles the
pre-input matching result as a postcondition. A newer satisfying observation is
reported separately from submission; neither fact proves causation. A timeout
means the requested state was not observed under that authority, not that input
was harmless or that retry is safe.

Ordinary output omits recognized/input text, pixels, window titles, credentials,
private model/runtime paths and unrestricted backend error detail. Preserve the
content-free receipt and cleanup records even when the process exits nonzero.

## Readiness boundaries

Both native CI jobs run the external path build and model-free smoke. Their
artifacts describe only the source, compiler, dependencies and scenarios actually
exercised. A local build on a newer macOS does not independently qualify that OS.

No real consuming application is selected or operationally verified by this
Change's build proof. Before an operational run, record the exact authorized
application workflow and retain every refusal, receipt, newer-frame observation
and cleanup result. An unselected platform remains build-only, not operationally
verified. Existing A9/A10, native/OCR workload evidence, benchmark ceilings,
C/C++ compatibility, packaging gates G-007/G-012 and release status are unchanged.
