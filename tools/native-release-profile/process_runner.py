"""Run one explicit native command with bounded output, a hard deadline, and owned cleanup.

``run_process`` reports operational facts; it never decides whether a
qualification row passed.  Record keys, always present:

- ``exit_code``: the child's own exit status; ``None`` when the runner killed
  it, could not reap it within ``cleanup_seconds``, or never launched it.
- ``timed_out``: the child had not exited with both pipes at EOF before
  ``timeout_seconds``.  A later exit, even zero, keeps this ``True``.
- ``output_limited``: stdout and stderr together exceeded
  ``output_limit_bytes``; the tree was killed and exactly the limit retained.
- ``cleanup_ok``: no owned member can execute, the leader was reaped, and both
  pipes hit EOF within ``cleanup_seconds``. Descendant zombies belong to their parent.
- ``stdout`` / ``stderr``: retained bytes decoded as UTF-8 with replacement.
- ``duration_seconds``: wall time from the launch attempt to this record.
- ``launch_error``: why the child never executed, else ``None``.
- ``process_id``: the owned root PID, or ``None`` when launch failed.
- ``cpu``: lifetime native-scope accounting, separate from operational success.
  Missing usage is null; POSIX descendant rollup remains partial. Native
  representation units do not establish accounting resolution or accuracy.

The child leads a new POSIX session/process group, or is the first member of a
Windows job that denies breakaway and kills on close; only that group or job is
ever signalled.  No shell is involved, stdin is the null device, and ``env`` is
passed verbatim (Windows children usually need ``SystemRoot`` in it).
"""

from __future__ import annotations

import math
import os
import select
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

_CHUNK_BYTES = 65536
_POLL_SECONDS = 0.01


def run_process(
    argv: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    timeout_seconds: float,
    output_limit_bytes: int,
    cleanup_seconds: float = 5.0,
) -> dict:
    """Run ``argv`` directly and return the record described in the module docstring."""
    _validate(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds)
    started = time.monotonic()
    capture = _Capture(output_limit_bytes)
    tree = _Tree(cleanup_seconds)
    readers = []
    proc = None
    try:
        try:
            proc = tree.launch(argv, os.fspath(cwd), env)
        except OSError as exc:
            return _record(
                process_id=None,
                exit_code=None,
                timed_out=False,
                output_limited=False,
                cleanup_ok=tree.launch_cleanup_ok,
                capture=capture,
                started=started,
                launch_error=str(exc) or type(exc).__name__,
                cpu=_cpu_record(tree, launch_completed=False),
            )
        for stream, index in ((proc.stdout, 0), (proc.stderr, 1)):
            readers.append(_start_reader(stream, capture, index))
        deadline = started + timeout_seconds
        with capture.cond:
            while capture.open_streams and not capture.limited:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                capture.cond.wait(remaining)
            limited = capture.limited
            drained = capture.open_streams == 0
        if limited:
            completed = False
            timed_out = time.monotonic() >= deadline
        elif drained:
            completed = tree.await_exit(deadline) and time.monotonic() <= deadline
            timed_out = not completed
        else:
            completed = False
            timed_out = True
        cleanup_deadline = time.monotonic() + cleanup_seconds
        # One sweep in every path: a live leader dies, a finished one only loses descendants.
        killed = tree.terminate()
        stopped = tree.await_cleanup(cleanup_deadline)
        status = tree.reap(cleanup_deadline)
        drain_deadline = cleanup_deadline - min(0.1, cleanup_seconds / 2)
        for reader in readers:
            reader.join(max(0.0, drain_deadline - time.monotonic()))
        capture.stop.set()
        for reader in readers:
            reader.join(max(0.0, cleanup_deadline - time.monotonic()))
        exit_code = status
        if not completed and status is not None and tree.is_kill_status(status):
            exit_code = None
        return _record(
            process_id=proc.pid,
            exit_code=exit_code,
            timed_out=timed_out,
            output_limited=capture.limited,
            cleanup_ok=(killed and stopped and status is not None and capture.eof_streams == 2
                        and not any(r.is_alive() for r in readers) and time.monotonic() <= cleanup_deadline),
            capture=capture,
            started=started,
            launch_error=None,
            cpu=_cpu_record(tree, launch_completed=True),
        )
    finally:
        capture.stop.set()
        tree.close()
        for reader in readers:
            reader.join(timeout=0.1)
        if proc is not None:
            for stream in (proc.stdout, proc.stderr)[len(readers):]:
                stream.close()


