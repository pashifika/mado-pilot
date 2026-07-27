# G-014 evidence: archive safety ceilings

This directory holds the measurements behind gate
[`G-014`](../../validation-gates.md#g-014) and the decision recorded in
[ADR 0001](../../adr/0001-asset-archive-container-and-safety-ceilings.md).

The gate asks for archive entry-count, uncompressed-byte, and compression-ratio
ceilings, and for adversarial fixtures that are rejected deterministically at
them. Ceilings for manifest bytes, per-entry bytes, and total compressed bytes
were added during measurement, because the three the gate named do not on their
own bound what a loader allocates.

| File | Contents |
|---|---|
| [probe.md](probe.md) | What was measured, how, and how to reproduce it without the probe |
| `report-aarch64-apple-darwin.json` | Raw report from the Apple Silicon run |
| `report-x86_64-pc-windows-msvc.json` | Raw report from the Windows 11 x64 run |

The adversarial fixtures the reports refer to are tracked in
[fixtures/assets/g-014](../../../fixtures/assets/g-014/) and pinned by
`SHA256SUMS`. Each report records the SHA-256 of every archive it measured, so a
report and a fixture set that no longer agree are visibly stale rather than
quietly wrong.

Some measured archives are not tracked: the representative packages above the
`tiny` size, and the 4,097-entry archive whose declared count is truthful. They
are large, their construction is fully described by their parameters, and their
hashes are in the reports. That fixture README records which and why.

## Hosts

| Field | Apple Silicon | Windows |
|---|---|---|
| Release target | `aarch64-apple-darwin` | `x86_64-pc-windows-msvc` |
| CPU | Apple M1 Pro, 10 logical cores | Intel Core i7-12700KF, 20 logical cores |
| Memory | 32 GiB | not recorded |
| Operating system | macOS 26.5.2 (build 25F84) | Windows 11 Pro 25H2 |
| Toolchain | rustc 1.97.1 (8bab26f4f 2026-07-14) | rustc 1.97.1 |
| Build profile | `release`, default settings | `release`, default settings |
| Probe dependencies | `zip` 8.6.0 (`deflate-flate2` only), `flate2` 1.1.9, `sha2` 0.11.0, `serde_json` 1.0.151, `png` 0.18.1 | identical, pinned by the probe lockfile and built with `--locked` |

Timings are the median of nine samples after two warm-up runs. Memory is peak
bytes requested from the global allocator during the measured section, not
resident-set size; [probe.md](probe.md) explains why.

## Representative workloads

Phase 1 asset packages hold window-automation templates: mostly small control
and icon sprites, a minority of panel-sized regions, and occasionally a
full-screen scene reference. The profiles below describe that workload. They are
not ceilings, and no profile was chosen to make a ceiling look comfortable.

| Profile | Templates | Entries | Manifest | Largest entry | Uncompressed | Archive | Ratio |
|---|---|---|---|---|---|---|---|
| `tiny` | 6 | 7 | 2,630 | 2,630 | 9,521 | 8,449 | 1.12 |
| `typical` | 64 | 65 | 25,649 | 64,744 | 647,615 | 633,813 | 1.02 |
| `large` | 512 | 513 | 203,375 | 2,337,729 | 8,442,823 | 8,329,745 | 1.01 |
| `ceiling` | 4,095 | 4,096 | 1,617,788 | 1,617,788 | 3,602,330 | 2,691,030 | 1.33 |

`large` includes 1920×1080 and 3840×2160 scene references; its largest entry,
2,337,729 bytes, is the 4K one. `ceiling` is not a realistic package: it exists
to measure the largest package the entry-count ceiling admits, so the peak cost
at the ceiling is measured rather than extrapolated.

Full validation — open, sweep, parse the manifest, expand and SHA-256 every
referenced entry, commit:

| Profile | Apple Silicon time | Apple Silicon peak | Windows time | Windows peak |
|---|---|---|---|---|
| `tiny` | 38 µs | 123,014 B | 74 µs | 123,022 B |
| `typical` | 580 µs | 180,955 B | 595 µs | 180,963 B |
| `large` | 6,694 µs | 973,193 B | 6,258 µs | 973,201 B |
| `ceiling` | 13,495 µs | 7,286,894 B | 17,796 µs | 7,286,902 B |

`large` gives the throughput the total-byte ceiling has to be affordable
against: 8,442,823 bytes expanded and hashed in 6,694 µs on Apple Silicon and
6,258 µs on Windows, or about 1.26 and 1.35 GB/s.

Peak allocation differs between the targets by a constant eight bytes, not by a
factor, and only where the peak includes a heap allocation whose size the two
platforms round differently. Where the peak is one fixed allocation — the 259
bytes the entry-count pre-parse reaches — the two targets agree exactly.

## Worst-case legitimate expansion ratio

A template package barely compresses, because PNG content is already
deflate-compressed. The most compressible thing a package can legitimately
contain is its own JSON manifest, so a manifest-only package is the real upper
bound on the aggregate ratio.

| Case | Uncompressed | Archive | Ratio |
|---|---|---|---|
| `tiny` stored / deflated | 9,521 | 8,449 / 8,479 | 1.12 / 1.12 |
| `typical` stored / deflated | 647,615 | 633,813 / 634,153 | 1.02 / 1.02 |
| `large` stored / deflated | 8,442,823 | 8,329,745 / 8,331,904 | 1.01 / 1.01 |
| `ceiling` stored / deflated | 3,602,330 | 2,691,030 / 2,711,505 | 1.33 / 1.32 |
| `tiny` manifest only | 2,630 | 814 | 3.23 |
| `typical` manifest only | 25,649 | 3,805 | 6.74 |
| `large` manifest only | 203,375 | 25,935 | 7.84 |
| `ceiling` manifest only | 1,617,788 | 198,708 | 8.14 |

The highest ratio any legitimate case reached is **8.14**.

## Central directory cost by entry count

Opening a ZIP central directory allocates in proportion to the entry count, so
an entry-count ceiling checked after the open is checked too late. Reading the
count out of the fixed-layout trailer first costs a constant 144 bytes.

Apple Silicon:

| Entries | Archive | Pre-parse time | Pre-parse peak | Open time | Open peak | Per entry |
|---|---|---|---|---|---|---|
| 1,024 | 94,230 | <1 µs | 144 B | 204 µs | 556,088 B | 543 B |
| 4,096 | 376,854 | <1 µs | 144 B | 846 µs | 2,224,184 B | 543 B |
| 16,384 | 1,507,350 | <1 µs | 144 B | 3,326 µs | 8,896,568 B | 543 B |
| 60,000 | 5,520,022 | <1 µs | 144 B | 12,924 µs | 32,679,704 B | 544 B |

Windows:

| Entries | Archive | Pre-parse time | Pre-parse peak | Open time | Open peak | Per entry |
|---|---|---|---|---|---|---|
| 1,024 | 94,230 | <1 µs | 144 B | 293 µs | 556,096 B | 543 B |
| 4,096 | 376,854 | <1 µs | 144 B | 1,725 µs | 2,224,192 B | 543 B |
| 16,384 | 1,507,350 | <1 µs | 144 B | 5,736 µs | 8,896,576 B | 543 B |
| 60,000 | 5,520,022 | <1 µs | 144 B | 22,373 µs | 32,679,712 B | 544 B |

Cost is linear at 543 bytes per entry on both targets with no observed
inflection, and the pre-parse is constant at 144 bytes regardless of the count
the archive claims.

A recorded entry count is attacker-controlled and need not match the records
present: `bomb-entry-count-declared.zip` is 1,347 bytes, holds two entries, and
claims 60,000. The pre-parse rejects it at 259 bytes of allocation on both
targets.

## Ceilings

| Ceiling | Value | Largest legitimate measurement | Headroom |
|---|---|---|---|
| `max_manifest_bytes` | 4 MiB (4,194,304) | 1,617,788 (`ceiling`) | 2.59× |
| `max_entry_count` | 4,096 | 513 (`large`) | 7.99× |
| `max_entry_uncompressed_bytes` | 64 MiB (67,108,864) | 2,337,729 (`large`) | 28.7× |
| `max_total_compressed_bytes` | 256 MiB (268,435,456) | 8,329,745 (`large`) | 32.2× |
| `max_total_uncompressed_bytes` | 512 MiB (536,870,912) | 8,442,823 (`large`) | 63.6× |
| `max_compression_ratio` | 64 | 8.14 (`ceiling` manifest only) | 7.9× |

Why each number is what it is:

- **Manifest bytes.** The manifest is the only entry a loader buffers whole, so
  its ceiling is the one that directly bounds a single allocation. A package at
  the entry ceiling produced a 1,617,788-byte manifest, about 395 bytes per
  template. 4 MiB is 2.59× that, which absorbs longer identifiers and richer
  per-template metadata without admitting an allocation worth attacking.
- **Entry count.** 4,096 entries cost 2,224,184 bytes of central directory,
  which is a bounded and unremarkable allocation, and 4,096 is eight times the
  largest representative package. Sixteen thousand entries would still work but
  buys nothing: no template-matching workload needs it.
- **Per-entry uncompressed bytes.** 64 MiB clears the largest measured entry by
  28.7×, and it also clears an uncompressed 3840×2160 RGBA surface
  (31,850,496 bytes) by 2.1×, so the ceiling is not accidentally tied to PNG
  staying the only supported encoding.
- **Total compressed bytes.** This bounds what a caller must hold or stream
  before anything is parsed. 256 MiB is 32× the largest representative package
  and is checked as a length comparison, so crossing it costs nothing.
- **Total uncompressed bytes.** At the measured 1.26 GB/s for expansion plus
  SHA-256, 512 MiB is about 0.43 s of work — long, but it is a bound on a
  deliberate load, and the operation deadline governs the rest.
- **Compression ratio.** 64 is 7.9× the highest ratio any legitimate case
  reached. The adversarial ratio bomb declares 957.

### What the ceilings bound together

The largest package the ceilings admit was measured, not assumed: `ceiling`
allocated 7,286,894 bytes at peak. Its manifest is 1,617,788 bytes against a
4 MiB ceiling, so a package that also saturated the manifest ceiling would
allocate more — bounded by the additional raw manifest bytes and their parsed
form, which is reasoning rather than a measurement and is recorded as such.

What matters is that the bound does not depend on what the archive declares
beyond these ceilings. Every stage after the manifest streams in 64 KiB chunks,
so total uncompressed bytes affect elapsed time and not peak memory.

## Adversarial outcomes

Every fixture was rejected in the expected category, at the expected stage, on
both hosts. `Guarded` is the bounded pipeline; `unguarded` is the same work with
no ceilings, which is what the ceilings prevent. The numbers below are the Apple
Silicon run; the Windows run agreed on every category, every stage, and every
expanded-byte count exactly, and on peak allocation to within the constant eight
bytes noted above.

| Fixture | Category | Stage | Guarded peak | Guarded expanded | Unguarded peak | Unguarded expanded |
|---|---|---|---|---|---|---|
| `path-absolute-posix` | `unsafe_path` | entry metadata | 1,361 B | 0 | 117,999 B | 1,391 |
| `path-absolute-drive` | `unsafe_path` | entry metadata | 1,486 B | 0 | 118,077 B | 1,391 |
| `path-unc-root` | `unsafe_path` | entry metadata | 1,417 B | 0 | 118,041 B | 1,391 |
| `path-traversal` | `unsafe_path` | entry metadata | 1,454 B | 0 | 118,038 B | 1,391 |
| `path-traversal-inner` | `unsafe_path` | entry metadata | 1,495 B | 0 | 118,068 B | 1,391 |
| `path-backslash-separator` | `unsafe_path` | entry metadata | 1,414 B | 0 | 118,026 B | 1,391 |
| `path-embedded-nul` | `unsafe_path` | entry metadata | 1,430 B | 0 | 118,029 B | 1,391 |
| `path-non-utf8` | `unsafe_path` | entry metadata | 1,415 B | 0 | 118,030 B | 1,391 |
| `path-duplicate-normalized` | `duplicate_path` | entry metadata | 1,895 B | 0 | 118,367 B | 2,124 |
| `entry-symlink` | `unsupported_entry_type` | entry metadata | 1,430 B | 0 | 118,026 B | 677 |
| `entry-fifo` | `unsupported_entry_type` | entry metadata | 1,430 B | 0 | 118,026 B | 658 |
| `entry-character-device` | `unsupported_entry_type` | entry metadata | 1,430 B | 0 | 118,026 B | 658 |
| `entry-directory-name-collision` | `unsupported_entry_type` | entry metadata | 1,430 B | 0 | 118,026 B | 658 |
| `bomb-entry-count` (not tracked) | `archive_limit` | directory pre-parse | 259 B | 0 | 2,212,940 B | 658 |
| `bomb-entry-count-declared` | `archive_limit` | directory pre-parse | 259 B | 0 | 1,116 B | 0 |
| `bomb-total-uncompressed-declared` | `archive_limit` | directory open | 5,888 B | 0 | 120,736 B | 667 |
| `bomb-entry-uncompressed-declared` | `archive_limit` | entry metadata | 1,479 B | 0 | 118,026 B | 1,391 |
| `bomb-compression-ratio` | `archive_limit` | entry metadata | 1,463 B | 0 | 118,026 B | 8,389,266 |
| `manifest-oversize` | `archive_limit` | entry metadata | 1,131 B | 0 | 117,694 B | 2 |
| `bomb-understated-declaration` | `declared_size_mismatch` | expansion | 122,034 B | 66,194 | 118,026 B | 8,389,266 |
| `manifest-missing` | `missing_manifest` | manifest | 1,034 B | 0 | 117,688 B | 733 |
| `manifest-malformed` | `malformed_manifest` | manifest | 118,174 B | 34 | 117,694 B | 34 |
| `manifest-unsupported-schema` | `unsupported_schema_version` | manifest | 119,152 B | 660 | 118,026 B | 1,393 |
| `hash-mismatch` | `hash_mismatch` | expansion | 122,022 B | 1,391 | 118,026 B | 1,391 |

Three rows carry the argument:

- `bomb-entry-count` allocates 259 bytes guarded against 2,212,940 unguarded,
  and the scaling table shows that gap growing linearly without limit.
- `bomb-compression-ratio` expands nothing guarded against 8,389,266 bytes
  unguarded, from an 8,765-byte archive.
- `bomb-understated-declaration` declares 1,024 bytes and contains eight
  mebibytes. The guarded pipeline stops after one 64 KiB chunk, having expanded
  66,194 bytes; the unguarded one expands all 8,389,266. This is the case that
  shows why a declared size may be used to reject early but never to authorise
  reading.

The container-byte ceiling was exercised against a synthetic 268,435,457-byte
buffer rather than a tracked fixture, and rejected it at the container stage
before any parse on both targets. An archive large enough to cross that ceiling
cannot reasonably be tracked, and the check is a length comparison.

## Cross-target result

The two runs agree on everything the decision depends on:

- All 24 adversarial fixtures produced the same failure category at the same
  stage on both targets.
- Expanded-byte counts are identical to the byte, including the 66,194 bytes
  `bomb-understated-declaration` reaches before the guarded pipeline cuts it off.
- Aggregate expansion ratios are identical, so the ratio ceiling means the same
  thing on both targets.
- Central-directory cost is 543 bytes per entry on both, and the pre-parse is
  144 bytes on both.
- Peak allocation differs by a constant eight bytes where it differs at all.

Fixture generation is also byte-deterministic across targets: the Windows run
regenerated every file the Apple Silicon run produced identically — 33 files at
the time of the comparison, verified byte by byte rather than by size. A fixture
that is safe on one release target is therefore the same fixture on the other,
which is what lets one tracked set serve both, and what makes the untracked
archives safe to rebuild rather than store.

Where the targets differ is speed, and only by a factor a benchmark budget would
care about rather than a safety ceiling: Windows was slower on the
central-directory sweep (22,373 µs against 12,924 µs at 60,000 entries) and on
the `ceiling` profile (17,796 µs against 13,495 µs), and marginally faster on
`large`. No ceiling in this decision is derived from elapsed time, so none of
that changes a number.

Neither run reached the probe's two-gibibyte unguarded abort, so every unguarded
measurement on both targets is complete.
