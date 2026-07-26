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

# 5. Documentation, with rustdoc warnings promoted to failures
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps

# 6. Dependency licenses, advisories, sources, and duplicate versions
cargo deny --locked check
```

On Windows PowerShell, step 5 is:

```powershell
$env:RUSTDOCFLAGS = "-D warnings"; cargo doc --locked --workspace --no-deps
```

Step 6 needs `cargo-deny`, which is not part of the toolchain:

```sh
cargo install --locked cargo-deny
```

No step modifies a tracked file. `cargo fmt --all` without `--check` applies the
formatting that step 2 verifies.

[docs/third-party-dependencies.md](docs/third-party-dependencies.md) records the
dependency policy that step 6 enforces, including the review required before a
native library or model file is added.

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
| `Repository policy` | `rust.yml` | Package inventory, dependency directions, formatting, documentation, and dependency policy. |
| `Windows x86_64-pc-windows-msvc` | `rust.yml` | Native `windows-2025` inventory, lint, test, and documentation checks against the committed lockfile. |
| `macOS aarch64-apple-darwin` | `rust.yml` | Native `macos-15` Apple Silicon inventory, lint, test, and documentation checks against the committed lockfile. |

A check is activated as a live required status only after that check has produced
its first successful run on the branch it will guard, and only with separate
maintainer authorization. Enabling a required check that has never reported turns
every open pull request into a blocked pull request.

Runner labels are pinned to `windows-2025` and `macos-15` so that a hosted-runner
migration is a reviewed change rather than a silent environment change.
