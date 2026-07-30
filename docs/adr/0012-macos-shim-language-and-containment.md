# ADR 0012: macOS shim language and containment boundary

- **Status:** Accepted
- **Date:** 2026-07-30
- **Resolves gate:** `G-003` from [../validation-gates.md](../validation-gates.md)
- **Supersedes:** _none_

## Context

`mado-pilot-platform-macos` is not implemented. Before it can be, the language of
the narrow native boundary it needs has to be settled, because ScreenCaptureKit
streams, Core Video and IOSurface frame lifetime, `CGEvent` input, Objective-C
object ownership, delegate callbacks, and Objective-C exceptions all cross that
boundary, and an exception or an unreleased object crossing it is a defect the
Rust side cannot see.

`G-003` asked for a prototype covering exception behavior across the language
boundary, object ownership, and build integration on Apple Silicon. Two candidate
translation units — Objective-C and Objective-C++ — were built from one
implementation file so that the language mode was the only independent variable,
and a third variant, Objective-C compiled with `-fobjc-arc-exceptions`, was added
as a control. Eighteen cases ran on each variant on the approved Apple M1 Pro host
at base revision `7ae9050e9445a746eb2237c721c05eca4f7a1618`, and no case failed.
The evidence is in [../evidence/g-003/](../evidence/g-003/README.md); the prototype
itself is throwaway and not tracked, and
[../evidence/g-003/probe.md](../evidence/g-003/probe.md) specifies it closely
enough to rebuild.

The measurement that decides the gate is not about exceptions being caught — all
three variants contain every injected native exception and map it to a status.
It is about what containment costs in ownership. The exceptions are injected — a
prototype-only `NSException` raised at four boundary positions, not a real
ScreenCaptureKit failure — so what is measured is unwinding through those positions,
which is the mechanism any real failure there would use. Under ARC without
`-fobjc-arc-exceptions`, clang emits no release on the unwind edge, so the strong
reference held at the throw point is never dropped: after a throw at the position
where starting a stream would fail, the object the session had already retained is
still alive (1 live native object, against 0 for the other two variants), which
means the session that owned it was not released either; and a throw at the position
after a frame callback returns leaks that frame object (1 live object after release,
against 0). The control reproduces Objective-C++'s result exactly, which establishes
that the difference is the compiler flag and not the language.

What Objective-C++ adds on its own is the ability to contain a C++ `throw` — its
`@catch (...)` handler caught one and still released the strong local — and
`/usr/lib/libc++.1.dylib` in the dependency list of every process that loads the
library.

That libc++ cost had to be measured twice before it could be relied on. In the
harness run, the Objective-C++ variant was the only one compiling a
`throw std::runtime_error` case and the only one linked with `-lc++`, so its
`otool -L` row had two possible causes besides the language. A deconfounded
measurement compiled the same source as Objective-C++ with every C++ construct
removed and no `-lc++` on the link: it still fails to link, on `std::terminate()`
and `___cxa_begin_catch`, both reached through `___clang_call_terminate` — the
cleanup path that `@finally` and ARC cleanup regions generate in Objective-C++.
The dependency is therefore a property of the language mode for a shim shaped like
this one, not of the prototype's C++ test. On that clean comparison Objective-C++
costs 8.2% more object bytes and 22% more compile time than the selected variant;
those two are single samples and decide nothing on their own.

Three further results constrain the boundary regardless of language, because all
three variants produced them identically. Delivery outlives the call that starts
it: 0 frames were delivered when `produce` returned and 64 once the queue drained,
so a caller that frees registered state without a fence races a live producer.
Without an explicit `@autoreleasepool` per work item, the pool does not drain
between work items: 1,024 temporaries were live across a 256-frame run, against 4
for the same shape with the pool, and the pooled peak stays at the per-item working
set from 4 work items to 1,024. And an unguarded Rust panic at the callback boundary
stopped the child process with signal 6 on every variant, so containment has to be
on the Rust side of the call.

## Decision

The macOS native shim is written in **Objective-C with Automatic Reference
Counting, compiled with `-fobjc-arc-exceptions`**. Objective-C++ is not used, and
C++ is not admitted into the shim. `mado-pilot-platform-macos` owns this boundary;
no Objective-C or Objective-C++ type appears in any Rust or public API.

The boundary is one internal C-callable surface with opaque handles,
size-versioned request structures, a status return on every entry point, and
output values written through validated pointers. Its rules are:

1. **Exception containment.** Every entry point and every callback trampoline
   wraps its body in `@try` with `@catch (NSException *)` and a `@catch (...)`
   catch-all, and maps the failure to a typed status. No native exception crosses
   the boundary, and no entry point relies on its callers not throwing. Both
   branches are load-bearing: the `NSException` branch is what the evidence
   exercises at the four injection positions, and the catch-all is reachable in
   plain Objective-C, because `@throw` accepts any object and `@catch (NSException *)`
   will not match one that is not an `NSException`. That was measured — a thrown
   `NSString` lands in the catch-all of a binary that links no libc++. The
   prototype exercises the catch-all only through the Objective-C++ `throw` in
   `X5`, so the Objective-C path to it is a language property established
   separately rather than a case in the harness run.
2. **`-fobjc-arc-exceptions` is mandatory.** It is a correctness requirement, not
   a style preference: without it, an exception unwinding out of a scope that holds
   a native object leaks it, which is what the injected failures measured.
3. **Ownership.** Native objects are owned by the session and released on close.
   Frames handed to a callback are borrowed for the duration of the call; retaining
   one is an explicit act with an explicit release, never the default.
4. **Autorelease.** Each frame work item wraps its body in `@autoreleasepool`.
5. **Callback fence.** Callback admission is guarded by a disable-and-drain fence
   that returns only when no callback is in flight, after which the caller may
   release the state it registered. The host callback is never invoked while an
   internal lock is held, and the in-flight count is decremented in `@finally` so
   that a thrown exception cannot strand the fence.
6. **Teardown.** Close is idempotent, and completes its resource release in
   `@finally` so a cleanup failure is reported without costing the cleanup.
   Releasing the handle is a separate operation from closing the session.
7. **Panic containment.** Every Rust callback invoked by the shim catches its own
   panics and returns a status; an `extern "C"` callback that lets one escape
   aborts the process.
8. **Linkage.** The shim's own build owns framework linkage and availability
   gating for capabilities newer than the deployment minimum — weak framework
   linking plus `@available` — rather than inheriting a binding crate's `#[link]`
   attribute.

## Alternatives

**Objective-C++ as the shim language.** Rejected. It adds libc++ to the
dependency set of every process that loads MadoPilot — verified with every C++
construct removed from the source and no `-lc++` on the link, so the dependency
follows from the language mode and not from the prototype's C++ test — in exchange
for containing C++ exceptions that a boundary with no C++ in it cannot raise. The
dependency is the reason; the 8.2% larger object and 22% slower cold compile from
that same clean comparison point the same way but are single samples and decide
nothing on their own. Its ownership advantage is not the language's: the
control variant reproduces it with one flag. Reconsider only if the boundary must
call a C++ library, in which case the same catch-all handler already covers a C++
`throw`, as `X5` measured.

**Objective-C with ARC and no exception flag.** Rejected. It leaks a native object
whenever an exception unwinds out of a scope holding one, measured at the position
where starting a stream fails and at the position after a frame callback returns.

**Manual retain and release instead of ARC.** Rejected. It would move the cleanup
obligation that one flag discharges into hand-written code on every failure path,
which is the opposite of narrowing the boundary. Not measured, and recorded as
reasoning.

**No native shim at all: a Rust-only boundary through `objc2`, using its optional
`exception` feature to catch Objective-C exceptions.** Rejected for version one on
reasoning rather than on measurement, and the distinction is recorded because it
matters. Two facts weigh against it. The published `objc2-screen-capture-kit`
0.3.2 declares `#[link(name = "ScreenCaptureKit", kind = "framework")]`, a hard
framework link, so a binary that adopts it fails to load below the framework's
minimum macOS version instead of reporting a clear unsupported status — which is
the opposite of the rule this baseline states for capabilities newer than the
deployment minimum. A shim compiled with `-weak_framework` and gated with
`@available` owns that directly, and the recorded `otool -L` output shows
ScreenCaptureKit as a weak dependency. Whether a Rust-only boundary could reach
the same result was not tested.

**A higher-level wrapper crate — `screencapturekit` or `cidre`.** Rejected. The
first would give a single-vendor wrapper ownership of the capture contract this
project owns; the second is far broader than the boundary needs and carries the
highest minimum supported Rust version of the reviewed candidates.

## Consequences

