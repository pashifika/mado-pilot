# G-005 change-detection default evidence

This directory retains the target-neutral aggregate used by [ADR 0050](../../adr/0050-change-detection-default.md). It qualifies only the frozen repository RGBA8 sequence boundary and closed policy facts. It contains no native capture, watcher scheduling, OpenCV, OCR, timing, arbitrary-scene, or release-support result.

## Frozen inputs

- Released/protected baseline: `dfee89e6542b432324b33395674a973e0e8f136b`, tree `be3f09b5f7ca84e31be191bd2896471f567a47a7`
- Contract: `g-005-evaluation-contract-v1`
- Fixture set: `g-005-v1`
- Manifest SHA-256: `dea51dc9862373f636870c3593f590fb65cc1489fd2f11a4cbb5842836fa532a`
- Expected-row SHA-256: `2c082e24628b64fdc23706226311eb59aab2d61351adb3f86aa42f1c2e6648a1`
- Formatted qualification evaluator SHA-256: `9f3f684cd8f418a97c9cbad74165936391934787a396ad29846e427f064a8631`
- Candidate-plan SHA-256: `4b84ee426177f3bcd97e77918f73526629067766b15803aecca291ab53ff037c`
- Canonical report SHA-256: `12cf52aab777bcfccf75748506a856a0ea4eb6e1435be63b783f2a85353732cf`

Fixture provenance, license, frame-byte digests, frame order, ROI, identities, and ground truth live in `fixtures/change-detection/g-005/`. The pre-observation contract and first-run record live in `rasen/changes/phase-4-change-detection-default/evidence/`.

The first pre-format observation and evaluator source
`d82314c27f72645fe2a6d42ae50191f7a142e3c5e50970021dd3702968aadef1`
remain historical Change evidence. Formatting changed no normative input,
decision, counter, or policy; the current report binds the separately rerun
formatted source.

## Aggregate result

`accepted-report.json` is canonical compact JSON with a trailing newline. The exact candidate passes all six must-detect rows and skips the three compatible unchanged rows. Every changed-pixel threshold and fixed-grid candidate is retained and rejected because it has at least one false skip.

The report binds complete document/evaluator/candidate-plan digests but includes no pixels, per-frame hashes, paths, desktop/window/process data, credentials, template identities, OCR/input text, or free-form decoder/backend/native payloads.

## Reproduction

```sh
cargo run --locked --package mado-pilot-testkit \
  --example change-detection-evaluator > /tmp/g-005-report.json
diff -u docs/evidence/g-005/accepted-report.json /tmp/g-005-report.json
```

The protected Windows and Apple Silicon jobs run the equivalent command using `RUNNER_TEMP`. ADR 0050 remains Proposed until both jobs reproduce identical bytes on one exact topic revision and independent review passes. Failed or mismatched output remains bound to its source and is never retried into acceptance.
