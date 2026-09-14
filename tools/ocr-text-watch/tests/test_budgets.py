"""Wholly synthetic numeric/protocol tests, not OCR measurements or budget evidence.

Every host, digest, ADR path, observation and ceiling below is fabricated solely
for deterministic tests. No fixture grants execution or production acceptance.
"""

from __future__ import annotations

from copy import deepcopy
import importlib.util
from pathlib import Path
import unittest

DIRECTORY = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("ocr_budgets", DIRECTORY / "budgets.py")
budgets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(budgets)
PARSER_SPEC = importlib.util.spec_from_file_location("ocr_budget_measurements", DIRECTORY / "measurements.py")
measurements = importlib.util.module_from_spec(PARSER_SPEC)
PARSER_SPEC.loader.exec_module(measurements)

APPLE = "aarch64-apple-darwin"
WINDOWS = "x86_64-pc-windows-msvc"
U64_MAX = (1 << 64) - 1
NS_PER_MS = 1000000
CONTROLLED = (
    "steady-nonmatch", "positive-consecutive", "slow-backend-saturation",
    "mixed-two-session", "retained-results", "cancellation-close", "cold-startup",
)
REAL = "real-cpu-cold-startup"
CANCELLATION = (
    "independent-wait-cancel", "query-cancel", "session-close", "last-runtime-owner-drop",
    "held-release-to-actual-resource-retirement",
)
ENDPOINTS = {
    "steady-nonmatch": ("publication-to-observation",) * 21 + ("query-cancel",),
    "positive-consecutive": ("first-positive-publication-to-terminal",),
    "slow-backend-saturation": ("eligible-expiry-advance-to-terminals", "held-release-to-physical-zero"),
    "mixed-two-session": ("mapping-release-to-physical-zero",),
    "retained-results": (),
    "cancellation-close": CANCELLATION * 2,
    "cold-startup": (
        "controlled-engine-construction", "controlled-session-open", "first-query-start",
        "first-positive-to-terminal", "logical-close",
    ),
}
STAGES = (
    "process-start", "runtime-initialized", "provider-prepared", "detector-session-ready",
    "recognizer-session-ready", "engine-ready", "session-ready", "query-terminal",
    "logical-close-returned", "physical-ocr-zero", "parents-dropped", "retained-results-dropped",
)
CONTROLLED_MEASURES = (
    "latency_p50_ms", "latency_p95_ms", "latency_max_ms", "backend_input_max_bytes",
    "mapped_cache_high_water_bytes", "retained_read_mapped_bytes", "retained_source_high_water_bytes",
    "retained_text_high_water_bytes", "retained_index_high_water_bytes", "separate_frame_high_water_bytes",
    "resident_current_high_water_bytes", "resident_process_peak_bytes", "physical_ocr_high_water",
)
REAL_MEASURES = (
    "process_elapsed_ms", "startup_to_session_ready_ms", "session_ready_to_query_terminal_ms",
    "query_terminal_to_logical_close_returned_ms", "logical_close_returned_to_physical_ocr_zero_ms",
    "parents_dropped_to_retained_results_dropped_ms", "process_start_to_retained_results_dropped_ms",
    "resident_current_bytes", "resident_process_peak_bytes", "retained_source_extent_bytes",
    "retained_text_extent_bytes", "retained_index_extent_bytes", "retained_read_mapped_bytes",
    "physical_ocr_after_close", "physical_ocr_final",
)


def required_measures(workload: str, target: str) -> tuple[str, ...]:
    if workload == REAL:
        memory = "physical_footprint_bytes" if target == APPLE else "private_bytes"
        return (*REAL_MEASURES, memory)
    memory = "physical_footprint_high_water_bytes" if target == APPLE else "private_high_water_bytes"
    endpoints = tuple(f"{stage.replace('-', '_')}_{statistic}_ms"
                      for stage in CANCELLATION for statistic in ("p95", "max"))
    return (*CONTROLLED_MEASURES, memory, *(endpoints if workload == "cancellation-close" else ()))


