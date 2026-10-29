"""Model-free workload admission and first-failure evidence regressions; no child executes."""

from __future__ import annotations

import io
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from test_measurements import controlled_lines, field

SCRIPT = Path(__file__).resolve().parents[1] / "workloads.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("ocr_workload_admission", SCRIPT)
workloads = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workloads)

TARGET = "aarch64-apple-darwin"
PROFILE = (workloads.ROOT / "docs/benchmarks" / f"ocr-text-watch-v3-{TARGET}.toml").read_bytes()


def controlled_output(*, measured=True) -> str:
    lines = controlled_lines(TARGET)
    if not measured:
        lines = [line for line in lines if not line.startswith("# ocr-measurement ")]
    return "\n".join(lines)


def real_output() -> str:
    stages = (
        "process-start", "runtime-initialized", "provider-prepared",
        "detector-session-ready", "recognizer-session-ready", "engine-ready",
        "session-ready", "query-terminal", "logical-close-returned",
        "physical-ocr-zero", "parents-dropped", "retained-results-dropped",
    )
    lines = [
        "ocr-text-watch: scenario=transition terminal=Matched sequence=1 regions=8 "
        "satisfying=1 confirmations=1 retained_bytes=2073600 cleanup=returned"
    ]
    lines.extend(
        f"# ocr-real-stage stage={stage} elapsed_ns={index * 1000} "
        "resident_current_bytes=Some(2048) resident_process_peak_bytes=Some(4096) "
        "private_bytes=None physical_footprint_bytes=Some(3072)"
        for index, stage in enumerate(stages)
    )
    lines.append(
        "# ocr-real-summary schema=1 scenario=transition open_stages=4 "
        "detector_sessions_created=1 recognizer_sessions_created=1 physical_ocr_after_close=0 "
        "physical_ocr_final=0 retained_source_extent_bytes=2073600 retained_text_extent_bytes=32 "
        "retained_index_extent_bytes=2 retained_read_mapped_bytes=4147200"
    )
    return "\n".join(lines)


def unavailable_output(mode: str) -> str:
    controlled = mode == "controlled"
    lines = (controlled_output() if controlled else real_output()).splitlines()
    fields = (
        ("resident_current_high_water_bytes", "resident_process_peak_bytes",
         "physical_footprint_high_water_bytes") if controlled else
        ("resident_current_bytes", "resident_process_peak_bytes", "physical_footprint_bytes")
    )
    prefix = "# ocr-measurement " if controlled else "# ocr-real-stage "
    for index, line in enumerate(lines):
        if line.startswith(prefix):
            for name in fields:
                line = field(line, name, "None")
            lines[index] = line
    return "\n".join(lines)