def _validate(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds):
    if (
        not isinstance(argv, list)
        or not argv
        or not all(isinstance(item, str) and "\0" not in item for item in argv)
    ):
        raise ValueError("argv must be a non-empty list of NUL-free str")
    if not os.path.isabs(argv[0]):
        raise ValueError("argv[0] must be an absolute executable path")
    if (
        not isinstance(cwd, (str, os.PathLike))
        or not isinstance(os.fspath(cwd), str)
        or not os.path.isabs(os.fspath(cwd))
    ):
        raise ValueError("cwd must be an absolute path")
    if not isinstance(env, dict) or not all(
        isinstance(name, str)
        and isinstance(value, str)
        and name
        and "=" not in name
        and "\0" not in name
        and "\0" not in value
        for name, value in env.items()
    ):
        raise ValueError("env must map non-empty '='-free str names to NUL-free str values")
    if not _positive_finite(timeout_seconds):
        raise ValueError("timeout_seconds must be a positive finite number")
    if (
        isinstance(output_limit_bytes, bool)
        or not isinstance(output_limit_bytes, int)
        or output_limit_bytes < 0
    ):
        raise ValueError("output_limit_bytes must be a non-negative int")
    if not _positive_finite(cleanup_seconds):
        raise ValueError("cleanup_seconds must be a positive finite number")


def _positive_finite(value) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(value)
        and value > 0
    )


def _record(*, process_id, exit_code, timed_out, output_limited, cleanup_ok, capture, started, launch_error, cpu):
    return {
        "exit_code": exit_code,
        "timed_out": timed_out,
        "output_limited": output_limited,
        "cleanup_ok": cleanup_ok,
        "stdout": capture.text(0),
        "stderr": capture.text(1),
        "duration_seconds": time.monotonic() - started,
        "launch_error": launch_error,
        "process_id": process_id,
        "cpu": cpu,
    }


def _cpu_record(tree, *, launch_completed):
    record = tree.cpu_snapshot()
    reason = None
    if not launch_completed:
        reason = "launch_not_completed"
    elif record["status"] != "unknown":
        values = (record["user_seconds"], record["system_seconds"])
        if not all(isinstance(value, (int, float)) and not isinstance(value, bool)
                   and math.isfinite(value) and value >= 0 for value in values):
            reason = "invalid_native_accounting"
    if reason is not None:
        record.update(user_seconds=None, system_seconds=None, status="unknown",
                      final=False, reason=reason)
    return record


def _reap(proc, deadline):
    try:
        return proc.wait(timeout=max(0.0, deadline - time.monotonic()))
    except subprocess.TimeoutExpired:
        return None


class _Capture:
    """Total-bounded byte capture shared by the two pipe readers."""

    def __init__(self, limit: int):
        self.cond = threading.Condition()
        self.remaining = limit
        self.limited = False
        self.open_streams = 2
        self.eof_streams = 0
        self.stop = threading.Event()
        self.buffers = (bytearray(), bytearray())

    def text(self, index: int) -> str:
        with self.cond:
            return self.buffers[index].decode("utf-8", "replace")


def _start_reader(stream, capture, index):
    thread = threading.Thread(
        target=_drain, args=(stream, capture, index), name=f"process-runner-pipe-{index}", daemon=True
    )
    thread.start()
    return thread


def _drain(stream, capture, index):
    buffer = capture.buffers[index]
    fd = stream.fileno()
    try:
        os.set_blocking(fd, False)
        while not capture.stop.is_set():
            try:
                chunk = os.read(fd, _CHUNK_BYTES)
            except BlockingIOError:
                capture.stop.wait(_POLL_SECONDS)
                continue
            except OSError:
                break
            if not chunk:
                with capture.cond:
                    capture.eof_streams += 1
                break
            with capture.cond:
                room = capture.remaining
                if len(chunk) > room:
                    buffer += chunk[:room]
                    capture.remaining = 0
                    if not capture.limited:
                        capture.limited = True
                        capture.cond.notify_all()
                else:
                    buffer += chunk
                    capture.remaining -= len(chunk)
    finally:
        stream.close()
        with capture.cond:
            capture.open_streams -= 1
            capture.cond.notify_all()