def synthetic_profile(target: str = APPLE) -> dict:
    """Illustrative ceilings only, never a candidate or reviewed product profile."""
    blocks = []
    for workload in (*CONTROLLED, REAL):
        limits = []
        for measure in required_measures(workload, target):
            unit = "milliseconds" if measure.endswith("_ms") else "bytes" if measure.endswith("_bytes") else "count"
            limits.append({
                "measure": measure, "kind": "absolute", "unit": unit, "direction": "at_most",
                "limit": {"milliseconds": 10000, "bytes": 100000000, "count": 8}[unit],
                "rationale": "Wholly synthetic test ceiling; not measurement or acceptance evidence.",
            })
        blocks.append({"workload": workload, "budget": limits})
    return {
        "format_version": 2,
        "benchmark": {"normative": True, "measurements_recorded": True, "id": "synthetic-not-evidence"},
        "profile": {"release_target": target},
        "acceptance": {
            "status": "accepted", "adr": "docs/adr/9999-ocr-text-watch-workload-profiles.md",
            "host_id": "synthetic-not-evidence-host",
            "controlled_executable_sha256": "a" * 64, "real_executable_sha256": "b" * 64,
        },
        "measurement": blocks,
    }


def synthetic_controlled(target: str = APPLE) -> dict:
    records = []
    for workload in CONTROLLED:
        for index in range(1 if workload == "cold-startup" else 22):
            phase = "sample" if workload == "cold-startup" or index >= 2 else "warmup"
            elapsed_ms = 7 if workload == "cold-startup" else 900 if phase == "warmup" else index - 1
            records.append({
                "workload": workload, "phase": phase, "iteration": index,
                "elapsed_ns": elapsed_ms * NS_PER_MS,
                "backend_input_mapped_bytes": 4096, "backend_input_max_bytes": 2048,
                "mapped_cache_high_water_bytes": 2048, "retained_read_mapped_bytes": 512,
                "retained_source_high_water_bytes": 1024, "retained_text_high_water_bytes": 64,
                "retained_index_high_water_bytes": 8,
                "separate_frame_high_water_bytes": 2073600 if workload == "mixed-two-session" else 0,
                "resident_current_high_water_bytes": 1000, "resident_process_peak_bytes": 2000,
                "private_high_water_bytes": 0 if target == WINDOWS else None,
                "physical_footprint_high_water_bytes": 1500 if target == APPLE else None,
                "memory_samples": 3, "physical_ocr_high_water": 1,
                "endpoints": [{"stage": stage, "duration_ns": (offset + 1) * NS_PER_MS}
                              for offset, stage in enumerate(ENDPOINTS[workload])],
            })
    return {"complete": True, "scope": "controlled-workload-invocations", "records": records, "failures": []}


def synthetic_real(target: str = APPLE) -> dict:
    # Nonzero common-clock origin and unequal gaps detect accidental absolute
    # clocks, sums, skipped stage intervals and substituted supervisor timing.
    clocks_ms = (100, 103, 110, 114, 122, 123, 131, 148, 153, 165, 174, 197)
    records = [{
        "kind": "stage", "stage": stage, "elapsed_ns": elapsed * NS_PER_MS,
        "resident_current_bytes": 1000, "resident_process_peak_bytes": 2000,
        "private_bytes": 0 if target == WINDOWS else None,
        "physical_footprint_bytes": 1500 if target == APPLE else None,
    } for stage, elapsed in zip(STAGES, clocks_ms)]
    records.append({
        "kind": "summary", "schema": 1, "scenario": "transition", "open_stages": 4,
        "detector_sessions_created": 1, "recognizer_sessions_created": 1,
        "retained_source_extent_bytes": 2073600, "retained_text_extent_bytes": 80,
        "retained_index_extent_bytes": 2, "retained_read_mapped_bytes": 4147200,
        "physical_ocr_after_close": 1, "physical_ocr_final": 0,
    })
    return {"complete": True, "scope": REAL, "records": records, "failures": []}


def observations(measurement: dict, workload: str, phase: str | None = None) -> list[dict]:
    return [record for record in measurement["records"] if record["workload"] == workload
            and (phase is None or record["phase"] == phase)]


def ceiling(profile: dict, workload: str, measure: str) -> dict:
    block = next(block for block in profile["measurement"] if block["workload"] == workload)
    return next(limit for limit in block["budget"] if limit["measure"] == measure)


def wire_row(prefix: str, record: dict, memory_fields: tuple[str, ...] = ()) -> str:
    fields = []
    for key, value in record.items():
        if key in ("kind", "endpoints"):
            continue
        if key in memory_fields:
            value = "None" if value is None else f"Some({value})"
        fields.append(f"{key}={value}")
    return prefix + " " + " ".join(fields)


