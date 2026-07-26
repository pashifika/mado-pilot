# Repository Guidelines

## Authority and scope

`CONTRIBUTING.md` defines the branch and pull request policy. Treat the rules in this file as the default project guidance.

When implementation, configuration, packaging, or examples change an architectural decision, keep the relevant documentation synchronized or add a focused architecture decision record (ADR).

## Product definition

MadoPilot is a headless visual automation runtime for applications and agents. It discovers windows and displays, captures frame streams, maps coordinate spaces, performs template matching and OCR, waits for visual conditions, injects input through explicit platform capabilities, and reports structured diagnostics.

The primary implementation is Rust. Public integration surfaces are:

- an idiomatic Rust API through the `mado-pilot` facade crate;
- a separately versioned C ABI with opaque handles and explicit ownership;
- a thin C++ RAII wrapper that consumes only the released C ABI.

The initial release targets are `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`.

MadoPilot does not own a GUI, tray, editor, overlay, updater, workflow catalog, time-based scheduler, or general scripting DSL. ADB, browser/CDP, and Apple Vision adapters remain future work until they have an owner, implemented contract, tests, and an explicit support statement.

## Requirements and artifact language

Write proposals and specifications in user-facing behavior and public contract language. Describe the experience and observable outcome before internal mechanisms. Prefer positive outcomes over implementation-negative requirements when both express the same rule.

Keep algorithms, crate wiring, queues, locks, and adapter details in design or task artifacts unless a mechanism is itself part of the public contract. Distinguish version-one scope, non-goals, future work, and unresolved prototype gates.

When behavior differs by platform, document separate Windows and macOS outcomes. Cover capabilities, permissions, fallback, target loss, resize, deadlines, cancellation, coordinates, input modes, and unsupported-system behavior when relevant.

## Architecture boundaries

Follow ports-and-adapters dependency direction:

- platform-neutral contract crates do not depend on platform or backend adapters;
- `core` contains no Windows, macOS, OpenCV, ONNX Runtime, GUI, or executor-specific types;
- `runtime` orchestrates contracts without knowing concrete adapter types;
- platform crates implement capture and input contracts;
- backend crates implement vision or OCR contracts;
- the public facade performs default wiring;
- the C ABI depends on the facade, never the reverse;
- the C++ wrapper links through the released C ABI only.

Organize modules by responsibility. Do not introduce a general `utils` layer. Do not add empty crates for deferred adapters.

## Runtime and concurrency

Use stream-first capture. One-shot operations consume maintained session state rather than creating a separate capture architecture. Frames are immutable and preserve native storage until mapping or backend conversion is required.

Queues are finite. Latest-wins behavior is the default for automation, and dropped or coalesced work is observable. Platform capture callbacks remain lightweight and never perform OCR, matching, host callbacks, or blocking consumer work.

Do not invoke host callbacks while holding internal locks. Keep lock ordering documented, make session close idempotent, and ensure native resources outlive in-flight operations. Public contracts remain independent of Tokio; optional async adapters may be added without exposing runtime-specific types in core APIs.

## Public API and ABI contracts

Public APIs keep target identity, coordinate space, deadline, cancellation, input mode, focus policy, backend selection, and fallback behavior explicit. Prefer typed request objects when options are non-trivial.

The C ABI uses product-prefixed `extern "C"` functions, fixed-width types, opaque handles, explicit pointer-length views, size-versioned structures, and module-owned allocation and release functions. Catch Rust panics and native exceptions before they cross the ABI boundary.

Treat ownership, lifetime, nullability, thread safety, callback dispatch, reentrancy, and callback unregistration fences as documented contract. Preserve field and function-table ordering within an ABI major version, and test compatible old headers against new libraries.

## Platform behavior

The Windows adapter owns target discovery, Windows Graphics Capture, D3D11 resource lifetime, coordinate mapping, and explicit system or background input implementations. Handle resize, device removal, focus, integrity, and operating-system errors as observable state or typed failures.

The macOS adapter owns target discovery, ScreenCaptureKit streams, native frame lifetime, coordinate transforms, permission probes, and `CGEvent` input. Keep Objective-C or Objective-C++ shims narrow and internal. Report Screen Recording and Accessibility permission states without presenting permission UI.

Unsupported operating systems fail with a clear status rather than linking successfully and failing unpredictably at runtime.

## Security and privacy

Screen capture and input are sensitive capabilities. Do not add implicit network access, automatic privilege escalation, or hidden permission behavior. Ordinary logs and diagnostics exclude captured images, recognized text, credentials, and other sensitive payloads by default.

Validate C pointer-length pairs, model and asset metadata, hashes when supplied, and all archive paths. Reject path traversal and never execute scripts from asset packages. Keep memory and queues bounded for untrusted or high-rate sources.

## Testing and verification

Add deterministic unit or contract tests for changed behavior. Each adapter also passes its relevant contract suite. Use replay frames, synthetic clocks, fake input, controlled backend latency and failures, and target lifecycle scripts where appropriate.

Include affected native verification:

- Windows DPI, capture, input, allocator boundary, and packaging behavior;
- Apple Silicon Retina coordinates, ScreenCaptureKit, permissions, and packaging behavior;
- C and C++ compilation, ownership, invalid inputs, concurrency, callback unregistration, panic containment, ABI layout, and old-header compatibility;
- target movement, resize, stream epochs, repeated frames, slow OCR, bounded queues, cancellation, stale results, and target loss.

Measure end-to-end latency, memory, mapped bytes, dropped work, and correctness for performance-sensitive changes. A higher capture rate is not an improvement when it creates stale work or greater memory use.

Use fixtures and expected outputs created for this repository or covered by a compatible license.

## Documentation and packaging

Documentation is part of implementation. Keep architecture, diagrams, examples, ownership rules, platform support tables, defaults, performance rationale, asset schema, packaging notes, and migration guidance synchronized with behavior.

Record model, asset, OpenCV, and ONNX Runtime licenses and deployment requirements. Make bundled versus host-provided native dependencies explicit, and report actionable backend-loading failures.

## Git workflow

Before implementation begins, create a short-lived topic branch from the active `dev/x.y.z` branch. If the active version branch cannot be determined from repository state or explicit user instruction, ask the user before changing implementation.
