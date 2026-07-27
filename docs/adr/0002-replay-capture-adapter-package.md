# ADR 0002: A production replay capture adapter package

- **Status:** Accepted
- **Date:** 2026-07-27
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

Phase 1 delivers one deterministic capture-to-match slice before native desktop
capture adds platform variability. That needs a capture adapter that produces
known frames from a configured source, on both release targets, with no host
state involved.

The Phase 0 inventory has no home for it. Every existing package is the wrong
one for a specific reason, and the reasons are not interchangeable:
`mado-pilot-testkit` is development-only and must never ship;
`mado-pilot-capture` is a contract package and must not contain a concrete
adapter; `mado-pilot` is a composition root, and putting decoding, source
validation, and publication state in it would make it an implementation
package; `crates/platform` is for adapters whose behavior is an operating
system's, and replay's behavior must be identical on both.

The architecture baseline also says, in as many words, that widening the
facade's dependency row is a normative change that needs an ADR rather than a
quiet allowlist edit. This is that ADR.

## Decision

Add `mado-pilot-adapter-replay` at `crates/adapter/replay`, in the same change
that implements its behavior and its tests. Its allowed MadoPilot dependencies
are `mado-pilot-core` and `mado-pilot-capture`. The facade's dependency row is
widened to include it, because default wiring is the facade's job; nothing else
may name it, and `mado-pilot-runtime` in particular continues to see capture
contracts only.

`crates/adapter/` is introduced as the home for platform-neutral adapters,
distinct from `crates/platform/`, which stays for adapters whose behavior is the
operating system's.

**A replay source stores raw pixels, not encoded images.** A frame is bytes plus
a declared extent, format, stride, timestamp, continuity, and optional target
placement, described by a JSON manifest for a directory source or supplied
directly for a memory source.

**The adapter publishes through the capture package's stream state** rather than
assigning identity itself, and advances its sequence when a consumer asks for a
newer frame rather than on a timer.

## Alternatives

**Put replay in `mado-pilot-testkit`.** Rejected: testkit is a development
dependency by policy and must never appear in a shipped graph. The Phase 1 Rust,
C, and C++ examples are shipped artifacts that need a working capture source, so
the source cannot live in a package they are forbidden to link.

**Put replay in `mado-pilot-capture`.** Rejected: a contract package that
contains one of its own implementations stops being a seam. The dependency
checker enforces exactly this, and relaxing it for the first adapter would
relax it for every later one.

**Put replay in the facade.** Rejected: source validation, manifest parsing, and
publication state are implementation, and the facade's only job is wiring. It
would also make the behavior unreachable from a test that does not go through
the whole facade.

**Put replay under `crates/platform`.** Rejected: that directory means "this
behavior is the operating system's". Replay's whole value is that it is not.

**Accept encoded images (PNG) in the source.** Seriously considered, and
rejected for Phase 1. A replay fixture exists so that a contract test, an
example, and a benchmark all observe the same bytes; a decoder between the
fixture and the oracle makes "the same" depend on the decoder's behavior across
versions and targets. It would also put an image parser — reading
caller-supplied files — inside a capture adapter, which is a security surface
this change does not need.

The cost is real: raw fixtures are large, so tracked replay fixtures stay small,
and a caller who wants to replay real screenshots has to convert them first.
Adding a decoder later is additive and needs only its own dependency review;
removing one after tests depended on its exact output would not be.

## Consequences

**Inventory.** The workspace now has fifteen product packages. The package
table, the dependency graph, the allowlist, the `REQUIRED_PACKAGES` constant,
and the checker tests move together, which is what makes the count meaningful
rather than decorative.

**A new role in the checker.** `PackageRole::Adapter` joins `Platform`. Both are
product roles; the distinction records whether a package's behavior is expected
to differ by operating system, which is what a reviewer needs to know when
asking why something is not covered by a native CI job.

**Dependencies.** The adapter adds `serde` and `serde_json` for its manifest.
Both are already resolved in the committed lockfile, carry approved licenses, and
match the manifest serialization chosen for asset packages in
[ADR 0001](0001-asset-archive-container-and-safety-ceilings.md), so a reader
meets one manifest format rather than two.

**What this does not authorize.** No native capture, no permission handling, no
watchers, and no queue policy. `G-002` — Windows producer-pool and frame
detachment — stays open and is untouched by this decision, because the frame
ownership contract this adapter satisfies was deliberately written so that
retaining a public frame never requires retaining a producer slot.

## Verification

- The dependency checker's `REQUIRED_PACKAGES` and `ALLOWED_DEPENDENCIES`
  contain the package and its two edges, and `tools/dependency-check` fails if
  the workspace disagrees.
- Checker tests assert that the adapter may depend on core and capture, may not
  reach input, vision, OCR, assets, runtime, the facade, or the C ABI, and that
  only the facade may name it. A test also asserts that a contract package
  depending on the adapter is rejected, since that is the inversion this package
  boundary exists to prevent.
- The adapter passes the capture contract suite, which the controlled test
  adapter in `mado-pilot-testkit` passes as well. Two behaviorally different
  implementations are what make the seam real rather than a description of one
  implementation.
- Replay source validation — unsafe pixel paths, byte-length mismatch, unknown
  format or continuity names, duplicate target names, empty sequences — is
  covered by unit tests in the adapter.
