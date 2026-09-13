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

SCRIPT = Path(__file__).resolve().parents[1] / "workloads.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("ocr_workload_admission", SCRIPT)
workloads = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(workloads)

TARGET = "aarch64-apple-darwin"
PROFILE = (workloads.ROOT / "docs/benchmarks" / f"ocr-text-watch-{TARGET}.toml").read_bytes()


def controlled_output() -> str:
    samples = [(name, "warmup" if index < 2 else "sample", index)
               for name in workloads.WORKLOADS[:-1] for index in range(22)]
    samples.append(("cold-startup", "sample", 0))
    lines = []
    for name, phase, index in samples:
        suffix = f"workload={name} phase={phase} iteration={index}"
        lines.extend((f"# ocr-start {suffix}", f"# ocr-raw {suffix} semantic=passed retained=0"))
    lines.extend((
        "ocr-text-watch-query: 6 workloads, 20 samples each, 0 oracle failure(s)",
        "ocr-text-watch-controlled-startup: 1 workloads, 1 samples each, 0 oracle failure(s)",
    ))
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
        self.native_path.write_text(json.dumps({
            str(self.image.resolve()): hashlib.sha256(self.image.read_bytes()).hexdigest(),
            str(self.executable.resolve()): hashlib.sha256(self.executable.read_bytes()).hexdigest(),
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
            "matched": True, "observed": {str(self.image.resolve()): "test-double observation"},
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

    def test_unapproved_executable_cannot_reach_process_launch(self):
        approved = json.loads(self.native_path.read_text())
        del approved[str(self.executable.resolve())]
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

    def test_completed_semantic_modes_never_promote_unmeasured_qualification(self):
        for mode, count in (("controlled", 3), ("real-cpu-cold-startup", 5)):
            with self.subTest(mode=mode):
                self.args.mode = mode
                self.args.output = self.root / mode
                self.args.corpus = self.root if mode != "controlled" else None
                self.process_record["stdout"] = controlled_output() if mode == "controlled" else (
                    "ocr-text-watch: scenario=transition terminal=Matched sequence=1 regions=8 "
                    "satisfying=1 confirmations=1 retained_bytes=2073600 cleanup=returned"
                )
                with patch.object(workloads.run_replay, "canonical_environment", return_value=dict(os.environ)), \
                        patch.object(workloads.run_replay, "inputs", return_value={"runtime": workloads.run_replay.identity(self.image)}):
                    self.assertTrue(workloads.execute(self.args))
                result = self.record()
                self.assertTrue(result["semantic_cohort_passed"])
                self.assertEqual(result["executed_processes"], count)
                self.assertEqual(result["unexecuted_processes"], [])
                self.assertFalse(result["required_measurements_complete"])
                self.assertFalse(result["passed"])
                self.assertFalse(self.record("process-1.json")["budget_passed"])

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
        with self.assertRaises(ValueError):
            workloads.execute(self.args)
        self.assertFalse(self.args.output.exists())
        self.launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
