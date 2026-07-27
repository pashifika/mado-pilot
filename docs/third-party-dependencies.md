# Third-party dependency policy

MadoPilot is distributed under Apache-2.0. Every dependency the project adds
becomes part of what integrators must license, ship, and keep patched, so a
dependency is a reviewed decision rather than a convenience. This document is the
normative policy; `deny.toml` is its machine-checked form, and the two are updated
together.

The policy covers two kinds of dependency:

- Rust crates resolved by Cargo and recorded in the committed root `Cargo.lock`.
- Native libraries and model files that MadoPilot links, loads, or bundles, such
  as OpenCV, ONNX Runtime, and OCR models.

## Project license

The project and every Cargo package declare `Apache-2.0`, matching the root
`LICENSE` file. A package must not declare a placeholder or a license that
conflicts with the root file.

## Approved Rust dependency licenses

The following permissive licenses are approved and enforced by `cargo deny check
licenses`:

`Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `MIT`, `MIT-0`, `BSD-2-Clause`,
`BSD-3-Clause`, `ISC`, `Unicode-3.0`, and `Zlib`.

Any other license — including weak or strong copyleft, source-available, and
dual-license offers that require choosing a non-approved term — requires an
architecture decision record before the dependency is added. The ADR states why
the dependency is necessary, what the license obligates integrators to do, and
what the alternative was.

A crate that publishes no license metadata is treated as unlicensed and is
rejected.

## Approved sources

Rust dependencies come from the crates.io registry. Git dependencies and alternate
registries are rejected by `cargo deny check sources`.

Path dependencies are enforced separately, because cargo-deny has no path-location
rule: the architecture checker rejects any path dependency that does not resolve to
a workspace member, and rejects a dependency that carries a member's name but
resolves from a registry or Git source instead of the member itself.

Adding a Git dependency requires an ADR that records the reviewed revision, why a
published release is unavailable, and the condition for returning to a published
release. The revision is pinned; a branch or tag reference is not sufficient.

## Security advisories

`cargo deny check advisories` runs against the RustSec advisory database and the
committed lockfile. An advisory affecting a resolved dependency, and a yanked
resolved version, are both actionable: the fix is to upgrade, to replace the
dependency, or to record a documented exception.

## Duplicate versions

Duplicate versions are reported as a review signal rather than a hard failure,
because a transitive duplicate is often outside the repository's control. The
reviewed resolution is either to unify the requirement or to record in the pull
request why the duplicate is accepted.

## Documented exceptions

An exception is a temporary, named relaxation of this policy. Every exception:

- appears in `deny.toml` under the matching `ignore`, `exceptions`, `skip`, or
  `allow` key;
- appears in the table below with the affected package, the policy it relaxes, the
  reason, and the condition that removes it;
- is re-checked whenever the lockfile changes.

An undocumented relaxation is a policy violation even when the automated check
passes, because the automated check only sees `deny.toml`.

| Package | Policy relaxed | Reason | Removal condition |
|---|---|---|---|
| _none_ | — | — | — |

## Before adding a Rust dependency

Confirm and record in the pull request that the dependency is necessary for
behavior being implemented in the same change, is maintained, builds on both
release targets, carries an approved license, and comes from crates.io. Prefer the
standard library and existing workspace capabilities first; prefer a smaller
dependency surface over a framework that pulls in unrelated features.

## Reviewed decisions not yet in the lockfile

A gate resolution can settle which dependency a later change will need before
that change exists. Recording the review here means the change that finally adds
the crate arrives into a decision rather than reopening one, and that a reviewer
can see the license position was checked when the choice was made.

| Decision | Crates the implementation will need | License position | Recorded in |
|---|---|---|---|
| Asset archive container and manifest format | `zip` (default features **off**, `deflate-flate2` only), `flate2`, `sha2`, `serde_json` | Whole closure is MIT, Apache-2.0, or Zlib; all already allowed, no exception needed | [ADR 0001](adr/0001-asset-archive-container-and-safety-ceilings.md) |

Disabling the `zip` crate's default features is part of that decision, not a
tuning preference: the defaults pull in bzip2, LZMA, PPMd, XZ, Zstd, and AES
support for compression methods the archive contract does not accept, which
would add unreviewed parsers to a boundary that reads untrusted input.

The exact versions are pinned by the change that adds them, against the
lockfile and the advisory database as they stand at that time.

## Before adding a native dependency

Native dependencies carry deployment obligations that a Cargo check cannot see. A
change that adds, links, or bundles one must document, in the same change:

- the exact library or model version and its license, including any notice or
  attribution text that must ship with a release;
- whether MadoPilot bundles the artifact or consumes a controlled host-provided
  installation, and how the loading path is restricted rather than relying on an
  unrestricted ambient library search;
- the failure mode when the artifact is absent or unloadable, which must be an
  actionable status rather than a crash or an eager-link failure;
- the resulting minimum operating-system requirement and release-package size
  impact;
- for a model file, its source, hash, language coverage, preprocessing metadata,
  and license compatibility with redistribution.

Notices for bundled artifacts are collected in the release package. Native
dependency packaging and static-link feasibility remain unresolved decisions; see
gates `G-007` and `G-008` in [validation-gates.md](validation-gates.md).

## Verification

[../CONTRIBUTING.md](../CONTRIBUTING.md) records the full local verification
sequence. The dependency-policy step needs network access, because it fetches the
RustSec advisory database:

```sh
cargo deny --locked check
```

It reports the advisory, ban, license, and source checks together and returns a
non-zero status naming the package and the policy reason when any of them fails.
