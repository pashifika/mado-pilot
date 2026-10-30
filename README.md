# MadoPilot

Automate Windows and macOS applications through screen capture, template matching, OCR, and explicit input control.

Building reliable visual automation across Windows and macOS normally requires stitching together separate screen-capture APIs, DPI coordinate transforms, vision backends, and input injectors. MadoPilot unifies window and display discovery, stream capture, template matching, text recognition, and input delivery behind one deterministic runtime for Rust, C, and C++. MadoPilot is a headless source-only library rather than a desktop GUI editor or scripting DSL; see the [release notes](docs/releases/) for each source release's scope and unavailable distribution artifacts.

## Highlights

- **Frame-linked visual results:** Locate image templates or read text while retaining the exact stream id, epoch, sequence, and geometry revision that produced every result.
- **Template watching:** Wait for an image template to remain visible across confirmed frames within an explicit deadline and cancellation context on replayed frames or supported native sessions.
- **Rust OCR text queries:** Wait for literal on-screen text with exact source-frame retention and configurable model profiles; see [OCR text queries](docs/ocr-text-watch.md) for replay and native qualification scope.
- **Explicit input control:** Select the target window and delivery route (`System`, Windows `WindowMessage`, or macOS `ProcessDirected`) and inspect the resulting submission receipt without implicit fallback.
- **Deterministic and privacy-bounded workflows:** Develop against replayed frame packages without live desktop permissions or input injection, while ordinary diagnostics exclude captured images, recognized text, and input payloads.

## Build from source

Install **Rust 1.97.1** via `rustup`, **Python 3.13+**, and the host-provided native dependencies for your target:

| Target | Minimum OS and host prerequisites |
|---|---|
| `aarch64-apple-darwin` (Apple Silicon macOS, minimum 26.5.2) | OpenCV 4 (`brew install opencv@4`), `libclang`, and Xcode Command Line Tools |
| `x86_64-pc-windows-msvc` (Windows 11 x64, minimum 25H2 build 26200) | OpenCV 4.14.0, LLVM/libclang 22.1.8, and Visual Studio C++ build tools with a Windows SDK |

See the [OS support policy](docs/architecture.md#os-support-policy) for the distinction between minimum deployment floors and revision-bound verification hosts. Follow [native development prerequisites](CONTRIBUTING.md#native-development-prerequisites) to install these dependencies and pass explicit `--opencv-root` and `--libclang-path` arguments on Windows.

On macOS, build the workspace and run [the deterministic matching walkthrough](crates/mado-pilot/examples/deterministic-slice.rs):

```bash
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example deterministic-slice
```

`tools/setup-native.py` validates existing native installations and configures only the child command it launches. The walkthrough loads a repository fixture package, matches a template, verifies an absent template, and reads retained results after closing the session.

## Usage

The examples below run against repository fixtures without desktop permissions or live input. On Windows, run the same Cargo commands after the `python tools/setup-native.py --opencv-root DIR --libclang-path DIR --` prefix from an x64 MSVC developer prompt.

1. **Wait for a stable template match:** Run the [template watcher example](crates/mado-pilot/examples/template-watch.rs) to observe a match after two confirmed frames without writing a manual polling loop:
   ```bash
   python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example template-watch
   ```
   To watch live Windows Graphics Capture or ScreenCaptureKit sessions, see [native template watching](docs/native-template-watch.md) for permission rules, configuration, and supported behavior.

2. **Inspect OCR result ownership:** Run the [simulated OCR walkthrough](crates/mado-pilot/examples/ocr-fixture.rs) to exercise the OCR request and result lifecycle without external model files:
   ```bash
   python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example ocr-fixture
   ```
   For real text recognition with [`ocr-default.rs`](crates/mado-pilot/examples/ocr-default.rs), see [ONNX Runtime prerequisites](docs/third-party-dependencies.md#implemented-onnx-runtime-prerequisite) for runtime setup and the [OCR model catalog](docs/ocr-model-catalog.md) for profile selection and evaluation results.

3. **Watch for literal text or pace capture intervals:** Run the [OCR text-watch example](crates/mado-pilot/examples/ocr-text-watch.rs) or the model-free [completion-paced OCR smoke check](crates/mado-pilot/examples/completion-paced-ocr.rs):
   ```bash
   python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example completion-paced-ocr -- --controlled-smoke
   ```
   See [OCR text queries](docs/ocr-text-watch.md) for CPU execution options and replay/native procedures, [capture pacing](docs/capture-pacing.md) for native frame intervals and caller cooldowns, and [standalone Rust input workflow](docs/rust-input-workflow.md) to build [`examples/rust-input-workflow`](examples/rust-input-workflow/) outside the workspace.

## API and integration

| Surface | Entry point and reference |
|---|---|
| Rust | Depend on [`mado-pilot`](crates/mado-pilot) and import `mado_pilot` ([crate overview](crates/mado-pilot/src/lib.rs)). Template and OCR watchers are currently Rust-only; API stability begins at 1.0. |
| C / C++ | Include [`madopilot.h`](crates/bindings/capi/include/madopilot/madopilot.h) and link `MadoPilot::C`. See the [C ABI guide](docs/c-abi.md) for handle ownership, `struct_size` prefix negotiation, and C/C++ samples. |

Generate the local Rust API reference:

```bash
python3 tools/setup-native.py -- cargo doc --locked --package mado-pilot --no-deps
```

Open `target/doc/mado_pilot/index.html`.

## Documentation

Start at the [documentation index](docs/README.md) to find every current contract, guide, and historical record by task:

| Guide | What it covers |
|---|---|
| [Architecture](docs/architecture.md) | Package boundaries, dependency allowlist, OS support policy, public naming, and runtime contracts |
| [Validation gates](docs/validation-gates.md) | Version-one `G-001`–`G-014` decision registry, blocking scope, and evidence links |
| [Windows input](docs/windows-input-verification.md) / [macOS input](docs/macos-input-verification.md) | Input delivery routes, permission probes, focus and DPI rules, and opt-in native verification |
| [Third-party dependencies](docs/third-party-dependencies.md) | License policy, OpenCV and ONNX Runtime prerequisites, and model provenance |
| [Performance](docs/performance.md) | Focused development measurements, outcome-driven metrics, and benchmark profile rules |

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for native setup, branch and pull-request rules, and the 10-step verification sequence. Review [docs/architecture.md](docs/architecture.md) before changing package boundaries or public contracts.

## License

Licensed under the [Apache License 2.0](LICENSE).
