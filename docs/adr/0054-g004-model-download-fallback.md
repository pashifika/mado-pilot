# ADR 0054: Allow verified fallback sources for G-004 CI models

- **Status:** Accepted
- **Date:** 2026-08-29
- **Resolves gate:** _none_
- **Supersedes:** only the ModelScope-only CI transport rule recorded by the G-004 qualification evidence; model identity and qualification remain unchanged

## Context

The native GitHub-hosted jobs provision the accepted G-004 detector and recognizer before compiling or testing the workspace. The original workflow downloaded both files only from the RapidOCR ModelScope `v3.9.2` paths. ModelScope later stopped serving both direct model requests and its repository file listing. Two unchanged macOS and Windows job pairs stalled in the provisioning step and then received HTTP 502 without reaching checkout-dependent compilation or tests. A rerun of previously passing native-watcher source `65da97b` stopped at the same boundary, excluding the product source as the cause.

The accepted model identities remain exact:

| Model | Bytes | SHA-256 |
|---|---:|---|
| `ch_PP-OCRv4_det_mobile.onnx` | 4745517 | `d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9` |
| `PP-OCRv6_rec_small.onnx` | 21234383 | `6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884` |

The revision-pinned SWHL detector repository and revision-pinned independent RapidOCR `v3.9.2` mirror reproduce those exact lengths and digests through both Hugging Face and its `hf-mirror.com` transport. The previously considered `webnn` detector and PaddlePaddle recognizer do not reproduce the accepted hashes and remain rejected as substitutions.

## Decision

Native CI may obtain each accepted G-004 model from an ordered allowlist of revision- or tag-pinned URLs. A candidate becomes usable only after its downloaded bytes pass the existing accepted model SHA-256. Download failure and digest mismatch both advance to the next candidate; no candidate passes by filename, HTTP success, repository identity, or claimed compatibility alone. Every candidate has a 10-second connection bound and 120-second total transfer bound. Exhausting the allowlist fails the job.

The order is:

1. revision-pinned `huggingface.co`;
2. the same revision through `hf-mirror.com`;
3. the original tag-pinned ModelScope path.

This is a CI fixture-transport rule only. The runtime remains network-free, caller-supplied model paths remain mandatory, and the accepted model length, SHA-256, graph/schema validation, correctness outputs, latency, heap, RSS, provider, C/C++, and packaging decisions do not change.

## Alternatives

- **Wait for ModelScope and keep one URL.** Rejected. An unrelated provider outage blocks every hosted product check before compilation and provides no additional model-integrity guarantee.
- **Retry the same ModelScope URL.** Rejected. Repeated macOS and Windows attempts stalled at the same boundary; retries only extend the outage because the exact-byte verifier already defines safe failover.
- **Use similarly named Hugging Face models.** Rejected. The inspected `webnn` detector and PaddlePaddle recognizer have different SHA-256 values; using them requires a new G-004 qualification rather than transport remediation.
- **Use mutable `main` URLs without digest verification.** Rejected. The selected Hugging Face repository revisions are fixed, and the accepted inner digest remains mandatory even for a fixed repository revision.
- **Redistribute copies in a MadoPilot release.** Rejected. The CI downloads remain ephemeral controlled-host inputs; this decision adds no product bundle or redistribution commitment.

## Consequences

A mirror can improve availability but cannot alter accepted bytes. A compromised or stale source can cause a warning and fallback, but cannot enter a test because the existing SHA-256 must match before the destination path is committed. If all sources are unavailable or wrong, CI remains red.

The independent mirror is not treated as a new upstream authority. Historical G-004 candidate, evaluation, and dependency-review evidence remains revision-bound and byte-identical; this ADR supersedes only its single-transport rule for current CI. Future model changes still require the complete G-004 quality, correctness, memory, performance, license, and both-target process.

## Verification

- The workflow runs the same bounded `download_verified_model` contract in Windows Git Bash and macOS Bash, using the target's native SHA-256 checker.
- A focused shell smoke must prove that an unreachable first candidate falls through to an exact second candidate and that a reachable wrong digest also falls through.
- The two accepted Hugging Face and `hf-mirror.com` candidate files have been downloaded and independently hashed to the exact accepted lengths and SHA-256 values above.
- Repository policy, both hosted native jobs, and the unchanged OCR/backend/facade/C/C++ matrices must pass on the fallback successor commit.
