"""Parse scoped OCR observations, never numeric-budget or process-cleanup acceptance."""

from __future__ import annotations

import re as _re

__all__ = ("controlled_measurements", "real_measurements")

_U64_MAX = (1 << 64) - 1
_UINT = _re.compile(r"(?:0|[1-9][0-9]{0,19})\Z")
_WORKLOADS = (
    "steady-nonmatch", "positive-consecutive", "slow-backend-saturation",
    "mixed-two-session", "retained-results", "cancellation-close", "cold-startup",
)
_INVOCATIONS = tuple(
    (name, "warmup" if index < 2 else "sample", index)
    for name in _WORKLOADS[:-1] for index in range(22)
) + (("cold-startup", "sample", 0),)
_CONTROLLED_SUMMARIES = (
    "ocr-text-watch-query: 6 workloads, 20 samples each, 0 oracle failure(s)",
    "ocr-text-watch-controlled-startup: 1 workloads, 1 samples each, 0 oracle failure(s)",
)
_CONTROLLED_INTEGERS = frozenset((
    "iteration", "elapsed_ns", "backend_input_mapped_bytes", "backend_input_max_bytes",
    "retained_read_mapped_bytes", "mapped_cache_high_water_bytes",
    "retained_source_high_water_bytes", "retained_text_high_water_bytes",
    "retained_index_high_water_bytes", "separate_frame_high_water_bytes",
    "memory_samples", "physical_ocr_high_water",
))
_CONTROLLED_MEMORY = (
    "resident_current_high_water_bytes", "resident_process_peak_bytes",
    "private_high_water_bytes", "physical_footprint_high_water_bytes",
)
_CONTROLLED_ENDPOINTS = {
    "steady-nonmatch": ("publication-to-observation",) * 21 + ("query-cancel",),
    "positive-consecutive": ("first-positive-publication-to-terminal",),
    "slow-backend-saturation": (
        "eligible-expiry-advance-to-terminals", "held-release-to-physical-zero",
    ),
    "mixed-two-session": ("mapping-release-to-physical-zero",),
    "retained-results": (),
    "cancellation-close": (
        "independent-wait-cancel", "query-cancel", "session-close",
        "last-runtime-owner-drop", "held-release-to-actual-resource-retirement",
    ) * 2,
    "cold-startup": (
        "controlled-engine-construction", "controlled-session-open", "first-query-start",
        "first-positive-to-terminal", "logical-close",
    ),
}
_STAGES = (
    "process-start", "runtime-initialized", "provider-prepared", "detector-session-ready",
    "recognizer-session-ready", "engine-ready", "session-ready", "query-terminal",
    "logical-close-returned", "physical-ocr-zero", "parents-dropped", "retained-results-dropped",
)
_REAL_MEMORY = (
    "resident_current_bytes", "resident_process_peak_bytes", "private_bytes",
    "physical_footprint_bytes",
)
_REAL_SUMMARY_INTEGERS = frozenset((
    "schema", "open_stages", "detector_sessions_created", "recognizer_sessions_created",
    "physical_ocr_after_close", "physical_ocr_final", "retained_source_extent_bytes",
    "retained_text_extent_bytes", "retained_index_extent_bytes", "retained_read_mapped_bytes",
))
_REAL_SEMANTIC = (
    "ocr-text-watch: scenario=transition terminal=Matched sequence=1 regions=8 "
    "satisfying=1 confirmations=1 retained_bytes=2073600 cleanup=returned"
)
_TARGETS = ("aarch64-apple-darwin", "x86_64-pc-windows-msvc")


def _uint(value: str, field: str) -> int:
    if not _UINT.fullmatch(value):
        raise ValueError(f"{field} is not a canonical nonnegative u64")
    number = int(value)
    if number > _U64_MAX:
        raise ValueError(f"{field} exceeds u64")
    return number


