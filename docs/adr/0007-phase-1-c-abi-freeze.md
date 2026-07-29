# ADR 0007: The Phase 1 C ABI freeze

- **Status:** Accepted
- **Date:** 2026-07-28
- **Resolves gate:** `G-010` from [../validation-gates.md](../validation-gates.md)
- **Supersedes:** _none_

## Context

Phase 1 built a C ABI with an exported `madopilot_get_api`, a hand-written header
([ADR 0004](0004-c-header-authorship-and-abi-verification.md)), and a header-only
C++ wrapper over it ([ADR 0005](0005-cpp-wrapper-shape-and-cmake-surface.md)).
Every value and layout it carries has been marked provisional since it was
written, because a C ABI can freeze a mistake permanently and the evidence needed
to freeze one well did not exist yet.

It exists now. Six environments — two macOS hosts, two Windows hosts, and both CI
runners, spanning two MSVC majors, two OpenCV minors, and a CMake 3 against a
CMake 4 — produce a **byte-identical 222-line layout report**, measured twice per
host, once by `rustc` and once by that host's C compiler. Sixty-nine tests in six
suites cover owned-handle lifecycles, independent failure-output initialization,
structure-size negotiation in both
directions, pointer and output-state validation, error ownership, and panic
containment. Both examples run the same flow and are required to print the same
match rectangles and the same scores.

Two rows of the gate's evidence table were outstanding: the review that freezes
the Rust-error-to-C-status mapping, and the frozen old-prefix fixture. This change
does both.

`G-010` blocks the ABI compatibility baseline, and therefore every later
old-header-prefix compatibility claim. Nothing can be promised about ABI
stability until it is closed.

## Decision

The C ABI at `crates/bindings/capi/include/madopilot/madopilot.h`, as of this
change, is **ABI major 1, minor 0**, and is frozen. What follows is what "frozen"
covers.

### Status values

Thirteen values, and these numbers are permanent for ABI major 1.

| Value | Name |
|---|---|
| 0 | `MADOPILOT_STATUS_OK` |
| 1 | `MADOPILOT_STATUS_INVALID_ARGUMENT` |
| 2 | `MADOPILOT_STATUS_UNSUPPORTED` |
| 3 | `MADOPILOT_STATUS_CANCELLED` |
| 4 | `MADOPILOT_STATUS_DEADLINE_EXCEEDED` |
| 5 | `MADOPILOT_STATUS_CLOSED` |
| 6 | `MADOPILOT_STATUS_TARGET_LOST` |
| 7 | `MADOPILOT_STATUS_LIMIT_EXCEEDED` |
| 8 | `MADOPILOT_STATUS_CAPTURE_FAILED` |
| 9 | `MADOPILOT_STATUS_ASSET_INVALID` |
| 10 | `MADOPILOT_STATUS_VISION_FAILED` |
| 11 | `MADOPILOT_STATUS_INTERNAL` |
| 12 | `MADOPILOT_STATUS_INTERNAL_PANIC` |

The error-category, asset-fault, and asset-stage enumerations in the same header
are frozen on the same terms: existing values keep their numbers, and a later
phase may only append.

### The Rust-to-C status mapping

One arm per `mado_pilot::Status`, in `crates/bindings/capi/src/status.rs`. Values
0 through 11 mirror the Rust vocabulary exactly, one for one, and the mapping is
frozen with the values.

`MADOPILOT_STATUS_INTERNAL_PANIC` is the **only** C-only status. It exists because
a panic caught at the boundary is a fact about this library that has no Rust
`Status`: a Rust caller sees an unwind, and a C caller must see something.

`Status` is `#[non_exhaustive]`, so the mapping ends in a catch-all that reports
`MADOPILOT_STATUS_INTERNAL`. That is the reviewed choice, not an oversight. A new
Rust status is invisible to a C caller until an ABI minor adds a value for it,
and reporting it as `INTERNAL` is the honest answer in the meantime — the
alternative, reusing the nearest existing status, would tell a C caller something
specific and wrong.

Category is a second axis and is chosen at the call site, not derived from the
status. The same status reaches C with different categories depending on which
subsystem produced it, which is what lets `MADOPILOT_STATUS_INVALID_ARGUMENT`
distinguish a boundary refusal (`_ABI`) from a template identity a package does
not declare (`_ASSET`).

