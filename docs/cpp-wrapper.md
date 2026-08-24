# The C++ wrapper

MadoPilot's C++ surface is a header-only RAII adapter over the released C ABI.
It owns handles, turns statuses into an exception-free `Result`, and marks
borrowed views. It performs no capture, matching, OCR, coordinate conversion,
normalization, scheduling, fallback, or status logic of its own: every answer
comes from a negotiated C table entry.

The declarations are in
[`crates/bindings/capi/include/madopilot/madopilot.hpp`](../crates/bindings/capi/include/madopilot/madopilot.hpp).
The replay workflow is
[`examples/cpp/deterministic-slice.cpp`](../crates/bindings/capi/examples/cpp/deterministic-slice.cpp);
the cross-target native workflow is
[`examples/cpp/native-input.cpp`](../crates/bindings/capi/examples/cpp/native-input.cpp).
The one-shot OCR fixture flow is
[`examples/cpp/ocr-fixture.cpp`](../crates/bindings/capi/examples/cpp/ocr-fixture.cpp).
The contract underneath is [c-abi.md](c-abi.md). Read that one first: every
rule here is a rule about how the C rules are expressed in C++, and none of them
replaces one.

## This wrapper declares no ABI of its own

The only ABI is the C one. ABI 1.0 and 1.2 are frozen complete prefixes under
ADRs 0007 and 0023. ABI 1.3 appends one-shot OCR and immutable owned OCR results
under [ADR 0035](adr/0035-ocr-public-surfaces-and-private-fixture-boundary.md).
The wrapper adds source compatibility only, governed by ADR 0006's reviewed
Rust-side naming policy.

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
| `Cancellation`, `Engine`, `TargetList`, `Package`, `Template`, `Session`, `Frame`, `Mapping`, `MatchResult`, `OcrResult`, `InputReceipt`, `DiagnosticReader`, `DiagnosticBatch` | one reference-counted C handle | `clone()` |

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
refuses it with `MADOPILOT_STATUS_INVALID_ARGUMENT`. The wrapper itself returns
`MADOPILOT_STATUS_INVALID_ARGUMENT` when there is no table to ask and
`MADOPILOT_STATUS_UNSUPPORTED` when the negotiated prefix omits a required
entry.

Any method that returns a new owner requires the complete negotiated lifecycle
and accessor suffix that owner exposes before invoking its creating C entry.
`Session::recognize` therefore requires through
`MADOPILOT_API_SIZE_OCR_RESULT_TEXT_AT`; `Session::send_input` and diagnostic
reader creation apply the same rule to their own complete suffixes. A C caller
may use a shorter individual-entry prefix directly; C++ refuses an owner it
could not safely clone, inspect, and destroy.

### Parents and children

The C rule is that releasing a parent never invalidates a separately retained
child. `Mapping` outlives frame/session/engine; `Template` outlives package;
`MatchResult` outlives its search parents; `OcrResult` outlives frame, package,
session, engine, and backend; receipts and diagnostic batches outlive their
producers. Borrowed OCR text and result-description strings remain valid only
while their `OcrResult` owner is retained.

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

The wrapper can throw only its own allocation failures: copying error text,
collecting match vectors, owning OCR request strings, copying an explicit
borrowed view, or engaging standard-library containers. MadoPilot failures remain
statuses in `Result`. Describing an error releases its C handle even when a text
copy throws.

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

**The asset detail survives.** Package loading can distinguish failures that
share one status, so
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

**An admitted input outcome is a successful owned value.**
`Session::send_input` returns a failed `Result<InputReceipt>` when no receipt
can be published: validation or table availability failed, the request was
refused before admission, or the boundary contained an internal failure. Once
admitted and returned normally, `Complete`, `Unexecuted`, and `Partial` are all
successful values. `InputReceipt::describe()` reports selected route and address
scope, submitted count and optional last-submitted index, submission evidence,
typed fault, fallback, possible partial native effect, and cleanup state;
`attempt_at()` preserves each route attempt.
Semantic counts in `InputReceiptInfo` and each attempt stay `std::uint64_t`,
exactly as the C records declare them; `attempt_count()` returns the
`std::size_t` index domain that `attempt_at()` consumes.

```cpp
const auto sent = session.send_input(request, operation);
if (!sent) {
    // No receipt exists; inspect the status before deciding what is safe.
    return sent.error();
}
const madopilot::InputReceipt receipt = sent.value().clone();
const auto described = receipt.describe();
if (!described) {
    return described.error();
}
const madopilot::InputReceiptInfo& info = described.value();
if (info.outcome == MADOPILOT_SEQUENCE_PARTIAL &&
    info.may_leave_state_held()) {
    // Incomplete, exhausted, and unknown cleanup values are conservative.
}
```

A zero submitted count does not make a `Partial` retry-safe: the current native
unit may have had an effect before a complete logical event reached the route's
submission threshold. Submission evidence does not claim application
consumption or visual change. A failed result with
`MADOPILOT_STATUS_INTERNAL_PANIC` likewise proves neither and must not be retried
automatically.

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
| `PermissionDiagnostic::platform_namespace`, `context` | the `Engine` |
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

