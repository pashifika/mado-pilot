# ADR 0079: Retained native window requirements

- **Status:** Accepted
- **Date:** 2026-09-28
- **Resolves gate:** none
- **Supersedes:** none

## Context

A host selecting a window needs to relate its UI to the provider's retained
capture target and require the same capture rectangle on acquisition. Process
provenance alone cannot establish that correspondence. Rediscovering by PID,
title or native window number can select a different incarnation. Existing
native storage defaults also do not express a smaller host-owned image budget.

## Decision

Expose descriptive `NativeWindowDescription` and `WindowGeometry` through the
Rust capture/runtime/facade layers. `Engine::describe_window` revalidates the
original retained target. `OpenRequest::require_window_geometry` and
`with_resource_limits` are requirements, not preferences: unsupported providers
refuse, and native geometry changes do not silently resize or reselect.

macOS preserves its original ScreenCaptureKit filter/window and process lifetime.
The retained filter's `contentRect` and `pointPixelScale` must agree with the
current, equality-validated window. Windows preserves its original
`GraphicsCaptureItem`, Closed registration and process creation identity, and
matches the actual item extent to an observed extended-frame or client rectangle.
Both retain signed desktop origins and reject unobservable eligibility.

Required byte ceilings cover accounted image payload, including known row
padding, not physical GPU memory or RSS. Reserve controlled detached/CPU storage
before allocation. Reserve declared native producer payload before construction;
charge newly observable OS padding before accepting publication or CPU copying.
Common format-conversion and region mappings use `FrameStorage::reserve_cpu_copy`
against the source adapter's budget; each copied pixel owner retains its charge.
Keep reservations with their final storage owners, including after close.

## Alternatives

- Reconstruct a target from public native keys: loses retained-object authority.
- Poll a closed flag or capture again: neither establishes original ownership nor
  replaces terminal-aware commitment from [ADR 0078](0078-capture-terminal-publication.md).
- Promise a physical native-allocation ceiling: OS APIs do not expose producer
  layout beforehand; D3D staging pitch is available only after mapping.
- Guess alignment, downsample or enlarge the requested limit: obscures refusal
  and changes the selected image contract.

## Consequences

Existing omitted-option behavior and platform/global limits remain. Replay and
controlled/custom providers must reject unsupported requirements. No public
C ABI, C++ wrapper, dependency or support-floor change is introduced; the internal
macOS shim advances to ABI25 with matching size/offset checks.

macOS configuration completion does not prove retirement of its previous pool,
so requested-budget sessions conservatively retain old producer charges until
teardown. Repeated resize can exhaust the ceiling even if the OS reused storage.

Windows `GetWindowDisplayAffinity` guarantees success only for layered windows
under DWM composition. Unknown protection is `UnsupportedOption`, not assumed
unprotected; known nonzero protection is `AccessDenied`. This limits the new
required-window path's applicability without changing ordinary capture defaults.

## Verification

Portable capture/replay/runtime/testkit regressions exercise foreign targets,
interruption, required-option refusal and unchanged replay consumption. macOS
regressions cover geometry, private ABI layout, producer padding, replacement
charges, detached/CPU boundaries, converted/cropped mappings and final-owner
release. The mapping regression failed before the repair (104 accounted bytes
instead of 144) and passes with exact-ceiling refusal and post-release reuse.
Windows regressions cover corresponding geometry, authority, pitch and storage
cases.

A temporary public-facade executable exercised required-option refusal and
retained replay pixels after close. Windows source was cross-checked with
`cargo check --locked -p mado-pilot-platform-windows --target x86_64-pc-windows-msvc --all-targets`.
These checks do not qualify native window reuse, permissions, mixed displays,
interactive authoring or a Windows host. Native acceptance remains separate.

Platform contracts: [Apple contentRect](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/contentrect)
and [Microsoft GetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdisplayaffinity).
