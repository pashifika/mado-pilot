# Windows capture ownership contract tests

This plan translates
[ADR 0013](adr/0013-windows-capture-frame-detachment.md) into tests for
`mado-pilot-platform-windows`. The production Adapter, controlled tests, and
approved interactive Windows host matrix now exist; this document distinguishes
their revision-bound evidence from Windows exact-candidate Phase 1 checks, Apple
Silicon Phase 1 reviewed applicability, repository checks, and release checks.

The implementation uses both layers below:

- deterministic Adapter-independent tests that exercise ownership, queue,
  lifecycle, and publication through controllable fakes;
- native WGC/D3D11 tests on the named Windows verification host for behavior a
  fake producer cannot establish.

The common public capture contract remains platform-neutral. Test-only
instrumentation may expose producer-slot, detached-texture, lease, callback, and
drop counters, but Windows or D3D11 types must not enter `mado-pilot-core`,
`mado-pilot-capture`, the facade, or the C ABI.

## Current implementation coverage

| Concern | Controlled evidence |
|---|---|
| Picker-free discovery, staged final arbitration, deterministic concurrent commit order, provider identity, required format | `r1_1_deadline_at_final_arbitration_leaves_all_registry_state_unchanged`, `discovery_snapshots_commit_in_query_order`, other `crates/platform/windows/src/provider.rs` unit tests, and `crates/platform/windows/tests/native_capture.rs` |
| Replacement identity and bounded current/previous generations | `r1_4_identical_raw_identity_with_a_fresh_lifetime_never_retargets_the_old_id`, `an_absent_identity_accepted_by_this_provider_is_conservatively_lost`, and the synthetic replacement-window path |
| Producer detachment, finite pressure, lease-safe reuse, resize retirement | `crates/automation/capture/tests/native_storage.rs` and `crates/platform/windows/src/storage.rs` unit tests |
| Checked surface and retained-byte bounds | `r1_2_surface_axes_and_bytes_accept_the_exact_boundary_and_reject_one_over`, `r1_2_surface_and_mapping_multiplication_overflow_is_typed_before_allocation`, `r1_2_production_limits_admit_the_exact_4k_retention_and_mapping_workload`, `r1_2_reported_capacity_is_truthful_for_4k_and_reduced_for_8k`, `r1_2_derived_4k_capacity_admits_first_and_fortieth_then_refuses_forty_one`, `r1_2_derived_8k_capacity_admits_the_first_and_twelfth_then_refuses_thirteen`, `r1_2_production_session_and_global_limits_are_exact_and_shared`, `r1_2_production_multi_session_description_matches_shared_pressure_and_resume`, `r1_2_mapping_budget_follows_pixels_that_outlive_the_session_owner`, `r1_2_process_shared_mapping_stays_charged_after_session_close_until_pixels_release`, and the resize budget-release assertion |
| Pressure/discontinuity precedence | `finite_path_drop_debt_survives_discontinuities_until_a_gap_can_represent_it` preserves two drops through two `FIRST` epochs, `unrepresentable_drop_debt_neither_wraps_nor_disappears_at_a_discontinuity` proves checked refusal retains debt, and `r1_2_production_multi_session_description_matches_shared_pressure_and_resume` covers production shared-budget refusal, resize/release, the new epoch's `FIRST`, and the next legal pressure-visible publication |
| Producer reservation and native teardown lifetime | `r1_2_producer_reservation_stays_with_queued_native_ownership_until_close`, `r1_2_producer_reservation_stays_charged_in_quarantine`, and `r1_2_resize_replaces_producer_reservation_only_after_native_success` use the production native-owner seam to hold queue/quarantine ownership and exercise failed/successful replacement |
| Capacity-one latest-wins handoff | `r1_3_declared_latest_wins_matches_two_publications_before_observation` publishes twice before observation and checks the returned bytes, sequence, and absence of an invented producer-drop gap |
| Aliasing negative control | `the_retained_byte_oracle_rejects_an_overwriting_two_slot_ring` intentionally overwrites a two-slot ring and proves the retained-byte oracle rejects it |
| Callback admission fence, post-drain native-end decision, and retry after cancellation | `callback_fence_is_retryable_after_cancelled_drain`, `native_end_state_is_sampled_after_the_admitted_callback_drain`, and `authoritative_native_end_latches_after_owner_admission_stops` |
| Bounded implicit teardown and initialized cross-thread apartment | `teardown_executor_starts_only_the_fixed_worker_count`, `cancelled_waiters_share_one_in_flight_teardown_generation`, `teardown_queue_is_finite_and_non_blocking`, `teardown_permits_bound_live_and_queued_session_ownership`, `teardown_start_observes_operation_cancellation_before_spawning`, `uninterruptible_drop_drain_waits_for_an_admitted_callback`, both injected startup-failure tests, and native cross-thread close and implicit-drop paths |
| Lazy mapping, exact byte length, retained byte lease, device-terminal commit fence, retained lifetime | `r1_2_mapping_budget_follows_pixels_that_outlive_the_session_owner`, `device_terminal_cancels_a_cache_assignment_before_it_becomes_visible`, `crates/automation/capture/tests/native_storage.rs`, and the synthetic-window native test |
| Resize identity and old-storage survival | `a_native_frame_correlates_to_the_geometry_it_was_captured_under` plus the synthetic-window native test |
| Precalibrated WGC time and signed mixed-DPI geometry | `native_frame_times_use_the_precalibrated_project_clock`, `signed_mixed_dpi_placement_preserves_virtual_screen_coordinates`, `differently_scaled_adjacent_monitors_share_one_desktop_seam`, and the synthetic-window movement case |
| Access, capture-item close, disconnect, removal, and reset classification | `native_target_faults_are_normalized_by_target_kind`, `native_device_and_lifecycle_errors_are_typed`, and the controlled target-close path |
| Idempotent native close and post-close mapping | `native_close_absorbs_an_already_closed_result` and `synthetic_window_exercises_retention_resize_loss_and_close` |
| Optional-export System32 loader boundary | `optional_modules_resolve_only_from_the_system_directory` verifies module paths, while `optional_windows_exports_are_absent_from_the_pe_import_table` parses the built test executable's PE imports |

