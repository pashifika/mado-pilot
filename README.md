# MadoPilot

A headless visual automation runtime for applications and agents.

When implemented, MadoPilot will discover windows and displays, capture frame
streams, map coordinate spaces, match templates, recognize text, wait for visual
conditions, inject input through explicit platform capabilities, and report
structured diagnostics — without owning a GUI, tray, overlay, editor, updater,
scheduler, or scripting language of its own.

## Status: one deterministic workflow plus a direct Windows capture adapter

**MadoPilot is not usable for real automation yet.** What works end to end is a
deterministic workflow over *replayed* frames: configure a replay source and
require the OpenCV CPU backend, discover and open a target, take a frame, map
it, load an asset package, prepare a template, find it in that exact frame, and
close. That runs on both release targets, from Rust, from C, and from C++, and
the three examples answer the same questions with the same numbers. A directly
consumable Rust Windows adapter also implements picker-free native window and
display capture, but facade wiring and its release-acceptance matrix remain
later work:

```text
crates/mado-pilot/examples/deterministic-slice.rs
crates/bindings/capi/examples/c/deterministic-slice.c
crates/bindings/capi/examples/cpp/deterministic-slice.cpp
```

Nothing recognizes text, waits on a condition, or injects input. Adding a
package here is not a claim that its behavior exists.

| Area | Status |
|---|---|
| Workspace, package boundaries, dependency enforcement | Implemented |
| Toolchain pin, lint, formatting, and dependency policy | Implemented |
| Architecture baseline, validation gates, benchmark format, ADR process | Implemented |
| Identities, geometry, coordinate transforms, deadlines, cancellation, statuses | Implemented |
| Capture contracts, immutable frames, CPU mapping, deterministic replay | Implemented |
| Asset manifests, directory/memory/archive loading, archive safety ceilings | Implemented |
| Template-matching contracts, ordering, suppression, source correlation | Implemented |
| Template matching against a real image | Implemented on OpenCV 4 for the Phase 1 profile |
| Deterministic Rust workflow: discovery, capture, mapping, assets, matching, close | Implemented over replay input |
| Native capture | Implemented in the direct Windows Rust adapter; facade wiring, release acceptance, and macOS capture remain open |
| OCR, watchers, input | Not implemented |
| C ABI, tracked C header, dynamic library | Implemented for the Phase 1 prefix |
| Header-only C++ RAII wrapper and CMake targets | Implemented for the Phase 1 prefix |
| C ABI static library, ABI-major loader names, pkg-config, CMake install | Not implemented |
| Numeric performance budgets for the Phase 1 workloads | Set on both release targets; see [ADR 0008](docs/adr/0008-phase-1-performance-budgets.md) |
| Release packaging | Not implemented |

