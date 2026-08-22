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
   one is an explicit act with an explicit release, never the default. **Extended by
   the implementation; see [Amendment: the session's own
   lifetime](#amendment-the-sessions-own-lifetime).** The rule as written covers the
   objects a session holds and says nothing about the session's own allocation, which
   turned out to have no owner at all.
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
   attribute. **Amended by the implementation; see
   [Amendment: the linkage mechanism](#amendment-the-linkage-mechanism).** The rule
   and the property it exists for are unchanged; the mechanism is controlled
   dynamic loading rather than a weak load command, because a build script in the
   Adapter package cannot put `-weak_framework` on the final link.

## Amendment: the linkage mechanism

Recorded on 2026-07-31 by the Change that implemented the boundary, at base
revision `9f4decf9cedf542d01b54eae703ea08d6706ff99`. It amends rule 8 above and
nothing else: the language, the exception flag, and rules 1 through 7 stand as
measured.

The prototype linked its own dynamic library directly, so `-weak_framework` was a
flag it could pass. The production shim is compiled into a static archive that a
Cargo package publishes as an rlib, and the binary that finally links it is a test
binary, the C ABI's dynamic library, or a consuming application. Cargo does not
propagate a dependency's `rustc-link-arg` to that link. That was measured on a
two-package workspace whose dependency's build script emitted
`cargo::rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit`: the consuming binary's
`otool -L` listed `libSystem` alone, with no ScreenCaptureKit load command of either
kind. `cargo::rustc-link-lib=framework=` does propagate, but produces a regular
load command, which is the eager dependency this rule exists to prevent.

The shim therefore loads ScreenCaptureKit with `dlopen` at its absolute system path,
resolves its classes with `NSClassFromString` and its exported attachment keys with
`dlsym`, declares the selectors it sends through protocols named for the shim rather
than for the framework, and gates every use behind `@available(macOS 12.3, *)`. The
absolute path is part of the decision: a bare name would let the loader's ambient
search decide which library answers, which is the unrestricted search this project's
packaging rules reject. Every other framework the shim needs — Foundation, Core
Foundation, Core Graphics, Core Media, Core Video, Application Services — predates
any minimum `G-001` could select and is declared by the build script as an ordinary
framework link.

Two consequences are worth stating. The first is that the observable property is
preserved and now tested rather than reviewed: `tests/linkage.rs` asserts that a
binary linking the Adapter carries no ScreenCaptureKit load command and does carry
the six frameworks the build script declares, so an unsupported host reports
`Unsupported` from an operation instead of failing to load. That test had to be made
to reference the Adapter before it meant anything — a linker drops an archive nothing
uses, and then drops the framework references that archive would have needed, so the
first version of it passed against a binary that never contained the boundary. The
second is that the unsupported host itself is still not exercised, for the same
reason the prototype could not exercise it: the framework is present on every host
available to this repository's verification.

## Amendment: the session's own lifetime

Recorded on 2026-08-01 by the Change that implemented the boundary, after a review
pass found three use-after-free defects in one place. It extends rule 3 above and
nothing else: the language, the exception flag, and the other rules stand as measured.
Rule 5 in particular is not amended — one of the three defects was a callback that did
not meet it, and the fix makes the existing wording true rather than changing it.

Rule 3 says native objects are owned by the session. Nothing said who owns the
session. Three parties reached `struct mp_shim_session` through a raw pointer — a
detached frame returning its lease, the stream output object delivering a sample or a
stop, and the Rust handle — and the handle's release destroyed both mutexes and freed
the allocation without consulting either of the others. All three orderings are
reachable, and the first was not a race at all: retain a published frame, close the
session, drop the frame, and the lease is returned through freed memory.

**The session's allocation is therefore reference counted, and the count is the rule.**
Every party that can dereference the pointer holds a reference for as long as it can,
and the allocation outlives the last of them. Four parties do: the Rust handle from
open until release, a detached frame from its detach until its release, the stream
output object for its whole lifetime, and the capture-start completion block for the
duration of the block. Releasing the handle no longer frees anything by itself.

Two consequences are part of the decision rather than incidental to it.

**Close breaks an ownership cycle, so releasing a handle without closing it leaks
both objects.** The session retains the stream output object; that object holds a
counted reference back. `mp_shim_session_release` closes first when the caller has not,
which is what makes the cycle unreachable in practice, and that ordering is now load
bearing rather than a convenience.

**The output object's session pointer is never cleared.** Clearing it after the fence
is what the implementation used to do, and it protected nothing: a callback that has
already read the pointer into a local holds the address whatever is written afterwards.
Holding the reference for the object's whole lifetime makes the property structural
instead — while the output object is alive, its session is — so the callbacks
dereference it without further synchronization, and what stops a late callback from
doing work is admission, which is what rule 5 is for. The residual assumption is that
the framework does not message a deallocated output object, which is what every
Objective-C delegate rests on; the shim retains that object itself from open until
close rather than depending on the framework's own retention to establish it.

What a retained public frame pins is worth stating precisely, because the count could
be misread as weakening the ownership property the Adapter advertises. A frame keeps
the session's bookkeeping allocation alive — not a producer surface, not the buffer
pool, not the stream. Close releases every native object the session held, so a closed
session still reports no live native object, and a retained frame still pins nothing
capture needs to make progress.

This amendment also records why it is verifiable at all. The ownership cases rule 3
is measured by assert that a live native object *count* returns to its baseline, and a
count cannot observe an access after a free — which is why 72 green cases sat on top of
the first defect. `CONTRIBUTING.md` step 10 runs the scenarios with the shim compiled
under AddressSanitizer, and it was confirmed to report the defect before the fix and
nothing after it. A rule whose violation no check can see is a comment.

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

`mado-pilot-platform-macos` now implements the boundary and carries the tests
listed below, so these rules are enforced by that package rather than by review.
Enforcement is uneven, and that is stated rather than averaged: the cases marked
*authorized host only* need a macOS host that has already granted Screen Recording
to the process running the tests. This Adapter will not prompt, so a host that has
neither granted nor denied it — a continuous-integration runner, for instance —
reaches the non-prompting refusal instead of the capture path. Those cases print a
skip with that reason rather than passing, so a green run on such a host is not
evidence that they ran. They were run on an authorized Apple Silicon host when the
boundary was implemented, and doing so found one thing this record did not
anticipate: the framework's shareable-content query requires the process to hold a
Core Graphics window-server connection and aborts on an internal assertion when it
does not, rather than returning a status. An abort is not an exception, so rule 1's
handlers cannot contain it; the shim establishes the connection before it loads the
framework instead. That is a precondition the boundary owns, not a containment rule
this record got wrong, and it is documented in `docs/architecture.md`.

- **Containment.** A test per injection site — session start, before a frame
  callback, after a frame callback returned, and teardown — asserting the typed
  status and that no exception escaped. The sites are reached through
  session-scoped raise seams on the shim's open request, which are zero in the
  product and inert unless a caller sets them. *Authorized host only.*
- **Ownership on the failure path.** The same sites asserting that no native
  object survives a contained failure, against the shim's own count of the objects
  it owns. These are the tests that fail if `-fobjc-arc-exceptions` is ever
  dropped, which is how a build flag becomes a tested invariant rather than a
  comment. *Authorized host only.*
- **Autorelease.** A long producer run asserting that live native objects stay
  bounded by the per-item working set rather than growing with the run.
  *Authorized host only.*
- **Fence.** Asserted through close, which fences before it releases: a repeated
  close, a close after a cancelled close, and a frame request after close all
  succeed or report their own typed outcome, and the strong reference the shim
  holds as its callback context is reclaimed only after a fence returns.
  *Authorized host only.*
- **Teardown.** A test asserting that close is idempotent and that the objects the
  session owned are released, with the injected-failure sites covering the case
  where close reports a failure and releases anyway. *Authorized host only.*
- **Panic containment.** A test asserting that a panicking host callback becomes a
  typed failure at the trampoline, and a separate child-process test asserting
  that the unguarded form is stopped by a signal — the one case that cannot be
  asserted in-process. Both run on any host, because neither needs a stream.
- **Linkage.** A check that a binary linking the Adapter carries no load command
  for the capture framework and does carry the frameworks the build script
  declares, so an unsupported host reports a status instead of failing to load. See
  [Amendment: the linkage mechanism](#amendment-the-linkage-mechanism) for why the
  mechanism is controlled dynamic loading rather than a weak load command. Runs on
  any host.
- **Boundary layout.** A check that the compiled shim and the Rust declarations
  that mirror it agree on the surface version and on every structure size, since
  the two sides are written by hand. Runs on any host.
