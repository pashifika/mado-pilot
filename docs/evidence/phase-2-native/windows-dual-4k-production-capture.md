# Windows dual-4K production capture qualification

## Scope and source

This record qualifies the repaired Windows mixed-DPI dual-4K profile on clean
source `c6ff39a9461c128d9a53e4896a34cb65e3c419a3`, tree
`8f2766a9b55c9964f57a096a720ec4a404ad3756`.

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
  --source-revision c6ff39a9461c128d9a53e4896a34cb65e3c419a3 \
  --source-tree 8f2766a9b55c9964f57a096a720ec4a404ad3756 \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified dual-4K topology> \
  --permissions-signing <approved classification>
```

- benchmark executable SHA-256: `0212181ef0c6ff8c2e0fbdb72ad0cd23f7597a9ca29311256a94a58405b58182`;
- fixture executable SHA-256: `7a0eacf152ea77f30f791d82e58e90424f8fe75457225bbe246df13a6554c7ed`.

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

Two unchanged-source precursor runs at `90a8bab` then passed the complete
stationary and moving matrix. They supplied the moving latency derivation. Final
source `c6ff39a` added those executable gates and passed again with every budget
enforced.

## Accepted final results

The stationary pair retained 600 samples per display after 20 shared warm-ups.
The movement workload retained exactly 300 samples and no warm-up samples.
Every row reported zero correctness failures.

| Workload | p50 | p95 | maximum | mapped/copy bytes | detached/staging/total | stale ratio | growth | resident peak |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `dual_display_frame_arrival` | 19.6059 ms | 39.8737 ms | 42.7787 ms | 66,355,200 / 165,888,000 | 8 / 1 / 13 | 0.477579451 | 392 B | 219,348,992 B |
| `dual_display_callback_copy` | 0.05805 ms | 0.11865 ms | 0.35645 ms | 66,355,200 / 165,888,000 | 8 / 1 / 13 | 0.477579451 | 392 B | 219,348,992 B |
| `dual_display_moving_seam` | 41.0642 ms | 43.4504 ms | 63.5555 ms | 66,355,200 / 165,888,000 | 7 / 1 / 12 | 0.485861183 | 160 B | 321,396,736 B |

Live Rust heap peaks were 99,583,153 bytes for the stationary pair and
99,577,160 bytes for movement. All values satisfy ADR 0032.

## Moving-seam and callback oracle

The movement schedule advances the 1280x720 fixture in deterministic 16-pixel
steps between physical X `-960` and `-320`, reversing at each bound while always
straddling X `0`. A per-monitor-v2 DPI context makes `SetWindowPos` and
`GetWindowRect` physical and exact. For every retained sample, one declared
content point is inside the negative-X display half and one is inside the
positive-X half.

Each display establishes its own callback baseline after observing its current
queue floor. The acquired frame must be strictly newer and must find one
coherently published callback record with the same stream, epoch, and frame
sequence. Elapsed duration and copied bytes come from those two records; no
process-wide callback can satisfy the other session. Both frames are mapped once
in the same retained system interaction.

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