def synthetic_wire(mode: str, measurement: dict) -> str:
    """Serialize synthetic parsed-shaped fixtures using the existing Rust wire."""
    lines = []
    if mode == "controlled":
        for record in measurement["records"]:
            suffix = f"workload={record['workload']} phase={record['phase']} iteration={record['iteration']}"
            lines.append(f"# ocr-start {suffix}")
            lines.extend(wire_row("# ocr-endpoint", {"workload": record["workload"], **endpoint})
                         for endpoint in record["endpoints"])
            lines.append(f"# ocr-raw {suffix} elapsed_ns={record['elapsed_ns']} "
                         "semantic=passed work=QueryWorkMetrics { admitted: 2 }")
            lines.append(wire_row("# ocr-measurement", record, (
                "resident_current_high_water_bytes", "resident_process_peak_bytes",
                "private_high_water_bytes", "physical_footprint_high_water_bytes",
            )))
        lines.extend((
            "ocr-text-watch-query: 6 workloads, 20 samples each, 0 oracle failure(s)",
            "ocr-text-watch-controlled-startup: 1 workloads, 1 samples each, 0 oracle failure(s)",
        ))
    else:
        lines.append("ocr-text-watch: scenario=transition terminal=Matched sequence=1 regions=8 "
                     "satisfying=1 confirmations=1 retained_bytes=2073600 cleanup=returned")
        for record in measurement["records"]:
            lines.append(wire_row("# ocr-real-" + record["kind"], record, (
                "resident_current_bytes", "resident_process_peak_bytes", "private_bytes", "physical_footprint_bytes",
            )))
    return "\n".join(lines)


