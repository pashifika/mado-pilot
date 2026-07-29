# ADR 0010: Asset source snapshot and archive ownership

- **Status:** Accepted
- **Date:** 2026-07-30
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

Two questions about the same subject — what one load reads, and who owns it —
were answered inconsistently across the repository, and each answer was
re-litigated every time the code around it moved.

**What one load reads.** `mado-pilot-assets` copies an archive *file* once, after
the source-size gate, and every later stage reads that copy
(`crates/automation/assets/src/archive.rs`). The copy exists because the trailer
pre-parse bounds an allocation the ZIP reader makes from a directory the
pre-parser proved, and two reads of a mutable file are two archives. Three texts
then described what the copy guarantees, in three different strengths:
[architecture.md](../architecture.md)'s summary said in-place mutation "cannot
silently change the committed bytes"; the focused section said the copy is a
sequence of reads holding "no mandatory writer exclusion on Unix", so bytes from
two versions were possible; and the change's normative requirement — *Stable
source snapshot* — required "one internally consistent source snapshot". A
reviewer reading all three could not tell which one the implementation was
claiming to satisfy.

None of the three was accurate. What the code does is compare the retained
handle's identity, change stamp, length and link count before the copy and again
after it (`crates/automation/assets/src/filesystem.rs`,
`crates/automation/assets/src/archive.rs`), and the strength of that comparison
differs by platform: on Windows a retained source is opened denying write and
delete sharing, so a concurrent writer never reaches the file at all; on Unix
there is no mandatory exclusion, and the comparison is what stands in for it.

**Who owns a caller-supplied archive.** The C boundary receives
`MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES` as a borrowed `madopilot_bytes_t` and
used to turn it into an owned `PackageSource::ArchiveBytes(Arc<[u8]>)`. Four
review rounds on `fix/phase-1-verification-findings` each fixed that copy and
each produced the next finding about it: the ceiling was checked after the copy,
then the fix copied twice, then the one-copy fix was infallible and blind to the
operation, then the operation-aware fix published without a final commit check
while remaining infallible. The requirements were not simultaneously satisfiable
as written: stable Rust has no fallible `Arc` slice allocation, and the only
fallible route (`Vec::try_reserve_exact` then `Arc::from`) reinstates the second
copy and doubles the peak the ceiling exists to bound. A caller-length-sized
infallible allocation at a C boundary can only answer allocation failure by
terminating the host, which is the one answer a boundary whose contract is
"every function returns a status" must never give.

## Decision

**The snapshot contract.** One load reads one immutable sequence of bytes, and a
committed package is assembled from that one sequence. That the sequence is also
one temporal version of a filesystem source is proved per platform, and the proof
is documented per platform rather than averaged: on Windows by the retained
handle's denial of write and delete sharing; on Unix by comparing identity,
change stamp, length and link count around the copy, so any write the filesystem
records refuses the load with `SourceChanged`. The residual — a filesystem whose
change-stamp granularity cannot separate a write from the checks around it — is
stated wherever the guarantee is stated, and is not implied by omission. A change
that lands after the last check is outside the contract: the bytes it would have
affected were already read.

**Archive ownership.** The C archive-bytes boundary borrows and never owns.
`PackageLoader::load_archive_bytes` (re-exported as `Engine::load_archive_bytes`)
reads a caller's archive in place for the duration of one call; the committed
package holds each template's content in its own allocation, so nothing retains
the archive. `PackageSource::ArchiveBytes` remains for a Rust caller that owns
its bytes and wants a reusable source. No allocation on the C load path is sized
by a caller's declaration: what remains is grown as bytes are actually produced.
That is the whole of the claim — allocation failure while reading package content,
whether the manifest or an entry, is still an abort, and the Consequences below
name where.

## Alternatives

**Record the infallible allocation as an accepted limitation.** The cheapest
option: close the missing commit check locally and record the allocation as an
architecture-level accepted-known. Rejected because the limitation is real rather
than cosmetic — a valid C caller could ask for up to the configured source
ceiling (256 MiB at the implementation maximum) and be answered with process
termination — and because the neighbouring filesystem path already reserves
fallibly and maps failure to `SourceUnreadable`, so accepting the abort would
freeze an asymmetry inside one module. Removing the copy narrows that asymmetry to
the sizes the loader chooses for itself; it does not remove it, and this ADR does
not claim otherwise.

**Change the retained representation to a fallible one.** `Arc<Vec<u8>>` would
give a fallible caller-sized allocation with one payload copy, unlike
`Arc<[u8]>`. Rejected because it changes the public payload type of
`PackageSource::ArchiveBytes`, which
[ADR 0006](0006-public-rust-names-and-compatibility-policy.md) treats as a
breaking change requiring a superseding ADR and a minor bump — a high price for
keeping a copy that nothing needs.

**Implement an atomic snapshot or mandatory writer exclusion on Unix.** This
would let the stronger reading of the snapshot requirement stand without a
residual. Rejected for version one: advisory `flock`/`fcntl` locks do not exclude
the writer that matters, so a real guarantee means a filesystem-dependent path
such as APFS `clonefile` plus a fallback for filesystems that lack it, new FFI in
a package that is `std`-only today, and a platform support matrix. The residual
that remains without it is narrow and now documented.

