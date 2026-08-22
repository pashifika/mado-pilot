# ADR 0006: Public Rust names and the compatibility policy that now applies

- **Status:** Accepted
- **Date:** 2026-07-28
- **Resolves gate:** `G-009` from [../validation-gates.md](../validation-gates.md)
- **Supersedes:** _none_

## Context

Phase 1 built the deterministic Rust workflow, a C ABI over it, and a C++ wrapper
over that, and kept every public Rust name marked provisional throughout. Writing
those three callers is what a naming review needs, because a name is only wrong
in use. Thirteen questions and two interface gaps accumulated in `G-009` while
that happened; none was decided at the time, deliberately, so that the whole
surface could be reviewed at once rather than one rename at a time.

The review is now due: `G-009` blocks Phase 1 exit, and it blocks any Rust
stability promise. This record settles every question it holds and states what
changing one of these names costs from here.

The evidence is the three complete callers —
`crates/mado-pilot/examples/deterministic-slice.rs`,
`crates/bindings/capi/examples/c/deterministic-slice.c`, and
`crates/bindings/capi/examples/cpp/deterministic-slice.cpp` — together with the
facade contract suite in `crates/mado-pilot/tests/deterministic_workflow.rs` and
the orchestration suite in `crates/automation/runtime/tests/orchestration.rs`.

## Decision

The names below are the reviewed Phase 1 public Rust surface. Six items are
renamed, four are added, one behaviour is aligned, and the rest are kept with
the reason recorded.

### Renamed

| Was | Is | Why |
|---|---|---|
| `EngineParts` | `EngineWiring` | It is the seam a composition root fills, and "parts" named the shape rather than the role. |
| `FrameChoice` | `SearchFrame` | Two enumerations with a `Latest` variant met in one call site. The capture-side `FrameSelection` asks a session for a frame; this one tells a search which frame to use, and now says so. |
| `Engine::prepare` | `Engine::prepare_template` | Both methods return a `PreparedTemplate`, so the general one takes the plain name. |
| `Engine::prepare_template` | `Engine::prepare_from_package` | The two differ only in where the source comes from. Each name now carries that. |
| `Session::frame` | `Session::acquire_frame` | It can block. Every other noun on `Session` is an accessor that cannot. |
| `SCHEMA_VERSION`, `MANIFEST_PATH`, `HASH_ALGORITHM` at the facade root | `ASSET_SCHEMA_VERSION`, `ASSET_MANIFEST_PATH`, `ASSET_HASH_ALGORITHM` | Renamed **only** in the `mado-pilot` re-export. `mado_pilot::SCHEMA_VERSION` sat beside `mado_pilot::replay::SCHEMA_VERSION` with nothing saying which schema it versioned. |

The contract packages keep the unqualified names: inside `mado-pilot-assets`,
`SCHEMA_VERSION` is already qualified by the package that owns it, and prefixing
it there would stutter. The facade is a curated surface and this is what curating
it looks like.

### Added

- **`MatchResult::options`** returns the options the search actually ran under.
  Every other condition a caller might compare two results by was already on the
  result; without the threshold and the limit, a caller holding an empty result
  cannot tell "nothing is there" from "nothing scored that high". The C ABI kept
  a second copy beside the result to answer `result_options` and no longer does.
- **`ContentDigest::of`** returns the digest of a byte slice under
  `HASH_ALGORITHM`. A manifest must declare a digest for every entry and the
  loader verifies each one, so assembling a package in memory previously required
  the caller to add a hashing crate to state a value this project already
  computes. It is the same computation the loader performs.
- **`ReplayEngineRequest`**, with `From<ReplaySource>`, so `replay_engine(source)`
  is unchanged and `replay_engine(ReplayEngineRequest::new(source).with_limits(l))`
  is now possible. This closes the second interface gap: `AssetLimits` can be
  configured at or below every ceiling and `Engine::limits` reports what is in
  effect, but the facade always wired the defaults, leaving the one knob that
  bounds what an untrusted package may allocate unreachable.
- **`AssetLimits`, `BackendDescriptor`, and `BackendId`** are now re-exported by
  the facade. `Engine::limits` and `Engine::backend` return the first two and
  `MatchResult::backend` the second, so a caller could receive values of types it
  could not name. This was not in the gate; the review found it.

### Aligned

**`Session::find_template` on a closed session reports `Status::Closed`,
whichever frame the request names.** Searching an exact frame the caller already
holds needs nothing from the capture side and previously succeeded after close,
while the C boundary added its own check. Two surfaces disagreeing about what a
closed session does is exactly the kind of thing a freeze must not preserve.

### Kept, with the reason recorded

- **`FindRequest` and `FindOutcome` beside `MatchResult`.** Two vocabularies, one
  per layer: `find_*` names the session operation, `match_*` names the backend's
  answer. The outcome is deliberately not `FindResult`, because that would read
  as a peer of `MatchResult` rather than as the envelope around one.
- **`FindOutcome` owns the frame it searched.** That is what keeps "which frame
  is this result about" answerable after close, and `into_result` is the release
  path. The cost is that the frame's pixels live as long as the outcome.
- **`REQUIRED_BACKEND` as a `&str`.** It is a policy constant, not the
  backend-selection axis, and Phase 1 has no such axis to name. A second backend
  introduces one and reviews this constant then.
