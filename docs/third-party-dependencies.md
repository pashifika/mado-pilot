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
| `sha2` 0.11 | `mado-pilot-assets`, `mado-pilot-ocr`, `mado-pilot-backend-onnx` | Verifies package entry digests, immutable OCR model component identities, and the recognizer graph's embedded vocabulary before session admission | MIT OR Apache-2.0 |
| `unicode-normalization` 0.1.25 | `mado-pilot-ocr` | Applies the accepted G-004 NFC rule once at the platform-neutral OCR commit boundary. Its only dependency is the small `tinyvec`/`tinyvec_macros` pair already pinned in `Cargo.lock` | MIT OR Apache-2.0 |
| `opencv` 0.99 | `mado-pilot-backend-opencv`, `mado-pilot-backend-onnx` | Binds the OpenCV C++ image-processing API used by the CPU matching profile and the accepted OCR profiles' direct resize, contour, dilation, and original-source perspective-crop rules. Default features **off**; `imgcodecs`, `imgproc`, and `clang-runtime` only | MIT |
| `ort` 2.0.0-rc.13 | `mado-pilot-backend-onnx` | Exact-pinned safe session/tensor/metadata/run-options wrapper. Default features **off**; only `std`, `alternative-backend`, and `api-17`. No downloader, dynamic-search helper, telemetry feature, GPU provider, or model fetcher is enabled | MIT OR Apache-2.0 |
| `libloading` 0.8.9 | `mado-pilot-backend-onnx` | Opens one caller-supplied canonical ONNX Runtime file with target-specific restricted flags and retains it for the process-global API lifetime. The version was already resolved by the OpenCV binding generator | ISC |
| `libc` 0.2.189 | `mado-pilot-backend-onnx` (macOS benchmark dev-only) | Supplies the target `rusage` layout and `getrusage` declaration used to enforce the Apple OCR process peak-RSS ceiling. The target-gated direct edge adds no crate to the lockfile and is absent from product dependencies | MIT OR Apache-2.0 |
| `windows` 0.62.2 | `mado-pilot-platform-windows` | Supplies Microsoft-maintained Rust bindings for the picker-free Win32 target inventory, WGC/WinRT interop, D3D11/DXGI ownership, DPI, system input, window messaging, and token-integrity APIs. Default features **off**; only the reviewed namespaces listed in the workspace manifest are enabled, and the dependency is `cfg(windows)`-gated | MIT OR Apache-2.0 |
| `cc` 1.4 | `mado-pilot-platform-macos` (build) | Compiles the Objective-C shim that owns the macOS native boundary. The package declares the build dependency unconditionally, so Cargo resolves the edge on every host; `build.rs` gates Objective-C compilation and Apple framework link directives on a macOS target. It was already an indirect build dependency through the OpenCV binding generator, so the graph gains an edge rather than a crate | MIT OR Apache-2.0 |

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
`getrandom`, `hashbrown`, `hybrid-array`, `indexmap`, `itoa`, `libc`,
`miniz_oxide`, `r-efi`, `serde_core`, `simd-adler32`, `typed-path`, `typenum`,
`winsplit`, and `zmij`, and the `serde` derive brings `proc-macro2`, `quote`,
`serde_derive`, `syn`, and `unicode-ident` with it.

Every one is accepted under `MIT`, `Apache-2.0`, `Zlib`, or `Unicode-3.0`, so no
exception is needed and `cargo deny check licenses` passes without one. Two are
worth naming because their expressions offer a term this project does not
approve: `adler2` offers `0BSD OR MIT OR Apache-2.0` and `r-efi` offers
`MIT OR Apache-2.0 OR LGPL-2.1-or-later`. Neither `0BSD` nor `LGPL-2.1-or-later`
is on the approved list; both crates are accepted under `MIT`, which is what
makes the check pass. An offered term is not an approved one, and this paragraph
previously cited `0BSD` as though it were.

The `opencv` crate adds a build-time closure of its own: `autocfg`, `cc`,
`clang`, `clang-sys`, `dunce`, `find-msvc-tools`, `glob`, `jobserver`,
`libloading`, `num-traits`, `opencv-binding-generator`, `percent-encoding`,
`regex` with `aho-corasick`, `memchr`, `regex-automata`, and `regex-syntax`,
`semver`, `shlex`, `vcpkg`, `pkg-config`, and on Windows targets the `windows`
family through `windows-core` — `windows-collections`, `windows-future`,
`windows-implement`, `windows-interface`, `windows-link`, `windows-numerics`,
`windows-result`, `windows-strings`, and `windows-threading`, named rather than
gestured at so the recorded closure can be checked against `Cargo.lock` entry by
entry. All are MIT or Apache-2.0. Most of them are the binding generator's, which
runs at build time and ships in nothing.

