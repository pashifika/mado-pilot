# MadoPilot

A headless visual automation runtime for applications and agents.

MadoPilot discovers windows and displays, captures frame streams, maps coordinate
spaces, matches templates, performs one-shot OCR through an explicit backend,
the accepted CPU profiles, or an explicit initialization-time provider policy,
waits for stable template presence through a bounded Rust query over replay or
qualified native sessions, and injects input through explicit platform
capabilities while reporting structured outcomes. OCR watchers remain future
work. The runtime owns no GUI, tray, overlay, editor, updater, workflow catalog,
general workflow/cron scheduler, or scripting language.

## Status: deterministic, native, OCR, and Rust watcher workflows

**MadoPilot is a developer-facing source runtime, not a packaged automation
product.** One complete deterministic workflow runs over replayed frames:
configure a replay source and require the OpenCV CPU backend, discover and open a
target, take a frame, map it, load an asset package, prepare a template, find it
in that exact frame, and close. That runs on both release targets from Rust, C,
and C++, and the three examples answer the same questions with the same numbers:

```text
crates/mado-pilot/examples/deterministic-slice.rs
crates/bindings/capi/examples/c/deterministic-slice.c
crates/bindings/capi/examples/cpp/deterministic-slice.cpp
```

Rust replay/OpenCV also exposes one bounded template-presence query with
non-blocking poll, a blocking wait under separate caller authority, explicit
cancel, confirmed-only stability, and immutable exact-frame results. Its
deterministic replay/OpenCV correctness, latency, RSS, live-heap, and growth
profiles are accepted on both release targets by ADR 0051. The example uses no
caller frame-polling loop:

```text
crates/mado-pilot/examples/template-watch.rs
```

The same Rust query boundary is qualified over maintained Windows WGC and macOS
ScreenCaptureKit window/display sessions by ADR 0057. The native example
requires an explicit discovery index and caller asset package; it presents no
permission UI, activates no target, injects no input, and prints no target title
or native identifier:

```text
crates/mado-pilot/examples/native-template-watch.rs
docs/native-template-watch.md
```

One-shot OCR over an exact retained replay/native frame is also exposed through
Rust, C ABI 1.3, and C++. The production default examples construct the accepted
G-004 ONNX CPU profile from caller-supplied canonical model-root/runtime paths,
recognize full and bounded blank replay regions, prove result ownership, and
close twice:

```text
crates/mado-pilot/examples/ocr-default.rs
crates/bindings/capi/examples/c/ocr-default.c
crates/bindings/capi/examples/cpp/ocr-default.cpp
```

The separate fixture examples exercise explicit caller-selected backends without
ONNX Runtime, network, or input. Their feature-gated C/C++ constructor remains
outside the public ABI:

```text
crates/mado-pilot/examples/ocr-fixture.rs
crates/bindings/capi/examples/c/ocr-fixture.c
crates/bindings/capi/examples/cpp/ocr-fixture.cpp
```

The same ownership flow over *real* windows and displays — including bounded
input submission with explicit route evidence and a separate newer-frame visual
observation — is implemented in the target adapters, composed by the runtime,
and exposed through the Rust facade, C ABI, and header-only C++ wrapper.
Optional finite engine-scoped diagnostics correlate those operations, OCR, and
Rust template queries without captured pixels, recognized text, caller
template/model identity, or input payloads.
The native examples each require the operator to name one window exactly and refuse anything ambiguous,
because the events they submit are real:

```text
crates/mado-pilot/examples/windows-native-input.rs
crates/mado-pilot/examples/macos-native-input.rs
crates/bindings/capi/examples/c/windows-native-input.c
crates/bindings/capi/examples/c/macos-native-input.c
crates/bindings/capi/examples/cpp/native-input.cpp
```

