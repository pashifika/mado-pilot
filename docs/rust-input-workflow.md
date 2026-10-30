# Independent Rust observation-to-input workflow

[`examples/rust-input-workflow`](../examples/rust-input-workflow/) is a
standalone Cargo application outside the product workspace. It depends only on
the public `mado-pilot` facade to compose an OCR text-presence wait, one
explicitly authorized input action, immutable receipt inspection, and a
strictly-newer-frame visual postcondition check.

## Build a genuinely external application

The consumer owns its own `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`
(Rust `1.97.1` with `rustfmt` and `clippy`), and `.cargo/config.toml` (macOS
deployment floor `26.5.2`). Install the native prerequisites in
[CONTRIBUTING.md](../CONTRIBUTING.md#native-development-prerequisites), ensure
no untracked files exist in the product crate/manifest/setup paths, and run the
external copy-and-verify script into a fresh destination directory:

```sh
# macOS (Apple Silicon)
rustup component add --toolchain 1.97.1 rustfmt clippy
python3 tools/check-external-rust-consumer.py \
  --work-root "$HOME/mado-input-consumer-proof"
```

```bat
rem Windows (x64 MSVC developer command prompt)
python tools\check-external-rust-consumer.py --work-root "%USERPROFILE%\mado-input-consumer-proof" --opencv-root C:\native\opencv --libclang-path "C:\Program Files\LLVM\bin"
```

`tools/check-external-rust-consumer.py` copies `examples/rust-input-workflow`
outside the repository, rewrites its `mado-pilot` path dependency to the
checkout's absolute path, runs `cargo fmt --check`, `cargo clippy`, `cargo test`,
`--help`, `--smoke`, and non-regular config-path refusal through
`tools/setup-native.py`, and verifies the macOS Mach-O `26.5.2` minimum version
without opening capture or sending input.

To build manually from a copied consumer directory (or when switching `Cargo.toml`
to a pinned `git = "https://github.com/pashifika/mado-pilot", rev = "<full-sha>"`
dependency and regenerating the consumer's `Cargo.lock`; the `git`/`rev` route is
unexercised and not covered by the path-route `tools/check-external-rust-consumer.py`
proof):

```sh
python3 /absolute/path/to/mado-pilot/tools/setup-native.py -- \
  cargo +1.97.1 build --locked
```

## Configure a selected workflow

Create a private JSON configuration file (`<= 64 KiB`, rejecting unknown fields;
all ROI coordinates are in capture pixels):

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

For an authorized input action, replace `action` (`{"kind": "click"}`,
`{"kind": "text", "text": "..."}`, or
`{"kind": "chord", "key": "a", "modifiers": ["control"]}`) and add both `input`
and `after`:

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

Enforced consumer admission bounds:

| Field | Enforced limit |
|---|---|
| Config file / window title / literal / input text | `64 KiB` / `512 B` / `256 B` / `1,024 B` UTF-8 |
| OCR `model_root` and `runtime_path` | Canonical absolute paths (`<= 4,096` OS-string bytes each) |
| ROI | Positive width/height `<= 2,048` each, area `<= 1,048,576` px, origin `<= 16,384` |
| `workflow_ms` / `wait_ms` & `postcondition_ms` | `1..=120,000 ms` / positive and `<= workflow_ms` |
| `close_ms`, `analysis_interval_ms`, `minimum_interval_ms` | `1..=10,000 ms` each |
| Input recipe / cleanup | `<= 10` events; `<= 10` releases within `250 ms` (`postcondition` may re-arm at most `32` times on newer frames) |
| Chord keys & modifiers | One printable non-whitespace char or `Enter`, `Tab`, `Backspace`, `Delete`, `Escape`, `Space`, `ArrowUp/Down/Left/Right`, `Home`, `End`, `PageUp/Down`; unique modifiers in `shift`, `control`, `alt`, `meta` |

Run observation-only mode with `--allow-capture`, and append `--allow-input`
only when authorizing the configured input action:

```sh
python3 /absolute/path/to/mado-pilot/tools/setup-native.py -- \
  cargo +1.97.1 run --locked -- --config /private/workflow.json --allow-capture
```

## Authority and lifetime

- `--smoke` exercises argument and config validation without opening an engine,
  models, capture, or input.
- Operational runs initialize the CPU bounded-detector OCR profile once using
  caller-supplied ONNX Runtime `1.29.0` and model paths (see
  [ONNX Runtime prerequisites](third-party-dependencies.md#implemented-onnx-runtime-prerequisite))
  and apply optional native capture pacing (see [capture pacing](capture-pacing.md)).
- One workflow deadline governs initialization, discovery, pre-input OCR wait,
  input submission, and postcondition observation, followed by a separate
  bounded session-close context.

## Actions and platform routes

Click actions require a single unambiguous matching OCR region and target its
center under `RequireUnchanged` geometry policy, with no target activation,
route fallback, or automatic retry:

| Platform / route | Submission evidence meaning |
|---|---|
| Windows `System` | Requires focus policy; `SystemInputAdmission` (`SendInput`). |
| Windows `WindowMessage` | Exact retained window; `TargetQueueAdmission` (`Unknown` ordinary app compatibility). |
| macOS `System` | Requires event-post permission and focus policy; `InvocationOnly` (`CGEventPost`). |
| macOS `ProcessDirected` | Owning-process scope (`Unknown` compatibility); `InvocationOnly` (`CGEventPostToPid`). |

See [Windows input verification](windows-input-verification.md) and
[macOS input verification](macos-input-verification.md) for route and permission
contracts.

## Submission and postcondition are different evidence

The consumer records the pre-input frame stamp, inspects the immutable
`InputReceipt` (`Complete`, `Partial`, or `Unexecuted`, plus `released`/`owed`
cleanup counts), and accepts a postcondition match only from a strictly newer
frame on the retained stream. Output omits recognized text, input text, pixels,
window titles, and model paths.

## Readiness boundaries

Both native CI jobs run `tools/check-external-rust-consumer.py` (path build and
`--smoke`). In a separate operator-authorized attended exercise on Apple Silicon
macOS `27`, the consumer drove `SSR.app 1.0.461` across three click actions
(menu reveal and control selection in a first invocation that stopped when the
host decision wait expired, followed by the return click in a second invocation
under renewed wait authority, with `3/3` complete receipts, `0/0` cleanup
owed/released, `2/2` session closes, and strictly newer postcondition
observation). Windows remains build- and model-free-smoke verified.
