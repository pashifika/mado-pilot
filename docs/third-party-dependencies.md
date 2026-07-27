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

## Wildcard version requirements

A wildcard version requirement on a registry crate accepts whatever is published
next, so `cargo deny check bans` rejects it.

Intra-workspace path dependencies are exempt, through
`allow-wildcard-paths` in `deny.toml`. They are not a supply-chain surface: each
one resolves to a tracked directory in this repository, and the architecture
checker independently rejects any path dependency that does not resolve to a
workspace member, or that carries a member's name while resolving from a registry
or Git source. Members inherit their version from `[workspace.package]`, so
writing a literal requirement on every internal edge would create a drift hazard
without adding a check.

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

## Product dependencies in use

| Crate | Used by | Why | License |
|---|---|---|---|
| `serde`, `serde_json` | `mado-pilot-adapter-replay`, `mado-pilot-assets` | Reads the replay sequence manifest and the asset package manifest. JSON is the manifest serialization [ADR 0001](adr/0001-asset-archive-container-and-safety-ceilings.md) chose, so a reader meets one format rather than two, and both crates were already resolved in the committed lockfile for the maintenance tool | MIT OR Apache-2.0 |
| `zip` 8.6 | `mado-pilot-assets` | Reads the ZIP central directory and decompresses entries. Default features **off**, `deflate-flate2` only | MIT |
| `flate2` 1.1 | `mado-pilot-assets` | Selects the DEFLATE backend `zip` decompresses with. Not called directly | MIT OR Apache-2.0 |
| `sha2` 0.11 | `mado-pilot-assets` | Verifies the content hash a manifest declares for each entry | MIT OR Apache-2.0 |
| `opencv` 0.99 | `mado-pilot-backend-opencv` | Binds the OpenCV C++ API the CPU matching profile uses. Default features **off**; `imgcodecs`, `imgproc`, and `clang-runtime` only | MIT |

A replay manifest is caller-supplied data. It is parsed into a typed schema that
rejects unknown fields, and the pixel paths it declares are validated the same
way an asset package's entry names are — relative, no traversal, no root, no
drive prefix, no symbolic link — because a source that could name any path would
be a file-read primitive wearing a capture adapter's clothes.

An asset archive is caller-supplied data of a stronger kind: its metadata is read
before anything is known about its content. Three consequences of that are
recorded here rather than left to the implementation.

Disabling the `zip` crate's default features is part of ADR 0001, not a tuning
preference. The defaults pull in bzip2, LZMA, PPMd, XZ, Zstd, and AES support for
compression methods the archive contract does not accept, which would add
unreviewed parsers to a boundary that reads untrusted input. A test that needed a
richer `zip` feature set would enable those parsers for the whole build graph, so
the test suite writes the archives it needs with its own stored-only writer
instead.

`zip`'s `deflate-flate2` feature declares the `flate2` dependency without
selecting a zlib backend, so the workspace selects one. `flate2`'s default
`rust_backend` is `miniz_oxide`, which keeps the archive reader free of a native
C library on both release targets and adds no build-time toolchain requirement.

The resolved closure adds `adler2`, `block-buffer`, `cfg-if`, `const-oid`,
`cpufeatures`, `crc32fast`, `crypto-common`, `digest`, `equivalent`,
`hashbrown`, `hybrid-array`, `indexmap`, `libc`, `miniz_oxide`, `simd-adler32`,
`typed-path`, and `typenum`. Every one offers MIT, Apache-2.0, Zlib, or 0BSD
terms, so no exception is needed and `cargo deny check licenses` passes without
one.

The `opencv` crate adds a build-time closure of its own: `autocfg`, `cc`,
`clang`, `clang-sys`, `dunce`, `find-msvc-tools`, `glob`, `jobserver`,
`libloading`, `num-traits`, `opencv-binding-generator`, `percent-encoding`,
`regex` with `aho-corasick`, `memchr`, `regex-automata`, and `regex-syntax`,
`semver`, `shlex`, `vcpkg`, `pkg-config`, and on Windows targets the `windows`
family through `windows-core`. All are MIT or Apache-2.0. Most of them are the
binding generator's, which runs at build time and ships in nothing.

The `opencv` feature selection is a review decision. Default features bind
thirteen OpenCV modules the matching profile never calls, and every one of them
becomes a module the host installation must provide: a build against an OpenCV
that omits `videoio` would fail for a package that does no video. Turning defaults
off and enabling `imgcodecs` and `imgproc` narrows both the generated bindings and
the installation requirement to what
[adr/0003-opencv-matching-profile-and-public-score.md](adr/0003-opencv-matching-profile-and-public-score.md)
uses. `clang-runtime` is separate and not optional in practice; the OpenCV section
below records why.

## Reviewed decisions not yet in the lockfile

A gate resolution can settle which dependency a later change will need before
that change exists. Recording the review here means the change that finally adds
the crate arrives into a decision rather than reopening one, and that a reviewer
can see the license position was checked when the choice was made.

| Decision | Crates the implementation will need | License position | Recorded in |
|---|---|---|---|
| _none_ | — | — | — |

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

## OpenCV

OpenCV is the project's first native dependency. Everything the checklist above
asks for is recorded here.

### Version and license

**OpenCV 4.14.0**, under **Apache-2.0** — the same licence as this project, so a
release that bundled it would add an attribution obligation and no term the root
`LICENSE` does not already carry. The Rust binding crate `opencv` 0.99.1 is MIT.