`Operation`, `EngineOptions`, `Source`, `PackageSource`, `InputOpenRequest`,
`InputEvent`, `OpenRequest`, `MapRequest`, `MatchOptions`, `FindRequest`, and
`InputRequest` are values a caller composes. Each fills the C structure's
`struct_size` itself, so no call site can write a stale one. `InputRequest` owns
its typed events and route plan, while each `to_c()` call owns an independent
event-record projection that borrows text and route storage from that request.

```cpp
const madopilot::Result<std::uint64_t> now = api.clock_now();
if (!now) {
    return now.error();
}

madopilot::Operation operation;
operation.deadline(now.value() + 30ull * 1000 * 1000 * 1000)
    .cancellation(token)
    .activity_tag(42);
```

The deadline is an **absolute instant** in the library's monotonic domain, read
from `Api::clock_now()` and added to. It is not a duration or wall clock. The
nonzero activity tag is opaque, non-secret correlation metadata and changes no
operation semantics. A platform route may expose it through documented native
observational metadata; macOS `ProcessDirected` carries it in Core Graphics
event-source user data visible to the addressed process.

Four request values borrow handles rather than owning them, and say so:

- an `Operation` borrows its `Cancellation`, which must outlive every call the
  operation is passed to;
- a `FindRequest` borrows its `Frame` and its `Template`;
- an `OcrRequest` borrows its `Frame` and `Package` through each
  `Session::recognize` call;
- an `InputRequest` borrows its source `Frame`, which must stay retained until
  `Session::send_input` returns.

`FindRequest::search_for` names the prepared template. It is not called
`template`, and cannot be: the word is a C++ keyword, so it cannot name a member
function any more than it can name the C structure field, which is `tmpl` for
the same reason.

Rectangles stay coordinate-qualified. `Match::bounds`, `ResultInfo::searched`,
and `DiagnosticRecord::region` are `madopilot_pixel_rect_t` under the alias
`Rect`, and each names the space it is measured in rather than reducing to an
integer pair.

A caller-supplied `Rect` is narrower than the Rust facade:
`MapRequest::region`, `FindRequest::region`, and `OcrRequest::region` accept
capture pixels only because the C ABI has no conversion entry. Other spaces
return the C entry's `MADOPILOT_STATUS_INVALID_ARGUMENT` unchanged.

### One-shot OCR

`OcrRequest` owns model/backend ID and version strings and borrows one `Frame`
and `Package` for each call. It keeps source region, clip policy, and output
space explicit. Every call constructs a fresh `OcrRequest::CView`; copy/move
construction and assignment rebind all pointer-length fields to that projection's
own storage, including the moved-from object's still-callable accessors.

`Session::recognize` gates on the complete ABI 1.3 OCR suffix before reading any
appended function pointer. Success returns move-only `OcrResult`; `clone()` is
the only refcounting copy. `describe()` returns source identity, effective region,
count, and borrowed descriptors; `region_at()` returns value geometry/confidence;
`text_at()` returns a borrowed normalized string. Borrowing accessors are
lvalue-only and deleted on temporaries. The wrapper adds no backend choice,
runtime loading, normalization, coordinate inference, queue, fallback, or input.

Input capability keeps operation and route separate.
`InputOpenRequest` accepts exact `MADOPILOT_INPUT_PAIR_*` masks;
`OpenRequest::input` selects the ABI 1.2 open entry without changing the frozen C
open record. `TargetList::input_capability` returns compatibility support,
address scope, focus, permission, pointer spaces, and submission evidence for
one exact pair. `InputEvent` factories expose one active variant and copy text.
`InputRequest` copies events and route order, and keeps focus, geometry,
source-frame, and cleanup policies explicit.