- `mado-pilot-platform-macos` gains a build script that compiles the shim.
  `cc` becomes a direct build dependency of that package; it is already an
  indirect build dependency of the workspace, so the dependency graph gains an
  edge rather than a crate.
- Building the workspace on macOS is expected to continue needing only the Xcode
  Command Line Tools. Everything the prototype did — compiling Objective-C and
  Objective-C++, archiving both into static libraries, linking those into a Rust
  binary, and separately linking each as a dynamic library for dependency
  inspection — succeeded with Command Line Tools alone, and full Xcode is not
  installed on the verification host. Two limits on that: it is a positive result
  about the weaker setup rather than a parity claim, and the prototype built no
  production shim, called no capture or input API, and pulled in no Cargo
  dependency, so the claim is an expectation carried by the build steps that were
  exercised, not a measurement of the finished adapter.
- `-fobjc-arc-exceptions` becomes a build-script invariant for the shim. A future
  change that drops it reintroduces measured leaks, so the flag belongs in the
  same review as the source it compiles.
- The C ABI, the C++ wrapper, the Rust facade, the Windows adapter, and the
  minimum supported Rust version are unaffected. No public contract changes.
- The minimum supported macOS version is still open as
  [`G-001`](../validation-gates.md#g-001). This decision constrains it in one
  direction only: the shim must weak-link and availability-gate anything newer
  than whatever minimum `G-001` settles on, so `G-001` can be decided without
  reopening this one.
- Choosing Objective-C means a later need for C++ in the boundary is a reviewed
  change: switching the translation unit to Objective-C++ and accepting the libc++
  dependency. The migration is a compiler flag and a link library, not a rewrite,
  because the surface and the containment rules are language-independent.
- The `objc2` family remains the reviewed Rust side of the boundary.
  `objc2-screen-capture-kit` is adopted only if a weak-linking arrangement is
  demonstrated for it; otherwise the shim declares what it needs.

## Verification

`G-003` is resolved by evidence, not by this record. The measurements are at
[../evidence/g-003/report-aarch64-apple-darwin.json](../evidence/g-003/report-aarch64-apple-darwin.json),
summarized in [../evidence/g-003/README.md](../evidence/g-003/README.md), and the
prototype is specified in
[../evidence/g-003/probe.md](../evidence/g-003/probe.md), together with three
measurements taken during the pre-landing review of this record: that the catch-all
handler is reachable in plain Objective-C, the deconfounded libc++ comparison, and
the pooled autorelease run. Each is recorded with its method and is reproducible from
the specification.

The recorded run names the base revision it was produced at and reports 54 cases as
40 `pass`, 12 `recorded`, 2 `unsupported`, and no failure. That no case failed is not
a statement that every variant behaved acceptably — unflagged Objective-C passes the
two exception cases while leaking a native object, because those cases gate on the
status crossing the boundary and record ownership rather than gating on it. Those
recorded leaks are what rejected the unflagged variant.

No repository check enforces these rules today, because there is no shim for one
to run against: `mado-pilot-platform-macos` exists as a repository seam, declares
no macOS dependency, and implements no operation. That is stated rather than
glossed: until that package implements the boundary, this ADR is enforced by
review, and its module documentation names this record as the decision it will be
built to.

The Change that implements the macOS shim carries the tests that make it
enforceable, and each one corresponds to a case recorded here:

- **Containment.** A test per injection site — session start, before a frame
  callback, after a frame callback returned, and teardown — asserting the typed
  status and that no exception escaped.
- **Ownership on the failure path.** The same sites asserting that no native
  object survives a contained failure. These are the tests that fail if
  `-fobjc-arc-exceptions` is ever dropped, which is how a build flag becomes a
  tested invariant rather than a comment.
- **Autorelease.** A long producer run asserting that live native objects stay
  bounded by the per-item working set rather than growing with the run.
- **Fence.** A test asserting that no callback is admitted after the fence
  returns, that the caller may free registered state immediately afterwards, and
  that repeated fences, repeated closes, and a fence after close all succeed.
- **Teardown.** A test asserting that a failing close reports its failure and
  still releases every native resource, and that close is idempotent.
- **Panic containment.** A test asserting that a panicking host callback surfaces
  as a typed failure with the session left consistent, and a separate
  child-process test asserting that the unguarded form is fatal — the one case
  that cannot be asserted in-process.
- **Linkage.** A check that anything newer than the declared minimum macOS version
  is weakly linked and availability-gated, so an unsupported host reports a status
  instead of failing to load.
