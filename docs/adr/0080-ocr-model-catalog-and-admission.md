# ADR 0080: OCR model catalog and admission

- **Status:** Accepted execution contract; model-admission/rating policy superseded by [ADR 0084](0084-ocr-model-choices-and-test-results.md)
- **Date:** 2026-10-15
- **Resolves gate:** none; the original model-quality admission requirement below is historical
- **Supersedes:** none; ADRs 0033 and 0040 retain their exact accepted tuples

## Context

Hosts need exact model resources and readable choices without maintaining another
support list. The official PP-OCRv6 tiny detector and small recognizer are a new
export, not the accepted RapidOCR model pair. Neither official graph embeds the
recognizer dictionary. The pinned recognizer `inference.yml` supplies it.

[ADR 0033](0033-default-ocr-model-profile.md) records a tiny-detector dense-tooltip
miss on both targets. That adverse oracle remains required. A private Python run
or a macOS 27 run cannot qualify the Rust implementation on the approved release
hosts.

[ADR 0081](0081-current-apple-ocr-verification-cohort.md) supersedes that host
exclusion for prospective, explicitly approved Apple CPU OCR verification.
Earlier runs and their qualification status remain unchanged.

## Decision

`mado-pilot-ocr` owns a strict versioned source JSON catalog, compiled into borrowed
immutable descriptors. Assets, closed identity admission, ONNX construction and
Rust host metadata use that authority. Build and maintainer validation share one
implementation. Recognition never reads JSON. No new workspace crate, package
schema, C ABI entry, default, provider claim or watcher admission is introduced.

Execution descriptors bind resource identities, source/license provenance,
graphs, vocabulary, pixel/geometry algorithms and finite bounds. Their digest
excludes presentation and evidence labels, but includes the referenced resources.
Only accepted entries appear in product enumeration and accepted-ID construction.
A non-default benchmark feature exposes an explicit candidate route; enabling it
does not admit candidates through ordinary constructors or package manifests.
Manifest v1/v2 cannot declare the new auxiliary source and must refuse that closed
profile. Existing constructor names, tuples and arithmetic remain unchanged.

### Frozen candidate revision

The profile is `ppocrv6-tiny-det-small-rec-max736-v1`, model revision
`paddle-ppocrv6-onnx-max736-v1`, preprocessing
`ppocrv6-tiny-det-bgr-max736-linear-meanstd-source-rec-v1`.
Resources are relative to the caller-selected root; none is distributed or fetched
by the SDK:

| Resource | Bytes | SHA-256 |
|---|---:|---|
| `huggingface/PaddlePaddle/PP-OCRv6_tiny_det_onnx/inference.onnx` | 1780590 | `193bab7a04fca699a6c82e6abb5b81bdb28177f0abd4062552b04908dafb19f8` |
| `huggingface/PaddlePaddle/PP-OCRv6_small_rec_onnx/inference.onnx` | 21159378 | `5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634` |
| `huggingface/PaddlePaddle/PP-OCRv6_small_rec_onnx/inference.yml` | 150579 | `ab078671bb49f06228eadccd34f1bb501e157f7a047095ffb943ba81512c77d1` |

