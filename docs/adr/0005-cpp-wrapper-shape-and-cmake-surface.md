# ADR 0005: A header-only C++ wrapper and a development-tree CMake surface

- **Status:** Accepted
- **Date:** 2026-07-28
- **Resolves gate:** _none_. It records evidence for
  [`G-010`](../validation-gates.md#g-010) and deliberately freezes nothing the
  gate owns.
- **Supersedes:** _none_

## Context

Phase 1 stage 7 adds the third integration surface. Three questions had to be
answered before a declaration was written, and none of them has a precedent in
this repository: there was no `.hpp`, no `.cpp`, and no `CMakeLists.txt`
anywhere in it.

**What kind of artifact the wrapper is.** `docs/architecture.md` reserves
`include/madopilot/madopilot.hpp`, the namespace `madopilot`, and the CMake
target `MadoPilot::Cpp`. It does not say whether that target produces a library.

**How the wrapper is built and consumed.** The C ABI is reached today through
`cargo`, a compiler invocation, and a linker flag. `MadoPilot::C` and
`MadoPilot::Cpp` are reserved names with no CMake behind them, and the two
costliest mistakes of stage 6 were both in the non-cargo build path rather than
in the product code.

**How much of the C error a C++ `Result` keeps.** A package-loading failure
carries `asset_fault` and `asset_stage` alongside its status, deliberately,
because a bad content hash and an unsafe entry path are both
`MADOPILOT_STATUS_ASSET_INVALID` and are not the same problem. An idiomatic C++
error type that reduced a failure to a status would undo that.

## Decision

### The wrapper is header-only

**`crates/bindings/capi/include/madopilot/madopilot.hpp` is the whole wrapper.**
There is no `.cpp`, no compiled artifact, and no second library to version.

It is the smaller claim. A compiled wrapper would have a C++ ABI of its own —
one that varies with the compiler, the standard library, the standard version,
and the exception and RTTI flags — and the project would then be promising two
compatibility contracts where it has evidence for neither. The C ABI stays the
only ABI, which is what `docs/architecture.md` says the C++ surface is for.

It also costs nothing. Every wrapper operation is a delegation to a function
pointer plus a status test; there is no state to hide behind a translation unit
and no algorithm worth isolating. The one piece of state a wrapper needs — the
negotiated function table — is carried by `madopilot::Api` and by each owner,
rather than by a file-scope variable, so the wrapper has no global state at all.

The minimum is **C++17**, for `std::string_view` and `std::optional`. Both
release-target toolchains have had it for years.

### The C++ vocabulary is aliases, not a second set of enumerations

`Status`, `ErrorCategory`, `Space`, `PixelFormat`, `ClipPolicy`, `Continuity`,
`Suppression`, `SourceKind`, `PackageSourceKind`, `AssetFault`, and
`AssetStage` are `using` aliases of the C types. A caller writes
`MADOPILOT_STATUS_OK`. Structures with no borrowed view —
`madopilot_pixel_rect_t`, the frame stamp, the frame and session descriptors,
the effective match options — are passed through as themselves under a C++ name.

Re-declaring them as `enum class` with the C macros as initializers would read
better at a call site and would cost about seventy-seven enumerators of pure
restatement, twenty-nine of them asset faults. Every one is a value `G-010` has
not frozen. Worse, the failure mode is silent: adding a status to the C header
would leave the C++ enumeration compiling perfectly and one value short.

The wrapper does add a C++ projection wherever a C structure carries a
`madopilot_str_t` or `madopilot_bytes_t`, because those need a name that says
they are borrowed and a way to copy them. That is `BorrowedStr` and
`BorrowedBytes` and the seven small structures that hold them.

### A C++ error owns everything, including the asset detail

`madopilot::Error` is a value. Constructing one describes the C error handle,
copies the message and the backend identifier into `std::string`, and releases
the handle immediately — on every failing path, including the one whose caller
looks only at the status. Nothing borrows, and there is no last-error slot.

The asset detail is preserved as `std::optional<AssetDetail>`, holding the fault
and the stage as the C pair. It is an accessor, not a variant a caller must
destructure: `error.status()` works without knowing that package loading exists,
and `error.asset_detail()` is there for the caller that wants to tell a bad hash
from an unsafe path.

### CMake describes consumption, and builds nothing of MadoPilot's own

**`crates/bindings/capi/CMakeLists.txt`** is the `MadoPilot` project. It defines
`MadoPilot::C` as an imported shared library and `MadoPilot::Cpp` as an
`INTERFACE` target that adds the include directory, requires C++17, and links
`MadoPilot::C`. It compiles no MadoPilot source: `cargo` builds the library, and
this says what a consumer needs in order to use it.

It sits beside the header it describes rather than at the repository root. The
root is a virtual Cargo workspace and not a product package; a `CMakeLists.txt`
there would claim the repository is a CMake project, when what is buildable by
CMake is exactly this directory.

`MADOPILOT_ARTIFACT_DIR` is a cache variable naming the profile directory cargo
built into, because that path depends on the profile and on `CARGO_TARGET_DIR`
and CMake cannot know either. It defaults to `target/debug` and fails at
configure time, naming the `cargo build` command, when the library is not there.

**No generator is named.** Ninja is not guaranteed on either CI runner or either
verification host; MSBuild and Unix Makefiles are. The consumer project sets
`CMAKE_BUILD_TYPE` for single-config generators and is built with `--config` for
multi-config ones, so one invocation works on both.

**The consumer test is a separate project.**
`crates/bindings/capi/tests/cmake/` has its own cache and knows the two target
names and nothing else. It builds one C consumer against `MadoPilot::C` and one
C++ consumer against `MadoPilot::Cpp`, and runs both under CTest. A
`MadoPilot::Cpp` that failed to carry its include directory, or failed to bring
`MadoPilot::C` with it, fails there.

### The C++ checks extend `c-abi-check` rather than becoming a sibling

`cargo run --package mado-pilot-capi --example c-abi-check` now also compiles
and runs the C++ ownership probe, compiles and runs the C++ example, and
configures, builds, and runs the CMake consumer project.

Extending it reuses the artifact discovery, the `\\?\` prefix stripping, the
Windows import-library selection, the child-process library path, and the MSVC
launch diagnostics that stage 6 already paid for — the exact code where stage
6's mistakes were. A sibling command would have duplicated all of it, and both
CI jobs and `CONTRIBUTING.md` would have needed a second entry that a future
change could forget.

CMake therefore becomes a prerequisite of a check that previously needed only a
C compiler. `CMAKE` names one explicitly; on Windows the copy Visual Studio
ships is found through `VSINSTALLDIR`, which the developer environment this
check already requires has set.

### The scene generator is shared, not copied a third time

`crates/bindings/capi/examples/deterministic-scene.h` holds the deterministic
96×64 scene as `static` C functions valid in both C and C++. The C example, the
C++ example, and the C++ ownership probe include it.

The alternative was a third copy of the arithmetic — after
`mado_pilot_testkit::match_fixtures` and the C example — or a tracked replay
directory holding the 24 KiB of pixels. Stage 5's rule is to track bytes when
the exact bytes are the test; here the exact bytes are a closed-form function of
the coordinate, and the template they must match is already tracked. Sharing one
header reduces the number of places that can drift from three to two.

## Alternatives

**A compiled C++ wrapper library.** Rejected. It would introduce a second ABI
that varies with the consumer's compiler and flags, for code that is entirely
delegation. Nobody asked for the artifact, and `G-008` has already withheld a
static C artifact on the narrower grounds that its supported combinations are
unrecorded.

**`enum class` mirrors of the C constants.** Rejected above: seventy-seven
enumerators of restatement, all of values `G-010` has not frozen, with a silent
failure mode when the C set grows. Reconsider when the gate resolves and the
vocabulary stops moving.

**A throwing interface as the default, with a non-throwing adapter.** Rejected
by design section 10 before this ADR: the default is exception-free, and any
throwing convenience is a separate header that throws only after the C call
returns. Phase 1 does not add that header, because no caller has asked for it.

**A destructor that closes the session.** Rejected. A destructor cannot report
a failed drain, and a close that silently failed would be worse than no close at
all. `Session::close` is explicit and status-returning; destruction releases the
reference and does not close.

**A `CMakeLists.txt` at the repository root.** Rejected: the root is a virtual
Cargo workspace and not a product package, and only one directory is buildable
by CMake.

**An install and export set, so a consumer could `find_package(MadoPilot)`.**
Rejected for Phase 1. Release packaging is not implemented — the ABI-major
decorated loader names are part of it — and an export set would describe an
installed layout the project does not produce. `add_subdirectory` consumes the
development tree, which is what exists.

**A `cmake` step that is skipped when CMake is absent.** Rejected. A check that
reports success without having run is worse than a check that fails with an
actionable message, and both CI runners and both verification hosts have CMake
or can reach the one Visual Studio ships.

**A third copy of the scene generator, in C++.** Rejected above.

## Consequences

- The C++ surface has no build artifact, so nothing verifies it except
  compiling it. `c-abi-check` therefore compiles two C++ translation units on
  every run, and `cargo test` alone still does not cover the wrapper.
- A change to the C++ header's declared types fails
  `crates/bindings/capi/tests/cpp_surface.rs`, which is a plain `cargo test` and
  needs no C++ compiler. That is what keeps a later phase's surface — input,
  OCR, watchers, queries, callbacks, native frames — from appearing in C++
  before it exists in C.
- CMake 3.22 or later is now a prerequisite of `c-abi-check`, alongside the C
  and C++ compilers. `CONTRIBUTING.md` and [../c-abi.md](../c-abi.md) say so.
- `examples/c/deterministic-slice.c` no longer contains the scene generator. Its
  observable output is unchanged, which the same check verifies.
- Nothing here freezes a status value, a structure layout, or the function-table
  prefix. The C++ header says so at the top, and the surface test asserts that
  it declares no enumeration of its own.

## Verification

- `cargo run --locked --package mado-pilot-capi --example c-abi-check --
  --label "<host>"` compiles the C++ ownership probe and the C++ example with
  the release target's own compiler, runs both, and configures, builds, and runs
  the CMake consumer project under CTest. It is a required step in the Windows
  and macOS CI jobs.
- The ownership probe's `static_assert`s prove the move-only shape at compile
  time; its checks prove clone independence, parent/child lifetime, borrowed-view
  stability, zero-match success, close reporting, and concurrent const access at
  run time.
- `crates/bindings/capi/tests/cpp_surface.rs` asserts the declared inventory,
  the absence of later-phase concepts, one owner per reference-counted handle,
  and that no C enumerated value is restated.
- The C++ example is required to print the same match rectangles and scores as
  the C example, by the same driver, so a wrapper that changed an answer fails
  rather than reporting a different one.
