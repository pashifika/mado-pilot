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
earlier failed diagnostics. [ADR 0082](adr/0082-current-windows-ocr-reference-precursor.md)
adds the independently identified Windows 11 Pro 26H2 CPU cohort; runtime and
model qualification remain separate. A diagnostic exit status of `2` means no
admission; inspect retained failures rather than treating it as qualification.

Absolute budgets have no implicit defaults. They name the exact cohort and
approval, per-process cold-open/close/RSS ceilings, every workload's p50/p95/max,
Rust peak/steady/growth, mapped bytes and detector tensor bytes. Required collectors
are `rust-accounting`, `native-peak-rss` and `onnx-stage-clocks`. The host policy
must independently allow the exact cohort. ADR 0081 fixes the reference-only
precursor and derives the complete Apple policy in
[`apple-m1-pro-macos27-budgets-v1.json`](../tools/ocr-models/apple-m1-pro-macos27-budgets-v1.json).
The precursor retains 132 exact-text failures and does not qualify the reference
or candidate. The Windows calibration at source `0cb9fc0` completed three
processes and 1,452 observations with zero shared validation failures. Its
independently calibrated numerical policy is
[`windows-i7-12700kf-26300-budgets-v1.json`](../tools/ocr-models/windows-i7-12700kf-26300-budgets-v1.json),
derived under ADR 0082. Calibration never qualifies either model. The final
common-source comparisons completed with candidate quality failures on both
targets; the source-bound refusal is recorded below.
[ADR 0083](adr/0083-platform-aware-ocr-calibration-and-percent-equivalence.md)
supersedes only ADR 0082's reference operational allowance for the separately frozen
campaign: exactly three fresh serial processes, 900 seconds and 33,554,432 output
bytes each, 15-second cleanup, 2,000-ms lifecycle bound and 3,300 seconds whole
(3,285 seconds primary plus 15 seconds cleanup). Before each launch reserve the
full process/cleanup allowances and 30 seconds for validation within primary time.
Stop later launches on operational, input/source, collector or lifecycle failure;
do not increment deadlines, automatically retry or change security software.
This is platform-specific headroom, not a measured security-software cost or a
Windows model-performance budget. Apple bounds, budget bytes and historical
results remain unchanged; the failed Windows precursor is not rerated or reused.
The ordinary `run` command, including `--diagnostic`, runs both profiles; it is not
a reference-only entry point. Freeze and bind a separate reference-only invocation
of the existing Rust collector before launch. Missing candidate assets need not
block that precursor, but remain required for any later candidate campaign.
Old ceilings for other workloads or another host are not silently reused.

The same precommitted policy binds `process_limits`: external process timeout,
output byte limit, cleanup deadline and admitted cancellation/deadline return
bound. The source/artifact/cohort binding is recorded before execution, not
committed into the source revision it identifies.

## Verdict policy

Every declared workload must pass NFC/trimmed text comparison with only U+0025
`%` and U+FF05 `％` treated as equivalent on both observed and expected text,
under ADR 0083. Same-implementation stability uses the same text equivalence but
retains exact geometry, confidence, region order and group membership/order.
Quality count/order, IoU at least 0.50, normalized center deltas at most 0.025,
finite model-local confidence, source correlation, resource/lifecycle and
absolute-budget gates remain required. No broad NFKC, other width folding,
fuzzy/case folding or cross-model confidence threshold substitutes for correctness.
All below-threshold observations remain. Keep raw text and fixture authoring
unchanged; this benchmark comparison does not change SDK result normalization.
The existing `criteria-v1.json` schema and ID remain; its exact SHA-256 and
ADR 0083 bind the revised semantics. Historical reports retain their original
criteria/source binding and failures.

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

## Final comparison at cf2763a

Source `cf2763a41929ad89366dd77ee1b6a57503a8f6bc`, tree
`518ca130aa8379ba000da1cd7fc6df160cc5830c`: the one-shot final comparison
completed on both approved cohorts. Each target retained three reference and
three candidate processes, all eleven images/22 workloads, 264 warmups and
2,640 measured observations. No process was replaced or retried.

| Target | Observations | Candidate quality-failure entries | Distinct failed observations | Negative-image confidence |
|---|---:|---:|---:|---:|
| `aarch64-apple-darwin` | 2,904 | 660 | 396 | 0.49752 |
| `x86_64-pc-windows-msvc` | 2,904 | 792 | 396 | 0.51158 |

These are repeated observation-level failures, not counts of independent defects.
Both candidates misread `[OK]` as `[0K]`, `一点照準` as `点照準`,
`追加攻撃35%` as `追加攻擊35%`, and `唯一` as `唯`. The nonuniform negative
produced an extra `●`: exact region count fails on both targets; Windows also
fails the unmatched-detection threshold of 0.5. Each target retains 264 raw
percent-width-only comparisons that pass the frozen equivalence rule.

Reference quality and both profiles' numerical, identity, source-correlation,
collector, lifecycle and owned-cleanup gates passed. The actual shared
`admission` CLI recomputed both original reports and reproduced every failure,
with no additional cross-target identity or completeness failure. It exited 2
with `admitted: false`; faster observations cannot override failed quality.

Original campaign packet SHA-256 values:

- Apple: `1977d71183b502716d680258e8d9a0007934101b9ed4da2ee02fc2c558757515`.
- Windows: `62e9f64973fe3bd0068892dc776518ef7f90d365ddd862b93ddfbacff995a5fc`.

`ppocrv6-tiny-det-small-rec-max736-v1` remains unsupported and absent from
ordinary construction/enumeration. This failed comparison assigns no new
Reference/Fast/Balanced/Unrated rating; accepted legacy metadata and the default
are unchanged. The new-model addition remains incomplete. Another candidate
attempt requires an explicit design decision, not weakened criteria, a silent
model replacement or an automatic retry.
