"""Owned loader lifetimes, refusal, output limits and cleanup; no capture/input."""

import base64
import ctypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "windows_image_observer",
    Path(__file__).resolve().parents[1] / "native-release-profile/windows_image_observer.py",
)
observer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(observer)


class ReceiptTests(unittest.TestCase):
    def test_interrupted_observer_preserves_output_and_never_invents_target_exit(self):
        prefix = json.dumps({"type": "output", "stream": 0,
                             "bytes": base64.b64encode(b"finished first operation\n").decode()})
        outer = {"exit_code": None, "timed_out": True, "output_limited": False,
                 "cleanup_ok": True, "stdout": prefix + '\n{"type":"image",',
                 "stderr": "", "duration_seconds": 1.0, "launch_error": None}
        result = observer._decode(outer, 1024, 4, 4)
        self.assertEqual(result["record"]["stdout"], "finished first operation\n")
        self.assertIsNone(result["record"]["exit_code"])
        self.assertTrue(result["record"]["timed_out"])
        self.assertTrue(result["cleanup_ok"])
        self.assertFalse(result["provenance_complete"])

    def test_completed_target_outcome_survives_outer_cleanup_failure(self):
        target = {"exit_code": 23, "timed_out": False, "output_limited": True,
                  "cleanup_ok": True, "stdout": "", "stderr": "",
                  "duration_seconds": 0.5, "launch_error": None}
        receipt = json.dumps({"type": "done", "record": target, "reason": "output_limit_exceeded",
                              "cleanup_ok": True, "provenance_complete": False}) + "\n"
        outer = {"exit_code": 0, "timed_out": False, "output_limited": False,
                 "cleanup_ok": False, "stdout": receipt, "stderr": "",
                 "duration_seconds": 1.0, "launch_error": None}
        for suffix in ("", '{"truncated":'):
            with self.subTest(suffix=suffix):
                result = observer._decode(dict(outer, stdout=receipt + suffix), 1024, 4, 4)
                self.assertEqual(result["record"]["exit_code"], 23)
                self.assertTrue(result["record"]["output_limited"])
                self.assertFalse(result["record"]["timed_out"])
                self.assertFalse(result["provenance_complete"])
                self.assertFalse(result["cleanup_ok"])

    def test_nul_working_directory_refuses_before_observer_launch(self):
        runner = observer._sibling("process_runner")
        with mock.patch.object(observer, "_sibling", return_value=runner), \
                mock.patch.object(runner, "run_process") as launch:
            result = observer.observe([sys.executable, "-c", "raise SystemExit(12)"],
                                      cwd=str(Path.cwd()) + "\0suffix", env={},
                                      timeout_seconds=1, output_limit_bytes=1, cleanup_seconds=1)
        launch.assert_not_called()
        self.assertTrue(result["reason"].startswith("invalid_arguments:"), result)
        self.assertFalse(result["provenance_complete"])
        self.assertTrue(result["cleanup_ok"])

    def test_image_budget_cannot_outlive_root_drain(self):
        state = object.__new__(observer._Observer)
        state.deadline, state.cleanup_deadline, state.root_drain_deadline = 100, None, 10
        state.pump, state.output_limited = lambda: None, False
        with mock.patch.object(observer.time, "monotonic", return_value=11):
            with self.assertRaisesRegex(observer._Refusal, "root_cleanup_deadline_exceeded"):
                state.budget()

    def test_expired_grace_cannot_be_successful_settlement(self):
        state = object.__new__(observer._Observer)
        state.deadline, state.cleanup_deadline, state.root_drain_deadline = 100, None, 10
        state.plan, state.reason, state.proc = {"cleanup_seconds": 5}, None, None
        state.root_reaped, state.root_exit, state.cleanup_ok = True, 23, True
        state.output_limited = state.timed_out = state.killed = False
        state.live, state.eof = {}, [True, True]
        state.pump = lambda: None
        state.counts = lambda: SimpleNamespace(active=0)
        with mock.patch.object(observer.time, "monotonic", return_value=11):
            state.loop()
        self.assertEqual(state.reason, "root_cleanup_deadline_exceeded")

    @unittest.skipIf(sys.platform == "win32", "unsupported-platform boundary")
    def test_unsupported_host_refuses_without_launch(self):
        result = observer.observe([sys.executable, "-c", "raise SystemExit(12)"],
                                  cwd=Path.cwd(), env={}, timeout_seconds=1,
                                  output_limit_bytes=1, cleanup_seconds=1)
        self.assertIsNone(result["record"]["exit_code"])
        self.assertFalse(result["provenance_complete"])
        self.assertTrue(result["cleanup_ok"])
        self.assertEqual(result["processes"], [])


