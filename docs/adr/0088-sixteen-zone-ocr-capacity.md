# ADR 0088: Sixteen-zone OCR capacity

- **Status:** Accepted
- **Date:** 2026-10-21
- **Resolves gate:** _none_
- **Supersedes:** ADRs 0042 and 0043 only for the zone-count bound and derived membership capacity

## Context

Callers need sixteen OCR regions on one retained frame without increasing the
recognition-result limit. Grouped scanning already uses one shared envelope,
one backend invocation and one immutable result. The eight-zone limit belongs
to request admission and membership representation, not a per-zone result limit.

## Decision

Rust, C and C++ accept one through sixteen caller-order zones and reject zero or
seventeen-or-more before work. Keep the grouped envelope, centroid membership,
ordering, ownership and final operation authority unchanged on Windows and macOS.

Use a `u16` membership mask and seventeen `u16` group offsets. Raise only derived
membership capacity to 16,000 references and 32,000 index bytes. Keep 1,000
operation-wide candidates, per-candidate and aggregate text bounds, mapping,
temporary semantic bytes and immutable-result semantic bytes unchanged.

The public Rust `candidate_interest_membership` return type becomes `Result<u16>`.
Workspace/CMake metadata and the current metadata checker advance to unreleased
0.5.0, following ADR 0006's minor-version boundary. C ABI 1.5, table ordering,
structure layouts and frozen headers do not change.
Current development-host Rust/C/C++ consumers verify backend version
`0.5.0+ort-1.29.0-api17`; historical qualification receipts keep their original
source revisions and backend identities.

## Alternatives

- A dynamic bitset adds allocation for a fixed sixteen-zone requirement.
- Splitting requests changes envelope-dependent recognition and atomicity.
- Raising candidate or result limits is outside the requested zone-only change.

## Consequences

Explicit Rust `u8` annotations for the membership helper must become `u16`.
Inferred callers and the C++ wrapper need no new API. C/C++ callers using more
than eight zones need the 0.5.0 library; ABI-minor negotiation alone does not
identify the higher capacity. Existing one-through-eight requests remain valid.

No dependency, model/profile, capture/input behavior, consumer output ceiling
or performance budget changes. Larger envelopes can increase work; sixteen-zone
quality or latency is not qualified by historical eight-zone measurements.
ADRs 0042–0044 and their evidence retain their original scope. The historical
v0.4.0 release checker retains its policy and reads a frozen CMake test input
from Git blob `470e22f689f58a08ac8e9f69b10564446935b859`, not current metadata.

## Verification

- `crates/support/testkit/tests/ocr_contract.rs` covers sparse ninth/sixteenth
  groups, caller reordering, honored/ignored interests, parent teardown,
  16,000 shared memberships and rejection of candidate 1,001.
- Facade and C contract tests cover sixteen groups and seventeen-zone refusal;
  C failure outputs are null and retained results survive parent release.
- C/C++ `ocr-fixture` executables report matching sixteen-group observations;
  `c-abi-check` checks their assertions, current layout, four frozen ABI prefixes,
  C++ ownership and CMake consumers. The Apple Silicon replay smoke completed
  without capture, input or production-model execution.
- Compile-time assertions enforce mask width, offset capacity and unchanged
  temporary/result byte ceilings. Target CI checks remain distinct from native
  quality/performance qualification.
