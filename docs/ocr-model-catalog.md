# OCR model choices and test results

The SDK exposes all three implemented catalog profiles through ordinary Rust
selection, without opening models or loading ONNX Runtime. Failed or missing
quality/performance results do not hide a choice, require an experimental opt-in,
change the default, or trigger fallback. [ADR 0084](adr/0084-ocr-model-choices-and-test-results.md)
separates model choices from named, scoped test outcomes.

The native G-004 default and all execution/resource identities are unchanged.
`ppocrv6-tiny-det-small-rec-max736-v1` is an ordinary CPU choice with recorded
text and negative-image failures, not a promise of accuracy on the caller's images.
[ADR 0080](adr/0080-ocr-model-catalog-and-admission.md) still fixes its execution
contract and mandatory pinned YAML vocabulary.

Package manifest v1/v2, the C/C++ ABI and provider capabilities are unchanged.
Text watching reuses the caller's compatible initialized OCR selection under
[ADR 0091](adr/0091-ocr-watch-caller-selection.md); its fixed bounded-v2
qualification workloads remain revision-bound. A manifest that cannot express
the required auxiliary resource is refused; no ambient dictionary is supplied.

## Rust integration

```rust
use mado_pilot::{ocr_profiles, OcrProfileConfig};

fn main() -> Result<(), mado_pilot::Error> {
    for profile in ocr_profiles() {
        println!("{}: {}", profile.profile_id, profile.display_name);
        for evaluation in profile.evaluations {
            for rule in evaluation.rules {
                println!("{} / {} / {}: {:?}",
                    evaluation.target, evaluation.provider, rule.rule_id, rule.outcome);
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

Pass `selection` to the existing replay, Windows or macOS
`*_engine_with_ocr_profile` constructor. `from_id` validates selection without
resource I/O. Engine construction revalidates it, including manually constructed
`OcrProfile::Catalog` values. Unknown IDs and invalid execution/resource contracts
still fail with typed errors. Quality results are not consulted by construction.

Descriptors are borrowed static data. `ocr_profile(id)` resolves the same choices.
`DEFAULT_OCR_PROFILE_ID` identifies the unchanged native G-004 default. Persist
the profile ID, not a display name or enum discriminant.

Each `evaluations` entry names a target, provider and criteria document. Its
`rules` distinguish `Passed`, `Failed`, `NotRun` and `Incomplete`. Optional
`evidence` names the detailed report, measured source revision, exact execution
digest and evaluator revision. The report supplies corpus/workload, host/runtime,
expected/observed details and measurements. No aggregate rating replaces them.
Missing evidence for a target/provider is not inherited from another scope.

Run the metadata/configuration-only example without model or runtime resources:

```bash
python3 tools/setup-native.py -- cargo run --locked -p mado-pilot --example ocr-profiles
```

Setup checks installed build prerequisites; the example itself performs no model
initialization, inference or acquisition. The SDK does not fetch report URLs.

`OcrProfileConfig::new` and `from_id` preserve `OcrProfile::BoundedDetector` for
the bounded catalog ID, keeping legacy pattern matching and configuration
equality consistent.

Explicit Rust profile callers may attach `OcrCpuOptions` before initialization.
The default remains one intra-op thread with spinning enabled; the selected
options apply to both native sessions and have a distinct runtime identity.
See the [CPU options and measured trade-offs](ocr-text-watch.md#cpu-execution-options).
Use `OnnxBackendFacts::runtime_profile_id()` for the initialized configuration,
not the provider enum's default-options identity.

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
| Root | `schema_version: 2`, `default_profile`, `resources`, `profiles`; at most 1 MiB, 128 resources and 64 profiles |
| Resource | Unique `id`, `kind` (`model` or `auxiliary`), safe `relative_path`, exact `byte_len`/`sha256`, immutable `source_url`/`source_revision`, reviewed `license`/`notice` |
| Profile | Nullable `legacy_constants`, `execution`, `presentation`; no quality-admission status or host-maintained selection list |
| Execution identity | `model_id`, `model_version`, `profile_id`, language/preprocessing/decoder/normalization IDs; detector/recognizer references and explicit nullable auxiliary reference |
| Graphs | Named float32 input/output tensors, ranks/shapes and finite axis limits; `-1` marks a dynamic axis |
| Detection | Implemented DB/resize/rounding/interpolation/inverse-geometry algorithms, exact pixel operation order, channel means/stds, thresholds and checked dimensions/bytes |
| Recognition | Original-source perspective crop, orientation, resize/padding, height/width/batch and normalization policy |
| Vocabulary | Explicit ONNX metadata or Paddle YAML source/key, canonical digest/count, class count and greedy-CTC rules |
| Limits/applicability | Finite tensors, contours, scanline queue and resource ceilings; exact target/provider/runtime/API scopes |
| Presentation | `display_name`, nullable `variant`, and `evaluations` with scoped rule outcomes and optional source-bound evidence; no execution parameters |

Unknown/duplicate keys, missing required nullable fields, duplicate identities or
case-colliding paths, dangling references, unsafe paths, unreviewed licenses,
nonfinite/invalid parameters and unimplemented algorithms fail validation.
Nullable fields use explicit JSON `null`, not omission. A vocabulary source cannot
silently fall back to the other source. Execution digests include referenced
resources and execution fields, excluding display labels and evaluation metadata.
Catalog schema v2 retains execution-fingerprint serialization version 1, so the
metadata cutover does not change a saved execution identity.
The current backend requires a DB candidate ceiling of 1000 and a vocabulary of
18,708 entries plus CTC blank and terminal space (18,710 classes). The validator
rejects other values until their execution contract is implemented.

Run the same validator used by the build:

```bash
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate
cargo run --locked -p mado-pilot-ocr --example ocr-catalog-validate -- path/to/catalog.json
```

The first command prints execution digests. To obtain a profile's exact resource
rows for controlled provisioning:

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
   Declare a distinct execution identity; do not mutate an existing tuple.
2. Use implemented algorithms and finite bounds. An algorithm change requires
   implementation and a focused ADR before evaluating the new configuration.
3. Freeze independent licensed inputs and exact oracles before inference. Keep
   every unchanged G-004 v3 row, including the adverse dense-tooltip case.
   Never import private pixels/labels/digests into public fixtures.
4. Verify resource refusal, pixels, geometry, singular/grouped source correlation,
   cancellation/deadline/close and existing legacy/provider/watch contracts.
5. Freeze host/cohort policies, numerical comparison bounds and required
   collectors before measurement. Missing prerequisites stay explicit.
6. Retain all observations and publish independently scoped rule results.
   Failure or missing coverage does not make a technically valid configuration
   disappear. Do not claim a measurement on a target, provider or source not run.

## Repeatable Rust measurements

The `onnx-cpu` benchmark has an explicit `--model-evaluation` mode behind
`benchmark-instrumentation`. It selects the same compiled profiles as ordinary
product construction. Build in a dedicated target directory and do
not subsequently rebuild a hash-bound executable in that directory:

```bash
CARGO_TARGET_DIR=target/ocr-model-evaluation \
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
python3 tools/ocr-models/run.py evaluate "$NEW_REPORT_DIRECTORY/report.json" \
  --evidence-roots "$NEW_REPORT_DIRECTORY"