The `opencv` feature selection is a review decision. Default features bind
thirteen OpenCV modules the matching profile never calls, and every one of them
becomes a module the host installation must provide: a build against an OpenCV
that omits `videoio` would fail for a package that does no video. Turning defaults
off and enabling `imgcodecs` and `imgproc` narrows both the generated bindings and
the installation requirement to what
[adr/0003-opencv-matching-profile-and-public-score.md](adr/0003-opencv-matching-profile-and-public-score.md)
uses. `clang-runtime` is separate and not optional in practice; the OpenCV section
below records why.

The ONNX CPU adapter's feature selection is equally contractual. `ort`'s default
features are disabled; `alternative-backend` selects `ort-sys/disable-linking`,
so its build entry performs no native discovery, eager link, binary copy, or
download. API 17 contains every session, tensor, metadata, and run-termination
entry this adapter needs and omits newer automatic device-selection policy. The
active `ort` closure is only exact-version `ort-sys` and `smallvec`; both are
`MIT OR Apache-2.0`. `libloading` reuses the existing 0.8.9 resolution and its
existing `cfg-if` / target `windows-link` closure. ADR 0034 records the exact
source, feature, maintenance, MSRV, license, and native compatibility review.

The lockfile also records `ort`'s disabled optional `ndarray` and `tracing`
families: `matrixmultiply`, `ndarray`, `num-complex`, `num-integer`,
`once_cell`, `pin-project-lite`, `portable-atomic`, `portable-atomic-util`,
`rawpointer`, `tracing`, and `tracing-core`. They are absent from the active
`cargo tree -p mado-pilot-backend-onnx` graph and are not compiled into the
backend, but lockfile-wide advisory and license policy still reviews them. Every
recorded package is MIT, Apache-2.0, or a permitted dual-license expression, so
no policy exception is required.

## Reviewed decisions not yet in the lockfile

A gate resolution can settle which dependency a later change will need before
that change exists. Recording the review here means the change that finally adds
the crate arrives into a decision rather than reopening one, and that a reviewer
can see the license position was checked when the choice was made.

| Decision | Crates the implementation will need | License position | Recorded in |
|---|---|---|---|
| The macOS shim boundary, `G-003` | Implemented with `cc` alone; the `objc2` family was reviewed and not needed | `cc` is `MIT OR Apache-2.0` and is now recorded above. The reviewed `objc2`, `objc2-foundation`, `objc2-core-*`, and `objc2-screen-capture-kit` positions stand unused and are kept below for the next Change that might need them | [adr/0012-macos-shim-language-and-containment.md](adr/0012-macos-shim-language-and-containment.md) |

The exact versions are pinned by the change that adds them, against the
lockfile and the advisory database as they stand at that time.

### G-004 accepted OCR profile

`G-004` selects RapidOCR v3.9.2's `ch_PP-OCRv4_det_mobile.onnx` plus
`PP-OCRv6_rec_small.onnx`, 25,979,900 bytes total, through an explicit
caller-selected model root and fixed safe relative paths. Under independently
reviewed evaluator v5, this is the only candidate that passes every identity and
quality row on Apple Silicon and `x86_64-pc-windows-msvc`; every deterministic
candidate and image gate matches across targets. The exact SHA-256 values,
model/vocabulary shapes, preprocessing/decoder, fixture terms, public provenance,
cross-target outcomes, review record, and privacy constraints are recorded in
[evidence/g-004/](evidence/g-004/).

This decision adds no Cargo dependency, model/runtime/font bytes, backend, or
support claim. The implementation Change must independently select and review its
native and Rust dependency closure while consuming the accepted immutable
profile.

The profile is controlled host-provided: MadoPilot bundles and downloads nothing,
performs no ambient search, verifies byte count and SHA-256 before session
creation, and must return an actionable typed absence/mismatch outcome. If a later
release redistributes a model or ONNX Runtime, that Change reopens the packaging
decision, supplies the applicable Apache-2.0 or MIT notices, records package size
and operating-system impact, and resolves the still-open `G-007` obligations.