- **`StreamId` and `TargetId` stay opaque with no fixed-width projection.** The C
  ABI mints its own per-session stream number, which correlates its own frames
  correctly and cannot be compared with anything a Rust caller sees. That is
  currently unobservable, because a C caller creates its own engine; it becomes a
  real defect only when one engine is shared across the boundary, which is when a
  projection should be added.
- **`Engine::prepare_from_package` still flattens the `AssetFault` that
  resolution produced.** `Error` lives in `mado-pilot-core`, which must not depend
  on `mado-pilot-assets`, so it cannot carry the typed fault structurally. The
  information is not lost: `AssetPackage::resolve_template` returns the fault, and
  a caller that wants the rule and the stage asks the package. The C ABI now does
  exactly that, which is what makes
  `MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` reachable; see
  [ADR 0007](0007-phase-1-c-abi-freeze.md).

## The compatibility policy that now applies

The reviewed names above are the `0.x` baseline. They are **reviewed, not
stable**: the Rust API stability promise begins at `1.0`.

Until then:

- Renaming or removing a public item listed here is a breaking change. It
  requires an ADR that supersedes this one, a minor version bump under Cargo's
  `0.x` semantics, and synchronized updates to the facade documentation, the
  examples, and any binding that names it.
- Adding an item, a method, or an enum variant is additive and needs no ADR.
  Every public enumeration a later phase may extend is already
  `#[non_exhaustive]`, so a caller keeping a fallback arm is not broken by one.
- Changing what an existing item *means* while keeping its name is the one
  change this policy refuses outright. A caller cannot detect it, and no version
  number communicates it.
- The C ABI's compatibility rules are separate and stricter; they are
  [ADR 0007](0007-phase-1-c-abi-freeze.md)'s to state. A Rust rename does not
  imply a C rename, and the two were aligned here only because they were being
  frozen in the same change.

## Alternatives

**Keep every provisional name and resolve `G-009` by declaring them reviewed.**
The cheapest option, and the review found six names that a caller had already
tripped over — including two methods a reader cannot tell apart from their names
and a blocking operation spelled as a field access. Spending the rename now,
while the only callers are in this repository, costs a day; spending it after a
release costs every integrator.

**Introduce a builder type for the composition root.** `G-009`'s first question
asked for one, as a discoverable home and an additive place for later options. A
builder names a decision, and Phase 1 has exactly one adapter pair to wire, so
the decision it would name does not exist yet. A typed request object is the
shape this codebase already uses for non-trivial options — `OpenRequest`,
`FrameRequest`, `FindRequest` — and it takes the later options a builder was
wanted for. If a second capture adapter arrives, a builder can be added beside
it without breaking the request.

**Move the asset vocabulary into a `mado_pilot::assets` module**, mirroring
`mado_pilot::replay`, instead of prefixing three constants. Symmetric, and it
would qualify the whole vocabulary rather than the part that collides. Rejected
because `replay` is configuration a caller names once at construction, while
`AssetPackage`, `PackageSource`, and `AssetFault` appear throughout a workflow;
putting them behind a module path would make the common case longer to fix a
collision that affects three constants.

**Rename `FindOutcome` to `FindResult` for symmetry with `MatchResult`.**
Rejected: the symmetry is the problem. They are not peers, and a name that says
they are would make the one-word-apart confusion worse rather than better.

**Give `StreamId` and `TargetId` a fixed-width projection now**, so a C caller's
frame identities and a Rust caller's are comparable. Rejected as premature: it
weakens opacity for a scenario Phase 1 cannot reach, and the right shape depends
on whether engine identity has to travel with the ordinal — which is a question
for whichever phase first shares an engine across the boundary.

## Consequences

- **Integrators.** There are none yet; nothing is published. Every caller in this
  repository was updated in the same change, and the C and C++ surfaces were
  aligned rather than left to diverge.
- **What is harder to change.** These names now cost an ADR and a version bump.
  That is the intent. The migration path for any future rename is a deprecated
  re-export for one minor version, then removal.
- **A Phase 2 constraint follows from keeping `FindOutcome` owning its frame.**
  On a native capture adapter, an outcome held by a caller pins that frame's
  storage. `docs/architecture.md` requires that retained public frames not pin
  producer buffer-pool slots capture progress needs, so the Windows
  producer-pool decision under `G-002` must provide detachment rather than
  assume outcomes are short-lived. This ADR does not decide that; it records
  that the naming decision hands `G-002` a requirement.
- **Documentation and tests changed in this change.** The facade and runtime
  crate documentation no longer say every public name is provisional; they point
  here. `crates/support/testkit/src/vision_contract.rs` gained a check that a
  result reports the options it ran under, so every backend must. The
  orchestration suite gained a check that a closed session refuses an exact-frame
  search. The asset suite gained a check that a memory package can be assembled
  with nothing but the public surface, and three private copies of a SHA-256
  helper in the test tree collapsed into `ContentDigest::of` — which is what the
  gap looked like from the inside.
- **No performance obligation follows.** None of these changes alters what any
  operation does; `MatchResult` grew by one `MatchOptions` value, which the C ABI
  previously stored separately.