A boundary refusal does not go through the mapping above at all, because it
originates here rather than in the facade. The two the Phase 1 prefix produces
are a malformed request — a null pointer with a length, a size below a mandatory
prefix, an unrecognized tag, an overflowing span — and a **caller-supplied region
whose coordinate space is not `MADOPILOT_SPACE_CAPTURE_PIXELS`**. Both are
`MADOPILOT_STATUS_INVALID_ARGUMENT` with `MADOPILOT_ERROR_CATEGORY_ABI`.

The second one is worth naming because the Rust facade answers the equivalent
question differently: it has a coordinate transform, so it returns its own
unsupported-coordinate outcome, which maps to `MADOPILOT_STATUS_UNSUPPORTED`.
This prefix has no conversion entry, so a region in a space it does not read is
a request it cannot interpret rather than one it read and cannot satisfy — the
same answer an unrecognized space tag gets, and the distinction that keeps
`MADOPILOT_STATUS_UNSUPPORTED` meaning "read and unsatisfiable". The consequence
is that the C prefix is deliberately narrower than the Rust surface here, and a
C caller converts before it asks. `docs/c-abi.md` states the rule where a caller
reads it, `crates/bindings/capi/include/madopilot/madopilot.h` states it at both
region fields, and `crates/bindings/capi/tests/abi.rs` asserts the status and the
category for all four unaccepted spaces on both entries so the two cannot drift.
A later phase that appends a conversion entry may revisit which status a
convertible-but-unsupported space gets; appending one does not change this rule
for the regions frozen here.

### The mandatory table prefix

```c
#define MADOPILOT_API_SIZE_INFORMATION \
    (offsetof(madopilot_api_t, status_text) + sizeof(void*))
```

Forty bytes on both release targets: everything through `status_text`. A caller
that knows less than this cannot report what it loaded and cannot build a
deadline, so negotiation refuses it rather than handing back a table it could not
use. `madopilot_get_api` refuses a different ABI major with
`MADOPILOT_STATUS_UNSUPPORTED` and a size below the prefix with
`MADOPILOT_STATUS_INVALID_ARGUMENT`, and nulls its output in both cases.

### Structure layouts

Semantic numeric values and frozen version/report fields use fixed-width C
integer types: structure sizes and reported table sizes are `uint32_t`, while row
strides and semantic result/package counts are `uint64_t`. `size_t` is limited to
ABI-native addressability quantities: pointer-view lengths, replay input counts
and element strides, target-list counts, accessor indexes, and the caller-known
table extent passed to negotiation. Both release targets are 64-bit, and those
deliberate choices are part of the frozen ABI-1.0 layout rather than implicit
Rust-size leaks.

Frozen as measured. The complete per-field report is the tracked evidence in
[../evidence/c-abi/](../evidence/c-abi/), one file per release target, and the
two are byte-identical. The type-level totals:

| Structure | Size | Align | Mandatory prefix |
|---|---|---|---|
| `madopilot_str_t` | 16 | 8 | — |
| `madopilot_bytes_t` | 16 | 8 | — |
| `madopilot_pixel_rect_t` | 20 | 4 | — |
| `madopilot_build_info_t` | 56 | 8 | 20 |
| `madopilot_operation_t` | 24 | 8 | 8 |
| `madopilot_frame_stamp_t` | 40 | 8 | 40 |
| `madopilot_frame_info_t` | 56 | 8 | 24 |
| `madopilot_image_t` | 72 | 8 | 48 |
| `madopilot_target_t` | 56 | 8 | 24 |
| `madopilot_session_info_t` | 32 | 8 | 32 |
| `madopilot_open_request_t` | 16 | 4 | 8 |
| `madopilot_map_request_t` | 36 | 4 | 12 |
| `madopilot_match_options_t` | 24 | 8 | 8 in, 24 out |
| `madopilot_find_request_t` | 56 | 8 | 24 |
| `madopilot_match_t` | 56 | 8 | 56 |
| `madopilot_result_info_t` | 72 | 8 | 72 |
| `madopilot_package_info_t` | 64 | 8 | 64 |
| `madopilot_template_info_t` | 64 | 8 | 64 |
| `madopilot_error_detail_t` | 56 | 8 | 16 |
| `madopilot_replay_frame_t` | 56 | 8 | 40 |
| `madopilot_source_t` | 64 | 8 | 48 |
| `madopilot_package_source_t` | 40 | 8 | 24 |
| `madopilot_api_t` | 424 | 8 | 40 |

