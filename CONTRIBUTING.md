# Contributing

Read [docs/architecture.md](docs/architecture.md) before changing package
boundaries, dependency directions, public naming, or platform support. It is the
tracked baseline that this repository is verified against.

## Toolchain and lockfile

`rust-toolchain.toml` pins Rust 1.97.1 with `rustfmt` and `clippy`, so a clean
checkout with `rustup` available selects the tested compiler automatically. That
pin is also the workspace minimum supported Rust version.

The workspace keeps one lockfile, the root `Cargo.lock`, and it is committed. No
member has its own lockfile. Run verification with `--locked` so that a check
fails instead of silently changing dependency resolution, and commit the lockfile
change in the same pull request whenever a manifest requirement changes.

## Native development prerequisites

Building the workspace needs an **OpenCV 4 development installation** and a
**libclang** the binding generator can load. `mado-pilot-backend-opencv` generates
its bindings at build time, so this is a prerequisite for `cargo build`, not only
for running the matching tests.

| Host | Install |
|---|---|
| macOS | `brew install opencv@4`. Xcode Command Line Tools supply libclang; no Homebrew LLVM and no `PKG_CONFIG_PATH` are needed. |
| Windows | Extract the official OpenCV prebuilt release, install LLVM, and set the discovery variables. |

[docs/third-party-dependencies.md](docs/third-party-dependencies.md) records the
exact versions, the Windows environment variables, why the discovery is restricted
rather than ambient, and what fails when the library is absent. This is a
development prerequisite only: the v0.1.0 source release bundles no native
dependency and makes no installable deployment-profile claim, which remains gate
`G-007`.

The macOS native shim adds no prerequisite beyond that. The production shim in
`mado-pilot-platform-macos` compiles, links, and passes its tests with the **Xcode
Command Line Tools alone**, on a host where full Xcode is not installed; its only
Cargo addition is `cc`, as a build dependency gated on macOS. That confirms on the
finished adapter what the `G-003` prototype had suggested on the same setup, and the
measurements are in [docs/evidence/g-003/](docs/evidence/g-003/README.md). Full Xcode
is still not evaluated, so this remains a positive result about the smaller
installation rather than a statement that the two are interchangeable. The shim is
compiled with `-fobjc-arc-exceptions`, which
[docs/adr/0012-macos-shim-language-and-containment.md](docs/adr/0012-macos-shim-language-and-containment.md)
records as a correctness requirement rather than a style choice: without it, an
exception unwinding out of a scope that holds a native object leaks it.

Running that adapter's capture scenarios needs one thing the build does not:
**Screen Recording granted to the process running the tests**. MadoPilot never
prompts, so on a host that has neither granted nor denied it the scenarios reach the
non-prompting refusal and print a skip naming that reason instead of passing. A green
`cargo test` on such a host — including a continuous-integration runner — is
therefore not evidence that macOS capture ran. To exercise them, grant Screen
Recording to the terminal or editor that launches `cargo test`, under System Settings
▸ Privacy & Security ▸ Screen & System Audio Recording, and restart it so the new
grant applies.

The Windows capture adapter adds no prerequisite beyond that environment. The
production adapter uses the target-gated `windows` crate for Windows Graphics
Capture, Direct3D 11, and DXGI, and needs no NuGet package, Windows App SDK,
vcpkg, vendored sample, or redistributable. The preceding `G-002` prototype used
the same native stack with the MSVC toolchain, a Windows SDK, and CMake; its
review is in
[docs/evidence/g-002/dependency-review.md](docs/evidence/g-002/dependency-review.md).
[docs/adr/0013-windows-capture-frame-detachment.md](docs/adr/0013-windows-capture-frame-detachment.md)
records the ownership decision that the production implementation follows.

## Verification

Run this sequence from the repository root before opening a pull request. The
steps are ordered so that the cheapest structural failure is reported first, and
each step returns a non-zero status with an actionable diagnostic when its policy
is violated.

