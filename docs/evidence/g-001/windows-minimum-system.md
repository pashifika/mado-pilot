# Windows minimum-system qualification

## Decision scope and source

This record qualifies the Windows half of [`G-001`](../../validation-gates.md#g-001) and supports [ADR 0019](../../adr/0019-windows-qualified-system-and-controlled-availability.md). It does not satisfy the separate 1280×720 or dual-4K production-capture matrices, set a `G-013` budget, or claim an unperformed physical device-removal, TDR, or driver-upgrade.

The retained run used clean product source commit `834a58f6c28ab94f3f7a6d5901e3370b07e93155`, tree `3294863552d502a64f60f59449560d7b71f4e8b7`, on 2026-08-22 JST. Focused child Change `phase-2-windows-native-phase2-clippy` repaired the only initial repository-gate failure, an unused target-conditional benchmark binding, without changing a runtime crate, benchmark oracle, profile, or budget. Every affected Windows row was rerun after that commit was integrated.

## Qualified host

| Field | Observed value |
|---|---|
| Operating system | Microsoft Windows 11 Pro, version 25H2, x64, build `26200.9168` |
| Requested registry fields | `ProductName=Windows 10 Pro` (compatibility value), `EditionID=Professional`, `DisplayVersion=25H2`, `CurrentBuildNumber=26200`, `UBR=9168` |
| Servicing | Microsoft listed 25H2 as a current General Availability Channel version and `26200.9168` as its latest build on 2026-08-11 |
| Execution context | Non-elevated; Developer Mode enabled |
| CPU and memory class | 12th Gen Intel(R) Core(TM) i7-12700KF; 32 GiB |
| GPU and driver | NVIDIA GeForce RTX 4080; `32.0.15.9186` |
| Displays observed during this boundary run | Two online, non-mirrored 3840×2160 displays: primary scale 1.5 at `[0,0,3840,2160)` and secondary scale 1.25 at `[-3840,0,0,2160)` |
| Visual Studio and compiler | Visual Studio 2022 17.14.37; MSVC 19.44.35228 for x64 |
| Build SDK | Windows SDK `10.0.26100.0` |
| Rust | rustc 1.97.1 (`8bab26f4f`), Cargo 1.97.1 |
| CMake and OpenCV | CMake 3.29.5; OpenCV 4.14.0 |

The current-version servicing fact comes from Microsoft's [Windows 11 release information](https://learn.microsoft.com/windows/release-health/windows11-release-information). The SDK is a build input; it is not the runtime support floor.

## Commands and observed results

All MSVC-dependent commands ran from one Visual Studio x64 Developer Command environment per invocation, with the Visual Studio compiler and linker first on `PATH`.

The complete repository sequence passed on the retained source:

```text
cargo run --locked --package mado-pilot-dependency-check
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo deny --locked check
```

The dependency check accepted 16 packages and 39 product dependency edges. Workspace tests, benchmark smoke entrypoints, doctests, and rustdoc completed. `cargo deny` reported the policy-allowed `syn` 2/3 duplicate and finished with advisories, bans, licenses, and sources all accepted.

The focused minimum-system sequence also passed:

```text
cargo test --locked --package mado-pilot-platform-windows --test loader_imports -- --nocapture
cargo test --locked --package mado-pilot-platform-windows --all-targets -- --test-threads=1
cargo build --locked --package mado-pilot-capi
cargo build --locked --package mado-pilot-platform-windows --bin mado-pilot-windows-input-fixture --bin mado-pilot-windows-window-message-fixture
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "Windows 11 25H2 G-001" --windows-native-fixture
cargo test --locked -p mado-pilot-platform-windows --test native_capture -- --nocapture --test-threads=1
cargo test --locked -p mado-pilot-platform-windows --test native_input -- --nocapture --test-threads=1
```

Observed outcomes:

- `loader_imports`: 1 passed; provider discovery remained reachable and the PE import table contained none of the controlled version-sensitive exports.
- Windows package all-targets: 109 library tests passed; focused loader and native capture passed; the acknowledged fixture-input row passed. The two deliberately ignored interactive rows remained excluded from the aggregate.
- Separate native capture: 1 passed with WGC discovery, retained progress, mapping, resize, target-loss/replacement, callback-fence, device-terminal state-machine, idempotent close, and cleanup oracles.
- Separate native input: the dedicated acknowledged fixture row passed; the user-focused `System` row remained ignored and is part of the later interactive matrix rather than this minimum-system claim.
- `c-abi-check`: current ABI 1.2 layout agreed across Rust and C; the product DLL loaded; current C and C++ deterministic and fixture-backed native flows completed; frozen ABI 1.0 retained all 222 declared layout lines and negotiated its 424-byte prefix; ownership and CMake consumers completed. Both ordinary and acknowledged repository fixtures were used without fallback.

Final repair source `9bfc0c023db4d39e7caa59aa38b196477b971e3a`,
tree `be1c57127d495f1345a6619f1851acde627430f0`, ran the missing
controlled native unsupported-capability row:

```text
cargo run --locked --package mado-pilot-capi \
  --features qualification-unsupported-api --example c-abi-check -- \
  --label "Windows 11 25H2 final unsupported qualification 9bfc0c0" \
  --windows-native-fixture --windows-unsupported-qualification
```

The off-by-default qualification feature makes only
`CreateDirect3D11DeviceFromDXGIDevice` unavailable through an environment value
checked before the resolver's process cache. It changes no operating-system
file, module, activation factory, public ABI, or ordinary production build.
Rust, C ABI 1.2, and C++ each loaded/created the native engine successfully,
then native discovery returned typed `Unsupported`. The override existed only
in isolated child processes. After it was absent, Rust discovery and both
fixture-backed C/C++ native flows succeeded, proving restoration of the ordinary
supported path. Frozen ABI 1.0, ownership, and CMake consumers also completed.

## Executable identity

| Artifact | SHA-256 |
|---|---|
| `target/debug/madopilot.dll` | `3d23f97dc748c4bf6b9fddab09a9a4754a0bf3947e2085405638d6f0ada26f6d` |
| `target/debug/mado-pilot-windows-input-fixture.exe` | `946b5258ad37e04ed395f1438fb31c2dd653862d0acb3f2ee63d2839da8c73be` |
| `target/debug/mado-pilot-windows-window-message-fixture.exe` | `91b5b9770e886a33b453c81ed585f9af83f2ece579869a6749081658e1c35dac` |
| `target/debug/examples/c-abi-check.exe` | `6a90a36d8f89d97f7abf2d27e9a39f77e38f266f120ae73aa6668567ae194b1e` |

The final controlled qualification artifacts were:

| Artifact | SHA-256 |
|---|---|
| `madopilot.dll` | `95a01a408d7a897a837f4551a4673a5cd708e7d4f35ddf0b9a7be14ef0da6333` |
| `c-abi-check.exe` | `89214b59bb3ea1123194dcdd6def36ba011e3fff7a9544581a64bcc980cb7914` |
| `windows-native-input.exe` | `ab69df83fbf75724291f73c90a0de44fd2503a8408ce1a327254169af4b1f9f7` |
| `windows-native-input-cpp.exe` | `3f36ff2b6fd199fec346a2adfc113107a3ebf7ce9421a738b14b6b3934f9e88e` |

## Acceptance and privacy

The observed desktop is the approved oldest Windows host available to the project, exactly matches Microsoft's current 25H2 Pro servicing build, and passed build, process-load, lazy-availability, native Rust, C, C++, frozen-header, and cleanup gates. ADR 0019 therefore accepts Windows 11 25H2 build family 26200 on a currently serviced x64 desktop installation as the minimum supported Windows boundary.

Unredacted command output remains untracked because it contains local build paths and transient process-local details. This record contains no captured pixels or hashes, recognized or input text, credentials, user paths, window titles, PIDs, raw HWND/display identifiers, process inventory, or unrelated desktop metadata.