# Frozen released-header compatibility fixtures

Each subdirectory is one released C header's declarations — every structure,
field, enumerator and function-table entry as that release declared them —
together with a C program written against it. `c-abi-check` compiles every
fixture against its own header and links it to the library built now.

Declarations rather than the file: a frozen header is never edited, while the
working header keeps gaining comments that declare nothing. Nothing here
compares the two files, and a comparison that did would report those comments as
a difference. What is compared is the frozen declarations against the library
built now, which is what the ABI-major promise is about.

This is what makes the ABI-major promise checkable instead of stated. Preserving
field and function-table ordering within a major version is a rule that costs
nothing to write down and is easy to break by accident; a fixture that fails to
compile, fails to negotiate, or answers differently is the rule enforcing
itself.

| Version | Frozen by | Header | Fixture |
|---|---|---|---|
| [`v1`](v1/) | [ADR 0007](../../../../../docs/adr/0007-phase-1-c-abi-freeze.md), resolving [`G-010`](../../../../../docs/validation-gates.md#g-010) | [`v1/madopilot/madopilot.h`](v1/madopilot/madopilot.h) | [`v1/old-prefix.c`](v1/old-prefix.c) |
| [`v1_2`](v1_2/) | [ADR 0023](../../../../../docs/adr/0023-input-submission-observation-and-abi-1-2.md) | [`v1_2/madopilot/madopilot.h`](v1_2/madopilot/madopilot.h) | [`v1_2/old-prefix.c`](v1_2/old-prefix.c) |
| [`v1_3`](v1_3/) | [ADR 0043](../../../../../docs/adr/0043-ocr-profile-and-zone-public-surfaces.md) | [`v1_3/madopilot/madopilot.h`](v1_3/madopilot/madopilot.h) | [`v1_3/old-prefix.c`](v1_3/old-prefix.c) |
| [`v1_4`](v1_4/) | [ADR 0043](../../../../../docs/adr/0043-ocr-profile-and-zone-public-surfaces.md) | [`v1_4/madopilot/madopilot.h`](v1_4/madopilot/madopilot.h) | [`v1_4/old-prefix.c`](v1_4/old-prefix.c) |
| [`v1_5`](v1_5/) | [ADR 0046](../../../../../docs/adr/0046-onnx-accelerator-provider-policy.md) | [`v1_5/madopilot/madopilot.h`](v1_5/madopilot/madopilot.h) | [`v1_5/old-prefix.c`](v1_5/old-prefix.c) |

## ABI 1.5 snapshot provenance

`v1_5/madopilot/madopilot.h` preserves the exact bytes of
`crates/bindings/capi/include/madopilot/madopilot.h` at the pre-change
`origin/dev/0.5.0` baseline:

- Source commit: `9ad16ae1cfea97421734f63ffd9754a0f7027052`.
- Source tree: `f354b2856b0cc02189eda2df4d0019a411d124f5`, identical to inspected
  commit `483a61298060140365df8341a5ff95758e109226`.
- Header size: `89960` bytes.
- Header SHA-256: `1e7186531f3a9e67a2702c7b71684883fb2e1f8efbee7974068aef693e96632a`.

The standalone caller uses the `v1_4` deterministic matching oracle and exercises
all 90 ABI 1.5 entries, including retained children after parent teardown.
Provider construction checks required records, the complete provider prefix,
closed policy selection, incompatible dependency roots, and cancellation before
prerequisite loading. Descriptor refusals reset provider facts and borrowed views
without writing past the frozen output extent. No external OCR models or OCR
runtime are needed; this fixture does not claim successful provider construction
or accelerator qualification. Earlier frozen headers and callers are unchanged.

## How a fixture is compiled

The fixture's own directory is passed to the compiler **in place of**
`crates/bindings/capi/include`, never alongside it. A fixture that could fall
through to the working header would pass on the day it should fail, so the
frozen `madopilot/madopilot.h` is the only one it can see. The one other include
directory it gets is `crates/bindings/capi/examples`, for the shared
deterministic scene, which declares nothing about the ABI.

## What a fixture checks

Each fixture checks four verbs, and each is a step of the run:

- **compiles** — against the frozen declarations alone;
- **links** — against the current `cdylib`, through the one exported symbol;
- **negotiates** — twice. Once at the frozen header's own
  `sizeof(madopilot_api_t)`, and once at `MADOPILOT_API_SIZE_INFORMATION`, the
  mandatory prefix. The second is the old-prefix path exercised for real: a
  caller declaring forty bytes against a four-hundred-byte table, told the
  library's size rather than its own. It is also why the fixture is not a
  tautology while the frozen declarations and the working ones still agree.
- **executes** — every entry that header declares, retaining the deterministic
  match rectangles and scores and adding that header's contract assertions.

## Adding a fixture

Freeze a header when an ADR freezes it, not when it changes. Copy the header
into a new version directory, write a program against every entry it declares,
and add the version to `FROZEN_HEADERS` in
[`examples/c-abi-check.rs`](../../examples/c-abi-check.rs).

**Do not edit a fixture that already exists.** Its value is that it is a
snapshot: extending it with coverage of behaviour its header never described
turns a record of what one release promised into a moving target. If a frozen
fixture fails, either the library broke a promise or the promise is being
retired with a new ABI major — and both of those are decisions for an ADR
rather than an edit here.

## A fixture is not exemplary code

That rule is worth stating in the direction it is usually met from. A fixture is
held to compiling, linking, negotiating, and answering correctly; it is not held
to the standards a reviewer would apply to an example, and reading it as one
produces findings that are true and not worth its frozenness.

`v1/old-prefix.c` is the standing case. It passes `&error` to eight entries and
releases the handle on one failure path, so a run in which an entry fails leaks
whatever error handles the later ones produced. That path exists only when the
fixture is already reporting a failure and `c-abi-check` is about to fail the
run; on a passing run every `out_error` stays null and `error_release(NULL)` is
the documented no-op, so nothing leaks. Raising it again is a false positive.
Editing the fixture to fix it would spend the snapshot on a leak that only
occurs in a process about to exit as failed.
