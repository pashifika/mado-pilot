# G-003 probe

The probe is the disposable program that produced the measurements in
[README.md](README.md). It is not product code and does not survive the change
that resolves the gate: what survives is the report, this description, and the
decision recorded in
[ADR 0012](../../adr/0012-macos-shim-language-and-containment.md).

This document exists so that the measurements can be reproduced without the
program. It states what the probe did precisely enough to rewrite, which is the
same standard the macOS adapter is held to.

## What it measures

Three things, on one release target:

1. Whether a native exception can be contained at a narrow Rust/Objective-C
   boundary, and what it costs in object ownership when it is.
2. What the callback fence, teardown, and autorelease arrangement have to be for
   the boundary to be safe at all, independently of the language.
3. What each candidate language adds to the build and to the process — compile
   cost, artifact size, and runtime dependencies.

Nothing is timed as a benchmark. Object lifetime is counted exactly, by
instrumenting a probe class rather than by sampling memory, because the decision
turns on whether one object was released and not on how many bytes were live.

## Experimental design

One implementation file is compiled three times. The language mode of the
translation unit is the only independent variable:

| Variant | Compiled as | Extra flag | Link prefix |
|---|---|---|---|
| `objective-c` | `-x objective-c` | — | `mp_shim_objc_` |
| `objective-c-arc-exceptions` | `-x objective-c` | `-fobjc-arc-exceptions` | `mp_shim_objcx_` |
| `objective-c++` | `-x objective-c++` | `-std=c++17` | `mp_shim_objcpp_` |

If each candidate had its own source, a measured difference could be a difference
in what was written. With one source text, it cannot be.

Two consequences of that design are worth stating because they affect how a
rewrite must be organized:

- Every function and every Objective-C class name is built from the link prefix
  by token pasting. Class names must be prefixed too, not only functions: three
  variants are statically linked into one process, and the Objective-C class
  namespace is process-wide, so two variants sharing a class name would silently
  share one implementation and the comparison would measure nothing.
- Counters are file-scope statics, so each translation unit has its own set and
  the variants cannot contaminate each other.

All three variants were linked into one harness binary, and the third variant is
a control rather than a candidate.

One limit of this arrangement has to be stated plainly, because the whole
comparison rests on it. That the three translation units really did include one
unchanged implementation is a *described* property, not a verifiable one: the
report records three source paths and no source hash, and the source itself is not
tracked. A reader who doubts it cannot check it against the retained artifacts —
they can only rebuild from the specification below and see whether the results
reproduce. That is the same trade this repository accepted for the `G-014` probe,
and it is why the specification is written to be re-implementable rather than
summarised.

## The surface under test

One header declares every entry point through a prefix-pasting macro, so the three
variants share one declaration list. The status vocabulary is `MP_SHIM_OK`,
`MP_SHIM_INVALID_ARGUMENT`, `MP_SHIM_NATIVE_FAILURE`, `MP_SHIM_CALLBACK_FAILED`,
and `MP_SHIM_UNSUPPORTED`.

| Entry point | Behavior |
|---|---|
| `session_open` | Creates the session, its serial producer queue, its condition gate, and one retained native object. Takes a size-versioned request carrying a prototype-only fault-injection site |
| `session_set_callback` | Installs the Rust callback and its context, and enables admission |
| `session_produce` | Dispatches one work item per frame onto the serial queue and returns without waiting |
| `session_wait_idle` | Barrier for the harness: `dispatch_sync` of an empty block |
| `session_fence_callbacks` | Disable and drain: takes the gate, clears admission, waits until no callback is in flight, then marks the fence complete |
| `session_close` | Idempotent. Fences, flushes the queue, then releases native resources in `@finally`. Leaves the handle valid |
| `session_release` | Closes, then frees the handle |
| `session_stats` | Copies the counters under the gate |
| `counters`, `counters_reset` | Prototype-only live, peak, allocation, and deallocation counts |
| `autorelease_probe` | Runs the `A` cases |
| `arc_exception_probe` | Runs the `L4` and `X5` cases |
| `environment` | Compile-time and runtime facts: ARC, C++ exceptions, `__cplusplus`, SDK and deployment version macros, `NSClassFromString(@"SCStream")`, `dlsym` for `CGEventCreateMouseEvent`, `@available(macOS 12.3, *)`, and `__clang_version__` |

Three shape choices matter to a rewrite:

