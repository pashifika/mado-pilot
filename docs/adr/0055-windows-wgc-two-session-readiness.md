# ADR 0055: Fence Windows WGC sessions before watcher fairness stimulus

- **Status:** Accepted
- **Date:** 2026-08-29
- **Resolves gate:** rejected Windows native watcher final-enforcement `two_session_fairness` rows
- **Direction / Slice:** `version-one-delivery` / `phase-4-native-template-watch-qualification`
- **Depends on:** ADR 0050 and ADR 0053

## Context

The Windows `two_session_fairness` qualification workload opened a second production Windows Graphics Capture session, immediately started one watcher per session, and issued one controlled visible fixture transition. Final-enforcement and non-acceptance full-load diagnostics intermittently ended with `DeadlineExceeded`; a focused one-warmup/one-sample loop passed 20/20, proving that the accumulated full-load prefix was initially load-bearing.

At exact base source `be3a5f5f531f1f00729dd07cec03fbb9785e7385`, a content-redacted ordered probe reproduced the failure. The second session opened at 19,638 microseconds, both queries started at 19,684 microseconds, and the fixture acknowledged the visible repaint at 19,974 microseconds: 336 microseconds after open. At timeout the first query had accepted two publications and completed, while the second had accepted zero publications, had no last frame, and remained at generation zero. The process recorded two WGC callback entries, zero transition-lock contentions, two drained frames, one session publication, 561 completed backend runs, and zero active backend work.

Those facts falsify callback-lock loss, scheduler starvation, and backend leakage for the exact red sample. They establish that neither `GraphicsCaptureSession::StartCapture()` nor query creation is a publication-completion signal. Windows WGC is asynchronous and change-driven; a single transition can occur before a newly opened capture pipeline is proven able to publish it.

Two narrower corrections were rejected and retained. Passively waiting for an initial frame timed out because an unchanged window need not produce one. Repainting `absent` while the pixels were already absent and then waiting for backend-completed `prime_pending` state also timed out; an identical repaint may be coalesced, and backend completion is wider than acquisition readiness. The latter retained process had no substage probe, so its runtime timeout remains bounded to the absent-frame loop, either `prime_pending`, or either terminal wait; it is not relabeled as post-visible evidence.

## Decision

Keep production WGC, watcher scheduling, public API, ABI, deadlines, and accepted numeric budgets unchanged. Correct only the native qualification harness.

Before either fairness query exists, the Windows target hook performs an active source round trip through both production `Session` values:

1. maintain an independent frame cursor for each session;
2. command the repository fixture visible;
3. observe mapped visible marker pixels through the newly opened second session and the retained first session;
4. command absent unconditionally;
5. observe a newer mapped absent frame through both sessions;
6. repeat only while one 50-millisecond acquisition quantum missed an edge and the existing five-second outer operation deadline remains;
7. succeed only after both sessions observed both sides of one complete round trip.

The short quantum is not a sleep, success condition, retry of the real match stimulus, or latency allowance. It only bounds one blocking acquisition probe so the harness can generate a newer source edge. Typed failures other than a short acquisition deadline remain hard failures. Every observed frame travels through fixture pixels, compositor/WGC, callback processing, stream publication, public session acquisition, and CPU mapping; callback or acknowledgement counters alone cannot satisfy readiness.

macOS implements the same private target-hook signature as a no-op because current evidence requires no ScreenCaptureKit-specific handshake. The shared workload retains ordering and terminal authority.

After both watchers start, the harness waits only until each query has accepted at least one absent-baseline capture publication and remains nonterminal with zero confirmed observations. It deliberately does not wait for slow backend completion. The workload then issues exactly one authoritative visible match stimulus, requires both queries to terminate as matched, and requires each query's publication count to increase after its pre-stimulus baseline. Existing backend-idle, second-session close, work-accounting, cleanup, allocation, privacy, and producer-progress gates remain unchanged.

## Alternatives

- **Sleep after opening the second session.** Rejected. Elapsed time correlates with readiness but does not prove a frame traversed the production path.
- **Increase the five-second deadline.** Rejected. Waiting longer after a lost change-driven edge does not create a newer source revision.
- **Treat `StartCapture()` or fixture acknowledgement as ready.** Rejected. Neither proves a frame entered the public maintained stream.
- **Repaint the already-absent state.** Rejected by retained evidence. Identical pixels may be coalesced and produced no bounded initial-frame guarantee.
- **Wait for backend-completed pending state.** Rejected. It widens an acquisition barrier into slow matching and changes the intended fairness schedule.
- **Retry visible while queries are active.** Rejected. It hides the race and contaminates the single-stimulus fairness oracle.
- **Block production `Session::open` on a first frame.** Rejected. It changes public latency and failure semantics to satisfy benchmark apparatus.
- **Change callback serialization or watcher scheduling.** Rejected by the ordered probe: callback contention and backend active work were both zero in the exact red process.

## Consequences

The correction is Windows-specific where WGC semantics differ, while the shared watcher contract remains strict. Readiness work stays inside measured `two_session_fairness` latency because opening and making the second production session usable is part of that workload. No existing budget is relaxed from a failed run.

The accepted uncommitted correction, diff SHA-256 `bc4fe0a907342cd33e771494778ad5e3d529b0f46923375d160427c0b66fbb77`, produced optimized diagnostic executable SHA-256 `ed68a9a23743c878b9dc432af99cccb1a37f0c944cebdc4c3c7ddc8beb647d0c`. It passed 100 consecutive fresh focused Windows processes with zero failure. Process wall time ranged from 1.089537 to 1.479357 seconds. Each success necessarily observed visible and newer absent through both sessions, accepted an absent-baseline publication on both queries, observed a post-stimulus publication increase on both queries, and completed both matches because those predicates are part of the hard sample result.

The same executable then passed three fresh sequential full-load diagnostics, all 24 workloads per process, with no panic or incomplete progress. Process wall times were 115.189718, 115.581089, and 115.135424 seconds. The complete `two_session_fairness` warmup-plus-sample spans were 8,791.720, 8,751.894, and 8,772.608 milliseconds, which does not justify changing ADR 0053's revision-bound latency ceilings.

Earlier apparatus-invalid, unchanged-source final, focused 0/20, red-capable full-load, ordered probe, passive-fence, and refined-fence evidence remains immutable and is not reused as acceptance.

## Verification

- Preserve the 100/100 focused evidence under `.omp/evidence/p040-be3a5f5-active-handshake-focused100-20260829T035644Z`.
- Preserve the 3/3 full-load evidence under `.omp/evidence/p040-be3a5f5-active-handshake-full3-20260829T035957Z`.
- Remove every temporary source probe before source freeze; retained probe evidence remains ignored ephemera.
- Commit and freeze a new exact source/tree and build one fresh optimized qualification executable.
- Run one fresh Windows five-process final-enforcement cohort sequentially, with no retry, exclusion, overlap, reorder, extra priming, or sample replacement.
- Any failed final process remains terminal evidence and stops the cohort; this ADR does not authorize a budget relaxation or another apparatus retry.
