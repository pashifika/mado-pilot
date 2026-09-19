"""Synthetic protocol records are validator tests, never native workload evidence."""

from __future__ import annotations

import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "report.py"
SPEC = importlib.util.spec_from_file_location("capture_pacing_report", SCRIPT)
reporter = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(reporter)
ROOT = SCRIPT.parents[2]
CASES = ("capture-off", "baseline", "cooldown-only", "native-only", "combined")
NS = 1_000_000_000
MIB = 1024**2


def consumer(case="baseline", platform="windows"):
    off = case == "capture-off"
    native = case in ("native-only", "combined")
    samples = [] if off else [
        {"ocr_ns": NS // 2, "frame_age_ns": 20_000_000, "cooldown_gap_ns": None if index == 0 else 250_000_000,
         "stream_id": 1, "sequence": 100 + index * 5, "epoch": 1, "geometry_revision": 1}
        for index in range(3)
    ]
    policies = [] if off else [{"selection": "required" if native else "source-default",
                               "requested_ns": 100_000_000 if native else None,
                               "configured_ns": 100_000_000 if native else None,
                               "outcome": "applied" if native else "source-default"}]
    checks = {key: "pass" for key in (
        "owned_source", "source_identity", "pacing_report", "cooldown_spacing", "sample_capacity",
    )}
    if off:
        checks.update({key: "not-applicable" for key in (
            "owned_source", "source_identity", "pacing_report", "cooldown_spacing",
        )})
    if case == "semantic":
        policies = [
            {"selection": "source-default", "requested_ns": None, "configured_ns": None, "outcome": "source-default"},
            {"selection": "preferred", "requested_ns": 60_000_000, "configured_ns": 60_000_000, "outcome": "applied"},
            {"selection": "required", "requested_ns": 100_000_000, "configured_ns": 100_000_000, "outcome": "applied"},
            {"selection": "required", "requested_ns": 100_000_000, "configured_ns": 100_000_000, "outcome": "applied"},
        ]
        checks = {key: "pass" for key in (
            "source_default", "required_applied", "preferred_override", "owned_source",
            "completion_cooldown_live_idle", "burst_latest", "static_observations", "resize_retained_mapping",
            "deadline", "cancellation", "session_close",
        )}
        checks["target_close"] = "target-lost"
    gpu_reason = ("gpu-own-process-instances-unavailable" if platform == "windows" else
                  "per-process-gpu-unavailable-without-privileged-collector")
    process = {
        "cpu_ns": 3 * NS, "max_resident_bytes": 128 * MIB,
        "callback_copied_bytes": 600 * MIB if platform == "windows" and not off else None,
        "capture_metrics_reason": "capture-disabled" if off else
            "native-copy-bytes-not-exposed" if platform != "windows" else None,
        "platform": platform, "sampler_elapsed_ns": 6 * NS, "sample_count": 62, "missed_poll_deadlines": 0,
        "max_sample_gap_ns": 101_000_000, "cpu_reason": None, "resident_reason": None,
        "max_private_bytes": 100 * MIB if platform == "windows" else None,
        "max_footprint_bytes": 100 * MIB if platform == "macos" else None,
        "callback_invalid_intervals": 0,
        "native_detached_textures_peak": (0 if off else 3) if platform == "windows" else None,
        "native_staging_textures_peak": (0 if off else 1) if platform == "windows" else None,
        "gpu_engine_percent_mean": None, "gpu_engine_percent_max": None,
        "gpu_sample_count": 0, "gpu_sampled_ns": 0, "gpu_reason": gpu_reason,
    }
    return {"schema": 2, "case": case, "semantic_status": "pass", "cleanup_status": "pass", "reason": None,
            "permission": "granted" if platform == "macos" else "not-required", "startup_ns": NS,
            "pacing": policies, "checks": checks, "samples": samples,
            "metrics": {"measurement_ns": 6 * NS, "ocr_admissions": len(samples), "ocr_committed": len(samples),
                        "caller_mapped_bytes": 0 if off else 18 * MIB, "max_mapping_bytes": 0 if off else 6 * MIB,
                        "max_retained_layout_bytes": 0 if off else 6 * MIB,
                        "process": process, "gpu_reason": gpu_reason}}


