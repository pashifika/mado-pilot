# G-002 production close-order amendment

## Scope

The accepted G-002 prototype established frame detachment, a two-frame WGC
producer pool, lease-safe private textures, and owner fencing. The production
Windows adapter preserves those results. This amendment records one teardown
ordering refinement found while implementing the real target-loss path.

## Production race

The prototype order removed both `FrameArrived` and
`GraphicsCaptureItem.Closed` before draining admitted callbacks. In production,
a frame-side closure or disappearing native key can stop owner admission before
the authoritative `Closed` delegate runs. Removing `Closed` at that point can
lose the only positive evidence that WGC already ended the capture.

Inferring the answer from a still-present HWND or monitor is not safe because a
target may be replaced without changing that mutable observation. Calling back
into the detached owner is also not safe.

## Refined order

The production adapter uses this order:

1. stop owner admission and unregister `FrameArrived`;
2. drain callbacks already admitted and publish the owner fence;
3. keep the target-`Closed` delegate's independent atomic native-end latch
   active through the capture-session close decision;
4. sample that latch after the drain;
5. close the capture session when the latch does not prove it already ended,
   treating an already-closed result as idempotent success;
6. unregister `Closed`, close the frame pool, and release the capture item last.

After step 1 the `Closed` delegate may only set its atomic latch. It cannot
admit or touch `SessionCore`, invoke host code, map, wait, or publish.

Native teardown runs on a fixed four-worker WinRT pool. A global 64-permit
budget is reserved before native session allocation and follows ownership
through live sessions, running jobs, queued jobs, and apartment-safe quarantine.
This keeps both worker concurrency and native ownership finite even if a worker
stalls or all workers fail. Explicit startup and restart checkpoint the caller's
operation while acquiring the executor slot and waiting for worker apartment
readiness, so cancellation and deadlines bound those waits as well. A caller
that stops waiting leaves one tracked startup generation in the global slot;
retries share it, and no replacement generation starts before every thread from
a failed generation has exited.

## Deterministic regression evidence

The production unit suite pins the amendment with:

- `native_end_state_is_sampled_after_the_admitted_callback_drain`;
- `authoritative_native_end_latches_after_owner_admission_stops`;
- `native_close_absorbs_an_already_closed_result`;
- `teardown_executor_starts_only_the_fixed_worker_count`;
- `cancelled_waiters_share_one_in_flight_teardown_generation`;
- `teardown_queue_is_finite_and_non_blocking`;
- `teardown_permits_bound_live_and_queued_session_ownership`;
- `teardown_start_observes_operation_cancellation_before_spawning`.

The synthetic native target test still exercises real target close, repeated
idempotent close, implicit cross-thread teardown, retained-frame mapping, and
its bounded watchdog. The amendment changes no callback publication work and no
frame-detachment or texture-reuse rule.