class ProcessSummaries(unittest.TestCase):
    def test_process_local_nearest_rank_keeps_single_outlier_out_of_p95_not_max(self):
        first = synthetic_controlled()
        samples = observations(first, "steady-nonmatch", "sample")
        times_ms = [1000, *range(19, 0, -1)]
        for record, elapsed in zip(samples, times_ms):
            record["elapsed_ns"] = elapsed * NS_PER_MS
        first_metrics = budgets.summarize_process("controlled", first, 3.0)
        row = first_metrics["steady-nonmatch"]
        self.assertEqual((row["latency_p50_ms"], row["latency_p95_ms"], row["latency_max_ms"]), (10, 19, 1000))
        startup = first_metrics["cold-startup"]
        self.assertEqual((startup["latency_p50_ms"], startup["latency_p95_ms"], startup["latency_max_ms"]), (7, 7, 7))
        second = synthetic_controlled()
        for record in observations(second, "steady-nonmatch", "sample"):
            record["elapsed_ns"] = 50 * NS_PER_MS
        second_metrics = budgets.summarize_process("controlled", second, 4.0)
        self.assertEqual(second_metrics["steady-nonmatch"]["latency_p95_ms"], 50)
        self.assertEqual(first_metrics["steady-nonmatch"]["latency_p95_ms"], 19)

    def test_warmups_count_for_independent_resource_maxima_not_latency(self):
        measurement = synthetic_controlled()
        warmups = observations(measurement, "steady-nonmatch", "warmup")
        warmups[0].update({
            "elapsed_ns": 2000 * NS_PER_MS, "resident_current_high_water_bytes": 9000,
            "resident_process_peak_bytes": 11000, "physical_footprint_high_water_bytes": 7000,
            "backend_input_max_bytes": 3000, "backend_input_mapped_bytes": U64_MAX,
            "retained_source_high_water_bytes": 5000, "retained_text_high_water_bytes": 100,
            "physical_ocr_high_water": 2,
        })
        warmups[1].update({
            "mapped_cache_high_water_bytes": 8000, "retained_read_mapped_bytes": 6000,
            "retained_index_high_water_bytes": 20, "separate_frame_high_water_bytes": 4000,
        })
        row = budgets.summarize_process("controlled", measurement, 4)["steady-nonmatch"]
        self.assertEqual(row, {
            "latency_p50_ms": 10, "latency_p95_ms": 19, "latency_max_ms": 20,
            "backend_input_max_bytes": 3000, "mapped_cache_high_water_bytes": 8000,
            "retained_read_mapped_bytes": 6000, "retained_source_high_water_bytes": 5000,
            "retained_text_high_water_bytes": 100, "retained_index_high_water_bytes": 20,
            "separate_frame_high_water_bytes": 4000, "resident_current_high_water_bytes": 9000,
            "resident_process_peak_bytes": 11000, "physical_footprint_high_water_bytes": 7000,
            "physical_ocr_high_water": 2,
        })

    def test_endpoint_pairs_reduce_inside_each_invocation_before_twenty_sample_p95(self):
        measurement = synthetic_controlled()
        for record in observations(measurement, "cancellation-close"):
            for endpoint in record["endpoints"]:
                endpoint["duration_ns"] = (900 if record["phase"] == "warmup" else 1) * NS_PER_MS
        samples = observations(measurement, "cancellation-close", "sample")
        for index in range(5):
            samples[-2]["endpoints"][index]["duration_ns"] = (100 + index) * NS_PER_MS
            samples[-1]["endpoints"][index + 5]["duration_ns"] = (200 + index) * NS_PER_MS
        row = budgets.summarize_process("controlled", measurement, 3)["cancellation-close"]
        for index, stage in enumerate(CANCELLATION):
            with self.subTest(stage=stage):
                name = stage.replace("-", "_")
                self.assertEqual(row[name + "_p95_ms"], 100 + index)
                self.assertEqual(row[name + "_max_ms"], 200 + index)

    def test_real_uses_named_clock_deltas_and_separate_supervisor_time(self):
        measurement = synthetic_real()
        measurement["records"][2]["resident_current_bytes"] = 9000
        measurement["records"][8]["resident_process_peak_bytes"] = 12000
        measurement["records"][0]["physical_footprint_bytes"] = 7000
        row = budgets.summarize_process(REAL, measurement, 0.25)[REAL]
        self.assertEqual(row, {
            "process_elapsed_ms": 250, "startup_to_session_ready_ms": 31,
            "session_ready_to_query_terminal_ms": 17,
            "query_terminal_to_logical_close_returned_ms": 5,
            "logical_close_returned_to_physical_ocr_zero_ms": 12,
            "parents_dropped_to_retained_results_dropped_ms": 23,
            "process_start_to_retained_results_dropped_ms": 97,
            "resident_current_bytes": 9000, "resident_process_peak_bytes": 12000,
            "physical_footprint_bytes": 7000, "retained_source_extent_bytes": 2073600,
            "retained_text_extent_bytes": 80, "retained_index_extent_bytes": 2,
            "retained_read_mapped_bytes": 4147200, "physical_ocr_after_close": 1,
            "physical_ocr_final": 0,
        })

    def test_equal_adjacent_clocks_are_valid_zero_intervals(self):
        measurement = synthetic_real()
        for record in measurement["records"][:-1]:
            record["elapsed_ns"] = 123
        row = budgets.summarize_process(REAL, measurement, 1)[REAL]
        self.assertEqual(row["logical_close_returned_to_physical_ocr_zero_ms"], 0)
        self.assertEqual(row["process_start_to_retained_results_dropped_ms"], 0)
        self.assertEqual(row["process_elapsed_ms"], 1000)

    def test_platform_counterpart_is_unavailable_not_zero_filled(self):
        for target in (APPLE, WINDOWS):
            with self.subTest(target=target):
                controlled = budgets.summarize_process("controlled", synthetic_controlled(target), 1)
                real = budgets.summarize_process(REAL, synthetic_real(target), 1)
                for workload, row in {**controlled, **real}.items():
                    self.assertEqual(set(row), set(required_measures(workload, target)))
                if target == WINDOWS:
                    self.assertEqual(controlled["cold-startup"]["private_high_water_bytes"], 0)
                    self.assertEqual(real[REAL]["private_bytes"], 0)

    def test_existing_wire_parsers_feed_comparison_without_invented_metrics(self):
        for target in (APPLE, WINDOWS):
            for mode, fixture, parser, workload, measure, observed in (
                ("controlled", synthetic_controlled, measurements.controlled_measurements,
                 "cancellation-close", "held_release_to_actual_resource_retirement_p95_ms", 10),
                (REAL, synthetic_real, measurements.real_measurements,
                 REAL, "logical_close_returned_to_physical_ocr_zero_ms", 12),
            ):
                with self.subTest(target=target, mode=mode):
                    parsed = parser(synthetic_wire(mode, fixture(target)), target)
                    self.assertTrue(parsed["complete"], parsed["failures"])
                    result = budgets.evaluate_process(synthetic_profile(target), target, mode, parsed, 1)
                    self.assertTrue(result["passed"], result["first_failure"])
                    self.assertEqual(result["metrics"][workload][measure], observed)


