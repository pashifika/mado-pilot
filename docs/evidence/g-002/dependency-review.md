# G-002 prototype dependency review

The disposable prototype uses platform components already installed on the
approved Windows host. It adds no dependency to a MadoPilot package, the root
`Cargo.lock`, or a release artifact.

## Reviewed components

| Component | Installed version | Role | Maintenance and compatibility | License and advisory position |
|---|---|---|---|---|
| Windows SDK | 10.0.26100.0 for the accepted confirmation; 10.0.22621.0 for the initial diagnostic pass | Win32, C++/WinRT, WGC interop, D3D11, DXGI headers and import libraries | `CreateForWindow` needs SDK 18362 and `CreateFreeThreaded` needs SDK 17763, so both families expose the complete probe surface. The complete accepted matrix and lifecycle/display scripts were rebuilt and rerun with the 26100 family after the host update. A newer compile-time SDK does not set the product's minimum runtime OS; `G-001` still requires API-availability probes. | Microsoft Windows SDK terms; no SDK binary is redistributed. Servicing and security notices come through Microsoft rather than RustSec. The prototype links only operating-system libraries. |
| Visual Studio Enterprise 2022 | 17.14.37, MSVC 19.44.35228 for the accepted confirmation; 17.7.4, MSVC 19.37.32824 for the initial diagnostic pass | MSVC x64 compiler, linker, and build environment | The accepted run uses the current supported Visual Studio 2022 baseline and compiles the complete C++20 probe against SDK 26100. The initial run remains diagnostic history and does not determine acceptance. | Microsoft Visual Studio license; compiler runtime is not bundled by this evidence run. Visual Studio Installer is the update/advisory channel. |
| CMake | 3.29.5 | Generates the isolated prototype build | Satisfies the prototype's declared minimum 3.25. No CMake module is copied into the product. | BSD-3-Clause; build-time only, not redistributed. |
| Windows operating-system libraries | Host build 26200 | `d3d11.dll`, `dxgi.dll`, `windowsapp.lib` / WinRT activation | WGC is supported on Windows desktop; the HWND interop path is available from Windows 10 version 1903. Calls remain runtime checked even though this host is newer. | Windows component; serviced by Windows Update and not redistributed. |

The current Windows SDK overview and version mapping are
<https://learn.microsoft.com/windows/apps/windows-sdk/> and
<https://learn.microsoft.com/windows/apps/get-started/versioning-overview>.
The API-specific minimums are documented by
<https://learn.microsoft.com/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow>
and
<https://learn.microsoft.com/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded>.
Those pages were consulted while this review was written, so no later than the
commit that recorded it: 2026-07-31 00:48 JST, 2026-07-30 15:48 UTC. Microsoft
revises them in place, and this review names the reading rather than retaining a
copy. The installed versions in the table above come from the accepted run rows
themselves and are reproducible from the retained evidence.

## Source reference

The minimal device/interoperation shape was checked against Microsoft's
MIT-licensed `ScreenCaptureforHWND` sample:
<https://github.com/microsoft/Windows.UI.Composition-Win32-Samples/tree/master/cpp/ScreenCaptureforHWND>.
The prototype is written for this Change and does not vendor the sample.

## Deliberately absent dependencies

- No `windows` Rust crate, NuGet package, Windows App SDK, DirectXTK, WIL,
  graphics framework, image codec, JSON library, or test framework.
- No product package or workspace manifest change.
- No downloaded DLL, redistributable, model, or asset.
- No implicit network access at build or run time.

JSON Lines are emitted by a small prototype-local writer so that dependency
review is about WGC/D3D11 behavior rather than a serialization library. The
output is validated separately before it is distilled into tracked evidence.

## Review result

The updated SDK and MSVC expose every API required for this ownership
experiment, and the complete accepted confirmation uses them. SDK 22621 remains
installed side by side only as the initial diagnostic build environment; it is
not the acceptance baseline.

No prototype component becomes a product dependency or a release toolchain
promise. The production Adapter Change still selects and reviews its Rust
Windows binding crate, pins exact Cargo versions, uses a supported native
toolchain, and resolves `G-001` from runtime availability evidence. G-002
measures ownership on a revision-bound host; it does not approve a minimum
Windows version or a release package.
