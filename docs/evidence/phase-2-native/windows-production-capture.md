# Windows production capture qualification

## Scope and source

The current 1280x720 production-capture profile is requalified on clean source
`c6ff39a9461c128d9a53e4896a34cb65e3c419a3`, tree
`8f2766a9b55c9964f57a096a720ec4a404ad3756`. The production-transition profile
was rerun on repaired source `7c31752bc632a26c4ba61faa0567ac78e2218ea0`,
tree `4e99487e184b3edfcbd62e31299599d2fbe13c7d`, after independent review
invalidated its earlier applicability decision.

The approved host ran Windows 11 Pro 25H2 build `26200.9168` on an Intel Core
i7-12700KF with 32 GiB RAM and an NVIDIA GeForce RTX 4080, driver
`32.0.15.9186`. Toolchains were Rust/Cargo 1.97.1, MSVC 19.44.35228, Windows SDK
10.0.26100.0, and OpenCV 4.14.0. The capture rerun used an unelevated
interactive desktop with one online non-mirrored primary display: 3840x2160
physical, 2560x1440 logical, 144 DPI, physical rectangle `[0,0,3840,2160)`.

## Requalification command and identity

```text
native-phase2 --bench --workload-set production-capture-1280x720 \
  --target x86_64-pc-windows-msvc \
  --fixture-executable <absolute built fixture> \
  --hardware <approved host> --os-version <approved OS> \
  --deployment-target "Windows 11 25H2 build family 26200" \
  --source-revision c6ff39a9461c128d9a53e4896a34cb65e3c419a3 \
  --source-tree 8f2766a9b55c9964f57a096a720ec4a404ad3756 \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified single-display topology> \
  --permissions-signing <approved classification>
```

- benchmark executable SHA-256: `0212181ef0c6ff8c2e0fbdb72ad0cd23f7597a9ca29311256a94a58405b58182`;
- fixture executable SHA-256: `7a0eacf152ea77f30f791d82e58e90424f8fe75457225bbe246df13a6554c7ed`.

The first repaired capture run passed on an earlier repair commit, then was
invalidated when dual-display correlation and budget enforcement changed the
benchmark source. The retained run above is the later exact-source run. Earlier
argument/setup failures emitted no measurement and were rejected.

## Accepted capture results

The run retained 150 samples for each of four workloads after 30 warm-ups per
workload. Every workload reported zero correctness failures and zero allocation
growth.

| Workload | p50 | p95 | maximum | mapped bytes | copied bytes | live heap peak |
|---|---:|---:|---:|---:|---:|---:|
| `steady_frame_acquisition` | 19.9394 ms | 40.7947 ms | 43.2769 ms | 3,686,400 | n/a | 7,417,893 B |
| `callback_copy` | 0.0941 ms | 0.3571 ms | 0.5691 ms | 3,686,400 | 3,686,400 | 7,411,898 B |
| `latest_acquisition` | 0.0010 ms | 0.0036 ms | 0.0253 ms | 3,686,400 | n/a | 7,411,852 B |
| `cpu_map_bgra8` | 1.3888 ms | 2.4473 ms | 4.9018 ms | 3,686,400 | n/a | 7,411,850 B |

`callback_copy` observed two detached textures, one staging texture, five total
producer/detached/staging resources, zero stale work, and a 66,310,144-byte
resident peak. These satisfy ADR 0031 without treating the resource counts as
exact equality requirements: each count must be present, nonzero, and no greater
than its accepted ceiling. Copied and mapped bytes remain exact.

## Accepted transition results

The exact-source transition rerun passed every unchanged ADR 0031 budget:

| Workload | p50 | p95 | maximum | mapped bytes | growth | live heap peak |
|---|---:|---:|---:|---:|---:|---:|
| `open_first_frame` | 104.4091 ms | 110.7733 ms | 110.7733 ms | 3,686,400 | 0 B | 3,732,539 B |
| `retained_pressure_resume` | 4.6247 ms | 4.6247 ms | 4.6247 ms | 0 | 0 B | 3,726,400 B |
| `resize_recreation` | 80.4254 ms | 103.6176 ms | 103.6176 ms | 4,665,600 | -979,200 B | 8,391,792 B |
| `target_loss_recovery` | 368.5183 ms | 368.5183 ms | 368.5183 ms | 7,372,800 | 48 B | 3,726,448 B |
| `close_drain` | 2.3781 ms | 2.4896 ms | 2.4896 ms | 0 | 0 B | 39,904 B |

The largest resident observation was 71,221,248 bytes. The run retained zero
correctness failures; every fixture process reached a terminal state.

## Repair applicability and lifecycle

Each callback observation now publishes count-equivalent identity, elapsed time,
and copied bytes as one frame-stamp-bound record. A record is visible before its
frame can be acquired; contention or overwrite invalidates the profile. The
1280 callback row requires the exact acquired stream/epoch/sequence record after
its own baseline.

The benchmark and fixture reported setup, warm-up, sampling, completion,
readiness, stopping, and terminal exit outside measured regions. No fixture
process remained after either retained run. The transition result now carries
its repaired source and executable identities rather than an applicability
claim.

## Privacy and exclusions

No retained record contains captured pixels or hashes, recognized or input text,
credentials, user paths, PIDs, raw HWND/display identifiers, unrelated window
titles, process inventories, or unrelated desktop metadata. Executable paths and
raw console output remain untracked. Physical device removal, TDR, and driver
upgrade were not performed and are not claimed.
