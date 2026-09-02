# ADR 0064: Adopt token-driven native template-watch Qualification V2

- **Status:** Accepted
- **Date:** 2026-09-02
- **Resolves gate:** `G-005` native WGC/ScreenCaptureKit qualification successor protocol
- **Supersedes:** _none_

## Context

The native template-watch V1 campaign opened a WGC or ScreenCaptureKit session
and immediately required an unstimulated `latest` frame. Both capture systems
start asynchronously, and accepted session open is not proof that an unchanged
target has published a pixel-bearing frame. The five-process, 24-workload
campaign also combined deterministic watcher semantics, native integration,
statistical evidence, fixture lifecycle, environment admission, privacy, and
support classification in one executable boundary.

PRs #59–#64 produced revision-bound implementation and qualification records.
Those records include accepted Windows cohorts, an Apple terminal-red cohort,
non-reproducing diagnostics, current-source qualification failures, repaired
private apparatus boundaries, and consumed builder attempts. None may be
rewritten or relabeled as evidence for a changed oracle. The PR #64 merge result
on `dev/0.4.0`, commit `6fcdad1c0e5ed912dc0a226a98fb47899a2454a4`
and tree `f75d0935343469791c7546e1e9fd6d21fbc68bb3`, is the implementation
base for a new qualification identity.

## Decision

Adopt Qualification V2 with an acknowledged, pixel-encoded visual token as the
only native post-open readiness authority. Opening a native session establishes
accepted asynchronous capture state, not initial pixel publication. After open,
the owned fixture commits a unique token and requested template-marker state in
one UI transaction, then acknowledges that token. Readiness requires a newer
pixel-bearing frame from every required session to decode the exact acknowledged
token under the expected target, stream, epoch, sequence, and geometry
authority. Sleeps, unstimulated `latest` acquisition, status-only samples, and
platform-specific priming are not readiness authority.

The private cross-platform logical encoding is a 10-by-9 cell grid:

- an asymmetric fixed 10-cell sentinel at the top and bottom;
- 32 little-endian token bits followed by their 32-bit inverse;
- one marker-visible bit; and
- a five-bit checksum over the token and marker state.

Token zero is reserved. A fixture-lifetime sequence issues values from one
through `u32::MAX` once and reports typed exhaustion instead of wrapping.
Decoders require the exact cell count, both sentinels, a nonzero token, the
inverse, and checksum agreement. A valid older token remains diagnostically
observable but cannot satisfy an expected newer token. Invalid or partially
rendered states do not establish progress; observation continues only within the
existing absolute phase deadline. Platform renderers retain their own physical
cell size and bounded pixel tolerance and place the grid outside the template
search region.

Split durable qualification authority into three lanes:

1. Lane A is required deterministic replay/OpenCV authority for scheduler and
   query semantics, including deadlines, stability, coalescing, saturation,
   fairness, stale generations, lifecycle outcomes, diagnostics, and complete
   work accounting.
2. Lane B is the required compact native semantic contract on each approved
   Windows and macOS host. It covers target and permission admission,
   post-open token progress, correlated matching, fair two-session progress,
   geometry generation, retained ownership with fresh-session progress,
   lifecycle termination, and explicit cleanup.
3. Lane C is optional evidence and endurance authority. It may retain the V1
   workload registry, statistical latency and resource distributions, topology,
   executable and fixture identities, and approved-host provenance, but it does
   not replace or reinterpret a Lane B semantic result.

Lane B reports semantic, cleanup, startup, watcher, teardown, and bounded native
diagnostic facts independently. Its outer result distinguishes `PASS`, product
`FAIL`, `INFRA`, and `UNSUPPORTED`. Fixture launch and finalization are excluded
from watcher latency. Pre-execution apparatus failure does not become a product
failure. The merged Apple boundary still requires exact launched lifetime
`Live` before fixture acceptance and explicit consumption of typed finalization;
the merged privacy boundary still separates bounded safe provenance from host
compatibility authority.

