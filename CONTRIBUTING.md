# Contributing

Read [docs/architecture.md](docs/architecture.md) before changing package
boundaries, dependency directions, public naming, or platform support. It is the
tracked baseline that this repository is verified against.

## Toolchain and lockfile

`rust-toolchain.toml` pins Rust 1.97.1 with `rustfmt` and `clippy`, so a clean
checkout with `rustup` available selects the tested compiler automatically. That
pin is also the workspace minimum supported Rust version.

The workspace keeps one committed lockfile at the root `Cargo.lock`; no workspace
member has its own lockfile. Run verification with `--locked` so that a check
fails instead of silently changing dependency resolution, and commit `Cargo.lock`
changes in the same pull request whenever a manifest requirement changes.

The independent [`examples/rust-input-workflow`](examples/rust-input-workflow/)
application is not a workspace member and owns its own `Cargo.lock`, toolchain
pin, and deployment configuration. See the
[standalone Rust input workflow guide](docs/rust-input-workflow.md) for the
external copy-and-verify procedure (`tools/check-external-rust-consumer.py`);
update its `Cargo.lock` alongside the workspace lockfile whenever product
dependencies change.

## Native development prerequisites

Developers acquire and maintain the native prerequisites; this source-only
project redistributes no native library or model. Building the workspace requires
an **OpenCV 4 development installation**, a loadable **libclang**, and the native
compiler/SDK:

