# Windows production capture qualification

## Scope and source

This record qualifies the 1280x720 production-capture and production-transition profiles on final source commit `0208798d9542aaae3a956d3e774c9ce57468bc9d`, tree `cac0020edbf5b3d28a4dcd5df41e020dc0c6257d`. It does not qualify the separately scheduled dual-4K profile.

The approved host ran Windows 11 Pro 25H2 build `26200.9168` on an Intel Core i7-12700KF with 32 GiB RAM and an NVIDIA GeForce RTX 4080, driver `32.0.15.9186`. The run used Rust/Cargo 1.97.1, MSVC 19.44.35228, Windows SDK 10.0.26100.0, and OpenCV 4.14.0. It was unelevated on an interactive desktop with one online, non-mirrored primary display: 3840x2160 physical, 2560x1440 logical, 144 DPI, signed physical rectangle `[0,0,3840,2160)`.

The final release benchmark executable SHA-256 was `63dc6186337cfd84d4430376afb0a1773f47f912ebfa105987439b8644dd6aa1`. The release fixture executable SHA-256 was `07151c066fc139bde56046cd1812b1d553243a27292569e86b33bd97003b226c`; its tracked source digest was `e2daf522336997f841bd8813c62371e001c7ef96dfe3e3ae44fafaa35a6d67eb`.

## Procedure and process observation

The release benchmark and fixture were built from the bound source, then the benchmark executable was launched directly with an argument vector for each workload set:

```text
native-phase2 --bench --workload-set production-capture-1280x720 \
  --fixture-executable <release fixture> \
  --hardware <approved host> --os-version <approved build> \
  --deployment-target "Windows 11 25H2 build family 26200" \
  --source-revision 0208798d9542aaae3a956d3e774c9ce57468bc9d \
  --source-tree cac0020edbf5b3d28a4dcd5df41e020dc0c6257d \
  --toolchain <recorded versions> --gpu-driver <recorded driver> \
  --display-topology <qualified single display> \
  --permissions-signing <unelevated and unsigned fixture>

native-phase2 --bench --workload-set production-transitions-1280x720 <same metadata>
```

Each set ran twice on the reviewed precursor and once on final source. A supervisor retained stdout as a profile candidate and streamed stderr live. The records identified each workload's setup, warm-up, sampling, and completion phases plus every fixture's transient PID, behavior, readiness, stopping, and exit. PIDs were observed only for process control and were not retained. All six benchmark processes exited `0`; every spawned fixture reached a terminal state; post-run process queries found no remaining fixture. The final two runs enforced every ADR 0031 budget in-process.

A PTY rendering that duplicated wrapped characters and a shell-redirection attempt that split quoted metadata were rejected before budget selection. Neither contributed measurement values. Final source and both accepted precursors used a direct argument vector and produced intact parseable metadata.

## Capture results

Each accepted capture run used 30 warm-ups and 150 samples for each of four workloads: 120 aggregate warm-ups and 600 retained frames. All three runs reported zero correctness failures, zero allocation growth, exact 3,686,400-byte mappings, and zero stale work where stale work was applicable.

The table records the largest observation across final source and both accepted precursors:

| Workload | largest p50 | largest p95 | largest maximum | largest live Rust heap |
|---|---:|---:|---:|---:|
| `steady_frame_acquisition` | 19.9035 ms | 44.4918 ms | 59.3198 ms | 7,416,613 bytes |
| `callback_copy` | 0.1136 ms | 0.4144 ms | 1.0699 ms | 7,410,618 bytes |
| `latest_acquisition` | 0.0013 ms | 0.0038 ms | 0.0188 ms | 7,410,570 bytes |
| `cpu_map_bgra8` | 1.8841 ms | 3.8514 ms | 6.4503 ms | 7,410,570 bytes |

All three callback-copy runs reported exactly 3,686,400 copied bytes per retained result, two detached textures, one staging texture, five total producer/detached/staging textures, and a largest resident peak of 66,203,648 bytes.

## Transition results

All three transition runs reported zero correctness failures. All allocation growth was at or below 48 bytes; resize released 979,200 bytes relative to its post-warm-up baseline.

The table records the largest observation across final source and both accepted precursors:

| Workload | largest p50 | largest p95 | largest maximum | largest live Rust heap |
|---|---:|---:|---:|---:|
| `open_first_frame` | 105.3187 ms | 105.7658 ms | 105.7658 ms | 3,730,605 bytes |
| `retained_pressure_resume` | 27.2020 ms | 27.2020 ms | 27.2020 ms | 3,724,466 bytes |
| `resize_recreation` | 80.8781 ms | 103.8856 ms | 103.8856 ms | 8,389,858 bytes |
| `target_loss_recovery` | 368.5875 ms | 368.5875 ms | 368.5875 ms | 3,724,514 bytes |
| `close_drain` | 2.3540 ms | 2.5039 ms | 2.5039 ms | 37,970 bytes |

The largest native process resident peak was 72,896,512 bytes. Mapped-byte observations were exact: 3,686,400 for open, zero for pressure and close, 4,665,600 for the 1440x810 resize result, and 7,372,800 for retained-old plus replacement mappings during target-loss recovery. Retained pressure reported a deliberate stale ratio of 0.8 after filling finite storage.

## Privacy and remaining gap

No retained record contains captured pixels or hashes, recognized or input text, credentials, user paths, PIDs, raw window/display identifiers, unrelated window titles, process inventories, or unrelated desktop metadata. Executable paths and rejected raw output remain untracked.

ADR 0031 accepts only the two 1280x720 profiles. The mixed-DPI dual-4K profile remains open under `G-013`; it must run outside 08:00-19:00 local time with both qualified displays online before any dual-4K ceiling is accepted.