class WorkloadAdmission(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.image = self.root / "inert-native-image"
        self.image.write_bytes(b"approved native image, never executed")
        self.executable = self.root / "inert-executable"
        self.executable.write_bytes(b"inert child, never executed")
        scoped = workloads.tomllib.loads(PROFILE.decode("utf-8"))["bindings"]
        digest = workloads.run_replay.identity(self.executable)["sha256"]
        self.profile_bytes = PROFILE.replace(scoped["host_id"].encode(), b"execution-host")
        for name in ("controlled_executable_sha256", "real_executable_sha256"):
            self.profile_bytes = self.profile_bytes.replace(scoped[name].encode(), digest.encode())
        self.profile_path = self.root / "profile.toml"
        self.profile_path.write_bytes(self.profile_bytes)
        self.source_root = (self.root / "owned-source").resolve()
        canonical = self.source_root / "docs/benchmarks" / f"ocr-text-watch-v3-{TARGET}.toml"
        canonical.parent.mkdir(parents=True)
        canonical.write_bytes(self.profile_bytes)
        supervisor = self.source_root / "tools/native-release-profile/process_runner.py"
        supervisor.parent.mkdir(parents=True)
        supervisor.write_bytes(b"owned identity fixture; never imported or executed")
        self.host_path = self.root / "host.json"
        self.declared_host = {
            "release_target": TARGET, "host_id": "execution-host", "cpu": "declared CPU",
            "os_version": "declared OS/build", "memory_bytes": 1073741824,
        }
        self.host_path.write_text(json.dumps(self.declared_host), encoding="utf-8")
        self.native_path = self.root / "native.json"
        images = (workloads.run_replay.identity(path) for path in (self.image, self.executable))
        self.native_path.write_text(json.dumps({
            image["path"]: image["sha256"] for image in images
        }), encoding="utf-8")
        self.actual_host = {
            "release_target": TARGET, "host_id": "execution-host", "system": "observed system",
            "release": "observed release", "version": "observed build", "machine": "observed machine",
        }
        self.args = argparse.Namespace(
            mode="controlled", executable=self.executable, profile=None,
            host_record=self.host_path, native_images=self.native_path, corpus=None,
            output=self.root / "evidence", execute=True, enforce_budgets=False,
        )
        self.git_status = ""
        self.process_record = {
            "exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
            "stdout": controlled_output(), "stderr": "", "duration_seconds": 1.0,
            "launch_error": None,
        }
        self.patch(patch.object(workloads.run_replay, "git", side_effect=self.git))
        self.patch(patch.object(workloads.run_replay, "selected_target", return_value=TARGET))
        self.patch(patch.object(workloads, "observed_host", return_value=self.actual_host))
        self.patch(patch.object(workloads, "ROOT", self.source_root))
        self.patch(patch.object(workloads, "HARNESS", ()))
        self.patch(patch.dict(os.environ, {"PATH": "reviewed-search-path"}, clear=True))
        self.patch(patch.object(workloads.run_replay, "dependency_environment", side_effect=lambda base, path: {
            **base, "MADO_PILOT_OCR_DEPENDENCY_REPORT": str(path),
        }))
        self.observe = self.patch(patch.object(workloads.run_replay, "observe_dependencies", return_value={
            "matched": True, "observed": {workloads.run_replay.identity(self.image)["path"]: "test-double observation"},
        }))
        self.launch = self.patch(patch.object(workloads.run_replay, "run_process", return_value=self.process_record))

    def patch(self, patcher):
        result = patcher.start()
        self.addCleanup(patcher.stop)
        return result

    def git(self, *arguments):
        if arguments == ("rev-parse", "--show-toplevel"):
            return str(self.source_root)
        if arguments == ("rev-parse", "HEAD"):
            return "a" * 40
        if arguments == ("rev-parse", "HEAD^{tree}"):
            return "b" * 40
        if arguments[0] == "status":
            return self.git_status
        raise AssertionError("unexpected Git query in model-free test")

    def record(self, name="result.json"):
        return json.loads((self.args.output / name).read_text(encoding="utf-8"))

    def select_numeric_profile(self):
        selected = workloads.tomllib.loads(self.profile_bytes.decode("utf-8"))
        digest = workloads.run_replay.identity(self.executable)["sha256"]
        selected["bindings"] = {
            "host_id": "execution-host", "controlled_executable_sha256": digest,
            "real_executable_sha256": digest,
        }
        selected["measurement"] = []
        for name, measures in workloads.budgets.REQUIRED_METRICS[TARGET].items():
            limits = []
            for measure in measures:
                unit = "milliseconds" if measure.endswith("_ms") else "bytes" if measure.endswith("_bytes") else "count"
                limit = 1000 if unit == "milliseconds" else 67108864 if unit == "bytes" else 1
                if measure == "physical_ocr_final":
                    limit = 0
                limits.append({
                    "measure": measure, "kind": "absolute", "unit": unit,
                    "direction": "at_most", "limit": limit,
                    "rationale": "Synthetic admission fixture; not a product budget",
                })
            selected["measurement"].append({"workload": name, "budget": limits})
        self.numeric_profile = selected
        self.patch(patch.object(workloads, "profile", return_value=(
            selected, workloads.run_replay.identity(self.profile_path),
        )))
        self.args.enforce_budgets = True
        self.args.profile = self.profile_path

    def test_complete_scoped_cohort_passes_only_its_target_mode(self):
        self.select_numeric_profile()
        self.assertTrue(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["passed"])
        self.assertTrue(result["execution_passed"])
        self.assertFalse(result["stopped_by_enforcement"])
        self.assertTrue(result["empirical_targets"]["passed"])
        self.assertTrue(result["required_measurements_complete"])
        self.assertEqual(result["qualification_scope"], {"target": TARGET, "mode": "controlled"})
        self.assertEqual(result["unexecuted_processes"], [])
        self.assertEqual(result["executed_processes"], 3)

    def test_numeric_exceedance_retains_first_process_and_stops_later_launches(self):
        self.select_numeric_profile()
        limit = next(row for row in self.numeric_profile["measurement"][0]["budget"]
                     if row["measure"] == "latency_max_ms")
        limit["limit"] = 0
        argv = [
            "workloads.py", "--mode", self.args.mode, "--executable", str(self.executable),
            "--profile", str(self.profile_path), "--host-record", str(self.host_path),
            "--native-images", str(self.native_path), "--output", str(self.args.output),
            "--execute", "--enforce-budgets",
        ]
        with patch.object(sys, "argv", argv), patch.object(sys, "stderr", io.StringIO()):
            self.assertEqual(workloads.main(), 1)
        result = self.record()
        self.assertEqual(result["first_failure"]["kind"], "budget-exceeded")
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        self.assertTrue(result["execution_passed"])
        self.assertTrue(result["stopped_by_enforcement"])
        self.assertFalse(result["semantic_cohort_passed"])
        self.assertFalse(result["required_measurements_complete"])
        self.assertFalse(result["passed"])
        self.assertFalse(result["observation_completed"])
        self.assertTrue(result["empirical_targets"]["requested"])
        self.assertFalse(result["empirical_targets"]["passed"])
        self.assertTrue(result["empirical_targets"]["misses"])
        process = self.record("process-1.json")
        self.assertTrue(process["required_measurements_complete"])
        self.assertFalse(process["budget_passed"])
        self.assertFalse(process["budget_comparison"]["passed"])
        self.assertEqual(self.launch.call_count, 1)

    def test_last_process_numeric_failure_does_not_erase_measurement_completeness(self):
        self.select_numeric_profile()
        limit = next(row for row in self.numeric_profile["measurement"][0]["budget"]
                     if row["measure"] == "latency_max_ms")
        limit["limit"] = 0.001
        calls = 0

        def last_slow_sample(*args, **kwargs):
            nonlocal calls
            calls += 1
            observed = dict(self.process_record)
            if calls == 3:
                observed["stdout"] = observed["stdout"].replace(
                    "# ocr-measurement workload=steady-nonmatch phase=sample iteration=21 elapsed_ns=100 ",
                    "# ocr-measurement workload=steady-nonmatch phase=sample iteration=21 elapsed_ns=5000 ",
                )
            return observed

        self.launch.side_effect = last_slow_sample
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["semantic_cohort_passed"])
        self.assertTrue(result["required_measurements_complete"])
        self.assertFalse(result["passed"])
        self.assertEqual(result["first_failure"]["process"], 3)
        self.assertEqual(result["unexecuted_processes"], [])
        self.assertTrue(result["execution_passed"])
        self.assertFalse(result["stopped_by_enforcement"])
        self.assertFalse(result["empirical_targets"]["passed"])

    def test_profile_mutation_invalidates_complete_numeric_comparisons(self):
        self.select_numeric_profile()
        calls = 0

        def change_after_last_process(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 3:
                self.profile_path.write_bytes(self.profile_bytes + b"\n")
            return self.process_record

        self.launch.side_effect = change_after_last_process
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertFalse(result["passed"])
        self.assertFalse(result["execution_passed"])
        self.assertFalse(result["stopped_by_enforcement"])
        self.assertIsNone(result["empirical_targets"]["passed"])
        self.assertEqual(result["first_failure"]["stage"], "final-bindings")
        self.assertTrue(self.record("process-3.json")["budget_passed"])

    def test_wrong_profile_host_or_executable_prevents_observation_and_enforcement(self):
        self.select_numeric_profile()
        for enforce in (False, True):
            for field, wrong in (
                ("host_id", "different-host"), ("controlled_executable_sha256", "0" * 64),
            ):
                with self.subTest(enforce=enforce, field=field):
                    self.args.enforce_budgets = enforce
                    self.args.output = self.root / f"{field}-{enforce}"
                    original = self.numeric_profile["bindings"][field]
                    self.numeric_profile["bindings"][field] = wrong
                    stderr = io.StringIO()
                    with patch.object(sys, "stderr", stderr):
                        self.assertFalse(workloads.execute(self.args))
                    self.assertEqual(self.record()["first_failure"]["stage"], "preflight-profile-bindings")
                    self.assertFalse(self.record()["passed"])
                    self.launch.assert_not_called()
                    reason = (
                        "execution host differs from the scoped profile host"
                        if field == "host_id" else "executable differs from the scoped profile candidate"
                    )
                    self.assertIn(f"OCR profile refused: {reason}", stderr.getvalue())
                    self.assertEqual(self.record()["attempted_processes"], 0)
                    self.assertFalse(self.record()["execution_passed"])
                    self.assertFalse(self.record()["stopped_by_enforcement"])
                    self.assertTrue(self.record()["empirical_targets"]["requested"])
                    self.assertIsNone(self.record()["empirical_targets"]["passed"])
                    self.numeric_profile["bindings"][field] = original

    def test_observation_retains_target_misses_without_failing_or_qualifying(self):
        self.select_numeric_profile()
        self.args.enforce_budgets = False
        self.numeric_profile["measurement"][0]["budget"][0]["limit"] = 0
        self.assertTrue(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["execution_passed"])
        self.assertTrue(result["observation_completed"])
        self.assertFalse(result["passed"])
        self.assertFalse(result["stopped_by_enforcement"])
        self.assertFalse(result["empirical_targets"]["passed"])
        self.assertEqual(len(result["empirical_targets"]["misses"]), 3)
        self.assertEqual(result["numeric_budgets"], "not-enforced")
        self.assertIsNone(result["first_failure"])
        self.assertEqual(self.launch.call_count, 3)

    def test_observation_cleanup_failure_is_non_success_even_with_complete_measurements(self):
        self.process_record["cleanup_ok"] = False
        self.assertFalse(workloads.execute(self.args))
        self.assertFalse(self.record()["execution_passed"])
        self.assertFalse(self.record()["observation_completed"])
        self.assertEqual(self.record()["first_failure"]["kind"], "process-failure")
        self.assertEqual(self.launch.call_count, 1)

    def test_profile_free_observation_records_own_identities_and_finite_settings(self):
        self.assertTrue(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["execution_passed"])
        self.assertTrue(result["observation_completed"])
        self.assertFalse(result["empirical_targets"]["requested"])
        self.assertIsNone(result["empirical_targets"]["passed"])
        self.assertFalse(result["passed"])
        self.assertEqual(result["measurement_gaps"], [])
        self.assertEqual(self.record("process-1.json")["measurement_gaps"], [])
        plan = self.record("plan.json")
        self.assertNotIn("profile", plan["bindings"]["documents"])
        self.assertEqual(plan["host"]["observed"]["host_id"], "execution-host")
        self.assertEqual(plan["workload_settings"]["process_timeout_seconds"], 300)
        self.assertEqual(plan["bindings"]["artifacts"]["executable"]["sha256"],
                         workloads.run_replay.identity(self.executable)["sha256"])

    def test_profile_free_observation_retains_nullable_os_memory_on_each_workload_mode(self):
        for mode, count in (("controlled", 3), ("real-cpu-cold-startup", 5)):
            with self.subTest(mode=mode):
                self.args.mode = mode
                self.args.corpus = self.root if mode != "controlled" else None
                self.args.output = self.root / mode
                self.process_record["stdout"] = unavailable_output(mode)
                with patch.object(workloads.run_replay, "canonical_environment", return_value=dict(os.environ)), \
                        patch.object(workloads.run_replay, "inputs", return_value={"runtime": workloads.run_replay.identity(self.image)}):
                    self.assertTrue(workloads.execute(self.args))
                result = self.record()
                self.assertTrue(result["execution_passed"])
                self.assertTrue(result["observation_completed"])
                self.assertTrue(result["required_measurements_complete"])
                self.assertFalse(result["empirical_targets"]["requested"])
                self.assertEqual(result["empirical_targets"]["unavailable"], [])
                self.assertEqual(result["executed_processes"], count)
                self.assertFalse(result["apparatus_invalid"])
                self.assertIsNone(result["first_failure"])
                record = self.record("process-1.json")["measurements"]["records"][0]
                current = "resident_current_high_water_bytes" if mode == "controlled" else "resident_current_bytes"
                self.assertIsNone(record[current])
                self.assertIsNone(record["resident_process_peak_bytes"])
                process = self.record("process-1.json")
                gaps = process["measurement_gaps"]
                self.assertTrue(gaps)
                self.assertEqual(gaps, process["measurements"]["measurement_gaps"])
                native = "physical_footprint_high_water_bytes" if mode == "controlled" else "physical_footprint_bytes"
                self.assertEqual({gap["field"] for gap in gaps},
                                 {current, "resident_process_peak_bytes", native})
                self.assertTrue(all(gap["target"] == TARGET and gap["reason"].strip() for gap in gaps))
                self.assertEqual(
                    [gap for gap in result["measurement_gaps"] if gap["process"] == 1],
                    [{"process": 1, **gap} for gap in gaps],
                )
                self.assertEqual({gap["process"] for gap in result["measurement_gaps"]},
                                 set(range(1, count + 1)))
                if mode == "controlled":
                    self.assertEqual((gaps[0]["workload"], gaps[0]["phase"], gaps[0]["iteration"]),
                                     ("steady-nonmatch", "warmup", 0))
                else:
                    self.assertEqual(gaps[0]["stage"], "process-start")
                self.assertEqual(gaps[0]["record_index"], 0)

    def test_scoped_observation_retains_unavailable_comparisons_without_false_pass(self):
        self.select_numeric_profile()
        self.args.enforce_budgets = False
        self.process_record["stdout"] = unavailable_output("controlled")
        self.assertTrue(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["execution_passed"])
        self.assertTrue(result["observation_completed"])
        self.assertIsNone(result["empirical_targets"]["passed"])
        self.assertTrue(result["empirical_targets"]["unavailable"])
        self.assertEqual({entry["process"] for entry in result["empirical_targets"]["unavailable"]}, {1, 2, 3})
        self.assertEqual(result["empirical_targets"]["misses"], [])
        comparison = self.record("process-1.json")["budget_comparison"]
        self.assertFalse(comparison["complete"])
        self.assertNotIn("resident_current_high_water_bytes", comparison["metrics"]["steady-nonmatch"])
        self.assertIsNone(result["first_failure"])
        self.assertEqual(self.launch.call_count, 3)

    def test_scoped_observation_keeps_known_misses_alongside_unavailable_memory(self):
        self.select_numeric_profile()
        self.args.enforce_budgets = False
        self.process_record["stdout"] = unavailable_output("controlled")
        self.numeric_profile["measurement"][0]["budget"][0]["limit"] = 0
        self.assertTrue(workloads.execute(self.args))
        result = self.record()
        self.assertFalse(result["empirical_targets"]["passed"])
        self.assertTrue(result["empirical_targets"]["misses"])
        self.assertTrue(result["empirical_targets"]["unavailable"])
        self.assertTrue(result["observation_completed"])
        self.assertIsNone(result["first_failure"])
        self.assertEqual(self.launch.call_count, 3)

    def test_enforcement_rejects_unavailable_metrics_and_retains_any_known_misses(self):
        self.select_numeric_profile()
        self.process_record["stdout"] = unavailable_output("controlled")
        limit = self.numeric_profile["measurement"][0]["budget"][0]
        original = limit["limit"]
        for known_miss in (False, True):
            with self.subTest(known_miss=known_miss):
                self.launch.reset_mock()
                self.args.output = self.root / f"unavailable-{known_miss}"
                limit["limit"] = 0 if known_miss else original
                self.assertFalse(workloads.execute(self.args))
                result = self.record()
                self.assertFalse(result["passed"])
                self.assertTrue(result["execution_passed"])
                self.assertTrue(result["stopped_by_enforcement"])
                self.assertFalse(result["required_measurements_complete"])
                self.assertFalse(result["apparatus_invalid"])
                self.assertEqual(result["unexecuted_processes"], [2, 3])
                self.assertIn("budget-unavailable", {failure["kind"] for failure in result["failures"]})
                self.assertTrue(result["empirical_targets"]["unavailable"])
                if known_miss:
                    self.assertFalse(result["empirical_targets"]["passed"])
                    self.assertTrue(result["empirical_targets"]["misses"])
                else:
                    self.assertIsNone(result["empirical_targets"]["passed"])
                    self.assertEqual(result["first_failure"]["kind"], "budget-unavailable")
                self.assertEqual(self.launch.call_count, 1)

    def test_observation_still_rejects_malformed_or_zero_memory(self):
        for value in ("Some(0)", "Some(NaN)", "0"):
            with self.subTest(value=value):
                self.launch.reset_mock()
                self.args.output = self.root / f"malformed-memory-{value}"
                self.process_record["stdout"] = unavailable_output("controlled").replace(
                    "resident_current_high_water_bytes=None",
                    f"resident_current_high_water_bytes={value}", 1,
                )
                self.assertFalse(workloads.execute(self.args))
                self.assertFalse(self.record()["execution_passed"])
                self.assertFalse(self.record()["observation_completed"])
                self.assertEqual(self.record()["first_failure"]["kind"], "measurement-incomplete")
                self.assertEqual(self.launch.call_count, 1)

    def test_budget_stopped_prefix_cannot_pass_execution_after_final_identity_failure(self):
        self.select_numeric_profile()
        self.numeric_profile["measurement"][0]["budget"][0]["limit"] = 0

        def change_profile(*args, **kwargs):
            self.profile_path.write_bytes(self.profile_bytes + b"\n")
            return self.process_record

        self.launch.side_effect = change_profile
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["stopped_by_enforcement"])
        self.assertFalse(result["execution_passed"])
        self.assertFalse(result["identity_unchanged"])
        self.assertFalse(result["empirical_targets"]["passed"])
        self.assertEqual(result["final_binding_check"]["failure"]["stage"], "final-bindings")

    def test_real_mode_checks_its_own_executable_binding_before_launch(self):
        self.select_numeric_profile()
        self.args.mode = "real-cpu-cold-startup"
        self.args.corpus = self.root
        self.numeric_profile["bindings"]["real_executable_sha256"] = "0" * 64
        with patch.object(workloads.run_replay, "canonical_environment", return_value=dict(os.environ)), \
                patch.object(workloads.run_replay, "inputs", return_value={"runtime": workloads.run_replay.identity(self.image)}):
            self.assertFalse(workloads.execute(self.args))
        self.assertEqual(self.record()["first_failure"]["stage"], "preflight-profile-bindings")
        self.launch.assert_not_called()

    def test_current_profile_needs_no_adr_archive_or_tuning_prose(self):
        self.args.profile = self.profile_path
        self.assertFalse((self.source_root / "docs/adr").exists())
        self.assertFalse((self.source_root / "rasen").exists())
        self.assertFalse((self.source_root / "docs/performance").exists())
        self.assertTrue(workloads.execute(self.args))
        self.assertEqual(self.record()["schema_version"], 5)
        documents = self.record("plan.json")["bindings"]["documents"]
        self.assertEqual(set(documents), {"profile", "host_record", "native_manifest"})


    def test_unapproved_executable_cannot_reach_process_launch(self):
        approved = json.loads(self.native_path.read_text())
        del approved[workloads.run_replay.identity(self.executable)["path"]]
        self.native_path.write_text(json.dumps(approved), encoding="utf-8")
        self.assertFalse(workloads.execute(self.args))
        self.assertTrue(self.record()["apparatus_invalid"])
        self.launch.assert_not_called()

    def test_changed_behavioral_profile_cannot_claim_the_fixed_workload(self):
        self.args.profile = self.profile_path
        self.profile_path.write_bytes(self.profile_bytes.replace(b"confirmations = 3", b"confirmations = 1"))
        self.assertFalse(workloads.execute(self.args))
        self.assertEqual(self.record()["first_failure"]["stage"], "preflight-profile")
        self.launch.assert_not_called()

    def test_dirty_controlled_source_is_retained_without_launch(self):
        self.git_status = " M crates/runtime/src/lib.rs"
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["apparatus_invalid"])
        self.assertEqual(result["first_failure"]["stage"], "preflight-source")
        self.assertEqual(result["unexecuted_processes"], [1, 2, 3])
        self.launch.assert_not_called()

    def test_foreign_git_root_cannot_qualify_the_actual_apparatus(self):
        self.source_root = self.root
        self.assertFalse(workloads.execute(self.args))
        self.assertEqual(self.record()["first_failure"]["stage"], "preflight-source")
        self.launch.assert_not_called()

    def test_copied_host_record_is_rejected_and_declarations_are_not_observations(self):
        self.declared_host["host_id"] = "another-host"
        self.host_path.write_text(json.dumps(self.declared_host), encoding="utf-8")
        self.assertFalse(workloads.execute(self.args))
        self.assertEqual(self.record()["first_failure"]["stage"], "preflight-host")
        self.launch.assert_not_called()
        self.declared_host["host_id"] = "execution-host"
        self.host_path.write_text(json.dumps(self.declared_host), encoding="utf-8")
        evidence, _ = workloads.host(self.host_path, TARGET)
        self.assertEqual(evidence["observed"]["version"], "observed build")
        self.assertNotIn("cpu", evidence["observed"])
        self.assertNotIn("memory_bytes", evidence["observed"])
        self.assertEqual(set(evidence["unverified_declared_fields"]), {"cpu", "memory_bytes", "os_version"})

    def test_profile_replaced_after_read_keeps_consumed_digest_and_blocks_launch(self):
        self.args.profile = self.profile_path
        read_document = workloads.run_replay.read_document
        replaced = False

        def replace_after_read(path, maximum_bytes=1048576):
            nonlocal replaced
            result = read_document(path, maximum_bytes)
            if Path(path).resolve() == self.profile_path.resolve() and not replaced:
                replaced = True
                self.profile_path.write_bytes(self.profile_bytes.replace(b"sample_count = 20", b"sample_count = 21"))
            return result

        with patch.object(workloads.run_replay, "read_document", side_effect=replace_after_read):
            self.assertFalse(workloads.execute(self.args))
        plan = self.record("plan.json")
        self.assertEqual(plan["bindings"]["documents"]["profile"]["sha256"], hashlib.sha256(self.profile_bytes).hexdigest())
        self.assertEqual(self.record()["first_failure"]["stage"], "before-process-bindings")
        self.assertEqual(self.record("process-1.json")["execution_state"], "not-attempted")
        self.launch.assert_not_called()

    def test_metadata_admission_refuses_nonregular_and_oversized_inputs(self):
        self.args.profile = self.profile_path
        for path in (self.root, self.root / "oversized.toml"):
            with self.subTest(path=path.name):
                if path != self.root:
                    path.write_bytes(b"x" * 1048577)
                self.args.profile = path
                self.args.output = self.root / ("oversized-evidence" if path != self.root else "directory-evidence")
                self.assertFalse(workloads.execute(self.args))
                self.assertEqual(self.record()["first_failure"]["stage"], "preflight-profile")
                self.assertEqual(self.record()["unexecuted_processes"], [1, 2, 3])
        self.launch.assert_not_called()

    def test_between_process_exception_retains_prefix_and_does_not_replace_first_failure(self):
        def remove_host(*args, **kwargs):
            self.host_path.unlink()
            return self.process_record

        self.launch.side_effect = remove_host
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertEqual(result["executed_processes"], 1)
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        self.assertEqual(result["first_failure"]["stage"], "before-process-bindings")
        self.assertEqual(result["first_failure"]["process"], 2)
        self.assertEqual(result["first_failure"]["exception_type"], "FileNotFoundError")
        self.assertEqual(self.record("apparatus-invalid.json"), result["first_failure"])
        self.assertEqual(result["final_binding_check"]["failure"]["stage"], "final-bindings")
        self.assertEqual(self.record("process-1.json")["observed"]["stdout"], self.process_record["stdout"])
        self.assertEqual(self.record("process-2.json")["execution_state"], "not-attempted")
        self.assertEqual(self.launch.call_count, 1)

    def test_dependency_mismatch_prevents_semantic_cohort_success_and_later_launches(self):
        self.observe.return_value = {"matched": False, "observed": {}}
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["identity_unchanged"])
        self.assertFalse(result["semantic_cohort_passed"])
        self.assertEqual(result["first_failure"]["stage"], "dependency-observation")
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        self.assertFalse(self.record("process-1.json")["dependencies_passed"])
        self.assertEqual(self.launch.call_count, 1)

    def test_native_image_change_is_rehashed_before_next_process(self):
        def change_native_image(*args, **kwargs):
            self.image.write_bytes(b"different native bytes at the same path")
            return self.process_record

        self.launch.side_effect = change_native_image
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertEqual(result["first_failure"]["stage"], "before-process-bindings")
        self.assertEqual(result["first_failure"]["process"], 2)
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        self.assertFalse(result["identity_unchanged"])
        self.assertEqual(self.launch.call_count, 1)

    def test_supervisor_exception_does_not_invent_execution_or_discard_failure(self):
        self.launch.side_effect = RuntimeError("supervision interrupted")
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertEqual(result["first_failure"]["stage"], "process-supervision")
        self.assertEqual(result["executed_processes"], 0)
        self.assertEqual(result["execution_unknown_processes"], [1])
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        self.assertEqual(self.launch.call_count, 1)

    def test_complete_measurements_do_not_accept_numeric_qualification(self):
        for mode, count in (("controlled", 3), ("real-cpu-cold-startup", 5)):
            with self.subTest(mode=mode):
                self.args.mode = mode
                self.args.output = self.root / mode
                self.args.corpus = self.root if mode != "controlled" else None
                self.process_record["stdout"] = controlled_output() if mode == "controlled" else real_output()
                with patch.object(workloads.run_replay, "canonical_environment", return_value=dict(os.environ)), \
                        patch.object(workloads.run_replay, "inputs", return_value={"runtime": workloads.run_replay.identity(self.image)}):
                    self.assertTrue(workloads.execute(self.args))
                result = self.record()
                self.assertTrue(result["semantic_cohort_passed"])
                self.assertEqual(result["executed_processes"], count)
                self.assertEqual(result["unexecuted_processes"], [])
                self.assertTrue(result["required_measurements_complete"])
                self.assertFalse(result["passed"])
                self.assertFalse(self.record("process-1.json")["budget_passed"])

    def test_missing_measurements_stop_later_processes_without_inventing_budget_success(self):
        self.process_record["stdout"] = controlled_output(measured=False)
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertEqual(result["first_failure"]["kind"], "measurement-incomplete")
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        self.assertTrue(self.record("process-1.json")["semantic_passed"])
        self.assertFalse(result["required_measurements_complete"])
        self.assertFalse(result["passed"])
        self.assertEqual(self.launch.call_count, 1)

    def test_process_failure_precedes_its_missing_measurements(self):
        self.process_record.update(exit_code=1, stdout="")
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertEqual(result["first_failure"]["kind"], "process-failure")
        self.assertEqual(result["first_failure"]["stage"], "process-result")
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        self.assertEqual(self.launch.call_count, 1)

    def test_real_startup_rejects_ambient_ort_profiling_before_launch(self):
        self.args.mode = "real-cpu-cold-startup"
        self.args.corpus = self.root
        with patch.object(workloads.run_replay, "canonical_environment", return_value={
            "MADO_PILOT_ORT_PROFILE_DIR": "unrelated-profile-output",
        }):
            self.assertFalse(workloads.execute(self.args))
        self.assertEqual(self.record()["first_failure"]["stage"], "preflight-environment")
        self.launch.assert_not_called()

    def test_final_binding_exception_invalidates_an_otherwise_complete_cohort(self):
        calls = 0

        def remove_after_cohort(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 3:
                self.host_path.unlink()
            return self.process_record

        self.launch.side_effect = remove_after_cohort
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertEqual(result["executed_processes"], 3)
        self.assertEqual(result["unexecuted_processes"], [])
        self.assertFalse(result["semantic_cohort_passed"])
        self.assertEqual(result["first_failure"]["stage"], "final-bindings")
        self.assertEqual(self.record("apparatus-invalid.json"), result["first_failure"])

    def test_unrelated_environment_payloads_never_enter_private_identity_records(self):
        secret = "unrelated-private-payload"
        with patch.dict(os.environ, {
            "MADO_PILOT_PRIVATE_TEXT": secret, "ORT_PRIVATE_TEXT": secret,
            "OPENCV_PRIVATE_TEXT": secret,
        }):
            self.assertTrue(workloads.execute(self.args))
        for filename in ("plan.json", "result.json", "process-1.json", "process-2.json", "process-3.json"):
            self.assertNotIn(secret, (self.args.output / filename).read_text(encoding="utf-8"))

    def test_authority_and_missing_enforcement_profile_cannot_launch(self):
        self.args.execute = False
        with self.assertRaises(ValueError):
            workloads.execute(self.args)
        self.args.execute = True
        self.args.enforce_budgets = True
        with self.assertRaisesRegex(ValueError, "--enforce-budgets requires"):
            workloads.execute(self.args)
        self.assertFalse(self.args.output.exists())
        self.launch.assert_not_called()

    def test_historical_execution_refuses_actionably_without_opening_an_adr(self):
        self.args.profile = self.profile_path
        self.profile_path.write_bytes(self.profile_bytes.replace(b"format_version = 3", b"format_version = 2"))
        self.assertFalse(workloads.execute(self.args))
        failure = self.record()["first_failure"]
        self.assertEqual(failure["stage"], "preflight-profile")
        self.assertIn("read-only", failure["reason"])
        self.assertIn("--inspect-profile", failure["reason"])
        self.assertTrue(self.record()["empirical_targets"]["requested"])
        self.assertIsNone(self.record()["empirical_targets"]["passed"])
        self.assertFalse(self.record()["execution_passed"])
        self.assertFalse(self.record()["stopped_by_enforcement"])
        self.assertEqual(self.record()["attempted_processes"], 0)
        self.launch.assert_not_called()

    def test_malformed_current_profile_remains_requested_and_never_launches(self):
        self.args.profile = self.profile_path
        self.profile_path.write_text("format_version = 3\n", encoding="utf-8")
        stderr = io.StringIO()
        with patch.object(sys, "stderr", stderr):
            self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["empirical_targets"]["requested"])
        self.assertIsNone(result["empirical_targets"]["passed"])
        self.assertFalse(result["execution_passed"])
        self.assertEqual(result["attempted_processes"], 0)
        self.assertEqual(result["first_failure"]["stage"], "preflight-profile")
        self.assertIn("OCR profile refused:", stderr.getvalue())
        self.launch.assert_not_called()

    def test_historical_inspection_is_read_only_and_never_executes(self):
        historical = self.profile_bytes.replace(b"format_version = 3", b"format_version = 2")
        self.profile_path.write_bytes(historical)
        output = io.StringIO()
        with patch.object(sys, "argv", ["workloads.py", "--inspect-profile", str(self.profile_path)]), \
                patch.object(sys, "stdout", output):
            self.assertEqual(workloads.main(), 0)
        inspected = json.loads(output.getvalue())
        self.assertEqual(inspected["profile"]["format_version"], 2)
        self.assertFalse(inspected["execution_requested"])
        self.assertEqual(inspected["identity"]["sha256"], hashlib.sha256(historical).hexdigest())
        self.assertEqual(self.profile_path.read_bytes(), historical)
        self.assertFalse(self.args.output.exists())
        self.launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