Native release acceptance now covers all fourteen controlled macOS
owning-process pairs, the accepted macOS production capture and transition
profiles, the qualified Windows 11 25H2 floor including its controlled native
unsupported path, the repaired Windows 1280×720 and mixed-DPI dual-4K production
profiles, and the final five-process native Rust watcher matrix on each approved
host. The watcher matrix retains identical semantic rows with independent WGC
and ScreenCaptureKit latency, heap, resident-memory, growth, geometry, lifecycle,
ownership, cleanup, and privacy enforcement. Each lineage remains bound to its
own source, topology, stimulus, oracle, and target-specific budgets. Historical
profiles and hosted CI never substitute for interactive native rows.
The dual capture profile retains 600 stationary samples per display and 300
moving-seam frame pairs with per-frame callback correlation. Windows
final-source Phase 1 and repository verification run on the exact exit candidate
under unchanged ceilings. Apple Silicon Phase 1 remains attributed to `d8336be`
and applies by reviewed complete diff; exact-candidate hosted checks bind both
release targets.

Released `v0.3.0` carries approved Windows 11 and Apple Silicon integrated
42-region default-OCR quality plus separate target-native numeric profiles.
The `v0.3.1` candidate adds an explicit non-default bounded profile,
one-to-eight caller-owned grouped zones, initialization-time OCR provider policy,
C ABI 1.5, and matching Rust/C++ ownership surfaces. CoreML qualification is
rejected; explicit Windows CUDA and its fresh-CPU initialization fallback pass
terminal provider enforcement, while automatic selection remains CPU on both
targets because CUDA exceeded the fixed RSS ratio. Protected release delivery
remains open.

macOS capture needs Screen Recording and input needs event-post access;
MadoPilot probes both without prompting. Windows has no permission probe and
reports integrity/UIPI outcomes without elevation. A green run that skipped a
permissioned or interactive native scenario is not evidence that scenario ran.

The public workflow recognizes one retained frame through explicit
caller-selected OCR wiring, a CPU default/profile constructor, or the provider
constructor. It can scan one to eight caller-owned zones through one shared
detector pass. Preferred provider fallback is restricted to initialization
before engine publication. It does not watch for text, schedule work, retry a
failed inference, switch provider after publication, or trigger input.

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
| Native capture | Implemented in both adapters and exposed through Rust, C ABI, and C++; ADR 0030 accepts macOS production capture/transitions, ADR 0031 accepts Windows 1280×720 production capture/transitions, and ADR 0032 accepts Windows mixed-DPI dual-4K production capture |
| Native input submission | Implemented in both adapters and exposed through Rust, C ABI, and C++; system input, Windows exact-window delivery, and macOS owning-process delivery are explicit, receipts state submission evidence rather than application consumption, and fixture-scoped automatic checks send no uncontrolled desktop input |
| Bounded diagnostic observation | Implemented through Rust, C ABI, and C++ with allocation-free `Off`, finite `Normal`/`Debug` streams, exact loss counts, and content-redacted OCR records; Rust-only watcher records add bounded query/source/region/state/stability/disposition/queue/elapsed/outcome facts without changing the released C ABI |
| One-shot OCR public contract | Implemented through Rust, C ABI 1.5, and C++ over explicit backends, accepted ONNX CPU profiles, and explicit initialization-time provider policy; singular and one-to-eight-zone operations remain separate, with no OCR watcher, OCR scheduling, per-inference retry, bundling, or download |
| Bounded template-presence query | Implemented and budget-qualified through the Rust replay/OpenCV facade and maintained Windows WGC/macOS ScreenCaptureKit sessions with current-once/strictly-newer frames, fixed finite latest-wins scheduling, separate query/wait authority, exact change/rate admission, confirmed-only stability, exact coalescing, fair two-worker progress, stale-result rejection, and one immutable terminal outcome. ADR 0057 limits native support to the qualified Rust boundary. Callbacks/subscriptions, Tokio/futures, C/C++, OCR predicates, automatic input, target activation, arbitrary application/template/ROI compatibility or timing, and real-time guarantees remain unavailable |
| C ABI, tracked C header, dynamic library | Implemented through ABI 1.5 while preserving complete ABI 1.0, 1.2, 1.3, and 1.4 prefixes at 424, 592, 648, and 720 bytes; provider construction and engine-owned provider descriptors extend the table to 736 bytes; the unreleased 1.1 draft is intentionally unsupported |
| Header-only C++ RAII wrapper and CMake targets | Implemented through ABI 1.5, including owning profile/zone/provider projections, move-only grouped results, explicit clone, and lvalue-only borrowed OCR/provider descriptor views |
| C ABI static library, ABI-major loader names, pkg-config, CMake install | Not implemented |
| Numeric performance budgets | Phase 1 and affected Phase 2 ceilings remain revision-bound and enforced. ADR 0037 accepts default-OCR target profiles; ADR 0041 accepts explicit bounded singular budgets. ADR 0044 accepts target-specific grouped-zone latency, lifecycle, and resource budgets for fixed non-overlapping layouts; ADR 0045 corrects only the new Apple integrated cache-cold startup ceiling after a retained failed source. ADR 0048 accepts explicit Windows CUDA and mixed-provider fallback profiles after retaining two stopped tail observations; automatic preference remains CPU. ADR 0051 accepts target-specific deterministic replay/OpenCV template-query latency, RSS, live-heap, and growth budgets; ADR 0052 corrects only the independently remediated Windows ROI maximum. ADR 0053 fixes independent native WGC and ScreenCaptureKit ceilings, and ADR 0057 accepts both final cohorts while preserving withheld single-run and cadence-dependent measures. Protected release delivery remains open |
| Release packaging | Not implemented |

