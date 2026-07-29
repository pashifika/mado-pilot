# ADR 0001: Asset archive container and safety ceilings

- **Status:** Accepted
- **Date:** 2026-07-27
- **Resolves gate:** `G-014` from [../validation-gates.md](../validation-gates.md)
- **Supersedes:** _none_

## Context

Phase 1 adds versioned, network-free asset packages loadable from a directory, a
caller-owned memory description, or a local archive. Archive loading is the one
of those three that reads attacker-shaped metadata before it knows anything
about the content, so gate `G-014` blocks it until the entry-count,
uncompressed-byte, and compression-ratio ceilings exist with adversarial
evidence behind them.

Nothing in the repository implements asset loading today, and Phase 0 declared
no product dependency, so the archive container and the manifest serialization
are also still open. They are settled here because a ceiling cannot be derived
without knowing what metadata the container exposes and when.

The measurements are in [../evidence/g-014](../evidence/g-014/), taken on native
`aarch64-apple-darwin` and native `x86_64-pc-windows-msvc`. The fixtures they
refer to are tracked in [../../fixtures/assets/g-014](../../fixtures/assets/g-014/)
and pinned by `SHA256SUMS`.

## Decision

Version-one asset archives are **ZIP**, restricted to the `Stored` and
`Deflated` compression methods, unencrypted, with entry names that are valid
UTF-8. Version-one manifests are **JSON**, UTF-8, parsed strictly into a typed
schema that rejects unknown fields, at the exact package-relative path
`madopilot-package.json`.

Archive loading enforces these implementation ceilings:

| Ceiling | Value |
|---|---|
| `max_manifest_bytes` | 4 MiB (4,194,304) |
| `max_entry_count` | 4,096 |
| `max_entry_uncompressed_bytes` | 64 MiB (67,108,864) |
| `max_total_compressed_bytes` | 256 MiB (268,435,456) |
| `max_total_uncompressed_bytes` | 512 MiB (536,870,912) |
| `max_compression_ratio` | 64 |

A caller may configure any limit **at or below** the matching implementation
ceiling. A configured limit above a ceiling is rejected as an invalid argument;
it never weakens the ceiling.

Three rules govern how the ceilings are applied.

**Enforce before the cost.** Every ceiling is checked before the allocation or
expansion it bounds, in this order: total source bytes; the entry count recorded
in one unambiguous single-disk archive trailer, read before the central directory
is materialized; a bounded, allocation-free scan proving that the selected
central-directory header sequence agrees with that trailer; the declared total
uncompressed bytes; per-entry declared sizes, entry types, and normalized names;
the aggregate declared ratio; the manifest; then streamed expansion. Ambiguous
trailers and disagreeing single-disk count fields are malformed archives rather
than alternate interpretations.

An archive presents one trailer, and enforcing a count before the open is worth
something only if the reader opens the trailer that was counted. The pre-parse
selects the single end-of-central-directory record whose comment accounts for
the exact file suffix and refuses any further record beginning after it, because
the ZIP reader searches backwards from the end of the file and would otherwise
open a record whose entry count was never enforced. After the archive is opened,
the central directory the reader reports is compared against the one the
pre-parse validated, and a disagreement is a malformed archive rather than an
alternate interpretation. The second guard runs after the reader has already
paid for the substitute directory, so it makes the assertion true unconditionally
rather than before the cost; the first guard is what keeps the cost bounded for
the candidates the reader reaches first.

**Recorded metadata may reject, never authorise.** A size or count recorded in
an archive is attacker-controlled. It may be used to stop early, and it must be
re-checked against bytes actually produced. An entry is cut off at its declared
size even when a ceiling would have allowed more, so an understated declaration
is rejected after one chunk rather than after a ceiling's worth of expansion.
Because a successfully read entry must produce exactly its declared uncompressed
length, the expansion-stage observed-ratio cross-check is mathematically
identical to the aggregate declared-ratio check against the same recorded
compressed lengths. Actual work is independently bounded by source, per-entry,
and total-uncompressed ceilings; the ratio is never the sole authorization.

**No trusted extraction.** Archive entries are read in place and never written
to a filesystem location that later reads treat as trusted. Entry names are
normalized to relative package paths; absolute paths, drive and UNC roots,
parent traversal, backslashes, embedded NULs, non-UTF-8 names, directory
entries, non-regular entry types, and duplicate normalized names are rejected.

## Alternatives

**TAR, with or without an outer compressor.** Rejected on metadata access. TAR
has no central directory, so entry count and total uncompressed size cannot be
known without streaming the whole archive, and for `tar.gz` that means
decompressing it. The entry-count and total-byte ceilings would then be
enforceable only after paying the cost they exist to prevent, which is the exact
failure the evidence shows: reading 60,000 ZIP entries costs 32,679,704 bytes
through the central directory and 144 bytes through the trailer.

**7z, or a bespoke container.** Rejected on dependency surface and review cost.
Both would add a less-exercised parser to a security-sensitive boundary for no
capability ZIP does not already provide.

**TOML manifests.** Seriously considered — the repository already uses TOML for
benchmark profiles, and it reads better by hand. Rejected on dependency surface
and on producer breadth: `toml` pulls in `toml_edit`, `winnow`, `indexmap`,
`serde_spanned`, and `toml_datetime`, whereas `serde_json` is already a
workspace dependency and adds nothing new; and a package producer may be a C or
C++ tool, for which emitting JSON is trivial and emitting TOML is not. TOML's
datetime type buys nothing for this schema.