These tests are deterministic except for the explicitly native synthetic-window
case, which skips with a reason only when WGC is unavailable. On a supported
host, failure to discover the test-owned target is a test failure. The tests do
not capture an unrelated desktop or application.

[ADR 0031](adr/0031-windows-1280-production-capture-performance-budgets.md)
accepts the revision-bound 120-warm-up/600-frame 1280×720 production matrix.
[ADR 0032](adr/0032-windows-dual-4k-production-capture-performance-budgets.md)
accepts the exact two-display mixed-DPI dual-4K matrix. Both retain
resource-zeroing observations and affected `G-013` budgets; a skip remains
ineligible as support evidence.

## Privacy review

Production code emits no target, frame, or pixel logs. Public discovery returns
the window title or display name because it is caller-visible selection
metadata, but debug output for the provider, target records, sessions, storage,
and refusals excludes titles, native handles, process paths, pixel bytes,
captured hashes, and native serial identifiers. Native test skip messages and
assertions name only the test-owned fixture and typed outcomes.

## Adapter-independent contract cases

| Case | Required observable result |
|---|---|
| Producer slots are returned before publication | Retaining more public frames than the producer-pool size does not retain a producer token or prevent the fake producer from delivering the complete sample |
| Detached owner is authoritative | A published frame owns private detached storage and contains no WGC-frame or producer-surface owner |
| Lease-safe reuse | A texture is never selected for a copy while a public frame, mapping, or backend lease owns it |
| Finite pressure | Holding every detached texture makes the next candidate an observable queue/capture drop; it does not block, overwrite, or allocate beyond the configured bound |
| Resume after release | Releasing one old lease makes capacity available and producer progress resumes without reopening the session |
| Lazy mapping boundary | Mapping begins outside the producer callback, uses the frame's own descriptor and geometry revision, and preserves exact row pitch and bytes |
| Backend lifetime | Backend work admitted before close observes the complete original frame after session and pool closure |
| Callback boundary | Test hooks prove the callback performs only acquire, validate, copy, account, enqueue, and release; mapping, waits, backends, and host callbacks fail the test if invoked there |
| Resize transition | The first changed-content-size frame is not published; the new pool uses the new size and advances geometry revision once |
| Old resize generation | Retained old-size frames and mappings complete under their old revision while unused incompatible textures retire |
| Pressure before resize | A discontinuous publication starts its new epoch at `FIRST` and therefore preserves all pending pressure debt; repeated discontinuities preserve it, and the first later non-discontinuous publication consumes it only by committing the checked same-epoch sequence gap |
| Close admission fence | Both native handlers use lifetime-independent shared state; owner detachment and admission are synchronized; no callback is admitted to the owner after the fence; a delegate deliberately paused before admission is rejected safely after close |
| Idempotent close | Concurrent and repeated close calls converge on one terminal state without double release or a host callback under a lock; native teardown runs on a fixed shared worker pool whose apartments and global ownership permits are established before open, startup failures are typed and retryable, and explicit close remains bounded by its operation deadline |
| Target loss | Loss stops admission, reports the typed terminal outcome, and does not mutate retained frames |
| Device removal/reset | Admission stops, the typed device outcome terminates the session, no late callback publishes or mapping state commits, and teardown remains bounded |
| Resource bound | WGC frames never exceed two; a surface stays within 16,384 per axis and 128 MiB; detached, staging, and mapped ownership stays within 2 GiB per session and 4 GiB process-wide; the extent-derived retained count is enforced as a session-local maximum capped at 40 and is publicly marked process-shared; global contention may refuse earlier and every byte lease returns after final release |
| Diagnostic redaction | Drop, close, reset, and mapping failures exclude pixel bytes, captured hashes, recognized text, titles, process paths, and native serial identifiers |