- **Close and release are separate.** Close stops the producer, fences, releases
  native resources, and is idempotent; release frees the handle. This is what
  makes "repeated close" a measurable case rather than a double free.
- **Frames are borrowed.** The callback receives a pointer valid only for the
  duration of the call, bridged with `__bridge` and no ownership transfer, so
  retaining a frame is an explicit act rather than the default.
- **The callback returns a status.** A Rust callback that fails reports it without
  unwinding, because unwinding is the thing under test.

Frames are produced one per work item on a serial `dispatch_queue`, which is the
shape ScreenCaptureKit delivers in. Producing them all inside one work item would
have hidden both the autorelease result and the late-callback result.

## The admission and fence protocol

The protocol is the part a rewrite must reproduce exactly, because the fence
results mean nothing without it.

A work item takes the gate, and returns immediately as *suppressed* if the session
is closed, the producer has stopped, admission is disabled, or no callback is
installed. Otherwise it increments the sequence, copies the callback and context,
records whether the fence had already completed, increments the in-flight count,
and releases the gate. The callback is invoked with no lock held. The item then
retakes the gate to record delivery, and decrements the in-flight count in
`@finally` so that a thrown exception cannot strand the fence.

The fence takes the gate, clears admission, waits on the condition until the
in-flight count reaches zero, marks itself complete, and releases the gate. After
it returns, no callback can be admitted *until a callback is registered again* —
registration re-enables admission and clears the fence flag. That is what allows a
caller to free the context it registered, and a rewrite must keep the same rule: the
fence is not permanent, it is a boundary the caller may cross deliberately.

`admitted_after_fence` is expected to be zero, and a rewrite that reports anything
else has a different protocol rather than a different result. Be precise about what
the counter measures, because it is narrower than its name. The admission decision is
recorded under the gate, at admission — the work item copies the fence flag there —
but the counter is incremented only on the normal return path, after the callback
comes back. An item admitted after the fence that raises at the before-callback
injection site before ever reaching that increment is therefore never counted. So a
zero means *nothing was admitted after the fence and delivered*, which is weaker than
*nothing was admitted after the fence*.

That blind spot does not touch the runs the evidence reports: `L3`, `C2`, and `C3`
inject no fault, so every admitted item reaches the increment. A rewrite that wants
the stronger statement counts at admission instead.

## Fault injection

Prototype-only, selected per session by the open request, at four sites: inside
`session_open`, in the producer before the callback, in the producer after the
callback returned, and inside `close`. Each site raises an `NSException`. The
Objective-C++ variant can also raise a C++ `throw std::runtime_error`, which the
Objective-C variants report as `MP_SHIM_UNSUPPORTED` rather than skipping.

Five places wrap their body in `@try` with `@catch (NSException *)` and a
`@catch (...)` catch-all, and map to a status: `session_open`, `session_produce`,
`session_close`, `arc_exception_probe`, and the work item. `close` additionally
completes teardown in `@finally`.

The other eight entry points — `session_set_callback`, `session_wait_idle`,
`session_fence_callbacks`, `session_release`, `session_stats`, `counters`,
`counters_reset`, `autorelease_probe`, and `environment` — have no handler in the
prototype, because no injection site can raise inside them. That is a gap between
the prototype and the rule
[ADR 0012](../../adr/0012-macos-shim-language-and-containment.md) states for the
production shim, which requires the boundary on *every* entry point. A rewrite of
this probe reproduces the prototype; an implementation of the shim does not stop
where the prototype stopped.

## How ownership is measured

A prototype `NSObject` subclass increments a counter in `init` and decrements it
in `dealloc`, tracking live, peak, allocation, and deallocation totals atomically.
Only instances of that one class are counted: the object the session retains, one
per frame, and the autoreleased temporaries. The session object itself, its
condition, its dispatch queues, and the exception objects are **not** counted, which
is why a leaked session is an inference from the object it holds rather than a
reading — the README says so where it reports the numbers.

Autoreleased temporaries are placed in the current pool with
`CFAutorelease(CFBridgingRetain(object))`. That is deliberate rather than
incidental: it is the ARC-legal way to put an object in the pool and nothing else,
so what keeps the object alive is the pool and the measurement is unambiguous.
The local strong reference is dropped at the end of the creating function.

In `L4` and `X5` — and only there — the strong local is declared with
`__attribute__((objc_precise_lifetime))` so that ARC cannot shorten its lifetime to
before the throw. Without it, a zero result would be ambiguous between "released
early" and "released on the unwind edge".

