# MadoPilot

A headless visual automation runtime for applications and agents.

When implemented, MadoPilot will discover windows and displays, capture frame
streams, map coordinate spaces, match templates, recognize text, wait for visual
conditions, inject input through explicit platform capabilities, and report
structured diagnostics — without owning a GUI, tray, overlay, editor, updater,
scheduler, or scripting language of its own.

## Status: contracts and two subsystems, no end-to-end workflow

**MadoPilot is not usable yet.** On top of the repository baseline — the Cargo
workspace, package boundaries and their enforcement, the toolchain pin, quality
and dependency policy, the architecture baseline, and continuous integration for
both release targets — it contains the platform-neutral vocabulary the rest of
the system is built from, the capture contracts with a deterministic replay
adapter behind them, and validated asset package loading.

That is a foundation, not a feature. Nothing captures a real window, matches,
recognizes, waits, or injects input. The public Rust facade and the C ABI package
exist as seams and expose no operation, no exported symbol, and no generated
header. Adding a package here is not a claim that its behavior exists.

| Area | Status |
|---|---|
| Workspace, package boundaries, dependency enforcement | Implemented |
| Toolchain pin, lint, formatting, and dependency policy | Implemented |
| Architecture baseline, validation gates, benchmark format, ADR process | Implemented |
| Identities, geometry, coordinate transforms, deadlines, cancellation, statuses | Implemented |
| Capture contracts, immutable frames, CPU mapping, deterministic replay | Implemented |
| Asset manifests, directory/memory/archive loading, archive safety ceilings | Implemented |
| Template-matching contracts, ordering, suppression, source correlation | Implemented |
| Native capture, OCR, watchers, input | Not implemented |
| Template matching against a real image | Not implemented; no backend yet |
| Public Rust operations, C ABI, C header, C++ wrapper | Not implemented |
| Numeric performance budgets, release packaging | Not established |

[docs/architecture.md](docs/architecture.md) is the tracked baseline and records
the full status table, the package inventory, and the dependency rules.

## Release targets

Version one targets `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. Each is
verified natively in continuous integration; a cross-compiled result never stands
in for the other target. The exact minimum supported Windows and macOS versions
are still unresolved, along with thirteen other version-one decisions recorded in
[docs/validation-gates.md](docs/validation-gates.md).

## Integration surfaces

Three surfaces are planned: an idiomatic Rust API through the `mado-pilot` facade
package, a separately versioned C ABI, and a thin C++ RAII wrapper that consumes
only the released C ABI.

The public names are reserved so that the language surfaces stay consistent.
**These are reservations; this repository produces none of these artifacts yet.**

| Artifact | Name |
|---|---|
| Rust facade package | `mado-pilot` |
| Rust import | `mado_pilot` |
| C header | `include/madopilot/madopilot.h` |
| C++ header | `include/madopilot/madopilot.hpp` |
| C symbol prefix | `madopilot_` |
| C++ namespace | `madopilot` |
| Windows ABI-major library | `madopilot-1.dll`, `madopilot-1.lib` |
| macOS ABI-major install name | `libmadopilot.1.dylib` |
| CMake package and targets | `MadoPilot`, `MadoPilot::C`, `MadoPilot::Cpp` |
| pkg-config package | `madopilot-1` |

## Repository layout

```text
crates/mado-pilot          public Rust facade
crates/automation/*        platform-neutral contracts and orchestration
crates/platform/*          Windows and macOS capture and input adapters
crates/backend/*           OpenCV and ONNX Runtime adapters
crates/bindings/capi       C ABI boundary
crates/support/testkit     deterministic test support
tools/dependency-check     workspace inventory and dependency-direction checker
docs/                      architecture, gates, performance format, ADRs, policy
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

## Documentation

| Document | Contents |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Workspace, package responsibilities, dependency allowlist, naming, scope, status |
| [docs/validation-gates.md](docs/validation-gates.md) | The `G-001`–`G-014` registry of unresolved version-one decisions |
| [docs/performance.md](docs/performance.md) | Benchmark profile and budget format, with a synthetic example |
| [docs/third-party-dependencies.md](docs/third-party-dependencies.md) | Dependency license, source, advisory, and native-deployment policy |
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

These are contract commitments for the implementing changes. Phase 0 requests no
permission, probes none, and logs nothing, because it performs no capture or
input.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md) for the branch strategy, the pull
request flow, and the verification sequence, then read
[docs/architecture.md](docs/architecture.md) before changing package boundaries,
dependency directions, public naming, or platform support.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE). Every
Cargo package in the workspace declares `Apache-2.0` to match.