The public Rust names have been reviewed and settled
([`G-009`](docs/validation-gates.md#g-009), resolved by
[ADR 0006](docs/adr/0006-public-rust-names-and-compatibility-policy.md)), but
they are not yet a stability promise: that begins at 1.0, and until then a
rename costs an ADR and a version bump rather than being impossible. The C ABI
is separately versioned and **is** frozen: ABI 1.0, ABI 1.2, ABI 1.3, and ABI
1.4 are permanent complete prefixes. ABI 1.5 appends provider-policy
construction and engine-owned provider descriptors without moving them
([ADR 0007](docs/adr/0007-phase-1-c-abi-freeze.md),
[ADR 0023](docs/adr/0023-input-submission-observation-and-abi-1-2.md),
[ADR 0035](docs/adr/0035-ocr-public-surfaces-and-private-fixture-boundary.md),
[ADR 0036](docs/adr/0036-default-ocr-composition-and-abi-prefix.md),
[ADR 0043](docs/adr/0043-ocr-profile-and-zone-public-surfaces.md), and
[ADR 0046](docs/adr/0046-onnx-accelerator-provider-policy.md)).
Within ABI major 1, no released value, field, or function-table entry moves. The
C++ wrapper declares no ABI of its own and inherits the C one.

[docs/architecture.md](docs/architecture.md) is the tracked baseline and records
the full status table, the package inventory, and the dependency rules.

## Releases

[`v0.1.0`](docs/releases/v0.1.0.md) is the published deterministic-workflow
baseline. [`v0.2.1`](docs/releases/v0.2.1.md) is the published native
capture/input/observation source release. [`v0.3.0`](docs/releases/v0.3.0.md) is
the published default-OCR source release. [`v0.3.1`](docs/releases/v0.3.1.md) is
the bounded-profile/grouped-zone release candidate and remains unpublished until
its protected exact-source qualification and release flow complete. None
publishes crates to crates.io or provides prebuilt libraries, installers, CMake
install/export metadata, pkg-config metadata, or bundled OpenCV, ONNX Runtime,
or model files. A tracked release-note file is the canonical release body.

## Release targets

Version one targets `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. Hosted
CI builds and tests both natively, but it never substitutes for an interactive
display, permission, input, device, or target-loss matrix. The qualified macOS
floor is Apple Silicon macOS 26.5.2 build 25F84. The qualified Windows floor is
Windows 11 25H2 build family 26200 on a currently serviced x64 desktop
installation; the retained boundary run used Pro build 26200.9168. Remaining
version-one decisions are tracked in [docs/validation-gates.md](docs/validation-gates.md).

## Integration surfaces

The integration surfaces are an idiomatic Rust API through the `mado-pilot`
facade package, a separately versioned C ABI, and a thin C++ RAII wrapper that
consumes only that C ABI. All three exist; packaging and registry publication do
not.

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

Running integrated OCR requires the accepted model files under one
caller-selected root and one canonical absolute ONNX Runtime 1.29.0 path.
Explicit Windows CUDA additionally requires the exact controlled
ORT/CUDA/cuBLAS/cuDNN/NVRTC root described by ADR 0048. MadoPilot never bundles,
downloads, installs, or searches for these files; see
[docs/third-party-dependencies.md](docs/third-party-dependencies.md#implemented-onnx-runtime-prerequisite).

```sh
cargo build --locked --workspace
cargo test --locked --workspace --all-targets
```

The deterministic one-shot, bounded template watcher, and default OCR workflows
are runnable. OCR reads its two paths from the example environment only; the
library itself performs no environment lookup:

```sh
cargo run --locked --package mado-pilot --example deterministic-slice
cargo run --locked --package mado-pilot --example template-watch
MADO_PILOT_G004_MODEL_ROOT=/canonical/model/root \
MADO_PILOT_ONNX_RUNTIME=/canonical/path/libonnxruntime.1.29.0.dylib \
cargo run --locked --package mado-pilot --example ocr-default
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
measured ABI layout against current and frozen headers, compiles the frozen ABI
1.0 and 1.2 callers against the current library, and runs consumer programs:
```sh
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
cargo run --locked --package mado-pilot --example ocr-fixture
cargo run --locked --package mado-pilot --example ocr-default
cargo build --locked --package mado-pilot-capi --features private-fixture
cargo run --locked --package mado-pilot-capi --features private-fixture \
  --example c-abi-check -- --label "<host>"
```

## Documentation

| Document | Contents |
|---|---|
| [docs/architecture.md](docs/architecture.md) | Workspace, package responsibilities, dependency allowlist, naming, scope, status |
| [docs/c-abi.md](docs/c-abi.md) | The C boundary contract: handles, structure prefixes, ABI 1.5 negotiation, OCR/provider ownership, submission evidence, diagnostics, and panic containment |
| [docs/cpp-wrapper.md](docs/cpp-wrapper.md) | The C++ adapter contract: move-only owners, `Result`, OCR/receipt/diagnostic owners, borrowed views, and CMake targets |
| [docs/validation-gates.md](docs/validation-gates.md) | The `G-001`–`G-014` registry of unresolved version-one decisions |
| [docs/performance.md](docs/performance.md) | Benchmark format, historical/applicable Phase 1 and Phase 2 profiles, and the partial Phase 3 OCR budget status |
| [docs/third-party-dependencies.md](docs/third-party-dependencies.md) | Dependency license, source, advisory, and native-deployment policy |
| [docs/windows-input-verification.md](docs/windows-input-verification.md) | Windows input capability matrix, focus/UIPI behavior, fixture privacy bounds, and native checks |
| [docs/macos-input-verification.md](docs/macos-input-verification.md) | macOS input capability matrix, authorization and focus behavior, process-directed qualification, fixture privacy bounds, and native checks |
| [docs/releases/](docs/releases/) | Canonical release notes and exact artifact limitations |
| [docs/adr/](docs/adr/) | Architecture decision records, and [the template](docs/adr/0000-template.md) with the rule for when one is required |
| [docs/evidence/](docs/evidence/) | The measurements behind decisions that rest on them |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Branch strategy, pull request flow, verification sequence, required checks |

## Security and privacy

Screen capture, OCR, and input injection are sensitive capabilities, so the
project commits up front to how it will treat them: no implicit network access,
automatic privilege escalation, or hidden permission behavior. Ordinary logs
and diagnostic records exclude captured images, recognized text, input payloads,
window titles, platform namespaces, backend/runtime names, caller asset/model
identifiers, native free-form messages, and credentials. On macOS, permission
state is probed and reported without presenting permission UI.

The deterministic replay workflow still requests no permission, captures only
tracked replay sequences and files, and injects no input. The native workflow is
now reachable from the public facade and keeps the same commitments: both
platform packages capture and submit input with no elevation and redacted fixture
evidence; macOS reads its two authorizations without prompting and reads the
event-post decision again before every irreversible event on both routes;
Windows reports that it reads no separate authorization rather than having one
invented for it. Receipts state native submission evidence and never claim
application consumption. The C ABI and C++ wrapper preserve the same explicit
capability, permission, route, receipt, diagnostic, and ownership contracts.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md) for the branch strategy, the pull
request flow, and the verification sequence, then read
[docs/architecture.md](docs/architecture.md) before changing package boundaries,
dependency directions, public naming, or platform support.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE). Every
Cargo package in the workspace declares `Apache-2.0` to match.
