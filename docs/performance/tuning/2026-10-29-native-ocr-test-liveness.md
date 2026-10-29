# 2026-10-29: Native OCR test completion and cleanup

## Problem and intended outcome

The native close-race tests reused their two-second admission-handshake bound
for an entire successful OCR operation. In the [failed macOS OCR lane](https://github.com/pashifika/mado-pilot/actions/runs/40765435649/job/123088270402),
the test timed out before the worker completed; its later result send failed
because assertion unwind had dropped the receiver without joining the worker.
The test must distinguish slow valid completion from a broken synchronization
or cancellation contract, and must not leave a worker behind for the next test.

## Change and reason

- Keep the two-second gate handshake and existing cancellation/deadline response
  checks, including the 250 ms termination bound, unchanged.
- Stop applying the gate bound to full OCR completion in the accepted,
  grouped-HUD and declared-candidate close races. Their exact result, source
  correlation, busy-close and post-close oracles remain unchanged.
- Retain the gate and result receiver until the worker has joined, on success
  and on assertion unwind. Release an admission gate before cleanup joins, and
  preserve the original assertion if the worker also panics.
- Run each affected native test in its own exact-name child, using the existing
  testkit bounded-child runner. A 300-second whole-test liveness fuse includes
  backend opens, contract checks, repeated oracles and cleanup. It is deliberately
  separate from an operation latency budget; a real native hang is contained and
  terminated rather than blocking the suite in an unbounded in-process join.
  The existing runner retains cleanup ownership until exit and pipe completion.
  Each output stream is capped at 256 KiB, with overflow treated as failure.

The CUDA cancellation test shares the same worker cleanup and child boundary.
The parent retains native-test serialization until its child finishes. There
are no production, model, runtime, performance-profile or CI deadline changes.

## Evidence and effect

**Performance outcome: not-measured.** No speedup, native qualification or
before/after latency claim is made. The earlier failed run remains failed.
The new native-free regressions exercise completion beyond the handshake bound,
receiver retention and gate reuse after assertion/worker panic, and termination
and reaping of a genuinely stuck worker. The receipt is checked on the receiving
thread so a swallowed secondary send panic cannot falsely satisfy cleanup.

Native ONNX build, lint and oracle execution remain required on the existing
Windows and macOS CI lanes; a standalone Linux helper check is not that evidence.

## Trade-offs and disposition

**Retained for review.** Subprocess launch adds test-only overhead. A broken
native operation may now consume the whole-test fuse, but cannot overlap the
next test's process-global gate. The fuse does not replace or relax any existing
cancellation, ownership, correctness, memory or public deadline contract.
