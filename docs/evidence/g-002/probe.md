# G-002 disposable probe

The G-002 probe is deliberately not product code. It was initially built under
the repo-local `.rasen/g-002/prototype/` work area and is not a Cargo workspace
member, dependency, example, or supported utility. After acceptance, its source
and raw observations were moved into the local Rasen repository at
`rasen/changes/phase-2-g002-windows-capture-ownership/work/`. This document
retains the experiment boundary and source identity; the Rasen work record
retains the lower-level audit trail.

## Source manifest

The accepted confirmation digest is computed as follows:

1. Sort the six relative file names below bytewise.
2. Compute SHA-256 for each file's exact bytes.
3. Form one UTF-8, no-BOM manifest line per file as
   `<lowercase-hash><two spaces><relative-name><LF>`.
4. Compute SHA-256 over the complete manifest bytes.

| File | SHA-256 |
|---|---|
| `CMakeLists.txt` | `3505673e27861b0be8811e8f56894da1d5003afdc8e116e27703ef3d200e9d17` |
| `invoke-probe.ps1` | `638d31b445e133dfaf18e167e94b09e807dbb38ff7628b4d76b27488db91ecb1` |
| `main.cpp` | `54f5ff9b7f5df88ff238d0ca713d0bd416018ed422b3643f87aeff2c83d38f8f` |
| `run-displays.ps1` | `12bb7abbb368cc801a9778c3f77c3dac306ff37b398cfee5e0cebb5853c6a155` |
| `run-lifecycle.ps1` | `4339bd127ba439b04608d34f6d7c99b2dd9e72fa432f4e9b10ca16835ba365da` |
| `run-matrix.ps1` | `93a6e678027a3ffc1db02992b1ee265b23c73b3810373427d0b97b1744ca3c55` |

The resulting manifest SHA-256 is
`3934dcf89d234cdf4f9460f8b53a30385c9397f6a0cb1f923ac806b6d82b84ae`.
Every accepted JSONL row carries that value and product base
`7ae9050e9445a746eb2237c721c05eca4f7a1618`.

## Instrument boundary

The C++20 executable uses only C++/WinRT, WGC, D3D11, DXGI, Win32, and operating
system import libraries. It creates a controlled borderless target window,
paints deterministic BGRA8 markers, captures it through
`Direct3D11CaptureFramePool::CreateFreeThreaded`, and emits one redacted JSON
object for each process run.

The four candidates are:

- `wgc-retained`: publish the WGC frame and producer surface directly;
- `copy-fresh`: copy into a newly created private default-usage texture;
- `copy-leased`: reuse a compatible private texture only after its lease count
  reaches zero, allocating only up to the experiment's bound of 40;
- `copy-blind-2`: overwrite a two-texture ring without checking leases.

Every non-direct candidate calls `ID3D11DeviceContext::CopyResource` before it
releases the WGC frame. Mapping uses a separate staging texture outside the WGC
callback. The callback never maps, waits, performs backend work, or invokes a
host callback.

Both asynchronous WinRT handlers capture lifetime-independent shared callback
state rather than the `CaptureProbe` owner directly. Callback admission,
owner detachment, active-callback accounting, and fence publication use one
mutex. Close revokes both handlers, waits for admitted callbacks, publishes
the fence, and only then closes the session and pool. A deterministic
`callback-fence` mode pauses one delegate before admission and proves it is
rejected after the fence without dereferencing the owner.

## Reproduction shape

The accepted build used the Visual Studio 17 2022 x64 generator, MSVC 19.44,
and Windows SDK 10.0.26100.0. A conforming independent reproduction must:

1. Implement the controlled source, four candidates, counters, and redacted
   row schema from [experiment-plan.md](experiment-plan.md).
2. Build a fresh process for each matrix row and use 120 warm-up frames followed
   by 600 samples or the 20-second post-warm-up deadline.
3. Run pool sizes 2, 3, and 4 for every candidate.
4. Run the resize, 100 close-race, in-flight-close, target-close,
   deterministic queued-delegate fence, and reset script against
   `copy-leased` with pool size 2.
5. Outside normal work hours, run 600 frames on each 4K display and 300 while
   moving the controlled window across their signed-coordinate seam.
6. Run each fresh probe process under the retained 45-second watchdog, parse
   every emitted line as JSON, verify its product and source digests, and
   compare it with the hard gates before looking at timing as a performance
   result.

The probe writes no pixel payload. After its SHA-256 and distilled rows have
been reviewed into
[report-x86_64-pc-windows-msvc.json](report-x86_64-pc-windows-msvc.json), the
raw output remains in the local Rasen work record so reviewers can reconstruct
how accepted and rejected runs led to the report.

## Instrument corrections

Rejected local runs were preserved during the experiment rather than silently
overwritten. They exposed and corrected these instrument defects before the
accepted confirmation:

- the sample deadline initially started before warm-up instead of after it;
- the resize transition needed to recreate the WGC pool and preserve detached
  old-generation leases;
- callback-drain notification needed the same mutex discipline as its waiter;
- the injected reset needed a complete fresh steady run;
- full-window GDI painting at 4K could be captured between drawing operations,
  so the oracle markers were moved into one small invalidated strip.
- asynchronous handlers captured a raw owner pointer, so both now use
  lifetime-independent shared state with admission and fence synchronization;
- the lifecycle workload did not force a queued delegate across the fence, so
  `callback-fence` now supplies that deterministic barrier;
- `drain_ms` covered only part of teardown, so the report now separately
  records complete `close_ms` and admission-stop-to-new-session `reset_ms`;
- the first strengthened reset timer ended immediately before `StartCapture()`,
  so the final timer ends after `StartCapture()` returns;
- coherent repeated markers could pass without freshness, so the oracle now
  decodes the 16-bit sequence, permits duplicates/coalescing, and rejects
  regression or more than 500 ms without progress;
- direct script invocation could hang without external evidence, so all run
  scripts now use the process watchdog and reject nonempty output files.

Each correction changed the source digest and invalidated affected rows. The
accepted confirmation reran all three scripts from the final manifest above.