`X1` and `X3` do not annotate their locals, and that limits what they prove on
their own: their zero in the two exception-safe variants is consistent with release
on the unwind edge, but also with ARC having released before the throw. They are
positioned at the two boundary sites that matter, which is their value; the
unambiguous demonstration is `L4`, and that is the case the conclusion rests on. A
rewrite that wants `X1` and `X3` to carry it too has to annotate those locals.

## The case list

Eighteen cases per variant. `L`, `A`, `C`, `X`, and `P` are ownership,
autorelease, callback and fence, native exception, and panic containment.

| Case | Shape |
|---|---|
| `L1` | Three cycles of open, 8 frames, wait, fence, close, close, release; live objects must be zero after each |
| `L2` | One session, 32 frames; allocation and deallocation totals and live count once the queue is idle, then again after close *and* release |
| `L3` | 64 frames, fence, free the context, 64 more frames, wait; delivered must not rise |
| `L4` | `@throw` past a strong local; deallocation delta and which handler caught it |
| `A1` | 4 work items × 8 temporaries, no explicit pool |
| `A2` | The same with `@autoreleasepool` around the body |
| `A3` | 256 work items × 4 temporaries, no explicit pool |
| `C1` | 64 frames; delivered when `produce` returned against delivered once idle |
| `C2` | 512 frames, fence immediately; admitted-after-fence must be zero and delivered plus suppressed must be 512 |
| `C3` | 32 frames, fence, free the context, 512 more frames; delivered must not rise and the suppressed count must show the producer ran |
| `C4` | Register a callback, 16 frames, then fence, fence, close, fence, close, release; every one of the six must return `MP_SHIM_OK` |
| `X1` | `@throw` inside `session_open`; status, whether a handle was written, and live objects after the failed open — there is no session to read stats from |
| `X2`, `X3` | The two producer injection sites, with status, session stats, live objects while the session is still open, and live objects after close *and* release |
| `X4` | `@throw` inside `close`; the status of each close, live objects after the first close, and the status of release — no session stats are read |
| `X5` | C++ `throw` at the `L4` site |
| `P1` | Rust callback panics on one sequence number, contained by `catch_unwind`, returns `MP_SHIM_CALLBACK_FAILED` |
| `P2` | The same panic with no `catch_unwind`, in a child process; the parent records the child's exit code and signal |

`P2` re-executes the harness binary with an environment variable naming the
variant. It must be a child process: the case exists to show what an uncontained
panic does, and a harness that survived it would be reporting the wrong thing.

Its pass predicate is weaker than its purpose, and a rewrite should tighten it. The
prototype passes the case when the child's exit code is not zero, which a setup
failure inside the child would also satisfy — the child exits 70 if it cannot open a
session. The signal and the child's first stderr line are recorded but not required,
so what actually establishes the cause in the retained run is that record: signal 6
on every variant, with a panic message. Require the signal, not merely a non-zero
exit.

## Build integration

The build script invokes `clang` directly, once per variant, and archives each
object into its own static library. The harness links those three static archives —
not the dynamic libraries — together with Foundation, CoreFoundation, libc++, and
`-weak_framework ScreenCaptureKit`. Each variant is *additionally* linked as a
standalone dynamic library for one purpose only: so that `otool -L` attributes a
runtime dependency to one candidate rather than to the harness. Nothing loads those
dynamic libraries at run time.

