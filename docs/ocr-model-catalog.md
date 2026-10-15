# OCR model catalog and admission

The SDK exposes accepted model choices and exact provisioning metadata without
opening models or loading ONNX Runtime. The default and the two accepted legacy
profiles are unchanged. `ppocrv6-tiny-det-small-rec-max736-v1` is an implemented,
unqualified CPU candidate, not a supported choice.

[ADR 0080](adr/0080-ocr-model-catalog-and-admission.md) fixes its execution contract.
Package manifest v1/v2, the C/C++ ABI, provider defaults and the bounded-v2 OCR text
watch gate are unchanged. A manifest that cannot express the candidate's required
auxiliary resource is refused, not interpreted with an implicit dictionary.

## Rust integration

```rust
use mado_pilot::{accepted_ocr_profiles, OcrProfileConfig};

fn main() -> Result<(), mado_pilot::Error> {
    for profile in accepted_ocr_profiles() {
        println!("{}: {}", profile.profile_id, profile.display_name);
        println!("{}", profile.detector.relative_path);
    }

    let selection = OcrProfileConfig::from_id(
        mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID,
        "/controlled/models",
        "/controlled/runtime/libonnxruntime.1.29.0.dylib",
    )?;
    assert_eq!(selection.profile_id(), mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID);
    Ok(())
}
```

Pass `selection` to the existing replay, Windows or macOS
`*_engine_with_ocr_profile` constructor. `from_id` validates selection without
resource I/O. Engine construction revalidates it, including manually constructed
`OcrProfile::Catalog` values. Unknown and unqualified IDs cannot reach model
loading through ordinary constructors, even in a build with maintainer features.

Descriptors are borrowed static data. `accepted_ocr_profile(id)` performs the
same accepted-only lookup. `DEFAULT_OCR_PROFILE_ID` identifies the unchanged
native G-004 default. Display names and variants are presentation, not identity;
applications persist the profile ID, not its label. Benchmark codes are scoped to
a target/provider and accompanied by criteria/report references when rated.
Existing accepted profiles are `unrated` under this new comparison policy; their
older support and revision-bound evidence remain valid.

`OcrProfileConfig::new` and `from_id` preserve `OcrProfile::BoundedDetector` for
the bounded catalog ID, keeping legacy pattern matching and configuration
equality consistent.

Direct users of `mado-pilot-backend-onnx` must handle the new
`OnnxOcrProfile::Catalog` variant in exhaustive matches and use
`OnnxBackendFacts::profile_id()` for exact identity. The facade's existing
`OcrProfile` enum was already non-exhaustive. This Rust backend enum extension
changes no C/C++ layout or discriminant.

## Source schema

The authority is
[`model-catalog.json`](../crates/automation/ocr/resources/model-catalog.json).
[`catalog_build.rs`](../crates/automation/ocr/catalog_build.rs) validates both
builds and the maintainer command before generating immutable Rust descriptors.
There is no runtime JSON parser, host-maintained support table or model-specific
constructor registry.

| Object | Contract |
|---|---|
| Root | `schema_version: 1`, `default_profile`, `resources`, `profiles`; at most 1 MiB, 128 resources and 64 profiles |
| Resource | Unique `id`, `kind` (`model` or `auxiliary`), safe `relative_path`, exact `byte_len`/`sha256`, immutable `source_url`/`source_revision`, reviewed `license`/`notice` |
| Profile | `status`, nullable `legacy_constants`, `execution`, `presentation`; old constant namespaces retain compatibility, not a second authority |
| Execution identity | `model_id`, `model_version`, `profile_id`, language/preprocessing/decoder/normalization IDs; detector/recognizer references and explicit nullable auxiliary reference |
| Graphs | Named float32 input/output tensors, ranks/shapes and finite axis limits; `-1` marks a dynamic axis |
| Detection | Implemented DB/resize/rounding/interpolation/inverse-geometry algorithms, exact pixel operation order, channel means/stds, thresholds and checked dimensions/bytes |
| Recognition | Original-source perspective crop, orientation, resize/padding, height/width/batch and normalization policy |
| Vocabulary | Explicit ONNX metadata or Paddle YAML source/key, canonical digest/count, class count and greedy-CTC rules |
| Limits/applicability | Finite tensors, contours, scanline queue and resource ceilings; exact target/provider/runtime/API scopes |
| Presentation | `display_name`, nullable `variant`, and scoped benchmark code/criteria/report references; no execution parameters |

