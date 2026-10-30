# Third-party dependency policy

MadoPilot is distributed under `Apache-2.0`. Every Rust crate, native library, or
model file added to the project is governed by this policy. `deny.toml`
(`cargo deny --locked check`) enforces the Cargo portion only; native libraries
and models require manual license, provenance, and deployment review under
[Before adding a native dependency](#before-adding-a-native-dependency).

## Project license

The repository and every workspace Cargo package declare `Apache-2.0`, matching
the root `LICENSE` file.

## Approved Rust dependency licenses

`cargo deny check licenses` permits these permissive licenses:

`Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `MIT`, `MIT-0`, `BSD-2-Clause`,
`BSD-3-Clause`, `ISC`, `Unicode-3.0`, and `Zlib`.

Any other license (including copyleft or source-available terms) requires a short
[ADR](adr/README.md) before addition. Crates without license metadata are
rejected. Dual-licensed crates such as `adler2` (`0BSD OR MIT OR Apache-2.0`) and
`r-efi` (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`) are accepted under their
approved `MIT` / `Apache-2.0` terms.

## Approved sources

`cargo deny check sources` permits crates from `crates.io` only. Intra-workspace
path dependencies are validated by `mado-pilot-dependency-check`. Adding a Git
dependency requires an ADR recording the pinned commit revision, rationale, and
condition for returning to a published release.

## Security advisories

`cargo deny check advisories` checks the committed `Cargo.lock` against the
RustSec advisory database and rejects vulnerable or yanked dependencies unless a
documented exception is recorded.

## Wildcard version requirements

`cargo deny check bans` rejects wildcard version constraints on external crates.
Intra-workspace path edges use `allow-wildcard-paths` in `deny.toml` and are
verified by `mado-pilot-dependency-check`.

## Duplicate versions

Transitive duplicate crate versions are surfaced as review warnings by
`cargo deny check bans` and are either unified or justified in pull-request
review.

## Documented exceptions

Every temporary relaxation of `deny.toml` must be listed here with its removal
condition and re-evaluated whenever `Cargo.lock` changes:

| Package | Policy relaxed | Reason | Removal condition |
|---|---|---|---|
| _none_ | — | — | — |

## Before adding a Rust dependency

Confirm in the pull request that the crate is required by behavior in the same
change, is actively maintained, builds on both `x86_64-pc-windows-msvc` and
`aarch64-apple-darwin`, uses an approved license, and comes from `crates.io`.

## Product dependencies in use

