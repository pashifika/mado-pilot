# ADR 0034: ONNX Runtime CPU loading boundary

- **Status:** Accepted
- **Date:** 2026-08-24
- **Resolves gate:** _none_ (`G-007` native bundling remains open)
- **Supersedes:** _none_

## Context

The Phase 3 CPU OCR backend must run the immutable G-004 model through ONNX Runtime without making process startup depend on a native library, downloading native code, or letting an ambient search choose executable bytes. The Rust binding and ONNX Runtime environment are process-global, while the selected runtime must remain alive longer than every environment, session, tensor, and destructor.

Source review found that `ort`'s `load-dynamic` feature is broader than this contract. An `ort` API used before `init_from` falls back to an environment variable and bare library name, and a second path can be silently ignored after the first process-global load. Source review also found that API 22 and newer make each new session prefer an automatically selected efficiency device, currently an NPU, unless the caller overrides that policy. These are wrong defaults for a backend whose accepted profile is CPU-only.

The source, feature, dependency, compatibility, loader, and local binary review is summarized in the [third-party dependency policy](../third-party-dependencies.md).

## Decision

`mado-pilot-backend-onnx` exact-pins `ort` 2.0.0-rc.13 and enables only `std`, `alternative-backend`, and `api-17`. No `ort` default feature is enabled. MadoPilot owns one process-global `libloading` boundary that:

1. accepts only a caller-supplied canonical absolute regular-file path with the target's exact filename;
2. loads it with `RTLD_NOW | RTLD_LOCAL` on macOS or `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32` on Windows;
3. requires `OrtGetApiBase`, exact runtime version `1.29.0`, and a non-null API-17 table;
4. retains the library handle for the process and installs the cloned table through `ort::set_api` before any other `ort` call; and
5. rejects a prior API/environment, a second path, a different runtime version, a missing symbol, or unsupported target with a closed typed fact and static public error.

Only the built-in CPU provider is used. API 17 keeps `ort`'s newer automatic device-selection path out of the build; no accelerator provider feature or environment provider is registered. Session construction applies the CPU arena policy explicitly and fails rather than falling back to another provider.

The accepted target filenames are `libonnxruntime.1.29.0.dylib` on `aarch64-apple-darwin` and `onnxruntime.dll` on `x86_64-pc-windows-msvc`. Paths, loader messages, build strings, and native errors remain private and are never placed in ordinary diagnostics.

## Alternatives

- **Use `ort` with `load-dynamic`.** Rejected because its pre-initialization fallback can use `ORT_DYLIB_PATH` or a bare filename and because its process-global loader does not expose a later ignored path. Wrapping only its public error does not prove which file supplied the API table.
- **Enable API 22 or newer and select a CPU device.** Viable but larger than necessary. It introduces automatic device-selection code and a newer device-factory surface solely to undo the binding's NPU-preferring default. Every required model, session, tensor, metadata, and termination operation exists in API 17.
- **Call `ort-sys` directly for all inference.** Rejected because duplicating session, tensor, output, status, and destructor wrappers would create substantially more unsafe code without a measured hot-path benefit.
- **Eager-link or enable downloaded/copied binaries.** Rejected because a missing library would fail process load, downloads violate the network-free contract, and bundling remains unresolved under `G-007`.
- **Accept any runtime at least as new as API 17.** Rejected because API-table compatibility does not qualify model output, performance, allocator behavior, or target deployment for another native build. Updating 1.29.0 requires fresh evidence and an amended decision.

## Consequences

- Integrators provision the exact host runtime and pass its absolute file path; MadoPilot does not install, discover, download, or bundle it.
- One runtime path and one ONNX environment win for the entire process. A caller cannot close and reopen the backend against another runtime without starting a new process.
- The runtime library deliberately outlives backend `close`; close releases sessions and per-call resources but not the process-global API table or native library.
- The `ort` release-candidate API is contained behind a private adapter and exact pin. Updating it requires source review, conformance and native-load reruns, dependency/license review, and confirmation that CPU selection did not change.
- Windows dependency resolution is limited to the selected DLL directory and System32. The reviewed macOS 1.29.0 artifact has only system-library/framework dependencies; every replacement artifact must repeat that closure check.
- ONNX Runtime remains host-provided under the MIT license. Model provenance remains Apache-2.0. Bundling and release notices remain `G-007` work.

## Verification

- Unit tests cover non-canonical paths, missing files, exact-version refusal, graph name/type/shape mismatch, vocabulary identity, tensor/output bounds, numeric ranges, and closed diagnostics.
- The explicit macOS native test loads the exact retained 1.29.0 runtime, opens and reopens the accepted session pair, passes the complete OCR backend contract, matches the tracked HUD oracle, terminates a cancelled run, rejects close during admitted work, and proves the session remains reusable.
- Cargo feature resolution proves downloader, TLS, copy, tracing, ndarray, `load-dynamic`, and accelerator features are absent from the active backend graph.
- The focused benchmark records cold/session load, warm inference, Rust allocation growth, resident memory, and repeated cleanup without declaring final budgets.
- Both release-target Rust surfaces compile with warnings denied. macOS additionally records the exact loaded native identity and dependency closure; the protected Windows CI job owns the real DLL load and must not be inferred from cross-compilation.
- Independent unsafe review covers symbol signatures, null checks, API-table clone, library lifetime, process-global ordering, session/tensor ownership, cancellation, and native teardown.
