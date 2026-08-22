# G-002 experiment plan

This plan was frozen before the first prototype build or measurement. If the
workload, oracle, candidate matrix, or hard gate changes after a result is
observed, the affected rows must be rerun and the evidence must name the old and
new plan revisions.

## Post-review amendment

The first complete result was rejected during review because its asynchronous
handlers captured the probe owner directly, `drain_ms` did not measure complete
close/reset, and coherent markers did not prove sequence freshness. The plan
was amended before the accepted rerun to require:

- lifetime-independent shared state for both WinRT handlers and one
  synchronized owner-admission/drain/fence protocol;
- a deterministic mode that pauses a queued delegate before admission, closes
  the owner, and proves the delegate is rejected after the fence;
- separate complete `close_ms` and admission-stop-to-new-`StartCapture`
  `reset_ms` measurements;
- wrap-aware 16-bit sequence progress with duplicates allowed, regressions
  rejected, and no more than 500 ms without progress;
- a 45-second external process watchdog and append-safe new output files.

These changes produced a new six-file source manifest. The complete matrix,
lifecycle, and display scripts were first rerun as
`*-vs17-14-sdk26100-v2.jsonl`. Re-review found that the reset timer ended
immediately before `StartCapture()` rather than after it returned. The timer
endpoint was corrected and all three scripts were rerun as
`*-vs17-14-sdk26100-v3.jsonl`. Only `v3` is accepted evidence; every earlier
complete result remains diagnostic history.

## Revision and host qualification

| Field | Frozen value |
|---|---|
| Plan frozen | 2026-07-30T20:45:00+09:00 |
| Product base | `7ae9050e9445a746eb2237c721c05eca4f7a1618` (`origin/dev/0.2.0`) |
| Product branch | `feat/phase-2-g002-windows-capture-ownership` |
| Prototype location | Local `.rasen/g-002/` work area; never a Cargo workspace member or product package |
| Required host | Windows 11, Core i7-12700KF, 32 GiB, NVIDIA GeForce RTX 4080 |
| Capture API | `Windows.Graphics.Capture` free-threaded frame pool, BGRA8 SDR |
| Native API baseline | Windows SDK 10.0.22621.0 and the installed MSVC x64 toolset |

That native API row records the environment present when the plan was frozen;
it is not a required candidate or hard-gate value. After the host development
environment was updated, the final source manifest was rebuilt with MSVC 19.44
and Windows SDK 10.0.26100.0 and all three scripts were rerun. The updated run,
not the initial diagnostic pass, is the acceptance record in
[report-x86_64-pc-windows-msvc.json](report-x86_64-pc-windows-msvc.json).

The prototype-location row records where measurement began and remains part of
the frozen plan. After the accepted run, the exact source and all raw
observations were relocated to
`rasen/changes/phase-2-g002-windows-capture-ownership/work/` in the local Rasen
repository. Generated build output was excluded because it embeds workstation
paths and is reproducible from the retained source and recorded toolchain.

The run record, rather than this plan, records the exact OS edition and build,
GPU driver, compiler, SDK, local prototype source digest, timestamp, display
topology, DPI scale, and selected adapter. The product base is the revision this
gate unblocks; the disposable prototype is bound separately by the digest of
every retained source file used for the run.

## Controlled source

The target is a prototype-owned borderless Win32 window. It continuously paints
synthetic BGRA8 tiles and a sequence marker derived from a monotonically
increasing counter. No desktop image, application title, process list, captured
pixel buffer, or unrelated window metadata is written to disk.

The target has three scripted sizes:

- `steady`: 1,280 x 720 physical pixels;
- `resize`: 1,280 x 720 -> 960 x 540 -> 1,920 x 1,080 -> 1,280 x 720;
- `four-k`: 3,840 x 2,160 physical pixels, once on each 4K display.

Painting runs at 60 requested updates per second. Capture is allowed to coalesce
updates; the oracle therefore validates each delivered image internally instead
of requiring one captured frame for every painted counter.

## Candidate matrix

Every compatible combination below is measured with WGC producer-pool sizes
2, 3, and 4.

| Candidate | Detachment point | Published native owner | Texture reuse |
|---|---|---|---|
| `wgc-retained` | None | `Direct3D11CaptureFrame` and its WGC surface | WGC-managed producer slot |
| `copy-fresh` | Callback, before releasing the WGC frame | One private default-usage texture | Never reused while retained |
| `copy-leased` | Callback, before releasing the WGC frame | Private texture lease | Reuse only after the last retained/map/backend lease releases it |
| `copy-blind-2` | Callback, before releasing the WGC frame | Private texture reference | Unconditionally reuse a two-texture ring; negative control |

`wgc-retained` is intentionally expected to pin a producer slot under the
retention schedule. `copy-blind-2` is intentionally expected to let a later
copy overwrite retained content. A negative control that unexpectedly passes
does not prove safety; it invalidates the workload and requires a stronger run.

For `copy-leased`, the initial reusable capacity equals the WGC pool size.
When all reusable textures are leased, the prototype allocates another texture
up to the predeclared retained-frame bound. It never overwrites a leased
texture. After release, that texture can return to the reuse queue.

## Workload and sample policy

Each steady candidate row is a fresh process and fresh D3D11 device:

1. Create the deterministic target, D3D11 device, capture item, free-threaded
   frame pool, and session.
2. Discard 120 delivered warm-up frames. Warm-up failures still fail the run;
   they are excluded only from timings.
3. Observe 600 delivered frames or a 20-second deadline, whichever occurs
   first.
