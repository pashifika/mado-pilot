"""Validate and compare one OCR process against an already reviewed numeric profile.

These helpers select no ceilings and establish no source, host, executable, ADR,
semantic, dependency, or cleanup acceptance. The runner owns those gates. Times
come from integer nanoseconds without rounding or comparison tolerances; native
memory observations remain independent of logical view extents.
"""

from __future__ import annotations

import math as _math
import re as _re

__all__ = ("REQUIRED_METRICS", "validate_profile", "summarize_process", "evaluate_process")

_U64_MAX = (1 << 64) - 1
_TIME_LIMIT_MAX_MS = 300000
_TARGETS = ("aarch64-apple-darwin", "x86_64-pc-windows-msvc")
_CONTROLLED_WORKLOADS = (
    "steady-nonmatch", "positive-consecutive", "slow-backend-saturation",
    "mixed-two-session", "retained-results", "cancellation-close", "cold-startup",
)
_REAL_WORKLOAD = "real-cpu-cold-startup"
_WORKLOADS = (*_CONTROLLED_WORKLOADS, _REAL_WORKLOAD)
_INVOCATIONS = tuple(
    (name, "warmup" if index < 2 else "sample", index)
    for name in _CONTROLLED_WORKLOADS[:-1] for index in range(22)
) + (("cold-startup", "sample", 0),)
_CANCELLATION_STAGES = (
    "independent-wait-cancel", "query-cancel", "session-close", "last-runtime-owner-drop",
    "held-release-to-actual-resource-retirement",
)
_ENDPOINTS = {
    "steady-nonmatch": ("publication-to-observation",) * 21 + ("query-cancel",),
    "positive-consecutive": ("first-positive-publication-to-terminal",),
    "slow-backend-saturation": (
        "eligible-expiry-advance-to-terminals", "held-release-to-physical-zero",
    ),
    "mixed-two-session": ("mapping-release-to-physical-zero",),
    "retained-results": (),
    "cancellation-close": _CANCELLATION_STAGES * 2,
    "cold-startup": (
        "controlled-engine-construction", "controlled-session-open", "first-query-start",
        "first-positive-to-terminal", "logical-close",
    ),
}
_CONTROLLED_RESOURCES = (
    "backend_input_max_bytes", "mapped_cache_high_water_bytes", "retained_read_mapped_bytes",
    "retained_source_high_water_bytes", "retained_text_high_water_bytes",
    "retained_index_high_water_bytes", "separate_frame_high_water_bytes",
    "resident_current_high_water_bytes", "resident_process_peak_bytes", "physical_ocr_high_water",
)
_CONTROLLED_MEMORY = (
    "resident_current_high_water_bytes", "resident_process_peak_bytes",
    "private_high_water_bytes", "physical_footprint_high_water_bytes",
)
_CONTROLLED_INTEGERS = frozenset((
    "iteration", "elapsed_ns", "backend_input_mapped_bytes", "backend_input_max_bytes",
    "retained_read_mapped_bytes", "mapped_cache_high_water_bytes",
    "retained_source_high_water_bytes", "retained_text_high_water_bytes",
    "retained_index_high_water_bytes", "separate_frame_high_water_bytes",
    "memory_samples", "physical_ocr_high_water",
))
_REAL_STAGES = (
    "process-start", "runtime-initialized", "provider-prepared", "detector-session-ready",
    "recognizer-session-ready", "engine-ready", "session-ready", "query-terminal",
    "logical-close-returned", "physical-ocr-zero", "parents-dropped", "retained-results-dropped",
)
_REAL_MEMORY = (
    "resident_current_bytes", "resident_process_peak_bytes", "private_bytes",
    "physical_footprint_bytes",
)
_REAL_INTERVALS = (
    ("startup_to_session_ready_ms", "process-start", "session-ready"),
    ("session_ready_to_query_terminal_ms", "session-ready", "query-terminal"),
    ("query_terminal_to_logical_close_returned_ms", "query-terminal", "logical-close-returned"),
    ("logical_close_returned_to_physical_ocr_zero_ms", "logical-close-returned", "physical-ocr-zero"),
    ("parents_dropped_to_retained_results_dropped_ms", "parents-dropped", "retained-results-dropped"),
    ("process_start_to_retained_results_dropped_ms", "process-start", "retained-results-dropped"),
)
_REAL_SUMMARY_METRICS = (
    "retained_source_extent_bytes", "retained_text_extent_bytes", "retained_index_extent_bytes",
    "retained_read_mapped_bytes", "physical_ocr_after_close", "physical_ocr_final",
)
_REAL_SUMMARY_INTEGERS = frozenset((
    "schema", "open_stages", "detector_sessions_created", "recognizer_sessions_created",
    *_REAL_SUMMARY_METRICS,
))
_CANCELLATION_METRICS = tuple(
    f"{stage.replace('-', '_')}_{statistic}_ms"
    for stage in _CANCELLATION_STAGES for statistic in ("p95", "max")
)

