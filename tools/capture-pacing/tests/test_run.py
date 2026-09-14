"""Model-free authority and immutable-input boundaries for the native runner."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

RUNNER_PATH = Path(__file__).resolve().parents[1] / "run.py"
SPEC = importlib.util.spec_from_file_location("capture_pacing_runner_tests", RUNNER_PATH)
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class AdmissionTests(unittest.TestCase):
    def test_unapproved_authority_never_reaches_source_or_native_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            authority = Path(temporary) / "authority.json"
            authority.write_text(json.dumps({"schema": 1, "approved": False}), encoding="utf-8")
            with mock.patch.object(runner, "source_identity", side_effect=AssertionError("unapproved source work")), \
                 mock.patch.object(runner, "run_process", side_effect=AssertionError("unapproved native work")):
                with self.assertRaisesRegex(runner.Refusal, "execution-not-approved"):
                    runner.execute(authority)

    def test_cargo_hardlinked_binary_is_accepted_only_with_exact_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            original = Path(temporary) / "hashed-cargo-artifact"
            original.write_bytes(b"controlled artifact bytes")
            executable = Path(temporary) / "example"
            os.link(original, executable)
            record = {"path": str(executable.resolve()), "bytes": original.stat().st_size,
                      "sha256": hashlib.sha256(original.read_bytes()).hexdigest()}
            self.assertEqual(runner.bound_file(record), executable.resolve())
            original.write_bytes(b"mutated artifact bytes___")
            with self.assertRaises(runner.Refusal):
                runner.bound_file(record)

    def test_control_json_rejects_link_aliases_and_duplicate_keys(self):
        with tempfile.TemporaryDirectory() as temporary:
            original = Path(temporary) / "control"
            original.write_text('{"approved":false,"approved":true}', encoding="utf-8")
            with self.assertRaisesRegex(runner.Refusal, "duplicate-json-key"):
                runner.read_json(original)
            alias = Path(temporary) / "alias"
            os.link(original, alias)
            with self.assertRaisesRegex(runner.Refusal, "linked-control-file"):
                runner.read_json(alias)

    def test_supervisor_record_cannot_turn_timeout_or_cleanup_into_success(self):
        good = {"exit_code": 0, "timed_out": False, "output_limited": False,
                "cleanup_ok": True, "launch_error": None}
        self.assertTrue(runner.operational_success(good))
        for key, value in (("exit_code", 1), ("timed_out", True), ("output_limited", True),
                           ("cleanup_ok", False), ("launch_error", "controlled refusal")):
            with self.subTest(key=key):
                self.assertFalse(runner.operational_success({**good, key: value}))

    def test_host_binding_rejects_os_updates_and_missing_identity_before_launch(self):
        observed = {"system": "Darwin", "architecture": "arm64", "os_build": "controlled",
                    "cpu": "controlled CPU", "machine_id_sha256": "a" * 64}
        with mock.patch.object(runner, "host_snapshot", return_value=observed), \
             mock.patch.object(runner, "run_process", side_effect=AssertionError("native launch")):
            self.assertEqual(runner.verify_host({"host": observed}), observed)
            for binding in ({}, {"host": {**observed, "os_build": "earlier"}},
                            {"host": {**observed, "machine_id_sha256": "b" * 64}}):
                with self.subTest(binding=binding), self.assertRaisesRegex(runner.Refusal, "host-binding-mismatch"):
                    runner.verify_host(binding)

    def test_native_binding_requires_transitive_bytes_and_rejects_loader_shadowing(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            libraries = root / "libraries"
            libraries.mkdir()
            indirect = root / "indirect"
            indirect.mkdir()
            inputs = {name: root / name for name in ("consumer", "fixture", "runtime")}
            inspector = root / "inspector"
            first = libraries / "libFirst.dylib"
            second = indirect / "libSecond.dylib"
            for path in (*inputs.values(), inspector, first, second):
                path.write_bytes(path.name.encode("ascii"))
            def binding(path):
                size, digest = runner.digest(path)
                return {"path": str(path), "bytes": size, "sha256": digest}
            authority = {"loader_directories": [str(libraries)],
                         "native_inspector": binding(inspector), "native_inputs": [binding(first)]}
            def inspect(path, tool):
                imports = [{"name": str(second), "kind": "LC_LOAD_DYLIB"}] if path == first else []
                return {"architecture": "ARM64", "imports": imports,
                        "rpaths": ["@loader_path", "@executable_path"]}
            with mock.patch.object(runner.platform, "system", return_value="Darwin"), \
                 mock.patch.object(runner, "inspect_file", side_effect=inspect):
                with self.assertRaisesRegex(runner.Refusal, "unbound-native-import"):
                    runner.verify_native_bindings(authority, inputs)
                authority["native_inputs"].append(binding(second))
                evidence = runner.verify_native_bindings(authority, inputs)
                self.assertIn({"owner": str(first), "import": str(second), "resolved": str(second)}, evidence["edges"])
                shadow = libraries / "libInjected.dylib"
                shadow.write_bytes(b"unbound lookup candidate")
                with self.assertRaisesRegex(runner.Refusal, "unbound-loader-candidate"):
                    runner.verify_native_bindings(authority, inputs)
                shadow.unlink()
                second.write_bytes(b"replacement dependency")
                with self.assertRaisesRegex(runner.Refusal, "file-binding-mismatch"):
                    runner.verify_native_bindings(authority, inputs)

    def test_native_manifest_cannot_be_empty(self):
        for authority in ({}, {"native_inputs": []}):
            with self.subTest(authority=authority), self.assertRaisesRegex(runner.Refusal, "native-input-bound"):
                runner.verify_native_bindings(authority, {})

    def test_aggregate_keeps_missing_evidence_distinct_from_observed_failure(self):
        def rows(*statuses):
            return [{"analysis": {"status": status}} for status in statuses]
        self.assertEqual(runner.aggregate_status(rows("incomplete", "not-run"), {"status": "incomplete"}, True), "incomplete")
        self.assertEqual(runner.aggregate_status(rows("not-run", "not-run"), {"status": "incomplete"}, True), "not-run")
        self.assertEqual(runner.aggregate_status(rows("pass", "not-run"), {"status": "not-run"}, True), "incomplete")
        self.assertEqual(runner.aggregate_status(rows("pass", "fail"), {"status": "incomplete"}, True), "fail")
        self.assertEqual(runner.aggregate_status(rows("pass"), {"status": "pass"}, False), "fail")

    def test_windows_runtime_uses_its_own_directory_not_path_or_redirection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            library = root / "loader"
            runtime = root / "runtime"
            system = root / "Windows" / "System32"
            for directory in (library, runtime, system):
                directory.mkdir(parents=True)
            inputs = {"consumer": root / "consumer.exe", "fixture": root / "fixture.exe",
                      "runtime": runtime / "onnxruntime.dll"}
            inspector = root / "dumpbin.exe"
            shared = runtime / "shared.dll"
            for path in (*inputs.values(), inspector, shared):
                path.write_bytes(path.name.encode("ascii"))
            def binding(path):
                size, digest = runner.digest(path)
                return {"path": str(path), "bytes": size, "sha256": digest}
            authority = {"loader_directories": [str(library)],
                         "native_inspector": binding(inspector), "native_inputs": [binding(shared)],
                         "host": {"system_root": str(system.parent)}}
            def inspect(path, tool):
                imports = [{"name": "shared.dll", "kind": "PE-import"}] if path == inputs["runtime"] else []
                return {"architecture": "x64", "imports": imports, "rpaths": []}
            with mock.patch.object(runner.platform, "system", return_value="Windows"), \
                 mock.patch.object(runner, "ROOT", root), \
                 mock.patch.object(runner, "windows_dll_directory_length", return_value=0) as dll_directory, \
                 mock.patch.object(runner, "inspect_file", side_effect=inspect):
                evidence = runner.verify_native_bindings(authority, inputs)
                self.assertIn({"owner": str(inputs["runtime"]), "import": "shared.dll",
                               "resolved": str(shared)}, evidence["edges"])
                relocated = shared.rename(library / "shared.dll")
                authority["native_inputs"] = [binding(relocated)]
                with self.assertRaisesRegex(runner.Refusal, "unresolved-native-import"):
                    runner.verify_native_bindings(authority, inputs)
                relocated.rename(shared)
                authority["native_inputs"] = [binding(shared)]
                dll_directory.return_value = 12
                with self.assertRaisesRegex(runner.Refusal, "inherited-dll-directory"):
                    runner.verify_native_bindings(authority, inputs)
                dll_directory.return_value = 0
                redirection = root / "consumer.exe.local"
                redirection.mkdir()
                with self.assertRaisesRegex(runner.Refusal, "external-loader-redirection"):
                    runner.verify_native_bindings(authority, inputs)
                redirection.rmdir()
                (root / "fixture.exe.manifest").write_bytes(b"external activation context")
                with self.assertRaisesRegex(runner.Refusal, "external-loader-redirection"):
                    runner.verify_native_bindings(authority, inputs)

    def test_dll_directory_size_query_does_not_prove_a_custom_directory(self):
        import ctypes
        from types import SimpleNamespace
        for directory in ("", "owned"):
            with self.subTest(directory=directory):
                def get_directory(capacity, buffer):
                    if capacity == 0:
                        return len(directory) + 1
                    buffer.value = directory
                    return len(directory)
                function = mock.Mock(side_effect=get_directory)
                kernel = SimpleNamespace(GetDllDirectoryW=function)
                with mock.patch.object(ctypes, "WinDLL", return_value=kernel, create=True), \
                     mock.patch.object(ctypes, "set_last_error", create=True), \
                     mock.patch.object(ctypes, "get_last_error", return_value=0, create=True):
                    self.assertEqual(runner.windows_dll_directory_length(), len(directory))

    def test_dll_directory_growth_cannot_be_accepted_as_empty(self):
        import ctypes
        from types import SimpleNamespace
        kernel = SimpleNamespace(GetDllDirectoryW=mock.Mock(side_effect=[1, 8]))
        with mock.patch.object(ctypes, "WinDLL", return_value=kernel, create=True), \
             mock.patch.object(ctypes, "set_last_error", create=True), \
             mock.patch.object(ctypes, "get_last_error", return_value=0, create=True):
            with self.assertRaisesRegex(runner.Refusal, "dll-directory-readback-failed"):
                runner.windows_dll_directory_length()


if __name__ == "__main__":
    unittest.main()
