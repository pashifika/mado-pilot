# ADR 0014: macOS Qualified Host and Frame Placement

- **Status:** Accepted
- **Date:** 2026-08-01
- **Resolves gate:** macOS half of [`G-001`](../validation-gates.md#g-001)
- **Supersedes:** _none_

## Context

Phase 2 must name an exact macOS baseline and must not derive a frame's desktop
placement from a shareable-content snapshot acquired after that frame. The current
Apple Silicon development environment reports macOS 26.5.2, build 25F84, with SDK
26.5. Apple documents `SCStreamFrameInfoScreenRect` as the onscreen location of the
captured content in a sample buffer's frame-information dictionary. That key gives
the implementation same-frame placement authority. Screen Recording is not granted
to the Codex test process, so that process can exercise only the refusal path. A
permissioned terminal on the same qualified host ran the live sanitizer and manual
matrix during review; the later owned-window replacement result is recorded below.

The toolchain accepts `-mmacosx-version-min=26.5.2`; an object inspected with
`otool -l` reports `minos 26.5.2` and `sdk 26.5`. This makes the exact qualified host
version expressible in both native and final artifact deployment metadata.

## Decision

The macOS implementation and deployment floor is Apple Silicon macOS 26.5.2
(25F84), built with SDK 26.5. Earlier macOS versions, including Sonoma, are
unqualified and unsupported. Every published native frame carries placement parsed
from that sample buffer's `screenRect`. Discovery constructs and retains an
`SCContentFilter` while the originating inventory is alive; capture open consumes
that filter directly, and no later shareable-content snapshot participates in
capture target selection, capture loss inference, frame publication, or
reconfiguration. Input liveness is the separate bounded comparison recorded in
the 2026-08-09 correction below.
The shim preserves the effective `scaleFactor × contentScale` for current-frame
publication separately from an optional producer-capacity recommendation. It
derives that recommendation from
the same sample's validated `screenRect.size × scaleFactor`, bounds it by the native
axis and byte ceilings, and gives it no geometry authority. Window surfaces grow only
when the recommendation exceeds capacity and otherwise retain their high-water size.

## Alternatives

- macOS 12.3 was rejected because it supplies the capture framework but cannot
  satisfy this change's same-frame placement contract across the declared range.
- Sonoma 14 was considered because `screenRect` is available from macOS 13.1, but
  no Sonoma host is part of the current qualification environment. Supporting it
  would turn an unrun compatibility assumption into a product promise.
- Target-extent-only publication was rejected because the current deployment
  baseline supplies same-frame placement and the public coordinate contract can
  preserve target-logical and desktop-logical conversion without snapshot races.

## Consequences

- Integrators must deploy the macOS artifact only to Apple Silicon macOS 26.5.2 or
  newer; older-version investigation does not imply support.
- `.cargo/config.toml` owns final Rust artifact deployment metadata, and the native
  shim repeats the same floor explicitly.
- ScreenCaptureKit remains controlled-loaded from its absolute system path. Missing
  required classes or frame keys is a typed unsupported outcome, not ambient lookup.
- Missing, non-finite, or extent/scale-inconsistent `screenRect` metadata is dropped
  observably and never published.
- A window recommendation accepts only the SDK's finite raw scale-factor range
  `[1, 4]` and the existing per-axis and 256 MiB surface bounds. An over-limit hint is
  omitted and capture remains self-consistent at its reduced extent rather than
  allocating past the ceiling.
- Reconfiguration stays prospective and off the producer callback: it can change a
  future surface, but cannot relabel the frame that supplied the hint. An oversized
  window surface is retained even when a window recommendation is missing or invalid;
  display capture keeps the existing same-frame content-extent path.
- Every discovery snapshot mints fresh public target identities. The current and
  previous generations remain openable; older unopened identities expire. An opened
  capture session owns its retained filter independently.
- Capture target loss is accepted only from explicit ScreenCaptureKit
  stream/no-source outcomes. Input separately re-enumerates current shareable
  content before an irreversible post and requires logical `SCWindow` equality
  with the retained discovery object; wrapper-address churn is not loss, while a
  missing or unequal current object is.
- Lowering the floor later requires a new ADR and qualification on the proposed
  oldest host.
- The Windows half of `G-001` remains unresolved.

## Verification

- `crates/platform/macos/tests/linkage.rs` asserts that the final test artifact
  reports `minos 26.5.2` and has no eager ScreenCaptureKit load command.
- The shim and Rust mirror have a structure-size contract test for frame metadata.
- Deterministic geometry tests reject a full logical-point disagreement between
  the same-frame screen rectangle, extent, and effective scale.
- A deterministic staging-slot test proves that terminal discard releases the
  candidate and makes a later commit a no-op; the contained commit trampoline also
  has its own panic regression.
- On 2026-08-01 the permissioned ASan suite passed all 95 library tests on the
  qualified host with live streams producing and no sanitizer finding. It compared
  display frames' `screenRect` origin, logical size, and scale with the host inventory,
  exercised the signed-origin window case, and proved that a fresh discovery does not
  terminate an open retained filter. The repaired after-callback case then passed ten
  consecutive runs against the same instrumented binary; that repetition is a
  stability sample for one binary, not cross-host evidence.
- Two pre-fix manual movement runs published roughly 5,800 states after the window
  had moved fully onto a 2x display, while the open stream remained at effective
  scale 1 and 1718x1108; fresh discovery after close alone saw 3436x2216 at scale 2.
  The SDK contract and code path showed that multiplying raw `scaleFactor` by
  `contentScale` discarded the source-resolution signal needed for future producer
  capacity. The probe did not print those two raw values separately, so their exact
  2x and 0.5 values remain an inference rather than direct log evidence.
- Deterministic tests cover the repaired same-sample recommended-extent algebra,
  C/Rust ABI bounds, non-shrinking capacity, and the one discontinuity after a 2x
  surface takes effect. The hardened permissioned probe then passed 2/2 while
  publishing 4,097 frames over 75 seconds without a stall: 3,371 same-scale moves
  preserved their epoch and 30 cross-scale moves advanced exactly from epoch 0
  through epoch 30. It published both 1718x1108 at scale 1 and 3436x2216 at scale 2,
  and its final frame agreed with the post-close placement reading. This closes the
  cross-scale movement acceptance item on the qualified host.
- A fresh post-repair ASan build then passed all 101 library tests with the live
  capture scenarios running and no sanitizer finding. The manual cross-scale probe
  used the ordinary debug build, so these are complementary samples rather than one
  sanitizer-instrumented movement run.
- On 2026-08-09 the
  [owned-window replacement probe](../evidence/g-001/macos-owned-window-replacement.md)
  was rerun with the complete one-display matrix on the same qualified host at
  commit `a1faf04505c8471deb4de8c136fddcc7f76105e7`. The fixture destroyed its
  selected window and created a same-process, same-title successor with distinct
  pixels. During a bounded ten-second observation the retained filter never
  published successor content; a fresh session captured the successor, and the
  retained original mapping stayed unchanged. ScreenCaptureKit emitted no
  explicit loss event, so the Adapter correctly left frame requests quiescent
  rather than inventing `TargetLost`. This closes the replacement acceptance
  item without weakening the explicit-loss rule in the decision.

### 2026-08-09 input-liveness correction

The owned-window replacement review disproved one premise without changing the
capture decision above. On the qualified host, the `SCWindow` retained through
`SCContentFilter.includedWindows` continued to report `isOnScreen = true` and its
old frame after the fixture destroyed that window. A fresh
`SCShareableContent` snapshot omitted the old object and reported only the
same-process, same-title successor. Two fresh wrappers for the still-live original
had different addresses but returned true from `isEqual:`.

Consequently, input no longer treats retained `SCWindow` properties as live state.
It performs a bounded current-content query, narrows by the recorded PID and window
number, and requires logical equality with the retained object before using the
current frame. The existing replacement regression then returned `TargetLost`
before any input and passed with:

```text
cargo test --locked --package mado-pilot-platform-macos --test native_input \
  owned_window_replacement_never_retargets_the_retained_filter -- \
  --ignored --exact --nocapture --test-threads=1
```