Note the confound that arrangement introduced, and how it was resolved: the build
script passes `-lc++` only to the Objective-C++ dynamic library, and only the
Objective-C++ translation unit compiles the C++ throw, so the libc++ row in
`otool -L` had two possible causes. The
[deconfounded measurement](README.md#the-libc-requirement-deconfounded) settles it by
compiling the same source as Objective-C++ with every C++ construct removed and no
`-lc++` on the link.

Common flags: `-c -O2 -g -fobjc-arc -arch arm64 -mmacosx-version-min=11.0
-isysroot $(xcrun --show-sdk-path) -Wall -Wextra`, plus
`-DMP_SHIM_LINK_PREFIX=<prefix>` and the include directory. The recorded report
carries the complete argument vector for every variant.

The probe has no Cargo dependencies. `clang` is invoked directly rather than
through the `cc` crate so that what the measurement attributes to the language is
not partly a build crate's defaults. A production adapter should still prefer
`cc`, which handles target, arch, and sysroot selection; `cc` 1.4.0 is already an
indirect build dependency of the workspace.

The deployment target of 11.0 is a prototype choice with a purpose: it puts
ScreenCaptureKit, which requires macOS 12.3, genuinely above the minimum, so the
weak-link and `@available` arrangement is compiled and linked rather than assumed.
Be precise about what that verified. Three things were observed: the linker records
the framework as a weak load command, `@available(macOS 12.3, *)` compiles and
evaluates, and the class lookup resolves. All three were observed on a host where
the framework is present. The path that matters for an unsupported system — the
framework absent, the availability check false, the capability reporting a clear
status — was **not** exercised, and cannot be from this host. The deployment target
is also not a proposal for the minimum supported version, which remains
[`G-001`](../../validation-gates.md#g-001).

## Reproducing a run

The probe is gone from the tree. Reproducing a measurement means implementing the
surface, the admission protocol, and the case list above, which is why they are
described here rather than pointed at.

The recorded run was produced by building in `release` against the probe's own
pinned lockfile with `--locked`, then running the harness with `--base-revision`
naming the repository revision, `--label` naming the host, and `--out` naming a
directory. The program writes one report and exits non-zero if any case failed.

The probe's source is not kept anywhere. That is the cost of this arrangement, and
it is why the description above is written as a specification rather than as a
summary: a reader who disagrees with a number has to be able to rebuild the thing
that produced it. If a future measurement contradicts one recorded here, the
contradiction is resolved by re-measuring, not by consulting the program.

## Reproducing the three review-time measurements

None of these goes through the harness and none appears in the report. They are
described here for the same reason as everything above.

**The catch-all is reachable in plain Objective-C.** Compile a single Objective-C
file — `-fobjc-arc -fobjc-arc-exceptions`, Foundation only — whose body is a `@try`
that does `@throw [NSString stringWithUTF8String:"..."]`, with a
`@catch (NSException *)` arm before a `@catch (...)` arm, and print which arm ran.
The catch-all arm runs, and `otool -L` on the binary lists no libc++. This is what
makes the catch-all in the containment rule load-bearing today rather than a
provision against a future dialect.

**The deconfounded libc++ comparison.** Take the implementation and add a
compile-time guard — `MP_SHIM_NO_CXX` below — around the `<stdexcept>` include, the
C++ throw function, its call site, and the `MP_SHIM_UNSUPPORTED` early return, so an
Objective-C++ build can exclude every C++ construct while an Objective-C build is
unchanged. Compile that one source twice:

```sh
COMMON="-c -O2 -g -fobjc-arc -fobjc-arc-exceptions -arch arm64 \
  -mmacosx-version-min=11.0 -isysroot $(xcrun --show-sdk-path) -Wall -Wextra -I include"
clang -x objective-c   $COMMON -DMP_SHIM_LINK_PREFIX=a_ -o a.o a.m
clang -x objective-c++ -std=c++17 $COMMON -DMP_SHIM_NO_CXX \
  -DMP_SHIM_LINK_PREFIX=b_ -o b.o b.mm
```

`MP_SHIM_LINK_PREFIX` is not optional — the header has an `#error` without it — and
the two prefixes must be the **same length**, for the reason in the next paragraph.
Take the best of three compiles for each. Then compare object bytes, compare `nm -u`
filtered to C++ and exception-handling symbols, and link each object as a dynamic
library against Foundation with **no** `-lc++`. The Objective-C link succeeds; the
Objective-C++ link fails on `std::terminate()` and `___cxa_begin_catch`, both
reached through `___clang_call_terminate`. Keep the `@finally` in the work item: a
probe without a cleanup region shows no libc++ symbols at all and would answer a
different question.

**Why object bytes are not comparable across experiments.** `-g` embeds symbol names
and source paths in DWARF, so the same source with the same flags produces a
different object size when only the link prefix or the containing directory changes.
Two demonstrations: compiling one source with a two-character prefix and then with a
fourteen-character one moves it by 2,248 bytes, and relocating the prototype moved
every object in the recorded report by exactly 16 bytes. Compare byte counts only
within one experiment that holds both constant. Nothing in the decision rests on a
byte count; the libc++ result is a link that either resolves or does not.

**Pooled autorelease over a long run.** Link a small C driver against one variant's
static archive, include the shim header with that variant's `MP_SHIM_LINK_PREFIX`,
and call the autorelease probe with the pool enabled at 4 × 8, 256 × 4, and
1,024 × 4, resetting the counters between cases, plus 256 × 4 unpooled as the
control. Read the peak from each result.
