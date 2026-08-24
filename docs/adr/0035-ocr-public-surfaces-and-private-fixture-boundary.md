# ADR 0035: OCR public surfaces and private fixture boundary

- **Status:** Accepted
- **Date:** 2026-08-24
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

The OCR contract and ONNX CPU adapter already exist, but no released Rust, C, or C++ surface could execute OCR. ABI review, result ownership, close arbitration, and diagnostic privacy must be proven independently of ONNX Runtime installation and model quality. Rust can inject a deterministic `OcrBackend` directly; separately compiled C and C++ examples cannot construct a Rust trait object.

Putting fake behavior in a production constructor would create an implicit OCR default. Adding fixture construction to the negotiated table would permanently consume released ABI surface for test policy. Requiring ONNX Runtime would couple ownership and compatibility checks to native loading that this Change does not own.

## Decision

ABI 1.3 appends one-shot OCR execution and immutable owned result access after the complete 592-byte ABI 1.2 table. Production Rust constructors accept only an explicit OCR backend and select none by default; the production C table adds no backend constructor or fallback.

A `private-fixture` build may export `madopilot_fixture_engine_create`, declared only by `examples/ocr-private-fixture.h`. It configures a fixed local fake backend for replay examples, is absent from release builds and the public C header/table, performs no input or network access, and hands all subsequent work to the ordinary ABI 1.3 entries.

## Alternatives

- **Wire the fake from normal replay construction.** Rejected because behavior would depend on hidden test policy and make an unconfigured production engine appear OCR-capable.
- **Append a fake-engine constructor to ABI 1.3.** Rejected because fixture policy is not a product contract and function-table positions are permanent within ABI major 1.
- **Use the ONNX backend in C and C++ examples.** Rejected because it would require native installation/loading and model execution to prove pointer validation, ownership, cancellation, and old-header compatibility.
- **Expose caller callbacks as a C backend seam.** Rejected because it adds callback lifetime, reentrancy, fencing, and panic/exception boundaries that one-shot OCR does not need.

## Consequences

- Rust callers configure `Arc<dyn OcrBackend>` explicitly and send requests that name its exact descriptor and a package-resolved model identity.
- C and C++ production callers can execute OCR only on an engine configured by a production composition root added by a later integration Change; there is no silent fallback.
- ABI 1.0 and 1.2 headers remain complete frozen prefixes. ABI 1.3 callers must negotiate through `MADOPILOT_API_SIZE_OCR_RESULT_TEXT_AT` before creating an OCR result owner.
- OCR results own immutable text, geometry, source identity, and descriptors only. Borrowed C/C++ views remain tied to the result owner.
- The private fixture duplicates a small descriptor and fixed observation intentionally. Its feature, symbol, declaration, asset package, and examples must remain visibly isolated from release packaging.
- No ONNX dependency, default wiring, support claim, or quality claim is added by this decision.

## Verification

- `crates/automation/runtime/tests/ocr_orchestration.rs` covers exact-frame correlation, deadline and close arbitration, retained-result independence, redacted diagnostics, and full-queue loss.
- `crates/bindings/capi/src/ocr_tests.rs` covers initialized outputs, invalid inputs, panic containment, concurrent close, and independent result ownership.
- `cargo run -p mado-pilot-capi --features private-fixture --example c-abi-check` compiles and runs current C/C++ examples, frozen ABI 1.0 and 1.2 callers, layout comparison, ownership checks, CMake consumers, and the isolated OCR fixture flows.
- `crates/bindings/capi/tests/cpp/madopilot-cpp-ownership.cpp` proves request-projection rebinding and negotiated-prefix refusal before missing entries are read.
- Change evidence is retained under `rasen/changes/phase-3-ocr-public-surfaces/evidence/`.