Both verification hosts run 4.14.0. That is deliberate rather than convenient: a
score measured on one host is only comparable with a score measured on the other
if the algorithm is the same, so a version difference between the hosts would make
the cross-target evidence in
[evidence/vision-opencv/](evidence/vision-opencv/) unreadable.

The adapter accepts major version 4 and reports
`VisionFault::BackendUnavailable` for anything else. The binding crate also
supports OpenCV 5, and Phase 1 does not, because nothing has been measured against
it.

### Bundled or host-provided

**Host-provided, and Phase 1 claims nothing about a release.** OpenCV is a
*development prerequisite*: what is documented is how to build and test this
repository, not what an installer would ship. `G-007` (bundling, deployment
profiles, notices) and `G-008` (static-link feasibility, controlled loading) are
both open, and no statement here should be read as settling either.

### Development installation

| Host | OpenCV | libclang |
|---|---|---|
| macOS `aarch64-apple-darwin` | `brew install opencv@4` | Xcode Command Line Tools |
| Windows `x86_64-pc-windows-msvc` | Official prebuilt release, extracted | LLVM 22.1.8 |

On macOS the formula is **`opencv@4`, not `opencv`**. Homebrew's unversioned
formula is OpenCV 5, which the adapter refuses as an unsupported major version,
so installing the obvious name produces a `BackendUnavailable` at run time rather
than a build failure.

No Homebrew LLVM is needed: the Command Line Tools' `libclang.dylib` is what the
binding generator loads, and that was measured rather than assumed.

`PKG_CONFIG_PATH` may or may not be needed, and the difference is worth knowing.
`opencv@4` is keg-only, but Homebrew still links it — including its
`opencv4.pc` — when no conflicting unversioned `opencv` is installed, which is
why `pkg-config --modversion opencv4` answers `4.14.0` on a host that never
configured anything. On a host that also has OpenCV 5 installed, that link belongs
to the other version or is absent. Setting it explicitly is therefore the
deterministic arrangement, and it is what CI does:

```sh
export PKG_CONFIG_PATH="$(brew --prefix opencv@4)/lib/pkgconfig"
```

On Windows the official prebuilt archive has no pkg-config or CMake metadata, so
the discovery variables are set explicitly at the user level. `<OPENCV_ROOT>` is
wherever the archive was extracted, and the toolset directory is `vc16`:

```text
OPENCV_INCLUDE_PATHS  = <OPENCV_ROOT>\build\include
OPENCV_LINK_PATHS     = <OPENCV_ROOT>\build\x64\vc16\lib
OPENCV_LINK_LIBS      = opencv_world4140
OPENCV_DISABLE_PROBES = pkg_config,cmake,vcpkg_cmake,vcpkg
PATH                 += <OPENCV_ROOT>\build\x64\vc16\bin
```

`OPENCV_LINK_LIBS` names the release import library. `opencv_world4140d.lib` is
the debug-CRT build, and Rust's MSVC target uses `/MD` even in a debug profile, so
the release library is correct for every profile this repository builds.

`OPENCV_DISABLE_PROBES` is what makes the discovery *controlled* rather than
ambient: without it the build script also tries pkg-config, CMake, and vcpkg, any
of which could silently find a different OpenCV than the one the variables name.

### libclang, and why `clang-runtime`

The `opencv` crate generates its bindings at build time and needs libclang to do
it. Its default arrangement links libclang dynamically, which on macOS resolves
`@rpath/libclang.dylib` when the build script *loads* — and the Command Line
Tools install no rpath entry, so the build script aborts before it runs. The
measured failure names every path the loader tried and none of them is the Command
Line Tools' library directory.

The `clang-runtime` feature makes the generator `dlopen` libclang through a
documented directory search that includes the Command Line Tools on macOS and
`LIBCLANG_PATH` and the standard install locations on Windows. It is enabled for
that reason, not as a preference, and it also removes any libclang version ceiling
from the build.

### Failure when the library is absent

**Phase 1 cannot make this an actionable status, and says so.** OpenCV is linked
dynamically at load time, so a missing or unloadable library stops the process
before any MadoPilot code runs — the eager-link failure the policy above asks
changes to avoid. `OpenCvBackend::new` probes the *runtime* version and refuses an
unsupported one with `VisionFault::BackendUnavailable`, which covers a wrong
version but cannot cover an absent one.

Turning absence into a status needs deferred or weak dynamic loading. That
belongs with `G-007`, whose resolution includes the controlled library search
paths, and not with `G-008`, which is about whether a static library is feasible.
Recording the gap here is the honest position: `docs/architecture.md` requires a
loadable unsupported capability to fail with a clear status, and Phase 1 satisfies
that for a version mismatch only.

### Operating-system requirement and package size

No new minimum operating-system version, because nothing is bundled and the
adapter calls no OpenCV API newer than 4.x. No release-package size impact for the
same reason. Both become real questions at `G-007`, where the bundled artifact set
is chosen: `opencv_world4140.dll` alone is tens of megabytes, which is the
material fact that decision will have to weigh.

## Verification

[../CONTRIBUTING.md](../CONTRIBUTING.md) records the full local verification
sequence. The dependency-policy step needs network access, because it fetches the
RustSec advisory database:

```sh
cargo deny --locked check
```

It reports the advisory, ban, license, and source checks together and returns a
non-zero status naming the package and the policy reason when any of them fails.
