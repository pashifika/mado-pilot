"""Model-free workload admission and first-failure evidence regressions; no child executes."""

from __future__ import annotations

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
from test_measurements import controlled_lines

SCRIPT = Path(__file__).resolve().parents[1] / "workloads.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("ocr_workload_admission", SCRIPT)
workloads = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workloads)

TARGET = "aarch64-apple-darwin"
PROFILE = (workloads.ROOT / "docs/benchmarks" / f"ocr-text-watch-{TARGET}.toml").read_bytes()


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


class WorkloadAdmission(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.image = self.root / "inert-native-image"
        self.image.write_bytes(b"approved native image, never executed")
        self.executable = self.root / "inert-executable"
        self.executable.write_bytes(b"inert child, never executed")
        self.profile_path = self.root / "profile.toml"
        self.profile_path.write_bytes(PROFILE)
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
            mode="controlled", executable=self.executable, profile=self.profile_path,
            host_record=self.host_path, native_images=self.native_path, corpus=None,
            output=self.root / "evidence", execute=True, enforce_budgets=False,
        )
        self.git_status = ""
        self.source_root = workloads.ROOT
        self.process_record = {
            "exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
            "stdout": controlled_output(), "stderr": "", "duration_seconds": 1.0,
            "launch_error": None,
        }
        self.patch(patch.object(workloads.run_replay, "git", side_effect=self.git))
        self.patch(patch.object(workloads.run_replay, "selected_target", return_value=TARGET))
        self.patch(patch.object(workloads, "observed_host", return_value=self.actual_host))
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

    def approve_numeric_profile(self):
        source = (self.root / "owned-source").resolve()
        self.adr = source / "docs/adr/0075-ocr-text-watch-workload-profiles.md"
        self.adr.parent.mkdir(parents=True)
        self.adr.write_text("# Synthetic acceptance fixture\n\n- **Status:** Accepted\n", encoding="utf-8")
        supervisor = source / "tools/native-release-profile/process_runner.py"
        supervisor.parent.mkdir(parents=True)
        supervisor.write_bytes(b"owned identity fixture; never imported or executed")
        self.patch(patch.object(workloads, "ROOT", source))
        self.source_root = source
        selected = workloads.tomllib.loads(PROFILE.decode("utf-8"))
        selected["benchmark"].update(
            status="precursor-measured-budgets-accepted", normative=True, measurements_recorded=True,
        )
        selected["measurements"] = {"performed": True, "scope": "synthetic unit fixture, not evidence"}
        digest = workloads.run_replay.identity(self.executable)["sha256"]
        selected["acceptance"] = {
            "status": "accepted", "adr": "docs/adr/0075-ocr-text-watch-workload-profiles.md",
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

    def test_complete_approved_cohort_passes_only_its_target_mode(self):
        self.approve_numeric_profile()
        self.assertTrue(workloads.execute(self.args))
        result = self.record()
        self.assertTrue(result["passed"])
        self.assertTrue(result["required_measurements_complete"])
        self.assertEqual(result["qualification_scope"], {"target": TARGET, "mode": "controlled"})
        self.assertEqual(result["unexecuted_processes"], [])
        self.assertEqual(result["executed_processes"], 3)

    def test_numeric_exceedance_retains_first_process_and_stops_later_launches(self):
        self.approve_numeric_profile()
        limit = next(row for row in self.numeric_profile["measurement"][0]["budget"]
                     if row["measure"] == "latency_max_ms")
        limit["limit"] = 0
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertEqual(result["first_failure"]["kind"], "budget-exceeded")
        self.assertEqual(result["unexecuted_processes"], [2, 3])
        process = self.record("process-1.json")
        self.assertTrue(process["required_measurements_complete"])
        self.assertFalse(process["budget_passed"])
        self.assertFalse(process["budget_comparison"]["passed"])
        self.assertEqual(self.launch.call_count, 1)

    def test_last_process_numeric_failure_does_not_erase_measurement_completeness(self):
        self.approve_numeric_profile()
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

    def test_accepting_adr_mutation_invalidates_complete_numeric_comparisons(self):
        self.approve_numeric_profile()
        calls = 0

        def change_after_last_process(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 3:
                self.adr.write_text("- **Status:** Proposed\n", encoding="utf-8")
            return self.process_record

        self.launch.side_effect = change_after_last_process
        self.assertFalse(workloads.execute(self.args))
        result = self.record()
        self.assertFalse(result["passed"])
        self.assertEqual(result["first_failure"]["stage"], "final-bindings")
        self.assertTrue(self.record("process-3.json")["budget_passed"])

    def test_unaccepted_adr_or_wrong_budget_binding_prevents_launch(self):
        self.approve_numeric_profile()
        for field, wrong in (
            ("host_id", "different-approved-host"),
            ("controlled_executable_sha256", "0" * 64),
            ("adr-status", "Proposed"),
        ):
            with self.subTest(field=field):
                self.args.output = self.root / field
                if field == "adr-status":
                    self.adr.write_text("- **Status:** Proposed\n", encoding="utf-8")
                else:
                    original = self.numeric_profile["acceptance"][field]
                    self.numeric_profile["acceptance"][field] = wrong
                self.assertFalse(workloads.execute(self.args))
                self.assertEqual(self.record()["first_failure"]["stage"], "preflight-budget-bindings")
                self.assertFalse(self.record()["passed"])
                self.launch.assert_not_called()
                if field != "adr-status":
                    self.numeric_profile["acceptance"][field] = original


    def test_unapproved_executable_cannot_reach_process_launch(self):
        approved = json.loads(self.native_path.read_text())
        del approved[workloads.run_replay.identity(self.executable)["path"]]
        self.native_path.write_text(json.dumps(approved), encoding="utf-8")
        self.assertFalse(workloads.execute(self.args))
        self.assertTrue(self.record()["apparatus_invalid"])
        self.launch.assert_not_called()

    def test_changed_behavioral_profile_cannot_claim_the_fixed_workload(self):
        self.profile_path.write_bytes(PROFILE.replace(b"confirmations = 3", b"confirmations = 1"))
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
        read_document = workloads.run_replay.read_document
        replaced = False

        def replace_after_read(path, maximum_bytes=1048576):
            nonlocal replaced
            result = read_document(path, maximum_bytes)
            if Path(path).resolve() == self.profile_path.resolve() and not replaced:
                replaced = True
                self.profile_path.write_bytes(PROFILE.replace(b"sample_count = 20", b"sample_count = 21"))
            return result

        with patch.object(workloads.run_replay, "read_document", side_effect=replace_after_read):
            self.assertFalse(workloads.execute(self.args))
        plan = self.record("plan.json")
        self.assertEqual(plan["bindings"]["documents"]["profile"]["sha256"], hashlib.sha256(PROFILE).hexdigest())
        self.assertEqual(self.record()["first_failure"]["stage"], "before-process-bindings")
        self.assertEqual(self.record("process-1.json")["execution_state"], "not-attempted")
        self.launch.assert_not_called()

    def test_metadata_admission_refuses_nonregular_and_oversized_inputs(self):
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

    def test_authority_and_unaccepted_numeric_budgets_cannot_launch(self):
        self.args.execute = False
        with self.assertRaises(ValueError):
            workloads.execute(self.args)
        self.args.execute = True
        self.args.enforce_budgets = True
        unaccepted = workloads.tomllib.loads(PROFILE.decode("utf-8"))
        unaccepted["benchmark"]["normative"] = False
        with patch.object(workloads, "profile", return_value=(unaccepted, {})):
            with self.assertRaises(ValueError):
                workloads.execute(self.args)
        self.assertFalse(self.args.output.exists())
        self.launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
