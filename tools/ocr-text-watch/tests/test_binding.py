"""Binding-only state transitions with inert inputs; no child or model executes."""

import argparse
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bind_replay
from test_workload_admission import real_output


class BindingEvidence(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.executable = self.root / "inert-executable"
        self.runtime = self.root / "inert-runtime"
        self.executable.write_bytes(b"never executed")
        self.runtime.write_bytes(b"never loaded")
        self.inputs = {name: bind_replay.run_replay.identity(path) for name, path in (
            ("executable", self.executable), ("runtime", self.runtime),
        )}
        self.images = {entry["path"]: entry["sha256"] for entry in self.inputs.values()}
        self.images[str(self.root / "observed-additional-image")] = "c" * 64
        self.args = argparse.Namespace(executable=self.executable, corpus=self.root,
                                       output=self.root / "output", execute_binding=True)
        for patcher in (
            patch.object(bind_replay.workloads, "source", return_value={"commit": "a" * 40, "tree": "b" * 40, "status": ""}),
            patch.object(bind_replay.workloads, "observed_host", return_value={"host_id": "owned-host"}),
            patch.object(bind_replay.run_replay, "canonical_environment", return_value={}),
            patch.object(bind_replay.run_replay, "selected_target", return_value="aarch64-apple-darwin"),
            patch.object(bind_replay.run_replay, "inputs", return_value=self.inputs),
            patch.object(bind_replay.run_replay, "observe_dependencies", return_value={"matched": False, "observed": self.images}),
        ):
            patcher.start()
            self.addCleanup(patcher.stop)
        self.observed = {"stdout": real_output(), "stderr": "", "exit_code": 0,
                         "launch_error": None, "cleanup_ok": True, "timed_out": False,
                         "output_limited": False}
        launcher = patch.object(bind_replay.run_replay, "run_process", return_value=self.observed)
        self.launch = launcher.start()
        self.addCleanup(launcher.stop)

    def result(self):
        return json.loads((self.args.output / "result.json").read_text(encoding="utf-8"))

    def test_binding_retains_observed_set_but_does_not_approve_or_qualify_it(self):
        self.assertTrue(bind_replay.execute(self.args))
        self.assertEqual(json.loads((self.args.output / "observed-native-images.json").read_text()), self.images)
        self.assertTrue(self.result()["binding_complete"])
        self.assertFalse(self.result()["image_set_approved"])
        self.assertFalse(self.result()["qualification_passed"])
        self.assertEqual(self.launch.call_count, 1)

    def test_process_failure_retains_raw_output_without_a_binding_manifest(self):
        self.observed.update(exit_code=7, stdout="first process failure")
        self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "process-result")
        self.assertEqual(self.result()["observed"]["stdout"], "first process failure")
        self.assertFalse((self.args.output / "observed-native-images.json").exists())
        self.assertEqual(self.launch.call_count, 1)

    def test_changed_final_input_cannot_publish_a_usable_binding(self):
        changed = {**self.inputs, "runtime": {**self.inputs["runtime"], "sha256": "d" * 64}}
        with patch.object(bind_replay.run_replay, "inputs", side_effect=[self.inputs, self.inputs, changed]):
            self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "final-bindings")
        self.assertFalse(self.result()["binding_complete"])
        self.assertFalse((self.args.output / "observed-native-images.json").exists())

    def test_no_fresh_authority_never_creates_evidence_or_launches(self):
        self.args.execute_binding = False
        with self.assertRaises(ValueError):
            bind_replay.execute(self.args)
        self.assertFalse(self.args.output.exists())
        self.launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
