# ADR 0076: Bound native OCR evidence channels

- **Status:** Accepted
- **Date:** 2026-09-14
- **Resolves gate:** _none_; task 7.4 preflight apparatus repair only
- **Supersedes:** _none_; extends ADR 0074's pipe mechanism, not its replay topology

## Context

At preflight source `4a9dc5b` / tree `771e76d7`, the Apple native runner sends
`DYLD_PRINT_LIBRARIES=1` output to regular stderr under
`RLIMIT_FSIZE=65536`. An owned no-op C probe linked to AppKit, QuartzCore and
ScreenCaptureKit emitted 84,029 bytes of loader stderr and reached `main` without
that cap. Under the exact runner `child_limits` and regular-file redirection, it
exited `-25` (`SIGXFSZ`), with stderr at 65,536 bytes, stdout at zero, and no entry
to `main`. No OCR, model, capture or permission API ran.

The Change's private `134-native-loader-unlimited-observation.json` and
`134-native-loader-legacy-cap-reproduction.json` retain this apparatus proof.
It is neither a native OCR failure nor a new qualification result. Increasing
the ordinary-output allowance is not evidence-channel isolation.

## Decision

Apply [ADR 0074](0074-isolate-darwin-loader-image-budget.md)'s anonymous-pipe
mechanism inside `tools/ocr-text-watch/macos/run.py`, without a collector process.
Keep exactly two direct native `Popen` children: consumer first, fixture only
after already-granted non-prompting Screen Recording permission. Each owned PID
is the actual executable PID; neither a wrapper nor a grandchild is introduced.
Keep `RLIMIT_CORE=0` and inherited `RLIMIT_FSIZE` unchanged, never raised.

Give each child separate stdout, stderr and native-image pipes with nonblocking
parent reads. Set `DYLD_PRINT_LIBRARIES=1` and direct loader bytes through an
inherited descriptor with `DYLD_PRINT_TO_FILE=/dev/fd/N`. Write exclusive private
mode-0600 files; only `consumer.native-images` and `fixture.native-images` are
excluded from the ordinary aggregate. Keep `each_output_file_bytes=65536` for
ordinary stdout/stderr and `total_output_bytes=262144` for all ordinary/control/
report output, including `record.json`; existing control/report limits remain.
Add `each_native_image_file_bytes=1048576` and `total_native_image_bytes=2097152`
to `BOUNDS`. System loader bytes count toward these image budgets.

Bound work per loop and drain all channels while running and throughout graceful,
terminate and kill/reap cleanup, including final exit tails. Never wait on a full
pipe. Pump or storage failure cannot skip exact-child reaping or descriptor
closure. Completeness requires EOF and successful bounded collection; saturation,
incomplete observation or malformed data fails. Preserve the first failure even
when cleanup succeeds; a retained prefix cannot become complete evidence.

Parse images only from complete separate image channels with strict text and row
validation, including the final row. Retain dyld's exact owned-PID
`move loaded to delayed: <basename>` state diagnostic without treating it as an
image identity; reject unknown syntax and foreign PIDs. Require the independently
approved canonical
non-system image union and hashes exactly, with prelaunch and post-exit disk
rehashing. Missing, additional, noncanonical or changed images fail admission;
system shared-cache identities remain bound to the approved OS build. Interpret
ordinary consumer errors from `consumer.stderr`, never from the image channel.

## Alternatives

- Raising regular stderr's cap weakens the unchanged 64 KiB ordinary contract
  without separating loader evidence. Raising global `RLIMIT_FSIZE` still couples
  unrelated native regular-file writes to an evidence budget, rejected by ADR 0074.
- Excluding system rows after capture cannot prevent the pre-main signal or
  recover already missing bytes. Collect their raw bytes within the image budget
  before applying the established non-system dependency-map policy.
- Reusing the replay collector would change the direct-child PID and cleanup
  topology. The existing native supervisor can drain these channels itself;
  no shared utility layer, runtime API or Rust loader callback is required.

## Consequences

Record `procedure: "ocr-text-watch-apple-v2"`; retain semantic/resource/cleanup,
row, exit and status fields, with private per-channel observed/retained byte,
EOF/completeness and failure facts. Fresh exact-binding review must cover the
complete v2 `BOUNDS` object and updated procedure hashes before native admission.
The [native procedure](../ocr-text-watch-macos.md) retains its isolated build,
new-artifact signing, host, permission, focus, nonce, source/checkpoint,
retention, resize and target-loss policies, with no native retries.

This changes neither historical ADRs, replay collector/evidence nor frozen
artifacts. It changes no `G-013` ceiling, numeric floor, deployment floor or
support status. Independent artifact/model validation remains required; bounded
evidence collection is not a sandbox for arbitrary native file writes.

## Verification

The model-free OCR procedure suite ran 118 tests: 114 passed and four
platform-specific cases skipped. Its 16 native-supervisor regressions cover
ordinary/image saturation, control/report and final-record accounting,
independent channels, complete exit tails, inherited file-size limits and
disabled core dumps, an unrelated 2 MiB regular-file write, malformed image
records, unapproved/symlink identities, and sticky failures. Injected pump/storage
failures still drain or close endpoints and reap exact owned children; two
SIGTERM-ignoring children exercised the bounded SIGKILL path.

The actual inert framework probe exited zero and reached `main`: 84,029 loader
bytes, 32 ordinary bytes, 646 parsed image identities, complete EOF on every
channel, no failure and one unforced/reaped child. The first repaired parser
correctly refused its unfamiliar dyld state diagnostics; those first failures
remain retained. Only the exact owned-PID delayed-state grammar was then added,
without granting image identity or weakening the ordinary limits.

Private Change records 134, 139, 140 and 141 retain the original pre-main
failure, first parser failure, repaired probe, and complete regression output.
Records 135–138 and the independent prepared-input review bind the new isolated
build/signing, written-export reconstruction, macOS 26.6.2 host, SDK 27.0,
deployment minimum 26.5.2 and prospective image set. Historical ADR 0074,
architecture benchmark history and old qualified binaries remain byte-identical.

Native admission still requires independent final committed-source, supervisor,
host, artifact and authority binding review. It permits only one authorized
owned procedure after the consumer observes already-granted Screen Recording
permission. Every native oracle/resource/cleanup verdict remains separate; no
permission request, settings change, input/focus action, foreign campaign reuse
or automatic native retry is allowed. This ADR accepts the apparatus repair,
not an OCR or ScreenCaptureKit qualification result.
