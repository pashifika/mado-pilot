# Windows capture ownership contract tests

This suite verifies the Windows Graphics Capture (`WGC`) and Direct3D 11 (`D3D11`)
ownership contract settled under [`G-002`](validation-gates.md#g-002) and defined
in [Windows native capture ownership](architecture.md#windows-native-capture-ownership)
for `mado-pilot-platform-windows`. It combines deterministic adapter-independent
contract tests over controllable doubles with native WGC/D3D11 tests on the
Windows verification host while keeping `mado-pilot-core`, `mado-pilot-capture`,
the facade, and the C ABI platform-neutral.

## Current implementation coverage

| Concern | Verifying tests and modules |
|---|---|
| Picker-free discovery, staged final arbitration, deterministic query-order commit, provider identity, required format | `r1_1_deadline_at_final_arbitration_leaves_all_registry_state_unchanged`, `discovery_snapshots_commit_in_query_order`, `crates/platform/windows/src/provider.rs` unit tests, and `crates/platform/windows/tests/native_capture.rs` |
| Retained-result commitment after native loss | `retained_commit_rechecks_native_key_without_closed_callback` and `retained_commit_liveness_preserves_completed_close` |
| Replacement identity and bounded current/previous generations | `r1_4_identical_raw_identity_with_a_fresh_lifetime_never_retargets_the_old_id`, `an_absent_identity_accepted_by_this_provider_is_conservatively_lost`, and the synthetic replacement-window path |
| Producer detachment, finite pressure, lease-safe reuse, resize retirement | `crates/automation/capture/tests/native_storage.rs` and `crates/platform/windows/src/storage.rs` unit tests |
| Checked surface (`16,384` px, `128 MiB`) and retained-byte (`2 GiB` session / `4 GiB` process, `<= 40` frames) bounds | `r1_2_surface_axes_and_bytes_accept_the_exact_boundary_and_reject_one_over`, `r1_2_surface_and_mapping_multiplication_overflow_is_typed_before_allocation`, `r1_2_production_limits_admit_the_exact_4k_retention_and_mapping_workload`, `r1_2_reported_capacity_is_truthful_for_4k_and_reduced_for_8k`, `r1_2_derived_4k_capacity_admits_first_and_fortieth_then_refuses_forty_one`, `r1_2_derived_8k_capacity_admits_the_first_and_twelfth_then_refuses_thirteen`, `r1_2_production_session_and_global_limits_are_exact_and_shared`, `r1_2_production_multi_session_description_matches_shared_pressure_and_resume`, `r1_2_mapping_budget_follows_pixels_that_outlive_the_session_owner`, and `r1_2_process_shared_mapping_stays_charged_after_session_close_until_pixels_drop` |
| Pressure/discontinuity precedence | `finite_path_drop_debt_survives_discontinuities_until_a_gap_can_represent_it`, `unrepresentable_drop_debt_neither_wraps_nor_disappears_at_a_discontinuity`, and `r1_2_production_multi_session_description_matches_shared_pressure_and_resume` |
| Producer reservation and native teardown lifetime | `r1_2_producer_reservation_stays_with_queued_native_ownership_until_close`, `r1_2_producer_reservation_stays_charged_in_quarantine`, and `r1_2_resize_replaces_producer_reservation_only_after_native_success` |
| Capacity-one `LatestWins` handoff & aliasing negative control | `r1_3_declared_latest_wins_matches_two_publications_before_observation` and `the_retained_byte_oracle_rejects_an_overwriting_two_slot_ring` |
| Callback admission fence, post-drain native-end decision, and retry after cancellation | `callback_fence_is_retryable_after_cancelled_drain`, `native_end_state_is_sampled_after_the_admitted_callback_drain`, and `authoritative_native_end_latches_after_owner_admission_stops` |
| Bounded implicit teardown and initialized cross-thread WinRT apartment | `teardown_executor_starts_only_the_fixed_worker_count`, `cancelled_waiters_share_one_in_flight_teardown_generation`, `teardown_queue_is_finite_and_non_blocking`, `teardown_permits_bound_live_and_queued_session_ownership`, `teardown_start_observes_operation_cancellation_before_spawning`, `uninterruptible_drop_drain_waits_for_an_admitted_callback`, startup-failure tests, and native cross-thread close/drop paths |
| Lazy mapping, retained byte lease, and device-terminal commit fence | `r1_2_mapping_budget_follows_pixels_that_outlive_the_session_owner`, `device_terminal_cancels_a_cache_assignment_before_it_becomes_visible`, `crates/automation/capture/tests/native_storage.rs`, and the synthetic-window native test |
| Resize identity, precalibrated WGC clock, and signed mixed-DPI geometry | `a_native_frame_correlates_to_the_geometry_it_was_captured_under`, `native_frame_times_use_the_precalibrated_project_clock`, `signed_mixed_dpi_placement_preserves_virtual_screen_coordinates`, `differently_scaled_adjacent_monitors_share_one_desktop_seam`, and synthetic-window movement/resize tests |
| Target/device fault classification and idempotent close | `native_target_faults_are_normalized_by_target_kind`, `native_device_and_lifecycle_errors_are_typed`, `native_close_absorbs_an_already_closed_result`, and `synthetic_window_exercises_retention_resize_loss_and_close` |
| Optional-export `SYSTEM32` loader boundary | `optional_modules_resolve_only_from_the_system_directory` and `optional_windows_exports_are_absent_from_the_pe_import_table` |

## Privacy review

Production capture code emits no target, frame, or pixel logs, and `Debug`
implementations exclude window titles, native handles, process paths, pixel
bytes, hashes, and monitor serial identifiers (see
[Bounded diagnostic observation](architecture.md#bounded-diagnostic-observation)).
Native tests capture only their own synthetic Win32 fixture window.

## Adapter-independent contract cases

| Case | Required observable result |
|---|---|
| Producer slots returned before publication | Retaining more public frames than the 2-frame producer pool never stalls delivery |
| Detached owner is authoritative | Published frames own private detached storage with no `Direct3D11CaptureFrame` or producer-surface reference |
| Lease-safe reuse & aliasing negative control | Textures are reused only after all public-frame, mapping, and backend leases drop; an overwriting 2-slot ring double is rejected by the retained-digest oracle |
| Finite pressure & resume after release | Exhausting all detached textures records an observable sequence drop without blocking or allocating past the bound, and releasing one lease resumes delivery |
| Lazy mapping & backend lifetime | Mapping runs outside the WGC callback under the frame's descriptor/revision, and backend work admitted before close reads the intact original frame |
| Callback boundary | The callback performs only acquire, validate, copy, account, enqueue, and release |
| Resize transition & pressure debt | Clipped transition frames are dropped; the recreated pool advances geometry revision once, retained old-size frames stay readable, and pre-resize drop debt survives `FIRST` epochs until the next non-discontinuous publication commits the sequence gap |
| Close admission fence & idempotent close | `FrameArrived` and `Closed` handlers use lifetime-independent state; close detaches the owner, drains callbacks, and runs teardown on the fixed WinRT worker pool without double release or host callbacks under locks |
| Target loss & device removal/reset | Target loss and device reset stop admission and publish typed terminal faults without committing late mappings |
| Resource bounds | 2 WGC producer frames, `<= 16,384` px/axis, `<= 128 MiB`/surface, `<= 2 GiB`/session, `<= 4 GiB`/process (`ProcessShared`, capped at `40` frames), with all byte leases returned on final drop |

## Native Windows contract cases

On an interactive Windows host, the native suite exercises a test-owned synthetic
Win32 target:

1. **Retained-frame stress (`G-002` baseline):** 2-frame WGC pool + finite
   detached pool, 120 warm-up frames discarded, 600 frames delivered within 20 s
   (no gap `> 500 ms`), every 3rd frame retained for 90 frames across alternating
   mapping and backend leases (up to 40 retained), verifying `BGRA8`,
   `RowPitch >= width * 4`, marker coherence, identical immediate/delayed
   digests, and zeroed resource counters after close.
2. **Lifecycle and contention:** Resize across `1280×720 -> 960×540 -> 1920×1080 -> 1280×720`;
   100 close races (`0`, `1`, `5`, `15`, `30 ms` × 20); close with in-flight
   leases and pre-admission barriers; cross-thread close; multi-session contention
   under the `4 GiB` shared budget; target movement across signed seams; stale
   `TargetId` refusal and replacement isolation; and injected device-loss
   teardown.
3. **Dual-4K stationary and cross-seam movement:** 600 frames on each of two
   `3840×2160` displays plus 300 frames while moving the synthetic target across
   their signed-coordinate seam.

## Performance obligations

Every timed sample enforces its correctness and zero-leak oracles first. The
[`1280×720`](benchmarks/phase-2-production-capture-1280x720-x86_64-pc-windows-msvc.toml)
and [`dual-4K`](benchmarks/phase-2-production-capture-dual-4k-x86_64-pc-windows-msvc.toml)
profiles record capture arrival and callback-copy p50/p95 latency, copied/mapped
bytes, detached/staging/resident/GPU peaks, producer progress, and
startup/resize/close latencies.

## Execution and evidence

Adapter-independent cases run in every workspace `cargo test`. Native WGC cases
skip with an explicit reason when WGC or an interactive session is unavailable
(a skip is never support evidence). The accepted revision-bound Windows
production capture records live in:

- [`evidence/phase-2-native/windows-production-capture.md`](evidence/phase-2-native/windows-production-capture.md)
- [`evidence/phase-2-native/windows-dual-4k-production-capture.md`](evidence/phase-2-native/windows-dual-4k-production-capture.md)