The Apache-2.0 sources are the official PaddlePaddle Hugging Face
[tiny detector revision `2ba1506c0380b8f0b03dd142459aac66d4421f6c`](https://huggingface.co/PaddlePaddle/PP-OCRv6_tiny_det_onnx/tree/2ba1506c0380b8f0b03dd142459aac66d4421f6c)
and [small recognizer revision `b8f84f0b80c529de40b4fbb3544b84fa7233a513`](https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx/tree/b8f84f0b80c529de40b4fbb3544b84fa7233a513).
Model-card language coverage is not a support result.

Both graphs use float32 `x` and `fetch_name_0`. Detector input/output shapes are
`[-1,3,-1,-1]` / `[-1,1,-1,-1]`; recognizer shapes are
`[-1,3,48,-1]` / `[-1,-1,18710]`. Any mismatch refuses initialization.
CPU intra/inter-op threads remain one. Candidate accelerator selection is refused.

Detector sampling applies `min(1, 736 / max(source_width, source_height))`, truncates
each scaled dimension to an integer, rounds to the nearest multiple of 32 with
ties to even, then clamps the minimum to 32. Final dimensions are at most 736.
The minimum clamp is explicit; it does not expand a short side to 736. OpenCV
`INTER_LINEAR` resizes directly once. Inverse geometry uses the actual independent
source/final X and Y ratios, including extreme aspect ratios.

Candidate BGR planar normalization is float32 multiplication by `1.0 / 255.0`,
then subtraction of `[0.485, 0.456, 0.406]`, then division by
`[0.229, 0.224, 0.225]`, without channel swapping or fused arithmetic. Legacy
normalization remains division by 255, subtraction of 0.5, then division by 0.5.
Recognition uses the original source crop and existing perspective/rotation,
height-48, nominal-width-320, right-zero-padding and greedy-CTC rules.

DB starts with the reviewed RapidOCR parameters: threshold 0.3, box threshold
0.5, unclip 1.6, 2x2 dilation, fast score and existing detector order. The detector
YAML is not an execution authority. Keep the 1,000-candidate, six-crop batch,
4,096 recognizer-width, 64 MiB model-component, 256 MiB tensor/output, 2 MiB
contour and 16 MiB scanline-queue ceilings. The candidate detector bound is
`736 * 736 * 3 * 4 = 6,500,352` bytes. Any execution change needs a distinct
candidate revision and a new frozen evaluation, not a rewritten failed attempt.

### Vocabulary parsing and ownership

Use `yaml-rust2` 0.13.0, default features off, only in the ONNX adapter. Its
[registry manifest](https://docs.rs/crate/yaml-rust2/0.13.0/source/Cargo.toml)
declares MIT OR Apache-2.0, Rust 1.85, pure Rust and no build script. Disabling
`encoding` excludes encoding detection; the captured resource must be UTF-8.
The event API avoids constructing or alias-expanding a general YAML value tree.

Read, length/hash-check and parse one bounded captured content version. Limits
are 1 MiB source/canonical vocabulary, depth 32, 100,000 events, 16 KiB scalar,
and exactly 18,708 ordered entries. Reject duplicate mapping keys, complex keys,
multiple documents, explicit tags, invalid/control-containing or empty entries,
and aliases/anchors in the selected dictionary. The official `Hpi` section has
an anchor and alias: ignored sections may contain them, but they are never
resolved or executed.

Only `PostProcess.character_dict` supplies this profile's entries. Join exact
UTF-8 entries with LF and no trailing LF; require digest
`f7aa897ca828a4c7c9e2739c30f9161a33306d532f020bcdb91dcfb664a5507e`.
Blank index zero and the appended space yield 18,710 graph classes. Old profiles
still require ONNX `character` metadata. There is no source fallback, ambient
lookup, second file read, graph rewrite or borrowed dictionary from another model.

### Qualification policy

Freeze the unchanged G-004 v3 corpus and independently authored compact-UI
fixtures before candidate inference. Require exact NFC/trimmed text, count/order,
IoU at least 0.50, normalized center deltas at most 0.025, finite confidence,
no unmatched region at confidence at least 0.5, stable repeated observations,
source correlation and lifecycle/resource correctness. Retain observations below
thresholds, missing rows and failures.

The actual Rust path runs reference and candidate serially in at least three
fresh processes each, with two warmups and twenty measured samples per workload.
Reports bind the implementation/executable, execution descriptor, resources,
corpora, criteria, host/runtime/provider/compiler, command and thread policy.
Record cold initialization separately from warm end-to-end and stage timings,
actual tensors, allocation/retention and native RSS, interruption and close.
Target/host-specific absolute budgets and required collectors must be reviewed
and committed before final measurement. No budget is inferred from the prototype.
Missing approved hosts, budgets or collectors blocks qualification, not the
implementation or its CI checks.

`reference` identifies an attributable qualified comparison baseline. `fast`
requires every hard/absolute gate and both p50/p95 at most 80% of the reference
in every workload of the same cohort. Complete passing evidence that misses that
threshold is `balanced`. `unrated` applies only to an already accepted profile
without applicable comparative evidence. It never admits a candidate.

## Alternatives

- Runtime JSON or host-owned support lists: duplicate authority and widen admission.
- A new package schema or C ABI: not required by the Rust host integration.
- A derived bundled dictionary or modified graph: loses direct official provenance.
- Generic YAML deserialization: retains unused configuration and risks alias expansion.
- Updating an old bounded identity or relaxing the dense-tooltip row: invalidates
  immutable execution and historical evidence.

## Consequences

Integrators can inspect accepted choices and provision exact resources without
loading models. Existing selections remain valid and the default does not move.
The new candidate stays unsupported until independently approved Apple and
Windows reports pass. The source-only deployment and permission policies remain
unchanged. No capture/input permissions are needed to evaluate licensed replay
pixels.

## Verification

Catalog/source admission, malformed vocabulary, pixel and geometry regressions,
existing provider/watch contracts, manifest v1/v2 and C/C++ checks enforce the
implementation boundary. The maintainer runner validates raw reports rather than
trusting a supplied verdict. Final native-host admission and absolute-budget
acceptance remain explicit, uncompleted gates; compilation and diagnostic smoke
cannot close them.
