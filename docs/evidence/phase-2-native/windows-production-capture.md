# Windows production capture qualification

## Scope and source

The current 1280x720 production-capture profile is requalified on clean source
`9bfc0c023db4d39e7caa59aa38b196477b971e3a`, tree
`be1c57127d495f1345a6619f1851acde627430f0`. The production-transition profile
remains bound to its accepted source `0208798d9542aaae3a956d3e774c9ce57468bc9d`,
tree `cac0020edbf5b3d28a4dcd5df41e020dc0c6257d`; the complete intervening diff
changes no transition workload, oracle, fixture mode, or accepted limit.

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
  --source-revision 9bfc0c023db4d39e7caa59aa38b196477b971e3a \
  --source-tree be1c57127d495f1345a6619f1851acde627430f0 \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified single-display topology> \
  --permissions-signing <approved classification>
```

- benchmark executable SHA-256: `0a82933f17fe9e37418604636829eb751a43a558d715b1234c85db9e93aea40c`;
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
| `steady_frame_acquisition` | 19.6759 ms | 40.3248 ms | 41.4159 ms | 3,686,400 | n/a | 7,417,895 B |
| `callback_copy` | 0.0958 ms | 0.3510 ms | 0.4569 ms | 3,686,400 | 3,686,400 | 7,411,900 B |
| `latest_acquisition` | 0.0011 ms | 0.0033 ms | 0.0558 ms | 3,686,400 | n/a | 7,411,852 B |
| `cpu_map_bgra8` | 1.4218 ms | 2.7636 ms | 5.3529 ms | 3,686,400 | n/a | 7,411,852 B |

`callback_copy` observed two detached textures, one staging texture, five total
producer/detached/staging resources, zero stale work, and a 66,306,048-byte
resident peak. These satisfy ADR 0031 without treating the resource counts as
exact equality requirements: each count must be present, nonzero, and no greater
than its accepted ceiling. Copied and mapped bytes remain exact.

## Repair applicability and lifecycle

Each callback observation now publishes count-equivalent identity, elapsed time,
and copied bytes as one frame-stamp-bound record. A record is visible before its
frame can be acquired; contention or overwrite invalidates the profile. The
1280 callback row requires the exact acquired stream/epoch/sequence record after
its own baseline.

The benchmark and fixture reported setup, warm-up, sampling, completion,
readiness, stopping, and terminal exit outside measured regions. No fixture
process remained after the run. The separately accepted transition profile is
unchanged and retains its original measurements and source identity.

## Privacy and exclusions

No retained record contains captured pixels or hashes, recognized or input text,
credentials, user paths, PIDs, raw HWND/display identifiers, unrelated window
titles, process inventories, or unrelated desktop metadata. Executable paths and
raw console output remain untracked. Physical device removal, TDR, and driver
upgrade were not performed and are not claimed.
