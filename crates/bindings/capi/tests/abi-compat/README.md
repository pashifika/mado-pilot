# Frozen header compatibility fixtures

Each subdirectory is one released C header, kept exactly as it was released,
together with a C program written against it. `c-abi-check` compiles every
fixture against its own header and links it to the library built now.

This is what makes the ABI-major promise checkable instead of stated. Preserving
field and function-table ordering within a major version is a rule that costs
nothing to write down and is easy to break by accident; a fixture that fails to
compile, fails to negotiate, or answers differently is the rule enforcing
itself.

| Version | Frozen by | Header | Fixture |
|---|---|---|---|
| [`v1`](v1/) | [ADR 0007](../../../../../docs/adr/0007-phase-1-c-abi-freeze.md), resolving [`G-010`](../../../../../docs/validation-gates.md#g-010) | [`v1/madopilot/madopilot.h`](v1/madopilot/madopilot.h) | [`v1/old-prefix.c`](v1/old-prefix.c) |

## How a fixture is compiled

The fixture's own directory is passed to the compiler **in place of**
`crates/bindings/capi/include`, never alongside it. A fixture that could fall
through to the working header would pass on the day it should fail, so the
frozen `madopilot/madopilot.h` is the only one it can see. The one other include
directory it gets is `crates/bindings/capi/examples`, for the shared
deterministic scene, which declares nothing about the ABI.

## What a fixture checks

Task 9.9 names four verbs, and each is a step of the run:

- **compiles** — against the frozen declarations alone;
- **links** — against the current `cdylib`, through the one exported symbol;
- **negotiates** — twice. Once at the frozen header's own
  `sizeof(madopilot_api_t)`, and once at `MADOPILOT_API_SIZE_INFORMATION`, the
  mandatory prefix. The second is the old-prefix path exercised for real: a
  caller declaring forty bytes against a four-hundred-byte table, told the
  library's size rather than its own. It is also why the fixture is not a
  tautology on the day it is created, when the two headers are still identical.
- **executes** — the whole Phase 1 flow, checking the same match rectangles and
  scores both examples print.

## Adding a fixture

Freeze a header when an ADR freezes it, not when it changes. Copy the header
into a new `v<major>/` directory, write a program against it, and add the
version to `FROZEN_HEADERS` in
[`examples/c-abi-check.rs`](../../examples/c-abi-check.rs).

**Do not edit a fixture that already exists.** Its value is that it is a
snapshot: extending it with coverage of behaviour its header never described
turns a record of what one release promised into a moving target. If a frozen
fixture fails, either the library broke a promise or the promise is being
retired with a new ABI major — and both of those are decisions for an ADR
rather than an edit here.