Unknown/duplicate keys, missing required nullable fields, duplicate identities or
case-colliding paths, dangling references, unsafe paths, unreviewed licenses,
nonfinite/invalid parameters and unimplemented algorithms fail validation.
Nullable fields use explicit JSON `null`, not omission. A vocabulary source cannot
silently fall back to the other source. Execution digests include referenced
resources and execution fields, excluding display labels and verdict metadata.
The current backend requires a DB candidate ceiling of 1000 and a vocabulary of
18,708 entries plus CTC blank and terminal space (18,710 classes). The validator
rejects other values until their execution contract is implemented.

Run the same validator used by the build:

```bash
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate -- path/to/catalog.json
```

The first command prints execution digests. To obtain a reviewed profile's exact
resource rows, including declared candidates for maintainer provisioning:

```bash
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate -- \
  --resources ppocrv6-tiny-det-small-rec-max736-v1
```

Rows are TSV in detector, recognizer, auxiliary order:
`relative_path`, `byte_len`, `sha256`, `source_url`. The command never downloads.
Acquire those exact HTTPS revisions under the controlled root, verify length and
SHA-256, retain the source license/notice, and supply the root explicitly. Do not
rename a different export to these paths. The SDK rechecks captured bytes before
constructing sessions. It distributes neither model files nor ONNX Runtime.
The [native dependency guide](third-party-dependencies.md) supplies runtime and
OpenCV requirements.

## Adding or revising a model

1. Pin original resources, licenses, graph metadata and vocabulary strategy.
   Declare a distinct candidate identity; do not mutate an accepted execution.
2. Use implemented algorithms and finite bounds. An algorithm change requires
   implementation and a focused ADR before evaluating the new candidate.
3. Freeze independent licensed inputs and exact oracles before inference. Keep
   every unchanged G-004 v3 row, especially the previous tiny-detector tooltip
   miss. Never import private pixels/labels/digests into public fixtures.
4. Verify resource refusal, pixels, geometry, singular/grouped source correlation,
   cancellation/deadline/close and existing legacy/provider/watch contracts.
5. Freeze approved host/cohort policies, absolute budgets and required collectors
   before final measurement. Missing prerequisites are failures, not skips.
6. Run and retain independent Apple and Windows cohorts; review their complete
   reports. Only then may a catalog revision publish support and rated metadata.

## Repeatable Rust measurements

The existing `onnx-cpu` benchmark has an explicit `--model-admission` mode behind
`benchmark-instrumentation`. It selects the compiled descriptor; normal product
constructors remain accepted-only. Build in a dedicated target directory and do
not subsequently rebuild a hash-bound executable in that directory:

```bash
CARGO_TARGET_DIR=target/ocr-model-admission \
  python3 tools/setup-native.py -- cargo bench --locked \
  --config 'profile.bench.build-override.strip="none"' \
  -p mado-pilot-backend-onnx --features benchmark-instrumentation \
  --bench onnx-cpu --no-run
```

The build-only strip override avoids rustc 1.97.1's macOS dylib alignment bug;
it does not change benchmark optimization. Windows may omit it. See
[ADR 0081](adr/0081-current-apple-ocr-verification-cohort.md).

