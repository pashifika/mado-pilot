# ADR 0043: OCR profile and zone public surfaces

- **Status:** Accepted
- **Date:** 2026-08-25
- **Resolves gate:** _none_; implements Direction Slice `phase-3-1-bounded-zone-ocr-v0-3-1`
- **Supersedes:** _none_; ADRs 0033–0042, released defaults, singular OCR, and ABI 1.0/1.2/1.3 remain unchanged

## Context

ADR 0042 freezes a platform-neutral borrowed one-to-eight-zone OCR scan and independent immutable grouped result, but the released Rust facade, C ABI 1.3, and C++ wrapper expose only default-profile construction and singular OCR. The merged ONNX adapter already constructs the accepted native G-004 default and the separate ADR 0040/0041 bounded-detector profile without fallback. Public projection therefore requires ownership, negotiation, and layout work, not another backend path.

The complete released C tables are 424 bytes for ABI 1.0, 592 bytes for ABI 1.2, and 648 bytes for ABI 1.3. The exact declarations and dependency baseline were frozen before implementation in [`../../rasen/changes/phase-3-1-ocr-profile-and-zone-public-surfaces/evidence/source-baseline.md`](../../rasen/changes/phase-3-1-ocr-profile-and-zone-public-surfaces/evidence/source-baseline.md). The cross-language, invalid-input, ownership, mutation, privacy, and layout pass/fail rows were predeclared in [`../../rasen/changes/phase-3-1-ocr-profile-and-zone-public-surfaces/evidence/qualification-plan.md`](../../rasen/changes/phase-3-1-ocr-profile-and-zone-public-surfaces/evidence/qualification-plan.md).

## Decision

Rust adds the closed `OcrProfile::BoundedDetector` selection and owning `OcrProfileConfig`, plus separate replay, Windows, and macOS `*_engine_with_ocr_profile` constructors. Construction uses one caller `OperationContext`, publishes only a complete engine whose descriptor reports the selected identity, and performs no search, download, retry, provider fallback, or default substitution. `DefaultOcrConfig::new` and every existing constructor continue to select native G-004 exactly.

Runtime adds `Session::scan_ocr_zones`, forwarding one borrowed `OcrZoneScanRequest` to the configured recognizer under the existing operation arbitration and returning its independent `OcrZoneScanResult`. Runtime and facade curate the existing grouped OCR types. No queue, watcher, stored result, latest-frame substitution, per-zone retry, normalization, membership, or detector algorithm is added.

### ABI 1.4 declarations and order

C ABI minor 1.4 adds fixed-width `madopilot_ocr_profile_kind_t`, size-versioned `madopilot_ocr_profile_options_t`, `madopilot_ocr_zone_t`, `madopilot_ocr_zone_scan_request_t`, `madopilot_ocr_zone_scan_result_info_t`, and `madopilot_ocr_zone_result_t`, plus opaque `madopilot_ocr_zone_scan_result_t`.

`MADOPILOT_OCR_PROFILE_BOUNDED_DETECTOR` is signed 32-bit value 1. Every other profile-kind value is invalid. The record field order is fixed as follows:

- profile options: `struct_size`, zero `flags`, `kind`, zero `reserved`, borrowed `model_root`, borrowed `runtime_path`;
- zone: `struct_size`, zero `flags`, capture-pixel `region`, `clip_policy`;
- request: `struct_size`, zero `flags`, retained-for-call `frame`, optional integrated-profile `package`, borrowed `model_id`, `backend_id`, `backend_version`, `output_space`, zero `reserved`, borrowed `zones`, `zone_count`, `zone_stride`;
- result info: `struct_size`, zero `flags`, exact `source`, capture-pixel `source_envelope`, `output_space`, semantic `zone_count`, `unique_candidate_count`, `membership_count`, then owner-borrowed `backend_id`, `backend_version`, `model_id`, `model_version`, and `profile_id`;
- zone result: `struct_size`, zero `flags`, caller-order `effective_zone`, zero `reserved`, semantic `region_count`.

