# Rust OCR text-presence queries

`Session::start_ocr_text_watch` waits for literal text inside a coordinate-qualified
region of a maintained capture session. It is a Rust-only surface (C ABI 1.5 is
unchanged).

**Current verification scope:**

- **Deterministic contracts and real-CPU replay:** Controlled API smoke,
  Unicode 17.0.0 normalization bounds, and fixed real-CPU replay on both
  `x86_64-pc-windows-msvc` and `aarch64-apple-darwin` are verified.
- **Owned-fixture native capture procedures:** Fixed-cohort native capture
  procedures have recorded passes on Windows WGC and Apple ScreenCaptureKit; see
  the [Windows OCR text-watch procedure](ocr-text-watch-windows.md) and
  [Apple OCR text-watch procedure](ocr-text-watch-macos.md) for their exact
  cohort bindings and scenarios. General native capture qualification across
  broader workloads and unexecuted candidate-authority steps remains open.
- **Model choice and OS support:** Per-model quality and timing results inform
  caller selection and tuning without filtering available profiles or altering OS
  support floors; see the [OS support policy](architecture.md#os-support-policy).

## Selection and request

Initialize an engine with an OCR backend using the existing facade constructors:

- `replay_engine_with_ocr_profile`, `windows_engine_with_ocr_profile`, or
  `macos_engine_with_ocr_profile`: Accepts `OcrProfileConfig::from_id` with any
  profile from `ocr_profiles()` / `ocr_profile(id)` (including the `G-004`
  default, `OcrProfile::BoundedDetector` / bounded-v2, and the PP-OCRv6
  tiny/small profile; see the [OCR model catalog](ocr-model-catalog.md)).
- `replay_engine_with_ocr_provider`, `windows_engine_with_ocr_provider`, or
  `macos_engine_with_ocr_provider`: Accepts `OcrProviderConfig::new` with an
  `OcrProviderProfile` and `OcrExecutionProviderPolicy`.

`start_ocr_text_watch` requires an initialized BGRA OCR backend with provider
metadata, captures the complete backend/model/profile/provider/CPU-option
identity at start, and re-verifies that exact identity before every analysis or
change skip without switching profiles or falling back.

### CPU execution options

Explicit Rust profile configurations accept `OcrProfileConfig::with_cpu_options`:
`OcrCpuOptions::new(intra_threads)` admits `1..=8` intra-op threads, and
`with_spinning(bool)` controls worker spinning (default is `1` thread with
spinning enabled; `DefaultOcrConfig` and provider-policy constructors keep that
default). Each `(intra_threads, allow_spinning)` pair produces a distinct
runtime profile identity:

```rust
use mado_pilot::{OcrCpuOptions, OcrProfileConfig};

fn selection() -> Result<OcrProfileConfig, mado_pilot::Error> {
    Ok(OcrProfileConfig::from_id(
        "ppocrv6-tiny-det-small-rec-max736-v1",
        "/controlled/models",
        "/controlled/runtime/libonnxruntime.1.29.0.dylib",
    )?
    .with_cpu_options(OcrCpuOptions::new(8)?.with_spinning(false)))
}
```

One inter-op thread, sequential execution, disabled memory patterns, and one
physical OCR analysis lease remain fixed. See the
[Apple Silicon CPU-options comparison](benchmarks/ocr-models/cpu-options-aarch64-apple-darwin-20261022.md)
for measured stage timings, whole-process memory, and recognition trade-offs
between default (`1` thread, spinning) and `8` threads / non-spinning across 11
images.

### Model-specific trade-offs

Select a profile and CPU configuration based on measured latency, memory, and
recognition accuracy for the target workload. Comparisons must record the exact
profile, provider, CPU options, target, source/runtime digests, and ROI against a
fixed oracle while keeping enforced runtime safety checks (hashes, identity
consistency, finite memory/process bounds, cancellation, and cleanup) mandatory.

### Request options

`OcrTextWatchRequest::new` requires:

- A non-empty coordinate-qualified `Rect`, `ClipPolicy`, and output coordinate space.
- A borrowed search literal (normalized at construction) and finite minimum
  confidence in `0.0..=1.0`.
- `OcrTextAnalysisRate::from_minimum_interval` (positive interval).
- `OcrTextStability::immediate()` or `OcrTextStability::consecutive(count)` (`count >= 1`).
- A closed `ChangeDetectionPolicy` and query-lifetime `OperationContext`.

Start performs no inference or blocking capture; mutable bounds and transforms
resolve against the first maintained frame.

## Literal and result semantics

- **Unicode 17.0.0 NFC + edge-trim normalization:** Search literals are NFC-normalized
  and Unicode edge-whitespace trimmed, retaining `1..=4,096` UTF-8 bytes.
  Preflight bounds working storage at `16,384` decomposed scalars (proven
  sufficient across all `1,112,064` Unicode 17.0.0 scalars, whose maximum
  canonical decomposition is `4` scalars, with edge-trim commuting with NFC).
- **Substring matching and result ownership:** Presence requires a case-sensitive
  substring match within a single normalized OCR region at `confidence >= minimum_confidence`
  (no cross-region concatenation, case/width folding, or fuzzy matching). A
  matched `OcrTextWatchResult` retains the exact target, frame, transform,
  effective ROI, immutable `OcrResult`, descriptors, predicate, confirmation
  count, and `u16` satisfying-region indices independently of query, session, and
  engine teardown. `Debug`, errors, and diagnostics redact recognized text and
  search literals.

## Progress, authority and finite scheduling

- **Query handles and terminal precedence:** `poll` is non-blocking, `wait` uses
  an independent caller-wait `OperationContext`, and `cancel` (or dropping the
  sole handle) is idempotent. Latched source/session authority precedes
  scheduler shutdown; for open queries, `Cancelled` > `DeadlineExceeded` >
  competing overload, backend failure, or match. Only distinct accepted positive
  analyses advance consecutive stability; epoch or geometry changes reset
  confirmation.
- **Shared watcher bounds and OCR mapping barrier:** Shares the `WatchRuntime`
  bounds (`256` engine queries, `64`/session, `16` sessions, `2` analysis workers,
  `1` latest pending frame/query, `64 MiB`/`256`-entry mapping cache, `30 s`
  eligible queue residence, and `256 MiB` OCR mapping ceiling). At most one
  physical OCR lease runs at a time. A mapping admission barrier drains active
  template mappings before OCR native conversion starts and releases once OCR
  inference begins so the second worker can continue running template queries.
  Compatible BGRA rows use direct byte-equality change detection without an RGBA
  conversion copy. Callers inspect profile detector tensor bounds via
  `OnnxOcrBackend::facts().max_detector_tensor_bytes()`.

## Logical close is not physical quiescence

Cancellation and logical close seal dispatch and wake waiters immediately without
blocking on an in-flight native ONNX inference call. Only the worker holding the
active physical OCR lease may outlive final Rust handle drop until the native
call returns; callers must keep executing code and loaded native dependencies
alive until physical completion.

## Fixed real-replay procedure

Run the deterministic model-free OCR text-query verification suite first using
fresh, isolated Cargo target roots (on Windows, pass the `--opencv-root` and
`--libclang-path` setup options from
[CONTRIBUTING.md](../CONTRIBUTING.md#native-development-prerequisites)):

```sh
: "${CARGO_TARGET_DIR:?set a new deterministic target root}"
: "${OCR_NORMALIZATION_PROOF_TARGET:?set a separate new proof target root}"
: "${OCR_QUALIFICATION_CHECK_TARGET:?set a separate new qualification-feature check root}"
cargo test --locked --package mado-pilot-ocr --lib
cargo run --locked --release --package mado-pilot-ocr --example ocr-normalization-bound --target-dir "$OCR_NORMALIZATION_PROOF_TARGET"
python3 tools/setup-native.py -- cargo test --locked --package mado-pilot-runtime --tests
python3 tools/setup-native.py -- cargo test --locked --package mado-pilot --example ocr-text-watch
python3 tools/setup-native.py -- cargo test --locked --package mado-pilot --features ocr-text-watch-qualification --example ocr-text-watch --target-dir "$OCR_QUALIFICATION_CHECK_TARGET"
python3 -m unittest discover -s tools/ocr-text-watch/tests -v
```

`tools/ocr-text-watch/tests` uses inert files and an owned Python child (no ONNX
or capture), and the two `--example ocr-text-watch` test configurations verify
the independent Retina oracle and qualification-only stage-record rejection
without executing the real-model `main`.

[`crates/mado-pilot/examples/ocr-text-watch.rs`](../crates/mado-pilot/examples/ocr-text-watch.rs)
exercises the explicit CPU bounded-v2 profile across three fixed replay
scenarios: blank-to-positive (`sequence 1`, `epoch/geometry 0`), negative
final-source drain, and 2× target-logical projection.

1. **Generate the deterministic replay corpus** (requires Python 3.13+ with
   Pillow 12.3.0; converts the Apache-2.0 / OFL-1.1 repository HUD PNG without
   inference or overwriting):
   ```sh
   : "${OCR_REPLAY_CORPUS:?choose a new corpus directory}"
   python tools/ocr-text-watch/replay_corpus.py --output "$OCR_REPLAY_CORPUS"
   ```
2. **Execute the three-process real-CPU replay cohort** (with a dedicated
   `CARGO_TARGET_DIR`, `MADO_PILOT_G004_MODEL_ROOT`, `MADO_PILOT_ONNX_RUNTIME`,
   and a prospectively approved native-image manifest of `<= 256` canonical
   path-to-SHA-256 entries; 90 s process + 10 s cleanup + 64 KiB output bounds):
   ```sh
   : "${OCR_REPLAY_EXAMPLE:?exact built example required}"
   : "${OCR_REPLAY_CORPUS:?fixed generated corpus required}"
   : "${OCR_NATIVE_IMAGES:?prospectively approved native image manifest required}"
   : "${OCR_REPLAY_EVIDENCE:?new private evidence directory required}"
   python tools/setup-native.py -- python tools/ocr-text-watch/run_replay.py \
     --executable "$OCR_REPLAY_EXAMPLE" --corpus "$OCR_REPLAY_CORPUS" \
     --native-images "$OCR_NATIVE_IMAGES" --output "$OCR_REPLAY_EVIDENCE" --execute
   ```
   On Windows, identity keys use resolved extended-length paths (`\\?\...`), and
   presence or absence of the canonical OS-system `apphelp.dll` (resolved via
   `GetSystemDirectoryW`) is recorded in `os_managed_presence_exclusions` without
   failing symmetric image-set equality (all common image SHA-256 digests must
   still match).
3. **One-shot image-binding discovery (when creating a new image manifest for
   independent review):**
   ```sh
   : "${OCR_BINDING_EVIDENCE:?new private binding evidence directory required}"
   python tools/setup-native.py -- python tools/ocr-text-watch/bind_replay.py --mode real-cpu \
     --executable "$OCR_REPLAY_EXAMPLE" --corpus "$OCR_REPLAY_CORPUS" \
     --output "$OCR_BINDING_EVIDENCE" --execute-binding
   ```
   For the model-free controlled benchmark executable (`300 s` process, `15 s`
   cleanup, `1 MiB` output):
   ```sh
   : "${OCR_CONTROLLED_EXAMPLE:?exact controlled workload executable required}"
   : "${OCR_CONTROLLED_BINDING_EVIDENCE:?new private binding directory required}"
   python tools/setup-native.py -- python tools/ocr-text-watch/bind_replay.py --mode controlled \
     --executable "$OCR_CONTROLLED_EXAMPLE" --output "$OCR_CONTROLLED_BINDING_EVIDENCE" \
     --execute-binding
   ```

## Model-free workload apparatus

`ocr-text-watch-query` runs 6 controlled workloads (2 warm-ups, 20 samples each)
plus 1 controlled startup sample over generated solid BGRA frames and scripted
OCR (`300 s` watchdog):

```sh
: "${CARGO_TARGET_DIR:?set a new semantic-check target root}"
python tools/setup-native.py -- cargo test --locked --package mado-pilot \
  --bench ocr-text-watch-query -- --test
```

`tools/ocr-text-watch/workloads.py` runs 3 controlled processes (or 5 fresh
`real-cpu-cold-startup` processes built with `--features ocr-text-watch-qualification`)
and emits **Report schema 5**, defined in full in the
[OCR-watch measurement method](performance/methods/ocr-watch.md):

- **Execution vs enforcement fields:** `execution_passed` judges whether all
  attempted rows passed semantic, cleanup, dependency, and required-measurement
  checks; `semantic_cohort_passed` and `required_measurements_complete` require
  all planned rows to finish; `observation_completed` is `true` when a
  non-enforcing run completes all planned rows; `measurement_gaps` records
  unavailable target-applicable OS memory fields (`<= 399` entries/controlled
  process, `<= 36`/real-startup process) as `null` without failing ordinary
  observation; `empirical_targets` (`requested`, `misses`, `unavailable`,
  `passed`) and top-level `passed` / `numeric_budgets` / `stopped_by_enforcement`
  track optional `--profile` and `--enforce-budgets` grading.
- **Format-3 vs historical format-2 profiles:** Current target-specific profiles
  ([`ocr-text-watch-v3-aarch64-apple-darwin.toml`](benchmarks/ocr-text-watch-v3-aarch64-apple-darwin.toml)
  and [`ocr-text-watch-v3-x86_64-pc-windows-msvc.toml`](benchmarks/ocr-text-watch-v3-x86_64-pc-windows-msvc.toml))
  use `format_version = 3` ([tuning record](performance/tuning/2026-10-26-ocr-watch-profile-authority.md)).
  Historical `format_version = 2` profiles are read-only and inspected with:
  ```sh
  python3 tools/ocr-text-watch/workloads.py --inspect-profile docs/benchmarks/ocr-text-watch-aarch64-apple-darwin.toml
  ```

## Native qualification and numerical outcomes

Target-owned native OCR watcher procedures use dedicated fixtures and
acknowledged text transitions:

- **Windows WGC procedure:** See [ocr-text-watch-windows.md](ocr-text-watch-windows.md)
  for the fixed 5-endpoint WGC procedure and its recorded cohort pass at
  `fd98dba`.
- **Apple ScreenCaptureKit procedure:** See [ocr-text-watch-macos.md](ocr-text-watch-macos.md)
  for the v3 9-row ScreenCaptureKit procedure (including window-loss and
  process-reap rediscovery) and its recorded cohort pass at `86a482d`.
- **Historical format-2 workload enforcement:** In the historical format-2
  profiles ([Apple](benchmarks/ocr-text-watch-aarch64-apple-darwin.toml),
  [Windows](benchmarks/ocr-text-watch-x86_64-pc-windows-msvc.toml)), Apple
  `controlled3`/`real5` and Windows `controlled3` passed, while Windows `real`
  process 1 measured `1.4887 ms` against the `1.0 ms`
  logical-close-to-physical-zero ceiling (`NONPASS`, leaving `real` 2–5
  unexecuted). Those historical verdicts remain unchanged while current
  development uses format-3 profiles and descriptive observations.