**Weaken the requirement to the immutable-byte-sequence property alone.** The
shortest documentation change. Rejected because the specification would then stop
claiming two properties that are implemented and tested — Windows's actual writer
exclusion, and the Unix comparison's refusal of any recorded write.

## Consequences

- Integrators gain `Engine::load_archive_bytes` for an archive they hold in
  memory and do not want to hand over. `PackageSource::copy_archive_bytes`, added
  earlier on this branch and never released, is gone; nothing released named it.
- The C contract for `MADOPILOT_PACKAGE_SOURCE_ARCHIVE_BYTES` is unchanged in
  shape: the view must be readable, and unmodified, for the call. That rule was
  not written down anywhere general — [ADR 0007](0007-phase-1-c-abi-freeze.md) and
  `docs/c-abi.md` state the *output* half, that a library-owned view is valid
  while its owner is retained, and said nothing about caller-supplied input. This
  change widens the window in which the rule matters, from a copy at the start of
  the call to the whole load, so it is now stated generally: in the header's
  function-table preamble and in `docs/c-abi.md` beside the output rule, together
  with what the library owes in return — that it retains no caller memory past the
  call, so a caller may release the archive the moment the call returns. No
  function-table entry, structure, or field moved, so the ABI layout is untouched.
- What this does **not** fix: reading package content allocates infallibly, in
  `read_capped`'s buffer and in the reference-counted copy the package keeps of it.
  That is every entry, bounded by `max_entry_uncompressed_bytes` each and
  `max_total_uncompressed_bytes` in total, and the manifest, bounded by
  `max_manifest_bytes` and then parsed by `serde_json`, which allocates infallibly
  too. Neither buffer is reserved from a declared size — `read_capped` grows its
  buffer as bytes arrive, which is why nothing here is sized by what a caller
  *said*, only by what a source actually produced under a ceiling. Reserving that
  buffer fallibly would not change the outcome while the copy beside it cannot be:
  the copy is `Arc<[u8]>`, unallocatable fallibly on stable Rust for the same
  reason the archive copy was, and changing what it is means changing
  `TemplateSourceRequest.content`, a public field — an ADR 0006 breaking change and
  a separate decision from this one. Any future statement about allocation failure
  at the C boundary has to name these sites.
- Peak memory for a C archive load drops by the archive's length. A caller that
  tightened `max_total_compressed_bytes` still tightens the largest view the
  boundary accepts, because the declared length is answered against that ceiling
  before the view is read.
- What becomes harder: the borrowed path means the loader's internal reader is no
  longer `'static`. A future feature that needs a package to outlive the load it
  came from *without* copying its content would have to revisit that, and the
  migration path is the owned `PackageSource::ArchiveBytes` kind, which still
  exists.
- Documentation changed with the code: `docs/architecture.md` (the asset summary,
  the `source` stage row, the snapshot statement, the allocation statement, the
  ceiling paragraph), the `mado-pilot-assets` archive module documentation,
  `docs/c-abi.md`, and the released C header's function-table preamble and
  package-source paragraph. The normative *Stable source snapshot* requirement and
  its scenarios are updated in the change's own specification.

## Verification

- `crates/automation/assets/tests/operation_context.rs` sweeps a borrowed archive
  load over every deadline and every observable cancellation point, and asserts
  that the **last two** points are both reported at `LoadStage::Commit`. Asserting
  that *some* point reaches commit would not discriminate: the stage has two
  consecutive observation points, the checkpoint and `Operation::commit` itself, so
  a load that checkpointed and then published would still report it once. Proved by
  removing the final `operation.commit` and re-running: both borrowed sweeps fail
  with `(Expansion, Commit)`, while the pre-existing directory and archive-file
  sweeps stay green. Both entries share one tail, so the guard covers both.
- The same file asserts a borrowed load, an owned load, and a file load of the
  same archive produce equal packages, which is what says ownership is not part
  of what a package is.
- `crates/bindings/capi/src/assets.rs` unit tests assert the boundary returns the
  caller's own memory — compared by address, not by contents — and that the
  declared length is refused against the ceiling before the view is read.
- `crates/bindings/capi/tests/abi.rs` loads a package from a lent archive,
  overwrites and drops the lender's buffer, and then still describes the package.
  A second test asserts that an expired operation is refused at admission and
  publishes no handle — at admission is all it proves, because the entry admits the
  operation before it reads the source structure; interruption during a load is
  the assets crate's sweeps, which can drive a controlled clock.
- The snapshot contract's platform halves are pinned by the filesystem
  conformance tests (`crates/automation/assets/tests/`), including the Windows
  assertion that a retained source denies concurrent writers and path
  replacement.
- The residual is not automatically verifiable — a filesystem with indistinct
  change stamps is not something the suite can conjure. The review step that
  catches a regression is the requirement that every statement of the snapshot
  guarantee also state the residual; a claim written without it is wrong on its
  face.
- No benchmark budget moves. `load_package_archive` can only get faster and
  smaller by removing a copy; the committed budgets are ceilings, and the
  profiles were not re-measured for this change.
