# OCR model choices and test results

MadoPilot exposes all three implemented OCR catalog profiles through static Rust
descriptors (`ocr_profiles()` and `ocr_profile(id)`) without loading model files
or ONNX Runtime. Model selection belongs to the caller: recorded quality or
timing failures are reported per rule alongside each profile rather than hiding a
choice, requiring an experimental feature flag, or changing the default
(`DEFAULT_OCR_PROFILE_ID` remains `g-004-rapidocr-ppocrv4-det-v6-rec-small-v1`),
while resource length/SHA-256, graph, vocabulary, and execution-bound validation
always apply.

## Rust integration

```rust
use mado_pilot::{ocr_profiles, OcrProfileConfig};

fn main() -> Result<(), mado_pilot::Error> {
    for profile in ocr_profiles() {
        println!("{}: {}", profile.profile_id, profile.display_name);
        for evaluation in profile.evaluations {
            for rule in evaluation.rules {
                println!(
                    "{} / {} / {}: {:?}",
                    evaluation.target, evaluation.provider, rule.rule_id, rule.outcome
                );
            }
        }
    }

    let selection = OcrProfileConfig::from_id(
        "ppocrv6-tiny-det-small-rec-max736-v1",
        "/controlled/models",
        "/controlled/runtime/libonnxruntime.1.29.0.dylib",
    )?;
    assert_eq!(selection.profile_id(), "ppocrv6-tiny-det-small-rec-max736-v1");
    Ok(())
}
```

Pass `selection` to `replay_engine_with_ocr_profile`,
`windows_engine_with_ocr_profile`, or `macos_engine_with_ocr_profile`:

- **Static descriptors and rule outcomes:** Each `evaluations` entry identifies a
  target, provider, criteria document, per-rule outcomes (`Passed`, `Failed`,
  `NotRun`, `Incomplete`), and optional source-bound `evidence` (report path,
  measured source revision, execution digest, evaluator revision). Persist the
  stable `profile_id` string rather than display labels.
