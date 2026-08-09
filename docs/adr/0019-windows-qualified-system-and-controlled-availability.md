# ADR 0019: Windows Qualified System and Controlled Availability

- **Status:** Proposed
- **Date:** 2026-08-09
- **Resolves gate:** Windows half of [`G-001`](../validation-gates.md#g-001) on acceptance
- **Supersedes:** _none_

## Context

MadoPilot's picker-free Windows capture path needs
`IGraphicsCaptureItemInterop::CreateForWindow`, introduced in Windows 10 1903,
and `Direct3D11CaptureFramePool::CreateFreeThreaded`, introduced in Windows 10
1809. Those API floors are not a support decision: both releases and their SDK
families are out of support in 2026, and the project has no approved oldest host
for either one.

The approved x64 desktop host is Windows 11 Pro 25H2 build `26200.8894`, with
Windows SDK `10.0.26100.0`. It already supplied Phase 1 performance and Phase 2
capture-ownership evidence. GitHub's `windows-2025` runner supplies useful native
CI, but it is Windows Server 2025 and cannot qualify a Windows desktop SKU. The
initial exit probe could not reach the approved desktop host, so this record must
remain proposed and [`G-001` evidence](../evidence/g-001/README.md) explicitly
retains the gap.

## Decision

On acceptance, the minimum supported Windows release will be Windows 11 version
25H2, build family `26200`, on a currently serviced x64 desktop installation.
The qualified boundary is Windows 11 Pro build `26200.8894`; Windows SDK
`10.0.26100.0` is the supported build input and does not lower or raise the
runtime floor.

The Adapter continues to resolve version-sensitive DPI, WinRT activation, and
WinRT-D3D exports from verified absolute system-library paths after an
operation-time availability check. Missing modules, exports, activation factories,
or WGC support return typed `Unsupported`; they never cause eager-load failure,
show a picker, or silently substitute another capture or input mechanism.

This decision is not active while the ADR status is Proposed. The architecture
and public support tables continue to report the Windows minimum as unresolved.

## Alternatives

- **Windows 10 1903/build 18362.** Rejected as an API floor rather than a support
  boundary. It is out of support, its SDK family is out of support, and no
  approved oldest-host load or native matrix exists.
- **Windows 11 24H2/build 26100.** Rejected for this release because the available
  build-26100 CI machine is a server SKU. Treating it as desktop qualification
  would convert cross-SKU inference into a product promise.
- **Windows 11 25H2 inferred from prior prototype evidence.** Rejected as
  acceptance evidence. Earlier G-002 runs used the right host but not the final
  release source; the final boundary build, load, and native matrix must run.
- **Leave Windows permanently unspecified.** Rejected for `v0.2.0`: a native
  release without an exact supported deployment boundary cannot satisfy G-001.

## Consequences

- Integrators may rely on Windows only after this ADR is accepted and the support
  tables name Windows 11 25H2. Earlier Windows releases may happen to load or run,
  but are unsupported and unqualified.
- Lowering the floor requires a replacement ADR and build, load, availability,
  native, C, and C++ evidence on the proposed oldest desktop host.
- The product keeps controlled resolution even though every currently used API
  predates the selected floor. This preserves typed capability failure and guards
  accidental eager imports.
- CI Server 2025 results remain cross-target deterministic evidence, not
  oldest-desktop qualification.
- No package, target triple, dependency, ABI, or input fallback changes.

## Verification

Acceptance requires the complete Windows plan in
[`docs/evidence/g-001/README.md`](../evidence/g-001/README.md) against the final
reviewed source revision. In particular:

- the approved host reports Windows 11 25H2/build 26200 and a current servicing
  revision;
- the full workspace and native public-language gates build and run with SDK
  10.0.26100.0;
- the product DLL and frozen ABI callers start, negotiate, execute, and close;
- `loader_imports` rejects eager imports of every controlled export while keeping
  provider discovery reachable; and
- native discovery, WGC capture, retained-frame progress, mapping, fixture input,
  cleanup, C, and C++ complete without fallback.

After that evidence is retained, accepting this ADR and updating
`docs/architecture.md`, `docs/validation-gates.md`, `README.md`, and the native
prerequisite guidance must occur in the same reviewed revision. Until then,
those files intentionally keep the Windows minimum unresolved.
