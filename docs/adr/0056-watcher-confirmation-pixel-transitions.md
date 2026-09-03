# ADR 0056: Require pixel transitions for watcher confirmation frames

- **Status:** Accepted
- **Date:** 2026-08-29
- **Resolves gate:** rejected Windows native watcher matched-duration rows
- **Direction / Slice:** `version-one-delivery` / `phase-4-native-template-watch-qualification`
- **Depends on:** ADR 0050 and ADR 0055

## Context

The native watcher harness used a second `set-visible` command to request the frame that confirms a duration-stable match. On Windows, this command stored the already-visible marker state and called `InvalidateRect` plus `UpdateWindow`. The resulting pixels were identical. The acknowledgement proved that USER32 processed the paint request, not that DWM or Windows Graphics Capture published a distinct frame. ADR 0055 had already established that a same-state repaint may be coalesced, but duration confirmation and several later-frame workload boundaries still relied on one.

A source-level full-matrix diagnostic reproduced `DeadlineExceeded` in `window_persistent_appearance`. A subsequent isolated `window_strictly_newer` stress diagnostic at revision `1a77d63c9279e1ac9f753a7c413b53b789636913` produced a classified terminal-stage red in its first fresh process. The first visible confirmation completed after 26.803 milliseconds. After the 25-millisecond stability interval, the second identical-visible command was acknowledged, but the query reached its five-second terminal deadline with one confirmed observation, no pending or in-flight work, four completed backend runs, and zero backend failures. This localizes the failed edge to the same-state confirmation stimulus rather than capture startup, backlog, backend completion, or watcher scheduling.

## Decision

Whenever native qualification requires a later production frame while `watch-marker-v1` remains visible, the fixture SHALL change deterministic pixels outside the marker without changing marker pixels, target identity, placement, or extent. Fixture acknowledgement remains only fixture authority; a later production frame and the workload's existing terminal or progress oracle remain capture authority.

The shared harness names this operation `transition-visual`. The macOS binding uses the existing fixture `Transition` command. The Windows binding adds one private control that toggles the deterministic background fill and repaints on the fixture GUI thread. Duration confirmation, disappearance-reset confirmation, stale-generation confirmation, and paired-query rate-limit boundaries use this operation instead of a same-state `set-visible` repaint.

## Alternatives

- **Increase the five-second terminal deadline.** Rejected. Waiting longer cannot force a change-driven capture source to publish identical pixels.
- **Retry `set-visible` until a frame appears.** Rejected. Retries hide the invalid stimulus, make sample counts schedule-dependent, and still do not require changed pixels.
- **Change the marker itself.** Rejected. It would alter the prepared template input and invalidate the stable-appearance oracle.
- **Change production WGC or watcher scheduling.** Rejected by the classified red: all admitted backend work completed, no work remained pending or in flight, and the first confirmation succeeded.
- **Keep the correction Windows-only in shared workloads.** Rejected. The qualification protocol should express the required source transition once; only its target-native transport and rendering implementation should differ.

## Consequences

The correction is private benchmark and fixture apparatus. Public Rust, C, C++, capture, watcher, deadline, cancellation, and budget contracts remain unchanged. The template region remains byte-identical while pixels elsewhere in the captured source change, so a published frame still proves the same marker match over the required duration.

Windows adds one private message and bounded acknowledgement. macOS reuses an existing private command. Neither path adds a production allocation, queue, retry, worker, timeout, or fallback. Current benchmark source and executable identities change and all affected current cohorts must rerun; rejected and historical revision-bound evidence remains frozen under its original hashes.

## Verification

- The Windows fixture protocol test `watcher_visual_controls_decode_to_exact_bounded_acknowledgements` fixes the new control and acknowledgement spelling and keeps it disjoint from marker-state controls.
- `cargo check -p mado-pilot --bench native-template-watch --features native-template-watch-qualification --locked` compiles the shared harness and macOS binding.
- The pre-correction `window_strictly_newer` classified red is the mutation proof for replacing `transition-visual` with the former same-state `set-visible` enforcement point.
- The corrected Windows binding must pass five fresh sequential isolated `window_strictly_newer` processes with three warmups and 1,000 measured samples each before full-matrix evidence is accepted.
- Final Windows and macOS cohorts retain their existing no-retry, no-exclusion, no-replacement, exact-source, exact-executable, and later-production-frame gates.
