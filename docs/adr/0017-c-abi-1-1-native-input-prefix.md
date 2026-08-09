# ADR 0017: Append the native-input C ABI 1.1 prefix

- **Status:** Superseded by ADR 0023
- **Date:** 2026-08-08
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

ABI 1.0 is permanent under [ADR 0007](0007-phase-1-c-abi-freeze.md), while the
now-landed Rust facade reaches native discovery, capture, non-prompting
permission probes, and input. The C and C++ boundaries need that workflow without
moving a released byte or introducing a second capture architecture.

The Phase 2 planning draft also conflicts with two implemented contracts. It
said a partial input receipt should become a non-success C result carrying an
owned partial-error record, but [ADR 0015](0015-partial-native-input-effects-and-receipt-accounting.md)
and `Session::send_input` make every admitted partial effect the operation's one
terminal receipt, including a `Partial` receipt with zero proven-complete logical
events. It also said Windows should return a successful global permission state,
but the facade deliberately wires no Windows permission probe and reports
`Engine::reads_permissions() == false`.

The first layout prototype appended an input-policy pointer to
`madopilot_open_request_t`. That made the current C and Rust declarations agree
at 24-byte size and 8-byte alignment, but an executable frozen-header probe
exposed the compatibility defect: the released ABI 1.0 record is 16 bytes with
4-byte alignment. Its size prefix permits a later library to read an available
suffix, but it cannot retroactively require an old caller's object to satisfy
8-byte pointer alignment.

The corrected prototype keeps `madopilot_open_request_t` exactly 16 bytes with
4-byte alignment and passes input policy to a separate table entry. Apple Clang
and Rust both measure the unchanged table prefix at 424 bytes, the new open
entry at offset 424, and the complete table at 480 bytes. The unchanged frozen
header compiles, links, negotiates, and runs against that library. ADR 0023 later
removed the unreleased 1.1 declarations and executable fixture from the current
tree; this superseded ADR and repository history retain the development record
without creating a compatibility obligation.

## Decision

ABI major 1 minor 1 appends native source tags, `INPUT_FAILED`, permission,
target/capture/input capability, input-policy, event, descriptor, and receipt
values. It appends fields only to `madopilot_target_t` and
`madopilot_session_info_t`; their ABI 1.0 mandatory prefixes remain 24 and 32
bytes respectively. `madopilot_open_request_t` remains exactly the released
16-byte, 4-byte-aligned record. Native construction continues through
`engine_create`, and native capture continues through the existing discovery,
session, frame, mapping, matching, and close handles and entries.

Seven function pointers append after the complete 424-byte ABI 1.0 table, in
this permanent order:

| Offset | Entry |
|---:|---|
| 424 | `session_open_with_input` |
| 432 | `engine_capabilities` |
| 440 | `engine_permission` |
| 448 | `target_list_capability` |
| 456 | `engine_input_descriptor` |
| 464 | `session_input_descriptor` |
| 472 | `session_send_input` |

`session_open_with_input` receives the unchanged capture request and a separate
input-open request. The existing `session_open` entry remains capture-only, so
an ABI 1.0 caller never needs the newer request type or alignment. The complete
table is 480 bytes on both 64-bit release targets. The header exposes one extent
constant per appended entry so a C++ caller checks the smaller of its negotiated
extent and the library's reported `struct_size` before reading a pointer.

An admitted input sequence always returns receipt data. `Complete`, `Partial`,
and `Unexecuted` receipts all return `MADOPILOT_STATUS_OK`; `Partial` is never
translated into an error, retried, or sent through a fallback after any possible
effect. A failure before a receipt exists returns its non-success status, leaves
the direct receipt in its independent zero-delivery failure state, and may return
the existing immutable owned `madopilot_error_t`. There is no receipt handle and
no fabricated partial-error record.

A permission entry calls the facade's non-prompting probe. An engine with no
probe returns `MADOPILOT_STATUS_UNSUPPORTED` and a failure-state permission
record; the C layer does not invent a successful `Unavailable` report. Its
`engine_capabilities` output lets callers establish that fact before asking.

All new input views are borrowed for one call and validated before focus or
input. Receipts and descriptors contain no borrowed caller memory. Permission
diagnostic views are Adapter-authored static text valid while the library is
loaded. New const accessors are concurrent with live retained handles;
`session_send_input` calls are serialized by the controller and wait under each
caller's operation context without an unbounded queue. The existing rule still
excludes releasing the last reference concurrently with a call that did not
retain its own.

## Alternatives

**Append the input-policy pointer to `madopilot_open_request_t`.** Rejected
after the frozen-header executable proved that the released record has only
4-byte alignment. A size-versioned suffix can add fields that preserve the
record's alignment; it cannot make a legally aligned old object suitable for a
new pointer-aligned Rust type. A separate suffix entry preserves both the record
and the function-table prefix.

**Return a non-success status for `Partial` and attach the receipt to an owned
error.** Rejected because it creates a second terminal outcome for one admitted
sequence and contradicts ADR 0015. A partial effect is not a failed question; it
is irreversible data the caller must inspect and must not retry.

**Synthesize `PermissionState::Unavailable` for a Windows engine in C.** Rejected
because it would make C report a successful probe that the public Rust engine
does not have. Capability absence is reported explicitly and the query remains
unsupported.

**Add a native engine entry and a parallel one-shot capture path.** Rejected
because source tags are sufficient to select the facade constructor, while the
existing stream-first handles already preserve native storage, frame identity,
and retained-child lifetime.

**Insert permission and input entries near related ABI 1.0 entries.** Rejected
because conceptual grouping cannot move a frozen function pointer. Documentation
can group related operations without changing their physical order.

## Consequences

- ABI 1.1 values, record fields, table order, and signatures are permanent within
  major 1 once released. A correction after release appends or requires ABI 2.
- C callers must inspect a successful receipt's `outcome`; success does not mean
  every event completed. A `Partial` receipt is non-retry-safe even when
  `delivered == 0`.
- C and C++ callers must check engine capability before assuming a permission
  probe exists, and must never request permission, launch settings, or infer a
  grant from probe absence.
- Event text, event and delivery arrays, and source-frame handles must remain
  valid for the complete call. Nothing from those inputs is retained afterwards.
- The C++ wrapper remains header-only and adds no ABI. It must carry the
  negotiated table extent through every owner and return `UNSUPPORTED` before
  reading an absent 1.1 entry.
- OCR, watchers, callbacks, acceleration selection, packaging, and native-frame
  handles remain absent.

## Verification

- Rust and C layout reports compare every size, alignment, field offset, numeric
  value, mandatory prefix, and table offset on both release targets.
- The frozen ABI 1.0 header remains unchanged and its caller negotiates both its
  40-byte mandatory prefix and 424-byte complete extent against the 1.1 library.
- Boundary tests cover invalid tags, flags, UTF-8, Unicode scalars, coordinates,
  counts, strides, extents, limits, lifecycle, concurrent input serialization,
  failure-output initialization, and panic containment.
- Receipt tests prove complete, unexecuted, and zero-complete-event partial
  outcomes, no fallback after possible effect, cleanup accounting, and validity
  after request, session, and engine release.
- The macOS Adapter's existing exception-injection suite remains the proof that
  Objective-C exceptions are contained before safe Rust; C boundary tests prove
  the resulting facade failure and unrelated retained handles remain usable.
- Native C/C++ checks compile, link, and run the safe dedicated-fixture examples;
  compile-time C++ tests reject access past a truncated table and the deferred
  later-phase surface.
