# ADR 0009: Phase 1 keeps two normalized coordinate spaces that coincide

- **Status:** Accepted
- **Date:** 2026-07-29
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

`CoordinateSpace` names both `FrameNormalized` and `TargetNormalized`, and the C
ABI advertises the second as `MADOPILOT_SPACE_TARGET_NORMALIZED` in the
`coordinate_spaces` bit set a target and a session report. Phase 1's placement
model asserts that a frame's capture pixels cover **exactly** its target's
rectangle and deliberately models no sub-region offset
(`crates/automation/core/src/transform.rs`, `TargetPlacement`). Under that
assertion the two normalized spaces address the same point, and both projections
correctly scale by the frame extent.

What did not follow from the assertion was the datum beside it.
`TransformSnapshot` stored an `Option<PixelExtent>` target extent that no
conversion read, and `with_target_extent` was an infallible `const fn` that
compared it to nothing. `StreamState` pinned one target extent for a session's
whole life while `publish` explicitly supports a mid-stream extent change, and
the replay provider supplied `ReplaySource::extent()`, which is the first frame's
extent. So the first replay resize — a case the delta spec requires — produced a
snapshot whose declared target extent contradicted its frame extent, and the
conversion returned frame-normalized numbers under a target-normalized label with
no fault. That is precisely the fallback
`rasen/changes/phase-1-deterministic-vertical-slice/specs/replay-capture-and-mapping/spec.md:160`
prohibits: an unsupported conversion "MUST NOT use host DPI, host display
placement, an identity transform, or another coordinate space as fallback."

The same assertion had no enforcement on the other side either.
`TargetPlacement::logical_size` was validated and exposed but read by no
conversion, and `with_target` accepted any placement for any frame extent. The
replay adapter builds placements from a manifest it did not author, so an
inconsistent manifest displaced every desktop-logical coordinate silently.

## Decision

**A snapshot records that its frame covers its target, and never a second
extent.** `TransformSnapshot::with_target_extent(geometry, frame_extent)` takes no
target extent — matching the shape `CoordinateSupport::with_target_extent()`
already had — and `StreamState::with_target_extent(stream)` follows.
`TransformSnapshot::covers_target()` replaces `target_extent()`.

**`TransformSnapshot::with_target` is fallible and checks the assertion it
carries**, returning `GeometryFault::SpaceMismatch` when the placement's logical
size scaled by its own factor is not the frame extent, within half a pixel on
each axis. `StreamState::publish` reports that as
`CaptureFault::InconsistentDescriptor`, so a replay source whose manifest
declares an inconsistent placement fails to open instead of publishing a
displaced transform.

Both normalized spaces stay in `CoordinateSpace` and both keep their C ABI
numbers. A later phase that captures a sub-region of a target is what makes them
differ numerically; that phase adds a frame-within-target offset to
`TargetPlacement`, which is additive.

## Alternatives

- **Scale target-normalized coordinates by the declared target extent.** Not
  well defined under Phase 1's own model: the product is a coordinate in the
  *target's* pixels, and converting that to frame pixels needs the
  frame-within-target offset and scale `TargetPlacement` explicitly does not
  model. Implementing the literal one-line version would produce coordinates
  wrong by the extent ratio and land input in the wrong place — worse than the
  mislabel it replaces. It is the right destination for the phase that adds
  sub-region capture, and the wrong shape for this one.
- **Keep the declared extent and fail loudly when it diverges.** Preserves a
  datum for a future sub-region model, but the constructor variant makes every
  resize fail unless the stream re-derives the target extent from the incoming
  publication — which is this decision reached by a longer route, with a fault
  path that can then never fire. The `supports()` variant is worse: a caller that
  converted at frame 5 gets `ConversionUnsupported` at frame 6 for a space the
  session still advertises, because the advertised bit comes from
  `CoordinateSupport` and not from the snapshot.
- **Leave it and document the magnitude as advisory.** A stored value that no
  conversion consumes can only be redundant or false, and this one is reported
  through a public accessor. Documenting that a reported number may contradict
  the frame it was read from is not a contract.

## Consequences

- **Integrators.** Rust callers that constructed a `TransformSnapshot`
  themselves must drop the target-extent argument, handle `with_target`'s
  `Result`, and read `covers_target()` instead of `target_extent()`. This is
  source-breaking inside `0.x`, which
  [ADR 0006](0006-public-rust-names-and-compatibility-policy.md) permits with an
  ADR; it renames and removes nothing in ADR 0006's reviewed name list, so that
  record stands unsuperseded.
- **ABI.** Compatible. No C enum value, structure field, or function-table entry
  changes, and `MADOPILOT_SPACE_TARGET_NORMALIZED` keeps its number and its bit.
  What changes is that the bit is now true as written: a session advertises
  target-normalized support only for frames that cover their target, and no
  conversion behind it can be silently displaced.
- **Harder later.** A frame that covers only part of its target can no longer be
  described by declaring a different extent — which never worked, but now says
  so. That phase adds an offset and a frame-within-target scale to
  `TargetPlacement`, splits the shared projection arm, and revisits this record.
- **A placement is now refused rather than trusted.** A source whose manifest
  declares a placement inconsistent with its frames fails at open. Half a pixel
  of tolerance on each axis is deliberate: an integral capture extent is the
  rounding of a logical size a host is free to report fractionally, so exact
  equality would refuse placements that are as consistent as a host can make
  them.
- **Landing with it.** The `CaptureSession` contract gained `lifecycle()`, with
  `is_open()` and `is_closed()` derived from it, so that "refuse work on a closed
  session" cannot be written against a predicate that is false during
  `Lifecycle::Closing`. That is additive for callers under ADR 0006 and
  source-breaking for an out-of-tree implementor, of which there are none.
- **Documentation.** The `transform.rs` module documentation, `docs/architecture.md`'s
  core-contracts section, and `docs/c-abi.md`'s coordinate rule state that the two
  normalized spaces coincide in Phase 1 and why a later phase separates them.

## Verification

- `crates/automation/core/src/transform.rs`:
  `target_normalized_requires_target_coverage_but_not_placement`,
  `target_normalized_and_frame_normalized_address_the_same_point`,
  `a_placement_that_does_not_scale_to_the_frame_extent_is_refused` (both axes and
  a non-integral scale), and
  `a_placement_a_host_can_only_report_fractionally_is_accepted`.
- `crates/automation/capture/src/stream.rs`:
  `target_normalized_tracks_the_frame_across_a_mid_stream_resize` — the case that
  made the removed datum wrong — and
  `a_placement_that_does_not_scale_to_the_published_extent_is_refused`.
- `crates/adapter/replay/src/provider.rs`:
  `a_source_whose_placement_does_not_cover_its_frame_fails_to_open` and
  `a_source_whose_placement_covers_its_frame_opens`.
- The C ABI layout probe and the frozen-header compatibility suite
  (`crates/bindings/capi/tests/layout.rs`, `crates/bindings/capi/tests/abi-compat/`)
  are the check that this decision cost the ABI nothing; they are unchanged by it.
