# ADR 0013: Detach Windows capture frames before publication

- **Status:** Accepted
- **Date:** 2026-07-30
- **Production amendment:** 2026-07-31
- **Resolves gate:** `G-002` from
  [../validation-gates.md](../validation-gates.md)
- **Supersedes:** _none_

## Context

Windows Graphics Capture (WGC) delivers each
`Direct3D11CaptureFrame` from a finite producer pool. A public MadoPilot frame
can outlive the next capture callback, can be mapped lazily, and can remain
owned by a backend after its capture session begins closing. Publishing the WGC
frame or its producer surface would therefore couple caller retention to
producer progress and to the WGC session's lifetime.

The G-002 prototype measured WGC pool sizes two, three, and four against direct
retention, fresh private textures, lease-aware private textures, and an unsafe
two-texture ring. It used 120 warm-up frames, 600 measured frames, delayed
retained mapping and backend work, lifecycle races, and two 4K displays.

Direct retention stopped after 6, 9, and 12 measured frames respectively:
increasing the pool only postponed exhaustion. Every detached positive row
delivered 600 frames, while the blind ring changed retained and backend
digests. Pool sizes three and four improved no hard outcome over two. The
selected candidate then passed 106 lifecycle rows and all three 4K display
rows. The deterministic fence case rejected a queued delegate without
admitting it to the owner, and every final live-resource count was zero.
The complete revision-bound record is
[../evidence/g-002/](../evidence/g-002/).

## Decision

The production Windows capture Adapter uses a WGC free-threaded producer pool
of exactly two frames and never publishes a `Direct3D11CaptureFrame` or its WGC
producer surface.

For each publishable frame, the WGC callback:

1. acquires the frame and validates its format and content size;
2. obtains a compatible Adapter-owned default-usage D3D11 texture whose lease
   count is zero;
3. issues the full-resource GPU copy;
4. releases the WGC frame before enqueuing or publishing detached ownership.

The callback may acquire, validate, copy, account, enqueue, and release. It may
not map to the CPU, wait for GPU or consumer completion, run a backend, invoke a
host callback, or block for a detached texture.

A detached texture is reusable only after the last public frame, mapping, and
backend lease releases it. The Adapter keeps a finite detached-texture budget.
When every compatible texture is leased and the budget is exhausted, the
callback releases the current WGC frame and records an observable bounded-queue
drop; it neither overwrites leased content, blocks, nor allocates without a
bound. The production Change sets the queue/budget value and its `G-013`
performance ceiling, but may not weaken this ownership rule.

Mapping and backend conversion happen outside the WGC callback. Their leases
keep the detached texture and the D3D11 device resources they need alive even
after capture admission stops.

On a content-size change, the transition frame is not published. The Adapter
recreates the two-frame WGC pool for the new size and advances geometry
revision. Detached old-size frames complete under their old revision; unused
incompatible textures retire, while leased old-generation textures remain
alive until release.

Close and reset follow this order:

1. stop admission;
2. detach the owner from lifetime-independent shared callback state under the
   same mutex used for callback admission;
3. unregister `FrameArrived`, drain callbacks already admitted, and publish the
   owner fence;
4. keep only the target-`Closed` handler's lifetime-independent terminal latch
   active through the native session-close decision;
5. close the capture session when the latch does not prove it already ended,
   then unregister `Closed` and close the producer pool;
6. allow detached frame, mapping, and backend leases to finish from their own
   resources;
7. destroy old private resources and the old device only after their last
   lease.

The WinRT delegates capture only the shared callback state, never a raw Adapter
owner. After owner detachment, `FrameArrived` is rejected without touching the
owner. The target-`Closed` delegate may still set its independent atomic latch,
but it cannot re-enter the owner. This refinement prevents an authoritative
native end from being lost between the owner fence and the session-close
decision. An already-closed native result is idempotent success during teardown.

Close is idempotent. Target loss or device reset starts a new stream epoch only
when a platform implementation proves continuity. The production Windows
Adapter proves no device-recovery continuity and therefore terminates with the
typed device outcome. If a later implementation adds recovery, it uses a fresh
WGC session and D3D11 device, and never repurposes storage still leased by an
old generation.

## Alternatives

