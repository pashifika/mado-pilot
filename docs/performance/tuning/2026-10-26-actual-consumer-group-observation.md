# Focused actual-consumer group observation

The authored `group-roi` application path completed once per frozen role on each
target. Apple uses fresh loader-off development smokes; Windows uses its already
completed historical pair, with no new local run. The selected group intervals
are lower in these observations, but this is not a robust speedup, p95, CPU or
memory qualification, nor execution of the current producer checkout.

Public allowlisted records:

- [Apple observation](../../evidence/p050-actual-consumer-group/apple-observation.json)
- [Windows retained observation](../../evidence/p050-actual-consumer-group/windows-observation.json)

## Exact scope and identities

Both targets use effective producer baseline `beb064f0c652648f92df5bfb65aaf05384a18b8d`
and current `dd320044cdd390ee662117c69e865391f72ed996`, with the final
`consumer-rss-direct-checkpoint-v1` consumer source, separate Cargo locks and
14-package local producer overrides. The raw `build.engine_revision` remains
`f61a57f661c0314ea0a1d860d9298fbff86d903d` in both roles: that is the authored
Cargo Git pin, not the effective local producer graph. It is not rewritten.
Executable hashes and effective source/Cargo evidence establish the selected
roles separately.

Configuration is Rust1.98.1-built dev-normal: `opt_level=0`, `debuginfo=2`, debug
assertions, `MADO_P050_OBSERVATIONS=1`, and no benchmark accounting. These are
not optimized release binaries or observation-off application measurements.
Neither target rebuilds or relabels the retained executables for this scope.

The saved1920×1080 RGBA case retains ROI(64,48,320,180), eight12×10 templates,
threshold0.9/max_results8 and `DropOverlapping`. Both roles pass the five-observation,
ten-recognition, fourteen-release sequence: eight retained matches followed by
the newer miss/hit checks. All35 host-call records are retained without drops;
application semantics and owned cleanup pass.

The full `dd320`→`c305` source review identified no changed preparation/mapping
behavior for this authored suppression path. PR114's bounded `KeepAll` selection
branch does not apply. Shared selector control flow did change, so this is not
binary/timing equivalence and cannot be relabeled as a `c305` or final-source run.

## Selected group interval

`group_span_ns` runs from the first recognize call's start to the eighth paired
release call's end on the same `control.started` clock. It includes inter-call
application/host handoff gaps. It excludes readiness, request-array preparation,
later result assertions, newer miss/hit operations and overall startup/cleanup.
It is neither one recognize call nor cross-process clock subtraction.

| Target/cohort | Baseline `group_span_ns` | Current `group_span_ns` |
| --- | ---: | ---: |
| Apple, fresh loader-off pair | 66844333 | 34444917 |
| Windows, retained historical pair | 49654500 | 26164800 |

The records also retain all eight recognize and release intervals and the
separate invocation/application-stage measurements. One invocation per role
cannot establish repeatability, drift, a joint p95/CPU improvement or unique
causality. The targets and collection histories are not pooled.

## Resource observations and limits

These are observed maxima of5ms periodic process RSS samples, not group-only
bracketed values, OS peaks, allocator bytes or post-owner leak measurements.
Child and supervisor are separate processes and are not summed.

| Target/process | Baseline sampled RSS bytes | Current sampled RSS bytes |
| --- | ---: | ---: |
| Apple child | 251789312 | 245399552 |
| Apple supervisor | 52953088 | 53116928 |
| Windows child | 211415040 | 212123648 |
| Windows supervisor | 43597824 | 43511808 |

Both targets' child/supervisor steady summaries remain `null`: the workflow ended
before the declared steady window completed. Windows startup and preflight are
also higher in its observed current invocation; their exact values remain in
the record. No generally lower-memory or startup claim follows.

Apple raw `posix-wait4` CPU is `partial` with `descendant_rollup_unproven`.
Windows raw Job CPU is `complete`, but effective resolution and the reconciled
lifetime CPU metric remain `null`. Raw Job5/0/0 is not a process-role census.
Raw CPU values are preserved without turning incomplete accounting or a
quantized observation into a benefit claim.

The post-workload thread getter reports TBB10 on Apple and ms-concurrency20 on
Windows. These are getter points, not observed worker activity or invariance.
Normal image closure, source-closed comparable CPU, UI responsiveness, capture,
input and OCR qualification are outside this observation.

## Collection and verification

PR116 adds explicit `smoke --role baseline|candidate` to the
[actual-consumer collector](../methods/actual-consumer.md), defaulting to baseline.
It preserves each role through sealed `consumer-phase-v2` receipts and `report`,
without changing qualification schedules or safety fences. The changed contract
suite passes112 tests and exact source `fa5d6e3e0a575ce2cef67ff4479588e52fc278b7`
passes all16 CI result checks.

Apple uses that collector source, one new plan and two separate smoke outputs
under the existing campaign. A retained finite owner covers the three stages,
three processor-metadata calls and two actual consumer invocations. Baseline
raw/semantic/cleanup inspection precedes candidate launch. Both actual `report`
calls return zero without changing original receipts. All2208 source/input
entries pass the collector's full-content checks; no build, native retry,
calibration, A/A or profile follows.

Windows remains bound to its original collector/owner and archive
`d3731f1d8012656e6fb87c32d8aef5ecda475b4aededba15ef309924b2b737c3`.
The final post-write seal is `PASS`; earlier `PROVISIONAL_PASS` records remain
unchanged. Its673 full-content and1520 metadata-only guards retain that
distinction. This historical pair does not verify the new role CLI on Windows.

Public files contain allowlisted observations and original raw hashes, not full
private records. Environment values, absolute private paths, host fingerprints,
PIDs and arbitrary logs are excluded. Full originals remain in the Change's
private ephemera. No old receipt, binary, failure or qualification status is
replaced by this publication.