def fixture():
    return {"schema": 1, "nonce": "0123456789abcdef", "pid": 123, "wall_ns": 6 * NS,
            "cpu_ns": NS // 10, "peak_rss_bytes": 32 * MIB, "paint_counter": 400,
            "render_intervals_ns": [16_000_000] * 374, "draw_durations_ns": [100_000] * 375,
            "lost_samples": 0}


def sck_log(*, publication=9000, callbacks=500):
    aggregate = {"event": "aggregate", "tier": 2, "closed_sessions": 1, "active_sessions": 0,
                 "callbacks_in_flight": 0, "detached_leases": 0, "detached_bytes": 0, "native_objects": 0,
                 "native_mask_schema": "stream:1,configuration:2,filter:4,output:8,queue:16,pool:32",
                 "error_domain_schema": "none:0,stream:1,other:2"}
    session = {
        "event": "session", "sequence": 1, "tier": 2, "close_phase": 4, "terminal": "closed",
        "close_error": "ok", "closed": 1, "start_nanos": NS, "start_status": "ok",
        "stop_requested_nanos": 10 * NS, "stop_completed_nanos": 10 * NS + 1, "stop_status": "ok",
        "stop_error": 0, "stop_error_domain": 0, "admission_stopped_nanos": 10 * NS,
        "callback_fenced_nanos": 10 * NS + 1, "release_completed_nanos": 10 * NS + 2,
        "close_completed_nanos": 10 * NS + 3, "publication_sequence": publication,
        "publication_nanos": 9 * NS, "publication_display_nanos": 9 * NS,
        "content": "960x576", "surface": "960x576", "callbacks_received": callbacks,
        "callbacks_admitted": callbacks - 2, "callbacks_refused": 2,
        "callbacks_entered": callbacks - 2, "callbacks_exited": callbacks - 2,
        "callback_accepting": 0, "callback_fenced": 1, "refs": 1, "native_before": 63, "native_after": 0,
        "detached_leases": 1, "detached_bytes": 3 * MIB, "process_native_objects": 0,
        "transition_count": 0, "transition_overflow": 0,
    }
    return "\n".join("benchmark-sck-diagnostics " + " ".join(f"{key}={value}" for key, value in row.items())
                     for row in (aggregate, session)) + "\n"


def record(case="baseline", platform="windows"):
    value = consumer(case, platform)
    stderr = sck_log() if platform == "macos" and case != "capture-off" else ""
    return {"case": case, "consumer": value, "fixture": fixture(), "stderr": stderr,
            "process": {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
                        "stdout": json.dumps(value), "stderr": stderr, "duration_seconds": 20.0,
                        "launch_error": None}}


def placeholder(case):
    return {"case": case, "consumer": None, "fixture": None, "stderr": "", "process": None}


def permission_refusal(case="semantic"):
    value = record(case, "macos")
    value["consumer"] = consumer("capture-off", "macos")
    value["consumer"].update(
        case=case, semantic_status="not-run", reason="permission-denied-or-undetermined",
        permission="denied-or-undetermined", startup_ns=None, pacing=[],
        checks={"capture_cleanup": "pass", "fixture_cleanup": "pass"},
    )
    value["consumer"]["metrics"].update(measurement_ns=0, process=None)
    value["fixture"] = None
    value["stderr"] = ""
    value["process"].update(exit_code=2, duration_seconds=0.02)
    synchronize(value)
    return value


def synchronize(value):
    value["process"]["stdout"] = json.dumps(value["consumer"])
    value["process"]["stderr"] = value["stderr"]