The qualification environment's Python, RapidOCR, OpenCV Python, Pillow, NumPy,
Shapely, and ONNX Runtime packages are disposable evidence tools only. They add no
workspace edge or lockfile entry. The implementation independently reviews its
actual native and Rust closure against the then-current advisory database.

### Implemented ONNX Runtime prerequisite

`mado-pilot-backend-onnx` implements the two exact closed profiles from ADRs
0033 and 0038 over the accepted G-004 detector, recognizer, vocabulary, and
decoder through ONNX Runtime 1.29.0's CPU provider and C API 17. Native G-004
keeps released preprocessing; bounded preprocessing is explicit and non-default.
[ADR 0034](adr/0034-onnx-runtime-cpu-loading-boundary.md) fixes the loading boundary:

- the host supplies one regular file at a caller-selected canonical absolute
  path: `libonnxruntime.1.29.0.dylib` on `aarch64-apple-darwin` or
  `onnxruntime.dll` on `x86_64-pc-windows-msvc`;
- the loader requires exact version string `1.29.0`, non-null API 17, and the
  target filename before model session creation;
- macOS uses `RTLD_NOW | RTLD_LOCAL`; Windows uses
  `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`;
- MadoPilot does not install, bundle, download, search `PATH`, accept an
  environment override, or fall back to another runtime/provider; and
- the runtime handle remains loaded for process life because the installed API
  table contains pointers into it.

The Phase 3 reviewed Apple runtime observation remains 33,332,888 bytes at its
original revision. It declares minimum macOS 14.0 and depends only on system
libraries/frameworks. The current CI-pinned official 1.29.0 archive used by the
bounded Change is independently bound by archive SHA-256 `d0706f…`; its selected
versioned dylib is 43,184,400 bytes with SHA-256 `68f6e5…`, minimum macOS 14.0,
SDK 26.2, and only system framework/library load commands. These are distinct
revision-bound observations, not interchangeable hashes. ONNX Runtime is
MIT-licensed and is not redistributed.

The accepted detector and recognizer remain caller-supplied Apache-2.0 model
bytes. Both `OnnxOcrBackend::open_accepted` and explicit
`OnnxOcrBackend::open_bounded_detector` read only
`rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx` and
`rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx` beneath the selected root, validate
exact length/SHA-256 before graph/session admission, and commit directly into
one immutable shared source. Explicit schema-v2 packages validate the same
`OcrModelSource` contract for either complete accepted tuple; default
composition does not create a duplicate 25,979,900-byte package allocation.

The two native CI jobs provision these inputs only inside an ephemeral runner.
They download the official ONNX Runtime 1.29.0 target archive and the two
tag-pinned ModelScope files, verify reviewed archive/model SHA-256 values before
extraction/session creation, and pass canonical paths to native tests. Both jobs
run the backend contract for native and bounded profiles, Rust default facade,
production C/C++/CMake default flows, frozen C headers, the released
default-profile benchmark smoke, and the new identical-input native/bounded
smoke matrix.

This is verification-fixture provisioning, not a product download path, release
bundle, shipped cache, ambient discovery mechanism, or approved-host
quality/numeric-performance evidence. The bounded benchmark adds only existing
workspace `serde`/`serde_json` development edges for a tracked fixture/report
schema; no production dependency, model/runtime byte, or license family changes.

ADR 0036 keeps the default an explicit native G-004 composition choice: ordinary
engine constructors remain unchanged, while `*_engine_with_default_ocr`,
`engine_create_with_default_ocr`, and C++ `DefaultOcrOptions` require the caller
to supply the two controlled paths. ADR 0040 adds only the explicit Rust
candidate-v2 backend selection; no dependency changes. ADR 0037's native G-004
budgets remain accepted, and ADRs 0039/0041 fix the bounded procedure and target
budgets. The rejected rectangular and accepted candidate-v2 precursors remain
separate evidence. Both approved-host precursors and strict final enforcement
pass; the explicit profile is qualified without changing dependency,
provisioning, bundling, or default-selection policy. Hosted Windows Server smoke
remains supporting hard-contract evidence only.

### G-003 macOS shim boundary