# Closed, target-specific measure sets; these are names, not selected ceilings.
REQUIRED_METRICS = {
    target: {
        **{
            workload: (
                "latency_p50_ms", "latency_p95_ms", "latency_max_ms", *_CONTROLLED_RESOURCES,
                "physical_footprint_high_water_bytes" if target == _TARGETS[0]
                else "private_high_water_bytes",
                *(_CANCELLATION_METRICS if workload == "cancellation-close" else ()),
            )
            for workload in _CONTROLLED_WORKLOADS
        },
        _REAL_WORKLOAD: (
            "process_elapsed_ms", *(interval[0] for interval in _REAL_INTERVALS),
            "resident_current_bytes", "resident_process_peak_bytes",
            "physical_footprint_bytes" if target == _TARGETS[0] else "private_bytes",
            *_REAL_SUMMARY_METRICS,
        ),
    }
    for target in _TARGETS
}


def _table(value: object, name: str) -> dict:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be a table")
    return value


def _fields(value: object, expected: set | frozenset, name: str) -> dict:
    table = _table(value, name)
    if table.keys() != expected:
        raise ValueError(f"{name} has missing or unexpected fields")
    return table


def _uint(value: object, name: str) -> int:
    if type(value) is not int or not 0 <= value <= _U64_MAX:
        raise ValueError(f"{name} must be a nonnegative u64 integer")
    return value


def _number(value: object, name: str) -> int | float:
    if (type(value) not in (int, float) or value < 0
            or (type(value) is float and not _math.isfinite(value))):
        raise ValueError(f"{name} must be a finite nonnegative number")
    return value


def _text(value: object, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} must be nonempty text")
    return value


def _unit(measure: str) -> str:
    if measure.endswith("_ms"):
        return "milliseconds"
    return "bytes" if measure.endswith("_bytes") else "count"


def validate_profile(profile: dict, target: str) -> None:
    """Require all eight target-specific absolute-budget blocks before any run.

    Existing descriptive profile metadata is allowed. Acceptance and budget
    tables are closed; predicates, global budgets, and relative limits have no
    interpretation here. ADR identity and all binding checks belong to the runner.
    """
    if target not in _TARGETS:
        raise ValueError("unsupported workload budget target")
    profile = _table(profile, "profile document")
    if type(profile.get("format_version")) is not int or profile["format_version"] != 2:
        raise ValueError("unsupported workload budget profile version")
    benchmark = _table(profile.get("benchmark"), "benchmark")
    if benchmark.get("normative") is not True or benchmark.get("measurements_recorded") is not True:
        raise ValueError("numeric budgets must be normative and measurements recorded")
    facts = _table(profile.get("profile"), "profile")
    if facts.get("release_target") != target:
        raise ValueError("profile and execution target differ")
    acceptance = _fields(profile.get("acceptance"), {
        "status", "adr", "host_id", "controlled_executable_sha256", "real_executable_sha256",
    }, "acceptance")
    if acceptance["status"] != "accepted":
        raise ValueError("numeric budgets lack accepted status")
    _text(acceptance["host_id"], "acceptance.host_id")
    adr = _text(acceptance["adr"], "acceptance.adr")
    if not _re.fullmatch(r"docs/adr/[0-9]{4}-ocr-text-watch-workload-profiles\.md", adr):
        raise ValueError("acceptance.adr must name a four-digit OCR workload-profiles ADR under docs/adr")
    for key in ("controlled_executable_sha256", "real_executable_sha256"):
        digest = acceptance[key]
        if not isinstance(digest, str) or not _re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError(f"acceptance.{key} must be a canonical lowercase SHA-256")
    if "budget" in profile:
        raise ValueError("global budgets are not part of the OCR workload profile")
    blocks = profile.get("measurement")
    if not isinstance(blocks, list) or len(blocks) != len(_WORKLOADS):
        raise ValueError("exactly eight ordered workload measurement blocks are required")
    for workload, block in zip(_WORKLOADS, blocks):
        block = _table(block, "measurement")
        if block.get("workload") != workload:
            raise ValueError("missing, duplicate, unknown or reordered workload measurement")
        limits = block.get("budget")
        required = REQUIRED_METRICS[target][workload]
        if not isinstance(limits, list) or len(limits) != len(required):
            raise ValueError(f"{workload} requires its exact absolute-budget measure set")
        seen = set()
        for limit in limits:
            limit = _fields(limit, {
                "measure", "kind", "unit", "direction", "limit", "rationale",
            }, f"{workload} budget")
            measure = limit["measure"]
            if not isinstance(measure, str) or measure not in required or measure in seen:
                raise ValueError(f"{workload} has an unknown or duplicate budget measure")
            seen.add(measure)
            if limit["kind"] != "absolute" or limit["direction"] != "at_most":
                raise ValueError(f"{workload}.{measure} requires an absolute at_most budget")
            unit = _unit(measure)
            if limit["unit"] != unit:
                raise ValueError(f"{workload}.{measure} requires unit {unit}")
            _text(limit["rationale"], f"{workload}.{measure} rationale")
            if unit == "milliseconds":
                value = _number(limit["limit"], f"{workload}.{measure} limit")
                if value > _TIME_LIMIT_MAX_MS:
                    raise ValueError(f"{workload}.{measure} limit exceeds the process watchdog")
            else:
                _uint(limit["limit"], f"{workload}.{measure} limit")


