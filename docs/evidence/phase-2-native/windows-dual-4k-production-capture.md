# Windows dual-4K production capture qualification

## Scope and source

This record qualifies the Windows mixed-DPI dual-4K production profile on final benchmark source `121d41a9eea341b7345a8b0dda4918b1f61ec74e`, tree `7e694a070d1e300642033b56aef499b8238c08ca`.

The approved host ran Windows 11 Pro 25H2 build `26200.9168` on an Intel Core i7-12700KF with 32 GiB RAM and an NVIDIA GeForce RTX 4080, driver `32.0.15.9186`. Toolchains were Rust/Cargo 1.97.1, MSVC 19.44.35228, Windows SDK 10.0.26100.0, and OpenCV 4.14.0. The process was unelevated on an interactive desktop with an unsigned repository fixture and no separate Windows capture permission.

The exact topology was:

- primary: online, non-mirrored, 3840x2160, 144 DPI, effective scale 1.5, physical rectangle `[0,0,3840,2160)`;
- secondary: online, non-mirrored, 3840x2160, 120 DPI, effective scale 1.25, physical rectangle `[-3840,0,0,2160)`;
- one shared signed-origin seam at `x=0`.

Holiday availability explicitly waived the normal shared-display workday exclusion for this run; it did not waive a benchmark, topology, correctness, source, or privacy condition.

The final benchmark executable SHA-256 was `9726a0dbf4da45e42543f7a8190cb0f9db73817a1995e08a5783c19548a838eb`. The final fixture executable SHA-256 was `484136b949d5ecaf6d325c4a9a71f8780ed876d37bff953ac0e0a3ae683f53fe`; the tracked fixture source digest was `e2daf522336997f841bd8813c62371e001c7ef96dfe3e3ae44fafaa35a6d67eb`.

## Procedure and process observation

The clean detached worktree built the release benchmark and fixture, then invoked the benchmark with a direct argument vector:

```text
cargo build --locked --release --package mado-pilot --bench native-phase2
cargo build --locked --release --package mado-pilot-platform-windows \
  --bin mado-pilot-windows-input-fixture

native-phase2 --bench --workload-set production-capture-dual-4k \
  --fixture-executable <release fixture> \
  --hardware <approved host> --os-version <approved build> \
  --deployment-target "Windows 11 25H2 build family 26200" \
  --source-revision 121d41a9eea341b7345a8b0dda4918b1f61ec74e \
  --source-tree 7e694a070d1e300642033b56aef499b8238c08ca \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified dual-4K topology> \
  --permissions-signing <unelevated and unsigned fixture>
```

A supervisor wrote stdout to a profile candidate and streamed stderr live. It observed setup, warm-up, sampling, completion, fixture readiness, and fixture termination. The final benchmark exited `0` after enforcing every ADR 0032 gate. No fixture process remained after the run. Transient PIDs were used only for supervision and were not retained.

The final workload performed 20 warm-ups followed by one shared 600-iteration pass. Every retained iteration acquired and mapped one strictly newer frame from each display. Arrival and callback-copy series therefore each contain 600 samples per display without duplicating capture or mapping work.

## Final-source results

| Workload | p50 | p95 | maximum | iteration span |
|---|---:|---:|---:|---:|
| `dual_display_frame_arrival` | 2.0331 ms | 27.0484 ms | 49.1074 ms | 31.111575 ms |
| `dual_display_callback_copy` | 0.0607 ms | 0.1182 ms | 0.2262 ms | 31.111575 ms |

Both timing views reported the same exact result and resource facts:

- correctness failures: `0`;
- mapped bytes per retained sample: `66,355,200`;
- callback-copy bytes per retained sample: `132,710,400`;
- detached/staging/total GPU-resource peaks: `8 / 1 / 13`;
- stale-work ratio: `0.206349206`;
- live Rust heap peak: `99,581,711 bytes`;
- steady live Rust heap: `66,403,175 bytes`;
- post-warm-up allocation growth: `-392 bytes`;
- native process resident high-water mark: `285,569,024 bytes`.

Two reviewed precursor runs on source `0208798d9542aaae3a956d3e774c9ce57468bc9d` also retained 600 samples per display with zero correctness failures and zero allocation growth. Their largest observations were 28.1022 ms arrival p95, 48.2191 ms arrival maximum, 0.100875 ms callback-copy p95, 0.2856 ms callback-copy maximum, five producer-surface copies, eight detached textures, thirteen total textures, 0.191919192 stale ratio, 99,581,689 bytes live Rust heap, and 285,605,888 bytes resident high-water memory. These values and the final run remain below ADR 0032.

## Mixed-DPI movement and input applicability

The final affected product source before budget-only changes, `0208798`, reran the ordinary `WindowMessage` native matrix on the same physical topology. It reported `monitors=2`, DPI `[144x144,120x120]`, executed `mixed-dpi-multi-display`, moved repository fixtures to `(48,48 480x320)` and `(-3792,48 512x344)`, established capture-backed correlated pointer/wheel stimuli on both displays, and passed drag, the 86-event vocabulary, queue-full, partial-prefix, hung-target, deadline cleanup, cancellation cleanup, foreground, cursor, geometry, close, and cleanup checks.

The complete intervening diff from `0208798` to final profile source `121d41a` adds the accepted 1280x720 profiles and documentation, profile-drift tests, and dual-4K benchmark budget constants/enforcement. It changes no Windows discovery, placement, capture, input, fixture, or public-language behavior, so the mixed-DPI input result remains applicable to final source without relabeling its executed revision.

Injected device loss remains state-machine evidence. Physical device removal, TDR, and driver upgrade were not performed and are not claimed.

## Privacy

No retained record contains captured pixels or hashes, recognized or input text, credentials, user paths, transient PIDs, raw HWND/display identifiers, unrelated window titles, process inventories, or unrelated desktop metadata. Executable paths and raw console output remain untracked ephemera.