Every V1 PR #59–#64 source, protocol, hash, failure, non-reproduction, consumed
attempt, and support classification remains immutable authority only for its
original revision. V2 source, token codec, fixtures, scenarios, and result schema
receive new identities. Correcting an infrastructure-only V2 defect creates a
new recorded attempt; an unchanged semantic failure receives no hidden retry,
replacement sample, deadline change, or oracle change.

This decision does not identify or repair the historical ScreenCaptureKit
suspension. It also does not promise an initial frame for an unchanged target.
Any capture bootstrap guarantee and any causal ScreenCaptureKit lifecycle repair
require separate evidence and decisions.

## Implementation and current evidence

Product commit `0dabd9fc2a336824d1ba5779344b9affd527cff6`, tree
`f8d983c063adaa8234ea2c2a92d636c20fb8f720`, implements the V2 token codec,
both fixture renderers, fair exact-token session synchronization, the fixed
eight-scenario Lane B runner, independent typed results, and explicit fixture
finalization. The private Lane B schema is
`mado-pilot.native-template-watch-contract.v2`; the former 24-workload registry
and target-specific statistical enforcement require the separate
`--lane-c-evidence` mode.

The approved Apple M1 Pro host on macOS 26.6.2 build 25G83 with SDK 26.5 ran the
exact release runner once. All eight semantic facts and all eight cleanup facts
were `PASS`; captured target scale was exactly `[2000, 2000]` milli-scale units,
and finalization reported the accepted-launch `Live` observation, Stop
acknowledgement, both process lifetimes `Lost`, bounded containment, joined
reader, drained output, unchanged executable identity, zero active cleanup, no
exhaustion, and scheduled cleanup equal to completed cleanup. The report,
runner, fixture, and codec source each have separately retained SHA-256
identities.

The required Windows Lane B result is pending. The Apple result therefore
establishes only the Apple integration boundary and does not yet satisfy the
cross-target support condition. Later source-tree changes need either affected
scenario reruns or reviewed complete-diff applicability; documentation,
workflow, or Lane A-only changes do not become attributed execution.

## Alternatives

- Retain unstimulated first-frame acquisition and extend its timeout. Rejected
  because elapsed time cannot correlate a frame to producer progress.
- Toggle only marker visibility. Rejected because a delayed or coalesced frame
  can contain a valid but stale marker state.
- Acknowledge a control nonce without rendering it. Rejected because command
  acceptance does not prove captured content.
- Keep one mandatory 24-workload native campaign. Rejected because it duplicates
  deterministic authority and couples product semantics to statistics and
  apparatus admission.
- Treat PR #59–#64 results as V2 evidence by reviewed applicability. Rejected
  because the readiness oracle, fixture protocol, and qualification identity
  change.

## Consequences

- Native WGC and ScreenCaptureKit watcher support remains withheld until the
  exact V2 candidate passes Lane A and both required Lane B host contracts with
  bounded cleanup.
- The V1 records and ADRs 0053, 0057, and 0060–0063 remain unchanged historical
  evidence. V2 adds evidence rather than mutating them.
- Native fixtures and benchmark adapters gain a private token protocol; no public
  Rust, C, or C++ API changes.
- The required native gate becomes smaller and causal. Long-running resource,
  topology, and statistical claims remain possible in Lane C without being
  confused with semantic execution.
- OCR predicates, callbacks, C/C++, automatic input, arbitrary external
  application/template/ROI compatibility, real-time guarantees, packaging, and
  the `v0.4.0` release remain outside this decision.

## Verification

- Platform-independent codec tests cover absent and visible round trips, reserved
  zero, every single-cell corruption, partial old/new transitions, stale tokens,
  orientation transforms, boundary values, and sequence exhaustion.
- Windows fixture capture tests must decode the acknowledged token across the
  approved DPI transition and reject stale or invalid intermediate states.
- macOS fixture capture tests must decode the same logical token at Retina scale
  and reject stale or invalid intermediate states.
- Lane B must exercise the compact contract once on each approved host and record
  exact source, codec, executable, fixture, host, semantic, cleanup, timing, and
  classification identities.
- Documentation and report-schema review must prove that no PR #59–#64 artifact
  changed and that no V2 result is attributed to an earlier revision.
