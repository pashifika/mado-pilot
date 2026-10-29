# Actual-consumer saved-RGBA trial observations

## Selected behavior

Fresh trial-driver/controller, capability-child and recognition-child invocations can initialize the chosen resources, match an immutable saved frame, deliver a result and finish terminal cleanup. Fresh does not mean fresh live-screen content, an authenticated application owner or an OS-cold environment. UI, native capture and input are not exercised.

Apple runs one fresh baseline/current pair for `trial-hit-roi` and `trial-miss-roi`. Windows uses the corresponding completed historical pairs; no Windows native rerun or PR116-collector execution is inferred. These trials are separate from the [query waits](2026-10-27-actual-consumer-query-observations.md) and [retained-observation groups](2026-10-26-actual-consumer-group-observation.md).

## Source, inputs and resources

Effective frozen producer roles remain `beb064f0c652648f92df5bfb65aaf05384a18b8d` and `dd320044cdd390ee662117c69e865391f72ed996`, dev/observation-on with normal allocator. The trial driver and child engine are separately bound executables. Authored driver revision constants and raw engine Cargo Git pin `f61a57f661c0314ea0a1d860d9298fbff86d903d` are not substituted for the effective local source graphs, path patches and original build receipts. No final-release binary or timing equivalence is claimed.

Each trial uses a hash-bound1920×1080 saved RGBA frame,12×10 PNG template, ROI64/48/320/180 and authored0.9/8 options. The trial-specific request keeps `DropOverlapping`; its limit remains min(8,257)=8, so PR114's KeepAll optimization is inapplicable. Hit returns one match at full-frame bounds80/64/12/10; miss returns no matches. All four Apple and four retained Windows invocations preserve these results and known cleanup.

The selected environment is `g-004-rapidocr-ppocrv4-det-v6-rec-small-v1`, CPU provider, `onnxruntime-1.29.0-api17-cpu`, OpenCV4.14. Source and actual successful trial paths verify resource identities, initialize a replay engine with the OCR profile, reverify resources and require CPU with no fallback. Capability probing is a separate child and does not substitute for recognition-child initialization. No OCR inference is requested. Initialization is a compound engine/OCR-profile stage, not inference time.

Capability reports retain a baseline/current `max_ocr_zones` difference of8/16 on both targets. The selected workload is template matching; its authored result limit stays8. Public projections include environment/snapshot identities and selected model/runtime input bindings, without claiming full loaded-image closure.

## Whole-operation observations

`operation_us` starts before trial reservation and ends after public shutdown and the final controller poll, before result-oracle evaluation. Driver preflight, the driver's pre-receipt total and the outer invocation duration retain separate boundaries; the pre-receipt total is not process-exit time.

| Target/case | Baseline operation us | Current operation us |
|---|---:|---:|
| Apple hit ROI |549162|467566|
| Apple miss ROI |473710|471342|
| Windows retained hit ROI |551778|564622|
| Windows retained miss ROI |539715|543622|

One pair per case is descriptive evidence, not statistical responsiveness or non-regression acceptance.

## Recognition-child search intervals

These intervals use `search_finished - search_started` within the recognition child's own `control.started` clock. They do not include resource verification, engine initialization, template preparation, parent delivery or cleanup, and are not subtracted from a different process clock.

| Target/case | Baseline search ns | Current search ns |
|---|---:|---:|
| Apple hit ROI |9485333|4811459|
| Apple miss ROI |8186875|4748875|
| Windows retained hit ROI |7145200|3511800|
| Windows retained miss ROI |5385900|3123900|

Shorter observed search intervals do not establish whole-operation benefit: both retained Windows operation durations increase. Complete child marks and separately scoped parent marks remain in the projections.

## Resource and lifecycle tradeoffs

Adverse observations are retained rather than retried:

| Observation | Current minus baseline |
|---|---:|
| Apple hit engine/OCR-profile initialization |+2546958ns|
| Apple hit resource reverification |+547876ns|
| Apple miss recognition-child RSS after owner release |+3211264B|
| Apple miss session open |+213249ns|
| Windows hit engine/OCR-profile initialization |+5812700ns|
| Windows hit recognition-child RSS after owner release |+249856B|

RSS uses the actual `native-self-rss-v1` recognition-child point after trial owner release. It is not sampled peak, OS high-water, same-owner growth, final-owner residual or a leak verdict. Apple raw lifetime CPU remains partial; Windows raw Job CPU is complete but effective resolution/closure and reconciled comparable lifetime CPU remain unavailable. Job5/0/0 is not a PID-role census. Native thread getters remain endpoint observations, not invariant worker counts.

The parent observes capability ChildStarted/Terminal, recognition ChildStarted/EntrySettled/Terminal and ControllerSettled with worker completion. EntrySettled can precede session close and owner drop: it is not terminal cleanup proof. Both children are separately identified and reaped; session close, owner-release stage, stream joins, terminal state and public shutdown complete. Child-local stage marks and parent `parent-control.started` delivery marks retain separate epochs.

## Finite execution and retained fences

Apple reuses each original case campaign, with fresh output registrations. The hit campaign's sealed numerical/incomplete control failure and explicit control fence remain byte-unchanged. The current collector's diagnostic smoke rule permits the selected runs only after validating the old attempted prefix and terminal cleanup. Qualification remains fenced; no new campaign root or historical regrading bypasses it.

Two prepare and four smoke stages complete once, with six owned processor metadata calls, four actual driver roots and eight reported capability/recognition children. All selected owner/native cleanup is known. Each baseline is inspected before candidate, hit before miss, and all four actual report calls preserve every original cohort hash. These are source-specific call roles, not a global process census.

The driver's declared60s/1MiB/5s contract remains separate from the collector's90s/4MiB/5s envelope and the finite enclosing stage. No native retry, build, calibration, A/A, quiet-host gate or following native scope is inferred.

Public evidence:

- [Apple projection](../../evidence/p050-actual-consumer-trial/apple-observation.json): fresh role identities, complete selected stage marks, resource points, child/parent settlement and receipt hashes.
- [Windows projection](../../evidence/p050-actual-consumer-trial/windows-observation.json): original archive/seal references, source-traced options/resource initialization, all outcomes and adverse values.

Private paths, environment values, PIDs and arbitrary logs are excluded. Original private receipts remain local; omitted originals are not represented as receiver-verified bytes. No qualified latency/CPU, allocation/leak freedom, OCR inference, UI responsiveness, final-release-source approval, archive, tag or release is claimed.
