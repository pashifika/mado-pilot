"""Synthetic protocol regressions only; these fixtures are not workload evidence."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "measurements.py"
SPEC = importlib.util.spec_from_file_location("ocr_measurements", SCRIPT)
measurements = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(measurements)

APPLE = "aarch64-apple-darwin"
WINDOWS = "x86_64-pc-windows-msvc"
WORKLOADS = (
    "steady-nonmatch", "positive-consecutive", "slow-backend-saturation",
    "mixed-two-session", "retained-results", "cancellation-close",
)
ENDPOINTS = {
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
STAGES = (
    "process-start", "runtime-initialized", "provider-prepared", "detector-session-ready",
    "recognizer-session-ready", "engine-ready", "session-ready", "query-terminal",
    "logical-close-returned", "physical-ocr-zero", "parents-dropped", "retained-results-dropped",
)
SEMANTIC = (
    "ocr-text-watch: scenario=transition terminal=Matched sequence=1 regions=8 "
    "satisfying=1 confirmations=1 retained_bytes=2073600 cleanup=returned"
)


def row(prefix: str, fields: dict) -> str:
    return prefix + " " + " ".join(f"{key}={value}" for key, value in fields.items())


def field(line: str, name: str, value: str | None) -> str:
    tokens = line.split(" ")
    index = next(index for index, token in enumerate(tokens) if token.startswith(name + "="))
    if value is None:
        del tokens[index]
    else:
        tokens[index] = f"{name}={value}"
    return " ".join(tokens)


def record_index(lines: list[str], prefix: str, **fields) -> int:
    expected = {f"{key}={value}" for key, value in fields.items()}
    return next(index for index, line in enumerate(lines)
                if line.startswith(prefix + " ") and expected.issubset(line.split(" ")))


def controlled_lines(target: str = APPLE) -> list[str]:
    invocations = [(name, "warmup" if index < 2 else "sample", index)
                   for name in WORKLOADS for index in range(22)]
    invocations.append(("cold-startup", "sample", 0))
    lines = []
    for name, phase, index in invocations:
        suffix = f"workload={name} phase={phase} iteration={index}"
        fields = {
            "workload": name, "phase": phase, "iteration": index, "elapsed_ns": 100,
            "backend_input_mapped_bytes": 4147200, "backend_input_max_bytes": 2073600,
            "retained_read_mapped_bytes": 0, "mapped_cache_high_water_bytes": 2073600,
            "retained_source_high_water_bytes": 0, "retained_text_high_water_bytes": 0,
            "retained_index_high_water_bytes": 0,
            "separate_frame_high_water_bytes": 2073600 if name == "mixed-two-session" else 0,
            "resident_current_high_water_bytes": "Some(10000000)",
            "resident_process_peak_bytes": "Some(11000000)",
            "private_high_water_bytes": "Some(0)" if target == WINDOWS else "None",
            "physical_footprint_high_water_bytes": "Some(9000000)" if target == APPLE else "None",
            "memory_samples": 3, "physical_ocr_high_water": 1,
        }
        lines.append(f"# ocr-start {suffix}")
        lines.extend(row("# ocr-endpoint", {
            "workload": name, "stage": stage, "duration_ns": index * 100 + endpoint_index,
        }) for endpoint_index, stage in enumerate(ENDPOINTS[name]))
        lines.extend((
            f"# ocr-raw {suffix} elapsed_ns=100 semantic=passed work=QueryWorkMetrics {{ admitted: 2 }}",
            row("# ocr-measurement", fields),
        ))
    lines.extend((
        "ocr-text-watch-query: 6 workloads, 20 samples each, 0 oracle failure(s)",
        "ocr-text-watch-controlled-startup: 1 workloads, 1 samples each, 0 oracle failure(s)",
    ))
    return lines


def real_lines(target: str = APPLE) -> list[str]:
    lines = [SEMANTIC]
    for index, stage in enumerate(STAGES):
        lines.append(row("# ocr-real-stage", {
            "stage": stage, "elapsed_ns": (index // 2) * 10,
            "resident_current_bytes": "Some(10000000)",
            "resident_process_peak_bytes": "Some(11000000)",
            "private_bytes": "Some(0)" if target == WINDOWS else "None",
            "physical_footprint_bytes": "Some(9000000)" if target == APPLE else "None",
        }))
    lines.append(row("# ocr-real-summary", {
        "schema": 1, "scenario": "transition", "open_stages": 4,
        "detector_sessions_created": 1, "recognizer_sessions_created": 1,
        "physical_ocr_after_close": 1, "physical_ocr_final": 0,
        "retained_source_extent_bytes": 2073600, "retained_text_extent_bytes": 80,
        "retained_index_extent_bytes": 2, "retained_read_mapped_bytes": 4147200,
    }))
    return lines


class MeasurementProtocol(unittest.TestCase):
    def assert_incomplete(self, parser, lines, target=APPLE):
        result = parser("\n".join(lines), target)
        self.assertFalse(result["complete"], result)
        self.assertTrue(result["failures"], result)
        return result

    def test_controlled_requires_all_invocations_once_in_order(self):
        lines = controlled_lines()
        result = measurements.controlled_measurements("\n".join(lines), APPLE)
        self.assertTrue(result["complete"], result)
        self.assertEqual(len(result["records"]), 133)
        for record in result["records"]:
            self.assertEqual(
                [endpoint["stage"] for endpoint in record["endpoints"]],
                list(ENDPOINTS[record["workload"]]),
            )
        first = record_index(lines, "# ocr-measurement", workload="steady-nonmatch", iteration=0)
        second = record_index(lines, "# ocr-measurement", workload="steady-nonmatch", iteration=1)
        raw = record_index(lines, "# ocr-raw", workload="steady-nonmatch", iteration=0)
        cold = record_index(lines, "# ocr-start", workload="cold-startup")
        reordered = lines.copy()
        reordered[first], reordered[second] = reordered[second], reordered[first]
        before_raw = lines.copy()
        before_raw[raw], before_raw[first] = before_raw[first], before_raw[raw]
        variants = {
            "missing": lines[:first] + lines[first + 1:],
            "duplicate": lines[:first + 1] + [lines[first]] + lines[first + 1:],
            "reordered": reordered,
            "measurement-before-raw": before_raw,
            "missing-cold-startup": lines[:cold] + lines[-2:],
        }
        wrong_phase = lines.copy()
        sample = record_index(lines, "# ocr-measurement", workload="steady-nonmatch", iteration=2)
        wrong_phase[sample] = field(wrong_phase[sample], "phase", "warmup")
        variants["wrong-warmup-boundary"] = wrong_phase
        for name, changed in variants.items():
            with self.subTest(name=name):
                self.assert_incomplete(measurements.controlled_measurements, changed)

    def test_controlled_requires_same_invocation_semantic_success(self):
        lines = controlled_lines()
        start = record_index(lines, "# ocr-start", workload="steady-nonmatch", iteration=0)
        raw = record_index(lines, "# ocr-raw", workload="steady-nonmatch", iteration=0)
        variants = {
            "missing-start": lines[:start] + lines[start + 1:],
            "failed-raw": lines[:raw] + [field(lines[raw], "semantic", "failed")] + lines[raw + 1:],
            "ambiguous-raw": lines[:raw] + [lines[raw] + " semantic=passed"] + lines[raw + 1:],
            "foreign-raw": lines[:raw] + [field(lines[raw], "iteration", "1")] + lines[raw + 1:],
            "missing-summary": lines[:-1],
            "duplicate-summary": lines + [lines[-1]],
        }
        for name, changed in variants.items():
            with self.subTest(name=name):
                self.assert_incomplete(measurements.controlled_measurements, changed)

    def test_parsed_endpoint_durations_support_per_invocation_consumers(self):
        result = measurements.controlled_measurements("\n".join(controlled_lines()), APPLE)
        self.assertTrue(result["complete"], result)
        stages = ENDPOINTS["cancellation-close"][:5]
        for iteration, expected_ns in (
            (2, (205, 206, 207, 208, 209)),
            (3, (305, 306, 307, 308, 309)),
        ):
            with self.subTest(iteration=iteration):
                record = next(record for record in result["records"]
                              if record["workload"] == "cancellation-close"
                              and record["iteration"] == iteration)
                durations_ms = {
                    stage: max(endpoint["duration_ns"] for endpoint in record["endpoints"]
                               if endpoint["stage"] == stage) / 1_000_000
                    for stage in stages
                }
                self.assertEqual(
                    durations_ms,
                    {stage: duration / 1_000_000 for stage, duration in zip(stages, expected_ns)},
                )

    def test_controlled_endpoints_cannot_cross_invocation_boundaries(self):
        lines = controlled_lines()
        endpoint = record_index(lines, "# ocr-endpoint", workload="positive-consecutive")
        raw = record_index(lines, "# ocr-raw", workload="positive-consecutive", iteration=0)
        measured = record_index(lines, "# ocr-measurement", workload="positive-consecutive", iteration=0)
        next_raw = record_index(lines, "# ocr-raw", workload="positive-consecutive", iteration=1)
        variants = {
            "foreign-workload": (
                lines[:endpoint] + [field(lines[endpoint], "workload", "steady-nonmatch")]
                + lines[endpoint + 1:]
            ),
            "next-invocation-same-workload": (
                lines[:endpoint] + lines[endpoint + 1:next_raw]
                + [lines[endpoint]] + lines[next_raw:]
            ),
            "before-start": [lines[endpoint]] + lines,
            "after-raw": lines[:raw + 1] + [lines[endpoint]] + lines[raw + 1:],
            "between-invocations": lines[:measured + 1] + [lines[endpoint]] + lines[measured + 1:],
            "after-summaries": lines + [lines[endpoint]],
        }
        for name, changed in variants.items():
            with self.subTest(name=name):
                self.assert_incomplete(measurements.controlled_measurements, changed)

    def test_controlled_requires_complete_ordered_endpoint_pairs(self):
        lines = controlled_lines()
        start = record_index(lines, "# ocr-start", workload="cancellation-close", iteration=0)
        raw = record_index(lines, "# ocr-raw", workload="cancellation-close", iteration=0)
        endpoints = lines[start + 1:raw]
        variants = {
            "missing-endpoint": endpoints[:-1],
            "missing-inference-pair": endpoints[:5],
            "duplicate-inference-pair": endpoints + endpoints[5:],
            "reordered-inference-pair": endpoints[:5] + [endpoints[6], endpoints[5]] + endpoints[7:],
        }
        for name, changed in variants.items():
            with self.subTest(name=name):
                result = self.assert_incomplete(
                    measurements.controlled_measurements, lines[:start + 1] + changed + lines[raw:],
                )
                if name == "missing-endpoint":
                    record = next(record for record in result["records"]
                                  if record["workload"] == "cancellation-close"
                                  and record["iteration"] == 0)
                    self.assertEqual(record["elapsed_ns"], 100)
                    self.assertEqual(
                        [endpoint["stage"] for endpoint in record["endpoints"]],
                        list(ENDPOINTS["cancellation-close"][:-1]),
                    )
                    self.assertEqual(
                        [endpoint["duration_ns"] for endpoint in record["endpoints"]],
                        list(range(9)),
                    )
        retained_raw = record_index(lines, "# ocr-raw", workload="retained-results", iteration=0)
        unexpected = row("# ocr-endpoint", {
            "workload": "retained-results", "stage": "query-cancel", "duration_ns": 0,
        })
        self.assert_incomplete(
            measurements.controlled_measurements,
            lines[:retained_raw] + [unexpected] + lines[retained_raw:],
        )

    def test_controlled_rejects_missing_duplicate_and_unknown_fields(self):
        lines = controlled_lines()
        measured = record_index(lines, "# ocr-measurement", workload="steady-nonmatch", iteration=0)
        endpoint = record_index(lines, "# ocr-endpoint", workload="steady-nonmatch")
        for index, changed in (
            (measured, field(lines[measured], "retained_read_mapped_bytes", None)),
            (measured, lines[measured] + " elapsed_ns=100"),
            (measured, lines[measured] + " extra=1"),
            (measured, field(lines[measured], "memory_samples", "0")),
            (measured, field(lines[measured], "backend_input_max_bytes", "4147201")),
            (endpoint, field(lines[endpoint], "duration_ns", None)),
            (endpoint, lines[endpoint] + " duration_ns=100"),
            (endpoint, lines[endpoint] + " extra=1"),
            (endpoint, field(lines[endpoint], "stage", "unobserved")),
            (endpoint, " " + lines[endpoint]),
            (endpoint, lines[endpoint].replace("# ocr-endpoint", "# ocr-endpoints", 1)),
        ):
            with self.subTest(row=changed):
                self.assert_incomplete(
                    measurements.controlled_measurements, lines[:index] + [changed] + lines[index + 1:],
                )

    def test_mixed_workload_cannot_hide_its_caller_owned_source(self):
        lines = controlled_lines()
        index = record_index(lines, "# ocr-measurement", workload="mixed-two-session")
        lines[index] = field(lines[index], "separate_frame_high_water_bytes", "0")
        self.assert_incomplete(measurements.controlled_measurements, lines)

    def test_integer_wire_validity_is_not_a_latency_budget(self):
        lines = controlled_lines()
        measured = record_index(lines, "# ocr-measurement", workload="steady-nonmatch", iteration=0)
        endpoint = record_index(lines, "# ocr-endpoint", workload="steady-nonmatch")
        lines[measured] = field(lines[measured], "elapsed_ns", str((1 << 64) - 1))
        lines[endpoint] = field(lines[endpoint], "duration_ns", str((1 << 64) - 1))
        result = measurements.controlled_measurements("\n".join(lines), APPLE)
        self.assertTrue(result["complete"], result)
        self.assertEqual(set(result), {"complete", "scope", "records", "failures"})
        self.assertEqual(result["records"][0]["elapsed_ns"], (1 << 64) - 1)
        self.assertEqual(result["records"][0]["endpoints"][0]["duration_ns"] + 1, 1 << 64)
        for index, key in ((measured, "elapsed_ns"), (endpoint, "duration_ns")):
            for invalid in ("True", "NaN", str(1 << 64), "-1", "1.5", "01"):
                with self.subTest(field=key, value=invalid):
                    changed = lines.copy()
                    changed[index] = field(changed[index], key, invalid)
                    incomplete = self.assert_incomplete(measurements.controlled_measurements, changed)
                    if key == "duration_ns":
                        self.assertEqual(
                            incomplete["records"][0]["endpoints"],
                            result["records"][0]["endpoints"][1:],
                        )

    def test_both_targets_require_actual_memory_observations(self):
        for target, specific in ((APPLE, "physical_footprint"), (WINDOWS, "private")):
            for parser, fixture, marker, current, native in (
                (measurements.controlled_measurements, controlled_lines,
                 "# ocr-measurement workload=cold-startup",
                 "resident_current_high_water_bytes", specific + "_high_water_bytes"),
                (measurements.real_measurements, real_lines,
                 "# ocr-real-stage stage=retained-results-dropped",
                 "resident_current_bytes", specific + "_bytes"),
            ):
                lines = fixture(target)
                index = record_index(lines, marker)
                result = parser("\n".join(lines), target)
                self.assertTrue(result["complete"], result)
                for key, value in ((native, "None"), (current, "Some(0)"),
                                   ("resident_process_peak_bytes", "None"), (current, "0"),
                                   (current, "Some(NaN)"), (current, f"Some({1 << 64})")):
                    with self.subTest(target=target, parser=parser.__name__, key=key, value=value):
                        changed = lines.copy()
                        changed[index] = field(changed[index], key, value)
                        self.assert_incomplete(parser, changed, target)
                self.assert_incomplete(parser, lines, "x86_64-unknown-linux-gnu")

    def test_real_requires_all_stages_and_summary_once_in_order(self):
        lines = real_lines()
        result = measurements.real_measurements("\n".join(lines), APPLE)
        self.assertTrue(result["complete"], result)
        self.assertEqual([record["stage"] for record in result["records"][:-1]], list(STAGES))
        self.assertEqual(result["records"][-1]["kind"], "summary")
        variants = {
            "feature-absent": [SEMANTIC],
            "missing-stage": lines[:6] + lines[7:],
            "duplicate-stage": lines[:6] + [lines[5]] + lines[6:],
            "reordered": lines[:5] + [lines[6], lines[5]] + lines[7:],
            "missing-summary": lines[:-1],
            "duplicate-summary": lines + [lines[-1]],
            "premature-summary": lines[:-2] + [lines[-1], lines[-2]],
            "missing-semantic": lines[1:],
            "duplicate-semantic": [SEMANTIC] + lines,
        }
        decreasing = lines.copy()
        decreasing[5] = field(decreasing[5], "elapsed_ns", "0")
        variants["clock-decreased"] = decreasing
        for name, changed in variants.items():
            with self.subTest(name=name):
                self.assert_incomplete(measurements.real_measurements, changed)

    def test_real_summary_requires_observed_construction_and_retention(self):
        lines = real_lines()
        for key, value in (
            ("schema", "2"), ("scenario", "retina"), ("open_stages", "3"),
            ("detector_sessions_created", "2"), ("recognizer_sessions_created", "0"),
            ("physical_ocr_final", "1"), ("retained_source_extent_bytes", "2073599"),
            ("retained_index_extent_bytes", "0"), ("retained_text_extent_bytes", "0"),
            ("retained_read_mapped_bytes", "0"), ("physical_ocr_after_close", "False"),
        ):
            with self.subTest(key=key, value=value):
                self.assert_incomplete(
                    measurements.real_measurements, lines[:-1] + [field(lines[-1], key, value)],
                )

    def test_real_rejects_malformed_or_extra_observation_records(self):
        lines = real_lines()
        variants = {
            "missing-summary-field": lines[:-1] + [field(lines[-1], "detector_sessions_created", None)],
            "duplicate-summary-field": lines[:-1] + [lines[-1] + " schema=1"],
            "extra-stage-field": [lines[0], lines[1] + " extra=1"] + lines[2:],
            "unknown-stage": [lines[0], field(lines[1], "stage", "unobserved")] + lines[2:],
            "malformed-stage-prefix": [lines[0], " " + lines[1]] + lines[2:],
            "unknown-record": lines + ["# ocr-real-unknown value=0"],
        }
        for name, changed in variants.items():
            with self.subTest(name=name):
                self.assert_incomplete(measurements.real_measurements, changed)


if __name__ == "__main__":
    unittest.main()