class CaseValidationTests(unittest.TestCase):
    def test_padded_native_mapping_uses_descriptor_budget(self):
        value = consumer()
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertGreater(result["metrics"]["max_mapping_bytes"], 1040 * 640 * 4)
        value["metrics"]["max_mapping_bytes"] = 8 * MIB + 1
        value["metrics"]["max_retained_layout_bytes"] = 8 * MIB + 1
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "fail")
        self.assertIn("mapping-budget", result["failures"])

    def test_cooldown_and_required_policy_reject_shortening(self):
        value = consumer("combined")
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "pass")
        value["samples"][1]["cooldown_gap_ns"] = 249_999_999
        value["pacing"][0]["configured_ns"] = 99_999_999
        result = reporter.analyze_case(value, fixture(), "")
        self.assertIn("completion-cooldown", result["failures"])
        self.assertIn("policy-required", result["failures"])

    def test_preferred_unapplied_is_not_required_success(self):
        semantic = consumer("semantic")
        semantic["pacing"][1].update(outcome="preferred-unapplied", configured_ns=None)
        result = reporter.analyze_case(semantic, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertIn("preferred-native-interval-applied", result["withheld_claims"])
        required = consumer("native-only")
        required["pacing"][0].update(outcome="preferred-unapplied", configured_ns=None)
        self.assertIn("policy-required", reporter.analyze_case(required, fixture(), "")["failures"])

    def test_unknown_outcome_and_missing_semantic_rule_do_not_pass(self):
        value = consumer("semantic")
        del value["checks"]["cancellation"]
        self.assertIn("check-cancellation", reporter.analyze_case(value, fixture(), "")["failures"])
        value = consumer()
        value["pacing"][0]["outcome"] = "unknown"
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "fail")

    def test_native_quiescence_is_platform_scoped_and_not_target_loss(self):
        value = consumer("semantic", "macos")
        value["checks"]["target_close"] = "quiescent-deadline"
        result = reporter.analyze_case(value, fixture(), sck_log())
        self.assertEqual(result["status"], "pass")
        self.assertIn("quiescent-target-lost-notification", result["withheld_claims"])
        value = consumer("semantic")
        value["checks"]["target_close"] = "quiescent-deadline"
        self.assertIn("check-target_close", reporter.analyze_case(value, fixture(), "")["failures"])

    def test_nonexposed_copy_gpu_and_future_age_remain_unavailable(self):
        value = consumer(platform="macos")
        for sample in value["samples"]:
            sample["frame_age_ns"] = None
        result = reporter.analyze_case(value, fixture(), sck_log())
        self.assertEqual(result["status"], "pass")
        self.assertIsNone(result["metrics"]["callback_copy_bytes_per_second"])
        self.assertIsNone(result["metrics"]["gpu_engine_percent_mean"])
        self.assertIsNone(result["metrics"]["frame_age"]["p95_ns"])
        self.assertEqual(result["metrics"]["frame_age_unavailable_samples"], 3)
        self.assertIn("native-copy-byte-savings", result["withheld_claims"])
        value["metrics"]["process"]["callback_copied_bytes"] = 0
        value["metrics"]["process"]["capture_metrics_reason"] = None
        self.assertEqual(reporter.analyze_case(value, fixture(), sck_log())["status"], "fail")

    def test_lost_copy_interval_invalidates_whole_total_not_ring_reuse(self):
        value = consumer()
        value["samples"][-1]["sequence"] = 100_000
        complete = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(complete["status"], "pass")
        self.assertIsNone(complete["metrics"]["native_callbacks_per_second"])
        value["metrics"]["process"].update(callback_invalid_intervals=1, callback_copied_bytes=None,
                                            capture_metrics_reason="callback-copy-interval-invalidated")
        lost = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(lost["status"], "pass")
        self.assertIsNone(lost["metrics"]["callback_copy_bytes_per_second"])
        self.assertEqual(lost["metrics"]["callback_invalid_intervals"], 1)
        self.assertIn("native-copy-byte-savings", lost["withheld_claims"])
        value["metrics"]["process"].update(callback_copied_bytes=1, capture_metrics_reason=None)
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "fail")

    def test_incomplete_memory_cannot_prove_safety_but_missing_cpu_only_withholds_cpu(self):
        value = consumer()
        value["metrics"]["process"].update(cpu_ns=None, cpu_reason="process-cpu-observation-unavailable")
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertIn("consumer-cpu-savings", result["withheld_claims"])
        value["metrics"]["process"].update(max_resident_bytes=None, resident_reason="process-resident-observation-incomplete")
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "incomplete")
        self.assertIn("process-memory-budget", result["unavailable"])
        self.assertEqual(reporter.analyze_case(consumer(), None, "")["status"], "incomplete")

    def test_exact_memory_ceiling_and_latency_sample_gates(self):
        value, producer = consumer(), fixture()
        value["metrics"]["process"]["max_resident_bytes"] = 2 * 1024**3
        producer["peak_rss_bytes"] = 256 * MIB
        self.assertEqual(reporter.analyze_case(value, producer, "")["status"], "pass")
        value["metrics"]["process"]["max_resident_bytes"] += 1
        producer["peak_rss_bytes"] += 1
        value["samples"][-1]["ocr_ns"] = 5 * NS + 1
        result = reporter.analyze_case(value, producer, "")
        for gate in ("process-memory-budget", "fixture-memory-budget", "ocr-p95-budget"):
            self.assertIn(gate, result["failures"])
        value = consumer()
        value["samples"].pop()
        value["metrics"].update(ocr_admissions=2, ocr_committed=2)
        self.assertIn("ocr-sample-minimum", reporter.analyze_case(value, fixture(), "")["failures"])

    def test_identity_order_is_per_stream_and_reopened_policy_is_revalidated(self):
        value = consumer("semantic")
        value["samples"][1].update(stream_id=2, sequence=1)
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["metrics"]["publication_sequence_gaps"], 9)
        value["pacing"][2]["configured_ns"] = 99_999_999
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["gates"]["policy-required"], "fail")
        value["samples"][-1]["sequence"] = 99
        self.assertIn("publication-stamp-order", reporter.analyze_case(value, fixture(), "")["failures"])

    def test_zero_based_frame_sequences_and_epoch_restart_remain_ordered(self):
        value = consumer("semantic")
        for sample, epoch, sequence in zip(value["samples"], (0, 0, 1), (0, 1, 0)):
            sample.update(epoch=epoch, sequence=sequence, geometry_revision=epoch)
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["metrics"]["publication_sequence_gaps"], 0)
        value["samples"][1]["sequence"] = 0
        self.assertIn("publication-stamp-order", reporter.analyze_case(value, fixture(), "")["failures"])

    def test_semantic_requires_both_required_reports_and_validates_each(self):
        missing = consumer("semantic")
        missing["pacing"].pop()
        self.assertIn("pacing-selections", reporter.analyze_case(missing, fixture(), "")["failures"])
        for index in (2, 3):
            with self.subTest(required_row=index):
                value = consumer("semantic")
                value["pacing"][index]["configured_ns"] = 99_999_999
                result = reporter.analyze_case(value, fixture(), "")
                self.assertEqual(result["status"], "fail")
                self.assertEqual(result["gates"]["policy-required"], "fail")

    def test_failed_ocr_attempt_samples_remain_measured_failure_evidence(self):
        value = consumer()
        value.update(semantic_status="fail", reason="ocr-failed")
        value["metrics"]["ocr_committed"] = 2
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "fail")
        self.assertEqual(result["metrics"]["ocr_committed"], 2)
        self.assertEqual(result["metrics"]["ocr_admissions"], 3)
        self.assertEqual(result["metrics"]["ocr_attempt_samples"], 3)
        self.assertEqual(result["metrics"]["ocr_latency"]["p95_ns"], NS // 2)
        self.assertEqual(result["metrics"]["measurement_ns"], 6 * NS)
        failed = record()
        failed["consumer"] = copy.deepcopy(value)
        failed["process"]["exit_code"] = 1
        synchronize(failed)
        campaign = reporter.compare_cases(
            [record("capture-off"), failed] + [placeholder(case) for case in CASES[2:]])
        self.assertEqual(campaign["status"], "fail")
        self.assertEqual(campaign["cases"][1]["metrics"]["ocr_attempt_samples"], 3)
        self.assertEqual(campaign["cases"][1]["metrics"]["ocr_committed"], 2)
        value.update(semantic_status="pass", reason=None)
        self.assertIn("committed-admissions", reporter.analyze_case(value, fixture(), "")["failures"])

    def test_admissions_and_stamp_order_cannot_hide_incomplete_ocr(self):
        value = consumer()
        value["metrics"]["ocr_admissions"] += 1
        value["samples"][1]["sequence"] = value["samples"][0]["sequence"]
        result = reporter.analyze_case(value, fixture(), "")
        self.assertIn("committed-admissions", result["failures"])
        self.assertIn("publication-stamp-order", result["failures"])
        value["metrics"]["ocr_committed"] += 1
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "fail")

    def test_sampler_and_fixture_loss_or_missing_measurement_fail(self):
        value, producer = consumer(), fixture()
        value["metrics"]["process"]["sample_count"] = 1
        producer["lost_samples"] = 1
        value["metrics"]["measurement_ns"] = 0
        result = reporter.analyze_case(value, producer, "")
        for gate in ("process-samples", "fixture-sample-loss", "measurement-wall"):
            self.assertIn(gate, result["failures"])
        value = consumer()
        value["metrics"]["process"]["sample_count"] = 2
        self.assertIn("sampler-cadence", reporter.analyze_case(value, fixture(), "")["failures"])

    def test_delayed_poll_keeps_cumulative_evidence_without_claiming_ideal_resolution(self):
        value = consumer()
        process = value["metrics"]["process"]
        process.update(sample_count=61, missed_poll_deadlines=1, max_sample_gap_ns=241_741_500)
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["metrics"]["process_sample_count"], 61)
        self.assertEqual(result["metrics"]["missed_poll_deadlines"], 1)
        self.assertEqual(result["metrics"]["max_sample_gap_ns"], 241_741_500)
        self.assertEqual(result["metrics"]["process_peak_rss_bytes"], 128 * MIB)
        self.assertEqual(result["metrics"]["process_cpu_cores"], 0.5)
        self.assertEqual(result["metrics"]["callback_copied_bytes"], 600 * MIB)
        self.assertIn("ideal-100ms-sampling-resolution", result["withheld_claims"])
        self.assertIn("continuous-private-memory-peak-savings", result["withheld_claims"])
        self.assertIn("instantaneous-gpu-engine-peak-savings", result["withheld_claims"])

    def test_poll_delay_cannot_hide_read_failures_copy_invalidation_or_capacity(self):
        value = consumer()
        process = value["metrics"]["process"]
        process.update(sample_count=61, missed_poll_deadlines=1, max_sample_gap_ns=241_741_500,
                       callback_invalid_intervals=1, callback_copied_bytes=None,
                       capture_metrics_reason="callback-copy-interval-invalidated")
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertIsNone(result["metrics"]["callback_copied_bytes"])
        self.assertEqual(result["metrics"]["callback_invalid_intervals"], 1)
        self.assertIn("native-copy-byte-savings", result["withheld_claims"])
        process.update(max_resident_bytes=None, resident_reason="process-resident-observation-incomplete")
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "incomplete")
        self.assertIn("process-memory-budget", result["unavailable"])
        process["sample_count"] = 2049
        self.assertIn("process-sample-cap-exceeded", reporter.analyze_case(value, fixture(), "")["failures"])

    def test_v1_poll_loss_evidence_requires_its_bound_reporter_without_mutation(self):
        legacy = consumer()
        legacy.update(schema=1, semantic_status="fail", reason="owned-pixel-color")
        process = legacy["metrics"]["process"]
        del process["missed_poll_deadlines"]
        process.update(sample_losses=1, max_sample_gap_ns=241_741_500)
        before = copy.deepcopy(legacy)
        result = reporter.analyze_case(legacy, fixture(), "")
        self.assertEqual(result["status"], "fail")
        self.assertIn("consumer-v1-evidence-requires-bound-reporter", result["failures"])
        self.assertEqual(legacy, before)
        legacy["schema"] = 2
        result = reporter.analyze_case(legacy, fixture(), "")
        self.assertEqual(result["status"], "fail")
        self.assertIn("legacy-sampler-accounting-requires-bound-reporter", result["failures"])

    def test_malformed_records_are_bounded_and_do_not_echo_payloads(self):
        bad_values = (True, -1, 1 << 64, float("nan"), "private-recognized-text")
        for bad in bad_values:
            with self.subTest(type=type(bad).__name__):
                value = consumer()
                value["metrics"]["ocr_admissions"] = bad
                result = reporter.analyze_case(value, fixture(), "")
                self.assertEqual(result["status"], "fail")
                self.assertNotIn("private-recognized-text", json.dumps(result))
        value = consumer()
        del value["cleanup_status"]
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "fail")
        value = consumer()
        value["samples"] *= 342
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "fail")
        producer = fixture()
        producer["draw_durations_ns"] = [1] * 4097
        self.assertEqual(reporter.analyze_case(consumer(), producer, "")["status"], "fail")

    def test_gpu_engine_samples_are_not_device_utilization_or_fabricated_zero(self):
        value = consumer()
        value["metrics"]["process"].update(gpu_engine_percent_mean=125.0, gpu_engine_percent_max=170.0,
                                            gpu_sample_count=60, gpu_sampled_ns=6 * NS, gpu_reason=None)
        value["metrics"]["gpu_reason"] = None
        result = reporter.analyze_case(value, fixture(), "")
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["metrics"]["gpu_engine_percent_mean"], 125.0)
        self.assertIsNone(result["metrics"]["device_gpu_utilization"])
        self.assertNotIn("own-process-gpu-engine-savings", result["withheld_claims"])
        value["metrics"]["process"]["gpu_sample_count"] = 0
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "fail")

    def test_capture_off_cannot_contain_capture_observations(self):
        value = consumer("capture-off")
        self.assertEqual(reporter.analyze_case(value, fixture(), "")["status"], "pass")
        value["checks"]["sample_capacity"] = "not-applicable"
        self.assertIn("check-sample_capacity", reporter.analyze_case(value, fixture(), "")["failures"])
        value["checks"]["sample_capacity"] = "pass"
        value["samples"] = consumer()["samples"]
        value["metrics"].update(ocr_committed=3, ocr_admissions=3)
        self.assertIn("capture-off-no-capture-work", reporter.analyze_case(value, fixture(), "")["failures"])


