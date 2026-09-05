# ADR 0065: Scope immutable source-release checks to their release

- **Status:** Proposed
- **Date:** 2026-09-05
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

The Phase 5 work keeps product version 0.4.0 and ABI 1.5 unchanged while adding
qualification tools and documentation. `--release-scope` is a different check:
it reads committed `HEAD` and requires exact v0.4.0 `.cargo`, `.github`, `crates`,
`docs`, and `fixtures` tree identities. Re-pinning those identities for feature
work would change the meaning of historical source-release evidence.

The pre-change baseline `2c80d4eb87a658069514cb0e83353e830a705882` passed both
architecture and source-release checks. A later documentation or CI tree is not
that frozen release tree even when public product behavior remains unchanged.

## Decision

Normal branch CI runs the current workspace architecture check without
`--release-scope`. The opt-in source-release check remains available for an
explicit release-candidate checkout; its v0.4.0 identities, validator, tests and
canonical release body stay unchanged.

A future release must define its own reviewed release boundary, not silently
reuse or overwrite v0.4.0 evidence. This does not approve native redistribution,
change a package version, or resolve a Phase 5 gate.

## Alternatives

- **Refresh historical hashes on every branch:** rejected because a passing
  check would no longer attest to the original source-release boundary.
- **Retain the frozen-tree check on every feature head:** rejects intentional
  development changes unrelated to the old release and requires either the
  preceding hash rewrite or an unnecessary product-version change.

## Consequences

Contributor commands and all three native/policy CI invocations now separate
current architecture validation from historical release validation. Integrator
APIs, ABI ordering, target support and default feature selections do not change.
Release review must explicitly invoke the source-release check from the exact
candidate checkout; normal PR CI no longer supplies that release-specific proof.

## Verification

`cargo run --locked --package mado-pilot-dependency-check` remains mandatory in
CI. Existing deterministic release-validator mutation tests remain in the
workspace suite. Review verifies that `tools/dependency-check/src/release.rs`,
its tests, and `docs/releases/v0.4.0.md` retain their prior bytes. Native profile
qualification evidence is separate under
[../evidence/native-release-profiles](../evidence/native-release-profiles/README.md).
