"""Temporary-file campaign checkpoints with controlled processes, never model inference."""

import copy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import report

spec = importlib.util.spec_from_file_location("ocr_model_runner", Path(report.__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class CampaignCheckpointTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        executable, runtime, image = (self.root / name for name in ("consumer", "runtime", "image"))
        for path in (executable, runtime, image):
            path.write_bytes(b"synthetic-" + path.name.encode())
        self.args = SimpleNamespace(
            executable=executable, runtime=runtime, report=self.root / "evidence", diagnostic=True,
            reference_profile="reference", profile="candidate", reference_root=self.root, model_root=self.root,
            frozen_revision="a" * 40, binding=None, budgets=None, host_policy=None, cohort_id=None,
            preflight_only=False, timeout_seconds=30, output_limit_bytes=4096, cleanup_seconds=5,
            lifecycle_bound_ms=5000)
        self.fixtures = [{"id": "synthetic/image", "file": str(image), "sha256": report.sha256(image),
                          "byte_len": image.stat().st_size, "width": 2, "height": 2, "regions": []}]
        self.good = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
                     "launch_error": None, "stdout": "", "stderr": "", "duration_seconds": 0.01}
        self.build = {"catalog_sha256": report.sha256(runner.CATALOG_PATH),
                      "executable_sha256": report.sha256(executable), "opencv": "synthetic-opencv",
                      "build_source_revision": "a" * 40, "build_source_tree": "b" * 40,
                      "build_source_dirty": False, "build_compiler": "synthetic-rustc"}
        self.binding = {**self.build, "source_revision": "a" * 40, "source_tree": "b" * 40,
                        "runtime_sha256": report.sha256(runtime), "criteria_sha256": report.sha256(runner.CRITERIA_PATH),
                        "frozen_revision": "a" * 40, "compiler": "synthetic-rustc"}
        self.sample = {"type": "sample", "workload": "synthetic/image/singular", "phase": "sample",
                       "index": 0, "status": "Ok", "regions": [], "metrics": {"elapsed_ms": 1}}
        self.calls = []
        self.interrupt_at = None
        self.interrupt = KeyboardInterrupt("original-interrupt")
        self.process_override = None

    def retained(self):
        return json.loads((self.args.report / "report.json").read_text(encoding="utf-8"))

    def bounded(self, command, **limits):
        persisted = self.retained()
        attempt = persisted["attempts"][-1]
        self.assertEqual(attempt["command"], command)
        self.assertEqual(attempt["status"], "attempted-unconfirmed")
        self.assertIsNone(attempt["process"])
        self.assertEqual(attempt["source"]["source_revision"], "a" * 40)
        self.assertEqual(attempt["source"]["source_tree"], "b" * 40)
        self.calls.append(list(command))
        if len(self.calls) == self.interrupt_at:
            # Partial child output is not a known process exit/cleanup receipt.
            if attempt["raw_path"]:
                (self.args.report / attempt["raw_path"]).write_text('{"partial":', encoding="utf-8")
            raise self.interrupt
        profile = command[command.index("--profile") + 1]
        if "--describe-profile" in command:
            identity = {"profile_id": profile, "model_id": profile, "model_version": "synthetic",
                        "preprocessing_id": "synthetic", "decoder_id": "synthetic", "normalization_id": "synthetic",
                        "execution_sha256": "5" * 64, "resources": [], "limits": {}}
            return dict(self.good, stdout=json.dumps({"build": self.build, "identity": identity}))
        self.assertEqual(attempt["binding"], self.binding)
        self.assertEqual(attempt["identity"], persisted["identities"][profile])
        self.assertEqual(attempt["index"], int(command[command.index("--process-index") + 1]))
        self.assertEqual(limits, {"seconds": 30, "output": 4096, "cleanup": 5})
        (self.args.report / attempt["raw_path"]).write_text(json.dumps(self.sample) + "\n", encoding="utf-8")
        return dict(self.process_override or self.good)

    def campaign(self):
        def git(*arguments):
            if arguments[0] == "status":
                return ""
            return "b" * 40 if arguments[-1] == "HEAD^{tree}" else "a" * 40

        def actual_binding(args, fixture_path, revision, build):
            self.binding["fixtures_sha256"] = report.sha256(fixture_path)
            return copy.deepcopy(self.binding)

        with patch.object(runner, "git", side_effect=git), \
                patch.object(runner, "fixtures", return_value=(self.fixtures, [])), \
                patch.object(runner, "actual_binding", side_effect=actual_binding), \
                patch.object(runner, "bounded", side_effect=self.bounded), \
                patch.object(report, "git_bytes", side_effect=ValueError("synthetic Git authority unavailable")), \
                patch("builtins.print"):
            return runner.campaign(self.args)

    def assert_unknown_attempt(self, retained, *, count, kind="evaluation"):
        self.assertEqual(len(self.calls), count)
        self.assertEqual(len(retained["attempts"]), count)
        attempt = retained["attempts"][-1]
        self.assertEqual(attempt["kind"], kind)
        self.assertEqual(attempt["status"], "attempted-unconfirmed")
        self.assertIsNone(attempt["process"])
        self.assertFalse((self.args.report / attempt["receipt_path"]).exists())
        return attempt

    def test_interrupt_before_any_descriptor_completes_retains_first_attempt(self):
        self.interrupt_at = 1
        with self.assertRaises(KeyboardInterrupt) as caught:
            self.campaign()
        self.assertIs(caught.exception, self.interrupt)
        retained = self.retained()
        self.assertEqual(retained["execution"], "interrupted")
        attempt = self.assert_unknown_attempt(retained, count=1, kind="descriptor")
        self.assertIsNone(attempt["identity"])
        self.assertIsNone(attempt["binding"])
        self.assertEqual(retained["commands"], [])
        self.assertFalse(report.evaluate(retained, evidence_root=self.args.report)["complete"])

    def test_interrupt_before_first_evaluation_completion_keeps_exact_bound_marker(self):
        self.interrupt_at = 3
        with self.assertRaises(KeyboardInterrupt):
            self.campaign()
        retained = self.retained()
        attempt = self.assert_unknown_attempt(retained, count=3)
        self.assertEqual(attempt["binding"], self.binding)
        self.assertEqual(attempt["identity"], retained["identities"]["reference"])
        self.assertEqual(attempt["index"], 0)
        self.assertEqual(retained["profiles"], {"reference": []})
        self.assertEqual(len(retained["commands"]), 2)

    def test_good_good_good_interrupt_retains_completed_parsed_prefix(self):
        self.interrupt_at = 4
        with self.assertRaises(KeyboardInterrupt) as caught:
            self.campaign()
        self.assertIs(caught.exception, self.interrupt)
        retained = self.retained()
        self.assertEqual(retained["execution"], "interrupted")
        pending = self.assert_unknown_attempt(retained, count=4)
        self.assertEqual(pending["index"], 1)
        processes = retained["profiles"]["reference"]
        self.assertEqual(len(processes), 1)
        process = processes[0]
        self.assertEqual(process["samples"], [self.sample])
        self.assertEqual(process["process"], self.good)
        receipt = self.args.report / process["receipt_path"]
        self.assertEqual(report.sha256(receipt), process["receipt_sha256"])
        self.assertEqual(json.loads(receipt.read_text()), {key: process[key] for key in ("index", "process", "command")})
        self.assertEqual([attempt["status"] for attempt in retained["attempts"]],
                         ["completed", "completed", "completed", "attempted-unconfirmed"])
        self.assertFalse(report.evaluate(retained, evidence_root=self.args.report)["complete"])

    def test_parsing_failure_preserves_receipt_and_previously_parsed_prefix(self):
        original_parse = runner.parse_process
        failure = ValueError("original-parse-failure")

        def parse(raw_path, operational, command, index, **options):
            if index == 1:
                retained = self.retained()
                process = retained["profiles"]["reference"][-1]
                self.assertEqual(process["receipt_sha256"], report.sha256(self.args.report / process["receipt_path"]))
                self.assertEqual(retained["profiles"]["reference"][0]["samples"], [self.sample])
                raise failure
            return original_parse(raw_path, operational, command, index, **options)

        with patch.object(runner, "parse_process", side_effect=parse):
            self.assertEqual(self.campaign(), 2)
        retained = self.retained()
        self.assertEqual(len(self.calls), 4)
        self.assertEqual(len(retained["profiles"]["reference"]), 2)
        self.assertEqual(retained["profiles"]["reference"][0]["samples"], [self.sample])
        self.assertTrue(any("original-parse-failure" in failure for failure in retained["preflight_failures"]))
        self.assertFalse(retained["evaluation"]["complete"])

    def test_parse_interrupt_reconciles_receipt_only_checkpoint(self):
        original_parse = runner.parse_process

        def parse(raw_path, operational, command, index, **options):
            if index == 1:
                raise self.interrupt
            return original_parse(raw_path, operational, command, index, **options)

        with patch.object(runner, "parse_process", side_effect=parse):
            with self.assertRaises(KeyboardInterrupt) as caught:
                self.campaign()
        self.assertIs(caught.exception, self.interrupt)
        self.assertEqual(len(self.calls), 4)
        retained = self.retained()
        self.assertEqual(retained["execution"], "interrupted")
        verified, errors = report.reconcile_raw(retained, self.args.report)
        self.assertEqual(errors, [])
        self.assertEqual(verified["reference"][1]["samples"], [self.sample])

    def interrupt_digest(self, name):
        original_sha256 = runner.sha256

        def sha256(path):
            if Path(path).name == name:
                raise self.interrupt
            return original_sha256(path)

        with patch.object(runner, "sha256", side_effect=sha256):
            with self.assertRaises(KeyboardInterrupt) as caught:
                self.campaign()
        self.assertIs(caught.exception, self.interrupt)
        self.assertEqual(len(self.calls), 4)
        retained = self.retained()
        self.assertEqual(retained["execution"], "interrupted")
        return retained, retained["attempts"][-1]

    def test_raw_digest_interrupt_retains_completed_receipt_without_raw_digest(self):
        retained, attempt = self.interrupt_digest("reference-1.jsonl")
        self.assertEqual((attempt["index"], attempt["status"], attempt["process"]), (1, "completed", self.good))
        receipt = self.args.report / attempt["receipt_path"]
        self.assertEqual(report.sha256(receipt), attempt["receipt_sha256"])
        self.assertEqual(json.loads(receipt.read_text()), {"index": 1, "process": self.good, "command": attempt["command"]})
        pending = retained["profiles"]["reference"][-1]
        self.assertEqual(pending["projection_state"], "raw-digest-pending")
        self.assertNotIn("raw_sha256", pending)
        verified, errors = report.reconcile_raw(retained, self.args.report)
        self.assertEqual(errors, [])
        self.assertEqual([process["index"] for process in verified["reference"]], [0])
        errors = report.evaluate(retained, evidence_root=self.args.report)["validation_errors"]
        self.assertIn({"kind": "incomplete", "profile": "reference", "process": 1,
                       "message": "process receipt retained before its raw JSONL digest"}, errors)
        self.assertFalse(any("attempt" in error["message"] and "differs" in error["message"] for error in errors))

    def test_receipt_digest_interrupt_keeps_unbound_receipt_unconfirmed(self):
        retained, attempt = self.interrupt_digest("reference-1.process.json")
        self.assertEqual((attempt["index"], attempt["status"], attempt["process"]),
                         (1, "attempted-unconfirmed", self.good))
        self.assertNotIn("receipt_sha256", attempt)
        self.assertTrue((self.args.report / attempt["receipt_path"]).is_file())
        self.assertEqual([process["index"] for process in retained["profiles"]["reference"]], [0])
        verified, errors = report.reconcile_raw(retained, self.args.report)
        self.assertEqual(errors, [])
        self.assertEqual([process["index"] for process in verified["reference"]], [0])

    def test_checkpoint_interrupt_survives_temporary_cleanup_failure(self):
        original_fsync, original_unlink = runner.os.fsync, Path.unlink
        interrupted = False
        cleanup_failed = False

        def fsync(descriptor):
            nonlocal interrupted
            if len(self.calls) == 3 and not interrupted:
                interrupted = True
                raise self.interrupt
            return original_fsync(descriptor)

        def unlink(path, *arguments, **options):
            nonlocal cleanup_failed
            if interrupted and not cleanup_failed and path.name.startswith(".report-"):
                cleanup_failed = True
                raise OSError("checkpoint-unlink-denied")
            return original_unlink(path, *arguments, **options)

        with patch.object(runner.os, "fsync", side_effect=fsync), \
                patch.object(Path, "unlink", unlink):
            with self.assertRaises(KeyboardInterrupt) as caught:
                self.campaign()
        self.assertIs(caught.exception, self.interrupt)
        self.assertTrue(any("checkpoint-unlink-denied" in note for note in caught.exception.__notes__))
        self.assertEqual(len(self.calls), 3)
        self.assertEqual(self.retained()["execution"], "interrupted")

    def test_receipt_persistence_failure_stops_without_fabricating_confirmation(self):
        original_save = runner.save

        def save(path, value):
            if path.name == "reference-0.process.json":
                raise OSError("receipt-write-denied")
            return original_save(path, value)

        with patch.object(runner, "save", side_effect=save):
            self.assertEqual(self.campaign(), 2)
        retained = self.retained()
        self.assertEqual(len(self.calls), 3)
        attempt = retained["attempts"][-1]
        self.assertEqual(attempt["status"], "attempted-unconfirmed")
        self.assertEqual(attempt["process"], self.good)
        self.assertNotIn("receipt_sha256", attempt)
        self.assertFalse((self.args.report / attempt["receipt_path"]).exists())
        self.assertTrue(any("receipt-write-denied" in failure for failure in retained["preflight_failures"]))
        self.assertFalse(retained["evaluation"]["complete"])

    def test_unknown_exit_or_cleanup_withholds_all_later_launches(self):
        for exit_code, cleanup_ok in ((None, None), (None, True), (0, False)):
            with self.subTest(exit_code=exit_code, cleanup_ok=cleanup_ok):
                self.args.report = self.root / f"evidence-{exit_code}-{cleanup_ok}"
                self.calls.clear()
                self.process_override = dict(self.good, exit_code=exit_code, cleanup_ok=cleanup_ok)
                self.assertEqual(self.campaign(), 2)
                retained = self.retained()
                self.assertEqual(len(self.calls), 3)
                attempt = retained["attempts"][-1]
                self.assertEqual(attempt["status"], "attempted-unconfirmed")
                self.assertEqual(attempt["process"]["exit_code"], exit_code)
                self.assertIs(attempt["process"]["cleanup_ok"], cleanup_ok)
                self.assertEqual(retained["profiles"]["reference"][0]["process"], self.process_override)
                self.assertFalse(retained["evaluation"]["complete"])

    def test_interrupt_checkpoint_failure_preserves_original_interrupt_and_prior_durable_prefix(self):
        self.interrupt_at = 4
        original_checkpoint = runner.checkpoint

        def checkpoint(path, value):
            if value.get("execution") == "interrupted":
                raise OSError("checkpoint-write-denied")
            return original_checkpoint(path, value)

        with patch.object(runner, "checkpoint", side_effect=checkpoint):
            with self.assertRaises(KeyboardInterrupt) as caught:
                self.campaign()
        self.assertIs(caught.exception, self.interrupt)
        self.assertTrue(any("checkpoint-write-denied" in note for note in caught.exception.__notes__))
        retained = self.retained()
        self.assertEqual(retained["profiles"]["reference"][0]["samples"], [self.sample])
        self.assert_unknown_attempt(retained, count=4)

    def test_prelaunch_checkpoint_failure_never_launches_child(self):
        original_checkpoint = runner.checkpoint
        refused = False

        def checkpoint(path, value):
            nonlocal refused
            if value["attempts"] and not refused:
                refused = True
                raise OSError("prelaunch-write-denied")
            return original_checkpoint(path, value)

        with patch.object(runner, "checkpoint", side_effect=checkpoint):
            self.assertEqual(self.campaign(), 2)
        self.assertEqual(self.calls, [])
        self.assertTrue(any("prelaunch-write-denied" in failure for failure in self.retained()["preflight_failures"]))

    def test_final_evaluation_interrupt_retains_all_completed_receipts_before_rethrow(self):
        with patch.object(runner, "evaluate", side_effect=self.interrupt):
            with self.assertRaises(KeyboardInterrupt) as caught:
                self.campaign()
        self.assertIs(caught.exception, self.interrupt)
        retained = self.retained()
        self.assertEqual(retained["execution"], "interrupted")
        self.assertEqual(len(self.calls), 8)
        self.assertEqual([len(retained["profiles"][profile]) for profile in ("reference", "candidate")], [3, 3])
        self.assertTrue(all(attempt["status"] == "completed" for attempt in retained["attempts"]))
        self.assertNotIn("evaluation", retained)

    def test_failed_atomic_checkpoint_keeps_previous_report_bytes(self):
        self.args.report.mkdir()
        path = self.args.report / "report.json"
        runner.checkpoint(path, {"prefix": [1]})
        previous = path.read_bytes()
        with patch.object(runner.os, "replace", side_effect=OSError("replace-denied")):
            with self.assertRaisesRegex(OSError, "replace-denied"):
                runner.checkpoint(path, {"prefix": [1, 2]})
        self.assertEqual(path.read_bytes(), previous)
        self.assertEqual(list(self.args.report.iterdir()), [path])


if __name__ == "__main__":
    unittest.main()
