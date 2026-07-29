# The C++ wrapper

MadoPilot's C++ surface is a header-only RAII adapter over the released C ABI.
It owns handles, turns statuses into an exception-free `Result`, and copies the
text a caller needs after a C handle is gone. It performs no capture, mapping,
matching, coordinate, or status logic of its own: every answer it returns came
from a C table entry.

The declarations are in
[`crates/bindings/capi/include/madopilot/madopilot.hpp`](../crates/bindings/capi/include/madopilot/madopilot.hpp),
a complete working caller is
[`crates/bindings/capi/examples/cpp/deterministic-slice.cpp`](../crates/bindings/capi/examples/cpp/deterministic-slice.cpp),
and the contract underneath is [c-abi.md](c-abi.md). Read that one first: every
rule here is a rule about how the C rules are expressed in C++, and none of them
replaces one.

## This wrapper declares no ABI of its own

The only ABI is the C one, frozen at 1.0 under gate
[`G-010`](validation-gates.md#g-010) by
[ADR 0007](adr/0007-phase-1-c-abi-freeze.md). What this header adds is source
compatibility, governed by the Rust-side policy in
[ADR 0006](adr/0006-public-rust-names-and-compatibility-policy.md): reviewed
names, not yet a stability promise.

The wrapper is deliberately not a second place those values are written down.
Its enumerated types are `using` aliases of the C types, so a caller writes
`MADOPILOT_STATUS_OK` and gets whatever the header it compiled against says that
is. The reasoning, and the C++ `enum class` that was rejected, are in
[ADR 0005](adr/0005-cpp-wrapper-shape-and-cmake-surface.md).

## Header-only, C++17

There is no `.cpp`, no compiled artifact, and no library to link beyond the C
one. A compiled wrapper would introduce a C++ ABI that varies with the compiler,
the standard library, and the flags, for code that is entirely delegation.

The minimum is C++17, for `std::string_view` and `std::optional`.

The negotiated function table is carried by `madopilot::Api` and by every owner,
so the wrapper has no global or thread-local state: no last-error slot, no
implicit initialization, and nothing to tear down at exit.

```cpp
#include "madopilot/madopilot.hpp"

auto loaded = madopilot::Api::load();
if (!loaded) {
    return 1;
}
const madopilot::Api api = loaded.take();
```

## Ownership

| Type | Owns | Clonable |
|---|---|---|
| `Api` | nothing — the table belongs to the library | copyable |
| `Error` | its own copies of the message and identifiers | copyable |
| `Cancellation`, `Engine`, `TargetList`, `Package`, `Template`, `Session`, `Frame`, `Mapping`, `MatchResult` | one reference-counted C handle | `clone()` |

**Every owner is move-only.** Copy construction and copy assignment are deleted,
because an implicit copy would hide a reference-count bump behind an assignment.
The explicit way to take a second reference is `clone()`, which calls the
corresponding C retain entry and returns an independent owner.

**Moving transfers the handle and leaves the source empty.** Both objects remain
destructible and exactly one release happens.

**Destructors release and never throw.** They also never report: a destructor
cannot answer a caller, which is why the one operation that can fail during
teardown stays explicit.

```cpp
madopilot::Result<void> closed = session.close(operation);
if (!closed) {
    // The failure is here, not swallowed by ~Session.
}
```

`Session::close` is idempotent, exactly as the C entry is. Destroying a
`Session` releases the reference and does **not** close it.

**An emptied owner keeps the table it came from.** Calling an operation on a
moved-from or reset owner forwards its null handle to the C boundary, which
refuses it with `MADOPILOT_STATUS_INVALID_ARGUMENT` — its own status, not one
the wrapper invented. The wrapper originates a status in only two places, both
of them "there is no library to ask": an owner that never held a table, and a
negotiation that reported success without returning one.

### Parents and children

The C rule is that releasing a parent never invalidates a separately retained
child, and the wrapper preserves it. A `Mapping` stays readable after its
`Frame`, its `Session`, and its `Engine` are destroyed. A `Template` outlives
its `Package`. A `MatchResult` outlives all of them, because it owns the exact
frame it searched.

In a function returning `madopilot::Error`:

```cpp
madopilot::Mapping mapping;
{
    madopilot::Session session = /* ... */;

    madopilot::Result<madopilot::Frame> acquired = session.acquire_frame(operation);
    if (!acquired) {
        return acquired.error();
    }
    const madopilot::Frame frame = acquired.take();

    madopilot::Result<madopilot::Mapping> mapped = frame.map(request, operation);
    if (!mapped) {
        return mapped.error();
    }
    mapping = mapped.take();

    const madopilot::Result<void> closed = session.close(operation);
    if (!closed) {
        return closed.error();
    }
}   // session and frame are gone
const auto image = mapping.describe();  // still valid
```

Each result is checked before it is extracted, for the reason
[below](#results-and-errors): `take()` has the precondition `ok()`.

## Results and errors

The default interface is exception-free in the sense that matters: no wrapper
operation throws to report a MadoPilot failure. Every failure the library can
report arrives as a status in a `Result`, and no status is translated into an
exception.

The wrapper does throw what its own allocations throw, which is `std::bad_alloc`.
Four places allocate, and all four are the wrapper making an owned copy for the
caller: an error's text, when a failing call describes it; the vector
`MatchResult::matches` fills; the copies a typed request keeps of what its C
structure points at; and an explicit `BorrowedStr::to_string` or
`BorrowedBytes::to_vector`. A caller that cannot tolerate `std::bad_alloc` from
those can read `Error::status()` and the borrowed views without ever making a
copy. Describing an error releases its C handle whether or not the copy of its
text succeeds.

```cpp
madopilot::Result<madopilot::Package> loaded = engine.load_package(source, operation);
if (!loaded) {
    const madopilot::Error& error = loaded.error();
    error.status();     // the C status, unchanged
    error.category();   // which subsystem
    error.message();    // owned std::string, redacted diagnostic text
    error.backend();    // std::optional<std::string>
    error.asset_detail();
}
```

**A `Result` cannot be dropped silently.** Both templates are `[[nodiscard]]`,
so discarding one is a compiler diagnostic. In a surface that reports failures
no other way, a dropped result is a dropped failure.

**Check a `Result` before reading it.** `value()` and `take()` have the
precondition `ok()`. They extract from a `std::optional` that a failure leaves
disengaged, so calling one on a failed result is undefined behaviour rather than
a thrown exception — the wrapper cannot throw to tell you, which is the price of
the exception-free surface. A build without `NDEBUG` fails an `assert` there; a
release build does not. Every snippet in this document, the in-repo example, and
the ownership probe test the result first.

`Error` is a value. Constructing one describes the C error handle, copies
everything out of it, and releases the handle immediately — on every failing
path, including the one whose caller reads only the status, and including the
one where a copy throws `std::bad_alloc`. The release is a scope guard rather
than a final statement for exactly that reason. Nothing in an `Error` borrows,
and there is no last-error slot to consult.

**The asset detail survives.** Package loading is the one Phase 1 operation
whose failures a caller may reasonably tell apart by more than a status, so
`Error::asset_detail()` carries the fault and the stage as
`std::optional<AssetDetail>`. It is an accessor rather than a variant every
caller must destructure: `status()` works without knowing package loading
exists.

```cpp
if (const auto& detail = error.asset_detail()) {
    detail->fault;  // MADOPILOT_ASSET_FAULT_HASH_MISMATCH, for instance
    detail->stage;  // and how far loading had got
}
```

Note what the status and the fault pair each tell you. A failing
`load_package` carries both. So does `prepare_from_package` for an identity the
package never declared — `MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` at
`MADOPILOT_ASSET_STAGE_COMMIT` — but its *status* is
`MADOPILOT_STATUS_INVALID_ARGUMENT` rather than
`MADOPILOT_STATUS_ASSET_INVALID`, because a package that loaded is valid and
asking it for something it never declared is the caller's mistake. The status
says whose mistake it was; the fault pair says which one. No backend is named,
because none ran. See
[ADR 0007](adr/0007-phase-1-c-abi-freeze.md), decision 4.

**Zero matches is a success.** A search that qualified nothing returns a
successful `Result` whose optional match is empty.

```cpp
const auto best = result.first_match();   // Result<std::optional<Match>>
if (best && !best.value().has_value()) {
    // A well-formed question with the answer "no".
}
```

## Borrowed views

A C accessor hands back a pointer and a length into memory a handle owns. The
wrapper names those `BorrowedStr` and `BorrowedBytes`, and each accessor
documents the owner that keeps them valid.

| View | Borrowed from |
|---|---|
| `BuildInfo::library_version`, `required_backend` | the loaded library |
| `TargetDescriptor::name`, `provider` | the `TargetList` |
| `PackageInfo::package_id`, `package_version`, `license` | the `Package` |
| `TemplateInfo::id`, `backend` | the `Template` |
| `Package::template_id` | the `Package` |
| `Image::bytes` | the `Mapping` |
| `ResultInfo::backend_id`, `backend_version` | the `MatchResult` |
| `Match::template_id` | the `MatchResult` |
| `Api::status_text` | the loaded library's static storage |

A view is valid only while its owner is retained. Copy anything that must
outlive it:

```cpp
if (info) {
    const std::string kept = info.value().package_id.to_string();
}
if (image) {
    const std::vector<std::uint8_t> pixels = image.value().bytes.to_vector();
}
```

**Name the owner, then ask it.** Every accessor above that borrows from a handle
is declared `const&` with its rvalue overload deleted, so it cannot be called on
a temporary owner:

```cpp
const auto id = engine.load_package(source, operation).take().template_id(0);
//                                                     ^ deleted: the package
//                                                       dies at the semicolon
madopilot::Package package = loaded.take();
const auto id = package.template_id(0);   // and this is fine
```

The first form reads correctly and leaves every view pointing into released
memory. Deleting the rvalue overload turns it into a compile error instead. The
two accessors whose views live in the library's own static storage —
`Api::describe_build` and `Api::status_text` — are unqualified, because nothing
they hand out can outlive the library.

`Error::message()` already returns owned `std::string`, because error text is
the text most likely to outlive the handle it came from.

## Typed requests

`Operation`, `Source`, `PackageSource`, `OpenRequest`, `MapRequest`,
`MatchOptions`, and `FindRequest` are values a caller composes. Each fills the
C structure's `struct_size` itself, so no call site can write a stale one, and
each owns whatever its C structure points at for as long as the request object
is alive.

```cpp
const madopilot::Result<std::uint64_t> now = api.clock_now();
if (!now) {
    return now.error();
}

madopilot::Operation operation;
operation.deadline(now.value() + 30ull * 1000 * 1000 * 1000)
    .cancellation(token);
```

The deadline is an **absolute instant** in the library's monotonic domain, read
from `Api::clock_now()` and added to. It is not a duration and not a wall clock.

Two of these borrow rather than own, and say so:

- an `Operation` borrows its `Cancellation`, which must outlive every call the
  operation is passed to;
- a `FindRequest` borrows its `Frame` and its `Template`.

`FindRequest::search_for` names the prepared template. It is not called
`template`, and cannot be: the word is a C++ keyword, so it cannot name a member
function any more than it can name the C structure field, which is `tmpl` for
the same reason.

Rectangles stay coordinate-qualified. `Match::bounds` and `ResultInfo::searched`
are `madopilot_pixel_rect_t` under the alias `Rect`, and each names the space it
is measured in rather than reducing to an integer pair.

A `Rect` the caller *supplies* is the other direction, and is narrower:
`MapRequest::region` and `FindRequest::region` accept
`MADOPILOT_SPACE_CAPTURE_PIXELS` only, because the Phase 1 C prefix has no
coordinate-conversion entry for the wrapper to delegate one to. Any other space
comes back as a failed `Result` carrying
`MADOPILOT_STATUS_INVALID_ARGUMENT` — the C entry's own answer, unchanged and
not thrown. Converting before asking is the caller's step; see
[c-abi.md](c-abi.md).

## Threads

Const accessors on an immutable owner are safe from several threads at once
while each thread holds a live owner. Take a `clone()` per thread; retain and
release are the C ABI's, and are atomic.

```cpp
madopilot::MatchResult mine = result.clone();
std::thread reader([mine = std::move(mine)]() mutable {
    const auto info = mine.describe();
});
```

Moving, resetting, or destroying the last owner concurrently with an unprotected
call on it is invalid caller behaviour, exactly as it is in C: no check can
distinguish it from a valid handle without racing the release it is trying to
detect.

Session close races an in-flight operation safely, and both sides observe the
terminal outcomes the C ABI defines.

## What Phase 1 does not wrap

The Phase 1 C table ends at match-result access, and the wrapper wraps only what
is in it. There is no input, OCR, watcher, query, callback, callback-fence, or
native-frame type. `crates/bindings/capi/tests/cpp_surface.rs` asserts that,
so a later phase's surface cannot appear in C++ before it exists in C.

`Api::table()` is the escape hatch: it returns the negotiated
`const madopilot_api_t*` for a caller that needs an entry the wrapper does not
expose.

## Building against it

### CMake

`crates/bindings/capi/CMakeLists.txt` is the `MadoPilot` project. It builds none
of MadoPilot's own code — `cargo` does that — and defines two targets:

| Target | What it carries |
|---|---|
| `MadoPilot::C` | the include directory and the built library, imported |
| `MadoPilot::Cpp` | the same include directory, C++17, and `MadoPilot::C` |

```cmake
cmake_minimum_required(VERSION 3.22)
project(my-app LANGUAGES CXX)

add_subdirectory(/path/to/mado-pilot/crates/bindings/capi madopilot)

add_executable(my-app main.cpp)
target_link_libraries(my-app PRIVATE MadoPilot::Cpp)
```

```sh
cargo build --locked --package mado-pilot-capi
cmake -S . -B build -DMADOPILOT_ARTIFACT_DIR=/path/to/mado-pilot/target/debug
cmake --build build --config Release
```

`MADOPILOT_ARTIFACT_DIR` names the cargo profile directory holding the built
library. It defaults to `target/debug` relative to the repository, and configure
fails with the `cargo build` command to run when the library is not there.

There is **no install or export set**, so `find_package(MadoPilot)` does not
work. Release packaging is not implemented — the ABI-major decorated loader
names are part of it — and an export set would describe an installed layout the
project does not produce. Consumption is from the development tree.

No generator is named. Ninja is not guaranteed on either release target's
runner; MSBuild and Unix Makefiles are.

### Without CMake

The wrapper is one header. A compiler invocation is enough:

```sh
cargo build --locked --package mado-pilot-capi
c++ -std=c++17 -I crates/bindings/capi/include \
    -o my-app main.cpp \
    target/debug/libmadopilot.dylib -Wl,-rpath,target/debug
```

On Windows, compile with `/std:c++17 /EHsc`, link `target\debug\madopilot.dll.lib`,
and put `target\debug` on `PATH` before running. `/EHsc` is needed because the
standard library the wrapper uses assumes exceptions are enabled, even though the
wrapper never throws one.

## How the wrapper is verified

```sh
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

That one command covers the C surface and the C++ surface together. For the C++
half it:

1. compiles and runs `tests/cpp/madopilot-cpp-ownership.cpp`, whose
   `static_assert`s prove the move-only shape and the lvalue-only view accessors
   at compile time, and whose checks prove clone independence, parent and child
   lifetime, borrowed-view stability, error release under a throwing copy,
   zero-match success, close reporting, and concurrent const access at run time;
2. compiles and runs `examples/cpp/deterministic-slice.cpp` and requires it to
   print the same match rectangles and scores as the C example, so a wrapper
   that changed an answer fails rather than reporting a different one;
3. configures, builds, and runs the CMake consumer project under CTest.

It needs a C++ compiler and **CMake 3.22 or later** in addition to the C
compiler. Both are the release target's own on both hosts; set `CXX` or `CMAKE`
to name a different one. On Windows, run it from a Developer Command Prompt: the
same environment that puts `cl` on `PATH` also sets `VSINSTALLDIR`, through
which the check finds the CMake that Visual Studio ships.

It has been run against CMake 3.29.5 on the Windows verification host and CMake
4.4.0 on the macOS one, which is a CMake 3 and a CMake 4 either side of the
declared minimum; the versions are recorded with the layout evidence in
[evidence/c-abi/](evidence/c-abi/).

The checks that need no C++ compiler are in
`crates/bindings/capi/tests/cpp_surface.rs` and run under plain `cargo test`:
the declared type inventory, the absence of later-phase concepts, one owner per
reference-counted handle, and that no C enumerated value is restated.
