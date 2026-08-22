# G-004 model, fixture, and evaluation dependency review

This review covers the exact files and tools used to decide the default OCR
profile. It does not add a Cargo dependency, ONNX Runtime library, model file, or
OCR implementation to a MadoPilot release.

## Decision summary

The qualifying profile remains controlled host-provided. MadoPilot will consume
only caller-provisioned files at the exact relative paths recorded in
`candidates.json`, verify each SHA-256 before creating an ONNX Runtime session,
and report a typed backend-unavailable outcome when a file is absent or differs.
The repository, crate packages, and release archives do not contain model bytes
and do not download them at runtime.

The identical v3 fixture produces matching v4 candidate outcomes on Windows and
Apple Silicon. Independent review nevertheless withholds the default: the
evaluator does not bind executed fixture bytes or installed RapidOCR code bytes,
and its unexpected-region threshold precedes expected-region matching. Those
gaps require a new evaluator identity and complete reruns on both targets before
this otherwise compatible controlled-host profile can be accepted.

## Model provenance and terms

| Component | Exact source | Copyright/license evidence | Distribution decision |
|---|---|---|---|
| RapidOCR conversion/profile metadata | RapidOCR v3.9.2, Git revision `095232a4c94f7f0e6600ba5bba1177010ad696d4` | [RapidOCR `LICENSE`](https://github.com/RapidAI/RapidOCR/blob/095232a4c94f7f0e6600ba5bba1177010ad696d4/LICENSE) is Apache-2.0 and names RapidOCR Authors; the tag contains no `NOTICE` file | Source and model metadata are referenced, not copied into a release |
| Candidate ONNX files | RapidAI/RapidOCR ModelScope tag `v3.9.2`; exact URLs, bytes, and SHA-256 values in `candidates.json` | The tag-pinned [model repository card](https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/README.md) declares Apache License 2.0 and identifies the repository as RapidOCR's hosted OCR models | Controlled host-provided; no repository or release redistribution |
| Original OCR project | PaddlePaddle/PaddleOCR | [PaddleOCR `LICENSE`](https://github.com/PaddlePaddle/PaddleOCR/blob/main/LICENSE) is Apache-2.0 and names PaddlePaddle Authors | Provenance acknowledgment only; no Paddle package or original training artifact is redistributed |
| Future CPU inference runtime | ONNX Runtime 1.29.0 for the qualification tool; the implementation Change pins its own supported version | [ONNX Runtime v1.29.0 `LICENSE`](https://github.com/microsoft/onnxruntime/blob/v1.29.0/LICENSE) is MIT and requires retaining the copyright and permission notice in redistributed copies or substantial portions | Controlled host-provided for Phase 3; Phase 5 `G-007` still decides native runtime bundling |

Apache-2.0 permits use and redistribution subject to its license, notice, changed-
file, and attribution conditions. Those terms are compatible with MadoPilot's
Apache-2.0 source release. Compatibility does not make bundling automatic: if a
later release redistributes any model, that Change must ship the applicable
Apache-2.0 license and retained notices, confirm whether the exact model repository
adds a notice, record package-size impact, and supersede the controlled-host
profile explicitly. This Change avoids those obligations by distributing no model
bytes.

ModelScope is the declared byte source rather than an unrestricted mirror. A user
or packager provisions both files before starting MadoPilot; the runtime must not
fall back to a compatible-looking filename, ambient search path, later RapidOCR
tag, network download, or model with a different digest.

## Fixture copyright and privacy

The five current fixture PNGs contain only repository-authored synthetic shapes
and text and are contributed under MadoPilot's Apache-2.0 license. They are
rendered once by `fixtures/ocr/g-004/generate.py` from Noto Sans JP at weight 400.
The exact input font is Google Fonts revision
`ec626514f79f831f1ab848a82114a0ce7e2d6372`, SHA-256
`c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f`.
Its [OFL-1.1 text](https://github.com/google/fonts/blob/ec626514f79f831f1ab848a82114a0ce7e2d6372/ofl/notosansjp/OFL.txt)
states that the font license does not apply to documents created with the font.
No font bytes are committed or shipped.

Four real game screenshots in ignored `local_docs/game-screenshots/` were reviewed
to identify contrast, density, scale, and layout classes missing from the first
synthetic fixture. Their copyright and redistribution rights are unresolved.
Therefore no screenshot pixel, crop, digest, path, game title, recognized output,
or artwork-derived file enters tracked evidence. The later `tooltip-v3.png` and
`mission.png` are newly rendered synthetic layouts; they copy no source pixels,
character imagery, or title.

Tracked target reports contain the approved fixture text already published in the
manifest, source-relative geometry, model/profile identities, aggregate timing and
resident memory, and typed outcomes. Unexpected recognized text remains only in
ignored local raw reports. Reports exclude user and machine names, home/model
paths, environment variables, credentials, serials, unrelated application data,
and raw tensors.

## Evaluation-only Python environment

`tool-requirements.txt` pins the complete Python 3.14.6 environment used by the
qualification script. RapidOCR, ONNX Runtime, OpenCV Python, Pillow, NumPy,
Shapely, and their Python closure are disposable evaluation tools in ignored Rasen
ephemera. They are not product dependencies, are not added to `Cargo.lock`, and do
not enlarge MadoPilot artifacts. Their versions are retained for reproducibility;
the implementation Change independently reviews and pins the smallest native and
Rust dependency surface it actually needs.

## Required notices and follow-through

- Current source/release artifacts add no third-party model or runtime notice
  because they redistribute no such bytes.
- Documentation must continue to acknowledge RapidOCR/PaddleOCR provenance and
  the model repository's Apache-2.0 declaration for the selected identifiers.
- A host distributing the model or ONNX Runtime remains responsible for the
  corresponding Apache-2.0 or MIT notice obligations.
- The eventual profile loader must verify file length and SHA-256 before session
  creation, use only the explicit model root plus recorded safe relative paths,
  keep loading and inference bounded, and return an actionable absence/mismatch
  status.
- `G-007` remains open for bundled native-runtime packaging. Resolving `G-004`
  must not claim that ONNX Runtime itself is bundled or supported before that work.
