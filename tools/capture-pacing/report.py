"""Validate the frozen owned-fixture campaign; never collect or infer native work."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import sys

__all__ = ("analyze_case", "compare_cases")

CASES = ("capture-off", "baseline", "cooldown-only", "native-only", "combined")
ALL_CASES = ("semantic",) + CASES
U64_MAX = (1 << 64) - 1
MAX_INPUT_BYTES = 8 * 1024 * 1024
MAX_CHANNEL_BYTES = 1024 * 1024
GATES = {
    "process_peak_rss_bytes": 2 * 1024**3,
    "fixture_peak_rss_bytes": 256 * 1024**2,
    "mapping_bytes": 8 * 1024**2,
    "semantic_retained_bytes": 24 * 1024**2,
    "ocr_p95_ns": 5_000_000_000,
    "startup_ns": 60_000_000_000,
    "measurement_min_ns": 6_000_000_000,
    "cooldown_min_ns": 250_000_000,
    "native_required_ns": 100_000_000,
    "ocr_samples_min": 3,
    "ocr_samples_max": 1024,
    "process_samples_max": 2048,
    "fixture_samples_max": 4096,
    "sample_losses": 0,
}
ROOT_KEYS = frozenset((
    "schema", "case", "semantic_status", "cleanup_status", "reason", "permission",
    "startup_ns", "pacing", "checks", "metrics", "samples",
))
METRIC_KEYS = frozenset((
    "measurement_ns", "ocr_admissions", "ocr_committed", "caller_mapped_bytes",
    "max_mapping_bytes", "max_retained_layout_bytes", "process", "gpu_reason",
))
PROCESS_UINTS = frozenset((
    "sampler_elapsed_ns", "sample_count", "missed_poll_deadlines", "max_sample_gap_ns",
    "callback_invalid_intervals", "gpu_sample_count", "gpu_sampled_ns",
))
PROCESS_OPTIONAL_UINTS = frozenset((
    "cpu_ns", "max_resident_bytes", "callback_copied_bytes", "max_private_bytes",
    "max_footprint_bytes", "native_detached_textures_peak", "native_staging_textures_peak",
))
PROCESS_REASONS = frozenset(("capture_metrics_reason", "cpu_reason", "resident_reason", "gpu_reason"))
GPU_VALUES = frozenset(("gpu_engine_percent_mean", "gpu_engine_percent_max"))
PROCESS_KEYS = PROCESS_UINTS | PROCESS_OPTIONAL_UINTS | PROCESS_REASONS | GPU_VALUES | {"platform"}
SAMPLE_KEYS = frozenset((
    "ocr_ns", "frame_age_ns", "cooldown_gap_ns", "stream_id", "sequence", "epoch", "geometry_revision",
))
FIXTURE_KEYS = frozenset((
    "schema", "nonce", "pid", "wall_ns", "cpu_ns", "peak_rss_bytes", "paint_counter",
    "render_intervals_ns", "draw_durations_ns", "lost_samples",
))
SEMANTIC_CHECKS = frozenset((
    "source_default", "required_applied", "preferred_override", "owned_source",
    "completion_cooldown_live_idle", "burst_latest", "static_observations",
    "resize_retained_mapping", "deadline", "cancellation", "session_close", "target_close",
))
COMPARISON_CHECKS = frozenset((
    "owned_source", "source_identity", "pacing_report", "cooldown_spacing", "sample_capacity",
))
CHECK_KEYS = SEMANTIC_CHECKS | COMPARISON_CHECKS | {
    "cpu_model", "sample_loss", "resource_budgets", "fixture_cleanup", "capture_cleanup", "sequence_gaps",
}
LABEL = re.compile(r"[a-z][a-z0-9-]{0,95}\Z")
UINT_TEXT = re.compile(r"(?:0|[1-9][0-9]{0,19})\Z")
SIGNED_TEXT = re.compile(r"-?(?:0|[1-9][0-9]{0,18})\Z")
NONCE = re.compile(r"[0-9a-f]{16}\Z")


def _require(condition: bool, reason: str) -> None:
    if not condition:
        raise ValueError(reason)


def _keys(value: object, expected: frozenset | set, reason: str) -> None:
    _require(type(value) is dict and value.keys() == expected, reason)


def _uint(value: object, reason: str, *, optional: bool = False, maximum: int = U64_MAX) -> None:
    _require((optional and value is None) or (type(value) is int and 0 <= value <= maximum), reason)


def _label(value: object, reason: str, *, optional: bool = False) -> None:
    _require((optional and value is None) or
             (type(value) is str and LABEL.fullmatch(value) is not None), reason)


def _report_schema(report: dict) -> None:
    _keys(report, ROOT_KEYS, "consumer-schema-invalid")
    if type(report["schema"]) is int and report["schema"] == 1:
        raise ValueError("consumer-v1-evidence-requires-bound-reporter")
    _require(type(report["schema"]) is int and report["schema"] == 2, "consumer-schema-invalid")
    _require(report["case"] in ALL_CASES, "consumer-case-invalid")
    _require(report["semantic_status"] in ("pass", "fail", "not-run"), "consumer-status-invalid")
    _require(report["cleanup_status"] in ("pass", "fail"), "consumer-cleanup-invalid")
    _require(report["permission"] in
             ("granted", "not-required", "denied-or-undetermined", "unavailable"),
             "consumer-permission-invalid")
    _label(report["reason"], "consumer-reason-invalid", optional=True)
    _uint(report["startup_ns"], "consumer-startup-invalid", optional=True)
    _require(type(report["pacing"]) is list and len(report["pacing"]) <= 4, "pacing-schema-invalid")
    selections = set()
    for policy in report["pacing"]:
        _keys(policy, {"selection", "requested_ns", "configured_ns", "outcome"}, "pacing-schema-invalid")
        _require(policy["selection"] in ("source-default", "required", "preferred"), "pacing-selection-invalid")
        _require(policy["selection"] not in selections or
                 (report["case"] == "semantic" and policy["selection"] == "required"), "pacing-selection-duplicate")
        selections.add(policy["selection"])
        _require(policy["outcome"] in ("source-default", "applied", "preferred-unapplied"),
                 "pacing-outcome-invalid")
        _uint(policy["requested_ns"], "pacing-request-invalid", optional=True)
        _uint(policy["configured_ns"], "pacing-configured-invalid", optional=True)
    checks = report["checks"]
    _require(type(checks) is dict and checks.keys() <= CHECK_KEYS, "checks-schema-invalid")
    for name, status in checks.items():
        allowed = ("pass", "fail", "not-run")
        if name == "target_close":
            allowed += ("target-lost", "closed", "quiescent-deadline")
        elif name in COMPARISON_CHECKS | {"sequence_gaps", "capture_cleanup"}:
            allowed += ("not-applicable",)
        _require(status in allowed, "check-outcome-invalid")
    metrics = report["metrics"]
    _keys(metrics, METRIC_KEYS, "metrics-schema-invalid")
    for key in METRIC_KEYS - {"process", "gpu_reason"}:
        _uint(metrics[key], "metric-value-invalid")
    _label(metrics["gpu_reason"], "gpu-reason-invalid", optional=True)
    process = metrics["process"]
    if process is not None:
        _require(type(process) is not dict or "sample_losses" not in process,
                 "legacy-sampler-accounting-requires-bound-reporter")
        _keys(process, PROCESS_KEYS, "process-metrics-schema-invalid")
        _require(process["platform"] in ("macos", "windows", "unsupported"), "process-platform-invalid")
        for key in PROCESS_UINTS:
            _uint(process[key], "process-metric-invalid")
        for key in PROCESS_OPTIONAL_UINTS:
            _uint(process[key], "process-metric-invalid", optional=True)
        for key in PROCESS_REASONS:
            _label(process[key], "process-metric-reason-invalid", optional=True)
        for key in GPU_VALUES:
            value = process[key]
            _require(value is None or (type(value) in (int, float) and math.isfinite(value)
                                      and 0 <= value <= 25_600), "gpu-value-invalid")
        gpu_available = process["gpu_reason"] is None
        _require(all((process[key] is not None) == gpu_available for key in GPU_VALUES),
                 "gpu-availability-invalid")
        _require(process["gpu_sample_count"] <= process["sample_count"] and
                 process["gpu_sampled_ns"] <= process["sampler_elapsed_ns"], "gpu-sample-accounting-invalid")
        if gpu_available:
            _require(process["platform"] == "windows" and process["gpu_sample_count"] > 0
                     and process["gpu_sampled_ns"] > 0 and
                     process["gpu_engine_percent_mean"] <= process["gpu_engine_percent_max"],
                     "gpu-sample-accounting-invalid")
        _require(metrics["gpu_reason"] == process["gpu_reason"], "gpu-reason-mismatch")
        _require(process["sample_count"] <= GATES["process_samples_max"], "process-sample-cap-exceeded")
        _require((process["cpu_ns"] is None) == (process["cpu_reason"] is not None),
                 "process-cpu-availability-invalid")
        _require(process["max_resident_bytes"] != 0, "process-resident-zero-invalid")
        _require(process["max_resident_bytes"] is not None or process["resident_reason"] is not None,
                 "process-resident-availability-invalid")
        _require((process["callback_copied_bytes"] is None) ==
                 (process["capture_metrics_reason"] is not None), "copy-availability-invalid")
        _require(process["callback_invalid_intervals"] == 0 or
                 process["callback_copied_bytes"] is None, "invalidated-copy-total-published")
        if process["platform"] != "windows":
            _require(all(process[key] is None for key in
                         ("callback_copied_bytes", "native_detached_textures_peak", "native_staging_textures_peak")),
                     "unexposed-native-metric-published")
    samples = report["samples"]
    _require(type(samples) is list and len(samples) <= GATES["ocr_samples_max"], "ocr-sample-cap-exceeded")
    for sample in samples:
        _keys(sample, SAMPLE_KEYS, "sample-schema-invalid")
        for key in SAMPLE_KEYS:
            _uint(sample[key], "sample-value-invalid", optional=key in ("frame_age_ns", "cooldown_gap_ns"))
        _require(sample["stream_id"] > 0, "sample-identity-invalid")
    _require(metrics["ocr_committed"] <= len(samples) <= metrics["ocr_admissions"],
             "ocr-observation-count-mismatch")


def _fixture_schema(fixture: dict) -> None:
    _keys(fixture, FIXTURE_KEYS, "fixture-schema-invalid")
    _require(type(fixture["schema"]) is int and fixture["schema"] == 1, "fixture-schema-invalid")
    _require(type(fixture["nonce"]) is str and NONCE.fullmatch(fixture["nonce"]) is not None,
             "fixture-identity-invalid")
    for key in ("pid", "paint_counter"):
        _uint(fixture[key], "fixture-identity-invalid", maximum=(1 << 32) - 1)
        _require(fixture[key] > 0, "fixture-identity-invalid")
    for key in ("wall_ns", "lost_samples", "cpu_ns", "peak_rss_bytes"):
        _uint(fixture[key], "fixture-metric-invalid", optional=key in ("cpu_ns", "peak_rss_bytes"))
    _require(fixture["peak_rss_bytes"] != 0, "fixture-resident-zero-invalid")
    for key in ("render_intervals_ns", "draw_durations_ns"):
        _require(type(fixture[key]) is list and len(fixture[key]) <= GATES["fixture_samples_max"],
                 "fixture-sample-cap-exceeded")
        for sample in fixture[key]:
            _uint(sample, "fixture-sample-invalid")
    _require(fixture["wall_ns"] > 0, "fixture-wall-invalid")
    _require(sum(fixture["draw_durations_ns"]) <= fixture["wall_ns"], "fixture-draw-time-invalid")
    _require(sum(fixture["render_intervals_ns"]) <= fixture["wall_ns"], "fixture-interval-time-invalid")


SCK_PREFIX = "benchmark-sck-diagnostics"
SCK_AGGREGATE = frozenset((
    "event", "tier", "closed_sessions", "active_sessions", "callbacks_in_flight",
    "detached_leases", "detached_bytes", "native_objects", "native_mask_schema", "error_domain_schema",
))
SCK_SESSION = frozenset((
    "event", "sequence", "tier", "close_phase", "terminal", "close_error", "closed", "start_nanos",
    "start_status", "stop_requested_nanos", "stop_completed_nanos", "stop_status", "stop_error",
    "stop_error_domain", "admission_stopped_nanos", "callback_fenced_nanos", "release_completed_nanos",
    "close_completed_nanos", "publication_sequence", "publication_nanos", "publication_display_nanos",
    "content", "surface", "callbacks_received", "callbacks_admitted", "callbacks_refused",
    "callbacks_entered", "callbacks_exited", "callback_accepting", "callback_fenced", "refs",
    "native_before", "native_after", "detached_leases", "detached_bytes", "process_native_objects",
    "transition_count", "transition_overflow",
))
SCK_TRANSITION = frozenset((
    "event", "session_sequence", "index", "raw", "normalized", "normalized_name", "status_sequence",
    "monotonic_nanos",
))
SCK_TERMINALS = frozenset((
    "ok", "invalid_argument", "unsupported", "permission_denied", "target_lost", "platform_failure",
    "native_exception", "closed", "timed_out", "budget_exhausted", "frame_incomplete", "stopped_by_user",
    "stopped_by_system", "geometry_changed", "focus_required", "unknown",
))
SCK_STATUSES = frozenset(("complete", "idle", "blank", "started", "suspended", "stopped", "missing", "unknown"))


def _sck_diagnostics(stderr: str) -> dict | None:
    _require(type(stderr) is str and len(stderr) <= MAX_CHANNEL_BYTES, "stderr-bound-exceeded")
    lines = [line for line in stderr.splitlines() if line.startswith(SCK_PREFIX)]
    if not lines:
        return None
    _require(len(lines) <= 273, "native-diagnostics-record-cap-exceeded")
    aggregate = None
    sessions = {}
    transitions = {}
    for line in lines:
        _require(len(line) <= 4096 and line.startswith(SCK_PREFIX + " "), "native-diagnostics-malformed")
        row = {}
        for token in line[len(SCK_PREFIX) + 1:].split(" "):
            key, separator, value = token.partition("=")
            _require(bool(separator and value) and key not in row, "native-diagnostics-malformed")
            row[key] = value
        event = row.get("event")
        expected = {"aggregate": SCK_AGGREGATE, "session": SCK_SESSION, "transition": SCK_TRANSITION}
        _require(event in expected and row.keys() == expected[event], "native-diagnostics-schema-invalid")
        for key, value in tuple(row.items()):
            if key == "event":
                continue
            if key == "native_mask_schema":
                _require(value == "stream:1,configuration:2,filter:4,output:8,queue:16,pool:32", "native-schema-invalid")
            elif key == "error_domain_schema":
                _require(value == "none:0,stream:1,other:2", "native-schema-invalid")
            elif key in ("terminal", "close_error", "start_status", "stop_status"):
                _require(value in SCK_TERMINALS, "native-status-invalid")
            elif key == "normalized_name":
                _require(value in SCK_STATUSES, "native-status-invalid")
            elif key in ("content", "surface"):
                _require(re.fullmatch(r"[0-9]{1,10}x[0-9]{1,10}", value) is not None, "native-extent-invalid")
                _require(all(int(part) <= (1 << 32) - 1 for part in value.split("x")), "native-extent-invalid")
            elif key in ("stop_error", "raw"):
                _require(SIGNED_TEXT.fullmatch(value) is not None and -(1 << 63) <= int(value) < (1 << 63),
                         "native-integer-invalid")
                row[key] = int(value)
            else:
                _require(UINT_TEXT.fullmatch(value) is not None and int(value) <= U64_MAX,
                         "native-integer-invalid")
                row[key] = int(value)
        if event == "aggregate":
            _require(aggregate is None, "native-aggregate-duplicate")
            aggregate = row
        elif event == "session":
            _require(row["sequence"] not in sessions, "native-session-duplicate")
            sessions[row["sequence"]] = row
        else:
            identity = (row["session_sequence"], row["index"])
            _require(identity not in transitions, "native-transition-duplicate")
            transitions[identity] = row
    _require(aggregate is not None, "native-aggregate-missing")
    _require(aggregate["tier"] == 2 and aggregate["closed_sessions"] <= 16, "native-tier-invalid")
    _require(set(sessions) == set(range(1, aggregate["closed_sessions"] + 1)), "native-session-loss")
    _require(all(aggregate[key] == 0 for key in
                 ("active_sessions", "callbacks_in_flight", "detached_leases", "detached_bytes", "native_objects")),
             "native-owners-not-released")
    expected_transitions = set()
    for sequence, row in sessions.items():
        _require(row["tier"] == 2 and row["closed"] == 1 and row["callback_accepting"] == 0
                 and row["callback_fenced"] == 1 and row["native_after"] == 0,
                 "native-session-not-closed")
        _require(row["callbacks_received"] == row["callbacks_admitted"] + row["callbacks_refused"]
                 and row["callbacks_admitted"] == row["callbacks_entered"] == row["callbacks_exited"],
                 "native-callback-accounting-invalid")
        _require(row["transition_count"] <= 16 and row["transition_overflow"] in (0, 1)
                 and (row["transition_overflow"] == 0 or row["transition_count"] == 16),
                 "native-transition-loss")
        _require(row["close_error"] == "ok" and row["stop_status"] == "ok" and row["stop_error"] == 0,
                 "native-close-failed")
        expected_transitions.update((sequence, index) for index in range(row["transition_count"]))
    _require(transitions.keys() == expected_transitions, "native-transition-loss")
    for field in ("callbacks_received", "callbacks_admitted", "callbacks_refused"):
        _uint(sum(row[field] for row in sessions.values()), "native-callback-total-overflow")
    return {
        "scope": "closed-session-lifetime-including-warmup",
        "closed_sessions": len(sessions),
        "status_history_complete": not any(row["transition_overflow"] for row in sessions.values()),
        "status_history_truncated_sessions": sum(row["transition_overflow"] for row in sessions.values()),
        "retained_status_transitions": len(transitions),
        "callbacks_received": sum(row["callbacks_received"] for row in sessions.values()),
        "callbacks_admitted": sum(row["callbacks_admitted"] for row in sessions.values()),
        "callbacks_refused": sum(row["callbacks_refused"] for row in sessions.values()),
        "measurement_callback_rate": None,
        "measurement_callback_rate_reason": "measurement-boundaries-not-exposed",
        "final_detached_leases": aggregate["detached_leases"],
        "final_detached_bytes": aggregate["detached_bytes"],
    }


def _percentiles(values: list[int]) -> dict:
    ordered = sorted(values)
    return {"count": len(ordered), "p50_ns": ordered[(len(ordered) * 50 + 99) // 100 - 1] if ordered else None,
            "p95_ns": ordered[(len(ordered) * 95 + 99) // 100 - 1] if ordered else None,
            "max_ns": ordered[-1] if ordered else None}


def _rate(value: int | None, elapsed_ns: int) -> float | None:
    return value * 1_000_000_000 / elapsed_ns if value is not None and elapsed_ns > 0 else None


def analyze_case(report: dict, fixture: dict | None, stderr: str) -> dict:
    """Return privacy-safe verdicts and scoped rates; malformed evidence never passes."""
    result = {"case": None, "status": "fail", "failures": [], "unavailable": [], "gates": {},
              "metrics": {}, "native": None, "reason": None,
              "withheld_claims": [
                  "device-gpu-utilization-savings", "native-callback-rate-savings",
                  "continuous-private-memory-peak-savings", "continuous-footprint-peak-savings",
                  "instantaneous-gpu-engine-peak-savings", "ideal-100ms-sampling-resolution",
              ]}
    if type(report) is dict and report.get("case") in ALL_CASES:
        result["case"] = report["case"]
    try:
        _report_schema(report)
        if fixture is not None:
            _fixture_schema(fixture)
        result["native"] = _sck_diagnostics(stderr)
    except (ValueError, TypeError, KeyError, OverflowError) as error:
        result["failures"] = [str(error) if type(error) is ValueError else "evidence-schema-invalid"]
        result["reason"] = "evidence-invalid"
        return result
    if result["native"] is not None and not result["native"]["status_history_complete"]:
        result["withheld_claims"].append("complete-native-status-history")
    case = report["case"]
    metrics, samples = report["metrics"], report["samples"]
    previous_by_stream = {}
    stamps_ordered = True
    sequence_gaps = 0
    for sample in samples:
        previous = previous_by_stream.get(sample["stream_id"])
        if previous is not None:
            stamps_ordered &= (sample["epoch"], sample["sequence"]) > (previous["epoch"], previous["sequence"])
            if sample["epoch"] == previous["epoch"]:
                stamps_ordered &= sample["geometry_revision"] >= previous["geometry_revision"]
                sequence_gaps += max(0, sample["sequence"] - previous["sequence"] - 1)
        previous_by_stream[sample["stream_id"]] = sample
    process = metrics["process"]

    def gate(name: str, condition: bool | None) -> None:
        result["gates"][name] = "unavailable" if condition is None else "pass" if condition else "fail"
        if condition is None:
            result["unavailable"].append(name)
        elif not condition:
            result["failures"].append(name)

    gate("consumer-outcome", report["semantic_status"] != "fail")
    gate("cleanup", report["cleanup_status"] == "pass")
    gate("consumer-reason-consistency", (report["reason"] is None) == (report["semantic_status"] == "pass"))
    if report["semantic_status"] == "not-run":
        gate("not-run-no-observations", not samples and process is None and fixture is None and
             result["native"] is None and report["startup_ns"] is None and not report["pacing"] and
             all(value in ("not-run", "not-applicable") or
                 (name in ("capture_cleanup", "fixture_cleanup") and value == "pass")
                 for name, value in report["checks"].items()) and
             all(metrics[key] == 0 for key in METRIC_KEYS - {"process", "gpu_reason"}))
        result["status"] = "fail" if result["failures"] else "not-run"
        result["reason"] = ("evidence-invalid" if result["failures"] else
                            "permission-denied-or-undetermined" if report["permission"] == "denied-or-undetermined"
                            else "consumer-not-run")
        return result
    gate("permission", report["permission"] in ("granted", "not-required"))
    gate("startup-budget", None if report["startup_ns"] is None else
         report["startup_ns"] <= GATES["startup_ns"])
    gate("gpu-unavailability-declared", process is not None or metrics["gpu_reason"] is not None)
    gate("committed-admissions", metrics["ocr_admissions"] == len(samples) == metrics["ocr_committed"])
    required_checks = SEMANTIC_CHECKS if case == "semantic" else COMPARISON_CHECKS
    for name in sorted(required_checks):
        accepted = ("pass",)
        if name == "target_close":
            accepted = ("target-lost", "closed")
            if process is not None and process["platform"] == "macos":
                accepted += ("quiescent-deadline",)
        elif case == "capture-off" and name in COMPARISON_CHECKS - {"sample_capacity"}:
            accepted += ("not-applicable",)
        gate("check-" + name, report["checks"].get(name) in accepted)
    gate("reported-checks", all(value != "fail" for value in report["checks"].values()))
    if report["checks"].get("target_close") == "quiescent-deadline":
        result["withheld_claims"].append("quiescent-target-lost-notification")
    expected_policies = ({"required": 100_000_000, "source-default": None, "preferred": 60_000_000}
                         if case == "semantic" else {} if case == "capture-off" else
                         {"required": 100_000_000} if case in ("native-only", "combined") else
                         {"source-default": None})
    expected_counts = {selection: 2 if case == "semantic" and selection == "required" else 1
                       for selection in expected_policies}
    observed_counts = {}
    for policy in report["pacing"]:
        observed_counts[policy["selection"]] = observed_counts.get(policy["selection"], 0) + 1
    gate("pacing-selections", observed_counts == expected_counts)
    policy_validity = {}
    for policy in report["pacing"]:
        requested = expected_policies.get(policy["selection"])
        correct = policy["requested_ns"] == requested
        if requested is None:
            correct &= policy["outcome"] == "source-default" and policy["configured_ns"] is None
        elif policy["selection"] == "preferred" and policy["outcome"] == "preferred-unapplied":
            correct &= policy["configured_ns"] is None
            result["withheld_claims"].append("preferred-native-interval-applied")
        else:
            correct &= policy["outcome"] == "applied" and policy["configured_ns"] is not None
            correct &= policy["configured_ns"] is not None and policy["configured_ns"] >= requested
        policy_validity[policy["selection"]] = policy_validity.get(policy["selection"], True) and correct
    for selection, correct in policy_validity.items():
        gate("policy-" + selection, correct)
    elapsed = metrics["measurement_ns"]
    limit = 180_000_000_000 if case == "semantic" else 120_000_000_000
    gate("measurement-wall", 0 < elapsed <= limit and (case == "semantic" or elapsed >= GATES["measurement_min_ns"]))
    gate("mapping-budget", metrics["max_mapping_bytes"] <= GATES["mapping_bytes"])
    gate("retained-layout-budget", metrics["max_retained_layout_bytes"] <=
         (GATES["semantic_retained_bytes"] if case == "semantic" else GATES["mapping_bytes"]))
    gate("mapping-accounting", metrics["caller_mapped_bytes"] >= metrics["max_mapping_bytes"] and
         metrics["max_retained_layout_bytes"] >= metrics["max_mapping_bytes"])
    gate("publication-stamp-order", stamps_ordered)
    if case == "capture-off":
        gate("capture-off-no-capture-work", not samples and all(metrics[key] == 0 for key in
             ("ocr_admissions", "ocr_committed", "caller_mapped_bytes", "max_mapping_bytes", "max_retained_layout_bytes")))
        gate("capture-off-native-sessions", result["native"] is None or result["native"]["closed_sessions"] == 0)
        if process is not None:
            gate("capture-off-copy-scope", process["callback_copied_bytes"] is None and
                 process["capture_metrics_reason"] == "capture-disabled")
            if process["platform"] == "windows":
                gate("capture-off-native-textures", process["native_detached_textures_peak"] == 0 and
                     process["native_staging_textures_peak"] == 0)
    else:
        gate("captured-mapping-observed", metrics["max_mapping_bytes"] > 0)
        if case != "semantic":
            gate("ocr-sample-minimum", metrics["ocr_committed"] >= GATES["ocr_samples_min"])
            gate("ocr-wall-accounting", sum(row["ocr_ns"] for row in samples) <= elapsed)
            gate("cooldown-origin", not samples or samples[0]["cooldown_gap_ns"] is None)
            gate("completion-cooldown", all(row["cooldown_gap_ns"] is not None and
                 row["cooldown_gap_ns"] >= (GATES["cooldown_min_ns"] if case in ("cooldown-only", "combined") else 0)
                 for row in samples[1:]))
    ocr = _percentiles([row["ocr_ns"] for row in samples])
    if case not in ("semantic", "capture-off"):
        gate("ocr-p95-budget", None if ocr["p95_ns"] is None else ocr["p95_ns"] <= GATES["ocr_p95_ns"])
    gate("process-samples", None if process is None else 2 <= process["sample_count"] <= GATES["process_samples_max"])
    gate("process-memory-budget", None if process is None or process["max_resident_bytes"] is None
         or process["resident_reason"] is not None else process["max_resident_bytes"] <= GATES["process_peak_rss_bytes"])
    if process is not None:
        gate("sampler-wall", 0 < process["sampler_elapsed_ns"] <= limit)
        gate("sampler-cadence", process["sample_count"] + process["missed_poll_deadlines"] >=
             process["sampler_elapsed_ns"] // 100_000_000 and
             process["missed_poll_deadlines"] <= process["sampler_elapsed_ns"] // 100_000_000 and
             process["max_sample_gap_ns"] <= process["sampler_elapsed_ns"])
        if process["platform"] == "macos" and case != "capture-off":
            gate("native-close-diagnostics", None if result["native"] is None else
                 result["native"]["closed_sessions"] >= 1)
        if process["platform"] == "windows":
            gate("platform-diagnostics-consistency", result["native"] is None)
        gate("supported-platform", process["platform"] in ("windows", "macos"))
    gate("fixture-observations", None if fixture is None else len(fixture["draw_durations_ns"]) >= 2)
    gate("fixture-sample-loss", None if fixture is None else fixture["lost_samples"] == 0)
    gate("fixture-memory-budget", None if fixture is None or fixture["peak_rss_bytes"] is None else
         fixture["peak_rss_bytes"] <= GATES["fixture_peak_rss_bytes"])
    if fixture is not None:
        gate("fixture-wall", fixture["wall_ns"] <= limit and (case == "semantic" or
             fixture["wall_ns"] >= GATES["measurement_min_ns"]))
        gate("fixture-draw-accounting", max(0, len(fixture["draw_durations_ns"]) - 1) <=
             len(fixture["render_intervals_ns"]) <= len(fixture["draw_durations_ns"]))
    sampler_wall = process["sampler_elapsed_ns"] if process else 0
    fixture_wall = fixture["wall_ns"] if fixture else 0
    copies = process["callback_copied_bytes"] if process else None
    cpu = process["cpu_ns"] if process else None
    fixture_cpu = fixture["cpu_ns"] if fixture else None
    gpu_mean = process["gpu_engine_percent_mean"] if process else None
    if gpu_mean is None:
        result["withheld_claims"].append("own-process-gpu-engine-savings")
    if copies is None:
        result["withheld_claims"].append("native-copy-byte-savings")
    if cpu is None:
        result["withheld_claims"].append("consumer-cpu-savings")
    if fixture_cpu is None:
        result["withheld_claims"].append("fixture-cpu-savings")
    result["metrics"] = {
        "measurement_ns": elapsed, "sampler_elapsed_ns": sampler_wall or None,
        "fixture_wall_ns": fixture_wall or None,
        "ocr_admissions": metrics["ocr_admissions"], "ocr_committed": metrics["ocr_committed"],
        "ocr_attempt_samples": len(samples),
        "callback_invalid_intervals": process["callback_invalid_intervals"] if process else None,
        "process_sample_count": process["sample_count"] if process else None,
        "missed_poll_deadlines": process["missed_poll_deadlines"] if process else None,
        "max_sample_gap_ns": process["max_sample_gap_ns"] if process else None,
        "process_polling_scope": "target-cadence-with-observed-gaps-not-discarded-observations",
        "ocr_per_second": _rate(metrics["ocr_committed"], elapsed),
        "ocr_latency": ocr,
        "frame_age": _percentiles([row["frame_age_ns"] for row in samples if row["frame_age_ns"] is not None]),
        "frame_age_unavailable_samples": sum(row["frame_age_ns"] is None for row in samples),
        "cooldown_gap": _percentiles([row["cooldown_gap_ns"] for row in samples if row["cooldown_gap_ns"] is not None]),
        "publication_sequence_gaps": sequence_gaps,
        "static_content_scope": "ocr-roi-only-excluding-the-changing-owner-marker",
        "caller_mapping_bytes_per_second": _rate(metrics["caller_mapped_bytes"], elapsed),
        "max_mapping_bytes": metrics["max_mapping_bytes"],
        "max_retained_layout_bytes": metrics["max_retained_layout_bytes"],
        "process_cpu_cores": _rate(cpu, sampler_wall) / 1_000_000_000 if cpu is not None and sampler_wall else None,
        "process_peak_rss_bytes": process["max_resident_bytes"] if process else None,
        "callback_copied_bytes": copies, "callback_copy_bytes_per_second": _rate(copies, sampler_wall),
        "native_callbacks_per_second": None, "device_gpu_utilization": None,
        "gpu_engine_percent_mean": gpu_mean,
        "gpu_engine_percent_max": process["gpu_engine_percent_max"] if process else None,
        "gpu_sample_count": process["gpu_sample_count"] if process else None,
        "gpu_sampled_ns": process["gpu_sampled_ns"] if process else None,
        "gpu_scope": "time-weighted-sum-of-own-process-engine-percentages",
        "fixture_cpu_cores": _rate(fixture_cpu, fixture_wall) / 1_000_000_000 if fixture_cpu is not None and fixture_wall else None,
        "fixture_peak_rss_bytes": fixture["peak_rss_bytes"] if fixture else None,
        "fixture_render_starts_per_second": _rate(len(fixture["draw_durations_ns"]), fixture_wall) if fixture else None,
        "fixture_draw_busy_fraction": sum(fixture["draw_durations_ns"]) / fixture_wall if fixture else None,
        "fixture_render_interval": _percentiles(fixture["render_intervals_ns"]) if fixture else None,
        "fixture_draw_duration": _percentiles(fixture["draw_durations_ns"]) if fixture else None,
    }
    result["metric_reasons"] = {
        "consumer_cpu": process["cpu_reason"] if process else "process-sampler-unavailable",
        "consumer_resident": process["resident_reason"] if process else "process-sampler-unavailable",
        "native_copy_bytes": process["capture_metrics_reason"] if process else "process-sampler-unavailable",
        "gpu_engines": process["gpu_reason"] if process else metrics["gpu_reason"],
        "native_callback_rate": "measurement-boundaries-not-exposed" if result["native"] else "native-callback-count-not-exposed",
    }
    result["status"] = "fail" if result["failures"] else "incomplete" if result["unavailable"] else "pass"
    result["reason"] = ("declared-gate-failed" if result["failures"] else
                        "gate-evidence-unavailable" if result["unavailable"] else None)
    return result


def _unique_object(pairs: list[tuple[str, object]]) -> dict:
    value = {}
    for key, item in pairs:
        _require(key not in value, "json-duplicate-key")
        value[key] = item
    return value


def _load_json(text: str) -> object:
    def invalid_constant(_: str) -> None:
        raise ValueError("json-nonfinite-number")
    try:
        return json.loads(text, object_pairs_hook=_unique_object, parse_constant=invalid_constant)
    except (json.JSONDecodeError, RecursionError, UnicodeError):
        raise ValueError("json-invalid") from None


def _runner_failures(record: dict) -> list[str]:
    runner = record["process"]
    required = {"exit_code", "timed_out", "output_limited", "cleanup_ok", "stdout", "stderr",
                "duration_seconds", "launch_error"}
    _require(type(runner) is dict and required <= runner.keys(), "runner-schema-invalid")
    _require(type(runner["exit_code"]) is int or runner["exit_code"] is None, "runner-exit-invalid")
    for key in ("timed_out", "output_limited", "cleanup_ok"):
        _require(type(runner[key]) is bool, "runner-flag-invalid")
    duration = runner["duration_seconds"]
    _require(type(duration) in (int, float) and math.isfinite(duration) and duration >= 0, "runner-duration-invalid")
    _require(runner["launch_error"] is None or type(runner["launch_error"]) is str, "runner-launch-invalid")
    for key in ("stdout", "stderr"):
        _require(type(runner[key]) is str and len(runner[key]) <= MAX_CHANNEL_BYTES, "runner-output-bound-exceeded")
    failures = []
    if runner["exit_code"] != 0:
        failures.append("runner-nonzero-exit")
    if runner["timed_out"]:
        failures.append("runner-timeout")
    if runner["output_limited"]:
        failures.append("runner-output-limit")
    if not runner["cleanup_ok"]:
        failures.append("runner-cleanup-failed")
    if runner["launch_error"] is not None:
        failures.append("runner-launch-failed")
    if not 0 < duration <= (190 if record["case"] == "semantic" else 130):
        failures.append("runner-wall-budget")
    if runner["stdout"]:
        _require(_load_json(runner["stdout"]) == record["consumer"], "runner-consumer-mismatch")
    else:
        failures.append("runner-consumer-missing")
    _require(runner["stderr"] == record["stderr"], "runner-stderr-mismatch")
    return failures


def _analyze_record(record: dict) -> dict:
    required = {"case", "consumer", "fixture", "stderr", "process"}
    _require(type(record) is dict and required <= record.keys() <= required | {"analysis"},
             "case-record-schema-invalid")
    _require(record["case"] in ALL_CASES, "case-record-name-invalid")
    if record["consumer"] is None:
        _require(record["fixture"] is None and record["process"] is None and
                 type(record["stderr"]) is str and record["stderr"] == "", "not-run-record-invalid")
        return {
            "case": record["case"], "status": "not-run", "reason": "case-not-executed",
            "failures": [], "unavailable": [], "gates": {}, "metrics": {}, "native": None,
            "withheld_claims": ["all-case-measurement-claims"],
        }
    analysis = analyze_case(record["consumer"], record["fixture"], record["stderr"])
    _require(analysis["case"] in (None, record["case"]), "case-record-consumer-mismatch")
    analysis["case"] = record["case"]
    failures = _runner_failures(record)
    if analysis["status"] == "not-run":
        if record["process"]["exit_code"] == 0:
            failures.append("runner-not-run-zero-exit")
        elif record["process"]["exit_code"] in (1, 2):
            failures = [failure for failure in failures if failure != "runner-nonzero-exit"]
    elapsed = analysis["metrics"].get("measurement_ns")
    if elapsed is not None and elapsed > record["process"]["duration_seconds"] * 1_000_000_000:
        failures.append("runner-measurement-window-mismatch")
    analysis["failures"].extend(failures)
    if failures:
        analysis["reason"] = "process-runner-failed"
    if analysis["failures"]:
        analysis["status"] = "fail"
    return analysis


def _aggregate_status(statuses: list[str], *, failures: bool = False) -> str:
    if failures or "fail" in statuses:
        return "fail"
    if statuses and all(status == "pass" for status in statuses):
        return "pass"
    if statuses and all(status == "not-run" for status in statuses):
        return "not-run"
    return "incomplete"


def compare_cases(cases: list[dict]) -> dict:
    """Analyze one record per fresh case; ratios use each counter's actual scope."""
    result = {"schema": 2, "status": "fail", "failures": [], "cases": [], "comparisons": [],
              "predeclared_gates": dict(GATES),
              "scope": "owned-gdi-appkit-renderer-not-compositor-gpu-or-general-game-fps",
              "normalization": {"ocr_and_caller_mapping": "consumer.metrics.measurement_ns",
                                "consumer_cpu_and_callback_copy": "consumer.metrics.process.sampler_elapsed_ns",
                                "fixture_cpu_and_draws": "fixture.wall_ns",
                                "native_callbacks": "unavailable-no-measurement-boundaries"}}
    if type(cases) is not list or not 1 <= len(cases) <= len(ALL_CASES):
        result["failures"].append("case-record-count-invalid")
        return result
    seen = set()
    platforms = set()
    for record in cases:
        try:
            _require(type(record) is dict and record.get("case") in CASES, "case-record-name-invalid")
            _require(record["case"] not in seen, "case-record-duplicate")
            seen.add(record["case"])
            analysis = _analyze_record(record)
            result["cases"].append(analysis)
            if analysis["metrics"]:
                process = record["consumer"]["metrics"]["process"]
                if process is not None and process["platform"] in ("macos", "windows"):
                    platforms.add(process["platform"])
        except (ValueError, TypeError, KeyError, OverflowError) as error:
            result["failures"].append(str(error) if type(error) is ValueError else "case-record-schema-invalid")
    if set(CASES) - seen:
        result["failures"].append("comparison-cases-missing")
    if len(platforms) > 1:
        result["failures"].append("comparison-platform-mismatch")
    by_case = {row["case"]: row for row in result["cases"]}
    baseline = by_case.get("baseline")
    rate_keys = ("ocr_per_second", "process_cpu_cores", "caller_mapping_bytes_per_second",
                 "callback_copy_bytes_per_second", "fixture_cpu_cores", "fixture_render_starts_per_second",
                 "fixture_draw_busy_fraction", "gpu_engine_percent_mean")
    for case in CASES:
        if case == "baseline" or case not in by_case:
            continue
        row = by_case[case]
        ratios = {}
        for key in rate_keys:
            reference = baseline["metrics"].get(key) if baseline else None
            observed = row["metrics"].get(key)
            reason = None
            if result["failures"]:
                reason = "campaign-not-qualified"
            elif baseline is None or baseline["status"] != "pass" or row["status"] != "pass":
                reason = "case-not-qualified"
            elif reference is None or observed is None:
                reason = "metric-unavailable"
            elif reference == 0:
                reason = "zero-reference-no-ratio"
            ratios[key] = {"ratio": observed / reference if reason is None else None, "reason": reason}
        result["comparisons"].append({"case": case, "reference": "baseline", "rates": ratios,
                                      "interpretation": "descriptive-single-owned-case-not-savings-guarantee"})
    statuses = [row["status"] for row in result["cases"]]
    result["status"] = _aggregate_status(statuses, failures=bool(result["failures"]))
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        with args.input.open("rb") as source:
            data = source.read(MAX_INPUT_BYTES + 1)
        _require(len(data) <= MAX_INPUT_BYTES, "input-bound-exceeded")
        records = _load_json(data.decode("utf-8"))
        _require(type(records) is list and 1 <= len(records) <= len(ALL_CASES), "case-record-count-invalid")
        semantic = [row for row in records if type(row) is dict and row.get("case") == "semantic"]
        comparison = [row for row in records if type(row) is not dict or row.get("case") != "semantic"]
        result = compare_cases(comparison)
        if len(semantic) != 1:
            result["failures"].append("semantic-case-missing-or-duplicate")
            result["status"] = "fail"
        else:
            try:
                result["semantic"] = _analyze_record(semantic[0])
                result["status"] = _aggregate_status(
                    [result["status"], result["semantic"]["status"]], failures=bool(result["failures"]))
            except (ValueError, TypeError, KeyError, OverflowError) as error:
                result["failures"].append(str(error) if type(error) is ValueError else "semantic-record-invalid")
                result["status"] = "fail"
    except (OSError, UnicodeError, ValueError, RecursionError):
        print("capture-pacing-report: input-invalid", file=sys.stderr)
        return 2
    try:
        args.output.write_text(json.dumps(result, ensure_ascii=True, allow_nan=False, indent=2) + "\n",
                               encoding="utf-8")
    except (OSError, ValueError):
        print("capture-pacing-report: output-failed", file=sys.stderr)
        return 2
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