Every new record's mandatory prefix is the whole declared record. Structure sizes and reported table sizes are `uint32_t`; zone-array counts and strides and accessor indexes are `size_t`; result/group/candidate/membership counts are `uint64_t`; profile and enum values are signed fixed-width. Every conversion is checked before allocation, dereference, or output publication.

Eight function pointers append after the complete 648-byte ABI 1.3 prefix in this permanent order:

```text
engine_create_with_ocr_profile
session_scan_ocr_zones
ocr_zone_scan_result_retain
ocr_zone_scan_result_release
ocr_zone_scan_result_info
ocr_zone_scan_result_zone_at
ocr_zone_scan_result_region_at
ocr_zone_scan_result_text_at
```

On the two 64-bit release targets the predeclared complete extent is 712 bytes, with entries beginning at offsets 648 through 704. Compiled Rust, Apple Clang, and MSVC probes are authoritative; a mismatch fails the Change and requires this ADR to be updated from measured evidence rather than forcing the expected layout.

The 2026-08-25 Apple Clang 21.0.0 and `rustc 1.97.1` probes agreed on every predeclared value:

| Declaration | Size / alignment | Field offsets in declaration order |
|---|---:|---|
| `madopilot_ocr_profile_options_t` | 48 / 8 | 0, 4, 8, 12, 16, 32 |
| `madopilot_ocr_zone_t` | 32 / 4 | 0, 4, 8, 28 |
| `madopilot_ocr_zone_scan_request_t` | 104 / 8 | 0, 4, 8, 16, 24, 40, 56, 72, 76, 80, 88, 96 |
| `madopilot_ocr_zone_scan_result_info_t` | 176 / 8 | 0, 4, 8, 48, 68, 72, 80, 88, 96, 112, 128, 144, 160 |
| `madopilot_ocr_zone_result_t` | 40 / 8 | 0, 4, 8, 28, 32 |
| `madopilot_api_t` ABI 1.4 suffix | 712 / 8 | 648, 656, 664, 672, 680, 688, 696, 704 |

The same C11 probe uses `_Alignof` on Apple Clang and `__alignof` on MSVC, checks fixed-width semantic/count field types with `_Generic`, and is a required native CI row. A differing MSVC result fails the Change; it is not normalized or replaced by the Apple measurement.

### Boundary and ownership

Profile construction and zone scanning initialize valid result/error outputs before reading inputs and contain Rust panics. The synchronous zone call borrows caller storage only for its duration. It checks the request and every element's complete mandatory prefix, flags, pointer, count, stride, alignment, checked final address, capture-pixel rectangle, clip policy, identity, handle lifetime, conversion, and aggregate limit before mapping or backend admission. Zero and nine zones fail without work; one through eight are admitted subject to the existing geometry and resource contracts.

One opaque `madopilot_ocr_zone_scan_result_t` owns all immutable source, envelope, descriptor, effective-zone, unique candidate, membership, geometry, confidence, and text values. It retains no caller array, parent handle, frame, producer slot, mapping, backend, model session, lock, or callback. Retain/release are atomic; null is a no-op. Concurrent const access is valid while each caller owns a live reference. `info` exposes the complete summary, `zone_at` exposes caller-order effective geometry and group count, and `region_at`/`text_at` use a zone index plus group-relative region index. Empty groups are successful and reject region index zero. Every accessor initializes its output failure state before validation; borrowed strings live only as long as the result owner.

C callers may use a partial ABI 1.4 table only through complete per-entry extents. C++ checks the complete extent required by each high-level operation before reading any new pointer and returns typed unsupported-version failure rather than falling back. ABI 1.0, 1.2, and 1.3 callers continue to compile, link, negotiate, and execute against their frozen extents without observing the suffix.

### C++ projection and diagnostics