def _record(line: str, prefix: str, integers: frozenset[str], memory: tuple[str, ...],
            labels: dict[str, tuple[str, ...]]) -> dict:
    if not line.startswith(prefix + " "):
        raise ValueError("malformed record prefix")
    fields = {}
    for token in line[len(prefix) + 1:].split(" "):
        key, separator, value = token.partition("=")
        if not separator or not value or key in fields:
            raise ValueError("malformed or duplicate field")
        fields[key] = value
    if fields.keys() != integers | set(memory) | labels.keys():
        raise ValueError("missing or unexpected fields")
    for key, allowed in labels.items():
        if fields[key] not in allowed:
            raise ValueError(f"invalid {key} label")
    for key in integers:
        fields[key] = _uint(fields[key], key)
    for key in memory:
        value = fields[key]
        if value == "None":
            fields[key] = None
        elif value.startswith("Some(") and value.endswith(")"):
            fields[key] = _uint(value[5:-1], key)
        else:
            raise ValueError(f"{key} is not Some(u64) or None")
    return fields


def _memory(record: dict, target: str, fields: tuple[str, ...]) -> None:
    current, peak, private, footprint = fields
    required = (current, peak, footprint if target == _TARGETS[0] else private)
    for key in required:
        value = record[key]
        if value is None:
            raise ValueError(f"required {key} is unavailable")
        if key != private and value == 0:
            raise ValueError(f"required {key} is zero")


def _result(scope: str, records: list, failures: list[str]) -> dict:
    return {"complete": not failures, "scope": scope, "records": records, "failures": failures}


def controlled_measurements(stdout: str, target: str) -> dict:
    """Require all 133 invocations and their semantic markers, not workload ceilings.

    Extents are logical views, not a deduplicated allocation total. OS high-water
    values cover existing checkpoint/retirement reads; process peak is not reset
    at the invocation boundary. Process supervision remains the runner's job.
    Endpoint durations are observed only inside their invocation's start/raw
    boundary; partial observations remain evidence, not zero-filled measurements.
    """
    records = []
    failures = []
    if target not in _TARGETS:
        failures.append("unsupported measurement target")
    event = 0
    summaries = 0
    kinds = ("start", "raw", "measurement")
    active_workload = None
    endpoints = []
    for line_number, line in enumerate(stdout.splitlines(), 1):
        candidate = line.lstrip()
        if candidate.startswith("# ocr-endpoint"):
            try:
                if active_workload is None:
                    raise ValueError("endpoint is outside its invocation start/raw boundary")
                order = _CONTROLLED_ENDPOINTS[active_workload]
                endpoint = _record(
                    line, "# ocr-endpoint", frozenset(("duration_ns",)), (),
                    {"workload": (active_workload,), "stage": order},
                )
                del endpoint["workload"]
                endpoints.append(endpoint)
                index = len(endpoints) - 1
                if index >= len(order) or endpoint["stage"] != order[index]:
                    raise ValueError("unexpected invocation endpoint order or extra endpoint")
            except ValueError as error:
                failures.append(f"line {line_number}: {error}")
            continue
        kind = next((kind for kind in kinds if candidate.startswith(f"# ocr-{kind}")), None)
        if kind is not None:
            invocation = event // 3
            expected_kind = kinds[event % 3]
            event += 1
            if expected_kind == "start":
                endpoints = []
            # A malformed boundary must not leave the prior endpoint window open.
            started_workload = active_workload
            active_workload = None
            try:
                if invocation >= len(_INVOCATIONS) or kind != expected_kind:
                    raise ValueError("unexpected invocation record order or duplicate record")
                name, phase, index = _INVOCATIONS[invocation]
                suffix = f"workload={name} phase={phase} iteration={index}"
                if kind == "start":
                    if line != f"# ocr-start {suffix}":
                        raise ValueError("unexpected workload, phase, iteration or start fields")
                    active_workload = name
                elif kind == "raw":
                    if not line.startswith(f"# ocr-raw {suffix} "):
                        raise ValueError("raw semantic row does not match its invocation")
                    if started_workload != name:
                        raise ValueError("raw row has no exact preceding invocation start")
                    semantics = [token for token in line.split(" ") if token.startswith("semantic=")]
                    if semantics != ["semantic=passed"]:
                        raise ValueError("raw invocation semantic success is missing or ambiguous")
                    if len(endpoints) != len(_CONTROLLED_ENDPOINTS[name]):
                        raise ValueError("required invocation endpoint rows are incomplete")
                else:
                    record = _record(
                        line, "# ocr-measurement", _CONTROLLED_INTEGERS, _CONTROLLED_MEMORY,
                        {"workload": _WORKLOADS, "phase": ("warmup", "sample")},
                    )
                    record["endpoints"] = endpoints
                    records.append(record)
                    if (record["workload"], record["phase"], record["iteration"]) != (name, phase, index):
                        raise ValueError("measurement does not match its ordered invocation")
                    _memory(record, target, _CONTROLLED_MEMORY)
                    if record["memory_samples"] == 0:
                        raise ValueError("memory sampling was not observed")
                    if record["backend_input_max_bytes"] > record["backend_input_mapped_bytes"]:
                        raise ValueError("backend input maximum exceeds observed total traffic")
                    if name == "mixed-two-session" and record["separate_frame_high_water_bytes"] < 2073600:
                        raise ValueError("mixed workload omits its caller-owned full source frame")
            except ValueError as error:
                failures.append(f"line {line_number}: {error}")
        elif candidate.startswith(("ocr-text-watch-query:", "ocr-text-watch-controlled-startup:")):
            if (event != 3 * len(_INVOCATIONS) or summaries >= len(_CONTROLLED_SUMMARIES)
                    or line != _CONTROLLED_SUMMARIES[summaries]):
                failures.append(f"line {line_number}: incomplete, duplicate or invalid semantic summary")
            summaries += 1
    if event != 3 * len(_INVOCATIONS) or len(records) != len(_INVOCATIONS):
        failures.append("required 133 ordered start/raw/measurement invocations are incomplete")
    if summaries != len(_CONTROLLED_SUMMARIES):
        failures.append("required controlled semantic summaries are incomplete")
    return _result("controlled-workload-invocations", records, failures)