class BudgetComparisons(unittest.TestCase):
    def test_every_required_measure_passes_at_its_exact_limit_on_each_target(self):
        for target in (APPLE, WINDOWS):
            profile = synthetic_profile(target)
            for mode, fixture in (("controlled", synthetic_controlled), (REAL, synthetic_real)):
                with self.subTest(target=target, mode=mode):
                    measurement = fixture(target)
                    metrics = budgets.summarize_process(mode, measurement, 0.25)
                    for workload, row in metrics.items():
                        for measure, observed in row.items():
                            ceiling(profile, workload, measure)["limit"] = observed
                    result = budgets.evaluate_process(profile, target, mode, measurement, 0.25)
                    self.assertTrue(result["passed"])
                    self.assertIsNone(result["first_failure"])
                    expected = {(workload, measure) for workload in metrics
                                for measure in required_measures(workload, target)}
                    self.assertEqual({(row["workload"], row["measure"]) for row in result["comparisons"]}, expected)
                    self.assertTrue(all(row["observed"] == row["limit"] and row["passed"]
                                        for row in result["comparisons"]))

    def test_one_nanosecond_over_millisecond_ceiling_is_a_failure_without_tolerance(self):
        profile = synthetic_profile()
        ceiling(profile, "steady-nonmatch", "latency_max_ms")["limit"] = 0.001
        measurement = synthetic_controlled()
        for record in observations(measurement, "steady-nonmatch", "sample"):
            record["elapsed_ns"] = 1000
        self.assertTrue(budgets.evaluate_process(profile, APPLE, "controlled", measurement, 1)["passed"])
        observations(measurement, "steady-nonmatch", "sample")[-1]["elapsed_ns"] = 1001
        result = budgets.evaluate_process(profile, APPLE, "controlled", measurement, 1)
        self.assertFalse(result["passed"])
        self.assertEqual(result["first_failure"], {
            "workload": "steady-nonmatch", "measure": "latency_max_ms", "observed": 0.001001,
            "unit": "milliseconds", "limit": 0.001, "passed": False,
        })

    def test_byte_comparisons_keep_u64_precision_and_report_all_exceedances(self):
        profile = synthetic_profile()
        measurement = synthetic_controlled()
        resource = "retained_source_high_water_bytes"
        observations(measurement, "steady-nonmatch")[0][resource] = U64_MAX
        ceiling(profile, "steady-nonmatch", resource)["limit"] = U64_MAX
        self.assertTrue(budgets.evaluate_process(profile, APPLE, "controlled", measurement, 1)["passed"])
        ceiling(profile, "steady-nonmatch", resource)["limit"] = U64_MAX - 1
        ceiling(profile, "cold-startup", "physical_ocr_high_water")["limit"] = 0
        result = budgets.evaluate_process(profile, APPLE, "controlled", measurement, 1)
        failures = [row for row in result["comparisons"] if not row["passed"]]
        self.assertFalse(result["passed"])
        self.assertEqual([(row["workload"], row["measure"]) for row in failures], [
            ("steady-nonmatch", resource), ("cold-startup", "physical_ocr_high_water"),
        ])
        self.assertEqual(result["first_failure"], failures[0])
        self.assertEqual(failures[0]["observed"], U64_MAX)
        self.assertEqual(failures[0]["limit"], U64_MAX - 1)
        self.assertIs(type(failures[0]["observed"]), int)

    def test_supervisor_over_watchdog_is_numeric_failure_not_malformed_observation(self):
        profile = synthetic_profile()
        ceiling(profile, REAL, "process_elapsed_ms")["limit"] = 300000
        result = budgets.evaluate_process(profile, APPLE, REAL, synthetic_real(), 301)
        self.assertFalse(result["passed"])
        self.assertEqual(result["first_failure"]["measure"], "process_elapsed_ms")
        self.assertEqual(result["first_failure"]["observed"], 301000)

    def test_target_memory_absence_is_invalid_not_a_passing_zero_comparison(self):
        for mode, fixture in (("controlled", synthetic_controlled), (REAL, synthetic_real)):
            with self.subTest(mode=mode):
                with self.assertRaises(ValueError):
                    budgets.evaluate_process(synthetic_profile(APPLE), APPLE, mode, fixture(WINDOWS), 1)
                with self.assertRaises(ValueError):
                    budgets.evaluate_process(synthetic_profile(WINDOWS), WINDOWS, mode, fixture(APPLE), 1)


