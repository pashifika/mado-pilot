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

CI at source `cd89903` exposed two harness defects before either result could be
accepted: the Windows-only reader moved a decoded line from a pattern guard, and
the Apple finalizer could observe channel disconnection immediately before the
reader thread's finished state. Product commit
`318ad1c49102d9fcd33448d12ee75d739bf04336`, tree
`00fe097cb4b1ea4eaf58abc87f496584f50d3ae8`, removes both races without changing
the watcher or public API.

The first `318ad1c` Apple invocation exited `0`, but its complete single-line
report was not retained by the execution apparatus; that observation is retained
as `INFRA` and has no authority. After stdout retention was corrected, the
approved Apple M1 Pro host on macOS 26.6.2 build 25G83 with SDK 26.5 performed
one separately identified report-bearing invocation. All eight semantic facts
and all eight cleanup facts were `PASS`; captured target scale was exactly
`[2000, 2000]` milli-scale units, and finalization reported the accepted-launch
`Live` observation, Stop acknowledgement, both process lifetimes `Lost`, bounded
containment, joined reader, drained output, unchanged executable identity, zero
active cleanup, no exhaustion, and scheduled cleanup equal to completed cleanup.
The report, runner, fixture, and codec source each have separately retained
SHA-256 identities.

The initial `318ad1c` Windows invocation was `UNSUPPORTED` because an
awareness-dependent monitor-DPI probe incorrectly reported the approved
mixed-DPI host as uniform. Source `936dde1` replaced that probe with
awareness-independent monitor scale and passed scenarios one through six, then
terminated red in `lifecycle_termination`: acknowledged fixture destruction did
not reach the pending watcher as `TargetLost` within five seconds. A minimized
platform regression at
`5b4d74e89298b18d6c69bcc6a0bc841a6147ca5a` established a quiet in-flight
frame wait before the same destruction acknowledgement and reproduced the
missing terminal.

Product source `bb412dc46fccadc2a23ab55c25248c9fda3874cd`, tree
`731febdd8ad58fe9a9a5b40334a8894144e974f1`, bounds an idle Windows frame
wait to a 100 ms interval and rechecks raw native-key presence. The interval
limits idle liveness work to about ten probes per second while returning control
to the existing target-loss mapping; it is a default, not a statistical
qualification budget. It shares the caller's clock, never extends an earlier
caller deadline, and preserves cancellation and first-terminal-fault
precedence. The bound applies only when the raw key disappears: a present or
immediately recycled HWND still requires authoritative `Closed` or a caller
deadline, and a synthetic caller clock must advance.

Independent review accepted that root fix but required a host-durable lock
before support promotion. Windows execution source
`3e3079f2d71243d1b25d5bda79e3672c0cd3df07`, tree
`77f127c281e35b4475cb5e931ae10b969b6f1a64`, extracts a private deterministic
liveness seam and covers missing-key recheck, already-expired caller deadline,
cancellation during an admitted wait, first-terminal-fault precedence, and a
frame arriving after one internal expiry. Mutating only
`acquire_frame(&bounded)` to `acquire_frame(operation)` kept the code compiling
and changed the named suite from five passes to three passes and two failures:
the missing-key test observed `DeadlineExceeded` instead of `TargetLost`, and
the arriving-frame test lost the retry to `DeadlineExceeded`. Restoring the
single expression returned all five tests to green on the exact source tree.
Test-only successor `53608af4aeb9f2cbdad3e7e22dc0408257304286`, tree
`10d9f168f65844890e16d8a7fe14c8c43b349859`, documents each scripted clock-read
sequence and separately asserts that a live earlier caller deadline, seven
milliseconds from origin when the clock is at five milliseconds, clamps a
100 ms liveness interval to that exact caller deadline. The approved Windows
host passed all six focused tests once, and independent delta review returned
`CLEAN`. Every added line is under `#[cfg(test)]`; no product artifact changed,
so the five-test primary mutation proof remains bound to `3e3079f` and Lane B
was not rerun for the test-only delta.

The hosted regression also established a 200 ms quiet wait, observed destruction
acknowledgement, returned `TargetLost`, passed the separate one-second
SystemClock post-acknowledgement latency assertion under a five-second safety
timeout, joined the waiter, and closed the session.

The first `bb412dc` Windows runner build omitted the approved OpenCV environment
and remains `INFRA` with zero `--native-contract` invocations. Corrected
infrastructure then produced a passing intermediate contract without semantic
retry. The `3e3079f` Windows execution run built once and invoked the contract
once. All eight semantic facts and all eight cleanup facts were `PASS`, including
mixed-DPI geometry, target/session/engine termination, fixture-process reaping,
reader join, bounded containment, and output drain. The runner, fixture, codec,
and report SHA-256 identities are respectively
`bd827d1be2114145a1f7c436ef70adbe0a6047d4e1e1f5bfdfcbc66bcf92b714`,
`5a50466c3fbf420df3d56ae466a6ae233254705f340098bca677a574ae9e8957`,
`8b59a9cbc375e21ca39514a6c2f2ca16ebdaec47b82126d7c7d36c7809dc8f10`,
and `a4b6800217f1f96872d7f25f0614c1ff2a3a6f1942293267f990193557a92fbe`.
Branch-policy/Rust runs `33610306454`/`33610306322` passed at the execution
source. Runs `33612368422`/`33612368443`, including repository, Windows, and
macOS jobs, passed at test-only successor `53608af`.

The complete product diff from accepted Apple source `318ad1c` through
`53608af` contains runtime changes only for Windows; the final delta is
test-only. Reviewed complete-diff applicability therefore preserves the Apple
result without attributing an unexecuted process. Required Lane A and the
complete hosted Windows/macOS jobs pass on their named evidence sources. The V2
cross-target support condition is satisfied on the named platform floors.
Later behavior-affecting changes still need affected-host execution or reviewed
complete-diff applicability; documentation-only changes receive no attributed
execution.

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

- Native WGC and ScreenCaptureKit watcher support is qualified on the named
  release targets by the exact V2 Lane A and two-host Lane B evidence above.
  Later behavior-affecting source changes require fresh affected authority.
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
- Windows execution source `3e3079f` decodes exact tokens through the approved
  mixed-DPI geometry transition, passes the deterministic liveness precedence
  suite and its single-point mutation proof, and passes the acknowledged
  in-flight target-loss regression. Test-only successor `53608af` pins the
  earlier caller deadline and passes all six focused tests and required CI.
- macOS source `318ad1c` decodes the same logical token at Retina scale and
  rejects stale or invalid intermediate states; reviewed Windows-only runtime
  applicability carries that result through `53608af`.
- Both Lane B reports record exact source, codec, executable, fixture, host,
  semantic, cleanup, timing, and classification identities, and all sixteen
  semantic/cleanup scenario pairs pass.
- Historical baseline comparison proves that no PR #59–#64 artifact or frozen
  prose section changed and that no V2 result is attributed to an earlier
  revision.
