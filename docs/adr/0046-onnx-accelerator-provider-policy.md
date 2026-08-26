# ADR 0046: ONNX accelerator provider policy

- **Status:** Accepted
- **Date:** 2026-08-26
- **Resolves gate:** provider-policy mechanics under `G-006`; support and budgets are decided by ADRs 0047 and 0048
- **Supersedes:** _none_; ADRs 0033–0045 and every CPU profile/default remain unchanged

## Context

MadoPilot's accepted ONNX OCR backend registers only the built-in CPU execution provider. The approved Apple runtime already exports CoreML registration and links CoreML/Metal. The approved Windows RTX 4080 host initially lacked a controlled CUDA provider set; after this policy and its qualification plan were fixed, the operator authorized CUDA 13/cuBLAS/cuDNN 9 installation and user-local ORT GPU acquisition for qualification. The provider expansion must preserve CPU defaults and exact evidence, prevent ambient native loading, and distinguish initialization fallback from ONNX Runtime's internal graph partition.

The policy and qualification plan are fixed before provider execution under [`../../rasen/changes/phase-3-1-accelerated-onnx-provider-policy/`](../../rasen/changes/phase-3-1-accelerated-onnx-provider-policy/). No accelerator support or performance claim follows from this record alone.

## Decision

Add a closed provider-policy axis independent from OCR model/preprocessing profile:

```text
Cpu
AutoPreferAccelerator
PreferCuda
RequireCuda
PreferCoreMl
RequireCoreMl
```

Existing constructors remain CPU-only. `AutoPreferAccelerator` selects only a target accelerator accepted for automatic use by an evidence ADR; otherwise it selects CPU without claiming initialization fallback. ADR 0047 rejects CoreML. ADR 0048 accepts explicit Windows CUDA but rejects automatic preference on the fixed RSS ratio. Automatic selection is therefore CPU on both v0.3.1 release targets.

Preferred-provider fallback occurs only during one atomic pre-publication initialization transaction. MadoPilot registers the accelerator fail-closed, constructs and validates both detector and recognizer sessions with that provider, and publishes only the complete pair. On eligible failure it destroys the entire candidate and constructs a fresh CPU pair. Required policy returns the typed failure. Cancellation or deadline prevents fallback.

A published backend never changes provider. Inference failure, cancellation, device loss, or native error does not retry on CPU. Detector and recognizer cannot use different active providers.

Facts and public descriptors distinguish requested policy, active provider, initialization fallback, bounded fallback reason, and provider runtime-profile identity. Registration is not proof of acceleration: qualification privately profiles graph assignment and requires nonzero accelerator work for detector and recognizer. The released diagnostic stream remains unchanged. A required-provider failure publishes no engine or reader, and appending provider fields to the frozen ABI 1.4 diagnostic record would violate prefix preservation; typed construction errors and engine-owned descriptors are the complete ordinary observation surfaces.

CoreML uses the explicit runtime plus system frameworks. CUDA uses one explicit canonical provider root containing the admitted ORT CUDA/shared and CUDA 13/cuBLAS/cuDNN 9/NVRTC regular files. There is no download, installation, `PATH` mutation, Python/PyTorch preload, system inbox ORT, link admission, or ambient search in product code. The CUDA dependency DLLs load from the explicit root; ONNX Runtime loads its provider DLL only after environment initialization.

Provider code is target-gated behind `coreml-provider` and `cuda-provider` Cargo features. Missing build capability is observable and follows preferred/required semantics. CPU-only feature graphs remain unchanged.

C ABI 1.5 appends new provider policy/options/descriptor contracts plus `engine_create_with_ocr_provider` and `engine_ocr_provider_descriptor` after the complete 720-byte ABI 1.4 prefix. Existing records/functions do not gain fields or change behavior. Compiled Rust, Clang, and MSVC probes fix 32-byte provider options, a 40-byte descriptor, entry offsets 720/728, and the complete 736-byte table. C++ owns/rebinds provider options and exposes owner-bound typed descriptor views.

## Alternatives

- **Retry failed accelerator inference on CPU.** Rejected because it doubles work, obscures deadline/cancellation and run counts, complicates cleanup, and can publish results after an accelerator failure.
- **Let ONNX Runtime silently register/fall back.** Rejected because registration failure and internal node partition would become indistinguishable and provider claims unauditable.
- **Configure providers through environment variables.** Rejected because provider selection, dependency ownership, and fallback would become process-global and non-local.
- **Use a generic ordered provider list.** Rejected because v0.3.1 supports one target accelerator plus CPU; a list exposes ordering and invalid combinations without additional capability.
- **Modify ABI 1.4 profile options or engine descriptor.** Rejected because released record size/alignment and old caller behavior are permanent. New ABI 1.5 records/entries preserve the prefix.
- **Emit required-provider failure through the engine diagnostic stream.** Rejected because that failure publishes no engine or reader. Adding a separate diagnostic owner output or growing the frozen ABI 1.4 diagnostic record would expand and contradict the two-entry ABI 1.5 decision; the typed construction error is authoritative.
- **Use ambient CUDA Toolkit/Python dependencies.** Rejected because unrestricted DLL search is unsafe and irreproducible even when a compatible toolkit is installed; qualification and product construction use the explicit controlled root.
- **Make acceleration default before measurement.** Rejected because CoreML/CUDA may be slower, alter OCR thresholds, or place little work on the accelerator.

## Consequences

Integrators using existing constructors change nothing. New callers can request preferred or required acceleration and inspect the outcome. Preferred initialization may take longer because a failed accelerator attempt precedes fresh CPU construction; that duration and cleanup become qualification gates.

Provider-capable source builds add target feature and native prerequisite obligations, but source releases bundle none of those dependencies. CUDA qualification uses the separately authorized controlled host installation and user-local ORT GPU archive; reproductions require equivalent explicit acquisition. ADR 0047 rejects CoreML support and assigns no budget. ADR 0048 accepts exact-boundary explicit CUDA and rejects automatic preference; three new provider profiles leave every historical CPU profile unchanged.

ABI 1.5 values, record prefixes, and function order become permanent once released. Provider diagnostics and descriptors must remain content-redacted. Historical CPU benchmark files and identities remain immutable.

## Verification

- Deterministic tests inject every dependency/registration/session/graph failure, terminal interruption, partial-construction drop, repeated close, and inference failure through the same construction seam used by product code.
- Rust, C, C++, old-header, layout, ownership, panic, invalid-input, concurrency, diagnostic, and independent CMake matrices cover CPU, preferred fallback, and required failure.
- Approved Apple and Windows evidence proves real provider registration/placement, exact output, performance/memory, cancellation, and cleanup; ADRs 0047 and 0048 own the resulting support decisions.
- [`qualification-plan.md`](../../rasen/changes/phase-3-1-accelerated-onnx-provider-policy/evidence/qualification-plan.md) fixes process counts, workloads, hard gates, automatic-preference criteria, and privacy exclusions before execution.
- Any failed provider source/process remains retained without retry, exclusion, cross-target inference, or oracle relaxation.
