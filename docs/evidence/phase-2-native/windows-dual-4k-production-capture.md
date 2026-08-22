# Windows dual-4K production capture qualification

## Scope and source

This record qualifies the corrected Windows mixed-DPI dual-4K profile on
shared-oracle source `f50285a630b07dcf10a675a0e94d34a735aa163c`, tree
`4c2f23f851669932dee304e46d2c947721598549`.

The approved host ran Windows 11 Pro 25H2 build `26200.9168` on an Intel Core
i7-12700KF with 32 GiB RAM and an NVIDIA GeForce RTX 4080, driver
`32.0.15.9186`. Toolchains were Rust/Cargo 1.97.1, MSVC 19.44.35228, Windows SDK
10.0.26100.0, and OpenCV 4.14.0. The process was unelevated with an unsigned
repository fixture and no separate Windows capture permission.

Exactly two online non-mirrored 3840x2160 displays were present: primary 144 DPI
/ scale 1.5 at `[0,0,3840,2160)`, secondary 120 DPI / scale 1.25 at
`[-3840,0,0,2160)`. No topology substitution was used.

## Command and artifact identity

```text
native-phase2 --bench --workload-set production-capture-dual-4k \
  --target x86_64-pc-windows-msvc \
  --fixture-executable <absolute built fixture> \
  --hardware <approved host> --os-version <approved OS> \
  --deployment-target "Windows 11 25H2 build family 26200" \
  --source-revision f50285a630b07dcf10a675a0e94d34a735aa163c \
  --source-tree 4c2f23f851669932dee304e46d2c947721598549 \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified dual-4K topology> \
  --permissions-signing <approved classification>
```

- benchmark executable SHA-256: `9e1e0b4255f00c8567ad6714311e66f71f58c6c666a94db33ab3662e50d01010`;
- fixture executable SHA-256: `725dcced8b71c0f5d9f875559594f17005495c5873d4c6a6efbf709056e7f073`.

## Review defects and rejected attempts

The original `121d41a` profile remains historical evidence but cannot close
release acceptance: it used independently published aggregate callback fields,
could credit one session's callback to both acquired frames, and omitted the
required 300-frame moving-seam row.

Repair attempts were retained as rejected setup/evidence rather than hidden:

1. a DPI-virtualized `GetWindowRect` result rejected the first exact-placement
   attempt;
2. callback reservations begun before a sample baseline exposed missing
   post-baseline correlation;
3. the first no-warm-up moving row exposed one 66,355,200-byte steady mapping
   allocation and failed the growth gate; and
4. a reservation-ordered ring intermittently excluded callbacks that completed
   after the baseline and was rejected.

The repairs select a queue floor before each baseline, publish the complete
callback record in frame-publication order, bind it to stream/epoch/sequence
before the frame becomes observable, and prime the steady mapped-frame
allocation outside retained sampling.

Round-two review then proved that the `90a8bab` and `c6ff39a` movement oracle
could accept a strictly newer frame containing the immediately prior uniform
fixture placement. Those runs remain truthful evidence for their older oracle,
but not for corrected movement latency or release acceptance.

Corrected source `7c31752` added fixed third-color markers on both display
halves. Two unchanged-source precursor runs passed and established the
125/175/225 ms ceilings. Budget-enforced source `fdcac29` passed separately.
Fresh review then required the synthetic prior-placement regression to call the
same marker-color predicate as production mapped-frame sampling. Shared-predicate
source `f50285a` reran and passed the final profile below.

## Accepted final results

The stationary pair retained 600 samples per display after 20 shared warm-ups.
The movement workload retained exactly 300 samples and no warm-up samples.
Every row reported zero correctness failures.

| Workload | p50 | p95 | maximum | mapped/copy bytes | detached/staging/total | stale ratio | growth | resident peak |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `dual_display_frame_arrival` | 19.8419 ms | 40.2103 ms | 42.0395 ms | 66,355,200 / 199,065,600 | 7 / 1 / 12 | 0.455782313 | 0 B | 219,213,824 B |
| `dual_display_callback_copy` | 0.05905 ms | 0.08885 ms | 0.16980 ms | 66,355,200 / 199,065,600 | 7 / 1 / 12 | 0.455782313 | 0 B | 219,213,824 B |
| `dual_display_moving_seam` | 40.9684 ms | 50.4713 ms | 71.2279 ms | 66,355,200 / 199,065,600 | 6 / 1 / 11 | 0.474145486 | 320 B | 288,911,360 B |

Live Rust heap peaks were 99,582,727 bytes for the stationary pair and
99,576,732 bytes for movement. All values satisfy ADR 0032.

## Moving-seam and callback oracle

The movement schedule advances the 1280x720 fixture in deterministic 16-pixel
steps between physical X `-960` and `-320`, reversing at each bound while always
straddling X `0`. A per-monitor-v2 DPI context makes `SetWindowPos` and
`GetWindowRect` physical and exact. A static 16x16 third-color marker remains
near each fixture edge, one on each display. The requested marker centers move
outside both prior marker rectangles at every step, so a strictly newer frame
containing the prior placement cannot pass. Both independently captured frames
must contain their requested-position marker.

Each display establishes its own callback baseline after observing its current
queue floor. The acquired frame must be strictly newer and must find one
coherently published callback record with the same stream, epoch, and frame
sequence. Elapsed duration and copied bytes come from those two records; no
process-wide callback can satisfy the other session. Both frames are mapped once
in the same retained system interaction and under one absolute deadline.

## Applicability, cleanup, and privacy

The previously accepted ordinary `WindowMessage` mixed-DPI input matrix remains
revision-bound to its executed source. The complete intervening diff changes
benchmark-only instrumentation, movement, resource enforcement, and an opt-in
availability qualification feature; it changes no production input route or
public-language contract.

All benchmark and fixture processes reached terminal states. No retained record
contains captured pixels or hashes, recognized or input text, credentials, user
paths, PIDs, raw HWND/display identifiers, unrelated window titles, process
inventories, or unrelated desktop metadata. Physical device removal, TDR, and
driver upgrade were not performed and are not claimed.
