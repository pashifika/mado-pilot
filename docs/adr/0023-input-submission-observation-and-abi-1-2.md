# ADR 0023: Define input submission evidence, bounded observation, and ABI 1.2

- **Status:** Accepted
- **Date:** 2026-08-10
- **Resolves gate:** _none_
- **Supersedes:** ADR 0017

## Context

The implemented input vocabulary exposed `System` and `BackgroundTarget` as if
both named equivalent delivery guarantees. Native evidence disproved that model.
Windows `SendInput` reports admission to the system stream, `PostMessageW`
reports admission to one window's queue, and the controlled fixture alone can
acknowledge consumption of its versioned protocol. macOS `CGEventPost` returns
no consumption result. A receipt field named `delivered` and a
`last_completed` index therefore claimed more than these APIs establish.

The facade also needed bounded correlation between visual work and input without
logging captured pixels, recognized text, key values, window titles, or native
identifiers. The existing redacted error strings could not reconstruct operation
order, route attempts, terminal outcomes, or loss under contention.

ABI 1.0 is released and permanent under [ADR 0007](0007-phase-1-c-abi-freeze.md).
ADR 0017 described an ABI 1.1 development draft, but no product release shipped
that header. Its direct receipt record could not be retained independently and
its route and completion vocabulary encoded the overclaim above. Keeping it
would make those mistakes permanent immediately before release.

## Decision

MadoPilot separates input operation kind from the explicit `System`,
`WindowMessage`, and `ProcessDirected` route. Every operation/route capability
reports compatibility support, recipient address scope, focus behavior,
permission, accepted coordinate spaces, and the strongest truthful submission
evidence. `Supported` requires positive contract evidence; an attemptable route
without application-consumption proof remains `Unknown`.

A terminal receipt records every attempted route and counts only complete logical
events submitted to the selected native API. It reports `Unexecuted`, `Complete`,
or `Partial`, the selected route and address scope, submission evidence, typed
fault, possible partial native effect, fallback, and cleanup. It never claims
that the target application consumed an event or that a visual state changed.

Each engine may optionally own one finite pull-based diagnostic stream. `Off` is
the default and allocates no queue or diagnostic identities. `Normal` retains
terminal public-operation summaries; `Debug` also retains admission and route
attempt detail. Records contain a monotonic timestamp, strict engine-local
sequence, checked operation identity, optional opaque caller activity tag, and a
closed privacy-reviewed payload. Producers never block or call host code. When
the queue or lock cannot accept a record, exact normal/debug loss counts travel
with the next immutable owned batch. Engine close seals production; an
independently retained reader drains to an explicit end-of-stream state.

ABI major 1 minor 2 preserves the complete 424-byte ABI 1.0 function-table prefix
and every released ABI 1.0 value, layout, ownership rule, and behavior. It
replaces the unreleased 1.1 suffix with the reviewed native/input/diagnostic
surface, including size-versioned engine options, route capability and receipt
accessors, owned receipt and diagnostic handles, and 21 appended table entries.
The complete table is 592 bytes on both 64-bit release targets. Minimum minor 1
is rejected; callers built against the development-only 1.1 draft must recompile.
Later ABI 1.x changes append only after this 1.2 extent.

## Alternatives

**Keep `BackgroundTarget` and reinterpret it.** Rejected because one name cannot
distinguish exact-window queue admission from process-directed transport, and
reinterpretation would preserve source compatibility while silently changing a
public claim.

**Preserve `delivered` as a familiar receipt term.** Rejected because neither an
API return count nor queue insertion proves application consumption. `submitted`
and explicit evidence state exactly what the native route established.

**Keep the ABI 1.1 draft and append corrections.** Rejected because it was never
released, and permanently carrying misleading values, records, aliases, and
entry points would enlarge every future ABI without preserving a real consumer.

**Use callbacks or a process-global logger for diagnostics.** Rejected because
callbacks add reentrancy and disable/drain lifetime obligations, while a global
logger cannot provide engine ownership, finite memory, exact loss accounting, or
payload privacy. A caller-owned pull reader makes all four explicit.

**Treat diagnostic timestamps as ordering or causality.** Rejected because equal
or close monotonic observations can come from concurrent producers. The record
sequence is the only total commit order; operation and activity identities are
correlation, not causation.

## Consequences

- Rust, C, and C++ callers must migrate route names, capability queries, and
  receipt accessors; there are no 1.1 aliases or deprecated shims.
- A successful receipt answers submission, not application effect. Visual success
  remains a separate capture/search operation correlated by the caller.
- Enabled diagnostic capacity is `1..=65,536`; `Off` requires zero. Producers may
  discard records rather than block, but each drain reports exact losses since
  the preceding committed batch.
- Diagnostic records retain no captured bytes, recognized text, event payloads,
  window titles, platform namespaces, backend names, or free-form native error
  strings. Adding a payload category requires the same privacy review.
- The C ABI and C++ wrapper negotiate minor 2 and inspect entry extents before
  calling them. ABI 1.0 consumers remain compatible; the unreleased 1.1 draft is
  intentionally unsupported.
- At this decision's acceptance, the Windows fixture advertised `WindowMessage`
  with target-protocol acknowledgement and ordinary targets exposed system
  routes only. [ADR 0027](0027-windows-window-message-queue-submission.md) later
  supersedes that ordinary-target consequence: retained top-level windows may
  expose explicit exact-window `WindowMessage` as `Unknown` with target-queue
  evidence. macOS still exposes system routes only. Neither platform fabricates
  consumption evidence.
- The diagnostic queue is an observability mechanism, not a scheduler, watcher,
  retry engine, or causal action primitive.

## Verification

- Core, input, runtime, facade, and platform contract tests cover route-pair
  admission, evidence, fallback, partial effects, submitted counts, independent
  post-input visual observation, diagnostic levels, order, loss accounting,
  privacy, concurrency, close sealing, and reader lifetime.
- C layout, numeric-freeze, output-prefix, ownership, invalid-input, panic, and
  compatibility tests compare the header and Rust declarations. The ABI 1.0
  caller still negotiates and runs against the ABI 1.2 library; minimum minor 1
  is rejected.
- C++ compile-time and runtime tests cover move-only owned receipts and diagnostic
  batches, retained readers, borrowed-view owners, truncated tables, and the
  absence of the removed ABI 1.1 vocabulary.
- Native C, C++, and Rust examples inspect submission evidence and perform a
  strictly newer visual observation as a separate success oracle. Platform
  fixture procedures retain their explicit permission and interaction gates.
- `cargo check --locked --workspace --all-targets`, the affected package suites,
  strict Rasen validation, both release-target CI jobs, and the documented native
  evidence commands are the acceptance path. Existing native performance
  evidence remains non-normative under ADR 0021 until `G-013` is requalified.
