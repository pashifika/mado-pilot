# Windows capture ownership contract-test plan

This plan translates
[ADR 0013](adr/0013-windows-capture-frame-detachment.md) into tests the future
`mado-pilot-platform-windows` implementation must add. It is an acceptance
contract, not evidence that native Windows capture exists today.

The production Change must implement both layers below:

- deterministic Adapter-independent tests that exercise ownership, queue,
  lifecycle, and publication through controllable fakes;
- native WGC/D3D11 tests on the named Windows verification host for behavior a
  fake producer cannot establish.

The common public capture contract remains platform-neutral. Test-only
instrumentation may expose producer-slot, detached-texture, lease, callback, and
drop counters, but Windows or D3D11 types must not enter `mado-pilot-core`,
`mado-pilot-capture`, the facade, or the C ABI.

## Adapter-independent contract cases

| Case | Required observable result |
|---|---|
| Producer slots are returned before publication | Retaining more public frames than the producer-pool size does not retain a producer token or prevent the fake producer from delivering the complete sample |
| Detached owner is authoritative | A published frame owns private detached storage and contains no WGC-frame or producer-surface owner |
| Lease-safe reuse | A texture is never selected for a copy while a public frame, mapping, or backend lease owns it |
| Finite pressure | Holding every detached texture makes the next candidate an observable queue/capture drop; it does not block, overwrite, or allocate beyond the configured bound |
| Resume after release | Releasing one old lease makes capacity available and producer progress resumes without reopening the session |
| Lazy mapping boundary | Mapping begins outside the producer callback, uses the frame's own descriptor and geometry revision, and preserves exact row pitch and bytes |
| Backend lifetime | Backend work admitted before close observes the complete original frame after session and pool closure |
| Callback boundary | Test hooks prove the callback performs only acquire, validate, copy, account, enqueue, and release; mapping, waits, backends, and host callbacks fail the test if invoked there |
| Resize transition | The first changed-content-size frame is not published; the new pool uses the new size and advances geometry revision once |
| Old resize generation | Retained old-size frames and mappings complete under their old revision while unused incompatible textures retire |
| Close admission fence | Both native handlers use lifetime-independent shared state; owner detachment and admission are synchronized; no callback is admitted to the owner after the fence; a delegate deliberately paused before admission is rejected safely after close |
| Idempotent close | Concurrent and repeated close calls converge on one terminal state without double release or a host callback under a lock |
| Target loss | Loss stops admission, reports the typed terminal outcome, and does not mutate retained frames |
| Device reset | The replacement device/session starts a new stream epoch; old-generation resources live only until their old leases release |
| Resource bound | WGC frames never exceed two; detached and staging resources never exceed their configured bounds; every counter reaches zero after final release |
| Diagnostic redaction | Drop, close, reset, and mapping failures exclude pixel bytes, captured hashes, recognized text, titles, process paths, and native serial identifiers |

The finite-pressure test must include an unsafe test Adapter that overwrites a
two-texture ring. The retained digest oracle must fail against that Adapter so a
test that can no longer detect aliasing cannot silently pass the production
implementation.

## Native Windows contract cases

Native tests use a test-owned synthetic Win32 target and
`Direct3D11CaptureFramePool::CreateFreeThreaded`. They never capture an unrelated
desktop or application window.

The retained-frame case matches the G-002 evidence:

1. Create a two-frame WGC pool and a finite detached-texture pool.
2. Discard 120 delivered warm-up frames.
3. Require 600 delivered frames within 20 seconds after warm-up, with no
   post-warm-up arrival gap above 500 ms.
4. Retain every third frame, delay validation by 90 delivered frames, alternate
   mapping and backend leases, and cap retained work at 40.
5. Validate BGRA8 format, dimensions, `RowPitch >= width * 4`, deterministic
   marker coherence, wrap-aware sequence progress with no regression or
   greater-than-500 ms stall, and identical immediate and delayed digests.
6. Stop admission, unregister both handlers, drain admitted callbacks, publish
   the fence, close WGC, complete in-flight work, and require every resource
   counter to reach zero.

The native lifecycle suite adds:

- resize through 1280×720, 960×540, 1920×1080, and back to 1280×720;
- 100 close races at 0, 1, 5, 15, and 30 milliseconds after callback
  admission, 20 repetitions each;
- close with mapping and backend leases in flight;
- a deterministic pre-admission barrier that holds one queued delegate while
  close detaches the owner and publishes the fence, then proves safe rejection;
- a real controlled-target close and an idempotent second close;
- injected device-loss admission stop followed by complete D3D11 device and WGC
  session recreation.

Injected device loss verifies state-machine ownership only. A physical device
removal, TDR, or driver upgrade is not claimed unless a separately reviewed
native run actually performs it.

Outside 08:00–19:00 local time, the named host also runs 600 frames on each of
its two 3840×2160 displays and 300 frames while moving the controlled target
across their signed-coordinate seam. The record includes Windows build, GPU and
driver, MSVC, SDK, DPI, scale, signed rectangles, and hashed display
identifiers, with no captured payload.

## Performance obligations

G-002 chooses correctness and ownership, not numeric product budgets. The
production Change must add Phase 2 `G-013` profiles that measure at least:

- capture arrival and callback-copy p50/p95 latency;
- full-frame copied bytes and lazy mapped bytes;
- detached-texture, staging, process-resident, and relevant GPU-memory peaks;
- producer progress, queue drops, stale/coalesced work, and recovery after
  pressure;
- session startup, resize recreation, callback drain, complete close, and
  admission-stop-to-new-session reset recovery;
- 1280×720 and two-display 4K workloads on the named Windows host.

Every timed sample keeps its correctness oracle. A throughput improvement that
changes retained pixels, pins producer slots, exceeds a bound, or hides a drop
fails before latency is considered.

## Execution and evidence

Adapter-independent cases run in ordinary workspace tests. Native WGC cases may
require an interactive Windows session and therefore may not be available on a
headless pull-request runner. A skipped native case must report why it did not
run; a skip is not support evidence.

Before the Windows Adapter is described as implemented, the production Change
must retain a revision-bound, redacted report from the named host, link each
case above to its test, set the affected `G-013` budgets, and pass the shared
capture contract suite. Until then,
[docs/architecture.md](architecture.md) continues to say native capture is not
implemented.