class InvalidObservations(unittest.TestCase):
    def test_incomplete_wrong_scope_and_pooled_processes_are_rejected(self):
        for mode, fixture in (("controlled", synthetic_controlled), (REAL, synthetic_real)):
            for key, value in (
                ("complete", False), ("complete", 1), ("scope", "other-process"),
                ("failures", ["synthetic parser failure"]), ("failures", ()), ("records", None),
            ):
                with self.subTest(mode=mode, key=key, value=value):
                    measurement = fixture()
                    measurement[key] = value
                    with self.assertRaises(ValueError):
                        budgets.summarize_process(mode, measurement, 1)
            for records in (fixture()["records"][:-1], fixture()["records"] * 2):
                with self.subTest(mode=mode, count=len(records)):
                    measurement = fixture()
                    measurement["records"] = records
                    with self.assertRaises(ValueError):
                        budgets.summarize_process(mode, measurement, 1)
        with self.assertRaises(ValueError):
            budgets.summarize_process("unknown", synthetic_controlled(), 1)

    def test_invalid_supervisor_numbers_are_rejected_in_both_modes(self):
        for mode, fixture in (("controlled", synthetic_controlled), (REAL, synthetic_real)):
            for duration in (True, -1, "1", None, float("nan"), float("inf"), float("-inf")):
                with self.subTest(mode=mode, duration=duration):
                    with self.assertRaises(ValueError):
                        budgets.summarize_process(mode, fixture(), duration)
        with self.assertRaises(ValueError):
            budgets.summarize_process(REAL, synthetic_real(), 1e308)

    def test_controlled_invocation_identity_cannot_reclassify_or_replace_samples(self):
        for field, value in (
            ("phase", "warmup"), ("iteration", 1), ("iteration", True),
            ("workload", "cold-startup"), ("workload", "unknown"),
        ):
            with self.subTest(field=field, value=value):
                measurement = synthetic_controlled()
                measurement["records"][2][field] = value
                with self.assertRaises(ValueError):
                    budgets.summarize_process("controlled", measurement, 1)
        for replacement in ("duplicate", "reorder"):
            with self.subTest(replacement=replacement):
                measurement = synthetic_controlled()
                records = measurement["records"]
                if replacement == "duplicate":
                    records[3] = deepcopy(records[2])
                else:
                    records[2], records[3] = records[3], records[2]
                with self.assertRaises(ValueError):
                    budgets.summarize_process("controlled", measurement, 1)

    def test_malformed_resource_and_elapsed_values_never_become_zero(self):
        for field in ("elapsed_ns", "backend_input_max_bytes", "retained_text_high_water_bytes",
                      "physical_ocr_high_water", "resident_current_high_water_bytes"):
            for value in (True, -1, 1.0, "1", U64_MAX + 1, float("nan"), None):
                with self.subTest(field=field, value=value):
                    measurement = synthetic_controlled()
                    measurement["records"][0][field] = value
                    with self.assertRaises(ValueError):
                        budgets.summarize_process("controlled", measurement, 1)
        measurement = synthetic_controlled()
        del measurement["records"][0]["retained_read_mapped_bytes"]
        with self.assertRaises(ValueError):
            budgets.summarize_process("controlled", measurement, 1)
        for field, value in (("memory_samples", 0), ("backend_input_max_bytes", 4097)):
            measurement = synthetic_controlled()
            measurement["records"][0][field] = value
            with self.assertRaises(ValueError):
                budgets.summarize_process("controlled", measurement, 1)

    def test_memory_must_cover_every_observation_including_warmups_and_open_stages(self):
        for mode, fixture, field in (
            ("controlled", synthetic_controlled, "physical_footprint_high_water_bytes"),
            (REAL, synthetic_real, "physical_footprint_bytes"),
        ):
            for scope in ("one-observation", "all-observations"):
                with self.subTest(mode=mode, scope=scope):
                    measurement = fixture()
                    rows = measurement["records"] if mode == "controlled" else measurement["records"][:-1]
                    for record in rows if scope == "all-observations" else rows[:1]:
                        record[field] = None
                    with self.assertRaises(ValueError):
                        budgets.summarize_process(mode, measurement, 1)

    def test_endpoint_shape_order_and_duration_fail_closed(self):
        for variant in ("missing", "extra", "reordered", "unknown", "unexpected-field", "not-list"):
            with self.subTest(variant=variant):
                measurement = synthetic_controlled()
                record = observations(measurement, "cancellation-close")[0]
                endpoints = record["endpoints"]
                if variant == "missing":
                    endpoints.pop()
                elif variant == "extra":
                    endpoints.append(deepcopy(endpoints[-1]))
                elif variant == "reordered":
                    endpoints[0], endpoints[1] = endpoints[1], endpoints[0]
                elif variant == "unknown":
                    endpoints[0]["stage"] = "invented-cancel"
                elif variant == "unexpected-field":
                    endpoints[0]["workload"] = "cancellation-close"
                else:
                    record["endpoints"] = {}
                with self.assertRaises(ValueError):
                    budgets.summarize_process("controlled", measurement, 1)
        for duration in (True, -1, 1.0, "1", None, U64_MAX + 1):
            with self.subTest(duration=duration):
                measurement = synthetic_controlled()
                measurement["records"][0]["endpoints"][0]["duration_ns"] = duration
                with self.assertRaises(ValueError):
                    budgets.summarize_process("controlled", measurement, 1)

    def test_real_requires_ordered_common_clock_stages_and_exact_summary_shape(self):
        for index, field, value in (
            (0, "kind", "summary"), (0, "stage", "unknown"), (1, "stage", "process-start"),
            (1, "elapsed_ns", 99 * NS_PER_MS), (1, "elapsed_ns", True),
            (12, "kind", "stage"), (12, "schema", 2), (12, "schema", True),
            (12, "scenario", "negative"), (12, "retained_text_extent_bytes", 1.0),
            (12, "physical_ocr_final", None),
        ):
            with self.subTest(index=index, field=field, value=value):
                measurement = synthetic_real()
                measurement["records"][index][field] = value
                with self.assertRaises(ValueError):
                    budgets.summarize_process(REAL, measurement, 1)
        measurement = synthetic_real()
        measurement["records"][2], measurement["records"][3] = measurement["records"][3], measurement["records"][2]
        with self.assertRaises(ValueError):
            budgets.summarize_process(REAL, measurement, 1)
        measurement = synthetic_real()
        del measurement["records"][-1]["retained_read_mapped_bytes"]
        with self.assertRaises(ValueError):
            budgets.summarize_process(REAL, measurement, 1)