@unittest.skipUnless(sys.platform == "win32" and ctypes.sizeof(ctypes.c_void_p) == 8,
                     "actual Windows x64 loader/Job contract")
class WindowsPreflightTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="mado-image-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.python = str(Path(sys.executable).resolve())

    def run_child(self, code, **options):
        limits = dict(timeout_seconds=30, output_limit_bytes=4096, cleanup_seconds=5)
        limits.update(options)
        return observer.observe([self.python, "-I", "-c", code], cwd=self.root,
                                env=dict(os.environ), **limits)

    def assert_stopped(self, result):
        self.assertTrue(result["cleanup_ok"], result)
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
        kernel.OpenProcess.restype = ctypes.c_void_p
        kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        kernel.WaitForSingleObject.restype = ctypes.c_uint32
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        kernel.CloseHandle.restype = ctypes.c_int
        for process in result["processes"]:
            handle = kernel.OpenProcess(0x00100000, False, process["pid"])
            if not handle:
                self.assertEqual(ctypes.get_last_error(), 87)
                continue
            try:
                self.assertEqual(kernel.WaitForSingleObject(handle, 0), 0)
            finally:
                kernel.CloseHandle(handle)

    def test_short_lived_image_and_grandchild_are_bound_before_exit(self):
        dll = self.root / "owned-transient.dll"
        shutil.copyfile(Path(os.environ["SystemRoot"]) / "System32/version.dll", dll)
        expected = hashlib.sha256(dll.read_bytes()).hexdigest()
        grandchild = (
            "import ctypes; k=ctypes.WinDLL('kernel32',use_last_error=True); "
            "k.LoadLibraryW.argtypes=[ctypes.c_wchar_p]; k.LoadLibraryW.restype=ctypes.c_void_p; "
            "k.FreeLibrary.argtypes=[ctypes.c_void_p]; k.FreeLibrary.restype=ctypes.c_int; "
            f"h=k.LoadLibraryW({str(dll)!r}); assert h; assert k.FreeLibrary(h); "
            "print('grandchild-complete',flush=True)"
        )
        child = f"import subprocess,sys; subprocess.run([sys.executable,'-I','-c',{grandchild!r}],check=True)"
        result = self.run_child(
            f"import subprocess,sys; subprocess.run([sys.executable,'-I','-c',{child!r}],check=True)")
        self.assert_stopped(result)
        self.assertEqual(result["record"]["exit_code"], 0, result)
        self.assertTrue(result["provenance_complete"], result)
        fixtures = [image for image in result["images"] if image["kind"] == "executable"
                    and Path(image["path"]).samefile(self.python)]
        self.assertEqual(len(fixtures), 3)
        self.assertTrue(all(process["exit_event"] is not None for process in result["processes"]))
        self.assertEqual(result["record"]["stdout"], "grandchild-complete\r\n")
        images = [image for image in result["images"] if Path(image["path"]).samefile(dll)]
        self.assertEqual(len(images), 1)
        image = images[0]
        self.assertEqual(image["sha256"], expected)
        self.assertEqual(image["post_sha256"], expected)
        self.assertEqual(image["release_kind"], "unload")
        self.assertLess(image["load_event"], image["release_event"])
        self.assertTrue(image["identity_verified_at_release"])
        self.assertTrue(all(image["identity_verified_at_release"] for image in result["images"]))
        dll.unlink()  # No observer file lease remains after unload/settlement.

    def test_nonzero_child_exit_is_not_lost_in_successful_observer(self):
        result = self.run_child("raise SystemExit(23)")
        self.assert_stopped(result)
        self.assertEqual(result["record"]["exit_code"], 23, result)
        self.assertTrue(result["provenance_complete"], result)
        self.assertFalse(result["record"]["timed_out"])

    def test_timeout_stops_owned_child_tree(self):
        child = "import time; print('child-ready',flush=True); time.sleep(120)"
        result = self.run_child(
            f"import subprocess,sys; subprocess.run([sys.executable,'-I','-c',{child!r}],check=True)",
            timeout_seconds=3)
        self.assert_stopped(result)
        self.assertTrue(result["record"]["timed_out"], result)
        self.assertIsNone(result["record"]["exit_code"])
        self.assertFalse(result["provenance_complete"])

    def test_descendant_finishing_after_root_is_observed_without_forced_cleanup(self):
        child = "import time; time.sleep(0.2); print('descendant-complete',flush=True)"
        result = self.run_child(
            f"import subprocess,sys; subprocess.Popen([sys.executable,'-I','-c',{child!r}]); "
            "raise SystemExit(23)")
        self.assert_stopped(result)
        self.assertEqual(result["record"]["exit_code"], 23, result)
        self.assertTrue(result["provenance_complete"], result)
        self.assertFalse(result["forced_owned_cleanup"])
        self.assertEqual(result["record"]["stdout"], "descendant-complete\r\n")
        self.assertTrue(all(image["identity_verified_at_release"] for image in result["images"]))

    def test_surviving_descendant_is_stopped_without_claiming_complete_lifetime(self):
        child = "import time; print('ready',flush=True); time.sleep(120)"
        result = self.run_child(
            "import subprocess,sys; "
            f"p=subprocess.Popen([sys.executable,'-I','-c',{child!r}],stdout=subprocess.PIPE); "
            "assert p.stdout.readline(); print('parent-complete',flush=True); raise SystemExit(23)")
        self.assert_stopped(result)
        self.assertEqual(result["reason"], "descendants_survived_root", result)
        fixtures = [image for image in result["images"] if image["kind"] == "executable"
                    and Path(image["path"]).samefile(self.python)]
        self.assertEqual(len(fixtures), 2)
        self.assertEqual(result["record"]["exit_code"], 23, result)
        self.assertTrue(result["forced_owned_cleanup"])
        self.assertFalse(result["provenance_complete"])

    def test_output_limit_retains_exact_prefix_and_stops_child(self):
        result = self.run_child("import os,time; os.write(1,b'x'*4096); time.sleep(120)",
                                output_limit_bytes=127)
        self.assert_stopped(result)
        self.assertEqual(result["record"]["stdout"], "x" * 127)
        self.assertTrue(result["record"]["output_limited"], result)
        self.assertIsNone(result["record"]["exit_code"])
        self.assertFalse(result["provenance_complete"])

    def test_image_limit_refuses_before_application_code(self):
        marker = self.root / "executed"
        result = self.run_child(f"from pathlib import Path; Path({str(marker)!r}).touch()", image_limit=1)
        self.assert_stopped(result)
        self.assertEqual(result["reason"], "image_limit_exceeded", result)
        self.assertFalse(result["provenance_complete"])
        self.assertFalse(marker.exists())

    def test_event_limit_refuses_before_application_code(self):
        marker = self.root / "executed"
        result = self.run_child(f"from pathlib import Path; Path({str(marker)!r}).touch()", event_limit=1)
        self.assert_stopped(result)
        self.assertEqual(result["reason"], "event_limit_exceeded", result)
        self.assertFalse(result["provenance_complete"])
        self.assertFalse(marker.exists())

    def test_missing_executable_has_no_execution_or_false_provenance(self):
        result = observer.observe([str(self.root / "missing.exe")], cwd=self.root,
                                  env=dict(os.environ), timeout_seconds=2,
                                  output_limit_bytes=0, cleanup_seconds=2)
        self.assertTrue(result["cleanup_ok"], result)
        self.assertIsNotNone(result["record"]["launch_error"])
        self.assertIsNone(result["record"]["exit_code"])
        self.assertEqual(result["processes"], [])
        self.assertFalse(result["provenance_complete"])

    def test_environment_and_working_directory_are_child_only(self):
        with mock.patch.dict(os.environ, {"MADO_PARENT_ONLY_SENTINEL": "must-not-leak"}):
            original = dict(os.environ)
            environment = dict(original, MADO_PREFLIGHT_TEST_VALUE="child-only")
            del environment["MADO_PARENT_ONLY_SENTINEL"]
            kernel = ctypes.WinDLL("kernel32")
            kernel.GetErrorMode.argtypes, kernel.GetErrorMode.restype = [], ctypes.c_uint
            error_mode = kernel.GetErrorMode()
            result = observer.observe(
                [self.python, "-I", "-c",
                 "import os,json; print(json.dumps([os.getcwd(),os.environ['MADO_PREFLIGHT_TEST_VALUE'],"
                 "os.environ.get('MADO_PARENT_ONLY_SENTINEL')]))"],
                cwd=self.root, env=environment, timeout_seconds=30, output_limit_bytes=4096, cleanup_seconds=5)
            self.assert_stopped(result)
            self.assertTrue(result["provenance_complete"], result)
            self.assertEqual(json.loads(result["record"]["stdout"]), [str(self.root), "child-only", None])
            self.assertEqual(os.environ, original)
            self.assertNotIn("MADO_PARENT_ONLY_SENTINEL", environment)
            self.assertEqual(environment["MADO_PREFLIGHT_TEST_VALUE"], "child-only")
            self.assertEqual(kernel.GetErrorMode(), error_mode)

    def test_application_breakpoint_is_not_swallowed_as_loader_breakpoint(self):
        result = self.run_child("import ctypes; ctypes.windll.kernel32.DebugBreak(); print('incorrectly-resumed')")
        self.assert_stopped(result)
        self.assertEqual(result["record"]["exit_code"], 0x80000003, result)
        self.assertNotIn("incorrectly-resumed", result["record"]["stdout"])
        self.assertTrue(result["provenance_complete"], result)

    def test_protocol_directory_cleanup_failure_retains_target_outcome(self):
        directory = tempfile.TemporaryDirectory(prefix="mado-preflight-cleanup-")

        class FailedCleanup:
            def __enter__(self):
                return directory.name

            def __exit__(self, *_):
                directory.cleanup()
                raise OSError("injected directory cleanup failure")

        with mock.patch.object(observer.tempfile, "TemporaryDirectory", return_value=FailedCleanup()):
            result = self.run_child("print('retained-output',flush=True); raise SystemExit(23)")
        self.assertEqual(result["record"]["exit_code"], 23, result)
        self.assertEqual(result["record"]["stdout"], "retained-output\r\n")
        self.assertIsNone(result["record"]["launch_error"])
        self.assertFalse(result["provenance_complete"])
        self.assertFalse(result["cleanup_ok"])
        self.assertEqual(result["reason"], "observer_io_failed")

    def test_invalid_limits_refuse_without_creating_target(self):
        marker = self.root / "executed"
        result = self.run_child(f"from pathlib import Path; Path({str(marker)!r}).touch()", image_limit=True)
        self.assertTrue(result["cleanup_ok"])
        self.assertFalse(result["provenance_complete"])
        self.assertIsNotNone(result["record"]["launch_error"])
        self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
