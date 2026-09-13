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
        lines.extend((
            f"# ocr-start {suffix}",
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

    def test_controlled_requires_all_invocations_once_in_order(self):
        lines = controlled_lines()
        result = measurements.controlled_measurements("\n".join(lines), APPLE)
        self.assertTrue(result["complete"], result)
        self.assertEqual(len(result["records"]), 133)
        variants = {
            "missing": lines[:2] + lines[3:],
            "duplicate": lines[:3] + [lines[2]] + lines[3:],
            "reordered": lines[:2] + [lines[5]] + lines[3:5] + [lines[2]] + lines[6:],
            "measurement-before-raw": [lines[0], lines[2], lines[1]] + lines[3:],
            "missing-cold-startup": lines[:-5] + lines[-2:],
        }
        wrong_phase = lines.copy()
        wrong_phase[8] = field(wrong_phase[8], "phase", "warmup")
        variants["wrong-warmup-boundary"] = wrong_phase
        for name, changed in variants.items():
            with self.subTest(name=name):
                self.assert_incomplete(measurements.controlled_measurements, changed)

    def test_controlled_requires_same_invocation_semantic_success(self):
        lines = controlled_lines()
        variants = {
            "missing-start": lines[1:],
            "failed-raw": [lines[0], field(lines[1], "semantic", "failed")] + lines[2:],
            "ambiguous-raw": [lines[0], lines[1] + " semantic=passed"] + lines[2:],
            "foreign-raw": [lines[0], field(lines[1], "iteration", "1")] + lines[2:],
            "missing-summary": lines[:-1],
            "duplicate-summary": lines + [lines[-1]],
        }
        for name, changed in variants.items():
            with self.subTest(name=name):
                self.assert_incomplete(measurements.controlled_measurements, changed)

    def test_controlled_rejects_missing_duplicate_and_unknown_fields(self):
        lines = controlled_lines()
        for changed in (
            field(lines[2], "retained_read_mapped_bytes", None),
            lines[2] + " elapsed_ns=100",
            lines[2] + " extra=1",
            field(lines[2], "memory_samples", "0"),
            field(lines[2], "backend_input_max_bytes", "4147201"),
        ):
            with self.subTest(row=changed):
                self.assert_incomplete(measurements.controlled_measurements, lines[:2] + [changed] + lines[3:])

    def test_mixed_workload_cannot_hide_its_caller_owned_source(self):
        lines = controlled_lines()
        index = next(index for index, line in enumerate(lines)
                     if line.startswith("# ocr-measurement workload=mixed-two-session "))
        lines[index] = field(lines[index], "separate_frame_high_water_bytes", "0")
        self.assert_incomplete(measurements.controlled_measurements, lines)

    def test_integer_wire_validity_is_not_a_latency_budget(self):
        lines = controlled_lines()
        lines[2] = field(lines[2], "elapsed_ns", str((1 << 64) - 1))
        result = measurements.controlled_measurements("\n".join(lines), APPLE)
        self.assertTrue(result["complete"], result)
        self.assertEqual(set(result), {"complete", "scope", "records", "failures"})
        self.assertEqual(result["records"][0]["elapsed_ns"], (1 << 64) - 1)
        for invalid in ("True", "NaN", str(1 << 64), "-1", "1.5", "01"):
            with self.subTest(value=invalid):
                changed = lines.copy()
                changed[2] = field(changed[2], "elapsed_ns", invalid)
                self.assert_incomplete(measurements.controlled_measurements, changed)

    def test_both_targets_require_actual_memory_observations(self):
        for target, specific in ((APPLE, "physical_footprint"), (WINDOWS, "private")):
            for parser, fixture, index, current, native in (
                (measurements.controlled_measurements, controlled_lines, -3,
                 "resident_current_high_water_bytes", specific + "_high_water_bytes"),
                (measurements.real_measurements, real_lines, -2,
                 "resident_current_bytes", specific + "_bytes"),
            ):
                lines = fixture(target)
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