The finite-pressure test must include an unsafe test Adapter that overwrites a
two-texture ring. The retained digest oracle must fail against that Adapter so a
test that can no longer detect aliasing cannot silently pass the production
implementation.

## Native Windows contract cases

Native tests use a test-owned synthetic Win32 target and a free-threaded WGC
frame pool. They never capture an unrelated desktop or application window.

The retained-frame case matches the G-002 evidence:

1. Create a two-frame WGC pool and a finite detached-texture pool.
2. Discard 120 delivered warm-up frames.
3. Require 600 delivered frames within 20 seconds after warm-up, with no
   post-warm-up arrival gap above 500 ms.
4. Retain every third frame, delay validation by 90 delivered frames, alternate
   mapping and backend leases, and cap retained work at 40.
5. Validate BGRA8 format, dimensions, `RowPitch >= width * 4`, deterministic
   marker coherence, wrap-aware sequence progress with no regression or
   greater-than-500 ms stall, and identical immediate and delayed digests.
6. Stop admission, unregister both handlers, drain admitted callbacks, publish
   the fence, close WGC, complete in-flight work, and require every resource
   counter to reach zero.

The native lifecycle suite adds:

- resize through 1280×720, 960×540, 1920×1080, and back to 1280×720;
- 100 close races at 0, 1, 5, 15, and 30 milliseconds after callback
  admission, 20 repetitions each;
- close with mapping and backend leases in flight;
- a deterministic pre-admission barrier that holds one queued delegate while
  close detaches the owner and publishes the fence, then proves safe rejection;
- close from a fresh thread whose WinRT apartment is initialized by the Adapter;
- retention past the two-frame producer-pool depth and finite 40-texture
  pressure, followed by release and resumed production; if resize intervenes,
  its `FIRST` preserves the debt and the next non-discontinuous publication
  exposes the gap;
- several successfully admitted sessions contending under the production 4 GiB
  shared budget, with each description reporting a process-shared local maximum,
  exact refusal, release/resume, and the first legal non-discontinuous sequence
  gap;
- movement of the synthetic target followed by a stable frame with a newer
  geometry revision and no spurious stream-epoch reset;
- repeated stale-target `TargetLost` under the finite generation lease, and a
  same-title replacement receiving a new `TargetId` without old-item retargeting;
- a real controlled-target close and an idempotent second close;
- injected device-loss admission stop followed by a typed terminal outcome, no
  late callback publication or mapping-state commit, and bounded teardown.

Injected device loss verifies state-machine ownership only. A physical device
removal, TDR, or driver upgrade is not claimed unless a separately reviewed
native run actually performs it.

Outside 08:00–19:00 local time, the named host also runs 600 frames on each of
its two 3840×2160 displays and 300 frames while moving the controlled target
across their signed-coordinate seam. The record includes Windows build, GPU and
driver, MSVC, SDK, DPI, scale, signed rectangles, and hashed display
identifiers, with no captured payload.

## Performance obligations

G-002 chooses correctness and ownership, not numeric product budgets. ADR 0031
accepts the 1280×720 `G-013` profiles for:

- capture arrival and callback-copy p50/p95 latency;
- full-frame copied bytes and lazy mapped bytes;
- detached-texture, staging, process-resident, and total GPU-resource peaks;
- producer progress, stale/coalesced work, and recovery after pressure;
- session startup, resize recreation, callback drain, complete close, and
  target-loss replacement recovery.

ADR 0032 accepts the same applicable capture, mapping, resource, progress, and
cleanup facts for the exact two-display 4K topology. It requires one shared
600-sample stationary pass per display plus 300 retained frame pairs while the
fixture moves across the signed seam. Both requested-position markers and each
frame's coherent post-baseline stream/epoch/sequence callback record must match.

Every timed sample keeps its correctness oracle. A throughput improvement that
changes retained pixels, pins producer slots, exceeds a bound, or hides a drop
fails before latency is considered.

## Execution and evidence

Adapter-independent cases run in ordinary workspace tests. Native WGC cases may
require an interactive Windows session and therefore may not be available on a
headless pull-request runner. A skipped native case must report why it did not
run; a skip is not support evidence.

Windows native production-capture acceptance is complete on repaired source.
The revision-bound reports link every applicable case above to its test, retain
accepted and rejected attempts, bind exact source/artifact identities and
approved metadata, enforce the affected `G-013` budgets, and pass the shared and
native matrices:

- [`windows-production-capture.md`](evidence/phase-2-native/windows-production-capture.md);
- [`windows-dual-4k-production-capture.md`](evidence/phase-2-native/windows-dual-4k-production-capture.md).

[docs/architecture.md](architecture.md) records both accepted production
lineages separately from Windows exact-candidate Phase 1 evidence, Apple Silicon
Phase 1 reviewed applicability, and release evidence.
