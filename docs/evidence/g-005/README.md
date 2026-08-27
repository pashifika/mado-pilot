# G-005 change-detection default evidence

This directory retains the target-neutral aggregate used by [ADR 0050](../../adr/0050-change-detection-default.md). It qualifies only the frozen repository RGBA8 sequence boundary and closed policy facts. It contains no native capture, watcher scheduling, OpenCV, OCR, timing, arbitrary-scene, or release-support result.

## Frozen inputs

- Released/protected baseline: `dfee89e6542b432324b33395674a973e0e8f136b`, tree `be3f09b5f7ca84e31be191bd2896471f567a47a7`
- Current contract: `g-005-evaluation-contract-v2`, SHA-256 `b104399d290e78b9d232bf439e91bc472560a57b5b0066d9d21b56e1d9796a94`
- Fixture set: `g-005-v1`
- Manifest SHA-256: `dea51dc9862373f636870c3593f590fb65cc1489fd2f11a4cbb5842836fa532a`
- Expected-row SHA-256: `2c082e24628b64fdc23706226311eb59aab2d61351adb3f86aa42f1c2e6648a1`
- Security-remediated evaluator SHA-256: `1b20e1653416806da32e2de4d16638cf07821b215d6a2c0a4912099ba2e88d8b`
- Candidate-plan SHA-256: `4b84ee426177f3bcd97e77918f73526629067766b15803aecca291ab53ff037c`
- Canonical v2 report SHA-256: `44be7f31af81ced9ac7553d210547d995552adcbbc2127cf82ba60854d5a4ab2`

Fixture provenance, license, frame-byte digests, frame order, ROI, identities, and ground truth live in `fixtures/change-detection/g-005/`. The pre-observation contract and first-run record live in `rasen/changes/phase-4-change-detection-default/evidence/`.

Contract v1 and both v1 reports remain historical Change evidence. Contract v2
bounds the complete expected-row transition id and canonical numeric suffix
before any input can enter an ordinary error, adds the privacy regression, and
advances only the aggregate schema to
`mado-pilot-change-evaluation-report-v2`. The manifest, expected rows, fixture
bytes, candidate plan, decisions, counters, authority, and selected policy are
unchanged.

## Aggregate result

`accepted-report.json` is canonical compact v2 JSON with a trailing newline. The exact candidate passes all six must-detect rows and skips the three compatible unchanged rows. Every changed-pixel threshold and fixed-grid candidate is retained and rejected because it has at least one false skip.

The report binds complete document/evaluator/candidate-plan digests but includes no pixels, per-frame hashes, paths, desktop/window/process data, credentials, template identities, OCR/input text, or free-form decoder/backend/native payloads.

## Reproduction

```sh
cargo run --locked --package mado-pilot-testkit \
  --example change-detection-evaluator > /tmp/g-005-report.json
diff -u docs/evidence/g-005/accepted-report.json /tmp/g-005-report.json
```

The protected Windows and Apple Silicon jobs run the equivalent command using `RUNNER_TEMP`. ADR 0050 remains Proposed until both jobs reproduce identical bytes on one exact topic revision and independent review passes. Failed or mismatched output remains bound to its source and is never retried into acceptance.