**YAML manifests.** Rejected. Anchors and aliases are a denial-of-service
surface in a format that must parse untrusted input, the implicit typing rules
are a correctness hazard, and there is no maintained serde implementation to
depend on.

**Deriving ceilings from a threat model rather than measurement.** Rejected
because the gate exists precisely to stop that. Each number here is anchored to
a measured legitimate maximum with the headroom stated.

**Leaving the ratio ceiling as the only expansion guard.** Rejected. A ratio
ceiling alone does not bound the central directory, which scales with entry
count independently of any expansion, and does not bound the single largest
allocation, which is the manifest.

## Consequences

**For integrators.** A package must fit inside the ceilings above. The most
restrictive in practice is 4,096 entries, which is eight times the largest
representative package measured. A host that wants stricter limits can set them;
a host that wants looser ones cannot, and needs new evidence and a superseding
ADR instead. Manifests must be UTF-8 JSON at `madopilot-package.json`, with no
unknown fields.

**What becomes harder to change.** The container and the manifest format become
part of what a version-one package *is*. Changing either is a schema-version
migration, not an implementation detail. Raising a ceiling is a superseding ADR
with fresh measurements on both release targets; lowering one is a
compatibility break for packages that already fit.

**Dependencies.** Implementing this commits the asset package to a ZIP reader, a
DEFLATE implementation, SHA-256, and a JSON parser. The probe used `zip` 8.6.0
with default features off and only `deflate-flate2` enabled, `flate2` 1.1.9,
`sha2` 0.11.0, and `serde_json` 1.0.151. Every crate in that closure carries
MIT, Apache-2.0, or Zlib terms, all already on the approved list in
[../third-party-dependencies.md](../third-party-dependencies.md), so no license
exception is needed. The exact versions are pinned by the change that adds them,
not by this ADR; disabling the `zip` crate's default features is not optional,
because they pull in bzip2, LZMA, PPMd, XZ, Zstd, and AES for methods this
decision does not support.

**A `zip` version bump is a review step, not a routine update.** The trailer
rules above are written against how `zip` 8.6.0 finds and accepts a trailer, so
a bump must re-verify three sites in the new version: `spec.rs:806`, the
whole-file backward search for the end-of-central-directory signature;
`spec.rs:823-828`, the relaxed comment check that accepts a record with trailing
bytes after it; and `read/zip_archive.rs:167-205`, the fallback to an earlier
candidate and the reservation it makes before the entry count can be compared.
Widen or narrow any of the three and the pre-parse is guarding a different set
of candidates than the reader will reach. The same sentence is carried as a
comment in `crates/automation/assets/src/archive.rs` beside the cross-check it
protects.

**Performance.** The guards are not a measurable tax on legitimate packages: the
representative 512-template package validates end to end in 6,694 µs on Apple
Silicon and 6,258 µs on Windows. The evidence probe's entry-count extraction that
avoids the largest unguarded allocation costs under a microsecond and 144 bytes on
both. The implementation additionally performs the bounded header-consistency scan
above before handing the archive to the ZIP reader.

**What this does not decide.** Directory and memory sources are not archives and
are not bounded by these ceilings; they have their own containment rules.
Nothing here implements archive loading, and no documentation may claim archive
loading exists until the change that implements and tests it lands.

**Changed in the same change.** [../validation-gates.md](../validation-gates.md)
marks `G-014` resolved; [../architecture.md](../architecture.md) records the
tracked fixture directory and the asset ceilings; the fixtures and evidence
above are added; the disposable probe that produced them is removed.

## Verification

- The tracked adversarial fixtures in
  [../../fixtures/assets/g-014/adversarial](../../fixtures/assets/g-014/adversarial/)
  each cross exactly one rule. The change that implements archive loading adds
  conformance tests asserting the failure category **and** the stage for every
  one of them, on both release targets. A fixture rejected later than its listed
  stage means an earlier guard is missing, and fails the test even though the
  package was refused.
- The entry-count boundary — 4,096 accepted, 4,097 rejected at the pre-parse —
  is asserted against archives the test builds, because an archive of N empty
  entries is fully described by N and storing two of them would cost most of the
  fixture directory. Everything whose exact bytes are the test stays tracked.
- `fixtures/assets/g-014/SHA256SUMS` pins every fixture, so a silent fixture
  edit invalidates the evidence visibly rather than quietly.
- The representative packages must load successfully, and the directory and
  archive forms of the `tiny` package must commit equivalent packages.
- A caller-supplied limit above any ceiling must be rejected as an invalid
  argument. This is a unit test, not a review rule.
- Both release targets ran the same fixtures and agreed on every failure
  category, every stage, and every expanded-byte count, and regenerated all 33
  tracked fixture files byte-identically. Cross-target agreement is therefore an
  established fact rather than an assumption the implementation may rely on
  untested; the conformance tests above re-establish it per target.
- The measurements themselves are not re-run automatically. The evidence records
  the exact hosts, toolchains, dependency versions, and fixture hashes, and
  [../evidence/g-014/probe.md](../evidence/g-014/probe.md) describes the
  pipeline precisely enough to reimplement without the deleted probe. A change
  that alters an implementation ceiling is the review step that must produce new
  measurements.