Two findings from that review are worth carrying forward, because they change what
the implementing change may assume. `cc` is already an indirect build dependency of
the workspace, so the shim's build script adds an edge rather than a crate. And
`objc2-screen-capture-kit` 0.3.2 declares
`#[link(name = "ScreenCaptureKit", kind = "framework")]`, a hard framework link:
by the linker's documented handling of a non-weak framework, adopting it as
published makes a binary fail to load below the framework's minimum macOS version
instead of reporting an actionable status — the eager-link failure the checklist
below rejects. That consequence follows from the linkage, not from a measurement:
the prototype verified the weak form produces a weak load command, and did not run
on a host without the framework. The shim owns weak framework
linking and availability gating instead, so that crate is adopted only if a
weak-linking arrangement is demonstrated for it.

**What the implementation actually needed.** `mado-pilot-platform-macos` adds `cc`
and nothing else. None of the `objc2` family is used, because the shim covers every
Apple object, callback, and exception interaction and the Rust side sees only the
shim's own C surface — so a Rust-side Objective-C binding had no work left to do. The
smallest dependency that satisfies the boundary is therefore one build-time crate,
and the reviewed `objc2` positions remain recorded above for a later Change that
finds a use for them. The shim also does not link the capture framework at all: it
loads it at runtime from an absolute system path, which is the same eager-link
failure being avoided, reached without a link-time dependency of either kind. See
the amendment in
[adr/0012-macos-shim-language-and-containment.md](adr/0012-macos-shim-language-and-containment.md)
for why the weak load command the review anticipated is not available to a Cargo
dependency's build script.

macOS input keeps the same arrangement and adds no crate. `CGEvent`,
`CGWindowList`, and the legacy Accessibility observation all come from
frameworks the build script already declares, and the process-directed entry
points `CGEventPostToPid` and `CGPreflightPostEventAccess` are resolved by
symbol from the absolute CoreGraphics framework path on first use, so a host
that cannot supply them reports a typed `Unsupported` result for exactly the
operation that needed them. The two frameworks input additionally needs are
loaded rather than linked, for the same reason ScreenCaptureKit is: **AppKit**
supplies the application activation `FocusPolicy::ActivateIfRequired` performs,
and **HIToolbox** — inside `Carbon.framework` — supplies the keyboard-layout
lookup that resolves a printable character to a key code. A headless automation
library must not carry a load command for the desktop UI framework or for Carbon,
so each is opened from its absolute system path on first use and the operation
that needed it reports `Unsupported` where it is unavailable. `tests/linkage.rs`
asserts the eager framework list is unchanged, and the interactive fixture's
window, private control protocol, and event recorder are compiled into a
separate archive that no released artifact links. The fixture alone opens
**OpenGL** from its absolute system framework path, and only in its opt-in
game-like renderer mode; no production artifact gains that load.

The same review recorded what the shim needs of the host, because a native
boundary's prerequisite belongs beside the one OpenCV declares. On the measured
Apple Silicon host the **Xcode Command Line Tools alone** were sufficient for every
step the prototype took: compiling Objective-C and Objective-C++, archiving both
into static libraries, linking those into a Rust binary, and separately linking each
as a dynamic library for dependency inspection. Full Xcode is not installed there
and was therefore not evaluated, so the smaller installation is the one with
evidence behind it and no parity between the two is claimed; and since the prototype
built no production shim and pulled in no Cargo dependency, this is the prerequisite
of the steps that were exercised rather than a measurement of the finished adapter.

The measurements ran against SDK 26.5 with a deployment target of macOS 11.0 —
deliberately below ScreenCaptureKit's 12.3 — so the weak-linking and `@available`
arrangement the shim owns was compiled and linked rather than assumed: `otool -L`
records the framework as a weak load command, the availability check evaluates, and
the class lookup resolves. All of that was observed on a host where the framework is
present. The unsupported-host path — framework absent, capability reporting a clear
status — was not exercised and cannot be from this host. The exact minimum supported
macOS version remains gate `G-001`. The measurements are in
[evidence/g-003/](evidence/g-003/README.md), and
[../CONTRIBUTING.md](../CONTRIBUTING.md) carries the same prerequisite as build
guidance.