C++ `OcrProfileOptions` owns path strings and `ZoneScanOcrRequest` owns identity strings and zone storage. One private rebind helper repairs every projected string, nested view, zone pointer, count, and stride after initial construction, copy construction/assignment, and move construction/assignment; moved-from objects remain safely destructible. Each C call builds and retains its call-local projection for the complete call. `ZoneScanOcrResult` is move-only, clones only through C retain, releases exactly once, and exposes lvalue-only owner-bound zone, region, and text views.

OCR diagnostics append only presence-qualified public profile classification, opaque library model-instance correlation, exact source identity, one shared source-envelope summary, bounded zone/unique-candidate/membership/result-byte aggregates, exact request-scoped detector/recognizer work when available, timing/resource summaries, typed outcome, and exact loss. They never retain caller zone arrays or individual zone geometry, model/runtime paths or hashes, pixels or captured-content hashes, recognized text, labels, backend/runtime names, credentials, raw native identifiers, free-form backend output, or unrelated desktop metadata. Process-wide counter deltas are not accepted as request evidence because concurrent operations make them non-authoritative. `Off` continues to allocate no diagnostic queue.

## Alternatives

- **Extend `madopilot_default_ocr_options_t` or reuse `engine_create_with_default_ocr`.** Rejected because it would reinterpret a frozen 40-byte default-named contract and make old-library negotiation ambiguous.
- **Make the bounded detector the `v0.3.1` default.** Rejected because it changes released G-004 behavior and evidence.
- **Repeat singular OCR for every zone.** Rejected because it changes grouped detector pixels, ordering, interruption, and performance and scales detector work with zone count.
- **Retain caller arrays or parent handles in the result.** Rejected because it creates hidden lifetime coupling and may pin capture/backend resources.
- **Default C++ copy/move of pointer-bearing C records.** Rejected because relocated strings and vectors leave dangling or cross-object pointers.
- **Infer detector/recognizer work from shared backend counters.** Rejected because concurrent snapshot deltas cannot be attributed to one operation.
- **Expose a callback, watcher, partial result, or async owner.** Rejected because the operation is synchronous, independently owned, and already bounded without another scheduling surface.

## Consequences

Integrators that want the bounded profile must select it explicitly and supply controlled model/runtime paths; existing callers change nothing. C callers must negotiate ABI 1.4 and keep request arrays and views live through the synchronous call. C++ request values pay explicit copy/move rebinding, while result ownership remains thin and exception-free over C.

ABI 1.4 type fields, numeric values, mandatory prefixes, function order, and ownership cannot move within major 1. Future additions append new fields or entries and preserve the frozen 648-byte ABI 1.3 prefix. The added owner stores each unique candidate once plus compact memberships; public projection adds no backend copy, queue, retry, or detector work.

## Verification

- Rust contract/facade tests cover explicit/default profile construction, singular preservation, one/three/eight zones, zero/nine refusal, empty groups, overlap safety, interruption, late completion, close, and independent ownership.
- Rust/C/Apple Clang/MSVC layout probes freeze sizes, alignments, field offsets, mandatory prefixes, values, entry offsets, and the complete extent.
- C boundary tests cover every pointer/count/stride/element/accessor failure, output initialization, panic containment, concurrency, parent teardown, and old-header execution.
- C++ ownership tests mutate sources after every copy/move form, vary result release order, reject incomplete tables, and mutation-test the centralized rebind rule.
- Diagnostic tests cover `Off`, ordinary, overflow, contention, close/drain, presence semantics, exact loss, and forbidden payload absence.
- Hosted Windows and macOS CI compile and run the tracked Rust/C/C++ examples, frozen headers, and independent CMake consumers before any release-host evidence is claimed.
- The predeclared matrix and retained observed outputs live under [`../../rasen/changes/phase-3-1-ocr-profile-and-zone-public-surfaces/evidence/`](../../rasen/changes/phase-3-1-ocr-profile-and-zone-public-surfaces/evidence/).