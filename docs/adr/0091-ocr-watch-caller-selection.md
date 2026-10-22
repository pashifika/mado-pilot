# ADR 0091: OCR watch caller selection

- **Status:** Accepted; implemented
- **Date:** 2026-10-22
- **Resolves gate:** none; native qualification and numerical rule outcomes remain separately reported
- **Supersedes:** ADR 0084's bounded-v2 watch-scope exception only, and the architecture's CPU bounded-v2 watcher-admission policy

## Context

[ADR 0084](0084-ocr-model-choices-and-test-results.md) separates implemented model
choices from empirical quality and timing outcomes, while explicitly leaving
bounded-v2 text-watch scope unchanged. A caller could therefore initialize a
supported OCR selection for one-shot use, yet the watcher rejected it solely
because it was not the fixed CPU bounded-v2 tuple. A configured G-004 regression
reproduced that rejection before this change.

The watcher needs a compatible initialized backend and stable execution identity,
not another model-quality or performance-admission policy. Failed or missing
empirical evidence remains useful to the caller but is not equivalent to a
technical contract refusal.

## Decision

`mado-pilot-runtime` text watching reuses the caller's initialized BGRA OCR
model/profile/provider. The adapter and existing facade constructors still own
supported initialization; the watcher adds no model, CPU, backend/version or
runtime-name whitelist.

- Missing OCR remains unavailable. Missing provider metadata or non-BGRA input
  refuses; ordinary request, geometry and OCR validation remain mandatory.
- Capture the complete backend/model/profile and provider descriptors. Before
  each analysis or change skip, recheck the provider and prepared backend
  descriptors against that captured selection. Drift fails without inference or
  an unchanged-result shortcut.
- Keep exact source/result correlation, finite scheduling, execution/resource
  bounds, deadline/cancellation and owned cleanup. Empirical quality, latency
  and RSS outcomes do not replace these technical safety contracts.
- Caller selection changes no default, model resource, provider capability or
  scheduler capacity. A query never changes selection or falls back. Existing
  preferred-provider fallback remains initialization-only; required-provider
  initialization failure still publishes no engine. [ADR 0092](0092-ocr-cpu-execution-options.md)
  separately adds explicit bounded CPU options to Rust profile construction,
  with distinct execution identities and unchanged default/provider-policy paths.
- Remove the unreleased
  `OcrTextSchedulerDescriptor::max_detector_input_tensor_bytes` accessor. Its
  bounded-v2 constant is not a scheduler limit for other initialized profiles.
  Tensor limits belong to the selected backend's resource facts, not a
  profile-independent watcher policy.
- Treat numerical performance and empirical recognition-quality rules as
  optimization targets and caller information, not conditions for model
  availability or continued implementation. Tune each model only as supported
  by measurements; different models may need different configurations.
  Compare latency, memory and recognition outcomes for the exact measured
  model/profile/provider and workload, rather than require one threshold for
  every model or recommend the fastest model without its trade-offs.
  Unmeasured platform/configuration rows remain unmeasured.

This extends ADR 0084's caller-choice policy to watching; its other decisions and
historical text remain unchanged. It does not make arbitrary model/provider
combinations constructible or qualify a real accelerator.

## Alternatives

- Keep the CPU bounded-v2 whitelist: duplicates adapter-owned selection and
  rejects an already initialized compatible choice without a technical reason.
- Admit models only after every quality/timing rule passes: reinstates the
  empirical choice filter rejected by ADR 0084.
- Generalize the fixed qualification workloads or change their thresholds:
  unnecessary for caller selection and would lose their original applicability.

## Consequences

Callers use the existing profile/provider constructors documented in the
[selection guide](../ocr-text-watch.md#selection-and-request). They choose the
model and performance/quality trade-offs; successful initialization does not
promise recognition quality, latency or arbitrary application compatibility.
Provider availability remains exactly what those constructors implement.

Code using the removed accessor must use the selected adapter's resource facts;
the ONNX adapter exposes `OnnxOcrBackend::facts().max_detector_tensor_bytes()`.
No replacement watcher constant or compatibility shim is provided. Existing
repository consumers do not use the removed accessor.

The fixed CPU bounded-v2 replay/native procedures remain fixed. [ADR
0075](0075-ocr-text-watch-workload-profiles.md), its thresholds, failed attempts,
profile pins and original qualification results are unchanged and source-bound.
A historical numerical `NONPASS` remains `NONPASS`; it is not a current
implementation blocker or a model-selection filter. New tuning and comparative
reports need their own source, configuration and measurement bindings and do
not regrade that evidence.

Native WGC/ScreenCaptureKit qualification remains unresolved. Permissions,
no implicit input, exact identities, valid runtime/output contracts, finite
memory/process bounds, cancellation and physical cleanup remain mandatory;
their violations are defects, not performance trade-offs. A statistical timing
or RSS target is distinct from these enforced safety and lifecycle bounds.
The guide, architecture baseline and development release notes reflect this
distinction.

## Verification

The permanent [runtime contract tests](../../crates/automation/runtime/tests/ocr_text_watch.rs)
cover every catalog profile independently of evaluation, configured backend and
provider identities, missing metadata/non-BGRA refusals, exact retained
identities, and provider/backend drift before initial or subsequent analysis and
change skipping. Controlled CUDA/CoreML descriptors prove selection semantics,
not real accelerator execution.

A real CPU replay smoke on 2026-10-22 used ONNX Runtime 1.29.0 and the retained
repository replay corpus on Apple Silicon macOS 27.0.1 (26A434). Both
`g-004-rapidocr-ppocrv4-det-v6-rec-small-v1` and
`phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v2` completed with
`terminal=Matched source_sequence=1 retained_pixels=exact parents=closed`; the
process exited 0. It performed no capture or input. This observation supports
caller selection and retained-result behavior only: it is not native capture,
macOS qualification, accelerator proof, an ADR 0075 budget run or a numerical
regrading of historical evidence.