The review covered maintenance, minimum-SDK compatibility, license, advisories, and
build requirements for a wider candidate set than the list above: `objc2` 0.6.4,
`objc2-foundation`, `objc2-core-graphics`, `objc2-core-video`, `objc2-core-media`,
`objc2-screen-capture-kit` and `block2` at 0.3.2/0.6.2, `dispatch2` 0.3.1,
`core-foundation` 0.10.1, `core-graphics` 0.25.0, `screencapturekit` 8.0.1, `cidre`
0.16.1, and `cc` 1.4.0. Versions, licences, release dates, and minimum supported Rust
versions come from the crates.io API, maintenance signals from the GitHub repository
API, and advisory status from the absence of a `crates/<name>` directory in the
RustSec advisory database — all queried on 2026-07-30. On that date no candidate had
an advisory and every minimum supported Rust version was below the pinned toolchain.
Those are review findings against a moving database, not retained evidence: the
change that adds a crate re-runs `cargo deny` against the lockfile and the advisory
database as they stand then. `screencapturekit` and `cidre` were rejected for version one — the first
because a single-vendor high-level wrapper would own the capture contract this
project owns, the second because its breadth and its 1.88 minimum both exceed what
the boundary needs. `core-foundation` and `core-graphics` are used only where the
`objc2` family lacks a binding, so that one framework does not end up with two.

### G-002 Windows ownership prototype

Resolving `G-002` adds no product dependency and therefore adds nothing to the
table above or to `Cargo.lock`. The disposable C++ probe used platform
components already present on the named host: Visual Studio 2022 17.14.37,
MSVC 19.44.35228, Windows SDK 10.0.26100.0, CMake 3.29.5, C++/WinRT, WGC,
D3D11, DXGI, and Win32 import libraries. It vendors no sample, header, runtime,
framework, JSON library, DLL, or redistributable.

The accepted ownership decision is
[ADR 0013](adr/0013-windows-capture-frame-detachment.md). The exact
prototype-only component, license, compatibility, and advisory review is in
[evidence/g-002/dependency-review.md](evidence/g-002/dependency-review.md).
Microsoft's SDK and Visual Studio terms apply to the development tools; the
evidence redistributes none of them. CMake is BSD-3-Clause and build-time only.
The linked Windows libraries are operating-system components.

The production `mado-pilot-platform-windows` Change independently reviewed and
pins `windows` 0.62.2 from crates.io. The crate is maintained by Microsoft,
declares `MIT OR Apache-2.0`, requires Rust 1.82, and is compatible with the
workspace's Rust 1.97.1 toolchain. Default features are disabled; the selected
features cover only WGC, D3D11/DXGI, WinRT interop, window/display enumeration,
DWM metadata, high-resolution timing, dynamic system-library lookup, DPI, system
pointer/keyboard input, bounded fixture messaging, process handles, and token
integrity inspection.

Cargo resolves its Rust-only `windows-*` support crates; MadoPilot adds no
Windows App SDK, WIL, DirectXTK, native redistributable, or runtime DLL.

The binding is target-gated in `crates/platform/windows/Cargo.toml`, so the
macOS product graph remains unchanged. WGC and its factories are still checked
at operation time. Version-sensitive DPI, WinRT activation/apartment, and
WinRT-D3D interop exports are resolved dynamically from host system DLLs after
that boundary, and a PE import-table test prevents them from becoming eager
imports. Selecting a binding does not claim a minimum Windows version,
preapprove a permission prompt, or resolve
[`G-001`](validation-gates.md#g-001). The host-provided D3D11, DXGI, DWM, User32,
GDI, and WinRT components remain serviced by Windows and are not bundled.
The lockfile, license, source, and advisory checks are rerun by this Change.
[../CONTRIBUTING.md](../CONTRIBUTING.md) carries the development-prerequisite
reading of the same review, beside the macOS one.

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

CI is a separate matter, and only its Windows job is pinned. Windows downloads the
same official 4.14.0 prebuilt; macOS installs whatever `opencv@4` Homebrew
currently carries, which was 4.13.0 on the first run. That is not a gap to close.
CI's job is native correctness on every pull request, not evidence, and an
unpinned minor version means every run also checks the adapter against a second
OpenCV 4 release. The two versions were measured to produce bit-identical fixture
scores, which is recorded with the evidence.

The adapter accepts major version 4 and reports
`VisionFault::BackendUnavailable` for anything else. The binding crate also
supports OpenCV 5, and Phase 1 does not, because nothing has been measured against
it.

### Bundled or host-provided

**Host-provided for development-tree consumers, not a released deployment
profile.** OpenCV is a *development prerequisite*: the source releases document
how to build and test the tree but bundle no OpenCV library and ship no
installer. `G-007` (bundling, deployment profiles, notices) and `G-008`
(static-link feasibility, controlled loading) are both open, and no statement
here should be read as settling either.

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
