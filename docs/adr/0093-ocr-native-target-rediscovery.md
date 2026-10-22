# ADR 0093: Separate native target disappearance from rediscovery

- **Status:** Accepted
- **Date:** 2026-10-22
- **Resolves gate:** _none_; finite native evidence is scoped to the independently recorded Apple and Windows cohorts, not general release acceptance
- **Supersedes:** [ADR 0076](0076-bound-native-ocr-evidence-channels.md)'s private child-count and role-list restriction only, for Apple OCR procedure v3

## Context

The [owned-window replacement evidence](../evidence/g-001/macos-owned-window-replacement.md)
records ScreenCaptureKit quiescence after exact-window destruction, no successor
content in the old session, successful fresh-session capture and unchanged
retained pixels. It does not promise immediate native `TargetLost` for every
window close. The [architecture baseline](../architecture.md#macos-native-capture-ownership)
already preserves that distinction and the SDK's typed source-loss, user/system
stop and permission outcomes. Windows WGC retains its explicit closed/target-loss
contract; that signal alone does not identify a process exit.

The current Change's source-bound `5ddc7806` Apple attempt retained a failed
`exact-window-loss` row with `DeadlineExceeded`, while acknowledged transition,
retained-producer progress, resize reset/invalidity and parent-close retention
passed. Resource/cleanup passed and both owned children were reaped without
force. Its separate owned AppKit lifetime probe found the window absent only
after both window and view were released. These observations do not prove the
new window/process classification or rediscovery scenarios and do not regrade
the failed row. ADR 0076's one-consumer/one-fixture topology cannot exercise a
successor process generation.

## Decision

Keep actual SDK/query termination separate from independently established exact
window or process disappearance, with explicit caller cancel/close before fresh
discovery/open through existing APIs, preserving Windows native loss and
ScreenCaptureKit typed stops without inventing immediate loss, rebinding an old
session, or adding a public API, background poller, generic restart engine,
automatic recovery or platform fallback.

Permit private `ocr-text-watch-apple-v3` to use one consumer and at most two
sequential owned fixture generations, with at most two native children alive
simultaneously as direct `Popen` children of the same bounded supervisor,
superseding only ADR 0076's exact child-count and role-list restriction while
retaining its other evidence, safety and authority constraints.

## Alternatives

- Require `TargetLost` immediately after every macOS window close: imposes an
  undocumented signal and contradicts the retained replacement evidence.
- Treat SDK stop, no frames, a caller deadline or a failed discovery as loss:
  confuses bounded observation with cause and can misclassify permission or
  backend failures as disappearance.
- Rebind or restart inside capture callbacks or OCR workers: changes session and
  query authority, risks admitting successor pixels to an old query, and adds
  automatic recovery outside the approved scope.
- Reuse one fixture process or add a collector: the former cannot prove actual
  process-generation replacement; the latter changes direct-child ownership
  without supplying a required observation.

## Consequences

The public caller sequence is explicit old-session cleanup, fresh discovery,
fresh open and a new query. Every replacement needs distinct target/stream
provenance and a real acknowledged frame/OCR result. Existing terminal outcomes
remain immutable; retained original OCR/frame/mapping bytes remain readable and
unchanged after all parent closes. Logical close is not physical OCR quiescence.

The private qualification must establish causes independently:

- Window loss requires controlled exact-window destruction, successful fresh
  SDK title inventory and independent WindowServer observation showing the
  original window/owner absent, plus affirmative original process-generation
  liveness through owned-child/nonce authority.
- Process loss requires supervisor reaping of the exact owned generation while
  its session exists, not a fixture exit acknowledgement or PID lookup. Normal
  owned termination is not crash or forced-kill qualification.
- Unknown liveness, hidden/minimized windows, absent frames and permission or
  discovery failures cannot prove disappearance. Record actual SDK/query
  outcomes and caller observation deadlines separately; unexpected terminal or
  backend errors remain failures, not substitutes for loss or fresh OCR.

SDK native-window and process provenance are optional metadata, not liveness
signals. The private consumer independently observes only the acknowledged
WindowServer ID, including offscreen windows, and checks its owner against the
supervisor-owned process generation. Fresh admission also requires the unique
nonce title; any supplied SDK identity must agree, and the new stream must
subsequently prove the exact nonce/state pixels. A missing or failed native
observation cannot authorize capture or certify disappearance. This adds no
public SDK API or automatic recovery path.

Launch a successor only after the exact predecessor is reaped and the consumer
requests the predefined phase after old-session close. Use separate private
control directories and generation/nonce/owner-bound receipts; stale receipts,
premature or duplicate launches and unowned transitions fail. No child spawns
a collector or grandchild, and a reused numeric PID grants no continuity.
Fresh authority binds the v3 procedure, role/file roster, complete bounds and
source/artifact identities; no v2 grant authorizes the additional generation.

Preserve the [native procedure](../ocr-text-watch-macos.md)'s other bounds:

- Consumer wall clock200s, construction45s, permission/launch65s, readiness10s,
  control/checkpoint/mapping/close/physical drain5s, semantic observation15s
  maximum, per-query lifetime100s, each fixture self-fuse210s, and exit5s grace
  followed by2s termination and2s kill/reap; no close retry or automatic native
  retry, zero warmups and one sample.
  The v3 loss observation is10s within the existing15s semantic maximum.
- Ordinary output64KiB/file and256KiB aggregate, including all generations,
  control/report files and the final record; native images1MiB/file and2MiB
  aggregate across the consumer and both fixture generations. Only
  `consumer.native-images`, `fixture-1.native-images` and
  `fixture-2.native-images` are excluded from ordinary output accounting.
- Separate bounded stdout/stderr/image pipes, complete EOF and final exit tails,
  strict owned-PID parsing, exact preapproved canonical non-system image union
  and hashes, prelaunch/post-exit rehashing, and approved OS-bound system images.
- `RLIMIT_CORE=0`, unchanged inherited file-size limits, private file modes and
  control/report limits, and bounded exact-child reaping/draining even after
  semantic, pump, storage or interruption failure. Cleanup cannot erase failure.
- Already-granted non-prompting Screen Recording, no Accessibility request,
  input, focus forcing, elevation, settings change, download or permission change;
  ordinary output contains no pixels or unrelated window titles.

The Change proposal/design/specification, Slice A9 and current architecture
lifecycle paragraph adopt this contract; the private guide and apparatus own
its generation protocol. Historical ADR 0076, failures, binaries, measurements,
thresholds, grants and verdicts remain unchanged. This changes no dependency,
packaging, deployment floor, G-013 ceiling or platform support claim.

## Verification

The source-bound `86a482d074e691c74aba8280c0ad42de31fead7a` v3 execution passed
all nine semantic rows on Apple M1 Pro, macOS27.0.1/build26A434, Retina2x and CPU
bounded-v2: acknowledged transition, retained producer progress, resize reset
and invalidity, window-loss classification, same-process rediscovery,
process-loss classification, new-process rediscovery and parent-close retention.
Separate resource/cleanup endpoints passed; all three direct children exited0
and were reaped without force. Both observed SDK signals were `TargetLost`;
the independently proven causes and explicit old-session closes remain separate.
The complete bound record is `macos-v3-window-server-native-20261022.json` in
the Change evidence. These are new v3 observations, not renamed historical rows.

Existing contract suites must cover forged/stale generation receipts,
premature/duplicate successor launch, unowned transitions, immutable terminal
state, unchanged resource/channel bounds and exact-child cleanup. Actual harmless
supervised subprocesses are required where the existing supervisor suite uses
them; source-text checks alone do not prove process ownership or reaping.

Native acceptance requires a freshly authorized source/artifact-bound run with
real acknowledged capture/OCR, independent window/process evidence, physical OCR
quiescence, retained-byte equality and complete bounded cleanup. Source review
keeps Windows explicit-loss expectations separate from the macOS classification
path; no production recovery path is added. Task7.4 is complete for the recorded
Apple cohort. Separate Windows task7.2 at `fd98dba` passed its five existing
semantic endpoints and resource/cleanup on the instrumented DELL-WCG cohort.
Together they satisfy scoped A9 evidence, not feature-off Windows reliability,
crash behavior, broad compatibility or product release.
