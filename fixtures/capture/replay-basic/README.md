# Basic replay capture fixture

A minimal replay source that exercises the frame-identity rules the capture
contract depends on. Consumed by the replay adapter's contract tests, and pinned
by `SHA256SUMS`.

## Provenance and licensing

Generated for this repository. Pixel bytes are a closed-form function of the
pixel coordinate and a per-frame seed, so the fixture is reproducible from its
description alone and carries no third-party material. It is covered by the
repository's Apache-2.0 license.

Frames are raw `rgba8` bytes with packed rows rather than encoded images, which
is the replay source format decided in
[ADR 0002](../../../docs/adr/0002-replay-capture-adapter-package.md). Raw bytes
keep a decoder out of the path between the fixture and the test oracle, at the
cost of size — which is why this fixture is measured in hundreds of bytes rather
than megabytes.

## What each frame is for

The `panel` target's three frames are chosen so that each one proves a different
identity rule:

| Frame | Extent | Continuity | Proves |
|---|---|---|---|
| `0000-8x6.bin` | 8×6 | continuous | The first frame is epoch 0, sequence 0 |
| `0001-8x6.bin` | 8×6 | continuous | **Byte-identical to frame 0.** A repeated frame is still a distinct observation and takes the next sequence |
| `0002-12x6.bin` | 12×6 | discontinuous | An extent change starts a later epoch at sequence 0 and advances the geometry revision |

Frame 1 being identical to frame 0 is the point of it. An implementation that
deduplicated frames, or that compared pixels to decide whether to publish, would
pass every other assertion here and fail this one.

The `placed` target declares a target placement — origin `(100, 50)` in
desktop-logical units, a 2×2 logical size, and a scale of 2 capture pixels per
logical unit — so target-normalized, target-logical, and desktop-logical
conversions are supported for it and refused for `panel`. That pair is what
makes "unsupported conversions are refused rather than guessed" testable.

## Manifest

`madopilot-replay.json` is schema version 1. Each frame declares its pixel file,
extent, format, timestamp in nanoseconds, and continuity, and optionally a target
placement. A stride may be declared for a padded source; these frames are packed,
so they omit it.

Pixel paths are relative and validated: an absolute path, a drive or UNC prefix,
a parent traversal, a backslash, or a symbolic link is refused. A replay manifest
is caller-supplied data, and one that could name any path would be a file-read
primitive wearing a capture adapter's clothes.
