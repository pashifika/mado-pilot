# ADR 0019: Windows Qualified System and Controlled Availability

- **Status:** Accepted
- **Date:** 2026-08-22
- **Resolves gate:** Windows half of [`G-001`](../validation-gates.md#g-001)
- **Supersedes:** _none_

## Context

MadoPilot's picker-free Windows capture path needs
`IGraphicsCaptureItemInterop::CreateForWindow`, introduced in Windows 10 1903,
and `Direct3D11CaptureFramePool::CreateFreeThreaded`, introduced in Windows 10
1809. Those API floors are not a support decision: both releases and their SDK
families are out of support in 2026, and the project has no approved oldest host
for either one.

The approved x64 desktop host is Windows 11 Pro 25H2 build `26200.9168`, with
Windows SDK `10.0.26100.0`. The same Core i7-12700KF / RTX 4080 host previously
supplied Phase 1 performance and Phase 2 capture-ownership evidence at an earlier
servicing revision. GitHub's `windows-2025` runner supplies useful native CI, but
it is Windows Server 2025 and cannot qualify a Windows desktop SKU. The final
boundary run is retained under
[`G-001` evidence](../evidence/g-001/windows-minimum-system.md).

## Decision

The minimum supported Windows release is Windows 11 version 25H2, build family
`26200`, on a currently serviced x64 desktop installation. The qualified boundary
run used Windows 11 Pro build `26200.9168`; Windows SDK `10.0.26100.0` is the
supported build input and does not lower or raise the runtime floor.

The Adapter continues to resolve version-sensitive DPI, WinRT activation, and
WinRT-D3D exports from verified absolute system-library paths after an
operation-time availability check. Missing modules, exports, activation factories,
or WGC support return typed `Unsupported`; they never cause eager-load failure,
show a picker, or silently substitute another capture or input mechanism.


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
- **Leave Windows permanently unspecified.** Rejected for `v0.2.1`: a native
  release without an exact supported deployment boundary cannot satisfy G-001.

## Consequences

- Integrators may rely on Windows 11 25H2 build family 26200 on a currently
  serviced x64 desktop installation. Earlier Windows releases may happen to load
  or run, but are unsupported and unqualified.
- Lowering the floor requires a replacement ADR and build, load, availability,
  native, C, and C++ evidence on the proposed oldest desktop host.
- The product keeps controlled resolution even though every currently used API
  predates the selected floor. This preserves typed capability failure and guards
  accidental eager imports.
- CI Server 2025 results remain cross-target deterministic evidence, not
  oldest-desktop qualification.
- No package, target triple, dependency, ABI, or input fallback changes.

## Verification

The complete retained result is
[`windows-minimum-system.md`](../evidence/g-001/windows-minimum-system.md).
The original positive matrix remains bound to clean source `834a58f`, tree
`3294863`; it records:

- Windows 11 Pro 25H2 build `26200.9168`, which Microsoft's current-version
  table listed as the latest serviced 25H2 build;
- a passing full repository sequence and Windows package all-target run under
  SDK `10.0.26100.0`;
- successful product-DLL process load, current ABI 1.2 and frozen ABI 1.0
  negotiation, C, C++, ownership, fixture-backed native flows, and CMake
  consumers;
- a passing `loader_imports` check with provider discovery reachable and no
  eager controlled exports; and
- passing native discovery, WGC capture, retained-frame progress, mapping,
  acknowledged fixture input, lifecycle, and cleanup rows without fallback.

Final repair source `9bfc0c0`, tree `be1c571`, adds the required controlled
negative result. An off-by-default qualification feature makes the named
WinRT-D3D export unavailable only inside isolated child processes, before the
resolver cache, without modifying the host. Rust, C, and C++ engines load, then
discovery returns typed `Unsupported`; removing the child-only environment value
restores successful Rust and fixture-backed C/C++ discovery. This closes the
native unsupported-capability row without changing the support floor or an
ordinary production artifact.

Lowering the floor still requires a replacement ADR and a new oldest-host
matrix.