Every opaque handle is a pointer: size 8, align 8.

`madopilot_match_options_t` is the one structure used in both directions, and it
has a different mandatory prefix each way. A caller *supplying* options need only
declare `struct_size` and `flags`, because a structure that sets no presence bit
is the documented way to ask for the template's defaults. A caller *receiving*
them must be able to store all three, because a report that dropped one would
answer `result_options` with a partial truth.

Field order within a structure and entry order within the function table are
frozen for ABI major 1. A later minor appends only, and a caller discovers what
exists from the smaller of its own `sizeof` and the returned table's
`struct_size`.

### Output states

Every function-table entry returns a status and reports values only through
outputs, and every independently valid output is initialized to its documented
failure state *before* the request is validated: owned handle outputs to null,
structures through their failure prefix, scalars to zero. An invalid sibling
output does not short-circuit that initialization. On failure valid outputs stay
in their failure state. An entry taking `out_error` may be passed null there and
then reports the status only.

### Ownership

Every owned handle has `retain`, `release`, and a complete lifecycle. Release is
a null-safe no-op. Variable-sized results are immutable owned handles, and every
view borrowed from one is valid exactly as long as that handle is retained.
Releasing a session does not close it and does not invalidate frames, mappings,
or results the caller still holds; a result owns the frame it searched, so
correlation survives the release of everything else. Every entry catches a panic
before it can cross into C.

### The six decisions the gate named

1. **The C status vocabulary does not diverge further.**
   `MADOPILOT_STATUS_INTERNAL_PANIC` stays the only C-only value. A distinct
   status for an unsupported ABI was considered and rejected: negotiation is the
   only place it could be observed, it happens before any table exists, and
   `MADOPILOT_STATUS_UNSUPPORTED` already says exactly that.
2. **The root handle stays `madopilot_engine_t`.** The specification calls it a
   context; the Rust type is `Engine` and `madopilot_operation_t` is already the
   per-call context. One name that disagrees with the specification is better
   than two things called "context", and the disagreement is with a document,
   not with an implementation. The specification wording follows the code.
3. **`tmpl` stays, and `template` stays the concept.** C++ reserves the lowercase
   word for members as well as fields, so the C++ builder needs
   `FindRequest::search_for` and no member may ever be spelled `template`. What
   the keyword does not block is the type, so `madopilot::Template` compiles.
   Renaming the concept in C — to `pattern` — would make one name work in both
   languages at the cost of a vocabulary split with Rust, where "template" is the
   domain word and the industry term for what this does. One awkward field name
   and one documented C++ constraint is the smaller price. The single ABI-visible
   occurrence is `madopilot_find_request_t.tmpl`.
4. **`MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` is now reachable.**
   `template_prepare_from_package` resolves the identity against the package
   before asking the backend for anything, so an undeclared identity reports
   `MADOPILOT_ERROR_HAS_ASSET_DETAIL` with that fault and
   `MADOPILOT_ASSET_STAGE_COMMIT`, and names no backend, because none ran. This
   is the same two-step a Rust caller performs through
   `AssetPackage::resolve_template`; see [ADR 0006](0006-public-rust-names-and-compatibility-policy.md).
   The alternative was deleting a declared value, which would have removed the
   one piece of information that distinguishes this mistake from every other
   malformed request.
5. **The asset detail does not generalize, and is not reshaped to pretend it
   might.** It stays two fields on `madopilot_error_detail_t`, guarded by a
   presence flag. A second structured detail is added as its own flag and its own
   fields appended to the same structure, not by reworking this one into a tagged
   union — which would be an ABI break for a generality no second subsystem has
   asked for. The C++ `Result`'s optional accessor is the pattern a second one
   would follow.
