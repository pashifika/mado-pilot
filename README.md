# MadoPilot

Automate Windows and macOS applications through screen capture, image matching, text recognition, and input control.

Use one API to automate on-screen controls across Windows and macOS. MadoPilot handles the platform-specific capture and input integration. Start with reproducible image-based workflows, then connect them to real windows and displays.

## Highlights

- **Frame-linked results:** Locate images or read text and keep the exact frame that produced each result.
- **Template watching:** Wait for an image to remain visible, with a deadline and cancellation. Available through Rust on replayed frames and supported native sessions.
- **Rust OCR text queries:** Wait for literal text with exact source retention. The Rust API and both-target real CPU replay are verified; workload ceilings are accepted, while native capture and final workload qualification remain open. See [OCR text queries](docs/ocr-text-watch.md).
- **Explicit input control:** Choose the target and delivery mode, then inspect what was submitted. Platform permission and capability failures are reported without prompting or elevation.
- **Reproducible workflows:** Replay supplied frames without capturing the desktop or injecting input. Ordinary diagnostics exclude images, recognized text, and input payloads.

MadoPilot is a headless library for Rust, C, and C++, not a desktop automation editor. Distribution is source-only: crates.io packages, prebuilt libraries, installers, and bundled native dependencies are not provided. See the [release notes](docs/releases/) for each release's scope.

## Build from source

Use a checkout of this revision. Install **Rust 1.97.1** through rustup, **Python 3.13+**, and the native dependencies for your host:

| Host | Native prerequisites |
|---|---|
| Apple Silicon macOS; current CPU OCR host and native-shim ASan host (library scenarios only) 27.0.1, historical platform host 26.6.2 (minimum 26.5.2) | OpenCV 4, libclang, and Xcode Command Line Tools |
| Supported Windows 11 and later serviced x64 desktop releases; minimum Windows 11 25H2 build family 26200 | OpenCV 4, LLVM/libclang, and Visual Studio C++ build tools with a Windows SDK |

Current verification uses Windows 11 and Apple Silicon macOS 27.0.1. Historical
platform qualifications retain their exact host and source. On macOS 27.0.1, CPU
OCR verification keeps its ADR 0081 scope. ASan provides native-shim memory-safety
evidence for its recorded source only; it does not qualify capture/input topology
or performance.
See the separate [OS support and qualification policy](docs/architecture.md#os-support-policy).

Follow [native development setup](CONTRIBUTING.md#native-development-prerequisites) to acquire these dependencies and select their paths. `tools/setup-native.py` checks existing installations and configures only the command it launches. Missing or incompatible dependencies fail setup or build; the script installs nothing and leaves the calling shell unchanged.

On **macOS**, with Homebrew `opencv@4` installed, run from the repository root:

```bash
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example deterministic-slice
```

This builds the library and runs [the matching walkthrough](crates/mado-pilot/examples/deterministic-slice.rs). It reports matches for a supplied image, an absent template, and results that remain readable after the session closes. The first build compiles native bindings and can take several minutes.

On **Windows**, use an x64 MSVC developer command prompt and the [Windows setup command](CONTRIBUTING.md#native-development-prerequisites). Pass `cargo run --locked --package mado-pilot --example deterministic-slice` after its `--` separator. The commands below use the macOS setup form; use the Windows form around the same Cargo commands on Windows.

## Usage

These walkthroughs use generated frames and repository assets. They need no desktop permissions and inject no input.

**Wait for a stable image match.** The [template watcher](crates/mado-pilot/examples/template-watch.rs) reports a match after two confirmed observations, without a caller-written frame-polling loop:

```bash
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example template-watch
```

**Explore OCR result ownership.** The [OCR walkthrough](crates/mado-pilot/examples/ocr-fixture.rs) uses a simulated recognition backend and prints a fixed text result after closing the engine. It demonstrates the API, not actual text recognition:

```bash
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example ocr-fixture
```

For real text recognition, use [the default OCR walkthrough](crates/mado-pilot/examples/ocr-default.rs) with the documented [ONNX Runtime and model prerequisites](docs/third-party-dependencies.md#implemented-onnx-runtime-prerequisite). To watch real windows or displays, follow [native template watching](docs/native-template-watch.md), including its permission requirements and supported behavior.

For model choices, exact resources and scoped test-rule results, see
[the OCR model catalog](docs/ocr-model-catalog.md). All three implemented profiles
are ordinary Rust selections. Failed or missing quality/performance evidence is
reported alongside a choice, not used to hide it or change the default.

The new [OCR text watcher example](crates/mado-pilot/examples/ocr-text-watch.rs) uses the explicit CPU bounded-v2 profile. Its [fixed replay and native procedures](docs/ocr-text-watch.md) keep controlled API checks separate from real-model and permissioned capture evidence; no new support claim follows from compilation.

To configure native capture intervals and wait a full cooldown after OCR and caller
interpretation, use [capture pacing](docs/capture-pacing.md). Its model-free
`completion-paced-ocr --controlled-smoke` example runs without capture or input.
Its separately authorized Windows/macOS owned-workload results are documented with
metric limits; they do not change deployment floors or promise general application performance.

For an independent Rust application, copy the
[standalone input workflow](examples/rust-input-workflow/) and follow its
[external build and usage guide](docs/rust-input-workflow.md). It composes an OCR
wait, one explicitly authorized action, a receipt, and newer-frame postcondition
observation. Model-free smoke sends no input; real application use requires
separate target/action authorization.

## API and integration

| Language | Entry point and reference |
|---|---|
| Rust | Depend on [`mado-pilot`](crates/mado-pilot) and import `mado_pilot`. The [crate documentation](crates/mado-pilot/src/lib.rs) introduces the public API. |
| C / C++ | Use [`madopilot.h` and `MadoPilot::C`](docs/c-abi.md) directly; small samples demonstrate explicit handle ownership. |

Generate the full Rust API reference locally:

```bash
python3 tools/setup-native.py -- cargo doc --locked --package mado-pilot --no-deps
```

Open `target/doc/mado_pilot/index.html`. Rust API stability begins at 1.0; the C ABI has its own compatibility policy. Template and OCR text watching are currently Rust-only; OCR watcher qualification remains open.

## Documentation

| Guide | Read it for |
|---|---|
| [Architecture](docs/architecture.md) | Package layout, dependency boundaries, public naming, and implementation status |
| [Platform support](docs/architecture.md#release-targets) | Supported systems and the distinction between deployment floors and verified hosts |
| [Windows input](docs/windows-input-verification.md) / [macOS input](docs/macos-input-verification.md) | Delivery modes, permissions, focus behavior, and native verification |
| [Native dependencies](docs/third-party-dependencies.md) | OpenCV, ONNX Runtime, models, licenses, and deployment requirements |
| [OCR model catalog](docs/ocr-model-catalog.md) | Model choices, exact resources, schema, and independent test results |
| [Performance](docs/performance.md) | Workload-specific budgets and measurements |
| [Validation gates](docs/validation-gates.md) | Remaining version-one decisions and acceptance criteria |
| [Decisions](docs/adr/) / [Evidence](docs/evidence/) | Design rationale and revision-bound verification records |
| [Release notes](docs/releases/) | Changes, compatibility, and artifact availability for each source release |

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup, branch and pull request policy, and the verification sequence. Consult the [architecture baseline](docs/architecture.md) before changing package boundaries or public contracts.

## License

[Apache License 2.0](LICENSE).
