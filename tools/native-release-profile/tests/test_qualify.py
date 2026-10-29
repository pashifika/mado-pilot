"""Attempt failures and incomplete evidence cannot become profile acceptance."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify
import staging


class AttemptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.python = Path(sys.executable).resolve()
        self.candidate = "cpu-host-windows" if os.name == "nt" else "cpu-host-macos"
        fixture = self.root / "input"
        fixture.mkdir()
        (fixture / "fixture.txt").write_bytes(b"fixture")
        (fixture / "LICENSE").write_bytes(b"license")
        self.manifest = {
            "schema_version": 1, "candidate": self.candidate,
            "source_commit": "a" * 40, "source_tree": "b" * 40, "features": [],
            "artifacts": [{"id": "fixture", "root": "input", "path": "fixture.txt", "destination": "fixture.txt",
                           "category": "fixture", "ownership": "product", "sha256": hashlib.sha256(b"fixture").hexdigest(),
                           "bytes": 7, "source": "repository", "version": "1", "license": "Apache-2.0",
                           "notice": "repository-license", "redistribution": "approved"}],
            "admission": {"kind": "development-host", "record_sha256": None},
            "rows": [{"id": "consumer", "argv": ["{python}/" + self.python.name, "-c", "print('MADO_PROFILE_RESULT=passed')"],
                      "executable_sha256": qualify.digest_file(self.python), "environment": {"PATH": "{python}"},
                      "expected_exit": 0, "required_stdout": "MADO_PROFILE_RESULT=passed", "unexecuted_reason": None}],
        }
        self.manifest["artifacts"].append({
            "id": "repository-license", "root": "input", "path": "LICENSE", "destination": "LICENSE",
            "category": "notices", "ownership": "product", "sha256": hashlib.sha256(b"license").hexdigest(),
            "bytes": 7, "source": "repository", "version": "1", "license": "Apache-2.0",
            "notice": "repository-license", "redistribution": "approved",
        })

    def execute(self, attempt="one"):
        # Procedure policy runs on Linux too, but no Linux result qualifies a native target.
        data = json.dumps(self.manifest).encode("utf-8")
        with patch.object(qualify, "native_target", return_value=qualify.CANDIDATES[self.candidate][0]):
            return qualify.execute(self.manifest, manifest_bytes=data, manifest_digest=hashlib.sha256(data).hexdigest(),
                                   attempt=attempt, roots={"python": self.python.parent, "input": self.root / "input"},
                                   output=self.root / "out")

    def test_success_is_not_profile_selection(self):
        result, code = self.execute()
        self.assertEqual(code, 0)
        self.assertEqual(result["rows"][0]["status"], "passed")
        self.assertEqual(result["qualification"], "not-selected")
        self.assertTrue(result["cleanup_ok"])

    def test_unknown_root_binding_fails_only_its_command_row(self):
        self.manifest["rows"].append(dict(self.manifest["rows"][0], id="after"))
        for field, invalid in (("argv", ["{missing}/consumer"]),
                               ("environment", {"PATH": "{missing}"})):
            with self.subTest(field=field):
                previous = self.manifest["rows"][0][field]
                self.manifest["rows"][0][field] = invalid
                result, code = self.execute(attempt=field)
                self.manifest["rows"][0][field] = previous
                self.assertEqual(code, 1)
                self.assertEqual(result["rows"][0]["status"], "failed")
                self.assertEqual(result["rows"][0]["reason"], "row-binding-invalid")
                self.assertEqual(result["rows"][1]["status"], "passed")

    @unittest.skipUnless(os.name == "nt", "Windows candidate junction")
    def test_candidate_junction_cannot_redirect_an_attempt(self):
        import _winapi
        outside = self.root / "outside"
        output = self.root / "out"
        outside.mkdir()
        output.mkdir()
        junction = output / self.candidate
        _winapi.CreateJunction(str(outside), str(junction))
        try:
            with self.assertRaises(ValueError):
                self.execute()
            self.assertEqual(list(outside.iterdir()), [])
        finally:
            junction.rmdir()

    def test_staged_executable_preserves_its_relative_resource_layout(self):
        self.manifest["artifacts"].append(dict(self.manifest["artifacts"][0],
            id="consumer", root="python", path=self.python.name, destination="bin/" + self.python.name,
            category="consumer", bytes=self.python.stat().st_size, sha256=qualify.digest_file(self.python)))
        self.manifest["rows"][0]["argv"] = [
            "{stage}/bin/" + self.python.name, "-c",
            "import sys; from pathlib import Path; "
            "assert (Path(sys.executable).parent.parent / 'fixture.txt').read_bytes() == b'fixture'; "
            "print('MADO_PROFILE_RESULT=passed')",
        ]
        result, code = self.execute()
        self.assertEqual(code, 0)
        self.assertEqual(result["rows"][0]["status"], "passed")

    def test_missing_mandatory_output_fails_even_with_zero_exit(self):
        self.manifest["rows"][0]["argv"][-1] = "pass"
        result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertEqual(result["rows"][0]["process"]["exit_code"], 0)
        self.assertFalse(result["rows"][0]["mandatory_output_seen"])
        self.assertEqual(result["rows"][0]["status"], "failed")

    def test_nonzero_child_cannot_be_hidden_by_terminal_output(self):
        self.manifest["rows"][0]["argv"][-1] += ";raise SystemExit(7)"
        result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertEqual(result["rows"][0]["process"]["exit_code"], 7)
        self.assertEqual(result["rows"][0]["status"], "failed")

    def test_duplicate_attempt_preserves_failure_bytes(self):
        self.manifest["rows"][0]["argv"][-1] = "raise SystemExit(7)"
        self.execute()
        retained = self.root / "out" / self.candidate / "one" / "result.json"
        before = retained.read_bytes()
        self.manifest["rows"][0]["argv"][-1] = "print('MADO_PROFILE_RESULT=passed')"
        with self.assertRaises(FileExistsError):
            self.execute()
        self.assertEqual(retained.read_bytes(), before)

    def test_cleanup_failure_overrides_successful_child(self):
        with patch.object(qualify.shutil, "rmtree", side_effect=OSError("injected cleanup failure")):
            result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertFalse(result["cleanup_ok"])
        self.assertEqual(result["rows"][0]["status"], "passed")
        self.assertEqual(result["procedure_status"], "failed")

    def test_executable_digest_mismatch_never_executes(self):
        self.manifest["rows"][0]["executable_sha256"] = "0" * 64
        with patch.object(qualify, "run_process", side_effect=AssertionError("must not launch")):
            result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertEqual(result["rows"][0]["reason"], "executable-identity-mismatch")

    def test_unexecuted_row_withholds_procedure_success(self):
        self.manifest["rows"].append({"id": "clean-admission", "argv": None, "environment": {},
            "executable_sha256": None, "expected_exit": None, "required_stdout": None,
            "unexecuted_reason": "clean-host-unavailable"})
        result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertEqual(result["procedure_status"], "unexecuted")
        self.assertEqual(result["rows"][1]["status"], "unexecuted")

    def test_staging_failure_retains_all_unexecuted_rows(self):
        with patch.object(qualify, "stage_files", side_effect=ValueError("unresolved license")):
            result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertEqual(result["rows"][0]["status"], "unexecuted")
        self.assertEqual(result["procedure_status"], "failed")

    def test_manifest_digest_and_unknown_fields_are_rejected(self):
        path = self.root / "manifest.json"
        data = json.dumps(self.manifest).encode()
        path.write_bytes(data)
        digest = hashlib.sha256(data).hexdigest()
        self.assertEqual(qualify.load_manifest(path, digest, self.candidate), (self.manifest, data))
        with self.assertRaises(ValueError):
            qualify.load_manifest(path, "0" * 64, self.candidate)
        self.manifest["automatic_pass"] = True
        data = json.dumps(self.manifest).encode()
        path.write_bytes(data)
        with self.assertRaises(ValueError):
            qualify.load_manifest(path, hashlib.sha256(data).hexdigest(), self.candidate)

    def test_retained_manifest_preserves_reviewed_bytes(self):
        data = json.dumps(self.manifest).encode("utf-8")
        result, _ = self.execute()
        retained = self.root / "out" / self.candidate / "one" / "manifest.json"
        self.assertEqual(retained.read_bytes(), data)
        self.assertEqual(hashlib.sha256(retained.read_bytes()).hexdigest(), result["manifest_sha256"])

    def test_duplicate_json_keys_are_refused(self):
        data = json.dumps(self.manifest).replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1').encode()
        path = self.root / "duplicate.json"
        path.write_bytes(data)
        with self.assertRaises(ValueError):
            qualify.load_manifest(path, hashlib.sha256(data).hexdigest(), self.candidate)

    def test_interrupted_cleanup_never_records_success(self):
        with patch.object(qualify.shutil, "rmtree", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.execute()
        retained = self.root / "out" / self.candidate / "one" / "result.json"
        result = json.loads(retained.read_text())
        self.assertEqual(result["procedure_status"], "failed")
        self.assertFalse(result["cleanup_ok"])

    def test_interrupted_row_records_failure_before_propagation(self):
        with patch.object(qualify, "run_process", side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):
                self.execute()
        retained = self.root / "out" / self.candidate / "one" / "result.json"
        result = json.loads(retained.read_text())
        self.assertEqual(result["procedure_status"], "failed")
        self.assertEqual(result["rows"][0]["reason"], "attempt-interrupted")

    @unittest.skipIf(os.name == "nt", "Windows privacy uses a DACL")
    def test_attempt_files_are_private_under_permissive_umask(self):
        previous = os.umask(0)
        try:
            self.execute()
        finally:
            os.umask(previous)
        attempt = self.root / "out" / self.candidate / "one"
        self.assertEqual(attempt.stat().st_mode & 0o077, 0)
        for path in attempt.iterdir():
            self.assertEqual(path.stat().st_mode & 0o077, 0)

    def test_builder_replacement_after_snapshot_cannot_change_the_executed_bytes(self):
        builder = self.root / "input" / ("python.exe" if os.name == "nt" else "python")
        shutil.copyfile(self.python, builder)
        builder.chmod(0o755)
        self.manifest["rows"][0]["argv"][0] = "{input}/" + builder.name
        original_run = qualify.run_process
        def replace_builder_then_run(argv, **options):
            builder.write_bytes(b"not the reviewed executable")
            return original_run(argv, **options)
        with patch.object(qualify, "run_process", side_effect=replace_builder_then_run):
            result, code = self.execute()
        self.assertEqual(code, 0)
        self.assertEqual(result["rows"][0]["status"], "passed")


class SnapshotQuotaTests(unittest.TestCase):
    execute = AttemptTests.execute

    def setUp(self):
        AttemptTests.setUp(self)
        executable = self.root / "input" / "external"
        executable.write_bytes(b"x" * 512)
        row = self.manifest["rows"][0]
        self.manifest["rows"] = [
            dict(row, id=f"stale-{index}", argv=["{input}/external"], executable_sha256="0" * 64)
            for index in range(3)
        ]

    def test_repeated_hash_refusals_reclaim_before_next_row_under_stage_ceiling(self):
        sizes, before = [], []
        original_snapshot = qualify.snapshot_executable
        original_writer = staging._fd_writer

        def staged_bytes(snapshot):
            scratch = snapshot.parent.parent
            return sum(path.stat().st_size for path in scratch.rglob("*") if path.is_file())

        def snapshot(source, destination, digest, **limits):
            before.append(staged_bytes(destination))
            try:
                return original_snapshot(source, destination, digest, **limits)
            finally:
                sizes.append(staged_bytes(destination))

        def writer(descriptor, label):
            write = original_writer(descriptor, label)

            def observe(data):
                write(data)
                scratch = next((self.root / "out" / self.candidate / "one").glob("scratch-*"))
                sizes.append(sum(path.stat().st_size for path in scratch.rglob("*") if path.is_file()))
            return observe

        with patch.dict(qualify.LIMITS, stage_bytes=1024), \
                patch.object(qualify, "snapshot_executable", side_effect=snapshot), \
                patch.object(staging, "_fd_writer", side_effect=writer), \
                patch.object(qualify, "run_process") as launch:
            result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertEqual(before, [14, 14, 14])
        self.assertEqual(max(sizes), 526)
        self.assertTrue(all(size <= 1024 for size in sizes))
        self.assertEqual([row["reason"] for row in result["rows"]], ["executable-identity-mismatch"] * 3)
        self.assertEqual(result["staging_bytes_before_cleanup"], 14)
        launch.assert_not_called()

    def test_unreclaimed_snapshot_stops_rows_with_primary_rejection_and_charged_bytes(self):
        original_unlink = Path.unlink
        retained = []
        original_rmtree = qualify.shutil.rmtree

        def unlink(path, *args, **kwargs):
            if path.name == "external" and path.parent.name.startswith("executable-"):
                raise PermissionError("snapshot-unlink-denied")
            return original_unlink(path, *args, **kwargs)

        def cleanup(path):
            retained.append(sum(item.stat().st_size for item in path.rglob("*") if item.is_file()))
            return original_rmtree(path)

        with patch.dict(qualify.LIMITS, stage_bytes=1024), \
                patch.object(Path, "unlink", new=unlink), \
                patch.object(qualify.shutil, "rmtree", side_effect=cleanup), \
                patch.object(qualify, "snapshot_executable", wraps=qualify.snapshot_executable) as snapshot, \
                patch.object(qualify, "run_process") as launch:
            result, code = self.execute()
        self.assertEqual(code, 1)
        self.assertEqual(snapshot.call_count, 1)
        launch.assert_not_called()
        self.assertEqual(retained, [526])
        self.assertEqual(result["staging_bytes_before_cleanup"], 526)
        self.assertFalse(result["rows"][0]["snapshot_cleanup"]["ok"])
        self.assertIn("SHA-256", result["rows"][0]["snapshot_cleanup"]["primary"])
        self.assertIn("snapshot-unlink-denied", result["rows"][0]["snapshot_cleanup"]["failure"])
        self.assertEqual([row["status"] for row in result["rows"]], ["failed", "unexecuted", "unexecuted"])
        failure = (self.root / "out" / self.candidate / "one" / "failure.log").read_text()
        self.assertIn("SHA-256", failure)
        self.assertIn("snapshot-unlink-denied", failure)


if __name__ == "__main__":
    unittest.main()