python3 tools/ocr-models/run.py evaluate "$APPLE_REPORT" "$WINDOWS_REPORT" \
  --evidence-roots "$APPLE_EVIDENCE_ROOT" "$WINDOWS_EVIDENCE_ROOT"
```

The fixed evaluation cohort below remains one intra-op thread with spinning
enabled. Explicit CPU-option comparisons are separate source-bound measurements,
not replacements for that cohort or its historical results. The benchmark
refuses CPU options that disagree with a supplied thread/spinning cohort binding.

All referenced inputs are explicit. Output directories are new and never
overwritten. Reference and candidate run serially, three fresh processes each;
every corpus image has singular and grouped workloads with two warmups and twenty
retained samples. Intra/inter-op CPU threads are one. Reports preserve exact
observations, failures, commands, cold open/close, warm end-to-end and stage
timings, actual detector dimensions/bytes, Rust allocation/retention and native
process peak RSS. The evaluator recomputes named rules and measurements from
original evidence. One or several reports can be evaluated independently; an
absent target is not an AND-gate failure. Complete verified evaluation exits 0
even when quality or empirical numerical rules fail. Invalid, incomplete or
operational/technical failure remains explicit and exits nonzero.

`not_run_targets` lists release targets without a supplied report. It is `null`
when an unreadable or malformed report cannot be attributed to a target: unknown
coverage is not a claim that no execution occurred. Other verified scopes remain
available. Typed recognition failures leave affected sample rules incomplete,
while any observed execution-bound or empirical-ceiling violations remain visible.

Validation requires the original JSONL files and the parent's independent
`.process.json` receipts, not only the JSON report. `raw_path` and `receipt_path`
are relative to the report directory; every retained file, digest, parsed
observation, invocation and external process outcome is reconciled before
evaluation. For relocated evidence, pass one `--evidence-roots` entry per report.
`--repository REPO` selects the Git object store used to verify each policy's
repository-relative `path`, frozen commit, regular-file tree entry and blob bytes.
Rewriting embedded policy bytes or summary values cannot replace those originals.

Before each descriptor or evaluation child, the runner checkpoints an
`attempted-unconfirmed` marker with its command, source and available binding/
profile identity. A failed prelaunch checkpoint prevents that launch. After a
child returns, its independent receipt is saved and checkpointed before parsing;
the completed parsed prefix is checkpointed separately. `completed` confirms an
observed exit and successful cleanup, not a passing quality result.

The intermediate evaluation projection uses `projection_state: "receipt-only"`.
The evaluator verifies its exact index, command, process outcome, evidence paths
and raw/receipt digests, then reconstructs the full original JSONL observations.
Already verified quality failures remain failed rather than becoming unverified
because parsing was interrupted. Before the raw JSONL digest is recorded, the
projection is `raw-digest-pending`: an interruption keeps the digest-bound
receipt and attempt status, and the evaluator verifies that receipt projection
but never evaluates the undigested raw bytes, so that process is incomplete while
earlier verified findings remain. Partial raw/receipt files of an
`attempted-unconfirmed` child without a recorded receipt digest are accounted
for, never evaluated. Parsed and legacy projections still require full
reconciliation; existing schema-1/2 evidence interpretations are unchanged.

On `KeyboardInterrupt`, the runner checkpoints `execution: "interrupted"` with
the retained parsed/receipt prefix and unconfirmed attempt, removes any aggregate
evaluation, and rethrows the same exception. If that checkpoint fails, the prior
durable prefix remains and the failure is attached as an exception note.
Temporary-file cleanup failure likewise does not replace the primary exception.
Unknown exit or cleanup remains unknown; failed or unconfirmed cleanup withholds
later launches. Neither raw output nor an interrupt proves child termination.
Inspect retained evidence with the `evaluate` command and its explicit
`--evidence-roots` above; evaluation launches no replacement child and provides
no resume or restart. This retention contract changes no model availability,
historical result or campaign authority.

The benchmark embeds its build revision, tree, compiler and dirty state behind
`benchmark-instrumentation`; ordinary SDK builds perform no Git collection.
Preflight and validation reconcile those facts against the requested source and
binding. An older binary with an unchanged catalog is not evidence for a newer
implementation. Dirty or unavailable build provenance cannot supply passing rules.

Use `--preflight-only` to check execution prerequisites without inference.
Explicit `--diagnostic` retains missing prerequisites and partial observations;
it cannot turn invalid or incomplete evidence into passing rules.
Hosted CI is not a final performance cohort.
[ADR 0081](adr/0081-current-apple-ocr-verification-cohort.md) approves the current
macOS 27.0.1 Apple M1 Pro cohort through
[`host-policy-v1.json`](../tools/ocr-models/host-policy-v1.json), without relabeling
earlier failed diagnostics. [ADR 0082](adr/0082-current-windows-ocr-reference-precursor.md)
adds the independently identified Windows 11 Pro 26H2 CPU cohort. These host
policies constrain measurement attribution and execution, not model selection.
Inspect completion and individual outcomes rather than treating exit 0 as an
accuracy or performance approval.

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
derived under ADR 0082. Calibration does not authorize another model run.
The final common-source comparisons completed with quality failures on both
targets; their original source-bound disposition is preserved below.
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

## Rule interpretation

The frozen suite compares NFC/trimmed text with only U+0025 `%` and U+FF05 `％`
treated as equivalent on observed and expected text, under ADR 0083.
Same-implementation stability uses that equivalence while retaining exact
geometry, confidence, region order and grouped membership/order. Separate rules
cover text/order, region count, IoU >= 0.50 and normalized center deltas <= 0.025,
unexpected detections at confidence >= 0.5, finite model-local confidence,
stability, fixture-character coverage, source correlation, lifecycle, execution
bounds, process/workload budgets and relative latency.

The relative-latency rule requires both p50 and p95 <= 80% of the reference in
every workload in the same target/provider/cohort. It is independent of accuracy;
the reference is a comparison role, not a rating. Empirical latency/RSS ceilings
are not enforced tensor limits or process deadlines.

No broad NFKC, other width/case folding, fuzzy match or universal confidence floor
substitutes for these rules. Raw text, all below-threshold observations, authored
fixtures and SDK normalization remain unchanged. The historical
`criteria-v1.json` ID contains `admission`; its bytes remain the frozen rule input,
not an active model-admission API.

A failing workload remains visible even when other workloads or targets pass.
Incomplete scopes retain established failures but cannot invent passes for
missing data. A new evaluation view identifies both its evaluator revision and
the original measurement source; it is neither a native rerun nor a retroactive
pass of an old admission decision.
[The unchanged G-004 evidence](evidence/g-004/) retains its historical
sources, hashes and adverse observations.

## Derived results from retained measurements

[`cf2763a-evaluation-v1.json`](benchmarks/ocr-models/cf2763a-evaluation-v1.json)
is the actual `evaluate` CLI output for the retained Apple and Windows reports.
The command completed successfully while preserving every original quality
finding. Its evaluator revision is content-addressed independently of the
measured source `cf2763a41929ad89366dd77ee1b6a57503a8f6bc`.

For the tiny/small profile, text/order and region-count rules fail on both CPU
targets; Windows also fails the unexpected-detection rule. Process/workload
budgets and the all-workload relative-latency rule pass independently. The
reference has no quality findings and no self-comparison timing result.

The catalog exposes those exact scoped summaries. Bounded-v2 and CUDA have
`not_run` outcomes under this comparison suite, not inherited CPU results or a
revocation of their separately recorded support. Neither this derived view nor
the metadata cutover is a new native execution or a retroactively passing old
admission decision.

The following section preserves the original `cf2763a` comparison and disposition,
including its superseded availability statement. It describes that historical
source and policy, not the ordinary selection API above.

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