def _records(measurement: dict, scope: str, count: int) -> list[dict]:
    measurement = _fields(measurement, {"complete", "scope", "records", "failures"}, "measurement")
    if (measurement["complete"] is not True or measurement["scope"] != scope
            or not isinstance(measurement["failures"], list) or measurement["failures"]):
        raise ValueError("complete measurements in the expected scope are required")
    records = measurement["records"]
    if not isinstance(records, list) or len(records) != count:
        raise ValueError(f"{scope} requires exactly {count} records from one process")
    return records


def _available_memory(records: list[dict], fields: tuple[str, ...]) -> tuple[str, ...]:
    current, peak, private, footprint = fields
    available = []
    for field in fields:
        values = [record[field] for record in records]
        if all(value is None for value in values):
            if field in (current, peak):
                raise ValueError(f"required {field} is unavailable")
            continue
        if any(value is None for value in values):
            raise ValueError(f"{field} is unavailable at some required observations")
        for value in values:
            _uint(value, field)
            if field != private and value == 0:
                raise ValueError(f"required {field} is zero")
        available.append(field)
    if private not in available and footprint not in available:
        raise ValueError("target-specific native memory is unavailable")
    return tuple(available)


def _quantiles_ns(samples: list[int]) -> tuple[int, int, int]:
    ordered = sorted(samples)
    count = len(ordered)
    return ordered[(count * 50 + 99) // 100 - 1], ordered[(count * 95 + 99) // 100 - 1], ordered[-1]


def _controlled(measurement: dict) -> dict[str, dict[str, int | float]]:
    records = _records(measurement, "controlled-workload-invocations", len(_INVOCATIONS))
    expected_fields = _CONTROLLED_INTEGERS | set(_CONTROLLED_MEMORY) | {"workload", "phase", "endpoints"}
    for expected, record in zip(_INVOCATIONS, records):
        record = _fields(record, expected_fields, "controlled observation")
        for field in _CONTROLLED_INTEGERS:
            _uint(record[field], field)
        if (record["workload"], record["phase"], record["iteration"]) != expected:
            raise ValueError("controlled workload, phase or iteration is missing, duplicate or reordered")
        if record["memory_samples"] == 0:
            raise ValueError("controlled memory sampling was not observed")
        if record["backend_input_max_bytes"] > record["backend_input_mapped_bytes"]:
            raise ValueError("backend input maximum exceeds observed total traffic")
        endpoints = record["endpoints"]
        stages = _ENDPOINTS[record["workload"]]
        if not isinstance(endpoints, list) or len(endpoints) != len(stages):
            raise ValueError("controlled invocation requires its exact ordered endpoint set")
        for stage, endpoint in zip(stages, endpoints):
            endpoint = _fields(endpoint, {"stage", "duration_ns"}, "controlled endpoint")
            if endpoint["stage"] != stage:
                raise ValueError("controlled endpoint is unknown, duplicate or reordered")
            _uint(endpoint["duration_ns"], "endpoint duration_ns")
    available_memory = _available_memory(records, _CONTROLLED_MEMORY)
    resources = (*_CONTROLLED_RESOURCES, *(field for field in available_memory
                                          if field not in _CONTROLLED_RESOURCES))
    metrics = {}
    offset = 0
    for workload in _CONTROLLED_WORKLOADS:
        count = 1 if workload == "cold-startup" else 22
        observations = records[offset:offset + count]
        offset += count
        samples = observations if count == 1 else observations[2:]
        p50, p95, maximum = _quantiles_ns([record["elapsed_ns"] for record in samples])
        row = {
            "latency_p50_ms": p50 / 1000000,
            "latency_p95_ms": p95 / 1000000,
            "latency_max_ms": maximum / 1000000,
        }
        row.update((field, max(record[field] for record in observations)) for field in resources)
        if workload == "cancellation-close":
            for index, stage in enumerate(_CANCELLATION_STAGES):
                # Mapping and inference are one invocation, not two independent samples.
                paired = [max(record["endpoints"][index]["duration_ns"],
                              record["endpoints"][index + len(_CANCELLATION_STAGES)]["duration_ns"])
                          for record in samples]
                _, p95, maximum = _quantiles_ns(paired)
                name = stage.replace("-", "_")
                row[f"{name}_p95_ms"] = p95 / 1000000
                row[f"{name}_max_ms"] = maximum / 1000000
        metrics[workload] = row
    return metrics


def _real(measurement: dict, duration_seconds: int | float) -> dict[str, dict[str, int | float]]:
    records = _records(measurement, _REAL_WORKLOAD, len(_REAL_STAGES) + 1)
    stages = records[:-1]
    clocks = {}
    previous = None
    for stage, record in zip(_REAL_STAGES, stages):
        record = _fields(record, {"kind", "stage", "elapsed_ns", *_REAL_MEMORY}, "real stage")
        if record["kind"] != "stage" or record["stage"] != stage:
            raise ValueError("real stages are missing, duplicate, unknown or reordered")
        elapsed = _uint(record["elapsed_ns"], "stage elapsed_ns")
        if previous is not None and elapsed < previous:
            raise ValueError("real common-clock elapsed_ns decreased")
        previous = elapsed
        clocks[stage] = elapsed
    available_memory = _available_memory(stages, _REAL_MEMORY)
    summary = _fields(records[-1], _REAL_SUMMARY_INTEGERS | {"kind", "scenario"}, "real summary")
    for field in _REAL_SUMMARY_INTEGERS:
        _uint(summary[field], field)
    if summary["kind"] != "summary" or summary["schema"] != 1 or summary["scenario"] != "transition":
        raise ValueError("real summary must describe the schema-1 transition workload")
    row = {"process_elapsed_ms": _number(duration_seconds * 1000, "supervisor elapsed milliseconds")}
    for measure, start, end in _REAL_INTERVALS:
        row[measure] = (clocks[end] - clocks[start]) / 1000000
    row.update((field, max(record[field] for record in stages)) for field in available_memory)
    row.update((field, summary[field]) for field in _REAL_SUMMARY_METRICS)
    return {_REAL_WORKLOAD: row}


def summarize_process(mode: str, measurement: dict, duration_seconds: float) -> dict[str, dict[str, int | float]]:
    """Summarize one complete process without pooling or zero-filling missing data.

    Normal controlled workloads retain exactly 20 latency samples after two
    warmups; controlled startup retains one. Resource maxima include warmups.
    Unavailable platform counterparts are omitted, never synthesized. Real clock
    intervals exclude the collector; process_elapsed_ms uses the supervisor.
    """
    duration_seconds = _number(duration_seconds, "supervisor duration_seconds")
    if mode == "controlled":
        return _controlled(measurement)
    if mode == _REAL_WORKLOAD:
        return _real(measurement, duration_seconds)
    raise ValueError("unsupported workload measurement mode")


def evaluate_process(profile: dict, target: str, mode: str, measurement: dict,
                     duration_seconds: float) -> dict:
    """Compare every required measure; malformed input raises, exceedance fails."""
    validate_profile(profile, target)
    metrics = summarize_process(mode, measurement, duration_seconds)
    comparisons = []
    first_failure = None
    for block in profile["measurement"]:
        workload = block["workload"]
        if workload not in metrics:
            continue
        limits = {limit["measure"]: limit for limit in block["budget"]}
        for measure in REQUIRED_METRICS[target][workload]:
            if measure not in metrics[workload]:
                raise ValueError(f"required {workload}.{measure} is unavailable for {target}")
            observed = metrics[workload][measure]
            limit = limits[measure]
            comparison = {
                "workload": workload, "measure": measure, "observed": observed,
                "unit": limit["unit"], "limit": limit["limit"], "passed": observed <= limit["limit"],
            }
            comparisons.append(comparison)
            if not comparison["passed"] and first_failure is None:
                first_failure = comparison
    return {
        "passed": first_failure is None, "metrics": metrics, "comparisons": comparisons,
        "first_failure": first_failure,
    }