6. **The freeze makes a machine-readable C++ mirror safe, and the wrapper still
   will not hand-write one.** A hand-written `enum class` mirror of a C constant
   set fails *silently* when the set grows: it compiles, one value short. That
   risk does not come from the values being unstable, so freezing them does not
   remove it. If a mirror is added later it must be generated from this header as
   part of the build, and until something generates it the wrapper keeps
   aliasing the C types so that the two cannot drift.

### The compatibility fixture

`crates/bindings/capi/tests/abi-compat/v1/` holds this header's declarations as
frozen — every structure, field, enumerator and function-table entry — plus a C
program written against it. `c-abi-check` compiles that program with the frozen
directory **in place of** the working include path, links it to the library
built now, and runs it. On the day of the freeze the two files were identical
and the check passed trivially; the fixture is created while it is trivial so
that it exists on the day it is not. The working header has gained comments
since, which costs the fixture nothing: no check compares the two files, and
what the frozen declarations are checked against is the library.

It is not entirely trivial even now: the program negotiates twice, once at the
frozen header's full 424 bytes and once at the 40-byte mandatory prefix, and
requires the library to report its own table size rather than the caller's. That
is the old-prefix path exercised for real.

## Alternatives

**Leave `G-010` open and exit Phase 1 without an ABI baseline.** Not available:
the gate blocks Phase 1 exit, and the reason it does is that every later
compatibility claim is measured against some frozen instance. Deferring means
each later phase freezes against nothing.

**Freeze the layout but not the status values**, on the grounds that statuses are
easier to get wrong. Rejected: a status number is exactly as much of the ABI as a
field offset, and a caller that switches on `9` and later gets a different
meaning has no way to detect it. Partial freezing communicates less than no
freezing.

**Generate the header from the Rust definitions**, removing the possibility of
disagreement rather than testing for it. Rejected in
[ADR 0004](0004-c-header-authorship-and-abi-verification.md) and not reopened
here; the cross-language layout probe is what makes the hand-written header
safe, and it is the same evidence this freeze rests on.

**Rename the concept in C from `template` to `pattern`** so one word works in
both languages. Costed above under decision 3: it splits the vocabulary with
Rust, the C++ wrapper, the documentation, and the domain.

**Keep `session_frame` and `template_prepare` unchanged** rather than renaming
them to `session_acquire_frame` and `template_prepare_from_package` in the same
change that freezes them. Rejected because it is the last moment either can be
renamed. `session_frame` was the one noun among the session's verbs and can
block; `template_prepare` would become ambiguous the moment a bytes-based form is
added, and adding `_from_package` then would leave the older, vaguer name as the
one every existing caller uses. Both renames cost nothing today and are
impossible tomorrow. Their offsets did not move.

## Consequences

- **The compatibility promise begins now.** For ABI major 1: no value changes, no
  field moves, no table entry moves, and nothing is removed. Additions go at the
  end of a structure or the end of the table and raise the minor. Anything else
  is ABI major 2, which is a different library that a v1 caller's negotiation
  refuses by design.
- **This freeze is independent of the Rust one.** [ADR 0006](0006-public-rust-names-and-compatibility-policy.md)
  governs the Rust names and permits a rename under `0.x` with an ADR; this one
  does not. A Rust rename after this point does **not** propagate to C, and the
  two surfaces will diverge in spelling the first time that happens. That is the
  cost of separate versioning and is preferable to letting a Rust decision break
  a released binary contract.
- **Every later phase adds a fixture rather than editing one.** The rule is in
  [`tests/abi-compat/README.md`](../../crates/bindings/capi/tests/abi-compat/README.md):
  a fixture's value is that it is a snapshot, so extending one with coverage its
  header never described turns a record of a promise into a moving target.
- **`c-abi-check` is now a seven-step check** and takes marginally longer on both
  CI runners: one more C compile, link, and run per frozen header. The list will
  grow by one entry per ABI major, which is the intended rate.
- **The layout evidence files are regenerated in this change**, because two
  function-table entries were renamed. The offsets are unchanged, which is
  visible in the diff and is the point.
- **`G-011` stays deferred.** Native-frame extension discovery is deliberately
  outside this freeze; nothing here reserves a slot for it, and adding one would
  be freezing a shape no prototype has produced.
