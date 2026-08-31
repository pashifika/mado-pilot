# ScreenCaptureKit suspension signature capture

Status: complete as a ten-process bounded non-reproduction. This is diagnostic evidence, not a production repair, qualification replacement, or support-promotion result. Native template-watch support remains `WITHHELD` under ADR 0057.

## Evidence identity

The prepublished protocol and retained outputs bind the campaign to the exact diagnostic source:

- source revision: `ba6b0451a1d2b13a8fe418c477e91ea00fe9cff8`;
- source tree: `c645124b42064ae2c10c4f2cb28b86f8d7b385ad`;
- protocol SHA-256: `5f1d519544df585e287fc6af4b5c28e8f1d94f5fb8d0d90196cd2b5367209316`;
- diagnostic executable SHA-256: `c8072bc557991d0762326b83e0c327d2c47cc3d359ff76e8d511a3a98ec6a623`;
- fixture executable SHA-256: `c647f79697c28800b5b28b27f97f9847c762149f074c0db198252e6981c9de7f`;
- ordered fixture-source SHA-256: `f1c60db58650c1770f67b22c0e65563cf125c3800f43242d973da900d795395a`;
- preflight SHA-256: `f5befecc11745dbb63e4666c302a2d2a7f87d00f348a8765aca7c04bca63ecd6`;
- aggregate SHA-256: `00b058b1cdbeb0fb38caa9ebea7407af487dd6dfdd761ce8b377e5f4f59bd4d1`;
- execution-index SHA-256: `b63eaa59c102887e171c72963b20ea65d1654321ee572b870eeb3a9f6227fb0e`.

Retained machine-readable evidence:

- [prepublished protocol](macos-sck-suspension-signature-capture/native-signature-protocol-ba6b045.json);
- [host and minimal cohort preflights](macos-sck-suspension-signature-capture/native-signature-preflight-ba6b045.json);
- [typed ten-row aggregate](macos-sck-suspension-signature-capture/native-signature-aggregate-ba6b045.json);
- [ordered execution index and output hashes](macos-sck-suspension-signature-capture/native-signature-execution-index-ba6b045.json).

The Rust `mado_pilot_testkit::sck_suspension_signature_report::validate_aggregate` validator accepted all ten rows in order. Every process exited `0`, every stderr stream was empty, and every final resource snapshot exactly matched its process baseline.

## Minimal topology protocol

The two cohorts ran in one authenticated graphical session without reboot or logout:

1. process indexes 1 through 5 used the single-display control class;
2. process indexes 6 through 10 used the two-display mixed-DPI class.

Topology evidence contains only `display_count` and `has_distinct_backing_scales`. It contains no dimensions, refresh rates, built-in or external classification, mirror state, backing-scale values, names, identifiers, or per-display records. The second cohort required exactly two displays whose backing scales differed; no exact display configuration was required or retained.

## Observed result

All ten rows reached `stage=complete` with `outcome=complete_frame` and `failure=none`.

The lifecycle observations were uniform across both cohorts:

- old close reached native phase 5 before fresh open;
- the retained terminal result and mapping kept the expected one detached lease and 4,628,480 detached bytes while reporting zero live old sessions and zero callbacks in flight;
- the fresh stream identity differed from the old stream identity;
- the fresh producer reported one complete status and no idle, blank, started, suspended, stopped, missing, or unknown status;
- the first fresh complete frame preceded asynchronous start-completion notification in every row, so post-start latency remained absent rather than being inferred;
- the retained mapping remained readable after fresh close;
- releasing the retained result, mapping, and diagnostic observer restored `{native_objects: 0, detached_bytes: 0, live_sessions: 0, callbacks_in_flight: 0}` in every process.

## Conclusion

This finite matrix did not reproduce the terminal-red suspension. The result is bounded non-reproduction, not evidence that the historical failure was invalid.

The observations reject these conditions as deterministic causes on `ba6b045` under the exercised classes:

- the exact public `retained_result_mapping` ownership and drop order;
- one expected detached frame lease and its bounded storage;
- close/open overlap after the old close fence;
- target authentication or fixture readiness failure;
- display count plus mixed backing scales by themselves.

No row selected recovery, queue drain, ownership refactoring, deadline extension, retry, public status exposure, or another production change. ADR 0059 therefore continues to prohibit speculative repair, and ADR 0057 remains authoritative for support.

The earlier `f0eab45` eight-process diagnostic and its hashes remain revision-bound historical evidence in [ScreenCaptureKit suspension diagnosis](macos-sck-suspension-diagnostic.md); this campaign does not rewrite or reinterpret that record.
