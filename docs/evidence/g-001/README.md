# G-001 minimum-system evidence

This directory records the build, process-load, SDK, availability, controlled-
linkage, and unsupported-capability probes for
[`G-001`](../../validation-gates.md#g-001). The macOS boundary is accepted by
[ADR 0014](../../adr/0014-macos-qualified-host-and-frame-placement.md). The
Windows boundary remains a candidate until the approved desktop host executes
the Windows matrix below on the final reviewed source.

## Candidate boundaries

| Target | Candidate deployment floor | Qualified host | Build SDK |
|---|---|---|---|
| `x86_64-pc-windows-msvc` | Windows 11 25H2, build family `26200`, on a currently serviced x64 desktop installation | Windows 11 Pro 25H2, build `26200.8894` | Windows SDK `10.0.26100.0` |
| `aarch64-apple-darwin` | macOS `26.5.2` | Apple Silicon macOS `26.5.2` (`25F84`) | macOS SDK `26.5` |

The Windows row is deliberately conservative and remains proposed. Windows 10
1903/build 18362 supplies the picker-free HWND interop API, and Windows 10
1809/build 17763 supplies the free-threaded frame pool, but those releases and
SDK families are out of support in 2026 and no approved oldest host exists.
Windows 11 24H2/build 26100 is not inferred from GitHub's Windows Server 2025
runner: a server SKU is not the selected desktop support boundary. An older
Windows floor requires its own oldest-host run and replacement ADR.

The API and SDK baselines were checked on 2026-08-09 against Microsoft's current
references:

- [`IGraphicsCaptureItemInterop::CreateForWindow`](https://learn.microsoft.com/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow)
- [`Direct3D11CaptureFramePool::CreateFreeThreaded`](https://learn.microsoft.com/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded)
- [Windows SDK overview](https://learn.microsoft.com/windows/apps/windows-sdk/)
- [Windows 11 release information](https://learn.microsoft.com/windows/release-health/windows11-release-information)

## Reviewed source

The initial probe is bound to `origin/dev/0.2.0` commit
`4de3308a7f3619223eca1556e183982d944d4a41`, tree
`3b69262eb8908b05d7b839ba912ee67ca267244d`. Later acceptance must name the
final exit-PR revision and either rerun or review the complete intervening diff.

## macOS result

The approved M1 Pro host reported:

| Field | Observed value |
|---|---|
| Operating system | macOS 26.5.2 (`25F84`) |
| Architecture | `arm64` |
| SDK | `26.5` |
| Compiler | Apple Clang 21.0.0 (`clang-2100.1.1.101`) |
| Rust | 1.97.1 (`8bab26f4f`) |
| CMake | 4.4.2 |
| OpenCV | 4.14.0 |

The deterministic repository sequence passed on the reviewed tree. The focused
probe also passed all three cases:

```sh
cargo test --locked --package mado-pilot-platform-macos --test linkage -- --nocapture
```

It proves that the final Mach-O declares `minos 26.5.2`, carries no eager
ScreenCaptureKit load command, and eagerly links exactly the six baseline
frameworks owned by the shim build. `c-abi-check` then loaded the dylib, negotiated
ABI 1.1, ran the C and C++ consumers and both frozen-header callers, completed the
non-prompting capability checks, and closed.

This closes build, load, SDK, deployment-metadata, and controlled-linkage evidence
for the already accepted macOS floor. It does not close ADR 0014's owned-window
destroy/replacement live oracle.

## Windows probe

Run from a non-elevated Visual Studio x64 Developer Command Prompt on the approved
Windows 11 desktop host. Developer Mode must be enabled so replay symlink tests
execute. Record `ProductName`, `EditionID`, `DisplayVersion`,
`CurrentBuildNumber`, and `UBR` from
`HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion`, then record `rustc -vV`,
Cargo, MSVC, `WindowsSDKVersion`, CMake, OpenCV, GPU/driver, and the approved
redacted display topology.

Run the complete documented verification sequence, followed by the focused
boundary checks:

```text
cargo test --locked --package mado-pilot-platform-windows --test loader_imports -- --nocapture
cargo test --locked --package mado-pilot-platform-windows --all-targets -- --test-threads=1
cargo build --locked --package mado-pilot-capi
cargo build --locked --package mado-pilot-platform-windows --bin mado-pilot-windows-input-fixture
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "Windows 11 25H2 G-001" --windows-native-fixture
```

Acceptance requires all of the following:

1. The boundary is Windows 11 25H2, build family 26200, x64, and currently
   serviced; SDK 10.0.26100.0 is recorded as a build input rather than confused
   with the runtime minimum.
2. The product DLL and both frozen callers actually start, negotiate, call, and
   tear down. A successful build alone is not load evidence.
3. `loader_imports` keeps discovery reachable and rejects eager PE imports of
   the version-sensitive DPI, WinRT activation, and WinRT-D3D exports.
4. Native discovery, picker-free capture, retained-frame progress, mapping,
   fixture input, cleanup, C, and C++ complete without fallback.
5. Missing controlled modules, exports, factories, or `IsSupported == false`
   still converge on typed `Unsupported` before capture. If no host exercises
   that negative branch, the result names the observation gap instead of claiming
   it ran.

## Current Windows gap

No qualifying desktop runner was reachable for the initial probe. The repository
has no self-hosted GitHub Actions runner, and `windows-2025` is Windows Server
2025. PR #29 passed the exact reviewed tree on that server runner, including WGC,
mapping, the dedicated input fixture, C, C++, frozen headers, ownership, and
CMake. That remains cross-target deterministic evidence only; it is not relabeled
as a Windows 11 oldest-desktop probe.

Until the approved host completes the matrix, the proposed Windows minimum is not
a support claim, [`G-001`](../../validation-gates.md#g-001) remains open for
Windows, and the platform support tables remain unchanged.

## Privacy

Retain only approved operating-system, architecture, SDK/toolchain, signature,
permission, GPU/driver, derived-display, command, outcome, and source-identity
fields. Do not retain captured pixels, target titles, recognized or input text,
credentials, raw display identifiers, or unrelated desktop metadata.
