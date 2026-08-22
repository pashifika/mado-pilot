# macOS production-capture acceptance

## Scope and source

This record accepts only the repository fixture's production capture and transition workloads on the approved Apple Silicon host. It is bound to:

| Fact | Value |
|---|---|
| Commit | `d182300cd8710891ded6cba17184c44d6d58a114` |
| Tree | `c570343d334a5c77415e6a885ef8821c731b0ad5` |
| Benchmark executable SHA-256 | `754c8234085ab1855630923326402d7a071e23c0174e8148f1d8ce0ed8e24af7` |
| Fixture source SHA-256 | `c5576c5290003c723f1d3797ab1c6032e935a9e04ab42d50ce5dc9108bc029ea` |
| Fixture executable SHA-256 | `40a33a5ceefff9ff01afef52b694799d2a1af91cc355a9d1020dda2a68891374` |
| Host | Apple M1 Pro, 10 cores, 32 GiB |
| OS / SDK | macOS 26.5.2 (`25F84`) / SDK 26.5 |
| Deployment target | macOS 26.5.2 |
| Displays | exactly two online non-mirrored displays; signed-origin 3840x2160 1x; main 2560x1440 logical / 5120x2880 backing 2x |
| Permissions / signing | Screen Recording and event-post access granted; fixture structurally ad-hoc signed |

No result transfers to another application, renderer, game, display topology, operating-system version, input stack, or anti-cheat system.

## Production transitions

Five warm-ups and fifty retained samples per workload:

| Workload | Correctness failures | p50 | p95 | max | Mapped bytes | Peak Rust heap | Growth |
|---|---:|---:|---:|---:|---:|---:|---:|
| `open_first_frame` | 0 | 89.774583 ms | 97.587625 ms | 109.289916 ms | 4,628,480 B | 4,635,745 B | 0 B |
| `resize_recreation` | 0 | 52.034250 ms | 73.230708 ms | 99.592917 ms | 0 B | 18,879 B | 0 B |
| `close_drain` | 0 | 63.066125 ms | 67.622291 ms | 84.241667 ms | 0 B | 7,041 B | 0 B |

The private resize acknowledgement is stimulus only. Acceptance requires the independently captured frame to advance epoch and geometry revision and to carry the fixture's exact next frame-authoritative target geometry; an arbitrary changed extent is rejected. The earlier 9,856-byte result was traced to geometry-history capacity growth; after the bounded conditional reservation repair this exact row reports zero growth.

## Production capture

Twenty warm-ups and two hundred retained samples per workload:

| Workload | Correctness failures | p50 | p95 | max | Mapped bytes | Stale ratio | Peak Rust heap | Growth |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `publication_age` | 0 | 0.689375 ms | 0.942709 ms | 1.311000 ms | 4,628,480 B | 0 | 9,300,304 B | 0 B |
| `steady_frame_acquisition` | 0 | 17.708916 ms | 38.072041 ms | 66.819584 ms | 4,628,480 B | 0 | 9,300,192 B | 0 B |
| `latest_acquisition` | 0 | 0.000667 ms | 0.000833 ms | 0.001000 ms | 4,628,480 B | 0 | 9,300,192 B | 0 B |
| `cpu_map_bgra8` | 0 | 0.187583 ms | 0.235750 ms | 0.456000 ms | 4,628,480 B | n/a | 9,300,192 B | 0 B |
| `retained_pressure_resume` | 0 | 3.725083 ms | 7.293875 ms | 18.786375 ms | 0 B | 0.833887043 | 4,671,392 B | 0 B |

The retained-pressure stale ratio is the expected consequence of filling the finite retained budget, proving one publication cannot commit, releasing one slot, and observing resumed publication with a sequence gap. It is not a steady-capture loss rate.

The final source creates the retained-pressure fixture once in that workload's
untimed setup. An intermediate post-budget run started it before four unrelated
long workloads, left it idle for about three minutes, and later failed the
confirmation-session close; that run emitted no profile and contributes no
measurement. Scoping the fixture to its workload preserves one fixture across
all pressure samples while avoiding overlapping unrelated fixture lifetimes.

The pre-landing enforcement repair produced three exact-source capture runs.
The first was rejected because one `cpu_map_bgra8` sample exceeded the unchanged
10 ms hard maximum (`13.930042 ms`) while its p95 remained `0.250125 ms`.
Without changing source, fixture, executable, topology, or budgets, two
subsequent runs passed every ceiling; their mapping maxima were `0.462750 ms`
and `0.456000 ms`. The final passing run above is the retained profile. The
rejected result remains recorded here and is not excluded from the review
history.

## Procedure

After building, assembling, signing, and strictly verifying the repository fixture as documented in [`macos-input-verification.md`](../../macos-input-verification.md), run each workload set with the exact source and approved metadata:

```sh
cargo bench --locked -p mado-pilot --bench native-phase2 -- \
  --workload-set production-transitions \
  --fixture-executable "$MADO_PILOT_MACOS_FIXTURE_EXECUTABLE" \
  --source-revision d182300cd8710891ded6cba17184c44d6d58a114 \
  --source-tree c570343d334a5c77415e6a885ef8821c731b0ad5 \
  --toolchain "rustc 1.97.1; cargo 1.97.1; Apple clang 21.0.0; macOS SDK 26.5" \
  --gpu-driver "Apple integrated GPU; system driver stack" \
  --hardware "Apple M1 Pro, 10 cores, 32 GiB" \
  --os-version "macOS 26.5.2 (25F84)" \
  --deployment-target "macOS 26.5.2" \
  --display-topology "exactly two online non-mirrored displays; signed-origin 3840x2160 1x; main 2560x1440 logical / 5120x2880 backing 2x; mixed-scale connection method unrestricted" \
  --permissions-signing "Screen Recording granted; event-post access granted; target bundle structurally ad-hoc signed"

# Repeat with --workload-set production-capture and identical metadata.
```

The tracked profiles retain the complete output and ADR 0030 budgets. Profile-drift tests bind their benchmark keys, hard predicates, latency arrays, live-heap ceilings, and mapped-byte ceilings to the executable harness.

## Privacy review

This record contains approved source and executable digests, aggregate timings, correctness/resource counts, permission/signing classifications, and non-sensitive host/topology metadata. It contains no captured pixels or hashes, recognized or injected text, credentials, PIDs, window titles, raw display/window identifiers, user paths, or unrelated desktop data.