4. Retain every third published frame. Submit alternating retained frames to a
   mapping worker and a simulated backend worker after a 90-delivered-frame
   delay. Release each after validation, with at most 40 retained frames.
5. Map every delivered frame once immediately and every retained frame again
   after its delay.
6. Stop admission, revoke and drain the callback, close the session and pool,
   release producer resources, then allow already admitted retained/map/backend
   work to finish from its own native owner.

Each fresh process is bounded by an external 45-second watchdog. A timed-out or
nonzero-exit process fails the script rather than leaving a partial row that
could be mistaken for acceptance evidence.

The simulated backend holds the same native texture lease as a real backend,
waits 25 milliseconds, then runs the same marker digest without retaining CPU
pixels. It exists to exercise lifetime and close ordering, not to estimate
backend performance.

The two-display run occurs outside normal work hours after a steady candidate
passes. It uses the same 120-frame warm-up and 600-frame sample on each signed
display, then repeats a 300-frame move across the display seam. Normal work
hours are 08:00 through 19:00 local time.

## Correctness oracle

Each frame is mapped through a private D3D11 staging texture. Validation checks:

- format is `DXGI_FORMAT_B8G8R8A8_UNORM`;
- content size and texture dimensions equal the current scripted target size;
- `RowPitch >= width * 4`, and rows are addressed using the reported pitch;
- eight interior sample tiles decode to one coherent target sequence and the
  predeclared colors for that sequence;
- decoded 16-bit sequence values may repeat when capture coalesces updates, but
  must advance wrap-aware, never regress, and never remain unchanged for more
  than 500 ms while the positive workload is active;
- the mapped marker digest is identical before and after the retention delay;
- mapped or backend work admitted before close/resize/reset observes either its
  original complete frame or an explicit typed prototype failure, never mixed
  generations or reused pixels;
- no validation path writes mapped bytes to disk.

A resize changes the geometry revision. A frame from the old size may complete
under its old revision; it may not be interpreted using the new dimensions.

## Lifecycle script

The lifecycle run uses the best steady candidate and executes:

1. the complete resize sequence at 180 delivered-frame intervals;
2. 100 close races at deterministic offsets 0, 1, 5, 15, and 30 milliseconds
   after callback admission, twenty repetitions each;
3. close while 20 mapping and 20 backend leases are in flight;
4. a queued-delegate fence that pauses `FrameArrived` before owner admission,
   closes the owner, then releases and safely rejects the delegate;
5. injected device-loss admission stop followed by complete session/device
   destruction and creation of a fresh D3D11 device and WGC session;
6. one real target close, followed by an idempotent second session close.

The injected device-loss case tests the ownership and teardown state machine;
it is not reported as a physical driver removal. A physical removal or TDR is
deliberately not induced on the development host.

The prototype counts live WGC frames, private default textures, staging
textures, callback invocations, admitted work, and completed work. The run
finishes only after every count returns to zero.

## Hard gates

The accepted policy must satisfy every hard gate:

| Gate | Required result |
|---|---|
| Producer progress | 600 delivered frames within 20 seconds, no post-warm-up arrival gap over 500 ms |
| Immediate mapping | Zero format, size, stride, tile, sequence, or digest failures |
| Retained mapping/backend | Zero changed digest, mixed-generation, use-after-release, or unexplained failure |
| Detachment | No retained public owner contains a WGC frame or producer-pool surface |
| Reuse | A texture is not copied into while any retained/map/backend lease owns it |
| Callback boundary | Callback only acquires, copies when selected, accounts, enqueues, and releases; it never maps, waits, or runs backend work |
| Close and reset | Admission stops before drain; complete close and reset each finish within 2 seconds; no callback is admitted to the owner after the fence, and a deliberately queued delegate is rejected safely |
| Resource bound | Live WGC frames <= pool size; private textures <= 40; staging textures <= 40; all return to zero |
| Negative controls | `wgc-retained` stalls or reaches the deadline and `copy-blind-2` reports a changed retained digest |

Latency, copy bytes, private-texture high-water marks, and frame-arrival
distribution are retained as rationale for Phase 2 `G-013`; they cannot override
a failed hard gate and are not product budgets.

## Evidence schema

The disposable probe writes JSON Lines with no image payload:

```text
schema_version
run_id
product_base_revision
prototype_source_sha256
started_at
host { os_edition, os_version, os_build, cpu, physical_memory_bytes }
toolchain { sdk, compiler, cmake }
gpu { adapter_luid, description, driver_version, driver_date }
topology [{ monitor_id_hash, left, top, right, bottom, dpi_x, dpi_y, scale_percent, primary }]
case { name, pool_size, detachment, reuse, target_size, display_id_hash }
sample { warmup_frames, required_frames, delivered_frames, max_gap_ms, elapsed_ms }
oracle { immediate_checks, retained_checks, backend_checks, sequence_progressions, sequence_duplicates, sequence_regressions, max_sequence_stall_ms, failures }
resources { wgc_frame_peak, private_texture_peak, staging_texture_peak, final_live_counts }
lifecycle { resize_revisions, close_offset_ms, callbacks_after_fence, delegates_rejected_after_fence, drain_ms, close_ms, reset_ms, target_closed }
traffic { copied_frames, copied_bytes, mapped_frames, mapped_bytes }
outcome { pass, rejected_reason }
```

Display identifiers are SHA-256 hashes of adapter LUID plus output ordinal,
truncated to 16 hexadecimal digits. Rectangle coordinates are signed so a
display left of or above the primary is represented correctly. The evidence
contains no EDID serial, user name, machine name, window title, target process
path, desktop pixels, recognized text, or captured-frame hash.
