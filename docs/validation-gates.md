# Validation gates

A validation gate is a version-one decision that is deliberately unresolved
because the evidence needed to settle it does not exist yet. Each gate records
what is undecided, what evidence resolves it, when it must be resolved, what it
blocks, and how the resolution is recorded.

Gates exist so that an unresolved decision stays visible instead of being settled
by accident in an implementation pull request. A gate is not permission to weaken
an architectural seam recorded in [architecture.md](architecture.md).

## How a gate is used

Before entering or exiting a phase, check every gate whose **Due** column names
that phase. A phase must not exit while a gate that blocks its exit is open, and
a phase must not begin implementing behavior that an open gate blocks.

Resolving a gate requires all of the following in one change:

1. The evidence named in the gate, committed or referenced from a tracked
   location.
2. An architecture decision record created from
   [adr/0000-template.md](adr/0000-template.md), recording the decision, the
   evidence, the rejected alternatives, and the consequences.
3. A synchronized update to [architecture.md](architecture.md) and any affected
   specification, test, policy, or example.
4. An update to this registry marking the gate resolved with its ADR number, or
   removing the gate when the underlying claim was withdrawn instead.

A gate is never presented as resolved on the strength of a plan, an opinion, or a
successful build alone.

## Status of Phase 0

No gate blocks Phase 0. Phase 0 is nonetheless incomplete until every gate below
has an explicit due phase, blocking scope, and resolution rule, because that
registry is itself a Phase 0 deliverable.

## Registry

