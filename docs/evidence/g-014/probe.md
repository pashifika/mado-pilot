# G-014 probe

The probe is the disposable program that produced the measurements in
[README.md](README.md). It is not product code and does not survive the change
that resolves the gate: what survives is the tracked fixtures, the reports, this
description, and the decision recorded in
[ADR 0001](../../adr/0001-asset-archive-container-and-safety-ceilings.md).

This document exists so that the measurements can be reproduced without the
program. It states what the probe did precisely enough to rewrite, which is the
same standard the asset-loading implementation is held to.

## What it measures

Three things, on each release target:

1. What a bounded loader costs on representative packages. If the guards were
   expensive, they would be argued away later.
2. Where a bounded loader stops on adversarial packages, and how much it
   allocated and expanded before stopping.
3. What an unbounded loader does with the same adversarial packages. A ceiling
   is only justified by what it prevents.

Time is the median of nine samples after two discarded warm-up runs. Memory is
bytes requested from the global allocator, tracked by a wrapper around the
system allocator and sampled as the peak live total during the measured section.
Resident-set size was rejected as the memory measure because Windows and macOS
account it differently and it includes allocator retention the loader does not
control, which would make the two targets incomparable.

## The pipeline under test

The probe implements the loading pipeline in the order the implementation is
expected to use, and every ceiling is enforced before the allocation or
expansion it bounds.

| Stage | Name | Checks |
|---|---|---|
| A | container bytes | Total source bytes against `max_total_compressed_bytes`, before anything is parsed. |
| B | directory pre-parse | The entry count recorded in the end-of-central-directory record, against `max_entry_count`, read without opening the central directory. |
| C | directory open | Central directory materialized; recorded entry count must match stage B; declared total uncompressed bytes against `max_total_uncompressed_bytes`. |
| D | entry metadata | Per entry, with no expansion: compression method, encryption, name normalization, entry type, duplicate normalized name, declared size against the per-entry or manifest ceiling, running declared total. Then the aggregate declared expansion ratio against `max_compression_ratio`. |
| E | manifest | Manifest entry present, read with a hard byte cap, parsed strictly, schema version supported. |
| F | expansion | Referenced entries streamed in 64 KiB chunks, observed bytes checked against the declared size on every chunk, SHA-256 compared against the declared hash, observed ratio re-checked. |
| G | commit | One immutable package. |

Two ordering choices in that table are deliberate.

Stage B exists because opening a central directory allocates in proportion to
the entry count, so an entry-count ceiling checked after the open is checked too
late. The recorded count is read straight out of the fixed-layout trailer,
following the Zip64 locator when the 16-bit field is saturated.

The aggregate ratio is checked at the end of stage D rather than at the start.
Both positions are before any expansion, so safety is identical, but a package
that crosses one absolute ceiling is far more likely to be a misconfiguration
than an attack, and naming that ceiling is the more actionable diagnostic.

Stage F treats the declared size as an upper bound to stop at, never as an
authorisation. An entry that declares 1,024 bytes is cut off after 1,024 bytes
even when the ceiling would have allowed sixty-four mebibytes, which is what
makes an understated declaration cheap to reject.

### Name normalization

An entry name is rejected when it is not valid UTF-8, is empty, contains a NUL
or a backslash, begins with `/`, carries a drive prefix (an ASCII letter
followed by `:`), ends with `/`, or contains a `..` segment. Otherwise `.` and
empty segments are dropped and the remaining segments are joined with `/`.

Collapsing `.` and empty segments rather than rejecting them is what makes a
duplicate detectable: `templates/button.png` and `./templates//button.png` are
the same package path spelled two ways, and the second must be reported as a
duplicate rather than as a malformed name.

### Entry type

Where an archive records Unix mode bits, the file-type field must be zero
(unset) or regular. Symbolic links, hard links, FIFOs, sockets, and device nodes
are rejected without being followed, opened, or materialized.

## The unguarded comparison

The same archives are run through a loader with no ceilings: open the central
directory, then read every entry to exhaustion. A probe-local abort stops it at
two gibibytes of expansion so a bomb cannot exhaust the measuring host, and the
report marks any run that hit the abort, so a truncated measurement is never
read as a completed one. No adversarial fixture in the tracked set reached it.

## Fixture generation

Archives are written by a minimal hand-rolled ZIP writer rather than by the
reader's own crate, so a fixture cannot be wrong in the same way the reader is,
and so the adversarial cases can record entry names, Unix modes, declared sizes,
and entry counts that a conforming writer refuses to produce.

Template images are synthetic PNGs from a fixed seed. Generation is
byte-deterministic: two runs on the same host produced identical files, verified
by comparison against `fixtures/assets/g-014/SHA256SUMS`.

The representative profiles are described in
[../../../fixtures/assets/g-014/README.md](../../../fixtures/assets/g-014/README.md).

## Reproducing a run

The probe is gone from the tree. Reproducing a measurement means implementing
the pipeline above against the tracked fixtures, which is why it is described
here rather than pointed at.

Both runs recorded in this directory were produced the same way: build in
`release` against the probe's own pinned lockfile, then run it with an `--out`
directory and a `--label` naming the target, CPU, operating-system version, and
compiler. The program wrote `report.json` and regenerated the full fixture set
beside it, and the regenerated files were compared byte by byte against the
tracked ones. That comparison is what makes the reports evidence about *these*
fixtures rather than about whatever the probe happened to build that day.

The probe's source is not kept anywhere. That is the cost of this arrangement,
and it is why the description above is written as a specification rather than as
a summary: a reader who disagrees with a number has to be able to rebuild the
thing that produced it. If a future measurement contradicts one recorded here,
the contradiction is resolved by re-measuring, not by consulting the program.
