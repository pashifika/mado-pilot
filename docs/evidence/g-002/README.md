# G-002 Windows capture ownership evidence

This directory records the experiment that resolves
[`G-002`](../../validation-gates.md#g-002). It selects the ownership rule a
future Windows capture Adapter must implement; it does not add a production
Adapter or a support claim.

The accepted rule is a two-frame WGC producer pool whose callback copies each
admitted frame into an Adapter-owned D3D11 texture before releasing the
`Direct3D11CaptureFrame`. Detached textures are reused only after their last
frame, mapping, and backend lease releases them. The complete rule and its
teardown consequences are in
[ADR 0013](../../adr/0013-windows-capture-frame-detachment.md).

## Tracked record

| File | Purpose |
|---|---|
| [experiment-plan.md](experiment-plan.md) | Workload, candidates, oracle, hard gates, and redaction policy frozen before measurement |
| [dependency-review.md](dependency-review.md) | Prototype-only Windows toolchain and dependency review |
| [probe.md](probe.md) | Disposable probe boundary, source manifest, and reproduction procedure |
| [report-x86_64-pc-windows-msvc.json](report-x86_64-pc-windows-msvc.json) | Machine-readable distilled measurements and raw-run hashes |

The product base is
`7ae9050e9445a746eb2237c721c05eca4f7a1618`
(`origin/dev/0.2.0`). The accepted confirmation uses prototype manifest
SHA-256
`3934dcf89d234cdf4f9460f8b53a30385c9397f6a0cb1f923ac806b6d82b84ae`.
The manifest in [probe.md](probe.md) makes that digest independently
reproducible. The exact throwaway source is retained with the experiment audit
trail in the local Rasen repository at
`rasen/changes/phase-2-g002-windows-capture-ownership/work/prototype/`.

The product repository keeps the distilled, reviewable record above rather than
treating verbose instrument output as a product fixture. The local Rasen
repository preserves all accepted and rejected JSON Lines at
`rasen/changes/phase-2-g002-windows-capture-ownership/work/raw/`, while the
accepted-run SHA-256 values remain bound into the product report:

| Run | UTC window | Rows | Raw SHA-256 |
|---|---|---:|---|
| Candidate matrix | 2026-07-30 15:19:13–15:23:19 | 12 | `578C3ED8EB234204651D8B07C8E1406D2FB5D23A4E2BBA9452E0FD707E33A5E9` |
| Lifecycle | 2026-07-30 15:24:01–15:25:40 | 106 | `FC98F10C7998362D2DF6F9EC6FC51B63316047DD9C1382891B28A0F0E50236D8` |
| Two-display | 2026-07-30 15:26:21–15:27:05 | 3 | `983A83B1936CB565C4A3D7B5123A881B576340AC22864127FDDC37DA27B142C6` |

## Host and topology

The confirmation ran on the approved host after its development toolchain was
updated:

| Field | Recorded value |
|---|---|
| Release target | `x86_64-pc-windows-msvc` |
| Operating system | Windows 11 Pro 25H2, build `26200.8894` |
| CPU and memory | 12th Gen Intel(R) Core(TM) i7-12700KF, 34,197,635,072 physical bytes |
| GPU | NVIDIA GeForce RTX 4080, driver `32.0.15.9186` dated 2026-01-20 |
| Toolchain | Visual Studio 2022 17.14.37, raw compiler ID `MSVC 1944.194435228` (MSVC 19.44.35228), Windows SDK 10.0.26100.0, CMake 3.29.5 |
| Primary display | hash `256a8910deb3558b`, `[0, 0, 3840, 2160)`, 144 DPI, 150% scale |
| Secondary display | hash `239cd96054f2204b`, `[-3840, 0, 0, 2160)`, 144 DPI, 150% scale |
| Seam case | hash `c389cd654fa78da9` |

Display hashes are derived identifiers, not EDID values. The two-display run
started at 00:26 JST on 2026-07-31, outside the plan's 08:00–19:00
work-hour exclusion.

## Candidate matrix

Every positive detached candidate delivered all 600 post-warm-up frames.
Every negative control failed for its predeclared reason, so the workload
demonstrated both failure modes it was designed to detect.

| Pool | Candidate | Frames | Max gap (ms) | Sequence advances | Private texture peak | Result |
|---:|---|---:|---:|---:|---:|---|
| 2 | `wgc-retained` | 6 | 41.766 | 116 | 0 | Expected rejection: producer progress |
| 2 | `copy-fresh` | 600 | 62.470 | 661 | 32 | Pass |
| 2 | `copy-leased` | 600 | 63.052 | 674 | 33 | Pass; selected |
| 2 | `copy-blind-2` | 600 | 63.247 | 667 | 2 | Expected rejection: retained and backend digests changed |
| 3 | `wgc-retained` | 9 | 62.836 | 119 | 0 | Expected rejection: producer progress |
| 3 | `copy-fresh` | 600 | 62.934 | 672 | 32 | Pass |
| 3 | `copy-leased` | 600 | 63.259 | 687 | 32 | Pass |
| 3 | `copy-blind-2` | 600 | 63.671 | 671 | 2 | Expected rejection: retained and backend digests changed |
| 4 | `wgc-retained` | 12 | 42.500 | 119 | 0 | Expected rejection: producer progress |
| 4 | `copy-fresh` | 600 | 63.449 | 681 | 32 | Pass |
| 4 | `copy-leased` | 600 | 62.763 | 675 | 32 | Pass |
| 4 | `copy-blind-2` | 600 | 63.422 | 683 | 2 | Expected rejection: retained and backend digests changed |

All detached rows copied 2,654,208,000 bytes and mapped 3,391,488,000 bytes.
There were zero immediate, retained, or backend correctness failures in the six
positive rows and zero final live resources in all twelve rows.
The sequence oracle observed no regression; its maximum sequence stall was
22.613 ms. Complete close, including session/pool close, lease completion,
worker join, and final resource release, took at most 527.587 ms.

Pool sizes three and four produced no correctness, progress, arrival-gap, or
resource-bound improvement over two. `copy-fresh` was correct but created a new
private texture for every callback; its peak only reports simultaneously live
textures and does not erase that allocation churn. `copy-leased` gives the same
correctness while making reuse explicit and safe. Direct WGC retention and
blind reuse are therefore rejected, and the smallest passing producer pool
with lease-aware detachment is selected.

## Lifecycle and display results

The selected policy passed all 106 lifecycle rows:

- one 600-frame resize run with three geometry revisions;
- 100 close races: 20 each at 0, 1, 5, 15, and 30 milliseconds;
- two in-flight-close checks totaling 50 retained-map and 50 backend checks;
- one real target close;
- one deterministic queued-delegate fence that rejected the late delegate
  without admitting it to the owner;
- one injected-loss teardown followed by a fresh device/session and a
  600-frame steady run.

The maximum callback drain was 0.037 ms, maximum complete close was 527.302 ms,
and complete reset from admission stop through the fresh session's
`StartCapture` return was 650.355 ms. No callback was admitted to the owner after the
fence, one intentionally delayed delegate was rejected there, sequence
regressions were zero, the maximum sequence stall was 22.234 ms, the
private-texture peak was 33, and every final resource count was zero. The
injected case tests the reset state machine; it is not evidence of a physical
TDR or driver removal.

The two 4K display rows each delivered 600 frames and the seam row delivered
300. Their maximum arrival gap was 62.784 ms, maximum sequence stall was
22.245 ms, sequence regressions were zero, maximum complete close was
528.001 ms, private-texture peak was 33, and all immediate, retained, and
backend checks passed. The two display rows each
copied 23,887,872,000 bytes and mapped 30,523,392,000 bytes; the seam row copied
13,934,592,000 and mapped 17,252,352,000 bytes. These traffic values are inputs
to the later Phase 2 `G-013` budget, not budgets set by this Change.

## Redaction and applicability

The probe owned and painted its target window. No desktop payload, mapped
pixel buffer, captured-frame hash, window title, process path, machine name,
user name, recognized text, EDID serial, or monitor PNP identifier is retained.
The tracked report contains only synthetic-oracle counts, bounded resource
counts, timing, byte traffic, hashed display identifiers, and environment
versions.

The initial experiment pass used the same product base with MSVC 19.37 and
Windows SDK 22621. After the host toolchain update, the complete matrix,
lifecycle script, and two-display script were rerun. Review then strengthened
callback ownership, complete close/reset timing, process watchdogs, and the
sequence-freshness oracle. Re-review then extended reset timing through
`StartCapture()` return. All three scripts were rerun again from the final
six-file source manifest. Only that final pass is the acceptance record above.
Earlier and rejected runs are retained in the local Rasen work record as
diagnostic history, but are not used to resolve the gate.