- **CPU options and backend identity:** Explicit Rust profile configs can attach
  `OcrCpuOptions` (`1..=8` intra-op threads and spin control; see
  [CPU execution options](ocr-text-watch.md#cpu-execution-options)). Inspect the
  initialized runtime identity via `OnnxBackendFacts::runtime_profile_id()` and
  `profile_id()` (`OnnxOcrProfile::Catalog` in `mado-pilot-backend-onnx`).

Run the metadata-only example without models or ONNX Runtime:

```bash
python3 tools/setup-native.py -- cargo run --locked -p mado-pilot --example ocr-profiles
```

## Source schema

The catalog authority is
[`crates/automation/ocr/resources/model-catalog.json`](../crates/automation/ocr/resources/model-catalog.json),
validated at build time and by the maintainer CLI via
[`crates/automation/ocr/catalog_build.rs`](../crates/automation/ocr/catalog_build.rs)
to generate immutable Rust descriptors (no runtime JSON parsing):

| Object | Contract |
|---|---|
| Root | `schema_version: 2`, `default_profile`, `resources`, `profiles` (`<= 1 MiB`, `<= 128` resources, `<= 64` profiles) |
| Resource | Unique `id`, `kind` (`model` or `auxiliary`), normalized `relative_path`, exact `byte_len` and `sha256`, immutable `source_url`/`source_revision`, and reviewed `license`/`notice` |
| Profile | Nullable `legacy_constants`, `execution`, and `presentation` |
| Execution identity | `model_id`, `model_version`, `profile_id`, language/preprocessing/decoder/normalization IDs, detector/recognizer references, and nullable auxiliary reference (execution digest version 1 excludes presentation/evaluation fields) |
| Graphs, detection, recognition | Named `f32` input/output tensors and axis limits (`-1` dynamic); DB resize/threshold (`1,000` candidate cap) and original-source perspective-crop recognition parameters |
| Vocabulary & limits | Embedded ONNX metadata or Paddle YAML (`inference.yml`) vocabulary (`18,708` entries + CTC blank + space = `18,710` classes), plus finite tensor, contour, scanline, and target/provider/API limits |
| Presentation | `display_name`, nullable `variant`, and scoped `evaluations` |

Validate the catalog or print a profile's required resource TSV (`relative_path`,
`byte_len`, `sha256`, `source_url` in detector, recognizer, auxiliary order):

```bash
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate -- path/to/catalog.json
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate -- \
  --resources ppocrv6-tiny-det-small-rec-max736-v1
```

See [third-party dependencies](third-party-dependencies.md#implemented-onnx-runtime-prerequisite)
for ONNX Runtime 1.29.0 and model acquisition rules.

## Adding or revising a model

1. Pin upstream resources, licenses, graph metadata, and vocabulary under a new
   execution identity (never mutate an existing tuple).
2. Implement any new algorithm or bound before evaluation, adding an
   [ADR](adr/README.md) if changing a backend or technology choice.
3. Freeze licensed inputs and exact oracles (retaining all [`G-004`](evidence/g-004/) v3 rows,
   including the dense-tooltip case) before inference.
4. Verify resource refusal, geometry, singular/grouped correlation,
   cancellation/deadline/close, and watcher compatibility.
5. Freeze host/cohort policies ([`tools/ocr-models/host-policy-v1.json`](../tools/ocr-models/host-policy-v1.json)) and
   numerical budgets before running the evaluation campaign.
6. Retain all observations and publish scoped per-rule outcomes without hiding
   technically valid profiles that fail empirical quality or latency rules.

## Repeatable Rust measurements

Build the `onnx-cpu` benchmark in a dedicated `CARGO_TARGET_DIR` (the strip
override avoids `rustc 1.97.1`'s macOS dylib alignment bug without changing
optimization):

```bash
CARGO_TARGET_DIR=target/ocr-model-evaluation \
  python3 tools/setup-native.py -- cargo bench --locked \
  --config 'profile.bench.build-override.strip="none"' \
  -p mado-pilot-backend-onnx --features benchmark-instrumentation \
  --bench onnx-cpu --no-run
```

Run or evaluate a frozen-cohort campaign with `tools/ocr-models/run.py`:

```bash
python3 tools/ocr-models/run.py run --help
python3 tools/ocr-models/run.py run \
  --executable "$OCR_BENCH" --model-root "$MODEL_ROOT" \
  --runtime "$ONNX_RUNTIME" \
  --reference-profile g-004-rapidocr-ppocrv4-det-v6-rec-small-v1 \
  --profile ppocrv6-tiny-det-small-rec-max736-v1 \
  --frozen-revision "$FROZEN_REVISION" \
  --binding "$BINDING" --budgets "$BUDGETS" --host-policy "$HOST_POLICY" \
  --report "$NEW_REPORT_DIRECTORY" \
  --timeout-seconds "$PROCESS_DEADLINE" --output-limit-bytes "$OUTPUT_LIMIT" \
  --cleanup-seconds "$CLEANUP_DEADLINE" --lifecycle-bound-ms "$LIFECYCLE_BOUND_MS"
python3 tools/ocr-models/run.py evaluate "$NEW_REPORT_DIRECTORY/report.json" \
  --evidence-roots "$NEW_REPORT_DIRECTORY"
python3 tools/ocr-models/run.py evaluate "$APPLE_REPORT" "$WINDOWS_REPORT" \
  --evidence-roots "$APPLE_EVIDENCE_ROOT" "$WINDOWS_EVIDENCE_ROOT"
```

Campaign and checkpoint invariants:

- **Serial cohort structure:** Runs 3 fresh processes each for reference and
  candidate (1 intra-op / 1 inter-op thread, spinning enabled; 2 warm-ups and 20
  samples per image across singular and grouped workloads). On Windows, the
  frozen reference campaign reserves `900 s` and `33,554,432` output bytes per
  process, `15 s` cleanup, `2,000 ms` lifecycle bound, and `3,300 s` total
  ([`windows-i7-12700kf-26300-budgets-v1.json`](../tools/ocr-models/windows-i7-12700kf-26300-budgets-v1.json),
  calibrated at `0cb9fc0`; Apple budgets live in
  [`apple-m1-pro-macos27-budgets-v1.json`](../tools/ocr-models/apple-m1-pro-macos27-budgets-v1.json)).
- **Interrupt-safe journal & reconciliation:** Before each child launch, the
  runner checkpoints `attempted-unconfirmed`; after exit, it checkpoints the
  `.process.json` receipt (`raw-digest-pending` -> `receipt-only`) before parsing
  raw JSONL. On `KeyboardInterrupt`, `execution: "interrupted"` preserves the
  verified prefix without evaluating undigested raw bytes. `evaluate`
  reconciles `.json`, `.jsonl`, `.process.json`, and Git blob policies (`--repository`).
- **Diagnostic and preflight modes:** `--preflight-only` checks prerequisites
  without inference; `--diagnostic` runs bounded investigations without host
  policy or budgets and never qualifies a model.

## Rule interpretation

Defined in [`tools/ocr-models/criteria-v1.json`](../tools/ocr-models/criteria-v1.json):

- **Text equivalence:** Compares NFC/trimmed text treating only ASCII `%`
  (`U+0025`) and fullwidth `％` (`U+FF05`) as equivalent (no broader NFKC or
  case/width folding).
- **Evaluated rules:** Text/order, region count, geometry (`IoU >= 0.50` and
  normalized center delta `<= 0.025`), unexpected detections at `confidence >= 0.5`,
  finite confidence, stability, fixture-character coverage, source correlation,
  lifecycle, execution bounds, process/workload budgets, and relative latency
  (candidate `p50` and `p95 <= 80%` of reference in every workload).

## Derived results from retained measurements

[`benchmarks/ocr-models/cf2763a-evaluation-v1.json`](benchmarks/ocr-models/cf2763a-evaluation-v1.json)
records the `evaluate` CLI output over the retained Apple and Windows reports at
measured source `cf2763a41929ad89366dd77ee1b6a57503a8f6bc`: for
`ppocrv6-tiny-det-small-rec-max736-v1`, process/workload budgets and the `<= 80%`
relative-latency rule pass on both CPU targets, while text/order and region-count
rules fail on both targets and unexpected-detection fails on Windows (`Bounded-v2`
and `CUDA` are `NotRun` under this suite).

## Final comparison at cf2763a

Historical one-shot comparison record at source
`cf2763a41929ad89366dd77ee1b6a57503a8f6bc` (tree
`518ca130aa8379ba000da1cd7fc6df160cc5830c`; 3 reference + 3 candidate processes
per target, 11 images / 22 workloads, 264 warm-ups, 2,640 measured observations;
packet SHA-256 `1977d71183b502716d680258e8d9a0007934101b9ed4da2ee02fc2c558757515`
on Apple and `62e9f64973fe3bd0068892dc776518ef7f90d365ddd862b93ddfbacff995a5fc`
on Windows):

| Target | Observations | Candidate quality-failure entries | Distinct failed observations | Negative-image confidence |
|---|---:|---:|---:|---:|
| `aarch64-apple-darwin` | 2,904 | 660 | 396 | 0.49752 |
| `x86_64-pc-windows-msvc` | 2,904 | 792 | 396 | 0.51158 |

Both targets misread `[OK]` -> `[0K]`, `一点照準` -> `点照準`, `追加攻撃35%` ->
`追加攻擊35%`, and `唯一` -> `唯`, and produced an extra `●` on the non-uniform
negative image (exceeding the `0.5` unmatched-detection threshold on Windows),
while 264 percent-width-only comparisons passed the `%`/`％` equivalence rule and
all reference quality, numerical, identity, and cleanup gates passed. At that
revision, the legacy `admission` CLI exited `2` (`admitted: false`); the later
catalog v2 cutover exposed all three profiles as ordinary caller choices while
preserving these exact quality findings.
