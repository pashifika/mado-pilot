# ADR 0092: Explicit OCR CPU execution options

- **Status:** Accepted
- **Date:** 2026-10-22
- **Resolves gate:** none
- **Supersedes:** the fixed one-thread CPU policy for explicit Rust profile construction only

## Context

[ADR 0091](0091-ocr-watch-caller-selection.md) preserves the caller's initialized
OCR selection and separates empirical performance from technical admission.
Capture-free HUD probes on a 10-core Apple M1 Pro with macOS 27.0.1 and optimized
ONNX Runtime 1.29.0 found lower p95 latency with eight intra-op threads and
spinning disabled than with the one-thread, spinning-enabled baseline for the
three tested model profiles. Each configuration used one fresh process per
profile, two warmups and twenty samples for each singular/grouped workload.
This motivates explicit bounded CPU configuration, not a new default or a
universal optimum. Full-corpus current-source comparison and native
qualification are separate evidence obligations.

## Decision

Expose executor-neutral `OcrCpuOptions` through the Rust OCR contract, runtime
and facade. `OcrCpuOptions::new(intra_threads)` returns the core `Result` and
accepts every count in `1..=8`; `with_spinning(bool)` selects native worker
spinning. `intra_threads()` and `allow_spinning()` expose the selection. Defaults
remain one intra-op thread with spinning enabled.

`OcrProfileConfig::with_cpu_options` and `cpu_options()` carry this immutable
initialization choice into the existing replay, Windows and macOS
`*_engine_with_ocr_profile` constructors. The ONNX adapter's
`open_profile_with_cpu_options` applies it to both detector and recognizer
sessions. Existing `DefaultOcrConfig`, provider-policy constructors and legacy
adapter entry points retain one thread with spinning enabled. No environment or
process-global tuning, per-query reconfiguration, fallback or C ABI addition is
introduced.

Keep one inter-op thread, sequential execution, disabled memory patterns, the
single detector/recognizer pair and one physical OCR analysis unchanged. The
maximum of eight bounds native CPU workers; it is not a benchmark-admission
threshold. Counts three, five, six and seven are available without measured
performance claims. Every thread-count/spinning pair has a distinct runtime
profile identity, retained and rechecked by the watcher. Model hashes, graph
validation, tensor/resource bounds, cancellation and cleanup remain mandatory.

## Alternatives

- Change the default to the fastest observed configuration: a single host/HUD
  probe does not establish application-wide latency, memory or recognition
  trade-offs and would change existing consumers implicitly.
- Expose only experimentally sampled counts: confuses empirical coverage with
  the finite native-worker safety bound.
- Use global or environment tuning: obscures per-engine identity and prevents
  independent explicit configurations.

## Consequences

Existing callers need no migration. Rust explicit-profile callers may choose
bounded CPU options before initialization and must compare their workload's
latency, memory and recognition results. More native workers can increase CPU
contention or memory, and disabling spinning can change wake-up latency; neither
choice promises a general improvement. The measured host example in the
[guide](../ocr-text-watch.md#cpu-execution-options) is not a new default.
No new provider, accelerator, dependency, packaging or platform support is added.
Historical workload profiles and qualification verdicts remain unchanged.

## Verification

The current-source checks passed 448 ordinary Rust tests with 9 opt-ins ignored,
all-target clippy, rustdoc and formatting. Six real public-facade watcher cases
covered every model at default and eight-thread/nonspinning settings, preserving
runtime identity and exact retained source pixels after teardown. Constructor
bounds and all 16 runtime-identity combinations have deterministic coverage.

The [final same-binary comparison](../benchmarks/ocr-models/cpu-options-aarch64-apple-darwin-20261022.md)
records 2640 measured calls across 11 images, both call modes, three models and
two configurations. Latency improved in the measured scope; G-004 peak RSS
increased, and model-specific recognition findings remained unchanged. All
measured calls and lifecycle checks returned their expected outcomes.
The Change retains source/artifact-bound raw observations, current-source
checks and independent review. These records establish neither native capture
nor Windows qualification and do not regrade ADR0075 evidence.