class NativeDiagnosticsTests(unittest.TestCase):
    def test_closed_session_callback_count_is_not_publication_or_six_second_rate(self):
        result = reporter.analyze_case(consumer(platform="macos"), fixture(), sck_log(publication=9000, callbacks=500))
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["native"]["callbacks_received"], 500)
        self.assertIsNone(result["native"]["measurement_callback_rate"])
        self.assertIsNone(result["metrics"]["native_callbacks_per_second"])
        self.assertEqual(result["native"]["scope"], "closed-session-lifetime-including-warmup")

    def test_rolling_status_tail_preserves_lifetime_evidence_without_full_history_claim(self):
        log = sck_log().replace("transition_count=0", "transition_count=16").replace(
            "transition_overflow=0", "transition_overflow=1")
        for index in range(16):
            row = {"event": "transition", "session_sequence": 1, "index": index,
                   "raw": index % 2, "normalized": index % 2,
                   "normalized_name": "idle" if index % 2 else "complete",
                   "status_sequence": 101 + index, "monotonic_nanos": NS + index}
            log += "benchmark-sck-diagnostics " + " ".join(f"{key}={value}" for key, value in row.items()) + "\n"
        result = reporter.analyze_case(consumer(platform="macos"), fixture(), log)
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["native"]["callbacks_received"], 500)
        self.assertFalse(result["native"]["status_history_complete"])
        self.assertEqual(result["native"]["status_history_truncated_sessions"], 1)
        self.assertIn("complete-native-status-history", result["withheld_claims"])
        missing = "\n".join(log.splitlines()[:-1]) + "\n"
        self.assertIn("native-transition-loss", reporter.analyze_case(
            consumer(platform="macos"), fixture(), missing)["failures"])

    def test_incomplete_or_overwritten_diagnostics_do_not_pass(self):
        logs = (
            sck_log().splitlines()[1] + "\n",
            sck_log().splitlines()[0] + "\n",
            sck_log().replace("event=session sequence=1", "event=session sequence=17"),
            sck_log().replace("transition_count=0", "transition_count=1"),
            sck_log().replace("transition_overflow=0", "transition_overflow=1"),
            sck_log() + sck_log().splitlines()[0] + "\n",
        )
        for log in logs:
            with self.subTest(log_index=logs.index(log)):
                self.assertEqual(reporter.analyze_case(consumer(platform="macos"), fixture(), log)["status"], "fail")

    def test_retained_snapshot_is_not_a_final_leak_but_live_aggregate_is(self):
        self.assertEqual(reporter.analyze_case(consumer(platform="macos"), fixture(), sck_log())["status"], "pass")
        log = sck_log().replace("detached_leases=0 detached_bytes=0", "detached_leases=1 detached_bytes=3145728", 1)
        result = reporter.analyze_case(consumer(platform="macos"), fixture(), log)
        self.assertEqual(result["status"], "fail")
        self.assertIn("native-owners-not-released", result["failures"])


