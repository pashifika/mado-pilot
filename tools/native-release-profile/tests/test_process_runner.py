"""Exercise process_runner with a stdlib child whose behaviour is gated by explicit arguments."""

import ctypes
import os
import shutil
import signal
import subprocess
import threading
import sys
import tempfile
import textwrap
import time
import unittest
import warnings
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import process_runner  # noqa: E402

WINDOWS = sys.platform == "win32"
if WINDOWS:
    import _windows_process  # noqa: E402

    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _kernel32.OpenProcess.restype = wintypes.HANDLE
    _kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    _kernel32.GetExitCodeProcess.restype = wintypes.BOOL
    _kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    _kernel32.CloseHandle.restype = wintypes.BOOL

CHILD = textwrap.dedent(
    """
    import os, subprocess, sys, time

    def emit(stream, data):
        stream.buffer.write(data)
        stream.buffer.flush()

    def record_pid(path, pid):
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(str(pid))

    mode = sys.argv[1]
    if mode == "exact":
        emit(sys.stdout, bytes.fromhex(sys.argv[2]))
        emit(sys.stderr, bytes.fromhex(sys.argv[3]))
        sys.exit(int(sys.argv[4]))
    if mode == "stdin":
        emit(sys.stdout, str(len(sys.stdin.buffer.read())).encode())
    if mode == "hang":
        record_pid(sys.argv[2], os.getpid())
        emit(sys.stdout, b"started\\n")
        time.sleep(60)
    if mode == "overflow":
        emit(sys.stdout, b"o" * int(sys.argv[2]))
        emit(sys.stderr, b"e" * 16)
        time.sleep(60)
    if mode == "sleep":
        time.sleep(60)
    if mode == "spawn":
        kind, pid_file = sys.argv[2], sys.argv[3]
        options = {"stdin": subprocess.DEVNULL}
        if kind == "detach":
            options.update(stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        else:
            options.update(stdout=sys.stdout, stderr=sys.stderr)
        if kind == "escape":
            options["start_new_session"] = True
        grandchild = subprocess.Popen([sys.executable, "-I", "-S", __file__, "sleep"], **options)
        record_pid(pid_file, grandchild.pid)
        emit(sys.stdout, b"spawned\\n")
        sys.exit(0)
    if mode == "marker":
        with open(sys.argv[2], "w", encoding="utf-8") as handle:
            handle.write("ran")
    """
)

STDOUT_ASCII = b"MADO_PROFILE_RESULT=passed\n"
STDERR_ASCII = b"note\n"


def child_env():
    names = ("SYSTEMROOT", "SYSTEMDRIVE", "TEMP", "TMP", "PATH")
    return {name: os.environ[name] for name in names if name in os.environ}


if WINDOWS:

    def process_alive(pid):
        handle = _kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            return False
        try:
            code = wintypes.DWORD()
            return bool(_kernel32.GetExitCodeProcess(handle, ctypes.byref(code))) and code.value == 259
        finally:
            _kernel32.CloseHandle(handle)

else:

    def process_alive(pid):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        observed = subprocess.run(["/bin/ps", "-p", str(pid), "-o", "stat="],
                                  capture_output=True, text=True, timeout=2, check=False)
        return any(not state.startswith("Z") for state in observed.stdout.split())


def process_gone(pid, seconds=5.0):
    deadline = time.monotonic() + seconds
    while process_alive(pid):
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.05)
    return True


def kill_quietly(pid):
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process_gone(pid)


class ProcessRunnerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = Path(tempfile.mkdtemp(prefix="process-runner-"))
        cls.script = cls.root / "child.py"
        cls.script.write_text(CHILD, encoding="utf-8")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.root, ignore_errors=True)

    def run_child(self, *args, timeout=30.0, limit=1 << 16, cleanup=5.0):
        return process_runner.run_process(
            [sys.executable, "-I", "-S", str(self.script), *args],
            cwd=self.root,
            env=child_env(),
            timeout_seconds=timeout,
            output_limit_bytes=limit,
            cleanup_seconds=cleanup,
        )

    def pid_from(self, path):
        return int(path.read_text(encoding="utf-8"))

    def assert_record(self, record):
        self.assertEqual(set(record), set(process_runner.RECORD_KEYS))
        self.assertIsInstance(record["duration_seconds"], float)
        self.assertGreaterEqual(record["duration_seconds"], 0.0)

    def assert_clean(self, record):
        self.assertFalse(record["timed_out"])
        self.assertFalse(record["output_limited"])
        self.assertTrue(record["cleanup_ok"])
        self.assertIsNone(record["launch_error"])

    def test_exact_output_at_the_limit_with_zero_exit(self):
        total = len(STDOUT_ASCII) + len(STDERR_ASCII)
        record = self.run_child("exact", STDOUT_ASCII.hex(), STDERR_ASCII.hex(), "0", limit=total)
        self.assert_record(record)
        self.assert_clean(record)
        self.assertEqual(record["exit_code"], 0)
        self.assertEqual(record["stdout"], STDOUT_ASCII.decode())
        self.assertEqual(record["stderr"], STDERR_ASCII.decode())

    def test_invalid_utf8_is_replaced_after_byte_capture(self):
        record = self.run_child("exact", b"ok \xff\n".hex(), b"caf\xc3\xa9\n".hex(), "0")
        self.assert_clean(record)
        self.assertEqual(record["exit_code"], 0)
        self.assertEqual(record["stdout"], "ok \ufffd\n")
        self.assertEqual(record["stderr"], "caf\u00e9\n")

    def test_nonzero_exit_is_reported_with_its_stderr(self):
        record = self.run_child("exact", "", b"refused\n".hex(), "3")
        self.assert_record(record)
        self.assert_clean(record)
        self.assertEqual(record["exit_code"], 3)
        self.assertEqual(record["stdout"], "")
        self.assertEqual(record["stderr"], "refused\n")

    def test_child_reads_no_input(self):
        record = self.run_child("stdin")
        self.assert_clean(record)
        self.assertEqual(record["exit_code"], 0)
        self.assertEqual(record["stdout"], "0")

    def test_one_byte_over_the_limit_marks_output_limited(self):
        total = len(STDOUT_ASCII) + len(STDERR_ASCII)
        record = self.run_child("exact", STDOUT_ASCII.hex(), STDERR_ASCII.hex(), "0", limit=total - 1)
        self.assert_record(record)
        self.assertTrue(record["output_limited"])
        self.assertFalse(record["timed_out"])
        self.assertTrue(record["cleanup_ok"])
        self.assertEqual(len(record["stdout"]) + len(record["stderr"]), total - 1)
        # The child may have finished on its own before the kill landed.
        self.assertIn(record["exit_code"], (0, None))

    def test_output_overflow_retains_the_limit_and_kills_the_child(self):
        record = self.run_child("overflow", "65536", limit=4096)
        self.assert_record(record)
        self.assertTrue(record["output_limited"])
        self.assertFalse(record["timed_out"])
        self.assertTrue(record["cleanup_ok"])
        self.assertIsNone(record["exit_code"])
        self.assertIsNone(record["launch_error"])
        self.assertEqual(len(record["stdout"]) + len(record["stderr"]), 4096)
        self.assertTrue(record["stdout"].startswith("o"))

    def test_overrun_times_out_after_draining_and_kills_the_child(self):
        pid_file = self.root / "hang.pid"
        record = self.run_child("hang", str(pid_file), timeout=3.0, cleanup=5.0)
        self.assert_record(record)
        self.assertTrue(record["timed_out"])
        self.assertIsNone(record["exit_code"])
        self.assertTrue(record["cleanup_ok"])
        self.assertFalse(record["output_limited"])
        self.assertIsNone(record["launch_error"])
        self.assertEqual(record["stdout"], "started\n")
        self.assertGreaterEqual(record["duration_seconds"], 3.0)
        self.assertLess(record["duration_seconds"], 10.0)
        self.assertTrue(process_gone(self.pid_from(pid_file)))

    def test_zero_exit_cannot_hide_a_descendant_holding_the_pipe(self):
        pid_file = self.root / "hold.pid"
        record = self.run_child("spawn", "hold", str(pid_file), timeout=3.0)
        self.assert_record(record)
        self.assertEqual(record["exit_code"], 0)
        self.assertTrue(record["timed_out"])
        self.assertTrue(record["cleanup_ok"])
        self.assertFalse(record["output_limited"])
        self.assertEqual(record["stdout"], "spawned\n")
        self.assertTrue(process_gone(self.pid_from(pid_file)))

    def test_completed_run_sweeps_a_detached_descendant(self):
        pid_file = self.root / "detach.pid"
        record = self.run_child("spawn", "detach", str(pid_file))
        self.assert_record(record)
        self.assert_clean(record)
        self.assertEqual(record["exit_code"], 0)
        self.assertEqual(record["stdout"], "spawned\n")
        self.assertFalse(process_alive(self.pid_from(pid_file)))

    @unittest.skipIf(WINDOWS, "a job denies breakaway, so no descendant can escape")
    def test_zero_exit_cannot_hide_cleanup_failure(self):
        pid_file = self.root / "escape.pid"
        prior_readers = {thread for thread in threading.enumerate() if thread.name.startswith("process-runner-pipe-")}
        record = self.run_child("spawn", "escape", str(pid_file), timeout=3.0, cleanup=0.5)
        pid = self.pid_from(pid_file)
        self.addCleanup(kill_quietly, pid)
        self.assert_record(record)
        self.assertEqual(record["exit_code"], 0)
        self.assertTrue(record["timed_out"])
        self.assertFalse(record["cleanup_ok"])
        self.assertFalse(record["output_limited"])
        self.assertEqual(record["stdout"], "spawned\n")
        self.assertTrue(process_alive(pid))
        self.assertEqual({thread for thread in threading.enumerate() if thread.name.startswith("process-runner-pipe-")},
                         prior_readers)

    def test_reader_start_failure_stops_the_child_and_started_reader(self):
        created = []
        popen = subprocess.Popen
        reader = process_runner._start_reader

        def launch(*args, **kwargs):
            child = popen(*args, **kwargs)
            created.append(child)
            return child

        def start(stream, capture, index):
            if index == 1:
                raise RuntimeError("reader-start-refused")
            return reader(stream, capture, index)

        with mock.patch.object(subprocess, "Popen", side_effect=launch), \
             mock.patch.object(process_runner, "_start_reader", side_effect=start):
            with self.assertRaisesRegex(RuntimeError, "reader-start-refused"):
                self.run_child("sleep")
        self.assertIsNotNone(created[0].returncode)
        self.assertFalse(process_alive(created[0].pid))
        self.assertFalse(any(thread.name.startswith("process-runner-pipe-") for thread in threading.enumerate()))

    def test_completion_observed_after_deadline_is_not_a_pass(self):
        clock = time.monotonic
        await_exit = process_runner._Tree.await_exit
        shifted = 0.0

        def observe(tree, deadline):
            nonlocal shifted
            completed = await_exit(tree, deadline)
            shifted = 60.0
            return completed

        with mock.patch.object(time, "monotonic", side_effect=lambda: clock() + shifted), \
             mock.patch.object(process_runner._Tree, "await_exit", observe):
            record = self.run_child("exact", "", "", "0")
        self.assertEqual(record["exit_code"], 0)
        self.assertTrue(record["timed_out"])

    def test_cleanup_observed_after_deadline_is_not_success(self):
        clock = time.monotonic
        reap = process_runner._reap
        shifted = 0.0

        def observe(child, deadline):
            nonlocal shifted
            status = reap(child, deadline)
            shifted = 60.0
            return status

        with mock.patch.object(time, "monotonic", side_effect=lambda: clock() + shifted), \
             mock.patch.object(process_runner, "_reap", side_effect=observe):
            record = self.run_child("exact", "", "", "0")
        self.assertEqual(record["exit_code"], 0)
        self.assertFalse(record["cleanup_ok"])

    def test_missing_executable_is_a_launch_error(self):
        record = process_runner.run_process(
            [str(self.root / "missing-consumer")],
            cwd=self.root,
            env=child_env(),
            timeout_seconds=5.0,
            output_limit_bytes=1024,
        )
        self.assert_record(record)
        self.assertTrue(record["launch_error"])
        self.assertIsNone(record["exit_code"])
        self.assertFalse(record["timed_out"])
        self.assertFalse(record["output_limited"])
        self.assertTrue(record["cleanup_ok"])
        self.assertEqual(record["stdout"], "")
        self.assertEqual(record["stderr"], "")

    def test_missing_cwd_is_a_launch_error(self):
        record = process_runner.run_process(
            [sys.executable, "-I", "-S", str(self.script), "stdin"],
            cwd=self.root / "absent",
            env=child_env(),
            timeout_seconds=5.0,
            output_limit_bytes=1024,
        )
        self.assertTrue(record["launch_error"])
        self.assertIsNone(record["exit_code"])
        self.assertTrue(record["cleanup_ok"])
        self.assertEqual(record["stdout"], "")

    def test_invalid_arguments_raise_value_error(self):
        good = dict(
            argv=[sys.executable, "-c", "pass"],
            cwd=self.root,
            env={"A": "1"},
            timeout_seconds=1.0,
            output_limit_bytes=1,
            cleanup_seconds=1.0,
        )
        for field, value in [
            ("argv", []),
            ("argv", ["python3"]),
            ("argv", [sys.executable, b"-c"]),
            ("argv", [sys.executable, "a\0b"]),
            ("argv", (sys.executable,)),
            ("cwd", Path("relative")),
            ("cwd", None),
            ("env", {"A": 1}),
            ("env", {"A=B": "1"}),
            ("env", {"": "1"}),
            ("env", {"A": "x\0y"}),
            ("env", [("A", "1")]),
            ("timeout_seconds", 0),
            ("timeout_seconds", -1.0),
            ("timeout_seconds", float("inf")),
            ("timeout_seconds", True),
            ("output_limit_bytes", -1),
            ("output_limit_bytes", 1.5),
            ("output_limit_bytes", True),
            ("cleanup_seconds", 0.0),
            ("cleanup_seconds", float("nan")),
        ]:
            with self.subTest(field=field, value=value):
                arguments = {**good, field: value}
                argv = arguments.pop("argv")
                with self.assertRaises(ValueError):
                    process_runner.run_process(argv, **arguments)

    @unittest.skipUnless(WINDOWS, "Windows job ownership")
    def test_windows_child_never_runs_when_job_assignment_fails(self):
        marker = self.root / "marker.txt"

        def refuse(job, process):
            ctypes.set_last_error(5)
            return 0

        with mock.patch.object(_windows_process._kernel32, "AssignProcessToJobObject", refuse):
            record = self.run_child("marker", str(marker))
        self.assert_record(record)
        self.assertIn("AssignProcessToJobObject", record["launch_error"])
        self.assertIn("WinError 5", record["launch_error"])
        self.assertIsNone(record["exit_code"])
        self.assertTrue(record["cleanup_ok"])
        self.assertFalse(record["timed_out"])
        self.assertEqual(record["stdout"], "")
        self.assertFalse(marker.exists())

    @unittest.skipUnless(WINDOWS, "Windows launch cleanup failures")
    def test_windows_failed_launch_preserves_failed_cleanup(self):
        popen = subprocess.Popen
        for operation in ("kill", "wait"):
            with self.subTest(operation=operation):
                created = []

                def launch(*args, **kwargs):
                    child = popen(*args, **kwargs)
                    created.append(child)
                    return child

                failure = OSError("kill-refused") if operation == "kill" else subprocess.TimeoutExpired("child", 0)
                try:
                    with mock.patch.object(subprocess, "Popen", side_effect=launch), \
                         mock.patch.object(popen, operation, side_effect=failure), \
                         mock.patch.object(_windows_process, "_assign", side_effect=OSError("assignment-refused")):
                        record = self.run_child("marker", str(self.root / "launch-marker"))
                    self.assertIn("assignment-refused", record["launch_error"])
                    self.assertFalse(record["cleanup_ok"])
                finally:
                    for child in created:
                        child.kill()
                        child.wait(timeout=5)
                        child._handle.Close()

    @unittest.skipUnless(WINDOWS, "Windows job ownership")
    def test_windows_job_termination_failure_is_reported(self):
        pid_file = self.root / "terminate.pid"

        def refuse(job, exit_code):
            ctypes.set_last_error(6)
            return 0

        with warnings.catch_warnings(record=True) as observed:
            warnings.simplefilter("always", ResourceWarning)
            with mock.patch.object(_windows_process._kernel32, "TerminateJobObject", refuse):
                record = self.run_child("hang", str(pid_file), timeout=3.0, cleanup=0.5)
        self.assertFalse([warning for warning in observed if issubclass(warning.category, ResourceWarning)])
        self.assert_record(record)
        self.assertTrue(record["timed_out"])
        self.assertFalse(record["cleanup_ok"])
        self.assertIsNone(record["exit_code"])
        self.assertEqual(record["stdout"], "started\n")
        # Closing the job still ends the tree.
        self.assertTrue(process_gone(self.pid_from(pid_file)))


if __name__ == "__main__":
    unittest.main()
