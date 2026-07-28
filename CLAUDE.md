# Repository Guidelines

## Authority and scope

`docs/architecture.md` is the tracked architecture baseline and the source of truth for package boundaries, dependency directions, public naming, toolchain policy, scope, and implementation status. `docs/validation-gates.md` records the unresolved version-one decisions, and `CONTRIBUTING.md` defines the branch, pull request, and verification policy. Treat the rules in this file as the default project guidance.

When implementation, configuration, packaging, or examples change an architectural decision, keep the relevant documentation synchronized or add a focused architecture decision record (ADR) from `docs/adr/0000-template.md`. The ignored `local_docs/` drafts are proposal material only and are not a repository source of truth; when present, `local_docs/mado-pilot-design-v2.md` supersedes `local_docs/mado-pilot-design.md`. Where a `local_docs/` draft and `docs/architecture.md` disagree about what exists today, `docs/architecture.md` wins.

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

Use stream-first capture. One-shot operations consume maintained session state rather than creating a separate capture architecture. Frames are immutable and preserve native storage until mapping or backend conversion is required. Public frame identity and visual-result correlation include stream id, epoch, sequence, and geometry revision.

Queues are finite. Latest-wins behavior is the default for automation, and dropped, coalesced, superseded, rejected, or queue-expired work is observable. Dropping a work item never silently drops its query. Platform capture callbacks remain lightweight and never perform OCR, matching, host callbacks, or blocking consumer work.

Propagate an executor-neutral operation context with an absolute monotonic deadline and cancellation through every potentially blocking contract, including a caller's query wait. Check it before irreversible input and result commit; late backend results cannot mutate state or trigger input. Only authoritative unchanged metadata may advance duration stability without confirmation inference.

Do not invoke host callbacks while holding internal locks. Callback registration has an explicit disable-and-drain fence after which caller state may be released. Keep lock ordering documented, make session close idempotent, and ensure native resources outlive in-flight operations. Retained public frames must not pin producer buffer-pool slots required for capture progress. Public contracts remain independent of Tokio; optional async adapters may be added without exposing runtime-specific types in core APIs.

## Public API and ABI contracts

Public APIs keep target identity, complete source-frame identity, coordinate space, operation deadline, cancellation, input operation and delivery mode, focus policy, geometry policy, backend selection, and fallback behavior explicit. Prefer typed request objects when options are non-trivial. Input operation kind and delivery mechanism are separate axes, and partial input-sequence execution is observable. After cancellation or partial failure, only bounded release of state pressed by that sequence may continue, and cleanup failure is reported.

The C ABI uses product-prefixed `extern "C"` functions, fixed-width semantic numeric and version/report fields (`uint32_t` structure and reported table sizes, `uint64_t` row strides and semantic result/package counts), and `size_t` only for ABI-native addressability quantities: pointer-view lengths, replay input counts and element strides, target-list counts, accessor indexes, and the caller-known table extent passed to negotiation. It uses opaque handles, explicit pointer-length views, size-versioned structures, and module-owned allocation and release functions. Every function-table entry returns a status and writes values through validated outputs. Every owned handle has a complete lifecycle, and variable-sized results use immutable owned handles with borrowed views tied to that handle. Every output has a documented success and failure state. The version-one table covers discovery, capture, mapping, matching, input, OCR, watchers, and query result/callback access in additive implementation-phase order. Catch Rust panics and native exceptions before they cross the ABI boundary.

Treat ownership, lifetime, nullability, thread safety, callback dispatch, reentrancy, callback unregistration fences, structure-prefix negotiation, and parent/child teardown as documented contract. Preserve field and function-table ordering within an ABI major version, and test every supported old-header prefix against new libraries.

## Platform behavior

The Windows adapter owns target discovery, Windows Graphics Capture, D3D11 resource lifetime, coordinate mapping, and explicit system or background input implementations. Handle resize, device removal, focus, integrity/UIPI, and operating-system errors as observable state or typed failures. Never substitute system input for unsupported background delivery unless the request explicitly permits that fallback.

The macOS adapter owns target discovery, ScreenCaptureKit streams, native frame lifetime, coordinate transforms, non-prompting permission probes, and `CGEvent` input. Keep Objective-C or Objective-C++ shims narrow and internal. Report Screen Recording and Accessibility separately without presenting permission UI or calling permission-request APIs.

Published artifacts declare exact minimum operating-system versions. APIs newer than the deployment minimum use availability checks and weak or controlled dynamic linking so loadable unsupported capabilities fail with a clear status rather than an eager-link failure.

## Security and privacy

Screen capture and input are sensitive capabilities. Do not add implicit network access, automatic privilege escalation, or hidden permission behavior. Ordinary logs and diagnostics exclude captured images, recognized text, credentials, and other sensitive payloads by default.

Validate C pointer-length structure, conversion ranges, model and asset metadata, hashes when supplied, and all archive paths. Reject absolute paths, traversal, links, special files, duplicate normalized entries, and decompression bombs; never execute scripts from asset packages. Keep memory, archive expansion, and queues bounded for untrusted or high-rate sources.

## Testing and verification

Add deterministic unit or contract tests for changed behavior. Each adapter also passes its relevant contract suite. Use replay frames, synthetic clocks, fake input, controlled backend latency and failures, and target lifecycle scripts where appropriate.

Include affected native verification:

- Windows DPI, capture, input, allocator boundary, and packaging behavior;
- Apple Silicon Retina coordinates, ScreenCaptureKit, permissions, and packaging behavior;
- C and C++ compilation, ownership, invalid inputs, concurrency, callback unregistration, panic containment, ABI layout, and old-header compatibility;
- target movement, resize, stream epochs and geometry revisions, repeated frames, static stability, slow and out-of-order OCR, bounded queues, deadline/cancellation races, stale results, and target loss.

Each phase that introduces performance-sensitive behavior establishes repeatable benchmark profiles and pass/fail budgets for correctness, p95 latency, memory, mapped bytes, stale or dropped work, and relevant startup cost. A higher capture rate is not an improvement when it creates stale work, greater memory use, or incorrect results.

Use fixtures and expected outputs created for this repository or covered by a compatible license.

## Documentation and packaging

Documentation is part of implementation. Keep architecture, diagrams, examples, ownership rules, platform support tables, defaults, performance rationale, asset schema, packaging notes, and migration guidance synchronized with behavior.

Record model, asset, OpenCV, and ONNX Runtime licenses and deployment requirements. Make bundled versus controlled host-provided native dependencies explicit, expose build-profile capabilities, and report actionable backend-loading failures without relying on unrestricted ambient library search.

## Git workflow

Before implementation begins, create a short-lived topic branch from the active `dev/x.y.z` branch. If the active version branch cannot be determined from repository state or explicit user instruction, ask the user before changing implementation.