class _PosixTree:
    """Owns the child's new session/process group and signals it before reaping the leader."""

    def __init__(self, cleanup_seconds: float):
        self._proc = None
        self._swept = False
        self._cleanup_seconds = cleanup_seconds
        self.launch_cleanup_ok = True
        self._cpu_times = None
        self._cpu_reason = "root_not_reaped"

    def launch(self, argv, cwd, env):
        self._proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            bufsize=0,
            start_new_session=True,
        )
        return self._proc

    def await_exit(self, deadline) -> bool:
        return _await_exit_unreaped(self._proc, deadline, self.reap)

    def terminate(self) -> bool:
        self._swept = True
        try:
            os.killpg(self._proc.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass  # Every member is gone, or (macOS) only the unreaped zombie leader remains.
        return True

    def await_cleanup(self, deadline) -> bool:
        from _process_group import no_executing_members
        while time.monotonic() < deadline:
            if no_executing_members(self._proc.pid):
                return True
            time.sleep(min(_POLL_SECONDS, max(0.0, deadline - time.monotonic())))
        return False

    def reap(self, deadline):
        if self._proc.returncode is not None:
            if self._cpu_times is None and self._cpu_reason == "root_not_reaped":
                self._cpu_reason = "already_reaped_without_usage"
            return self._proc.returncode
        if not hasattr(os, "wait4"):
            self._cpu_reason = "wait4_unavailable"
            return _reap(self._proc, deadline)
        while True:
            try:
                pid, status, usage = os.wait4(self._proc.pid, os.WNOHANG)
            except InterruptedError:
                pid = 0
            except ChildProcessError:
                self._cpu_reason = "already_reaped_without_usage"
                return _reap(self._proc, deadline)
            except OSError:
                self._cpu_reason = "wait4_failed"
                return _reap(self._proc, deadline)
            if pid == self._proc.pid:
                if os.WIFEXITED(status) or os.WIFSIGNALED(status):
                    self._proc.returncode = os.waitstatus_to_exitcode(status)
                    self._cpu_times = (usage.ru_utime, usage.ru_stime)
                    self._cpu_reason = "descendant_rollup_unproven"
                    return self._proc.returncode
            elif pid != 0:
                self._cpu_reason = "wait4_pid_mismatch"
                return _reap(self._proc, deadline)
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self._cpu_reason = "reap_deadline"
                return None
            time.sleep(min(_POLL_SECONDS, remaining))

    def cpu_snapshot(self):
        values = self._cpu_times
        return {
            "user_seconds": None if values is None else values[0],
            "system_seconds": None if values is None else values[1],
            "source": "posix-wait4",
            "scope": "root-and-native-descendant-rollup",
            "status": "unknown" if values is None else "partial",
            "final": values is not None,
            "reason": self._cpu_reason,
            "source_unit_seconds": 1e-6,
            "accounting_resolution_seconds": None,
        }

    @staticmethod
    def is_kill_status(status) -> bool:
        return status == -signal.SIGKILL

    def close(self):
        if self._proc is not None and not self._swept:
            self.terminate()
            deadline = time.monotonic() + self._cleanup_seconds
            self.await_cleanup(deadline)
            self.reap(deadline)


# The leader stays a zombie until the sweep so its pgid cannot be recycled meanwhile.
if hasattr(select, "kqueue"):

    def _await_exit_unreaped(proc, deadline, reap) -> bool:
        queue = select.kqueue()
        try:
            exit_event = select.kevent(
                proc.pid,
                select.KQ_FILTER_PROC,
                select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                select.KQ_NOTE_EXIT,
            )
            try:
                queue.control([exit_event], 0)
            except ProcessLookupError:
                return True
            return bool(queue.control(None, 1, max(0.0, deadline - time.monotonic())))
        finally:
            queue.close()

elif hasattr(os, "waitid"):

    def _await_exit_unreaped(proc, deadline, reap) -> bool:
        flags = os.WEXITED | os.WNOHANG | os.WNOWAIT
        while os.waitid(os.P_PID, proc.pid, flags) is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return False
            time.sleep(min(remaining, _POLL_SECONDS))
        return True

else:

    def _await_exit_unreaped(proc, deadline, reap) -> bool:
        return reap(deadline) is not None


if sys.platform == "win32":
    from _windows_process import JobTree as _Tree
else:
    _Tree = _PosixTree