**Publish the WGC frame or surface directly.** Rejected because retained frames
exhausted every producer-pool size. Pools two, three, and four stopped after 6,
9, and 12 measured frames, so a larger pool delays rather than removes the
ownership defect.

**Use pool size three or four with detachment.** Rejected because every detached
candidate already met correctness and progress at size two, and the larger
pools improved no hard gate, maximum arrival gap, or resource bound. Two is the
smallest passing producer allocation.

**Allocate a fresh private texture for every delivered frame.** Rejected as the
production policy because it creates and destroys a texture per callback.
It proved that detachment is sufficient for correctness, but lease-aware reuse
provides the same result without making per-frame allocation the design.

**Blindly overwrite a two-texture private ring.** Rejected because capture
continued while delayed retained and backend digests changed. Producer progress
does not make reused pixels immutable.

**Wait in the callback for a texture lease.** Rejected because caller retention
could then stall the WGC producer through the Adapter's own wait, recreating the
failure detachment is meant to remove.

**Map every frame to CPU memory in the callback.** Rejected because it adds a
GPU wait and full CPU transfer to the producer path and violates the
non-blocking platform-callback boundary. CPU mapping remains lazy consumer work.

## Consequences

- Every admitted Windows frame pays one full-frame GPU copy before publication.
  The 4K runs measured that traffic explicitly. Phase 2 `G-013` must budget
  capture/copy latency, mapped bytes, memory, drops, and startup without
  relaxing correctness.
- Public retained frames cannot pin WGC producer slots. Their native owner is a
  private texture lease whose lifetime is independent of the session and pool.
- The detached-texture budget introduces an observable drop case. A slow or
  indefinitely retaining caller consumes its finite capacity but cannot cause
  overwrite, unbounded growth, or callback blocking.
- Resize and reset need generation-aware resource retirement rather than one
  global texture ring.
- The Windows Adapter must retain device resources for old in-flight leases
  while a new session or device begins. Destruction order is therefore part of
  its contract and tests.
- The production adapter refines the prototype's handler-unregistration order
  without weakening owner isolation: only a lifetime-independent atomic
  target-end latch remains callable after the owner fence. The amendment and
  deterministic regression evidence are recorded in
  [../evidence/g-002/production-amendment.md](../evidence/g-002/production-amendment.md).
- The prototype source is not imported into a product package. Production code
  must implement the rule through the established capture contracts and pass
  the plan in
  [../windows-capture-contract-tests.md](../windows-capture-contract-tests.md).
- This decision adds no product dependency and makes no minimum-Windows claim.
  `G-001` still decides minimum operating-system versions and availability
  handling.

## Verification

The accepted Windows report is bound to product base
`7ae9050e9445a746eb2237c721c05eca4f7a1618`, prototype manifest
`3934dcf89d234cdf4f9460f8b53a30385c9397f6a0cb1f923ac806b6d82b84ae`,
Windows 11 build `26200.8894`, MSVC 19.44, Windows SDK 26100, the RTX
4080 driver, and a signed two-4K topology. It records:

- six passing detached matrix rows and six expected negative-control
  rejections, with no unexpected result;
- 106 passing lifecycle rows, including 100 close races, resize, in-flight
  leases, target close, a deterministic queued-delegate fence, and injected
  reset/recreation;
- three passing 4K rows outside normal work hours;
- no owner callback admission after the fence, one intentionally queued
  delegate rejected safely, zero sequence regressions, complete close/reset
  below 2 seconds, and zero final live resources.

The evidence, source-manifest method, raw-run hashes, candidate rows, lifecycle
aggregates, and redaction statement are in
[../evidence/g-002/](../evidence/g-002/).

The production amendment adds deterministic checks for an admitted target-close
callback completing before the teardown decision, a target-close latch arriving
after owner admission has stopped, idempotent already-closed native cleanup, and
finite teardown ownership. It does not change the accepted producer-pool,
detachment, texture-reuse, or callback-work result.

There is no production Windows Adapter yet, so this Change cannot add its
contract tests. The later implementation is conforming only when it implements
[../windows-capture-contract-tests.md](../windows-capture-contract-tests.md),
passes the shared Adapter contract suite and native WGC cases, and records its
Phase 2 `G-013` budgets. Documentation and strict Rasen validation prevent that
Change from treating this ADR as a capability claim by itself.
