# 2026-10-26: Preparation reuse observations

## Question and source roles

Observe existing image/BGR preparation reuse independently of selector tuning.
Four existing eight-template batch workloads compare retained baseline
`8cc67f5a20a1e4726913cf4da6963b5d658c9e84` with retained current
`36f0cafb02b5e350ea77a6570e68171d44674655` on native Apple Silicon and Windows
x64 MSVC. The existing binaries and original attestations are reused without
rebuilding or relabeling them. No preparation implementation changes here.

Each role runs once per row through protocol-4 instrumented smoke: one warmup,
three retained units, separate manifest discovery, and baseline inspection before
current. This is eight row processes per target, not normal timing, calibration
or qualification. Templates are prepared outside the timed operations. Row-wide
counters include fixture construction, warmup and, for warm-full, an additional
eight-search prewarm. Retained-loop CPU includes iteration preparation, oracles
and sample storage; operation timing surrounds the eight public searches.

## Evidence and effect

The [Apple observation](../../evidence/p050-selector-allocation/apple-observation.json)
and [Windows preparation observation](../../evidence/p050-selector-allocation/windows-preparation-observation.json)
bind source/executable identities, raw producer outputs, metric scopes and
cleanup. Each target independently observes the same BGR conversion and mapping
copy counts; allocation totals remain target-specific.

| Workload | BGR conversions, before → after | Mapping copies, before → after | Apple Rust allocations, before → after | Windows Rust allocations, before → after |
| --- | ---: | ---: | ---: | ---: |
| `batch8-cold-full1080p` | 32 → 4 | 32 → 0 | 523 → 476 | 486 → 435 |
| `batch8-cold-roi320x180` | 32 → 4 | 32 → 32 | 523 → 544 | 486 → 499 |
| `batch8-live-owner-roi320x180` | 32 → 4 | 36 → 4 | 535 → 492 | 498 → 447 |
| `batch8-warm-full1080p` | 40 → 1 | 40 → 0 | 455 → 380 | 445 → 369 |

The released-owner cold ROI still copies 7,372,800 mapping bytes in both roles,
while BGR conversion falls from 5,529,600 to 691,200 bytes. Retaining the explicit
mapping owner reduces mapping bytes from 8,294,400 to 921,600. Full-frame current
rows perform no mapping copy. These are separate byte domains, not additive
estimates of total memory traffic or native acquisition cost.

All sixteen target/role rows have zero oracle failures and successful cleanup;
all final cache-held and active-evicted BGR gauges are zero. ROI peak counted
Rust heap is 688 bytes higher on Apple and 664 bytes higher on Windows. Apple
post-drop counted heap is 64 bytes above its pre-fixture value in both roles;
Windows returns to its pre-fixture value. These residuals are retained rather
than rewriting Rust heap, native payload or RSS as equivalent metrics.

## Trade-offs and disposition

Retain preparation reuse: the observations demonstrate fewer BGR conversions
with independent correct template results and bounded logical owner release.
The cold-ROI allocation increase remains an adverse observation, not a defect
attributed to one component by this whole-revision comparison. Weak mapping
reuse intentionally requires a live compatible mapping owner; cached BGR data
does not retain such an owner or a producer lease.

The next engineering investigation is the released-owner ROI mapping path and
its allocation/copy work. Do not add strong mapping retention, increase the
cache's two-entry/33,554,432-byte bound or change public ownership to make this
row's counters smaller. Any adjustment needs separate source-bound before/after
verification. No latency, CPU, actual-consumer or all-native leak claim follows
from these mechanism counts.

Actual getters report TBB/10 on Apple and ms-concurrency/20 on Windows before
and after each row. These are library endpoint settings, not active-worker
census or complete runtime-image closure. Windows Job CPU has unavailable
effective resolution; nominal units are not a precision guarantee. Private
Windows plans/configs are retained locally with explicit redacted projections;
transported projection hashes do not verify omitted original bytes. All raw
attempts remain attributable; no host-idle admission or numerical retry is used.

## Source reconciliation, 2026-10-26

The cold-ROI investigation reconciles the observed allocation deltas without
changing the cache design. This is source-level accounting against completed
rows, not sampled allocation stacks or proof of unique runtime causality.

Cold rows construct nine backends: one template compiler, four search matchers
and four replay-engine defaults. Each current backend adds one
`Arc<PreparedImages>`; four first conversions add four `Arc<CachedImage>`.
The `run()` version-probe backend precedes the allocation snapshot and is not
a tenth counted constructor. Warm-full constructs three backends and one
cached-image owner.

Both revisions already allocate a boxed pixel buffer and `Arc<CpuPixels>` for
each copied mapping. Removing 32 copies saves 64 counted allocations; cold ROI
removes none. Rust 1.97.1 uses lazy pinned pthread mutex storage on Apple,
initialized at first lock, versus inline futex storage on Windows. Accessed
cache locks add four/eight/eight/one Apple allocations respectively.

| Row | Mapping allocation delta | Backend-cache allocations | Cached-image owner allocations | Apple first-use mutex allocations | Reconciled Windows delta | Reconciled Apple delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Cold full | -64 | 9 | 4 | 4 | -51 | -47 |
| Cold ROI | 0 | 9 | 4 | 8 | +13 | +21 |
| Live-owner ROI | -64 | 9 | 4 | 8 | -51 | -43 |
| Warm full | -80 | 3 | 1 | 1 | -76 | -75 |

All eight deltas match the retained observations. The relevant paths are
`SearchFixture::{new,prepare_iteration}`, `replay_frame_with_descriptor`,
`replay_engine_inner`, `OpenCvBackend::new`, `PreparedRegion::new` and
`map_region`; the pinned standard-library paths are
`sys/sync/mutex/{mod,pthread,futex}.rs`. Independent Windows source/data review
agrees, without claiming a new Apple execution.

Retain the ownership and synchronization costs. Rewriting fixture setup,
retaining strong mappings or adding custom locks solely to lower these counts
would change the question or contract. The next selected observation uses the
normal allocator and preserves the separate selector/preparation comparisons;
instrumented allocation accounting does not predict its timing or CPU result.