Use the emitted executable path as `OCR_BENCH`. Record its exact digest, source
revision/tree and controlled runtime identity. The command surface is:

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
python3 tools/ocr-models/run.py validate "$NEW_REPORT_DIRECTORY/report.json"
python3 tools/ocr-models/run.py admission "$APPLE_REPORT" "$WINDOWS_REPORT"
```

All referenced inputs are explicit. Output directories are new and never
overwritten. Reference and candidate run serially, three fresh processes each;
every corpus image has singular and grouped workloads with two warmups and twenty
retained samples. Intra/inter-op CPU threads are one. Reports preserve exact
observations, failures, commands, cold open/close, warm end-to-end and stage
timings, actual detector dimensions/bytes, Rust allocation/retention and native
process peak RSS. The validator recomputes quality, summaries and verdicts.
One report can establish only `target_qualified`, never global admission.
`admission` independently recomputes both target reports and reports eligibility;
it never mutates the catalog.

Validation requires the original JSONL files and the parent's independent
`.process.json` receipts, not only the JSON report. `raw_path` and `receipt_path`
are relative to the report directory; every retained file, digest, parsed
observation, invocation and external process outcome is reconciled before
classification. For relocated evidence,
use `validate --evidence-root DIR` or `admission --evidence-roots APPLE_DIR WINDOWS_DIR`.
`--repository REPO` selects the Git object store used to verify each policy's
repository-relative `path`, frozen commit, regular-file tree entry and blob bytes.
Rewriting embedded policy bytes or summary values cannot replace those originals.

The benchmark embeds its build revision, tree, compiler and dirty state behind
`benchmark-instrumentation`; ordinary SDK builds perform no Git collection.
Preflight and validation reconcile those facts against the requested source and
binding. An older binary with an unchanged catalog is not evidence for a newer
implementation. Dirty or unavailable build provenance cannot qualify.

Use `--preflight-only` to inspect admission without inference. Explicit
`--diagnostic` permits implementation observations with missing release
prerequisites, preserves those failures, and can never rate or admit a model.
Hosted CI is not a final performance cohort.
[ADR 0081](adr/0081-current-apple-ocr-verification-cohort.md) approves the current
macOS 27.0.1 Apple M1 Pro cohort through
[`host-policy-v1.json`](../tools/ocr-models/host-policy-v1.json), without relabeling
earlier failed diagnostics. A diagnostic exit status of `2` means no admission;
inspect the retained failures rather than treating it as successful qualification.

Absolute budgets have no implicit defaults. They name the exact cohort and
approval, per-process cold-open/close/RSS ceilings, every workload's p50/p95/max,
Rust peak/steady/growth, mapped bytes and detector tensor bytes. Required collectors
are `rust-accounting`, `native-peak-rss` and `onnx-stage-clocks`. The host policy
must independently allow the exact cohort. ADR 0081 fixes the reference-only
precursor and derives the complete Apple policy in
[`apple-m1-pro-macos27-budgets-v1.json`](../tools/ocr-models/apple-m1-pro-macos27-budgets-v1.json).
The precursor retains 132 exact-text failures and does not qualify the reference
or candidate. Windows policy/budgets and a common final source remain required.
Old ceilings for other workloads are not silently reused for compact-UI inputs.

The same precommitted policy binds `process_limits`: external process timeout,
output byte limit, cleanup deadline and admitted cancellation/deadline return
bound. The source/artifact/cohort binding is recorded before execution, not
committed into the source revision it identifies.

## Verdict policy

Every declared workload must pass exact NFC/trimmed text, count/order, IoU at
least 0.50, normalized center deltas at most 0.025, finite model-local confidence,
source correlation, same-implementation stability, resource/lifecycle and
absolute-budget gates. No fuzzy/case/width folding or cross-model confidence
threshold substitutes for correctness. All below-threshold observations remain.

- `reference`: the accepted, fully attributable comparison baseline.
- `fast`: every hard gate passes and both p50/p95 are at most 80% of the reference
  in every workload in the same target/provider/cohort.
- `balanced`: complete comparative evidence passes every hard gate but not the
  full `fast` condition.
- `unrated`: already accepted, with no applicable comparative report. Never a
  route to accepting a candidate.

A failing workload cannot be hidden by aggregation or another target's result.
A failed attempt stays immutable. A changed execution needs a new candidate
revision and frozen evaluation, not retries until a favorable run or rewritten
oracles. [The unchanged G-004 evidence](evidence/g-004/) retains its historical
sources, hashes and adverse observations.
