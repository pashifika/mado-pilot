# Windows production capture qualification

## Scope and source

The current 1280x720 production-capture profile is requalified on shared-marker
source `f50285a630b07dcf10a675a0e94d34a735aa163c`, tree
`4c2f23f851669932dee304e46d2c947721598549`. The production-transition profile
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
  --source-revision f50285a630b07dcf10a675a0e94d34a735aa163c \
  --source-tree 4c2f23f851669932dee304e46d2c947721598549 \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified single-display topology> \
  --permissions-signing <approved classification>
```

- benchmark executable SHA-256: `9e1e0b4255f00c8567ad6714311e66f71f58c6c666a94db33ab3662e50d01010`;
- fixture executable SHA-256: `725dcced8b71c0f5d9f875559594f17005495c5873d4c6a6efbf709056e7f073`.

Round-one source `c6ff39a` passed after callback/resource repair. Fresh review
then found that shared production marker painting changed this fixture mode.
The retained `f50285a` run above requalifies that exact operation and artifact;
earlier argument/setup failures emitted no measurement and remain rejected.

## Accepted capture results

The run retained 150 samples for each of four workloads after 30 warm-ups per
workload. Every workload reported zero correctness failures and zero allocation
growth.

| Workload | p50 | p95 | maximum | mapped bytes | copied bytes | live heap peak |
|---|---:|---:|---:|---:|---:|---:|
| `steady_frame_acquisition` | 19.8553 ms | 40.7298 ms | 41.6363 ms | 3,686,400 | n/a | 7,417,467 B |
| `callback_copy` | 0.0827 ms | 0.2242 ms | 0.8707 ms | 3,686,400 | 3,686,400 | 7,411,472 B |
| `latest_acquisition` | 0.0011 ms | 0.0036 ms | 0.0044 ms | 3,686,400 | n/a | 7,411,424 B |
| `cpu_map_bgra8` | 1.3605 ms | 2.8779 ms | 4.6277 ms | 3,686,400 | n/a | 7,411,424 B |

`callback_copy` observed two detached textures, one staging texture, five total
producer/detached/staging resources, zero stale work, and a 66,506,752-byte
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
