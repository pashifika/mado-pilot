"""Binding-only state transitions with inert inputs; no child or model executes."""

import argparse
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bind_replay
from test_workload_admission import PROFILE, controlled_output, real_output


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
        self.args = argparse.Namespace(mode="real-cpu", executable=self.executable, corpus=self.root,
                                       output=self.root / "output", execute_binding=True)
        self.real_inputs = bind_replay.run_replay.inputs
        self.real_environment = bind_replay.run_replay.canonical_environment
        for patcher in (
            patch.object(bind_replay.workloads, "source", return_value={"commit": "a" * 40, "tree": "b" * 40, "status": ""}),
            patch.object(bind_replay.workloads, "observed_host", return_value={"host_id": "owned-host"}),
            patch.object(bind_replay.run_replay, "canonical_environment", return_value={}),
            patch.object(bind_replay.run_replay, "selected_target", return_value="aarch64-apple-darwin"),
            patch.object(bind_replay.run_replay.platform, "system", return_value="Darwin"),
            patch.object(bind_replay.run_replay, "inputs", return_value=self.inputs),
            patch.object(bind_replay.run_replay, "observe_dependencies", return_value={"matched": False, "observed": self.images}),
            patch.dict(bind_replay.os.environ, {"PATH": "reviewed-search-path"}, clear=True),
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

    def controlled(self):
        self.args.mode = "controlled"
        self.args.corpus = None
        self.observed["stdout"] = controlled_output()
        self.images.pop(self.inputs["runtime"]["path"])

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

    def test_unsupported_host_still_refuses_before_candidate_launch(self):
        with patch.object(bind_replay.run_replay.platform, "system", return_value="Linux"):
            self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "preflight")
        self.assertFalse(self.result()["binding_complete"])
        self.launch.assert_not_called()

    def test_mode_is_required_by_the_cli_before_any_evidence_or_launch(self):
        argv = ["bind_replay.py", "--executable", str(self.executable),
                "--corpus", str(self.root), "--output", str(self.args.output), "--execute-binding"]
        with patch.object(sys, "argv", argv), patch.object(sys, "stderr", io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                bind_replay.main()
        self.assertEqual(raised.exception.code, 2)
        self.assertFalse(self.args.output.exists())
        self.launch.assert_not_called()

    def test_corpus_selection_must_match_the_explicit_mode(self):
        for mode, corpus in (("real-cpu", None), ("controlled", self.root)):
            with self.subTest(mode=mode):
                self.args.mode, self.args.corpus = mode, corpus
                with self.assertRaises(ValueError):
                    bind_replay.execute(self.args)
                self.assertFalse(self.args.output.exists())
                self.launch.assert_not_called()

    def test_controlled_discovery_needs_no_models_and_does_not_approve_or_repeat(self):
        self.controlled()
        self.runtime.unlink()
        with patch.dict(bind_replay.os.environ, {}, clear=True), \
                patch.object(bind_replay.run_replay, "inputs", side_effect=self.real_inputs), \
                patch.object(bind_replay.run_replay, "canonical_environment", side_effect=self.real_environment):
            self.assertTrue(bind_replay.execute(self.args))
        self.assertTrue(self.result()["measurements"]["complete"])
        self.assertTrue(self.result()["identity_unchanged"])
        self.assertFalse(self.result()["image_set_approved"])
        self.assertFalse(self.result()["qualification_passed"])
        self.assertEqual(json.loads((self.args.output / "observed-native-images.json").read_text()), self.images)
        self.assertEqual(self.launch.call_count, 1)

    def test_controlled_semantic_failure_cannot_publish_discovered_images(self):
        self.controlled()
        self.observed["stdout"] = self.observed["stdout"].replace("semantic=passed", "semantic=failed", 1)
        self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "measurement-observation")
        self.assertEqual(self.result()["observed"]["stdout"], self.observed["stdout"])
        self.assertFalse((self.args.output / "observed-native-images.json").exists())
        self.assertEqual(self.launch.call_count, 1)

    def test_controlled_semantics_without_measurements_are_not_a_binding(self):
        self.controlled()
        self.observed["stdout"] = controlled_output(measured=False)
        self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "measurement-observation")
        self.assertFalse(self.result()["measurements"]["complete"])
        self.assertFalse((self.args.output / "observed-native-images.json").exists())

    def test_controlled_discovery_must_contain_the_exact_executable(self):
        self.controlled()
        self.images[self.inputs["executable"]["path"]] = "d" * 64
        self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "image-observation")
        self.assertFalse((self.args.output / "observed-native-images.json").exists())

    def test_controlled_host_change_stops_before_launch(self):
        self.controlled()
        with patch.object(bind_replay.workloads, "observed_host",
                          side_effect=[{"host_id": "owned-host"}, {"host_id": "changed-host"}]):
            self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "before-process")
        self.assertEqual(self.result()["execution_state"], "not-attempted")
        self.launch.assert_not_called()

    def test_controlled_changed_profile_or_harness_cannot_publish_a_binding(self):
        self.controlled()
        profile = self.root / "profile.toml"
        harness = self.root / "harness.rs"
        read_profile = bind_replay.workloads.profile
        for changed in (profile, harness):
            with self.subTest(changed=changed.name):
                profile.write_bytes(PROFILE)
                harness.write_bytes(b"inert harness source")
                self.args.output = self.root / f"output-{changed.name}"

                def launch(*_args, **_kwargs):
                    changed.write_bytes(changed.read_bytes() + b"\n")
                    return self.observed

                with patch.object(bind_replay.workloads, "profile", side_effect=lambda _: read_profile(profile)), \
                        patch.object(bind_replay.workloads, "HARNESS", (str(harness),)), \
                        patch.object(bind_replay.run_replay, "run_process", side_effect=launch):
                    self.assertFalse(bind_replay.execute(self.args))
                self.assertEqual(self.result()["failure"]["stage"], "final-bindings")
                self.assertFalse(self.result()["identity_unchanged"])
                self.assertFalse((self.args.output / "observed-native-images.json").exists())

    def test_controlled_changed_child_loader_selection_cannot_publish_a_binding(self):
        self.controlled()

        def launch(*_args, **kwargs):
            kwargs["env"]["PATH"] = "changed-search-path"
            return self.observed

        self.launch.side_effect = launch
        self.assertFalse(bind_replay.execute(self.args))
        self.assertEqual(self.result()["failure"]["stage"], "final-bindings")
        self.assertFalse((self.args.output / "observed-native-images.json").exists())


if __name__ == "__main__":
    unittest.main()
