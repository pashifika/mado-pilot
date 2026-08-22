# G-003 evidence: the macOS shim language

This directory holds the measurements behind gate
[`G-003`](../../validation-gates.md#g-003) and the decision recorded in
[ADR 0012](../../adr/0012-macos-shim-language-and-containment.md).

The gate asks for a prototype covering exception behavior across the language
boundary, object ownership, and build integration on Apple Silicon. What was
measured is wider than that, because the exception question turned out not to be
answerable on its own: the same failure paths that leak an object under one
compiler setting release it under another, so ownership, autorelease behavior, the
callback fence, teardown, and Rust panic containment were measured together.

| File | Contents |
|---|---|
| [probe.md](probe.md) | What the prototype was, precisely enough to rewrite, and how to reproduce a run without it |
| `report-aarch64-apple-darwin.json` | Raw report from the Apple Silicon run |

The prototype is a throwaway program in the change work area. It is not tracked
and does not survive the change: what survives is this directory, `probe.md`, and
the decision. The report records the base revision it was produced at, so a
report and a repository that no longer agree are visibly stale rather than
quietly wrong.

## Provenance

| Field | Value |
|---|---|
| Base revision | `7ae9050e9445a746eb2237c721c05eca4f7a1618` |
| Branch | `feat/phase-2-g003-macos-shim`, from `dev/0.2.0` |
| Release target | `aarch64-apple-darwin` |
| Case verdicts in the recorded run | 54 cases — eighteen on each of three variants — as 40 `pass`, 12 `recorded`, 2 `unsupported`, 0 `fail` |

`0 fail` means every case produced the observation its gate predeclared. It does
**not** mean every variant behaved acceptably: unflagged Objective-C passes `X1` and
`X3` while leaking a native object, because those cases gate on the status crossing
the boundary and record the ownership outcome rather than gating on it. The leaks
are the finding, not an absence of one — see
[ownership on the exception path](#ownership-on-the-exception-path). `recorded` marks
a case whose result is a measurement rather than a pass/fail predicate, and
`unsupported` marks `X5` on the two Objective-C variants, which cannot express a C++
throw at all.

## Host and toolchain

| Field | Value |
|---|---|
| CPU | Apple M1 Pro, 10 logical cores |
| Memory | 32 GiB (34,359,738,368 bytes) |
| Operating system | macOS 26.5.2 (build 25F84) |
| Developer tools | Xcode Command Line Tools only, at `/Library/Developer/CommandLineTools` |
| Full Xcode | Not installed; `xcodebuild` reports that the active developer directory is a Command Line Tools instance |
| SDK | MacOSX.sdk 26.5, `__MAC_OS_X_VERSION_MAX_ALLOWED` = 260500 |
| Compiler | Apple clang 21.0.0 (clang-2100.1.1.101) |
| Rust | rustc 1.97.1 (8bab26f4f 2026-07-14), cargo 1.97.1 (c980f4866 2026-06-30) |
| Deployment target | `-mmacosx-version-min=11.0`, `__MAC_OS_X_VERSION_MIN_REQUIRED` = 110000 |
| Signing and launch | Ad-hoc linker signature (`flags=0x20002(adhoc,linker-signed)`), Mach-O thin arm64, no bundle, no entitlements, launched from a terminal |

Command Line Tools were sufficient for every measurement here: compiling
Objective-C and Objective-C++, archiving, linking a dynamic library, and linking
the Rust harness. Full Xcode is not installed on this host, so the result is a
positive statement about the weaker setup and not a parity claim.

## What was compared

One implementation file, written in Objective-C with Automatic Reference
Counting, is included unchanged by three translation units. The language mode of
the translation unit is the only independent variable, so a measured difference
cannot be a difference in what was written.

| Variant | Role | Compiled as | Extra flag |
|---|---|---|---|
| `objective-c` | Candidate | `-x objective-c` | — |
| `objective-c-arc-exceptions` | Control | `-x objective-c` | `-fobjc-arc-exceptions` |
| `objective-c++` | Candidate | `-x objective-c++` | `-std=c++17` |

The control is what makes the comparison decidable. Without it, every row
Objective-C++ wins looks like a property of the language.

## Build integration

Single-sample wall clock from one cold `cargo build --locked`, with `clang`
invoked directly from a build script and no Cargo dependencies at all. These are
not benchmark numbers, and the size and time columns are not a like-for-like
language comparison: the Objective-C++ object also contains the C++ throw case that
only it can compile. The
[deconfounded comparison](#measurements-added-during-review) below is the one to
read for cost.

| Variant | Compile | Link | Object | Archive | Dynamic library | Warnings under `-Wall -Wextra` |
|---|---|---|---|---|---|---|
| `objective-c` | 281 ms | 79 ms | 77,480 | 80,008 | 85,896 | 0 |
| `objective-c-arc-exceptions` | 253 ms | 77 ms | 80,784 | 83,360 | 86,216 | 0 |
| `objective-c++` | 340 ms | 78 ms | 92,648 | 95,296 | 86,984 | 0 |

The Rust harness links the three static archives. Each variant was *additionally*
linked as its own dynamic library, for one purpose: so that `otool -L` attributes a
runtime dependency to one candidate rather than to the harness. No Rust binary was
linked against a dynamic library here.

| Variant | `otool -L` |
|---|---|
| `objective-c` | Foundation, CoreFoundation, `libobjc.A.dylib`, `libSystem.B.dylib`, ScreenCaptureKit (weak) |
| `objective-c-arc-exceptions` | identical |
| `objective-c++` | the same, **plus `/usr/lib/libc++.1.dylib`** |

That libc++ row cannot carry the decision on its own, and the reason is a flaw in
this run rather than in the reading: the Objective-C++ variant is the only one that
compiles the prototype's `throw std::runtime_error` case, and it is the only one the
build script passes `-lc++` to. Both are confounds. The
[deconfounded measurement](#measurements-added-during-review) below removes them and
is what the decision actually rests on.

All three export the same 17 symbols: the thirteen entry points of the surface
being compared, plus the class and metaclass symbols of the prototype's two
Objective-C classes. Those four are visible because each variant's class names
carry its link prefix, which is what keeps three statically linked variants from
sharing one class implementation.

## Ownership on the exception path

This is the decisive measurement. A strong local holds a native object when an
exception unwinds past it; the counter reports whether the object was ever
released.

| Case | What throws | `objective-c` | control | `objective-c++` |
|---|---|---|---|---|
| `L4` | `@throw` past a strong local | **0 released (leaked)** | 1 released | 1 released |
| `X1` | `@throw` inside `session_open` | **1 live object after the failed open** | 0 | 0 |
| `X3` | `@throw` after the callback returned, with the frame object still held | **1 live object after release** | 0 | 0 |

`X1` and `X3` sit at the two positions that matter rather than at exotic ones: where
a stream that cannot start would fail, and where a frame fails after the host has
already seen it. What raises there is an injected `NSException`, not a real
ScreenCaptureKit failure — Cocoa reports operational errors through a status and an
`NSError`, and reserves exceptions mostly for programmer error — so what these cases
measure is unwinding through those positions, which is the mechanism any exception
raised there would use. Under ARC without `-fobjc-arc-exceptions`, clang does not
emit the release on the unwind edge, so the strong reference held at the throw point
is never dropped.

What the counter sees is one native object still alive. In `X1` that object is the
one the session had already retained, which means the session that owned it was
never released either — the counter tracks the probe objects, so the session
itself is an inference from the object it holds, not a separate measurement. In
`X3` it is the frame object that was still held when the throw happened, and it
leaks once per occurrence.

The control settles what that means. Objective-C++ does not contain exceptions
better than Objective-C; it defaults to a setting Objective-C has to be given.
With the flag, the two agree on every ownership and containment row measured here
except `X5`, the C++ `throw`, which Objective-C cannot express at all — the one
row where the difference really is the language.

`__has_feature(objc_arc_exceptions)` is not a usable check for this: it reports
false in all three variants, including the one built with the flag. The flag's
effect was established behaviorally instead.

## Containment

Every row below produced the same result on all three variants unless noted.

| Case | What is measured | Result |
|---|---|---|
| `X1` | `@throw` inside `session_open` | `MP_SHIM_NATIVE_FAILURE`; no handle written on the failure path |
| `X2` | `@throw` before the callback | Contained; 1 native failure, 0 delivered, 3 suppressed of 4 work items |
| `X3` | `@throw` after the callback returned | Contained; 1 delivered, 1 native failure, 3 suppressed |
| `X4` | `@throw` during close | First close returns `MP_SHIM_NATIVE_FAILURE` **and** leaves 0 live native objects; repeated close returns `MP_SHIM_OK` |
| `X5` | C++ `throw` at the same boundary | `objective-c++`: caught by the `@catch (...)` catch-all, strong local still released. Both Objective-C variants: `MP_SHIM_UNSUPPORTED`, because a C++ `throw` cannot be written there at all |
| `P1` | Rust callback panics, contained by `catch_unwind` | Reaches the native side as status 3, 1 reported callback failure, 0 native failures, 0 live objects after release |
| `P2` | The same panic with no `catch_unwind`, in a child process | Child aborted with signal 6 on every variant |

`X4` is the teardown rule in one line: a cleanup failure is reported without
costing the cleanup. `P2` is why the Rust side of the boundary must catch its own
panics — nothing on the native side can.

## Autorelease behavior in the frame path

Frames are produced one per work item on a serial `dispatch_queue`, which is the
shape ScreenCaptureKit delivers in. Temporaries are placed in the current
autorelease pool and their deallocation is counted.

| Case | Work items × per item | Explicit pool | Live after creating, in the item | Live at the next item's start | Live after the pool block | Peak | Live once idle |
|---|---|---|---|---|---|---|---|
| `A1` | 4 × 8 | no | 32 | 8 | not measured | 32 | 0 |
| `A2` | 4 × 8 | yes | 8 | 0 | 0 | 8 | 0 |
| `A3` | 256 × 4 | no | 1,024 | 4 | not measured | 1,024 | 0 |

All three variants produced identical numbers, so this is not a language
difference — it is a rule the shim needs either way.

The work-item pool does not drain between work items. In `A1` the eight
temporaries from the first item are still alive when the second starts, and by
the fourth item all thirty-two are alive. `A3` shows what that costs over a
realistic run: 1,024 live temporaries across 256 frames, released only once the
queue went idle. With an explicit `@autoreleasepool` per work item the peak falls
to the per-item working set: 8 in `A2`'s 4 × 8 shape. The harness stops there, so
whether that peak stays flat over a long run was measured separately during review —
it does, at 4 items, 256, and 1,024, and the like-for-like comparison against `A3`
is 1,024 unpooled against 4 pooled. See
[measurements added during review](#pooled-autorelease-over-a-long-run).

## Callbacks, the fence, and teardown

| Case | What is measured | Result |
|---|---|---|
| `C1` | Whether delivery outlives `produce` | 0 delivered when `produce` returned, 64 after the queue drained |
| `C2` | Callbacks admitted after the fence returned | 0, on every variant, out of 512 work items; delivered plus suppressed is exactly 512 |
| `C3` | Callback context freed right after the fence, producer kept running | Delivered did not rise; 544 work items suppressed |
| `C4` | Repeated fence, fence after close, repeated close, release | `MP_SHIM_OK` for all six calls, 0 live objects afterwards |
| `L1` | Three full open/produce/fence/close/close/release cycles | 24 frames delivered, 0 live native objects after each cycle |
| `L2` | Frame-path retain and release balance | 33 allocations, 33 deallocations, exactly one object alive before close — the retained stream object |
| `L3` | Context freed after the fence | Delivered unchanged, 0 admitted after the fence, 0 live objects |

`C1` is what makes the rest of the group meaningful: delivery genuinely continues
after `produce` returns, so a caller that frees its state without a fence would be
racing a live producer. `C2` and `C3` are the fence doing its job, and the
suppressed counts prove the producer was still running rather than quietly
finished.

The number of frames delivered before the fence takes effect is scheduling
dependent. In `C2` the retained run measured 71, 69, and 81 across the three
variants — the same number, three values. `L3`'s `delivered_at_fence` and
`frames_observed_by_host_at_fence` are the same kind of quantity; the retained run
records 0 for all three variants, and an unretained re-run of the same binary during
review measured 6 for one of them, which is what makes the point that the value is
not a property of the code.

Nothing in the decision depends on either number. What the cases assert as gate
conditions, so that a run where one failed would have reported a failed case, is
narrower than a single sentence can cover: `L3` and `C2` require
`admitted_after_fence` to be 0, `L3` and `C3` require delivered not to rise after the
fence, `C2` alone requires delivered plus suppressed to account for all 512 work
items exactly, and `C3` requires at least its 512 later items to be suppressed. The
fence's guarantee is checked; the exact accounting identity is checked in one case.

One more limit on `admitted_after_fence`, since these three cases lean on it: the
counter is incremented on the normal return path only, so it would miss an item that
was admitted after the fence and then raised before the callback returned.
[probe.md](probe.md#the-admission-and-fence-protocol) says where that happens. None of
these three cases injects a fault, so nothing in them can take that path — but the
zero they report is "nothing admitted and delivered", not "nothing admitted".

## Measurements added during review

Two claims in the first draft of this record went further than the harness run
supported. Both were re-measured on the same host, against the same prototype
sources, during the pre-landing review. They are not part of the harness report,
which is why they are recorded here with their own method.

### The catch-all handler is reachable in plain Objective-C

The containment rule keeps a `@catch (...)` arm after the `@catch (NSException *)`
arm. Whether that arm can ever run in Objective-C decides whether it is load-bearing
or decoration, and the first draft of this record got it wrong by asserting it could
not.

It can. A single Objective-C file — ARC, `-fobjc-arc-exceptions`, Foundation only —
that does `@throw` of an `NSString` lands in the catch-all arm, in a binary whose
`otool -L` lists no libc++. `@throw` accepts any object, and
`@catch (NSException *)` does not match one that is not an `NSException`.

The prototype reaches that arm only through the Objective-C++ `throw` in `X5`, so
this is a language property established beside the harness run rather than a case
inside it.

### The libc++ requirement, deconfounded

The question the build table cannot answer: does compiling *this shim* as
Objective-C++ require libc++, or did the original run only show that because the
Objective-C++ variant contained a C++ throw and was handed `-lc++`?

Method: one gated copy of the implementation, compiled twice from the same source
text, both with `-fobjc-arc -fobjc-arc-exceptions -O2 -g`, the C++ throw excluded
from the Objective-C++ build by a compile-time guard, and neither link given
`-lc++`.

| | Objective-C | Objective-C++, every C++ construct removed |
|---|---|---|
| Compile, best of three | 0.267 s | 0.326 s (+22%) |
| Object bytes | 78,760 | 85,224 (+8.2%) |
| Undefined EH symbols | `___objc_personality_v0` | the same, plus `___cxa_begin_catch` and `std::terminate()`, both reached through `___clang_call_terminate` |
| Linked as a dylib without `-lc++` | succeeds | **fails**: `Undefined symbols … std::terminate(), ___cxa_begin_catch` |

So the libc++ requirement is a property of the language mode, not of the prototype's
C++ test: an Objective-C++ translation unit with no C++ in it at all still cannot
link without libc++. The mechanism is the cleanup path — `@finally` and ARC cleanup
regions, which clang's Objective-C++ codegen routes through
`___clang_call_terminate`, a libc++abi entry point. A smaller probe with no
`@finally` shows no libc++ symbols at all, which is why this had to be measured
against the real implementation rather than a toy.

The deconfounded cost is therefore +8.2% object bytes and +22% compile time, both
against the *selected* variant rather than the rejected unflagged one. The original
table's 20% and 21% compared against the unflagged variant and included the C++
test; neither number should be quoted as the language's cost.

One caveat governs every byte count in this record. `-g` embeds symbol names and
source paths in DWARF, so the same source with the same flags changes size when only
the link prefix or the containing directory changes. Measured: swapping a
two-character prefix for a fourteen-character one moves one object by 2,248 bytes,
and relocating the prototype moved every object in the recorded report by exactly 16
bytes (77,480 → 77,496, 80,784 → 80,800, 92,648 → 92,664). The table above holds
prefix length and path constant, which is what makes its +8.2% meaningful; its
absolute numbers must not be compared with the build-integration table's. No part of
the decision rests on a byte count — the libc++ result is a link that either resolves
or does not.

### Pooled autorelease over a long run

The harness measured the explicit pool only over four work items, so "the peak does
not grow with the run" was an inference. Measured directly, by calling the shim's
autorelease probe from a one-off C driver linked against the same static archive:

| Case | Peak live temporaries | Live once idle |
|---|---|---|
| Pooled, 4 items × 8 | 8 | 0 |
| Pooled, 256 items × 4 | 4 | 0 |
| Pooled, 1,024 items × 4 | 4 | 0 |
| Unpooled, 256 items × 4 | 1,024 | 0 |

With a pool per work item the peak equals the per-item working set and is flat from
4 items to 1,024. The like-for-like comparison at 256 × 4 is **1,024 unpooled
against 4 pooled**; the first draft compared 1,024 against the 4 × 8 pooled peak of
8, which is a different per-item size.

## What this evidence does not cover

- **Full Xcode.** Not installed on this host, so not evaluated. No parity claim.
- **Any capture or input API.** No ScreenCaptureKit stream was created and no
  `CGEvent` was posted. SDK availability was probed with
  `NSClassFromString(@"SCStream")`, `dlsym` for `CGEventCreateMouseEvent`, and
  `@available(macOS 12.3, *)` — all true here, none of which presents or requests
  Screen Recording or Accessibility access.
- **The minimum supported macOS version**, which remains
  [`G-001`](../../validation-gates.md#g-001). The deployment target was set to
  11.0 so ScreenCaptureKit was genuinely newer than the minimum, and `otool -L`
  confirms it is recorded as a weak dependency. That the arrangement links and
  loads on macOS 26.5.2 is not evidence about macOS 11.
- **Any signing context other than one.** An ad-hoc linker signature on a
  command-line binary with no bundle and no entitlements. Bundled,
  hardened-runtime, and notarized contexts were not evaluated.
- **A Rust-only boundary.** Whether `objc2`'s exception feature could replace the
  shim was not measured; ADR 0012 records it as a rejected alternative on
  reasoning, and says so.

## Redaction

The report contains counts, statuses, timings, and toolchain strings. User paths
are replaced with `<home>`, `<probe>`, and `<out-dir>`. It carries no captured
desktop content, no recognized text, no window titles, and no input events,
because the prototype never captures or injects anything: frames carry synthetic
identity and geometry metadata only.