The wrapper aliases rather than restates fixed C limits.
`InputEvent::max_text_chars`, `max_text_utf8_bytes`, `max_delay_nanos`,
`max_scroll_notches`, `min_function_key`, and `max_function_key` expose event
ceilings. `InputRequest::abi_max_events`, `max_cleanup_events`, and
`max_cleanup_timeout_nanos` expose sequence and cleanup ceilings. A returned
`InputDescriptor::max_events` may be lower than the ABI-wide sequence ceiling.
See the [C input-limit table](c-abi.md#input-admission-submission-evidence-and-receipts)
for units and inclusive-range rules.

```cpp
madopilot::InputOpenRequest input;
input.requirement(MADOPILOT_INPUT_REQUIRED)
    .require_pairs(MADOPILOT_INPUT_PAIR_POINTER_SYSTEM |
                   MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM);

madopilot::OpenRequest open;
open.input(input);

madopilot::InputRequest request;
request.event(madopilot::InputEvent::pointer_move(
                 MADOPILOT_SPACE_CAPTURE_PIXELS, x, y))
    .delivery(MADOPILOT_INPUT_DELIVERY_SYSTEM)
    .focus_policy(MADOPILOT_FOCUS_REQUIRE_FOCUSED)
    .geometry_policy(MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED)
    .source_frame(frame);
```

`Engine::capabilities`, `Engine::permission`,
`TargetList::input_capability`, `Engine::input_descriptor`, and
`Session::input_descriptor` project ABI 1.2 records into value types. Optional
fields remain `std::optional`; an unknown C numeric value stays in its
fixed-width alias rather than being narrowed into a wrapper enum.

### Engine-scoped diagnostics

Pass `EngineOptions::diagnostics(level, capacity)` to the engine constructor.
The default is `diagnostics_off()` and allocates no queue.
`Engine::take_diagnostic_reader()` returns the engine's single reader as an
optional; diagnostics-off engines and repeated takes return an empty optional.

`DiagnosticReader::drain()` is non-blocking and returns `BATCH`, `OPEN_EMPTY`, or
`END_OF_STREAM`. `DiagnosticBatch` carries exact normal/debug losses and indexed
fixed-width values. OCR records project opaque model-instance ID, accepted
profile classification, source/requested/effective geometry, typed outcome,
count, elapsed nanoseconds, and source-pixel count. They expose no recognized
text, backend/runtime name, caller model/asset identity, model path/digest/bytes,
vocabulary, pixels/hash, credential, or free-form failure. The wrapper performs
no logging, callback dispatch, ordering inference, or causal synthesis.

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

Session close races in-flight operations under the C contract. Two threads may
submit the same immutable `OcrRequest` or `InputRequest`: each call owns an
independent repaired C projection. Clone an immutable result/reader per thread;
retain/release remain atomic. Mutating a request or destroying the final owner
concurrently with a call remains invalid caller behavior.

## What ABI 1.3 does not wrap

The ABI 1.3 table ends at immutable one-shot OCR access. There is no watcher,
wait-for-text, query, callback/fence, retry, automatic action/input, acceleration,
packaging, or native-frame extension. OCR and later input remain separate facts.
`cpp_surface.rs` asserts this inventory.

`Api::table()` is the escape hatch: it returns the negotiated
`const madopilot_api_t*` for a caller that needs an entry this wrapper does not
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
cargo run --locked --package mado-pilot-capi --features private-fixture \
  --example c-abi-check -- --label "<host>"
```

That one command covers the C surface and the C++ surface together. For the C++
half it:

1. compiles and runs `tests/cpp/madopilot-cpp-ownership.cpp`: move-only OCR
   ownership, explicit clone, lvalue-only views, copy/move request rebinding,
   ABI 1.2 and partial ABI 1.3 refusal, parent independence, panic/error release,
   receipts, diagnostics, close, and concurrent const access;
2. runs deterministic matching plus feature-gated C/C++ OCR examples and
   requires identical source/text/confidence observations; the OCR examples also
   validate content-redacted diagnostic projection;
3. compiles and runs `examples/cpp/native-input.cpp`. The default `--check`
   creates the real target Adapter and reads only non-prompting permission state
   before stopping without discovery or input. Windows CI instead asks
   `c-abi-check --windows-native-fixture` to own both repository fixtures and
   pass each exact title to both native language examples. Each language runs
   discovery, capture, mapping, one bounded window-message sequence, immutable
   receipt/attempt inspection, a strictly newer visual observation, diagnostics,
   and explicit close twice: the ordinary contract must report unknown
   compatibility and target-queue admission, while the dedicated contract must
   report supported compatibility and target-protocol acknowledgement. Neither
   run takes focus or permits system fallback;
4. configures, builds, and runs the independent CMake consumer project under
   CTest. That project also builds the native example through `MadoPilot::Cpp`
   alone and runs its safe `--check` mode.

When the caller owns a fixture lifecycle, pass `--ordinary "<full title>"` or
`--acknowledged "<full title>"` directly to the native example. Those modes send
explicit focus-preserving `WindowMessage` on Windows and evaluate a newer frame
separately. On macOS, an exact title selects the explicit process-directed
route: the wrapper flow requires `ProcessDirected` with owning-process scope,
unknown compatibility, and invocation-only evidence, preserves foreground, and
permits no fallback. Run real input only against a target you own and have
selected exactly, as described in the platform verification documents.
This macOS mode exercises the qualified process-directed pairs. Final candidate
`dec43d7` passed the controlled-fixture performance profiles, and independent
`single`, exact two-display non-mirrored `same-scale`, and `mixed-scale`
matrices passed for all fourteen controlled pairs. The mode makes no claim about
arbitrary applications, games, renderers, input stacks, exact-window delivery,
or application consumption.

The check needs a C++ compiler and **CMake 3.22 or later** in addition to the C
compiler. Both are the release target's own on both hosts; set `CXX` or `CMAKE`
to name a different one. On Windows, run it from a Developer Command Prompt: the
same environment that puts `cl` on `PATH` also sets `VSINSTALLDIR`, through
which the check finds the CMake that Visual Studio ships.

The checks that need no C++ compiler are in
`crates/bindings/capi/tests/cpp_surface.rs` and run under plain `cargo test`:
the declared ABI 1.2 type inventory, absence of deferred concepts, one owner per
reference-counted handle, compile-time rejection of removed ABI 1.1 vocabulary,
and proof that no C enumerated value is restated.