```sh
# 1. Workspace package inventory and dependency directions
cargo run --locked --package mado-pilot-dependency-check

# 2. Formatting
cargo fmt --all --check

# 3. Lints, with warnings promoted to failures
cargo clippy --locked --workspace --all-targets -- -D warnings

# 4. Tests
cargo test --locked --workspace --all-targets

# 5. Documentation examples. `--all-targets` above deliberately excludes doctests,
#    so they need their own run.
cargo test --locked --workspace --doc

# 6. Documentation, with rustdoc warnings promoted to failures
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps

# 7. Dependency licenses, advisories, sources, and duplicate versions
cargo deny --locked check

# 8. The C and C++ surfaces: the header against the Rust definitions, both
#    examples against the built library, the C++ ownership probe, and the CMake
#    consumer project. Only run natively on a release target.
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

Step 4 needs one thing of a Windows host: the privilege to create a symbolic
link. Two tests in `mado-pilot-adapter-replay` prove that a linked path component
cannot reach outside a replay source, and they are the only coverage of that
rule. A host that cannot create the link has proven nothing, so they fail rather
than return early — a skipped test and a passing one are the same line of output,
and a green suite has to mean the case ran. The failure names the requirement and
carries `Os { code: 1314 }`, which is `ERROR_PRIVILEGE_NOT_HELD`.

Turn on **Settings → System → For developers → Developer Mode**. It takes effect
in a console opened afterwards, with no reboot, and `cmd` confirms it:

```bat
reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock" /v AllowDevelopmentWithoutDevLicense
```

`0x1` is on. An elevated prompt also holds the privilege, but do not verify from
one: `cargo` writes `target/` as whoever ran it, and mixing elevated and ordinary
runs in one working copy produces permission failures later that look like
anything but their cause.

Running with `-- --skip a_symlinked_` gets the rest of the suite through on a host
without the privilege. That run is not a verification: it is exactly the silent
gap the two tests were changed to expose, so record it as what it is.

Step 8 is the only check in this repository that is not `cargo` alone. It needs a
C compiler, a C++ compiler, and CMake 3.22 or later. On both release targets the
compilers are the ones the platform already has — MSVC on Windows, the Xcode
Command Line Tools on macOS — and both CI runners and both verification hosts
already have a CMake. Set `CC`, `CXX`, or `CMAKE` to choose a different one.

[docs/c-abi.md](docs/c-abi.md) records what it compiles and why the header is
verified this way rather than generated;
[docs/cpp-wrapper.md](docs/cpp-wrapper.md) records the C++ half.

On Windows, run step 8 from a Developer Command Prompt. `cl` is not on `PATH`
otherwise, and the same environment sets `VSINSTALLDIR`, through which the check
finds the CMake that Visual Studio ships when none is on `PATH`.

Step 6 sets an environment variable, which each Windows shell spells its own way.
The Developer Command Prompt the paragraph above asks for is `cmd`, so that form
comes first:

```bat
set "RUSTDOCFLAGS=-D warnings"
cargo doc --locked --workspace --no-deps
set "RUSTDOCFLAGS="
```

Quote the whole assignment. `set RUSTDOCFLAGS="-D warnings"` puts the quotation
marks *inside* the value, and rustdoc is then passed an argument it does not
recognize. The third line clears it again: `set` outlives the command, and a
`RUSTDOCFLAGS` left behind applies to every later `cargo doc` in that window.

On Windows PowerShell, step 6 is:

```powershell
$env:RUSTDOCFLAGS = "-D warnings"; cargo doc --locked --workspace --no-deps
```

Step 7 needs `cargo-deny`, which is not part of the toolchain, and needs network
access because it fetches the RustSec advisory database:

```sh
cargo install --locked cargo-deny
```

No step modifies a tracked file. `cargo fmt --all` without `--check` applies the
formatting that step 2 verifies.

[docs/third-party-dependencies.md](docs/third-party-dependencies.md) records the
dependency policy that step 7 enforces, including the review required before a
native library or model file is added.

### When a step contradicts the one before it

A build cache can go stale in a way that makes one step disagree with another,
and the disagreement is the symptom worth recognising rather than debugging.
Observed on the Windows verification host: step 6 reported that a trait was
missing methods its callers use, on a working tree where step 3 and step 4 had
both just passed over the same source. `cargo doc` had checked the dependents
against metadata for two crates that was thirteen commits old.

`cargo clippy` runs through `RUSTC_WORKSPACE_WRAPPER` and writes its check
artifacts under a fingerprint of its own, so a green step 3 does not refresh what
step 6 consumes. When one step reports an API that the source plainly does not
have, do not read the error: rebuild what it read.

```sh
cargo clean -p <the crates the error names>
```

Then re-run from step 1. A cache that was wrong once about one artifact kind was
not necessarily right about the others, and a verification is worth only as much
as the freshness of what it compiled — so if a run needed this, say so alongside
its result, or `cargo clean` and take the run again.

## Branch strategy

`main` contains released, production-ready code. Development for each release
is collected in a version branch named `dev/x.y.z`, where `x.y.z` is a semantic
version without a leading `v` (for example, `dev/0.2.0`).

Use short-lived topic branches for individual changes:

- `feat/<name>` for features
- `fix/<name>` for bug fixes
- `docs/<name>` for documentation
- `refactor/<name>` for refactoring
- `test/<name>` for tests
- `chore/<name>` for maintenance
- `ci/<name>` for CI changes
- `build/<name>` for build changes
- `perf/<name>` for performance changes

Branch names after the prefix must contain lowercase letters, numbers, `.`,
`_`, or `-`.

## Pull request flow

1. Create `dev/x.y.z` from `main` for the next release.
2. Create each topic branch from that `dev/x.y.z` branch.
3. Open a pull request from the topic branch into the same `dev/x.y.z` branch.
4. When the release is ready, open a pull request from `dev/x.y.z` into `main`.
5. After merging into `main`, tag the release as `vx.y.z`.

Emergency fixes follow the same flow: create the next patch-version
`dev/x.y.z` branch from `main`, then merge a `fix/<name>` branch into it.

Pull requests directly from topic branches into `main`, or from one version
development branch into another, are rejected by the branch policy check.

## Release checklist

A version branch is not a release by itself. Use this order so that the permanent
tag, public notes, and native verification identify one final source revision:

1. On a short-lived topic branch from `dev/x.y.z`, add or update the canonical
   `docs/releases/vx.y.z.md` notes. Confirm that the workspace version, public
   behavior, prerequisites, compatibility, artifact list, limitations, security
   boundaries, and retained evidence all agree.
2. Run the complete [verification](#verification) sequence. Retained benchmark
   or ABI evidence from an earlier revision needs a reviewed applicability record
   covering the complete intervening diff; otherwise rerun it on its named native
   host.
3. Merge the release-readiness topic branch to `dev/x.y.z` through its protected
   pull request and wait for every required check.
4. Open the release pull request from `dev/x.y.z` to `main`. Verify that its head
   contains only the intended release history and that its checks pass.
5. Merge the release pull request, record the full resulting `main` commit id, and
   wait for the `main` push runs of Repository policy,
   Windows `x86_64-pc-windows-msvc`, and macOS
   `aarch64-apple-darwin` to pass on that exact commit.
6. Create an annotated `vx.y.z` tag at that verified `main` commit and push that
   tag. Never tag the version-branch head before the release merge.
7. Publish the release-provider record using the tracked
   `docs/releases/vx.y.z.md` file verbatim as its body. Verify the tag, body,
   source archives, and public URL.
8. Create the next `dev/x.y.z` branch from the released `main` commit before
   accepting implementation for that version.

A published version tag is immutable: never move, delete, or reuse it to replace
a release. Correct a source or documentation defect in a later semantic version.
A release-provider metadata correction may clarify the record only when it does
not change what the tagged source contains or claims.

## Protected branches

The repository rulesets protect `main` and all `dev/*` branches:

- Changes must be submitted through a pull request.
- The branch policy check must pass.
- Review conversations must be resolved.
- Merge commits, squash merging, and rebasing are allowed.
- Force pushes are blocked.

Deletion is blocked for `main`. Merged topic and `dev/*` branches are deleted
automatically; deletion remains allowed for `dev/*` so completed release
branches do not accumulate.

The required approval count is currently zero because a pull request author
cannot approve their own pull request. Increase it to one or more when another
maintainer is available to review changes.

## Required status checks

`.github/rulesets/*.json` are tracked exports that describe the intended state of
the branch rulesets. They are not applied automatically, so editing them does not
change the live repository configuration.

The stable check names are:

| Check | Workflow | What it verifies |
|---|---|---|
| `Validate branch flow` | `branch-policy.yml` | The source and target branches follow the pull request flow. |
| `Repository policy` | `rust.yml` | Package inventory, dependency directions, formatting, and dependency policy. |
| `Windows x86_64-pc-windows-msvc` | `rust.yml` | Native `windows-2025` inventory, lint, test, doctest, and documentation checks against the committed lockfile, and step 8's C ABI and C++ wrapper check. |
| `macOS aarch64-apple-darwin` | `rust.yml` | Native `macos-15` Apple Silicon inventory, lint, test, doctest, and documentation checks against the committed lockfile, and step 8's C ABI and C++ wrapper check. |

The `Repository policy` job builds no product package, which is why
documentation, lints, tests, and the C and C++ boundary are verified only in the
two native jobs. Building any product package needs OpenCV and a loadable
libclang, because `mado-pilot-backend-opencv` generates its bindings at build
time, and that installation exists on the two release targets rather than on a
host that is neither. Of the verification sequence above, steps 3 through 6 and
step 8 therefore run only in the two native jobs, steps 2 and 7 run only in the
repository-policy job, and step 1 runs in all three.

A check is activated as a live required status only after that check has produced
its first successful run on the branch it will guard, and only with separate
maintainer authorization. Enabling a required check that has never reported turns
every open pull request into a blocked pull request.

The native jobs name `windows-2025` and `macos-15` rather than a `-latest` label, so
moving to a new operating-system version is a reviewed change. Those labels still
pin only an OS version: GitHub migrates the images behind them on its own schedule,
so the toolchain and the exact image contents are not frozen by the label. The
`Repository policy` job deliberately uses the moving `ubuntu-latest`, because it
verifies host-independent policy rather than a release target.