class ProfileSchema(unittest.TestCase):
    def test_acceptance_and_target_are_mandatory_before_numeric_comparison(self):
        variants = (
            (("format_version",), True), (("format_version",), 1),
            (("benchmark", "normative"), False), (("benchmark", "normative"), 1),
            (("benchmark", "measurements_recorded"), False),
            (("benchmark", "measurements_recorded"), "true"),
            (("profile", "release_target"), WINDOWS),
            (("acceptance", "status"), "prospective"), (("acceptance", "host_id"), " "),
            (("acceptance", "host_id"), None), (("acceptance",), []),
        )
        for path, value in variants:
            with self.subTest(path=path, value=value):
                profile = synthetic_profile()
                table = profile
                for key in path[:-1]:
                    table = table[key]
                table[path[-1]] = value
                with self.assertRaises(ValueError):
                    budgets.evaluate_process(profile, APPLE, "controlled", synthetic_controlled(), 1)
        for key in synthetic_profile()["acceptance"]:
            with self.subTest(missing=key):
                profile = synthetic_profile()
                del profile["acceptance"][key]
                with self.assertRaises(ValueError):
                    budgets.validate_profile(profile, APPLE)
        with self.assertRaises(ValueError):
            budgets.validate_profile(synthetic_profile(), "unsupported-target")
        with self.assertRaises(ValueError):
            budgets.validate_profile(None, APPLE)

    def test_acceptance_paths_and_digests_are_canonical_not_binding_substitutes(self):
        for path in ("", "docs/adr", "/docs/adr/9999.md", "docs/adr/../9999.md",
                     "docs/adr//9999.md", "docs/adr/./9999.md", "other/9999.md",
                     "docs\\adr\\9999.md", "docs/adr/9999.md/", "docs/adr/9999.md\n",
                     "docs/adr/9999-unrelated-accepted-profile.md",
                     "docs/adr/999-ocr-text-watch-workload-profiles.md",
                     "docs/adr/99999-ocr-text-watch-workload-profiles.md",
                     "docs/adr/9999-ocr-text-watch-workload-profiles.md\n"):
            with self.subTest(path=path):
                profile = synthetic_profile()
                profile["acceptance"]["adr"] = path
                with self.assertRaises(ValueError):
                    budgets.validate_profile(profile, APPLE)
        for field in ("controlled_executable_sha256", "real_executable_sha256"):
            for value in ("A" * 64, "g" * 64, "a" * 63, "a" * 65, "a" * 64 + "\n", "", True):
                with self.subTest(field=field, value=value):
                    profile = synthetic_profile()
                    profile["acceptance"][field] = value
                    with self.assertRaises(ValueError):
                        budgets.validate_profile(profile, APPLE)
        profile = synthetic_profile()
        profile["acceptance"]["approved"] = True
        with self.assertRaises(ValueError):
            budgets.validate_profile(profile, APPLE)

    def test_exact_eight_workload_blocks_are_required_even_for_one_mode(self):
        for variant in ("missing-real", "extra", "duplicate", "unknown", "reordered", "not-list", "not-table"):
            with self.subTest(variant=variant):
                profile = synthetic_profile()
                blocks = profile["measurement"]
                if variant == "missing-real":
                    blocks.pop()
                elif variant == "extra":
                    blocks.append(deepcopy(blocks[-1]))
                elif variant == "duplicate":
                    blocks[-1] = deepcopy(blocks[0])
                elif variant == "unknown":
                    blocks[-1]["workload"] = "unreviewed-startup"
                elif variant == "reordered":
                    blocks[0], blocks[1] = blocks[1], blocks[0]
                elif variant == "not-list":
                    profile["measurement"] = {}
                else:
                    blocks[-1] = None
                with self.assertRaises(ValueError):
                    budgets.evaluate_process(profile, APPLE, "controlled", synthetic_controlled(), 1)

    def test_exact_measure_sets_reject_duplicates_omissions_and_foreign_platform_limits(self):
        for workload in (*CONTROLLED, REAL):
            for variant in ("missing", "extra", "duplicate", "unknown", "foreign-platform", "not-list"):
                with self.subTest(workload=workload, variant=variant):
                    profile = synthetic_profile()
                    block = next(block for block in profile["measurement"] if block["workload"] == workload)
                    limits = block["budget"]
                    if variant == "missing":
                        limits.pop()
                    elif variant == "extra":
                        limits.append(deepcopy(limits[-1]))
                    elif variant == "duplicate":
                        limits[-1] = deepcopy(limits[0])
                    elif variant == "unknown":
                        limits[-1]["measure"] = "backend_input_mapped_bytes"
                    elif variant == "foreign-platform":
                        limits[-1]["measure"] = "private_bytes" if workload == REAL else "private_high_water_bytes"
                    else:
                        block["budget"] = {}
                    with self.assertRaises(ValueError):
                        budgets.validate_profile(profile, APPLE)

    def test_absolute_budget_rows_have_no_predicate_or_ignored_limit_fields(self):
        for field, value in (
            ("kind", "hard"), ("kind", "relative"), ("unit", "ms"), ("unit", "bytes"),
            ("direction", "at_least"), ("rationale", ""), ("rationale", " "),
            ("rationale", None), ("measure", []), ("predicate", "True"),
        ):
            with self.subTest(field=field, value=value):
                profile = synthetic_profile()
                ceiling(profile, "steady-nonmatch", "latency_p50_ms")[field] = value
                with self.assertRaises(ValueError):
                    budgets.validate_profile(profile, APPLE)
        for field in synthetic_profile()["measurement"][0]["budget"][0]:
            with self.subTest(missing=field):
                profile = synthetic_profile()
                del profile["measurement"][0]["budget"][0][field]
                with self.assertRaises(ValueError):
                    budgets.validate_profile(profile, APPLE)
        profile = synthetic_profile()
        profile["budget"] = []
        with self.assertRaises(ValueError):
            budgets.validate_profile(profile, APPLE)

    def test_limits_reject_bool_nonfinite_negative_and_wrong_integer_domains(self):
        for measure, invalid in (
            ("latency_p50_ms", (True, -1, "1", None, float("nan"), float("inf"), float("-inf"), 300000.001)),
            ("backend_input_max_bytes", (True, -1, "1", None, 1.0, U64_MAX + 1, float("nan"), float("inf"))),
            ("physical_ocr_high_water", (True, -1, "1", None, 1.0, U64_MAX + 1, float("nan"), float("inf"))),
        ):
            for value in invalid:
                with self.subTest(measure=measure, value=value):
                    profile = synthetic_profile()
                    ceiling(profile, "steady-nonmatch", measure)["limit"] = value
                    with self.assertRaises(ValueError):
                        budgets.validate_profile(profile, APPLE)


if __name__ == "__main__":
    unittest.main()
