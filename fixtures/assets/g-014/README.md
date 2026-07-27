# G-014 asset package fixtures

These fixtures are the adversarial and representative evidence behind gate
[`G-014`](../../../docs/validation-gates.md#g-014). They are consumed by the
archive-safety measurements recorded in
[docs/evidence/g-014](../../../docs/evidence/g-014/), and the asset-loading
conformance tests added by the change that implements archive loading must keep
producing the outcomes listed below.

`SHA256SUMS` pins every file. A fixture change that is not accompanied by a
re-measured evidence record invalidates the gate resolution, so the checksum
file is what makes a silent edit visible.

## Provenance and licensing

Every byte here was generated for this repository by the disposable probe
described in
[docs/evidence/g-014/probe.md](../../../docs/evidence/g-014/probe.md). No image,
archive, or manifest is derived from third-party material, so the fixtures carry
the repository's own Apache-2.0 license and impose no attribution obligation.

Template images are synthetic. They are generated from a fixed seed as a panel
gradient with solid control blocks, text-like strokes, and a small share of
dithering noise, because content that compressed like flat colour would make
every measured expansion ratio optimistic.

## Representative packages

`valid/` holds packages that must load successfully. They describe the workload
sizes Phase 1 expects rather than the ceilings that bound them.

| Fixture | Templates | Manifest bytes | Largest entry | Uncompressed | Archive |
|---|---|---|---|---|---|
| `valid/tiny-directory/` | 6 | 2,630 | 2,630 | 9,521 | directory source |
| `valid/valid-tiny.zip` | 6 | 2,630 | 2,630 | 9,521 | 8,449 |

They contain the same manifest and the same template bytes, so a directory
source and an archive source must commit equivalent packages from them. That
equivalence is the property a tracked valid fixture exists to pin, and it does
not get truer at a larger size.

Three further profiles were measured and are deliberately not tracked:
`typical` (64 templates, 647,615 uncompressed bytes), `large` (512 templates
including 1920×1080 and 3840×2160 scene references, 8,442,823 bytes), and
`ceiling` (4,095 templates, the largest package the ceilings admit). Their
measurements are in the evidence reports along with their hashes. Tracking the
bytes as well would add megabytes to every clone to re-assert what the `tiny`
pair already proves; a test or benchmark that needs a package at those sizes
builds one.

## Adversarial packages

`adversarial/` holds packages that must be rejected. Each one crosses exactly one
rule and stays inside every other one, so the outcome identifies which check
stopped it. `Stage` is the earliest point at which rejection is required; a later
rejection means an earlier guard is missing.

| Fixture | Failure category | Stage |
|---|---|---|
| `path-absolute-posix.zip` | `unsafe_path` | entry metadata |
| `path-absolute-drive.zip` | `unsafe_path` | entry metadata |
| `path-unc-root.zip` | `unsafe_path` | entry metadata |
| `path-traversal.zip` | `unsafe_path` | entry metadata |
| `path-traversal-inner.zip` | `unsafe_path` | entry metadata |
| `path-backslash-separator.zip` | `unsafe_path` | entry metadata |
| `path-embedded-nul.zip` | `unsafe_path` | entry metadata |
| `path-non-utf8.zip` | `unsafe_path` | entry metadata |
| `path-duplicate-normalized.zip` | `duplicate_path` | entry metadata |
| `entry-symlink.zip` | `unsupported_entry_type` | entry metadata |
| `entry-fifo.zip` | `unsupported_entry_type` | entry metadata |
| `entry-character-device.zip` | `unsupported_entry_type` | entry metadata |
| `entry-directory-name-collision.zip` | `unsupported_entry_type` | entry metadata |
| `bomb-entry-count-declared.zip` | `archive_limit` | directory pre-parse |
| `bomb-total-uncompressed-declared.zip` | `archive_limit` | directory open |
| `bomb-entry-uncompressed-declared.zip` | `archive_limit` | entry metadata |
| `bomb-compression-ratio.zip` | `archive_limit` | entry metadata |
| `manifest-oversize.zip` | `archive_limit` | entry metadata |
| `bomb-understated-declaration.zip` | `declared_size_mismatch` | expansion |
| `manifest-missing.zip` | `missing_manifest` | manifest |
| `manifest-malformed.zip` | `malformed_manifest` | manifest |
| `manifest-unsupported-schema.zip` | `unsupported_schema_version` | manifest |
| `hash-mismatch.zip` | `hash_mismatch` | expansion |

Several fixtures record sizes or an entry count that disagree with what they
actually contain. That is deliberate: archive metadata is attacker-controlled, so
the loader may use it to reject early but never to authorise reading more than a
ceiling allows. `bomb-understated-declaration.zip` is the case that proves the
distinction — it declares 1,024 bytes and contains eight mebibytes.

## Built at test time rather than tracked

Two cases are cheap to build and expensive to store, so the implementing change
builds them instead of reading them from here.

**The entry-count boundary.** An archive of 4,096 empty entries must load and one
of 4,097 must be rejected at the directory pre-parse. Each is roughly 370
kilobytes on disk and a few milliseconds to construct with the same ZIP writer
the tests already depend on. `bomb-entry-count-declared.zip` is tracked because
it is 1,347 bytes and covers what a generated archive cannot: an entry count
that disagrees with the records actually present.

**Packages above the `tiny` size.** As above — build one at the size the test or
benchmark needs.

The dividing line is whether the construction parameters say everything. An
archive of N empty entries is fully described by N. An adversarial archive is
not: its exact bytes are the test, so a paraphrase of how it was built is not a
substitute and those bytes stay tracked.

## What is not covered here

Directory sources can carry symbolic links, hard links, and device nodes that no
archive entry can express, and Git cannot track them portably: a symlink checked
out on Windows without developer mode becomes a regular file, which would make
the fixture test nothing. Those cases are created at test time by the change that
implements directory loading, not stored here.

`G-014` also does not bound the container-byte ceiling with a fixture. Crossing
it requires an archive larger than the ceiling itself, and the check is a length
comparison made before anything is parsed, so it is measured against a synthetic
buffer instead.
