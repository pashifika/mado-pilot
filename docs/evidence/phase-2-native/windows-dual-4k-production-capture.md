# Windows dual-4K production capture qualification

## Scope and source

This record qualifies the corrected Windows mixed-DPI dual-4K profile on clean
source `fdcac294f602c172bdcebd44efddef2a7b858d18`, tree
`c906aa870ccdd67d3608f979320bfcddb7b8259d`.

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
  --source-revision fdcac294f602c172bdcebd44efddef2a7b858d18 \
  --source-tree c906aa870ccdd67d3608f979320bfcddb7b8259d \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified dual-4K topology> \
  --permissions-signing <approved classification>
```

- benchmark executable SHA-256: `2e96b490c463a7a4b2eb010ad1d51e2af9c7fec33b441d72c6d26e8e82eefc2f`;
- fixture executable SHA-256: `93f60dde8c469e188a467b6f122d91d8295c07de11c000a8f7cb26626a1f2ec0`.

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
halves and a deterministic test that rejects every immediately prior placement
across the 300-step schedule. Two unchanged-source precursor runs passed and
measured moving p50/p95/maximum values of `41.0535/45.6627/71.4985 ms` and
`40.9366/51.1564/71.1795 ms`. ADR 0032 applied its three-times/readable-rounding
policy to derive 125/175/225 ms. Final source `fdcac29` then passed the separate
budget-enforced profile below.

## Accepted final results

The stationary pair retained 600 samples per display after 20 shared warm-ups.
The movement workload retained exactly 300 samples and no warm-up samples.
Every row reported zero correctness failures.

| Workload | p50 | p95 | maximum | mapped/copy bytes | detached/staging/total | stale ratio | growth | resident peak |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `dual_display_frame_arrival` | 19.6206 ms | 38.9474 ms | 43.1449 ms | 66,355,200 / 199,065,600 | 8 / 1 / 13 | 0.456767768 | 0 B | 219,111,424 B |
| `dual_display_callback_copy` | 0.05765 ms | 0.08940 ms | 0.33250 ms | 66,355,200 / 199,065,600 | 8 / 1 / 13 | 0.456767768 | 0 B | 219,111,424 B |
| `dual_display_moving_seam` | 40.9370 ms | 51.8592 ms | 71.9731 ms | 66,355,200 / 199,065,600 | 7 / 1 / 12 | 0.489795918 | -232 B | 321,548,288 B |

Live Rust heap peaks were 99,583,805 bytes for the stationary pair and
99,577,812 bytes for movement. All values satisfy ADR 0032.

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