| ID | Unresolved decision | Due | Blocks | Status |
|---|---|---|---|---|
| [`G-001`](#g-001) | Minimum Windows and macOS versions | Before Phase 2 exit | Windows and macOS support claims | Resolved by [ADR 0019](adr/0019-windows-qualified-system-and-controlled-availability.md) and [ADR 0014](adr/0014-macos-qualified-host-and-frame-placement.md) |
| [`G-002`](#g-002) | Windows capture producer-pool and frame-detachment strategy | Before Phase 2 implementation | Windows capture ownership | Resolved by [ADR 0013](adr/0013-windows-capture-frame-detachment.md) |
| [`G-003`](#g-003) | macOS shim language | Before Phase 2 implementation | macOS shim implementation | Resolved by [ADR 0012](adr/0012-macos-shim-language-and-containment.md) |
| [`G-004`](#g-004) | Default OCR model profile | Before Phase 3 implementation | Default OCR profile | Open |
| [`G-005`](#g-005) | Default change-detection algorithm and threshold | Before Phase 4 implementation | Default watcher policy | Open |
| [`G-006`](#g-006) | Acceleration candidates and provider ordering | Before Phase 5 implementation | Acceleration defaults | Open |
| [`G-007`](#g-007) | Native dependency bundling profiles | Before Phase 5 implementation | Release packaging | Open |
| [`G-008`](#g-008) | Static-library feasibility | Before Phase 5 exit | Static artifact claim only | Open |
| [`G-009`](#g-009) | Stable public Rust item names | Before Phase 1 exit | Rust stability promise | Resolved by [ADR 0006](adr/0006-public-rust-names-and-compatibility-policy.md) |
| [`G-010`](#g-010) | Version-one C ABI status, prefix, and layout | Before Phase 1 exit | ABI compatibility baseline | Resolved by [ADR 0007](adr/0007-phase-1-c-abi-freeze.md) |
| [`G-011`](#g-011) | Native-frame extension discovery | Future roadmap | Does not block version one | Deferred |
| [`G-012`](#g-012) | Published Cargo and C build profiles | Before Phase 5 implementation | Release capability matrix | Open |
| [`G-013`](#g-013) | Numeric benchmark budgets | Before each affected phase exits | That phase's exit | Open per workload; Phase 1's thirteen resolved by ADR 0008, and the affected Phase 2 diagnostic, native input, controlled ownership, production capture/transition, and corrected dual-4K workloads resolved by ADRs 0024–0032. Exact-candidate Phase 1 reruns preserve those ceilings as exit evidence rather than another budget decision |
| [`G-014`](#g-014) | Archive safety ceilings | Before Phase 1 implementation | Version-one archive loading | Resolved by [ADR 0001](adr/0001-asset-archive-container-and-safety-ceilings.md) |

## G-001

**Decision.** [ADR 0019](adr/0019-windows-qualified-system-and-controlled-availability.md)
fixes the Windows floor at Windows 11 25H2 build family 26200 on a currently
serviced x64 desktop installation. [ADR 0014](adr/0014-macos-qualified-host-and-frame-placement.md)
fixes the macOS floor at Apple Silicon macOS 26.5.2 (25F84), SDK 26.5.

**Required evidence.** Windows build, process-load, SDK, API-availability,
unsupported-capability, native Rust, C, and C++ evidence is retained in
[`windows-minimum-system.md`](evidence/g-001/windows-minimum-system.md). macOS
deployment metadata and controlled linkage are pinned. Its
2026-08-01 permissioned ASan run passed all 95 library tests with live display and
window capture, signed and mixed-scale placement checks, retained-filter discovery,
and no sanitizer finding. Two later manual runs moved the window fully onto a 2x
display while thousands of frames kept arriving, but the open stream stayed at its
old 1x extent and effective scale; only fresh discovery after close saw 2x. That
exposed the missing same-sample producer-capacity signal. The repaired, hardened
permissioned probe then passed 2/2 over 4,097 frames: 3,371 same-scale moves preserved
their epoch and 30 cross-scale moves advanced exactly from epoch 0 through epoch 30,
publishing both the 1x and 2x extents without a stall. Cross-scale acceptance is
closed. A fresh post-repair ASan build then passed all 101 library tests with live
capture scenarios running and no sanitizer finding. On 2026-08-09 the
[owned-window replacement probe](evidence/g-001/macos-owned-window-replacement.md)
destroyed the selected fixture window, created a same-process successor with
distinct content, and proved the retained filter published none of that content
while a fresh session captured it and the retained original mapping stayed
unchanged. ScreenCaptureKit remained quiescent rather than reporting explicit
loss, so the Adapter correctly did not infer `TargetLost`. This closes the macOS
replacement acceptance item.

The 2026-08-22 Windows positive run used Pro build `26200.9168`, SDK
`10.0.26100.0`, and clean source `834a58f`; build, lazy-import, native Rust,
product-DLL, C/C++, frozen ABI 1.0, ownership, fixture, and CMake rows passed.
Final repair source `c6ff39a` then used the isolated missing-WinRT-D3D-export
apparatus: Rust, C, and C++ loaded successfully, discovery returned typed
`Unsupported`, and the ordinary supported paths passed after restoration.

**Due.** Before Phase 2 exit.

**Blocks.** The Windows support claim and release.

**Status.** Resolved. ADR 0019 accepts the Windows minimum; ADR 0014 accepts the
macOS minimum, cross-scale movement, and owned-window replacement boundary.

**Resolution.** ADR 0019 records the Windows minimum and its build, process-load,
SDK, API-availability, unsupported-capability boundary, native Rust, C, and C++
evidence. ADR 0014 records the complete macOS resolution.

## G-002

**Decision.** The Windows Graphics Capture producer-pool sizing, public-frame
detachment, and texture-reuse strategy.

**Required evidence.** A retained-frame stress prototype showing that published
frames do not pin producer buffer-pool slots that capture progress requires, and
that native resources outlive in-flight mapping and backend work.

**Due.** Before Phase 2 implementation.

**Blocks.** Windows capture ownership design until resolved. It does not by
itself make native Windows capture an implemented capability.

**Status.** Resolved by
[ADR 0013](adr/0013-windows-capture-frame-detachment.md). The producer pool holds
two frames; the callback copies each publishable frame into an Adapter-owned
D3D11 texture and releases the WGC frame before publication; a compatible private
texture is reused only after every public-frame, mapping, and backend lease
releases it.

**Resolution.** The revision-bound
[G-002 record](evidence/g-002/README.md) compares four ownership candidates at
pool sizes two, three, and four; exercises retained mapping, backend lifetime,
resize, 100 close races, target close, injected reset, and two 4K displays; and
records the final supported MSVC/SDK confirmation without captured pixels. A
deterministic queued-delegate case proves owner admission is fenced, while
complete close/reset and sequence freshness are measured explicitly. Direct WGC
retention stalled every producer pool and blind texture reuse changed retained
content. Lease-aware detachment at pool size two passed every hard gate.

**Implementation obligation.** The later Windows capture Change must implement
the [retained-frame contract-test plan](windows-capture-contract-tests.md), keep
the detached pool finite with observable pressure drops, and establish affected
Phase 2 `G-013` budgets. It may not treat the ADR or prototype as a support
claim.

## G-003

**Decision.** The language of the macOS capture and input shim, and the
containment rules of that boundary.

**Required evidence.** A prototype covering exception behavior across the language
boundary, object ownership, and build integration on Apple Silicon.

**Due.** Before Phase 2 implementation.

**Blocks.** macOS shim implementation.

**Status.** Resolved by
[ADR 0012](adr/0012-macos-shim-language-and-containment.md). The shim is
Objective-C with ARC, compiled with `-fobjc-arc-exceptions`.

**Resolution.** The prototype built one implementation file as Objective-C, as
Objective-C with `-fobjc-arc-exceptions`, and as Objective-C++, and ran eighteen
cases on each on the approved Apple Silicon host; the measurements are in
[evidence/g-003/](evidence/g-003/README.md). All three variants contain every
injected native exception, so the gate was not decided by exception handling. It
was decided by what containment costs in ownership: without
`-fobjc-arc-exceptions`, ARC emits no release on the unwind edge, and an exception
raised at the position where a stream start or a frame callback would fail leaves the
object the session retained, or the frame object, alive — the counter observes the
retained object, and the session's own lifetime is an inference from it. The control
variant reproduces the Objective-C++ result with that one flag, so Objective-C++'s
ownership advantage is a default rather than a language property, and it requires
libc++ in every consuming process for a boundary that contains no C++ — a
requirement re-measured during review with every C++ construct removed, so that it
rests on the language mode rather than on the prototype's C++ test.

The ADR also records the containment rules the evidence forces — a catch-all
boundary handler, the mandatory exception flag, borrowed frames, a per-work-item
autorelease pool, a disable-and-drain callback fence, teardown that reports
failure without skipping cleanup, Rust-side panic containment, and shim-owned
linkage with availability gating — together with the tests the implementing Change
must carry. `mado-pilot-platform-macos` now implements the boundary and carries
those tests, so the rules are enforced by the package rather than by review. The
linkage rule is met by controlled dynamic loading rather than by weak framework
linking, because Cargo does not propagate a dependency's `rustc-link-arg` to the
binary that consumes the dependency; the ADR records that amendment and the
property it preserves.

## G-004

**Unresolved decision.** The default OCR model, its language set, size,
preprocessing metadata, expected hash, and license.

**Required evidence.** A cross-target quality fixture showing reproducible
recognition results on both release targets, plus a license review confirming
redistribution is permitted.

**Due.** Before Phase 3 implementation.

**Blocks.** The default OCR profile.

**Status.** Open.

**Resolution.** An ADR recording the model choice, the fixture results, and the
license and deployment obligations, followed by an update to
[third-party-dependencies.md](third-party-dependencies.md).

## G-005

**Unresolved decision.** The default change-detection algorithm and its threshold
for watcher scheduling.

**Required evidence.** A false-skip evaluation over recorded frame sequences,
showing how often a real change would be skipped at the chosen threshold.

**Due.** Before Phase 4 implementation.

**Blocks.** The default watcher policy.

**Status.** Open.

**Resolution.** An ADR recording the evaluation and the chosen default, plus the
recorded sequences kept as regression fixtures.

## G-006

**Unresolved decision.** The Core ML and Windows acceleration candidates and the
execution-provider ordering.

**Required evidence.** Compatibility and correctness runs for each candidate
provider on its release target, including the observable behavior when a provider
is rejected during model loading.

**Due.** Before Phase 5 implementation.

**Blocks.** Acceleration defaults.

**Status.** Open.

**Resolution.** An ADR recording the candidate results and the provider ordering,
plus the fallback policy that limits fallback to model loading.

## G-007

**Unresolved decision.** Whether OpenCV and ONNX Runtime are bundled or consumed
from a controlled host-provided installation, and which release profiles exist.

**Required evidence.** Clean-system package prototypes for each candidate profile
on both release targets, plus a license review of every redistributed artifact.

**Due.** Before Phase 5 implementation.

**Blocks.** Release packaging.

**Status.** Open.

**Phase 1 input.** Phase 1 links OpenCV 4.14.0 as a *development prerequisite* and
claims nothing about a release. Two facts it established belong to this gate. The
library is Apache-2.0, the same licence as this project, so bundling it would add
an attribution obligation and no new term. And because OpenCV is linked dynamically
at load time, an absent library stops the process before any MadoPilot code runs,
so it cannot be reported as an actionable status — the adapter reports an
unsupported *version* and nothing more. Closing that gap is part of this gate's
controlled library search paths; it is not `G-008`, whose scope is static-link
feasibility. See
[third-party-dependencies.md](third-party-dependencies.md#opencv).

**Resolution.** An ADR recording the profile matrix, the controlled library search
paths, and the license and notice obligations, followed by updates to
[third-party-dependencies.md](third-party-dependencies.md) and the packaging
documentation.

## G-008

**Unresolved decision.** Whether a static library is feasible for each advertised
dependency combination.

**Required evidence.** Link results and license review for each combination that a
static artifact would advertise.

**Due.** Before Phase 5 exit.

**Blocks.** The static artifact claim only. It does not block the shared-library
release.

**Status.** Open.

**Resolution.** An ADR that either records the supported static combinations with
their evidence, or withdraws the static artifact claim.

## G-009

**Decision.** The reviewed public Rust item names of the facade, and the
compatibility policy that now applies to them.

**Required evidence.** The Phase 1 vertical slice with working Rust examples, and
an interface review of the names those examples exercise.

**Due.** Before Phase 1 exit.

**Blocks.** Any Rust API stability promise.

**Status.** Resolved by
[ADR 0006](adr/0006-public-rust-names-and-compatibility-policy.md). The review
took the thirteen questions and two interface gaps this gate had accumulated —
raised by writing the Rust example, the C ABI, and the C++ wrapper against the
provisional names — and settled every one.

Six items were renamed: `EngineParts` to `EngineWiring`, `FrameChoice` to
`SearchFrame`, `Engine::prepare` to `Engine::prepare_template`, the old
`Engine::prepare_template` to `Engine::prepare_from_package`, `Session::frame` to
`Session::acquire_frame`, and the three asset constants at the facade root to
`ASSET_SCHEMA_VERSION`, `ASSET_MANIFEST_PATH`, and `ASSET_HASH_ALGORITHM`. Four
things were added: `MatchResult::options`, `ContentDigest::of`,
`ReplayEngineRequest` — which closes the asset-limits gap — and facade
re-exports of `AssetLimits`, `BackendDescriptor`, and `BackendId`, three types
public methods returned but no caller could name. One behaviour was aligned:
`Session::find_template` now reports `Status::Closed` for every frame choice, so
the Rust and C surfaces agree about what a closed session does.

The rest were kept with the reason recorded, including the two the ADR expects to
revisit: `StreamId` and `TargetId` stay opaque with no fixed-width projection,
which becomes a real defect only when one engine is shared across the language
boundary, and `REQUIRED_BACKEND` stays a `&str` policy constant until a second
backend introduces a selection axis to name.

The policy: these names are the `0.x` baseline and are reviewed rather than
stable. Renaming or removing one is a breaking change needing a superseding ADR
and a version bump; adding is free; changing what a name means while keeping it
is refused outright. The promise begins at 1.0.

**Resolution.** An ADR recording the reviewed names and the compatibility policy
that then applies to them.

## G-010

**Decision.** The exact version-one C status codes, the mandatory function-table
prefix, the structure layouts, and the mapping from Rust errors to C status
values.

**Required evidence.** The Phase 1 minimal C ABI exercised by C and C++ examples,
with owned-handle lifecycle, structure-size negotiation, and error-ownership tests
passing.

**Due.** Before Phase 1 exit.

**Blocks.** The ABI compatibility baseline, and therefore every later
old-header-prefix compatibility claim.

**Status.** Resolved by [ADR 0007](adr/0007-phase-1-c-abi-freeze.md). The ABI is
frozen at major 1, minor 0: thirteen status values with fixed numbers, a
forty-byte mandatory table prefix ending at `status_text`, twenty-three
structures whose sizes, alignments, and field offsets are the tracked reports in
[evidence/c-abi/](evidence/c-abi/), and a Rust-to-C status mapping with one arm
per `Status`.

Every row of this gate's evidence table is now filled. The two that were
outstanding: the mapping review happened and is recorded in the ADR — including
why `MADOPILOT_STATUS_INTERNAL_PANIC` is the only C-only value and why a
`Status` added to Rust later reports as `MADOPILOT_STATUS_INTERNAL` rather than
as the nearest existing category — and the frozen old-prefix fixture exists at
`crates/bindings/capi/tests/abi-compat/v1/`, where `c-abi-check` compiles a C
program against the frozen header instead of the working one, links it to the
library built now, negotiates at both the full table size and the mandatory
prefix, and runs the complete flow.

The six decisions this gate held were settled as: the C status vocabulary does
not diverge further; the root handle stays `madopilot_engine_t` and the
specification's wording follows the code rather than the other way round;
`tmpl` stays and `template` stays the concept, at the cost of one awkward field
and a permanent C++ member-naming constraint;
`MADOPILOT_ASSET_FAULT_UNKNOWN_TEMPLATE` is now reachable, because
`template_prepare_from_package` resolves the identity before asking the backend
for anything; the asset detail does not generalize and a second structured
detail is appended rather than folded into it; and freezing the values makes a
generated C++ mirror safe but not a hand-written one, so the wrapper keeps
aliasing.

Two function-table entries were renamed in the same change, before the freeze
took effect: `session_frame` to `session_acquire_frame` and `template_prepare`
to `template_prepare_from_package`. Their offsets did not move.

**Resolution.** An ADR freezing the allocation and layout rules, followed by the
ABI layout and old-header compatibility tests that enforce them.

## G-011

**Unresolved decision.** How a native-frame extension table is discovered and
allocated in the C ABI.

**Required evidence.** A portable extension prototype covering discovery,
allocation, and version negotiation.

**Due.** Future roadmap.

**Blocks.** Nothing in version one.

**Status.** Deferred. An ADR is required if this is activated.

**Resolution.** An ADR recording the prototype and the negotiation rule, plus a
scope update that moves the feature out of the future roadmap.

## G-012

**Unresolved decision.** The published Cargo feature defaults and the C build
profiles.

**Required evidence.** A feature-matrix build across both release targets with
binary-size measurements for each profile.

**Due.** Before Phase 5 implementation.

**Blocks.** The release capability matrix.

**Status.** Open.

**Resolution.** An ADR recording the matrix and the chosen defaults, plus a
build-profile capability table published with the release.

## G-013

**Unresolved decision.** The numeric benchmark budgets for every workload a phase
introduces.

**Required evidence.** Repeatable baseline measurements captured with the profile
format in [performance.md](performance.md), on the release target the workload
runs on.

**Due.** Before each affected phase exits.

**Blocks.** That phase's exit.

**Status.** Open per workload. Phase 0 defined the profile and budget format and
deliberately assigned no numeric product budget.

**Phase 1 is resolved** by
[ADR 0008](adr/0008-phase-1-performance-budgets.md). Thirteen workloads across
two benchmarks are measured on both release targets — Apple M1 Pro under macOS
26.5.2 and Core i7-12700KF under Windows 11 Pro 26200 — two hundred samples each
after twenty warm-up iterations, every sample checked against its oracle, and
zero oracle failures anywhere. Four profiles are committed under
[benchmarks/](benchmarks/): `phase-1-deterministic-slice-*` for the eight-operation
Rust workflow and `phase-1-c-boundary-*` for what the C ABI costs. Each
benchmark's two profiles share a fixture hash, so both targets measured the same
bytes.

Two hard gates apply to every workload on both targets — `result_correctness` is
zero and `allocated_growth_bytes` is bounded at one page, and both targets
measured zero growth everywhere. Live heap peaks below half a mebibyte across
the whole slice. The numeric latency ceilings are regression ceilings rather
than product requirements, set at three times the measured value, and the ADR
says so rather than dressing them as a requirement nobody has stated.

`map_full_frame` deliberately has **no** latency budget: it measures exactly zero
on `x86_64-pc-windows-msvc`, because a matching-format mapping is a
reference-count increment and the host clock is coarser than that. It is bounded
by `mapped_bytes_per_result` and by a batched `iteration_span_ms` instead, on
both targets, so the two profiles agree about what the workload may do.
`negotiate_table` is bounded the same way, for the same reason.

`engine_create_rust` and `match_warm_rust` carry no per-measurement ceiling at
all. Each is the control for the C workload above it in the same profile,
measured so the pair can be compared in one process, one build, one run; the
Rust workflow's own ceilings live in the deterministic-slice profile, and a
second set beside the control would be the same claim measured twice and free to
disagree with itself. Eleven of the thirteen workloads therefore carry a
per-measurement ceiling, and all thirteen sit under the two hard gates above,
which apply to every measurement in their file.

Task 9.2's "any material C ABI startup overhead" is answered rather than
assumed. The boundary costs a fixed amount per table entry: negotiation does not
register on either host's clock, engine creation costs a sub-microsecond
constant more than the facade, and a warm match costs 0.1% more on
`aarch64-apple-darwin` and 3.4% more on `x86_64-pc-windows-msvc` across four
crossings. It is not material at this size of work, and the ADR records why that
conclusion does not automatically transfer to a later phase's per-frame entry
point.

**Phase 2 is resolved for `v0.2.1`.**
[ADR 0024](adr/0024-input-diagnostic-performance-budgets.md) accepts the
`aarch64-apple-darwin` diagnostic profile. ADR 0026 accepts the matching Windows
profile. Both targets retain 200 samples after 20 warmups for each of ten
capture/mapping, input, overflow, and close/drain workloads, with zero oracle
failures and allocation growth, exact mapped-byte accounting, and per-workload
regression ceilings.

The `native-phase2` workloads are resolved on Windows and partially resolved on
macOS. ADR 0020 historically accepted three macOS profiles at source
`a1faf04505c8471deb4de8c136fddcc7f76105e7`; [ADR 0021](adr/0021-invalidate-phase-2-native-performance-evidence.md)
invalidated all three after source drift and false-positive stimulus and
latest-frame oracles. The macOS capture and transition profiles remain
`normative = false`.

[ADR 0025](adr/0025-macos-native-input-performance-budgets.md) replaces the
macOS input and public-language profile and records its post-review refresh at
source `c4bc8135ae36cf9b110fc435e4fa1b8dfc3ba848`. Its six workloads retained
300 correct samples with zero allocation growth, exact mapped-byte accounting,
and measured Rust-heap and child-process resident bounds.

[ADR 0026](adr/0026-windows-native-and-diagnostic-performance-budgets.md)
accepts the post-review Windows diagnostic, capture, transition, and
input/public-language requalification at source commit
`6873d4b05a13fd15cb3ffd961892b1153f606d78`, implementation tree
`2483269ee071d14adfe14f829d318a4c59337f85`. Its 2,980 retained samples all
satisfy their exact oracles, report zero allocation growth, and pass the
unchanged ceilings. The rejected precursor runs proved four apparatus defects
rather than product failures: insufficient post-resize fixture publication,
reuse of a 1,024-event fixture for 2,050 redacted summaries, a missing child-only
Cargo profile DLL path, and a C++ oracle that expected macOS submission evidence
on Windows. The ADR records the bounded repairs and target-specific oracle.

[ADR 0030](adr/0030-macos-production-capture-performance-budgets.md) accepts the
separate macOS production-capture and production-transition profiles at measured
source `d182300cd8710891ded6cba17184c44d6d58a114`, tree
`c570343d334a5c77415e6a885ef8821c731b0ad5`. Their eight workloads retain 1,150
correct samples with zero allocation growth and exact mapped-byte accounting on
the approved exactly-two-display mixed-scale host. The executable harness now
enforces every accepted latency, live-heap, mapped-byte, correctness, and growth
budget and requires the fixture's exact next frame-authoritative target geometry
after resize. The ADR retains the rejected enforcement-repair attempt and changes
no historical profile or source attribution.

[ADR 0031](adr/0031-windows-1280-production-capture-performance-budgets.md)
accepts the separate Windows 1280×720 profiles. Shared-marker source `f50285a`,
tree `4c2f23f`, reran all four capture workloads with zero
correctness/allocation failures, exact mapping/copy bytes, and nonzero resource
counts at or below their limits. Transition source `7c31752`, tree `4e99487`,
reran all five lifecycle workloads after callback completion/binding
instrumentation changed their publication path; every unchanged gate passed.

[ADR 0032](adr/0032-windows-dual-4k-production-capture-performance-budgets.md)
accepts the corrected mixed-DPI dual-4K profile at shared-predicate source
`f50285a`, tree `4c2f23f`. It retains 600 stationary samples per display plus
300 controlled moving-seam samples, requires requested-position markers on both
frames, binds each frame to its own post-baseline callback record, and passes
every latency, mapping/copy, resource, stale-work, heap, resident, correctness,
growth, and cleanup gate.

The Windows controlled `native-phase2`, ADR 0031 1280×720, and ADR 0032 dual-4K
profiles remain distinct. Historical measurements keep their original source
identities. Final-source Phase 1 reruns preserve their existing ceilings and
remain exact-candidate exit evidence rather than another profile lineage.

OCR, watcher scheduling, and acceleration remain open for the phases that
introduce them.

**Resolution.** Committed benchmark profiles and budgets plus an ADR for each
budget that is set or relaxed, recording the evidence behind the number.

## G-014

**Decision.** The archive entry-count, uncompressed-byte, and compression-ratio
safety ceilings for asset loading.

**Required evidence.** Adversarial fixtures covering traversal, links, special
files, duplicate normalized entries, and decompression bombs, each rejected
deterministically at the chosen ceilings.

**Due.** Before Phase 1 implementation.

**Blocks.** Version-one archive loading.

**Status.** Resolved by
[ADR 0001](adr/0001-asset-archive-container-and-safety-ceilings.md). The
container is ZIP restricted to `Stored` and `Deflated`, the manifest is strict
UTF-8 JSON, and six implementation ceilings are set: 4 MiB of manifest bytes,
4,096 entries, 64 MiB per entry, 256 MiB of source bytes, 512 MiB of total
uncompressed bytes, and an expansion ratio of 64. A caller may lower a limit and
may not raise one. Three ceilings beyond the ones this gate named were added,
because entry count, expansion bytes, and ratio do not on their own bound what a
loader allocates.

The evidence is in [evidence/g-014](evidence/g-014/) and the fixtures in
[../fixtures/assets/g-014](../fixtures/assets/g-014/). `mado-pilot-assets`
implements the decision, and its conformance suite asserts the failure category
and the refusing stage for every tracked adversarial fixture, on both release
targets. A fixture refused later than its documented stage fails that suite even
though the package was refused, which is what keeps an earlier guard from being
quietly removed.

**Resolution.** An ADR recording the ceilings and the adversarial fixture results,
followed by the asset schema and security documentation that states them.