class NotRunRecordTests(unittest.TestCase):
    def test_strict_placeholders_remain_present_without_observations(self):
        records = [placeholder(case) for case in CASES]
        for value in records:
            value["analysis"] = {"status": "pass", "metrics": {"private": "untrusted-cached-payload"}}
        result = reporter.compare_cases(records)
        self.assertEqual(result["status"], "not-run")
        self.assertEqual([row["case"] for row in result["cases"]], list(CASES))
        self.assertEqual(result["failures"], [])
        self.assertTrue(all(row["status"] == "not-run" and row["metrics"] == {} for row in result["cases"]))
        self.assertNotIn("untrusted-cached-payload", json.dumps(result))

    def test_placeholder_shape_cannot_hide_an_attempt_or_evidence(self):
        mutations = ({"fixture": fixture()}, {"process": {}}, {"stderr": "\\n"},
                     {"stderr": None}, {"consumer": {}}, {"extra": None})
        for mutation in mutations:
            with self.subTest(keys=tuple(mutation)):
                records = [placeholder(case) for case in CASES]
                records[0].update(mutation, analysis={"status": "not-run"})
                self.assertEqual(reporter.compare_cases(records)["status"], "fail")

    def test_permission_refusal_and_following_placeholders_stay_not_run(self):
        records = [permission_refusal("capture-off")] + [placeholder(case) for case in CASES[1:]]
        result = reporter.compare_cases(records)
        self.assertEqual(result["status"], "not-run")
        self.assertEqual(result["failures"], [])
        self.assertEqual(result["cases"][0]["reason"], "permission-denied-or-undetermined")
        self.assertEqual([row["case"] for row in result["cases"]], list(CASES))

    def test_permission_record_cannot_hide_signal_or_exception_exit(self):
        for exit_code in (-9, 3, 0xC0000005):
            with self.subTest(exit_code=exit_code):
                records = [permission_refusal("capture-off")] + [placeholder(case) for case in CASES[1:]]
                records[0]["process"]["exit_code"] = exit_code
                result = reporter.compare_cases(records)
                self.assertEqual(result["status"], "fail")
                self.assertIn("runner-nonzero-exit", result["cases"][0]["failures"])

    def test_partial_campaign_distinguishes_missing_evidence_from_failure(self):
        records = [record("capture-off")] + [placeholder(case) for case in CASES[1:]]
        self.assertEqual(reporter.compare_cases(records)["status"], "incomplete")
        records[0]["fixture"]["peak_rss_bytes"] = None
        result = reporter.compare_cases(records)
        self.assertEqual(result["status"], "incomplete")
        self.assertEqual(result["cases"][0]["status"], "incomplete")
        records[0]["process"]["cleanup_ok"] = False
        self.assertEqual(reporter.compare_cases(records)["status"], "fail")


