# ScreenCaptureKit suspension diagnosis

Status: complete and green as an eight-process diagnostic; non-discriminating for the historical intermittent suspension. This is not formal qualification, not a production repair, and not support-promotion evidence.

## Evidence identity

The retained bundle binds the prepublished protocol, host preflight, each process output, and the aggregate to the exact diagnostic source:

- source revision: `f0eab45c3918914098040b96458e5d583bf2a32a`;
- source tree: `b948622ffb5ac7c3c572a2dea2ab4694d7601010`;
- protocol manifest SHA-256: `5bc10135c5aceb18c5da7989f9057af95808baf2eefd246013a922a11b5b1193`;
- diagnostic executable SHA-256: `3aa9485c733654834524bab10aee701bdf3cae29dad83dce87c1bbebb8be44ee`;
- fixture executable SHA-256: `4591eb891a93e133be7f9b7f5d55007618809cc72ee99e198ec56fe92a94fdfe`;
- ordered fixture-source SHA-256: `f1c60db58650c1770f67b22c0e65563cf125c3800f43242d973da900d795395a`;
- aggregate SHA-256: `463aea72246710bce9782a23d8d64781fd27e078275274d3dcce32cfccd93d2c`;
- execution-index SHA-256: `846d8e22a27ad6d4d17c8786d535311ca4185565f482bf64e8db9f4fdc2da30f`.

Retained machine-readable evidence:

- [prepublished protocol](macos-sck-suspension-diagnostic/native-diagnostic-protocol-f0eab45.json);
- [host and executable preflight](macos-sck-suspension-diagnostic/native-diagnostic-preflight-f0eab45.json);
- [typed eight-row aggregate](macos-sck-suspension-diagnostic/native-diagnostic-aggregate-f0eab45.json);
- [ordered execution index and row hashes](macos-sck-suspension-diagnostic/native-diagnostic-execution-index-f0eab45.json).

The topology was one built-in, non-mirrored `Color LCD`, 1512 by 982 logical pixels at 120 Hz. Screen Recording was authorized through the non-prompting public preflight probe. The typed `mado_pilot_testkit::sck_suspension_report::validate_aggregate` validator accepted all eight retained rows. Every process exited `0`; stderr was empty. The retained row bytes exactly equal each process's stdout and are bound by the execution-index hashes. Temporary raw output was deleted only after row hashing, typed aggregate validation, and privacy review completed.

## Fixed process matrix

Every row committed fixture revision 4, reached `stage=complete` with `outcome=complete_frame`, preserved its selected owner through fresh close, and restored `{native_objects: 0, detached_bytes: 0, live_sessions: 0, callbacks_in_flight: 0}` exactly.

| Process | Policy | Retainer | Elapsed | Old/fresh complete samples | Old ref / lease / detached bytes | Drain generation | Result |
|---:|---|---|---:|---:|---:|---:|---|
| 1 | `unchanged` | `none` | 3.344 s | 19 / 3 | 1 / 0 / 0 | 0 / 0 | complete |
| 2 | `unchanged` | `mapping_only` | 2.389 s | 16 / 2 | 1 / 0 / 0 | 0 / 0 | complete |
| 3 | `unchanged` | `frame_only` | 2.414 s | 18 / 2 | 2 / 1 / 4,628,480 | 0 / 0 | complete |
| 4 | `unchanged` | `both` | 2.406 s | 18 / 2 | 2 / 1 / 4,628,480 | 0 / 0 | complete |
| 5 | `drain_sample_queue` | `none` | 2.415 s | 17 / 2 | 1 / 0 / 0 | 1 / 1 | complete |
| 6 | `drain_sample_queue` | `mapping_only` | 2.434 s | 17 / 2 | 1 / 0 / 0 | 1 / 1 | complete |
| 7 | `drain_sample_queue` | `frame_only` | 2.417 s | 17 / 2 | 2 / 1 / 4,628,480 | 1 / 1 | complete |
| 8 | `drain_sample_queue` | `both` | 2.410 s | 17 / 2 | 2 / 1 / 4,628,480 | 1 / 1 | complete |

`mapping_only` retained readable copied CPU bytes after fresh close without retaining a native lease, detached native bytes, or an extra old-session reference. `frame_only` and `both` retained the exact immutable frame after fresh close and intentionally kept one old detached lease, one additional old reference/native object, and 4,628,480 detached bytes until cleanup. Every final process baseline returned to zero after the selected owners were dropped.

Every old and fresh producer observation normalized to `complete` with raw SDK value 0. No row observed `idle`, `blank`, `started`, `suspended`, `stopped`, `missing`, or `unknown`. All callbacks were admitted and exited; no callback was refused or left in flight. In every row the first complete status preceded or coincided with asynchronous stream-start completion, so the derived post-start first-complete latency was 0 ns.

## Conclusion

The observations support only these bounded conclusions:

- copied mapping ownership was not a necessary or deterministic cause on this host and topology;
- retaining a source frame was not sufficient to stall a fresh producer in this cohort;
- a sample-queue drain was not required for fresh progress in this green cohort;
- the failure-case effect of the drain remains unresolved because no suspension occurred;
- no status sequence, refused callback, leak, or target substitution explained the historical failure.

The matrix narrows the problem but does not identify a production cause. It does not disprove an intermittent frame/queue interaction or the historical two-display suspension. Because no failure signature differentiated `unchanged` from `drain_sample_queue`, no permanent queue drain, ownership refactor, recovery, deadline change, typed status, or public contract change is justified. ADR 0059 records the no-speculative-repair decision. Native template-watch support remains `WITHHELD` under ADR 0057.