| Crate | Used by | Role and feature boundary | License |
|---|---|---|---|
| `serde`, `serde_json` | `mado-pilot-adapter-replay`, `mado-pilot-assets`; `mado-pilot-ocr` build/maintainer tools | Strict replay/package manifest parsing and build-time catalog validation (OCR runtime uses generated static descriptors) | MIT OR Apache-2.0 |
| `zip` 9.0.3 | `mado-pilot-assets` | Bounded ZIP central-directory and raw entry reader; default features **off**, `deflate-flate2` only ([`G-014`](validation-gates.md#g-014) excludes bzip2/LZMA/XZ/Zstd/AES) | MIT |
| `flate2` 1.1 | `mado-pilot-assets` | Pure-Rust (`miniz_oxide`) DEFLATE decoding under `G-014` expansion limits | MIT OR Apache-2.0 |
| `crc32fast` 1.5 | `mado-pilot-assets` | Incremental ZIP entry CRC-32 validation | MIT OR Apache-2.0 |
| `sha2` 0.11 | `mado-pilot-assets`, `mado-pilot-ocr`, `mado-pilot-backend-onnx` | Package entry, OCR model component, and recognizer vocabulary SHA-256 verification | MIT OR Apache-2.0 |
| `unicode-normalization` 0.1.25 | `mado-pilot-ocr` | NFC normalization for OCR outputs and search literals under Unicode 17.0.0 decomposition bounds | MIT OR Apache-2.0 |
| `opencv` 0.99 | `mado-pilot-backend-opencv`, `mado-pilot-backend-onnx` | OpenCV C++ bindings for the [matching profile](architecture.md#template-matching) and OCR preprocessing; default features **off**, `imgcodecs`, `imgproc`, and `clang-runtime` only | MIT |
| `ort` `=2.0.0-rc.13` | `mado-pilot-backend-onnx` | Safe ONNX Runtime API 17 wrapper; default features **off**, `std`, `alternative-backend` (`ort-sys/disable-linking`), and `api-17` only (plus target-gated `cuda-provider` / `coreml-provider` registration modules; no downloader or telemetry) | MIT OR Apache-2.0 |
| `libloading` 0.8.9 | `mado-pilot-backend-onnx` | Restricted dynamic loading of caller-supplied ONNX Runtime and Windows CUDA/cuBLAS/cuDNN/NVRTC libraries without `PATH` search | ISC |
| `yaml-rust2` 0.13.0 | `mado-pilot-backend-onnx` | Pure-Rust bounded event parsing for the pinned Paddle `inference.yml` vocabulary; default features **off** | MIT OR Apache-2.0 |
| `libc` 0.2.189 | `mado-pilot-backend-onnx` (macOS benchmark dev-only) | `getrusage` RSS accounting in Apple OCR benchmarks | MIT OR Apache-2.0 |
| `windows` 0.62.2 | `mado-pilot-platform-windows` (`cfg(windows)`) | Microsoft Win32, WGC, D3D11/DXGI, DPI, `SendInput`, `PostMessageW`, and token-integrity bindings; default features **off** | MIT OR Apache-2.0 |
| `cc` 1.4 | `mado-pilot-platform-macos` (build); `mado-pilot-backend-opencv` (optional `benchmark-selection` build) | Compiles the macOS Objective-C shim and optional OpenCV benchmark C++17 scalar probe | MIT OR Apache-2.0 |

Notes on transitive and build-time closures in `Cargo.lock`:

- **`zip` 9.0.3 security guards:** Replaces yanked `8.6.0` while retaining the
  adapter's exact-suffix EOCD, zero-offset ZIP64, pre-parse entry-count/header,
  and `read_capped` expansion guards. On macOS/BSD, upstream pulls in `nix`
  0.31.3 (`fs`), `bitflags` 2.13.2, and `cfg_aliases` 0.2.2 (`MIT`/`Apache-2.0`),
  though MadoPilot never calls upstream filesystem extraction.
- **Build-time and inactive lockfile entries:** `opencv` resolves its build-time
  `opencv-binding-generator` / `clang-sys` closure, and `ort` records inactive
  optional `ndarray` / `tracing` entries in `Cargo.lock`; all carry approved
  `MIT`, `Apache-2.0`, `Zlib`, or `Unicode-3.0` licenses.

## Reviewed decisions not yet in the lockfile

| Decision | Reviewed crates | Status | Canonical reference |
|---|---|---|---|
| `G-003` macOS shim boundary | `objc2` 0.6.4 family, `block2`, `dispatch2`, `core-foundation`, `core-graphics` (queried 2026-07-30; `screencapturekit` and `cidre` rejected) | Not needed; `mado-pilot-platform-macos` uses build-time `cc` alone | [The macOS native boundary](architecture.md#the-macos-native-boundary) |

### Official PP-OCRv6 profile resources

The `ppocrv6-tiny-det-small-rec-max736-v1` catalog profile uses two official
PaddlePaddle ONNX graphs and `inference.yml` under `Apache-2.0`, pinned by byte
length and SHA-256 in the [OCR model catalog](ocr-model-catalog.md#source-schema).
MadoPilot neither downloads nor bundles these files.

### G-004 accepted OCR profile

[`G-004`](validation-gates.md#g-004) selected RapidOCR v3.9.2's
`ch_PP-OCRv4_det_mobile.onnx` and `PP-OCRv6_rec_small.onnx` (`25,979,900` bytes
total, `Apache-2.0`) from caller-supplied paths verified by byte length and
SHA-256 before session creation. Full provenance, digests, and cross-target
evaluation records live in [evidence/g-004/](evidence/g-004/).

### Implemented ONNX Runtime prerequisite

`mado-pilot-backend-onnx` loads ONNX Runtime `1.29.0` (C API 17, `MIT` license,
not redistributed) from a caller-supplied canonical absolute path:

- **Target library and loader flags:** `libonnxruntime.1.29.0.dylib` on macOS
  (`RTLD_NOW | RTLD_LOCAL`; Phase 3 observation `33,332,888` bytes and CI-pinned
  archive `d0706f…` / dylib `43,184,400` bytes `68f6e5…`, minimum macOS 14.0,
  system-only dependencies) and `onnxruntime.dll` on Windows
  (`LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`).
- **Process-lifetime singleton:** One canonical runtime path and one ONNX
  environment serve the process; initializing a second path returns
  `RuntimeConflict` (`Status::Unsupported`).
- **Execution provider boundaries:** `CoreML` uses macOS system frameworks (not
  release-qualified). Explicit Windows `CUDA` validates and loads the fixed
  18-DLL ONNX Runtime CUDA 13 + CUDA/cuBLAS/cuDNN 9/NVRTC set from one canonical
  flat directory (retaining 16 dependency handles before ORT initialization and
  deferring `onnxruntime_providers_cuda.dll`). Automatic selection remains `CPU`
  on both targets; see [OCR recognition](architecture.md#ocr-recognition).
- **CUDA licensing and provenance:** ONNX Runtime's `MIT` license does not cover
  CUDA Toolkit/cuBLAS/NVRTC or cuDNN, which remain subject to NVIDIA's supplied
  licenses. Qualification retains their source, version, byte length, SHA-256,
  signature, and license identity privately.
- **CI fixture transport:** Hosted CI jobs download the pinned ORT 1.29.0 archive
  and SHA-256-verified model files from an ordered source allowlist (ModelScope,
  Hugging Face, `hf-mirror.com`) inside ephemeral runners for test execution
  only.

Updating exact-pinned `ort` requires source review, conformance and native-load
reruns on both release targets, dependency and license review, and confirmation
that CPU provider selection and API 17 are unchanged. Changing the required ONNX
Runtime version from `1.29.0` requires new loading and compatibility evidence.
Every replacement macOS runtime artifact must repeat the check that its
dependency closure contains only system libraries and frameworks.

### G-003 macOS shim boundary

As recorded in [evidence/g-003/](evidence/g-003/README.md) and
[the macOS native boundary](architecture.md#the-macos-native-boundary),
`mado-pilot-platform-macos` compiles its Objective-C shim with Xcode Command Line
Tools and `cc` alone, loading `ScreenCaptureKit`, `AppKit`, `HIToolbox`, and
`Security.framework` from absolute system paths at runtime (`tests/linkage.rs`
verifies the eager framework list).

### G-002 Windows ownership prototype

As recorded in [evidence/g-002/dependency-review.md](evidence/g-002/dependency-review.md)
and [Windows native capture ownership](architecture.md#windows-native-capture-ownership),
`mado-pilot-platform-windows` uses only the target-gated `windows` 0.62.2 crate
against OS-provided Win32, WGC, D3D11, and DXGI system DLLs, with no Windows App
SDK, vcpkg, or redistributable dependency.

## Before adding a native dependency

Any change that adds, links, or bundles a native library or model must document
 in the same pull request:

1. Exact version, license, and required attribution/notice text.
2. Whether the artifact is developer/caller-provided or bundled, and how loading
   is restricted to canonical paths without ambient search.
3. Build-time versus runtime failure semantics (preserving typed runtime refusals).
4. Minimum OS version and package-size impact.
5. Source URL, byte length, SHA-256 digest, and license compatibility for models.

Broader native deployment profiles (`G-007`), static libraries (`G-008`), and
published build profiles (`G-012`) remain open in
[validation-gates.md](validation-gates.md); see
[native development prerequisite ownership](architecture.md#native-development-prerequisite-ownership).

## OpenCV

### Version and license

MadoPilot uses **OpenCV 4.x** under `Apache-2.0` (`opencv` Rust crate 0.99.1 under
`MIT`). Verification hosts use OpenCV `4.14.0` (Windows CI pins the official
`4.14.0` prebuilt archive; macOS CI uses Homebrew `opencv@4`, measured as
score-equivalent in [evidence/vision-opencv/](evidence/vision-opencv/)). Major
versions other than 4 are rejected at runtime with
`VisionFault::BackendUnavailable`.

### Bundled or host-provided

OpenCV is a developer-owned host prerequisite; nothing is bundled or downloaded
by MadoPilot. The historical five-candidate packaging investigation remains
frozen under [evidence/native-release-profiles/README.md](evidence/native-release-profiles/README.md)
 while `G-007` and `G-012` stay open.

### Development installation

Developers install OpenCV 4 and `libclang` (`brew install opencv@4` and Xcode
Command Line Tools on macOS; OpenCV `4.14.0` release `opencv_world4140` `/MD`
import library and LLVM `22.1.8` in an x64 MSVC prompt on Windows) and run builds
through `tools/setup-native.py`. See
[native development prerequisites](../CONTRIBUTING.md#native-development-prerequisites)
for the exact macOS and Windows setup commands, CLI flags, and module-linking
rules.

### libclang, and why `clang-runtime`

The `opencv` crate generates bindings at build time using `libclang`. Enabling
its `clang-runtime` feature makes the binding generator load `libclang` via
`dlopen` from the Xcode Command Line Tools directory on macOS and `LIBCLANG_PATH`
on Windows, avoiding `@rpath/libclang.dylib` load aborts on macOS where Command
Line Tools install no rpath entry.

### Failure when the library is absent

Because OpenCV is dynamically linked at build time, removing the OpenCV shared
library after setup prevents process entry. When the process loads,
`OpenCvBackend::new` validates that the runtime major version is 4 and returns
`VisionFault::BackendUnavailable` otherwise. All ONNX Runtime, model hash, and
platform capability checks remain enforced at runtime.

### Operating-system requirement and package size

Because no native library or model is redistributed in source releases,
developer-installed OpenCV adds zero bytes to source release archives and does
not alter the [`G-001`](validation-gates.md#g-001) OS deployment floors.

## Verification

Run step 7 of [the verification sequence](../CONTRIBUTING.md#verification) (requires
network access to fetch the RustSec advisory database):

```sh
cargo deny --locked check
```