class CampaignTests(unittest.TestCase):
    def test_five_case_rates_use_three_actual_elapsed_scopes(self):
        records = [record(case) for case in CASES]
        combined = records[-1]
        combined["consumer"]["metrics"]["measurement_ns"] = 12 * NS
        combined["consumer"]["metrics"]["process"].update(sampler_elapsed_ns=10 * NS, sample_count=102)
        combined["fixture"]["wall_ns"] = 15 * NS
        synchronize(combined)
        result = reporter.compare_cases(records)
        self.assertEqual(result["status"], "pass")
        comparison = next(row for row in result["comparisons"] if row["case"] == "combined")
        self.assertEqual(comparison["rates"]["ocr_per_second"]["ratio"], 0.5)
        self.assertAlmostEqual(comparison["rates"]["process_cpu_cores"]["ratio"], 0.6)
        self.assertAlmostEqual(comparison["rates"]["callback_copy_bytes_per_second"]["ratio"], 0.6)
        self.assertAlmostEqual(comparison["rates"]["fixture_render_starts_per_second"]["ratio"], 0.4)
        off = next(row for row in result["comparisons"] if row["case"] == "capture-off")
        self.assertIsNone(off["rates"]["callback_copy_bytes_per_second"]["ratio"])

    def test_missing_duplicate_and_cross_platform_cases_are_not_comparable(self):
        records = [record(case) for case in CASES]
        self.assertIn("comparison-cases-missing", reporter.compare_cases(records[:-1])["failures"])
        self.assertIn("case-record-duplicate", reporter.compare_cases(records + [copy.deepcopy(records[0])])["failures"])
        records[-1] = record("combined", "macos")
        self.assertIn("comparison-platform-mismatch", reporter.compare_cases(records)["failures"])

    def test_cached_analysis_cannot_override_raw_failure(self):
        records = [record(case) for case in CASES]
        for value in records:
            value["analysis"] = {"status": "pass", "private": "untrusted-cached-payload"}
        records[-1]["consumer"]["samples"][1]["cooldown_gap_ns"] = 0
        synchronize(records[-1])
        result = reporter.compare_cases(records)
        self.assertEqual(result["status"], "fail")
        self.assertNotIn("untrusted-cached-payload", json.dumps(result))

    def test_runner_owns_timeout_exit_output_and_cleanup_acceptance(self):
        failures = (("exit_code", 1), ("timed_out", True), ("output_limited", True),
                    ("cleanup_ok", False), ("launch_error", "private-launch-detail"))
        for key, value in failures:
            with self.subTest(key=key):
                records = [record(case) for case in CASES]
                records[0]["process"][key] = value
                result = reporter.compare_cases(records)
                self.assertEqual(result["status"], "fail")
                self.assertNotIn("private-launch-detail", json.dumps(result))

    def test_outer_cleanup_allowance_does_not_extend_the_consumer_deadline(self):
        records = [record(case) for case in CASES]
        records[0]["process"]["duration_seconds"] = 130.0
        self.assertEqual(reporter.compare_cases(records)["status"], "pass")
        records[0]["process"]["timed_out"] = True
        self.assertEqual(reporter.compare_cases(records)["status"], "fail")
        records[0]["process"].update(timed_out=False, duration_seconds=130.001)
        result = reporter.compare_cases(records)
        self.assertEqual(result["status"], "fail")
        self.assertIn("runner-wall-budget", result["cases"][0]["failures"])

    def test_runner_json_identity_and_unique_keys_are_required(self):
        records = [record(case) for case in CASES]
        records[0]["process"]["stdout"] = json.dumps(consumer("baseline"))
        self.assertIn("runner-consumer-mismatch", reporter.compare_cases(records)["failures"])
        records = [record(case) for case in CASES]
        records[0]["process"]["stdout"] = '{"schema":1,"schema":1}'
        self.assertIn("json-duplicate-key", reporter.compare_cases(records)["failures"])

    def test_permission_refusal_is_not_run_not_zero_resource_success(self):
        value = permission_refusal("capture-off")["consumer"]
        result = reporter.analyze_case(value, None, "")
        self.assertEqual(result["status"], "not-run")
        self.assertEqual(result["reason"], "permission-denied-or-undetermined")
        self.assertEqual(result["metrics"], {})
        value["checks"]["cpu_model"] = "pass"
        self.assertEqual(reporter.analyze_case(value, None, "")["status"], "fail")
        del value["checks"]["cpu_model"]
        value["cleanup_status"] = "fail"
        self.assertEqual(reporter.analyze_case(value, None, "")["status"], "fail")
        value["cleanup_status"] = "pass"
        del value["metrics"]
        malformed = reporter.analyze_case(value, None, "")
        self.assertEqual(malformed["status"], "fail")
        self.assertNotEqual(malformed["reason"], "permission-denied-or-undetermined")

    def test_cli_consumes_raw_records_without_native_or_model(self):
        scratch = ROOT / ".rasen"
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="capture-pacing-report-", dir=scratch) as directory:
            source, output = Path(directory) / "records.json", Path(directory) / "report.json"
            records = [record("semantic")] + [record(case) for case in CASES]
            records[0]["process"]["duration_seconds"] = 190.0
            source.write_text(json.dumps(records), encoding="utf-8")
            completed = subprocess.run([sys.executable, str(SCRIPT), "--input", str(source), "--output", str(output)],
                                       capture_output=True, text=True, timeout=10, check=False)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["status"], "pass")
            source.write_text(json.dumps([permission_refusal()] + [placeholder(case) for case in CASES]), encoding="utf-8")
            completed = subprocess.run([sys.executable, str(SCRIPT), "--input", str(source), "--output", str(output)],
                                       capture_output=True, text=True, timeout=10, check=False)
            self.assertEqual(completed.returncode, 1)
            result = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(result["status"], "not-run")
            self.assertEqual(result["semantic"]["reason"], "permission-denied-or-undetermined")
            self.assertTrue(all(row["status"] == "not-run" for row in result["cases"]))
            incomplete = record("semantic")
            incomplete["fixture"]["peak_rss_bytes"] = None
            source.write_text(json.dumps([incomplete] + [placeholder(case) for case in CASES]), encoding="utf-8")
            completed = subprocess.run([sys.executable, str(SCRIPT), "--input", str(source), "--output", str(output)],
                                       capture_output=True, text=True, timeout=10, check=False)
            self.assertEqual(completed.returncode, 1)
            self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["status"], "incomplete")
            source.write_text("[NaN]", encoding="utf-8")
            completed = subprocess.run([sys.executable, str(SCRIPT), "--input", str(source), "--output", str(output)],
                                       capture_output=True, text=True, timeout=10, check=False)
            self.assertEqual(completed.returncode, 2)


if __name__ == "__main__":
    unittest.main()
