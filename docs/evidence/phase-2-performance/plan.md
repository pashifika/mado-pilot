# Phase 2 native performance plan

This plan freezes the Phase 2 `G-013` workloads, correctness oracles, profile
conditions, and privacy boundary before measurement. A timing without its oracle
is diagnostic only. A faster run that accumulates stale work, pins producer
storage, maps extra bytes, grows memory, or leaves input state held fails.

## Source identity

The pre-measurement product source is `origin/dev/0.2.0` commit
`4de3308a7f3619223eca1556e183982d944d4a41`, tree
`3b69262eb8908b05d7b839ba912ee67ca267244d`. That is the exact tree that passed
PR #29 on both native CI targets and the local deterministic baseline.

Each measured report must record its own executable commit and tree. A later
release candidate may reuse a report only when a reviewer records that the
complete intervening diff cannot affect the measured executable, fixture,
compiler inputs, native dependencies, profile format, or oracle. Otherwise the
workload is rerun.

## Runs and sample plans

Two runs are kept separate because capture can be sampled without irreversible
input, while the public-language flow intentionally submits to the repository's
bounded fixture.

| Run | Warmup | Samples | Reason |
|---|---:|---:|---|
| `phase-2-native-capture` | 20 | 200 per steady workload | Matches Phase 1 and gives a useful p95 while exercising finite storage and bounded growth long enough to expose churn |
| `phase-2-native-flow` | 5 | 50 per Rust/C/C++ flow | Limits focus and acknowledged input while retaining enough samples for p95; each iteration is independently acknowledged and cleaned up |

Startup, target-loss, resize/reset, and pressure transitions are not falsely
repeated as steady samples. Each is run 20 times after two warmups and reports
all outcomes plus p50/p95 where a percentile is meaningful. A single unavailable
or unexercised path is recorded as a gap, never as zero latency.

## Capture workloads

| Workload | Measured interval | Correctness oracle |
|---|---|---|
| `native_open_first_frame` | Accepted `open` request through first usable frame commit | Exact selected target and provider; BGRA8 descriptor is non-empty and within bounds; stream identity is fresh; first frame has epoch/sequence/geometry identity consistent with its transform |
| `steady_capture` | `newer_than(previous)` admission through each newer frame | Stream id is stable; ordering is strictly after the prior stamp; unchanged geometry keeps epoch and geometry; every fixture marker is from the generated allowed sequence; no timeout, target change, or invented frame |
| `retained_frame_progress` | Acquire while holding the declared retained capacity, release one, then acquire the recovery frame | Capacity and backing policy equal the session descriptor; producer advances beyond its native producer pool; full storage yields the documented bounded outcome; releasing one slot resumes; any pressure drop is an observable sequence gap; every retained frame remains immutable and mappable |
| `latest_acquisition` | `latest()` admission through returned maintained frame after the fixture advances | Returned stamp is not older than the stamp observed before admission; sequence gaps are counted, never hidden; no queued query is lost; frame and transform stay correlated |
| `map_full_frame_native` | Explicit BGRA8 CPU map of one held native frame | Mapping carries the exact source stamp, width, height, stride, format, and byte extent; the in-memory fixture marker matches; completed bytes remain unchanged after newer frames and session close |
| `resize_recreate` | Fixture resize request through first frame at the new extent | One prospective epoch transition; sequence restarts at first; geometry revision advances; old frames remain readable; no mixed old metadata/new pixels |
| `close_cleanup` | First explicit close admission through completed native teardown | Close is bounded, idempotent, callback admission is drained, no late publication commits, held public frames/mappings remain valid, and platform-owned live-resource counters return to baseline after their last public owner drops |
| `reset_recovery_windows` | Injected device reset/removal through first frame of the recovered or terminal result | No stale pre-reset frame is relabeled; recovery advances epoch or reports the typed terminal fault; native resources return to baseline; retries do not create a second live session |

The Windows 1280×720 fixture and the scheduled two-4K display run are separate
profiles. macOS uses the current display/window fixture and a separately scheduled
external-display profile. A profile never averages across topologies.

## Native flow workloads

| Workload | Measured interval | Correctness oracle |
|---|---|---|
| `input_sequence` | Final preflight through receipt commit | Exact PID-qualified fixture; requested route and focus policy are preserved; receipt is `Complete`; per-route attempts, submitted/last-submitted accounting, address scope, evidence, fault, possible partial native effect, and cleanup agree; fixture observes the bounded event kinds/count without retaining text; no fallback |
| `rust_common_flow` | Native engine creation through discovery, open, frame, map, fixture input receipt, and close | Public Rust API completes against one exact target; every intermediate identity agrees; mapping and receipt survive owner teardown as documented; fixture observes once; no capability substitution |
| `c_common_flow` | ABI negotiation through the same native flow and release of every owned handle | ABI 1.2/table extent is exact; C projections match Rust values; frozen ABI 1.0 ownership rules hold; all handles and borrowed views obey their lifetimes; fixture observes once; no panic/native exception crosses the table |
| `cpp_common_flow` | API negotiation through the same flow using only `madopilot.hpp` and released C ABI | Move-only owners, child lifetime, owned receipt, error copies, and cleanup behave as documented; the wrapper never uses a Rust symbol directly; fixture observes once |

The C and C++ workloads are measured as independent processes. Their process-load
cost is reported separately from in-process ABI negotiation, so dynamic loading
is not hidden inside a label that says table negotiation.

## Measures and hard predicates

Every applicable workload records latency p50/p95, iteration span, correctness
failures, peak/steady process memory, memory growth, Rust live-heap peak/steady/
growth where available, and exact source revision. Native capture additionally
records:

- copied bytes per published frame;
- mapped bytes per result;
- peak producer, detached, staging, mapped, and total retained bytes;
- published frames, sequence gaps, rejected publications, and query outcomes;
- stale/drop/coalesce ratio with numerator and denominator stated; and
- startup, close drain, resize recreation, and reset recovery durations.

Input/common-flow reports attempted routes and submitted event counts, address
scope, evidence, cleanup owed and released, partial/unexecuted outcomes, process
startup, and handle/resource baselines. Sensitive payloads are never a measure.

These predicates are hard gates on both targets:

1. `result_correctness == 0`.
2. Sampled steady memory and every native retained-byte category remain bounded;
   end-of-run growth is no more than one 4 KiB page after warmup.
3. Producer progress succeeds below the declared retained-storage bound and
   resumes after one pressure slot is released.
4. Every drop, supersession, rejection, queue expiry, or sequence gap is counted;
   no query disappears.
5. `mapped_bytes_per_result` equals the requested region's exact byte extent;
   native copied bytes match the ownership implementation rather than a derived
   throughput estimate.
6. Every admitted input sequence returns a receipt; cleanup never releases state
   the sequence did not press, and final held state is zero.
7. Close leaves no platform-owned resource once the last retained public owner is
   released.

Numeric latency and memory ceilings are set independently for each target after
that target produces a valid report. A workload whose clock reading is zero
receives an iteration-span or byte bound, not a fabricated sub-tick latency
limit. A control workload may deliberately withhold a ceiling when its paired
public-boundary workload owns the actual gate.

## Queue and storage policy

- Windows capture: handoff depth 1, `LatestWins`; retained-storage count is
  extent-derived (40 at 4K) and process-shared under the 2 GiB session / 4 GiB
  process retained-byte ceilings; WGC producer pool depth 2.
- macOS capture: handoff depth 1, `Reject`; eight detached buffers guaranteed per
  session; the ScreenCaptureKit producer frame is released before publication.
- Input: synchronous bounded sequence execution; there is no hidden input queue.
  Each flow waits for its fixture acknowledgement and receipt before the next
  sample.

Profiles record the descriptor values observed at runtime. If implementation and
this plan disagree, measurement stops; the plan is not silently rewritten around
the output.

## Fixtures and native context

- Windows capture uses the repository-owned synthetic Win32 window. Input
  profiles use both the ordinary legacy-message fixture and the dedicated
  `MadoPilotInputFixture`: ordinary `WindowMessage` requires unknown
  compatibility and target-queue evidence, while the dedicated path requires
  supported compatibility and protocol acknowledgement. Both preserve focus and
  permit no system fallback. The two-4K profile uses the approved signed-origin
  topology outside work hours.
- macOS uses the repository-owned bundled input fixture, with Screen Recording
  and Accessibility separately granted to the launching terminal. System input
  may focus only the exact PID-qualified fixture. The external-display profile is
  separately scheduled.
- The in-memory marker oracle is generated by the fixture and compared during the
  run; reports retain only pass/fail/count/digest metadata, never pixels or input
  text.

Each report records target triple, CPU/memory, OS version/build, GPU and driver,
display count/bounds/scale through approved derived identifiers, Rust, native
compiler, SDK, CMake, OpenCV, Cargo profile/features, signing mode, Screen
Recording/Accessibility state on macOS, Windows integrity/focus context,
fixture digest, queue policy, warmup/sample counts, exact command, and commit/tree.

## Approved environment declarations

The values below are the approved host declarations frozen for the run. A report
replaces none of them by auto-detection; it records the observed value and fails
qualification when it differs.

| Field | Windows | macOS |
|---|---|---|
| Hardware | Core i7-12700KF, 20 threads, 32 GiB | Apple M1 Pro, 10 CPU cores, 32 GiB |
| OS | Windows 11 Pro 25H2, build family 26200; qualified host `26200.8894` | macOS 26.5.2 (`25F84`) |
| GPU/driver | NVIDIA GeForce RTX 4080, `32.0.15.9186` dated 2026-01-20 | integrated 16-core Apple M1 Pro GPU; driver supplied by macOS 26.5.2 |
| Native toolchain | Visual Studio 2022 17.14.37, MSVC 19.44.35228, Windows SDK 10.0.26100.0, CMake 3.29.5 | Apple Clang 21.0.0, SDK 26.5, CMake 4.4.2 |
| Shared product toolchain | Rust 1.97.1; OpenCV 4.14.0 | Rust 1.97.1; OpenCV 4.14.0 |
| Routine display | one 3840×2160 display at 144 DPI / 150%; bounds recorded at run | built-in 3024×1964 Retina display, main, internal |
| Scheduled display | two 3840×2160 displays at 144 DPI / 150%; primary `[0,0,3840,2160)`, secondary `[-3840,0,0,2160)` in the last accepted run | one external display; bounds and scale recorded only after attachment |
| Permission/signing | no global capture permission; fixture integrity, target integrity, focus, and signature recorded at run | Screen Recording granted; Accessibility granted; generated fixture bundle must be structurally valid ad-hoc |

The macOS non-prompting C ABI probe observed both permission kinds as
`Granted`. The Windows fixture/integrity row and both scheduled-topology rows
remain run observations, not inherited claims; a mismatch creates another
profile or a gap rather than changing this table.

## Privacy

Ordinary output and committed profiles exclude captured images, pixel dumps,
recognized text, input text, credentials, window titles, bundle identifiers,
raw process paths, raw display identifiers, and unrelated desktop metadata.
Target and display references are per-run derived hashes. Raw interactive logs
remain local until a redaction review produces the distilled tracked profile.