The public Rust names have been reviewed and settled
([`G-009`](docs/validation-gates.md#g-009), resolved by
[ADR 0006](docs/adr/0006-public-rust-names-and-compatibility-policy.md)), but
they are not yet a stability promise: that begins at 1.0, and until then a
rename costs an ADR and a version bump rather than being impossible. The C ABI
is separately versioned and **is** frozen, at ABI 1.0
([`G-010`](docs/validation-gates.md#g-010), resolved by
[ADR 0007](docs/adr/0007-phase-1-c-abi-freeze.md)): within ABI major 1 no value
changes, no field moves, and no function-table entry moves. The C++ wrapper
declares no ABI of its own and inherits the C one.

[docs/architecture.md](docs/architecture.md) is the tracked baseline and records
the full status table, the package inventory, and the dependency rules.

## Releases

[`v0.1.0`](docs/releases/v0.1.0.md) is the first developer-facing source, Rust
API, C ABI 1.0, and C++ API baseline for the deterministic workflow above. It
does not publish crates to crates.io or provide prebuilt libraries, installers,
CMake install/export metadata, pkg-config metadata, or bundled OpenCV. The
release notes are the canonical public scope and are used verbatim as the
release-provider body.

## Release targets

Version one targets `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. Each is
verified natively in continuous integration; a cross-compiled result never stands
in for the other target. The exact minimum supported Windows and macOS versions
are still unresolved, along with thirteen other version-one decisions recorded in
[docs/validation-gates.md](docs/validation-gates.md).

## Integration surfaces

Three surfaces are planned: an idiomatic Rust API through the `mado-pilot` facade
package, a separately versioned C ABI, and a thin C++ RAII wrapper that consumes
only the released C ABI. The public names are reserved so that the language
surfaces stay consistent.

| Artifact | Name | State |
|---|---|---|
| Rust facade package | `mado-pilot` | Exists |
| Rust import | `mado_pilot` | Exists |
| C header | `include/madopilot/madopilot.h` | Exists, tracked and hand-written |
| C++ header | `include/madopilot/madopilot.hpp` | Exists, header-only |
| C symbol prefix | `madopilot_` | Exists — `madopilot_get_api` is the one exported symbol |
| C++ namespace | `madopilot` | Exists |
| CMake package and targets | `MadoPilot`, `MadoPilot::C`, `MadoPilot::Cpp` | Exist, for development-tree consumption |
| Windows ABI-major library | `madopilot-1.dll`, `madopilot-1.lib` | Reserved; release packaging is not implemented |
| macOS ABI-major install name | `libmadopilot.1.dylib` | Reserved; release packaging is not implemented |
| pkg-config package | `madopilot-1` | Reserved; not generated |

[docs/c-abi.md](docs/c-abi.md) and [docs/cpp-wrapper.md](docs/cpp-wrapper.md) are
the two boundaries' contract documents: handle lifetimes, ownership rules,
borrowed views, structure-prefix negotiation, and how to build against each.

## Repository layout

```text
crates/mado-pilot          public Rust facade
crates/automation/*        platform-neutral contracts and orchestration
crates/adapter/replay      deterministic replay capture from files and memory
crates/platform/*          Windows and macOS capture and input adapters
crates/backend/*           OpenCV and ONNX Runtime adapters
crates/bindings/capi       C ABI boundary, C++ wrapper, and CMake targets
crates/support/testkit     deterministic test support
tools/dependency-check     workspace inventory and dependency-direction checker
docs/                      architecture, gates, performance format, ADRs, policy
fixtures/                  tracked replay sequences and asset packages
```

## Building and verifying

The repository pins Rust 1.97.1 through `rust-toolchain.toml`, so a clean checkout
with `rustup` available selects the tested compiler. The single committed root
`Cargo.lock` is used with `--locked`.

Building also needs an **OpenCV 4 development installation** and a **libclang**,
because the OpenCV matching adapter generates its bindings at build time:
`brew install opencv@4` on macOS, or the official prebuilt release plus LLVM on
Windows. This is a development prerequisite and not a statement about what a
release ships; the exact versions, the Windows discovery variables, and the failure
modes are in
[docs/third-party-dependencies.md](docs/third-party-dependencies.md#opencv).

```sh
cargo build --locked --workspace
cargo test --locked --workspace --all-targets
```

The deterministic workflow is runnable, and running it is the shortest check
that this host's OpenCV, the replay adapter, and the asset loader all agree:

```sh
cargo run --locked --package mado-pilot --example deterministic-slice
```

The full verification sequence — architecture check, formatting, lints,
tests, documentation, and dependency policy — is in
[CONTRIBUTING.md](CONTRIBUTING.md#verification). The architecture check is the one
that is specific to this project:

```sh
cargo run --locked --package mado-pilot-dependency-check
```

It prints the resolved workspace inventory and fails when a package is missing,
unexpected, misplaced, deferred, or connected against the documented dependency
directions.

The C and C++ boundaries are checked by a step of their own, because `cargo test`
cannot compile them. It needs a C and C++ toolchain and CMake, and it compares the
measured ABI layout against the committed evidence, compiles the frozen v1 header
against the current library, and builds and runs both consumer programs:

```sh
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

## Documentation

| Document | Contents |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Workspace, package responsibilities, dependency allowlist, naming, scope, status |
| [docs/c-abi.md](docs/c-abi.md) | The C boundary's contract: handles, structure prefixes, statuses, panic containment, building against it |
| [docs/cpp-wrapper.md](docs/cpp-wrapper.md) | The C++ adapter's contract: move-only owners, `Result`, borrowed views, the CMake targets |
| [docs/validation-gates.md](docs/validation-gates.md) | The `G-001`–`G-014` registry of unresolved version-one decisions |
| [docs/performance.md](docs/performance.md) | Benchmark profile and budget format, the Phase 1 workloads, and their correctness oracles |
| [docs/third-party-dependencies.md](docs/third-party-dependencies.md) | Dependency license, source, advisory, and native-deployment policy |
| [docs/releases/](docs/releases/) | Canonical release notes and exact artifact limitations |
| [docs/adr/](docs/adr/) | Architecture decision records, and [the template](docs/adr/0000-template.md) with the rule for when one is required |
| [docs/evidence/](docs/evidence/) | The measurements behind decisions that rest on them |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Branch strategy, pull request flow, verification sequence, required checks |

## Security and privacy

Screen capture and input injection are sensitive capabilities, so the project
commits up front to how it will treat them: no implicit network access, no
automatic privilege escalation, no hidden permission behavior, and ordinary logs
and diagnostics that exclude captured images, recognized text, and credentials by
default. On macOS, permission state will be probed and reported without presenting
permission UI.

These are contract commitments for the implementing changes. Phase 1 still
requests no permission and probes none: it captures from tracked replay
sequences and files on disk, not from the screen, and injects no input. The
commitments above become testable with the platform adapters.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md) for the branch strategy, the pull
request flow, and the verification sequence, then read
[docs/architecture.md](docs/architecture.md) before changing package boundaries,
dependency directions, public naming, or platform support.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE). Every
Cargo package in the workspace declares `Apache-2.0` to match.
