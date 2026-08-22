# G-001 minimum-system evidence

This directory records the build, process-load, SDK, availability, controlled-
linkage, and unsupported-capability probes for
[`G-001`](../../validation-gates.md#g-001). The Windows boundary is accepted by
[ADR 0019](../../adr/0019-windows-qualified-system-and-controlled-availability.md)
and the macOS boundary by
[ADR 0014](../../adr/0014-macos-qualified-host-and-frame-placement.md).

## Accepted boundaries

| Target | Deployment floor | Qualified host | Build SDK |
|---|---|---|---|
| `x86_64-pc-windows-msvc` | Windows 11 25H2, build family `26200`, on a currently serviced x64 desktop installation | Windows 11 Pro 25H2, build `26200.9168` | Windows SDK `10.0.26100.0` |
| `aarch64-apple-darwin` | macOS `26.5.2` | Apple Silicon macOS `26.5.2` (`25F84`) | macOS SDK `26.5` |

The Windows boundary is deliberately conservative. Windows 10 1903/build 18362
supplies the picker-free HWND interop API, and Windows 10 1809/build 17763
supplies the free-threaded frame pool, but those releases and SDK families are
out of support in 2026 and have no approved oldest host. Windows Server 2025 CI
remains supporting native build and contract evidence; a server SKU does not
replace the accepted desktop boundary. Lowering the floor requires its own
oldest-host run and replacement ADR.

The API and SDK baselines were checked on 2026-08-09 against Microsoft's current
references:

- [`IGraphicsCaptureItemInterop::CreateForWindow`](https://learn.microsoft.com/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow)
- [`Direct3D11CaptureFramePool::CreateFreeThreaded`](https://learn.microsoft.com/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded)
- [Windows SDK overview](https://learn.microsoft.com/windows/apps/windows-sdk/)
- [Windows 11 release information](https://learn.microsoft.com/windows/release-health/windows11-release-information)

## Reviewed source

The accepted Windows probe is bound to clean commit
`834a58f6c28ab94f3f7a6d5901e3370b07e93155`, tree
`3294863552d502a64f60f59449560d7b71f4e8b7`. The macOS records below retain
their own revision identities. Later release acceptance must rerun affected rows
or review the complete intervening diff.

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
for the accepted macOS floor. The
[owned-window replacement run](macos-owned-window-replacement.md) was rerun with
the complete one-display matrix at commit
`a1faf04505c8471deb4de8c136fddcc7f76105e7`: the retained filter published no
successor content, a fresh session captured the distinct successor, and the
retained original mapping stayed unchanged. ScreenCaptureKit emitted no explicit
loss event during the bounded observation, so the Adapter correctly did not
infer `TargetLost` from request timeouts.

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
cargo build --locked --package mado-pilot-platform-windows --bin mado-pilot-windows-input-fixture --bin mado-pilot-windows-window-message-fixture
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
5. A controlled missing `CreateDirect3D11DeviceFromDXGIDevice` export converges
   on typed `Unsupported` before capture in Rust, C, and C++, then ordinary
   supported discovery succeeds after the isolated apparatus is absent.

## Windows result

The approved desktop completed the positive boundary on 2026-08-22. The full
repository sequence and focused loader, Windows all-target, native capture,
acknowledged fixture input, product-DLL, ABI 1.2, frozen ABI 1.0, C, C++,
ownership, and CMake rows passed on the reviewed source.

Final repair source `9bfc0c0` completed the controlled negative row without
changing an operating-system file: isolated child processes suppressed one lazy
WinRT-D3D export before resolver caching. Rust, C, and C++ engines loaded and
returned typed `Unsupported` from discovery; normal discovery and fixture flows
passed after restoration. The detailed, privacy-reviewed record is
[`windows-minimum-system.md`](windows-minimum-system.md).

The user-focused `System` input and ordinary `WindowMessage` rows remain part of
their separate interactive matrix.

## Privacy

Retain only approved operating-system, architecture, SDK/toolchain, signature,
permission, GPU/driver, derived-display, command, outcome, and source-identity
fields. Do not retain captured pixels, target titles, recognized or input text,
credentials, raw display identifiers, or unrelated desktop metadata.