def real_measurements(stdout: str, target: str) -> dict:
    """Require the transition's common-clock stages and observed native constructions.

    Session creation counts do not imply native quiescence. Physical OCR zero
    covers this engine's work, not ORT process-global resource release; the
    supervisor must independently confirm bounded whole-process cleanup.
    """
    records = []
    failures = []
    if target not in _TARGETS:
        failures.append("unsupported measurement target")
    stages = 0
    summaries = 0
    semantics = 0
    previous_elapsed = None
    for line_number, line in enumerate(stdout.splitlines(), 1):
        candidate = line.lstrip()
        try:
            if candidate.startswith("# ocr-real-stage"):
                index = stages
                stages += 1
                record = _record(
                    line, "# ocr-real-stage", frozenset(("elapsed_ns",)), _REAL_MEMORY,
                    {"stage": _STAGES},
                )
                records.append({"kind": "stage", **record})
                if summaries or index >= len(_STAGES) or record["stage"] != _STAGES[index]:
                    raise ValueError("unexpected stage order or duplicate stage")
                elapsed = record["elapsed_ns"]
                if previous_elapsed is not None and elapsed < previous_elapsed:
                    raise ValueError("common-clock elapsed_ns decreased")
                previous_elapsed = elapsed
                _memory(record, target, _REAL_MEMORY)
            elif candidate.startswith("# ocr-real-summary"):
                summaries += 1
                record = _record(
                    line, "# ocr-real-summary", _REAL_SUMMARY_INTEGERS, (),
                    {"scenario": ("transition", "negative", "retina")},
                )
                records.append({"kind": "summary", **record})
                if summaries != 1 or stages != len(_STAGES):
                    raise ValueError("summary must follow exactly 12 stages once")
                required = {
                    "schema": 1, "scenario": "transition", "open_stages": 4,
                    "detector_sessions_created": 1, "recognizer_sessions_created": 1,
                    "physical_ocr_final": 0, "retained_source_extent_bytes": 2073600,
                    "retained_index_extent_bytes": 2,
                }
                for key, value in required.items():
                    if record[key] != value:
                        raise ValueError(f"unexpected {key} observation")
                for key in ("retained_text_extent_bytes", "retained_read_mapped_bytes"):
                    if record[key] == 0:
                        raise ValueError(f"positive {key} observation is required")
            elif candidate.startswith("# ocr-real"):
                raise ValueError("unknown real measurement record")
            elif candidate.startswith("ocr-text-watch:"):
                semantics += 1
                if line != _REAL_SEMANTIC:
                    raise ValueError("required transition semantic success is absent")
        except ValueError as error:
            failures.append(f"line {line_number}: {error}")
    if stages != len(_STAGES) or summaries != 1 or len(records) != len(_STAGES) + 1:
        failures.append("required 12 stages and one measurement summary are incomplete")
    if semantics != 1:
        failures.append("required transition semantic success is missing or duplicate")
    return _result("real-cpu-cold-startup", records, failures)
