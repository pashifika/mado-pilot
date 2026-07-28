# ADR 0004: A hand-written C header verified by cross-language layout probes

- **Status:** Accepted
- **Date:** 2026-07-28
- **Resolves gate:** _none_. It records how [`G-010`](../validation-gates.md#g-010)
  will be evidenced, and deliberately freezes nothing the gate owns.
- **Supersedes:** _none_

## Context

Phase 1 stage 6 implements the first C ABI functions. Two questions had to be
answered before a single declaration was written, and neither has a default in
this repository.

**Where the header comes from.** `docs/architecture.md` reserves
`include/madopilot/madopilot.h` and says the generated header is produced by the
change that implements the first C ABI functions. No binding generator exists
anywhere in the workspace, nothing is pinned, and the change's task list requires
a reproducible approach that does not depend on an unpinned global tool. "Run
whatever `cbindgen` the developer has installed" is therefore excluded by the
requirement itself.

**What CI does with C.** The Ubuntu `repository-policy` job compiles nothing on
purpose: every step reads manifests, metadata, or source text, so the job needs
no OpenCV on a host that is neither release target. Adding a step that needs a C
compiler there would also need OpenCV there, because
`mado-pilot-backend-opencv` is a build prerequisite for the whole workspace. The
two native jobs already provision OpenCV and already run the release-target C
toolchain — MSVC on `windows-2025`, the Xcode Command Line Tools on `macos-15`.

## Decision

**The C header is hand-written and tracked at
`crates/bindings/capi/include/madopilot/madopilot.h`.** It is a reviewed source
artifact, not build output. No binding generator is added to the workspace.

**Agreement between the header and the Rust definitions is proven by a
cross-language layout probe rather than asserted.** `mado-pilot-capi` carries a
`c-abi-check` example that:

1. locates the built `cdylib` next to its own executable;
2. compiles and runs `tests/c/madopilot-abi-layout.c`, which reports the size,
   alignment, and every field offset of every public structure and of the
   function table, as the C compiler laid them out;
3. compares that report field by field against the same values computed from the
   Rust `#[repr(C)]` definitions with `size_of`, `align_of`, and `offset_of!`;
4. compiles, links, and runs the C example against the library and checks its
   observable outcome.

The two reports come from two different compilers, so a divergence between the
header and the Rust definitions fails the check on the host where it matters.

**CI runs that check in the two native jobs only.** The Ubuntu
`repository-policy` job continues to compile nothing.

## Alternatives

**Generate the header with `cbindgen` as a pinned build dependency.** Rejected on
three counts, none of which is unfamiliarity. It adds a large build-time
dependency tree — `syn`, `proc-macro2`, `quote`, `clap`, `heck`, `tempfile`,
`toml` and their transitives — to a workspace whose dependency policy requires a
per-crate license and maintenance review, in exchange for declarations that a
reviewer can read directly. It verifies nothing: `cbindgen` re-parses Rust source
text and emits what it believes the layout to be, so a generated header and a
compiled library can still disagree, and the layout probe above would still be
required. And the header is a prose contract as much as a set of declarations —
ownership rules, borrowed-view owners, structure-prefix defaults, and the
provisional status of every value are things a reader needs at the declaration,
and a generator degrades them to whatever survives doc-comment translation.

**Generate the header from a build script in `mado-pilot-capi`.** Rejected: it
makes the released contract a build artifact that differs with the host, and
[`G-010`](../validation-gates.md#g-010) requires the frozen Phase 1 header to
become a permanent old-prefix compatibility fixture. A fixture that is
regenerated is not a fixture.

**Assert the layout with `_Static_assert` in C only, against literals.** Rejected
as insufficient on its own: it proves the C side matches the numbers a human
wrote in the C file, not that the Rust side produces those numbers. The probe
keeps the `_Static_assert`s for the invariants that must hold on any conforming
target — `struct_size` first at offset zero in every versioned structure — and
gets the numeric agreement from the comparison instead.

**Add the C compilation to the `repository-policy` job.** Rejected: it would need
OpenCV on `ubuntu-latest`, which is neither release target, to compile a
workspace that the job exists specifically not to compile.

**Run the C check only on the hands-on verification hosts.** Rejected: native
correctness is a per-pull-request property in this repository, and the two native
jobs already pay for the toolchain. The measured cost is a single-translation-unit
compile and two short program runs on top of a job that already builds the
workspace.

## Consequences

- A change to any `#[repr(C)]` definition in `mado-pilot-capi` must be made in
  the header in the same change. The probe fails otherwise, on both release
  targets.
- The repository now depends on a C toolchain for one verification step. It is
  the release target's own toolchain in both cases and no new installation is
  required on either CI runner or either verification host, but
  `cargo test --workspace` no longer covers everything: `c-abi-check` is a
  separate command, documented in [../c-abi.md](../c-abi.md) and in
  `CONTRIBUTING.md`.
- The header is now the artifact that `G-010` freezes. When the gate resolves,
  the then-current header is copied to a fixture directory unchanged and every
  later library in the same ABI major is compiled against it.
- Nothing here freezes a status value, a structure layout, or the function-table
  prefix. Those remain provisional and the header says so at the top.

## Verification

- `cargo run --locked --package mado-pilot-capi --example c-abi-check --
  --label "<host>"` compiles the probe and the example, compares the two layout
  reports, and fails on the first divergence. It is a required step in the
  Windows and macOS CI jobs.
- `crates/bindings/capi/tests/layout.rs` asserts the invariants that hold
  independently of the C compiler — `struct_size` at offset zero, the documented
  mandatory prefix sizes, and the function table's Phase 1 size — so a Rust-only
  `cargo test` still catches a reordering.
- Evidence for both release targets is recorded under
  [../evidence/](../evidence/) with the host, toolchain, and reported layout.