| Host | Developer-owned installation |
|---|---|
| macOS (`aarch64-apple-darwin`) | Xcode Command Line Tools, Homebrew, and `brew install opencv@4` (not `opencv`). The Command Line Tools supply `libclang` and compile `mado-pilot-platform-macos` (`-fobjc-arc-exceptions`) without full Xcode; see [the macOS native boundary](docs/architecture.md#the-macos-native-boundary). |
| Windows (`x86_64-pc-windows-msvc`) | Official OpenCV 4.14.0 prebuilt archive, LLVM 22.1.8, and Visual Studio C++ build tools with Windows SDK 10.0.26100.0. Open an **x64 MSVC developer command prompt**. |

With **Python 3.13+**, validate existing installations and run commands in a
child-scoped environment:

```sh
# macOS: discover opencv@4 and the selected Xcode Command Line Tools
python3 tools/setup-native.py
python3 tools/setup-native.py -- cargo build --locked
```

```bat
rem Windows: OPENCV_ROOT contains build\include and build\x64\vc16\{lib,bin}
python tools/setup-native.py --opencv-root C:\native\opencv --libclang-path "C:\Program Files\LLVM\bin"
python tools/setup-native.py --opencv-root C:\native\opencv --libclang-path "C:\Program Files\LLVM\bin" -- cargo build --locked
```

`tools/setup-native.py [--opencv-root DIR] [--libclang-path DIR] [--github-env FILE --github-path FILE] [-- COMMAND ARG...]`
validates inputs and prints JSON when invoked without a command, or runs one
child command with the configured environment. Both root flags are required on
Windows and optional overrides on macOS. Key behavioral rules:

- **No ambient mutation or installation:** The script never downloads, installs,
  elevates, or modifies the calling shell. Wrap every OpenCV-dependent Cargo
  command with `tools/setup-native.py --`.
- **Link selection and loader paths:** macOS links only `opencv_core`,
  `opencv_imgproc`, and `opencv_imgcodecs` (not all of `opencv4.pc`), keeping
  inherited `DYLD_LIBRARY_PATH` entries after the selected OpenCV library
  directory. Windows links the versioned `opencv_world` import library and DLL
  and exports the selected MSVC include/library paths when `--github-env` and
  `--github-path` are supplied. GitHub environment export refuses additional
  inherited `DYLD_LIBRARY_PATH` entries on macOS; use `-- COMMAND` directly in
  that case.
- **Ownership and OS support boundaries:** Removing an eagerly linked OpenCV
  library after setup invalidates the environment; runtime ONNX Runtime/model
  canonical-path, length/hash, and platform capability checks remain mandatory.
  See [the OpenCV dependency policy](docs/third-party-dependencies.md#opencv) for
  license and link details,
  [native development prerequisite ownership](docs/architecture.md#native-development-prerequisite-ownership)
  for `G-007`/`G-012` scope, and the
  [OS support policy](docs/architecture.md#os-support-policy) for minimum
  deployment floors (`macOS 26.5.2`, `Windows 11 25H2 build 26200`) versus
  verified qualification hosts.

## Native and feature-specific verification

Open a topic pull request first so hosted CI validates compilation, architecture
contracts, ABI negotiation, and examples on both targets before reserving
interactive hardware. Because hosted runners do not hold macOS Screen Recording
or Accessibility grants, interactive foreground focus, or multi-display hardware
topologies, a green `cargo test` or hosted CI run is contract/compile evidence
rather than native capture, input, or topology qualification.

When running native or feature-specific verification on a dedicated host, always
use a fresh, purpose-specific `CARGO_TARGET_DIR` so ordinary builds and tests
never overwrite hash-pinned qualification binaries. Consult the canonical guide
for each area's prerequisites, opt-in commands, and revision-bound evidence:

| Verification area | What it requires and verifies | Canonical guide |
|---|---|---|
| macOS capture & non-prompting permissions | Four frame-callback boundary tests run without permissions (including under ASan). Live ScreenCaptureKit scenarios require **Screen Recording** granted to the test runner; without it, the preflight returns `NotAuthorized` before reaching the shareable-content query (which would otherwise abort without a WindowServer connection) and prints an explicit skip. | [Architecture: macOS native boundary](docs/architecture.md#the-macos-native-boundary) and [macOS owned-window replacement evidence](docs/evidence/g-001/macos-owned-window-replacement.md) |
| macOS native input (`System` & `ProcessDirected`) | Ordinary `cargo test` sends no macOS input. Opt-in `private-fixture` tests verify `CGPreflightPostEventAccess` (Accessibility), interactive `System` delivery, seven `ProcessDirected` background-delivery scenarios across signed fixture bundles and display topologies, owned-window replacement, and the one-read `RequireUnchanged` performance profile. | [macOS input verification](docs/macos-input-verification.md) |
| Windows native capture (`WGC`) | Uses the target-gated `windows` crate (WGC, D3D11, DXGI) with no external redistributable. Covers producer-pool lease isolation, resize, device removal, and single-display / dual-4K mixed-DPI capture evidence. | [Windows capture contract tests](docs/windows-capture-contract-tests.md) and [Architecture: Windows capture ownership](docs/architecture.md#windows-native-capture-ownership) |
| Windows native input (`System` & `WindowMessage`) | Ordinary `cargo test` starts `MadoPilotInputFixture` in no-activation mode and sends no `System` input. Opt-in tests cover the `WindowMessage` native matrix, interactive `System` delivery (without `AttachThreadInput` or elevation), and Audit 2/6/16 DPI, cleanup, and non-ASCII C consumer acceptance. | [Windows input verification](docs/windows-input-verification.md) |
| Native template-watch lanes (`Lane A`, `Lane B`, `Lane C`) | Separates hosted deterministic replay (`Lane A`, `template-watch-query`), required interactive WGC and ScreenCaptureKit contracts (`Lane B`, `native-template-watch --native-contract`), and optional statistical/resource evidence (`Lane C`, `--lane-c-evidence`). | [Native template watching](docs/native-template-watch.md#qualification-v2-protocol-and-migration) |
| Rust OCR text-watch & model catalog | Covers deterministic Unicode normalization and replay tests (`mado-pilot-ocr`, `mado-pilot-runtime`, `tools/ocr-text-watch/tests`), fixed real-CPU replay, target-owned native capture procedures, and frozen-cohort model evaluations (`tools/ocr-models/run.py`). | [OCR text queries](docs/ocr-text-watch.md) and [OCR model catalog](docs/ocr-model-catalog.md#repeatable-rust-measurements) |

## Verification

Run this 10-step sequence from the repository root before opening a pull request.
Steps are ordered from cheapest structural checks to full native verification;
no step modifies tracked files (`cargo fmt --all` without `--check` formats in
place). On Windows, replace `python3 tools/setup-native.py --` with
`python tools/setup-native.py --opencv-root DIR --libclang-path DIR --` in an
x64 MSVC developer prompt, and use the Windows shell forms for step 6. Steps 9
and 10 run on macOS hosts only.

```sh
# 1. Workspace architecture
cargo run --locked --package mado-pilot-dependency-check

# 2. Formatting
cargo fmt --all --check

# 3. Lints, with warnings promoted to failures
python3 tools/setup-native.py -- cargo clippy --locked --workspace --all-targets -- -D warnings

# 4. Tests
python3 tools/setup-native.py -- cargo test --locked --workspace --all-targets

# 5. Documentation examples (`--all-targets` excludes doctests)
python3 tools/setup-native.py -- cargo test --locked --workspace --doc

# 6. Documentation, with rustdoc warnings promoted to failures
RUSTDOCFLAGS="-D warnings" python3 tools/setup-native.py -- cargo doc --locked --workspace --no-deps

# 7. Dependency licenses, advisories, sources, and duplicate versions
cargo deny --locked check

# 8. Direct C/C++ samples, ABI 1.5, provider compilation, ownership, frozen
#    1.0/1.2/1.3/1.4 callers, and CMake consumers (including C-only without CXX).
#    Keep the private OCR fixture library separate from the ordinary artifact.
python3 tools/setup-native.py -- cargo build --locked --package mado-pilot-capi
python3 tools/setup-native.py -- cargo check --locked --package mado-pilot-capi --all-targets --all-features
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot --example ocr-fixture
python3 tools/setup-native.py -- cargo build --locked --package mado-pilot-capi --features private-fixture --target-dir target/c-abi-private-fixture
python3 tools/setup-native.py -- cargo run --locked --package mado-pilot-capi --features private-fixture --target-dir target/c-abi-private-fixture --example c-abi-check -- --label "<host>"

# 9. macOS host only: lint the macOS adapter as the Windows target sees it
cargo clippy --locked --target x86_64-pc-windows-msvc \
  -p mado-pilot-platform-macos --all-targets -- -D warnings

# 10. macOS host only: capture scenarios with the Objective-C shim under ASan
#     (requires Screen Recording granted; fails rather than skips without it)
MADO_PILOT_MACOS_ASAN=1 python3 tools/setup-native.py -- cargo test --locked \
  -p mado-pilot-platform-macos --target-dir target/asan --lib -- --test-threads=1
```

Operational requirements for specific steps:

- **Step 1 (`--release-scope` opt-in):** The separate `--release-scope` flag on
  `mado-pilot-dependency-check` validates the immutable `v0.4.0` source-release
  tree and is run only against that release checkout; never rewrite its frozen
  tree/blob identities for later branches.
- **Step 4 (Windows symbolic-link privilege):** Two tests in
  `mado-pilot-adapter-replay` verify that symlinked path components cannot escape
  a replay source. Because a skipped test proves nothing, a Windows host lacking
  symlink privilege fails with `Os { code: 1314 }` (`ERROR_PRIVILEGE_NOT_HELD`).
  Enable **Settings → System → For developers → Developer Mode** and open a new
  console (confirm `0x1` via
  `reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock" /v AllowDevelopmentWithoutDevLicense`).
  Never run verification from an elevated prompt: mixing elevated and ordinary
  file ownership inside `target/` breaks subsequent builds. Passing
  `-- --skip a_symlinked_` is an incomplete local workaround, not verification.
- **Step 6 (Windows `RUSTDOCFLAGS` syntax):** In `cmd` (Developer Command Prompt),
  quote the entire assignment and clear the variable afterward so it does not leak
  into later commands:
  ```bat
  set "RUSTDOCFLAGS=-D warnings"
  python tools/setup-native.py --opencv-root DIR --libclang-path DIR -- cargo doc --locked --workspace --no-deps
  set "RUSTDOCFLAGS="
  ```
  In PowerShell with the x64 MSVC environment imported:
  ```powershell
  $env:RUSTDOCFLAGS = "-D warnings"; python tools/setup-native.py --opencv-root DIR --libclang-path DIR -- cargo doc --locked --workspace --no-deps
  ```
- **Step 7 (`cargo-deny`):** Install with `cargo install --locked cargo-deny`
  (requires network access to fetch the RustSec advisory database). See
  [third-party dependencies](docs/third-party-dependencies.md) for the enforced
  license and source policy.
- **Step 8 (C/C++ consumers and CMake):** Requires a C compiler, C++ compiler, and
  CMake 3.22+ (`CC`/`CXX` are selected by `setup-native.py`; set `CMAKE` to
  override). On Windows, run from a Developer Command Prompt so `cl` and
  `VSINSTALLDIR` are present. See [the C ABI guide](docs/c-abi.md) for direct
  C/C++ consumption and frozen header compatibility rules.
- **Step 9 (Cross-target `cfg` lint on macOS):** On macOS, step 3 compiles the
  Windows adapter with its `#![cfg(target_os = "windows")]` items stripped, and
  step 9 checks the symmetric case for `mado-pilot-platform-macos` without running
  the `opencv` build script for another target. Every target-gated test file in
  both platform adapters must place `#![cfg_attr(not(target_os = "..."), allow(missing_docs))]`
  before `#![cfg(target_os = "...")]` so stripping the file does not leave the
  crate root undocumented.
- **Step 10 (macOS ASan library suite):** `MADO_PILOT_MACOS_ASAN=1` instruments
  the Objective-C shim with AddressSanitizer to catch native use-after-free bugs
  that object-count assertions cannot see. Every flag is load-bearing: build only
  `-p mado-pilot-platform-macos` (because `cargo::rustc-link-arg` does not
  propagate to downstream crates), isolate `--target-dir target/asan` (so steps
  3–8 never link instrumented archives), and run `--test-threads=1` (because ASan
  aborts on the first violation). Under `MADO_PILOT_MACOS_ASAN=1`, missing Screen
  Recording fails rather than skips. Window-subject scenarios still pass with a
  note when no matching active window exists on the desktop, so inspect test
  output notes when verifying window-capture changes.

### When a step contradicts the one before it

`cargo clippy` runs under `RUSTC_WORKSPACE_WRAPPER` with its own artifact
fingerprint, so a passing step 3 does not refresh metadata consumed by `cargo doc`
in step 6. If step 6 reports missing trait methods or API mismatches on source
that just passed steps 3 and 4, clean the stale crate metadata and rerun from
step 1:

```sh
cargo clean -p <the crates the error names>
```

## Documentation, decisions and tuning

Start at the [documentation index](docs/README.md); every current rule has one
purpose-based home. Update the affected guide in the same pull request as the
change, following
[documentation governance](docs/architecture.md#documentation-governance).

- **Decision records:** Write a short [ADR](docs/adr/README.md) only for a
  consequential architectural or technical choice (package boundaries, backend
  selection, public naming/compatibility, platform/toolchain policy, or security
  boundaries). Keep procedures, measurement tables, and run history in their
  canonical guides or evidence.
- **Benchmarks and tuning:** Focused, bounded measurements require no ADR or prior
  qualification campaign; see [performance](docs/performance.md) for measurement
  methods and profile rules. Changing an empirical target, sample count, or tuning
  parameter requires a concise [tuning record](docs/performance/tuning/README.md)
  that applies prospectively without regrading earlier runs.
- **Mandatory checks and claims:** Correctness, resource limits, ownership,
  privacy, cancellation, ABI compatibility, and required native semantics are
  enforced by their contracts and never relaxed by tuning records. Performance or
  support claims require matching evidence for their stated scope.
- **Historical immutability:** Evidence (`docs/evidence/`), release notes
  (`docs/releases/`), and benchmark profiles (`docs/benchmarks/`) keep their
  original bytes and verdicts. Pre-`646369f` references to `docs/adr/NNNN-*.md`
  are historical context only.

## Branch strategy

`main` contains released, production-ready code. Development for each release
is collected in a version branch named `dev/x.y.z`, where `x.y.z` is a semantic
version without a leading `v` (for example, `dev/0.5.1`).

Use short-lived topic branches for individual changes:

- `feat/<name>`, `fix/<name>`, `docs/<name>`, `refactor/<name>`, `test/<name>`,
  `chore/<name>`, `ci/<name>`, `build/<name>`, or `perf/<name>`

Branch names after the prefix must contain lowercase letters, numbers, `.`,
`_`, or `-`.

## Pull request flow

1. Create `dev/x.y.z` from `main` for the next release.
2. Create each topic branch from that `dev/x.y.z` branch.
3. Open a pull request from the topic branch into the same `dev/x.y.z` branch.
4. When the release is ready, open a pull request from `dev/x.y.z` into `main`.
5. After merging into `main`, tag the release as `vx.y.z`.

Emergency fixes follow the same flow through the next patch-version `dev/x.y.z`
branch. Pull requests directly from topic branches into `main`, or between
different `dev/*` branches, are rejected by `branch-policy.yml`. Group related
improvements into one coherent pull request per category with reviewable commits.

## Release checklist

A version branch is not a release by itself. Follow this order so the permanent
tag, public notes, and native verification identify one final source revision:

1. On a topic branch from `dev/x.y.z`, add or update `docs/releases/vx.y.z.md`
   and confirm that workspace version, public behavior, prerequisites,
   compatibility, artifact list, limitations, security boundaries, and retained
   evidence agree.
2. Run the complete [verification](#verification) sequence. Retained ABI, native,
   or benchmark evidence from an earlier revision requires a reviewed
   applicability record covering the complete intervening diff; otherwise rerun
   it on its named native host.
3. Merge the release-readiness topic branch to `dev/x.y.z` and wait for all
   required checks.
4. Open the release pull request from `dev/x.y.z` to `main`, verify its history,
   and wait for all required checks to pass.
5. Merge the release pull request, record the resulting `main` commit id, and
   verify `origin/main` history. Required Rust statuses come from the release PR
   checks in step 4.
6. Create and push an annotated `vx.y.z` tag at that verified `main` commit
   (never tag `dev/x.y.z` before merging).
7. Publish the release-provider record using `docs/releases/vx.y.z.md` verbatim,
   and verify the tag, body, source archives, and public URL.
8. Create the next `dev/x.y.z` branch from the released `main` commit before
   accepting new implementation.

Numerical performance qualification supports a specific published claim rather
than release eligibility by itself. A published version tag is immutable: never
move, delete, or reuse it; ship corrections in a later semantic version.

## Protected branches

Repository rulesets (tracked as reference exports in `.github/rulesets/*.json`)
protect `main` and `dev/*`: changes require a pull request, passing branch-policy
and status checks, and resolved review conversations; force pushes and `main`
deletion are blocked, while merged topic and `dev/*` branches auto-delete.

## Required status checks

Every pull request targeting `main` or `dev/**` runs these required checks
without a path filter:

| Check | Workflow | What it verifies |
|---|---|---|
| `Validate branch flow` | `branch-policy.yml` | Source and target branches follow the `topic -> dev/x.y.z -> main` flow. |
| `Repository policy` | `rust.yml` (`ubuntu-latest`) | Package inventory, dependency directions, formatting, and `cargo deny`. |
| `Windows x86_64-pc-windows-msvc` | `rust.yml` (`windows-2025`) | Aggregates six clean-build native lanes (`workspace`, `ocr`, `matching-normal`, `matching-instrumented`, `consumers`, `external`), including Lane A execution and Lane B/C compilation. |
| `macOS aarch64-apple-darwin` | `rust.yml` (`macos-26`) | Aggregates the same six clean-build native lanes on Apple Silicon without Screen Recording authority. |

CI execution rules:

- **Dev-push deduplication:** Pushes to `dev/**` run `Repository policy` and both
  native groups (`(dev push)` suffix) unless `Select dev push checks` (`pull-requests: read`)
  finds an open `dev/x.y.z -> main` release PR in this repository with the exact
  same head branch and commit. Any mismatch or API error leaves push checks
  enabled.
- **Clean-build lane isolation:** Each of the six native lanes runs on a fresh
  runner with a 30-minute safety timeout, keeps compiled outputs local to that
  job (no shared Cargo target cache or cross-job binary artifact download), and
  runs with `fail-fast: false` before reporting to its Ubuntu aggregation job.
  See the [verification baseline](docs/architecture.md#verification-baseline) for
  architectural verification boundaries.
- **Activating new checks:** Enable a check as a required ruleset status only
  after it has reported a successful run on the target branch and received
  maintainer authorization.
