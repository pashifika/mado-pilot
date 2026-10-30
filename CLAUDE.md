# Repository Guidelines

## Authority and scope

`docs/architecture.md` is the tracked architecture baseline and canonical source for package boundaries, dependency directions, public naming, OS support policy, and runtime contracts. `docs/validation-gates.md` tracks the `G-001`–`G-014` decisions, `CONTRIBUTING.md` owns setup, branch/PR flow, and the verification sequence, and `docs/README.md` indexes all topic guides. Whenever one document references another's domain, state a brief summary and link to the canonical guide rather than duplicating full procedures or histories.

When a change replaces an important architectural or technical choice, update the canonical contract and record the rationale in a short ADR using `docs/adr/template.md` under `docs/adr/README.md`. Empirical budget or tuning changes use a tuning record under `docs/performance/tuning/` instead of an ADR, while mandatory safety, ownership, and compatibility contracts remain enforced. Historical records (`docs/evidence/`, `docs/releases/`, `docs/benchmarks/`, dated `docs/performance/tuning/2026-*.md`, and `rasen/legacy-adrs/`) remain byte-identical historical provenance and never override current contracts. Where ignored `local_docs/` drafts (`local_docs/mado-pilot-design-v3.md`) differ from `docs/architecture.md`, `docs/architecture.md` wins.

## Product definition

MadoPilot is a headless visual automation runtime for applications and agents on `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. It discovers windows and displays, captures frame streams, maps coordinate spaces, performs template matching and OCR, waits for visual conditions, injects input through explicit platform capabilities, and reports structured diagnostics.

Public integration surfaces are:

- an idiomatic Rust API through the `mado-pilot` facade crate;
- a separately versioned C ABI (`madopilot.h` and `MadoPilot::C`) with opaque handles and explicit caller-owned lifetimes.

MadoPilot does not own a GUI, tray, editor, overlay, updater, workflow catalog, time-based scheduler, C++ wrapper, or general scripting DSL. Deferred adapters (ADB, browser/CDP, Apple Vision) remain out of scope until assigned an owner, contract, tests, and support statement.

## Requirements and artifact language

Write proposals and specifications in user-facing behavior and public contract language, preferring positive outcomes over negative disclaimers. Keep internal algorithms, locks, and crate wiring in design or task artifacts unless part of the public contract. Document separate Windows and macOS outcomes whenever capabilities, permissions, fallback, target loss, resize, deadlines, cancellation, coordinates, or input delivery differ.

## Architecture boundaries

Follow ports-and-adapters dependency direction (enforced by `mado-pilot-dependency-check`):

- `core` contains no Windows, macOS, OpenCV, ONNX Runtime, GUI, or executor-specific types;
- platform-neutral contract crates (`capture`, `input`, `vision`, `ocr`, `assets`) do not depend on concrete adapters;
- `runtime` orchestrates contracts without knowing concrete adapter types;
- platform (`windows`, `macos`) and backend (`opencv`, `onnx`) crates implement those contracts;
- the `mado-pilot` facade performs default wiring, and `mado-pilot-capi` depends on the facade only.

Organize modules by responsibility without a general `utils` layer or empty crates for deferred adapters.

## Runtime and concurrency

Use stream-first capture where one-shot operations consume maintained session state. Frames are immutable, preserve native storage until mapping or conversion is required, and correlate visual results by stream id, epoch, sequence, and geometry revision. Retained public frames never pin producer buffer-pool slots needed for capture progress.

Queues are finite with latest-wins default behavior; displaced, coalesced, or expired work is observable and never silently drops a caller's query. Platform capture callbacks stay lightweight and never run OCR, matching, host callbacks, or blocking work. Propagate an executor-neutral operation context (monotonic deadline and cancellation) through every blocking contract, checking it before irreversible input or result commit. Never invoke host callbacks while holding internal locks; enforce an explicit disable-and-drain callback unregistration fence and idempotent session close.

## Public API and ABI contracts

Public APIs keep target identity, source-frame identity, coordinate space, deadline, cancellation, input operation and delivery route, focus policy, geometry policy, backend selection, and fallback explicit. Partial input-sequence execution is observable; after cancellation or failure, only bounded release of sequence-pressed state may run, and cleanup outcomes are reported.

The C ABI (`docs/c-abi.md`) uses `madopilot_`-prefixed `extern "C"` functions, `uint32_t` struct/table sizes, `uint64_t` strides/counts, `size_t` for addressability lengths/indices, opaque handles, size-versioned structs, and module-owned release functions. Preserve field and function-table ordering within ABI major 1, test all frozen header prefixes (`1.0`/`1.2`/`1.3`/`1.4`) against current builds, and contain Rust panics and native exceptions before crossing the ABI boundary.

## Platform behavior

- **Windows (`mado-pilot-platform-windows`):** Owns target discovery, WGC capture, D3D11 lifetime, DPI coordinate mapping, and `System` / `WindowMessage` input (`docs/windows-input-verification.md`). Never substitute `System` input for unsupported background delivery unless explicitly requested.
- **macOS (`mado-pilot-platform-macos`):** Owns target discovery, ScreenCaptureKit capture, native frame lifetime, Retina transforms, non-prompting permission preflights (`Screen Recording` and `CGPreflightPostEventAccess`), and `System` / `ProcessDirected` `CGEvent` input (`docs/macos-input-verification.md`). Keep Objective-C shims narrow and internal (`-fobjc-arc-exceptions`).

## Security and privacy

Perform no implicit network access, privilege escalation, or permission prompting. Ordinary logs and diagnostics exclude captured pixels, recognized text, input payloads, credentials, and host-local paths. Validate C pointer-length views, numeric ranges, asset/model hashes, and archive entries (`G-014` safety ceilings rejecting traversal, symlinks, special files, duplicates, and decompression bombs).

## Testing, documentation, and Git workflow

Add deterministic contract tests for changed behavior and run the verification sequence in `CONTRIBUTING.md#verification`. Synchronize affected guides under `docs/` in the same pull request, and keep host-provided versus bundled native dependency requirements explicit (`docs/third-party-dependencies.md`). Before implementation begins, create a short-lived topic branch from the active `dev/x.y.z` branch (or ask the user if the active version branch cannot be determined).
