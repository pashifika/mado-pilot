# The C ABI

MadoPilot's C boundary is a separately versioned contract with its own
compatibility rules. This document is the part of it that does not fit in a
declaration: what a handle's lifetime is, what a `struct_size` means in each
direction, which status a caller can branch on, and how to build against the
library on each release target.

The declarations themselves live in
[`crates/bindings/capi/include/madopilot/madopilot.h`](../crates/bindings/capi/include/madopilot/madopilot.h),
and a complete working caller is
[`crates/bindings/capi/examples/c/deterministic-slice.c`](../crates/bindings/capi/examples/c/deterministic-slice.c).

## Nothing here is frozen yet

Every status value, structure layout, field offset, and function-table position
is **provisional**. Gate [`G-010`](validation-gates.md#g-010) freezes them once
the Phase 1 evidence has been reviewed, and until then they may change without an
ABI major bump. A caller recompiles against the header it links with.

What is *not* provisional is the shape of the rules below. Those are what
`G-010` will be freezing an instance of.

## One exported symbol

`madopilot_get_api` is the only symbol the library exports. Everything else is a
member of the immutable function table it returns.

```c
const madopilot_api_t* api = NULL;
madopilot_status_t status = madopilot_get_api(
    MADOPILOT_ABI_MAJOR, MADOPILOT_ABI_MINOR, sizeof(madopilot_api_t), &api);
```

One symbol means one thing to negotiate. A caller that obtained a table has, by
construction, agreed the ABI major, the minimum minor, and how much of the table
it understands; fifty exported symbols would each be a separate chance to bind
to something the negotiation would have refused.

Negotiation refuses a different major and a minimum minor newer than the library
with `MADOPILOT_STATUS_UNSUPPORTED`, and a caller-declared table size below
`MADOPILOT_API_SIZE_INFORMATION` with `MADOPILOT_STATUS_INVALID_ARGUMENT`. A
caller *larger* than the library is not an error: the returned table's
`struct_size` says how much of what the caller knows is really there, and the
caller uses the smaller of the two.

Within an ABI major a member's position is permanent. Later phases append.
Nothing is reordered, removed, repurposed, or reserved as a null slot for work
that does not exist yet.

## Handles

Every owned handle is reference counted.

| Rule | What it means |
|---|---|
| `*_retain` adds one owned reference | Null is a no-op |
| `*_release` drops one owned reference | Null is a no-op; the last one destroys the state exactly once, with the allocator that created it |
| Every other entry rejects null | "Do nothing" is not an answer to a question that has one |
| A handle must stay retained for the whole call it is passed to | Releasing the last reference concurrently with a call that has not retained one of its own is outside the contract, because no check can distinguish it from a valid pointer without racing the release it is trying to detect |
| Const access is safe from several threads at once | While each keeps a live reference |

**Releasing a parent never invalidates a separately retained child.** This is
the rule the whole design turns on, and it is what makes the following true:

- a mapping stays readable after the frame, the session, and the engine are
  released, because the mapping owns or retains the storage its byte view
  borrows;
- a prepared template outlives the package it was compiled from;
- a match result outlives the session, the template, the package, and the
  engine, because the result owns the exact frame it searched. That is what
  keeps "which frame is this about" answerable at any later point.

**A borrowed view is valid only while its owner is retained.** Each declaration
names the owner. `madopilot_error_detail_t.message` borrows from the error
handle, `madopilot_match_t.template_id` borrows from the result, a target's
`name` borrows from the target list, and `madopilot_image_t.bytes` borrows from
the mapping. Copy anything you still need before the final release.

## Size-versioned structures

Every extensible structure begins with `uint32_t struct_size`, immediately
followed by a second 32-bit field so that no implicit padding is introduced
between them. That second field is `flags` where the structure has presence bits
and a meaningful discriminant where it has one.

A caller sets `struct_size` to `sizeof` the structure **as its own header
declares it**.

**Reading an input**, the library:

- refuses a size below the documented mandatory prefix with
  `MADOPILOT_STATUS_INVALID_ARGUMENT`, without reading past `struct_size`;
- applies the documented default to every field the size omits;
- ignores trailing bytes it does not recognize.

**Writing an output**, the library:

- refuses the same way;
- writes only within the declared size;
- writes back the number of bytes it actually populated, so a caller built
  against a newer header learns how much of what it knows is really there.

Two structures carry no `struct_size`: `madopilot_str_t` and
`madopilot_bytes_t`. They are the boundary's primitives rather than extensible
records — they appear inside other structures, so growing one would move every
field after it. A later phase that needs more than a pointer and a length
introduces a different type.

An array of versioned structures needs its element stride passed explicitly:
`madopilot_source_t.frame_stride` is `sizeof(madopilot_replay_frame_t)` as the
caller's header declares it. A caller built against an older header has smaller
elements, and the library cannot guess the spacing of an array it did not
declare.

## Validation, and what an output looks like on failure

Before a request is validated, every valid output is set to its documented
failure state: an owned handle output to null, a structure through its failure
prefix, a scalar to zero. On failure they stay that way, so a caller never sees a
partially initialized value.

Validated before use: every pointer-length pair, every active tagged-source
field, every integer conversion, every alignment requirement, every offset,
every element stride, and every allocation-size calculation. A null pointer with
a nonzero length is rejected before the pointer is read; a null pointer with a
zero length is accepted only where the declaration documents an empty view as
meaningful.

The library does not probe arbitrary addresses. The caller remains responsible
for the validity of the addresses it passes, for the declared duration of the
call.

## Deadlines and cancellation

Every potentially blocking entry takes a `madopilot_operation_t`. Its deadline
is an **absolute instant** in the library's own monotonic domain, in nanoseconds
since an origin fixed for the life of the loaded library. Read the current
instant with `clock_now` and add to it.

A duration would restart at every hop; an absolute instant means the same moment
everywhere it is carried. The origin is not a wall-clock time and must not be
presented as one.

The implementation checks cancellation and the deadline before admission and
again before committing a successful result, so a value that loses the race is
dropped rather than published. Each contract underneath does the same, and in
the Phase 1 pipeline an inner one usually observes an interruption first — that
is the intent rather than a redundancy.

## Statuses, and the one place a status is not enough

A caller branches on `madopilot_status_t`. The message an error handle carries is
diagnostic and is never required for control flow.

| Status | Meaning |
|---|---|
| `MADOPILOT_STATUS_OK` | Every required output is populated |
| `MADOPILOT_STATUS_INVALID_ARGUMENT` | Malformed, out of range, or naming something unknown |
| `MADOPILOT_STATUS_UNSUPPORTED` | Well-formed, but this build cannot satisfy it |
| `MADOPILOT_STATUS_CANCELLED` | The token was set before the result committed |
| `MADOPILOT_STATUS_DEADLINE_EXCEEDED` | The deadline passed before the result committed |
| `MADOPILOT_STATUS_CLOSED` | The session starts no further work |
| `MADOPILOT_STATUS_TARGET_LOST` | The capture target no longer exists |
| `MADOPILOT_STATUS_LIMIT_EXCEEDED` | A configured or implementation limit |
| `MADOPILOT_STATUS_CAPTURE_FAILED` | Capture could not produce the frame |
| `MADOPILOT_STATUS_ASSET_INVALID` | A package broke a rule that makes it trustworthy |
| `MADOPILOT_STATUS_VISION_FAILED` | The backend was unavailable or could not finish |
| `MADOPILOT_STATUS_INTERNAL` | An invariant the library owns did not hold |
| `MADOPILOT_STATUS_INTERNAL_PANIC` | A Rust panic was contained at the boundary |

`MADOPILOT_STATUS_INTERNAL_PANIC` is the boundary's own status and has no Rust
counterpart.

**Package loading carries more than a status.** Every other operation in the
facade reports a status plus diagnostic text. Package loading reports which rule
was broken *and* how far loading had got, deliberately: a bad content hash and an
unsafe entry path are both `MADOPILOT_STATUS_ASSET_INVALID` and are not the same
problem. `madopilot_error_detail_t` therefore carries `asset_fault` and
`asset_stage` alongside the status, flagged by
`MADOPILOT_ERROR_HAS_ASSET_DETAIL`. Flattening them into one status here would
have thrown away detail the Rust layer took care to keep.

A related line worth knowing: asking a **loaded** package for a template identity
it never declared is `MADOPILOT_STATUS_INVALID_ARGUMENT`, not
`MADOPILOT_STATUS_ASSET_INVALID`. A package that loaded is valid; the mistake is
the caller's. The error's category is still `MADOPILOT_ERROR_CATEGORY_ASSET`,
because the mistake is about the package's contents.

There is no global, thread-local, or engine-wide last-error slot. A failure
belongs to the call that produced it, and a slot would make two threads' failures
each other's business. `out_error` may be null, and then only the status is
reported.

## Panic containment

Every exported symbol and every table entry contains a Rust panic before it can
cross into C. A contained panic returns `MADOPILOT_STATUS_INTERNAL_PANIC`, leaves
every valid output in its failure state, releases whatever the unwinding call had
allocated, and poisons nothing: handles unrelated to the failed call remain
usable, and repeating the call is expected to work.

Containment requires an unwinding panic profile. An advertised C ABI build must
not be compiled with `panic = "abort"`.

## What Phase 1 does not contain

The Phase 1 table ends at match-result access. There is no entry for input
delivery, OCR model loading or recognition, watchers, query handles, callbacks,
callback unregistration, or platform-native frame extensions, and none of them is
reserved as a null slot. A later phase appends them.

## Building against the library

### Artifacts

`mado-pilot-capi` builds as a `cdylib` plus an ordinary Rust `lib`.

| Target | Built | Reserved release name |
|---|---|---|
| `aarch64-apple-darwin` | `libmadopilot.dylib` | `libmadopilot.1.dylib` |
| `x86_64-pc-windows-msvc` | `madopilot.dll`, `madopilot.dll.lib` | `madopilot-1.dll`, `madopilot-1.lib` |

The ABI-major decorated names in the right column are what a release ships, so
that an incompatible ABI is a different library rather than a silent breakage.
Applying them is a release-packaging step — an install name on macOS, a linked
file name and matching import library on Windows — and Phase 1 does not implement
packaging. What is built today is the undecorated development artifact.

No `staticlib` is produced. Gate [`G-008`](validation-gates.md#g-008) has not
recorded which static dependency combinations are supported, and emitting the
artifact would advertise a claim the project has not made.

### Prerequisites

The library links OpenCV dynamically, so a host that runs a C program against it
needs the same OpenCV a Rust host does; see
[third-party-dependencies.md](third-party-dependencies.md). An absent OpenCV
stops the process at load time, before any MadoPilot code runs, so it is not
reachable as a status — recorded against [`G-007`](validation-gates.md#g-007).

Compiling the C example additionally needs a C toolchain. Both are the release
target's own, and neither CI runner nor either verification host installs
anything extra for it:

| Target | Compiler | Flags used by the check |
|---|---|---|
| `aarch64-apple-darwin` | Xcode Command Line Tools `cc` | `-std=c11 -Wall -Wextra` |
| `x86_64-pc-windows-msvc` | MSVC `cl` | `/std:c11 /W3` |

Set `CC` to choose a different compiler.

**On Windows, run the check from a Developer Command Prompt**, or call
`vcvars64.bat` first. `cl` is not on `PATH` in a plain shell even when Visual
Studio is installed, and it needs `INCLUDE` and `LIB` set to find the C runtime
headers and import libraries. `c-abi-check` says so when it cannot launch the
compiler. The Windows CI job discovers the install path with `vswhere` and calls
`vcvars64.bat` itself, so nothing has to be hard-coded there either.

One Windows-specific trap is worth knowing because it looks like a missing file:
MSVC cannot open a source or include path in the `\\?\C:\...` extended-length
form that `std::fs::canonicalize` returns, and reports **C1083** as though the
file did not exist. `c-abi-check` strips that prefix from every path it hands to
a compiler.

### Compiling

```sh
cargo build --locked --package mado-pilot-capi
cc -std=c11 -I crates/bindings/capi/include \
   -o deterministic-slice \
   crates/bindings/capi/examples/c/deterministic-slice.c \
   target/debug/libmadopilot.dylib -Wl,-rpath,target/debug
./deterministic-slice --package fixtures/assets/phase1-slice
```

On Windows, link `target\debug\madopilot.dll.lib` and put `target\debug` on
`PATH` before running.

## How the header is verified

The header is hand-written and tracked, not generated; the reasoning is in
[ADR 0004](adr/0004-c-header-authorship-and-abi-verification.md). Its agreement
with the Rust `#[repr(C)]` definitions is proved rather than asserted:

```sh
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

That compiles and runs `tests/c/madopilot-abi-layout.c`, which reports every
size, alignment, and field offset as the C compiler produced them; compares the
report line by line against the same values measured from the Rust definitions;
and then compiles, links, and runs the C example and checks its outcome. Two
compilers, one comparison — a divergence names the structure and the field.

The invariants that hold without a C compiler — `struct_size` first, mandatory
prefixes that land on field boundaries, thin handle pointers, and the
function-table order — are checked by `cargo test` in
`crates/bindings/capi/tests/layout.rs`.

Both native CI jobs run the C check on every pull request. The Ubuntu
repository-policy job deliberately compiles nothing and does not.

## When `G-010` resolves

The gate is resolved by an ADR that records the exact status values, the
mandatory table prefix, the structure sizes, alignments, and field offsets, and
the Rust-error-to-C-status mapping. The same change copies the then-current
header into a fixture directory unchanged, and every later library in the same
ABI major is compiled, linked, and run against that frozen fixture. Until that
happens, the repository does not describe any of these values as stable.
