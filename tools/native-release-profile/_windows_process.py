"""Windows job ownership for process_runner: own the child before its first instruction.

The child is created suspended, assigned to an anonymous job that denies
breakaway and kills on close, then resumed.  It therefore cannot execute or
spawn before ownership exists, and every descendant stays in the job.
"""

from __future__ import annotations

import ctypes
import subprocess
import time
from ctypes import wintypes

_CREATE_SUSPENDED = 0x00000004
_JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION = 0x00000400
_JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
_JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9
_PROCESS_TERMINATE = 0x0001
_PROCESS_SET_QUOTA = 0x0100
_THREAD_SUSPEND_RESUME = 0x0002
_TH32CS_SNAPTHREAD = 0x00000004
_SEM_FAILCRITICALERRORS = 0x0001
_SEM_NOGPFAULTERRORBOX = 0x0002
_SEM_NOOPENFILEERRORBOX = 0x8000
_INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
_RESUME_FAILED = 0xFFFFFFFF

# Exit code of every process the runner terminates (STATUS_CONTROL_C_EXIT).
TERMINATED_EXIT_CODE = 0xC000013A


class _IoCounters(ctypes.Structure):
    _fields_ = [
        ("ReadOperationCount", ctypes.c_ulonglong),
        ("WriteOperationCount", ctypes.c_ulonglong),
        ("OtherOperationCount", ctypes.c_ulonglong),
        ("ReadTransferCount", ctypes.c_ulonglong),
        ("WriteTransferCount", ctypes.c_ulonglong),
        ("OtherTransferCount", ctypes.c_ulonglong),
    ]


class _JobBasicLimits(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", wintypes.LARGE_INTEGER),
        ("PerJobUserTimeLimit", wintypes.LARGE_INTEGER),
        ("LimitFlags", wintypes.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", wintypes.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", wintypes.DWORD),
        ("SchedulingClass", wintypes.DWORD),
    ]


class _JobExtendedLimits(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", _JobBasicLimits),
        ("IoInfo", _IoCounters),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


class _ThreadEntry(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ThreadID", wintypes.DWORD),
        ("th32OwnerProcessID", wintypes.DWORD),
        ("tpBasePri", wintypes.LONG),
        ("tpDeltaPri", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
    ]


class _JobAccounting(ctypes.Structure):
    _fields_ = [("times", ctypes.c_int64 * 4), ("faults", wintypes.DWORD),
                ("total", wintypes.DWORD), ("active", wintypes.DWORD), ("terminated", wintypes.DWORD)]


_kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
_kernel32.CreateJobObjectW.argtypes = [wintypes.LPVOID, wintypes.LPCWSTR]
_kernel32.CreateJobObjectW.restype = wintypes.HANDLE
_kernel32.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]
_kernel32.SetInformationJobObject.restype = wintypes.BOOL
_kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
_kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
_kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
_kernel32.TerminateJobObject.restype = wintypes.BOOL
_kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
_kernel32.OpenProcess.restype = wintypes.HANDLE
_kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
_kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
_kernel32.Thread32First.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ThreadEntry)]
_kernel32.Thread32First.restype = wintypes.BOOL
_kernel32.Thread32Next.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ThreadEntry)]
_kernel32.Thread32Next.restype = wintypes.BOOL
_kernel32.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
_kernel32.OpenThread.restype = wintypes.HANDLE
_kernel32.ResumeThread.argtypes = [wintypes.HANDLE]
_kernel32.ResumeThread.restype = wintypes.DWORD
_kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
_kernel32.CloseHandle.restype = wintypes.BOOL
_kernel32.GetErrorMode.argtypes = []
_kernel32.GetErrorMode.restype = wintypes.UINT
_kernel32.SetErrorMode.argtypes = [wintypes.UINT]
_kernel32.SetErrorMode.restype = wintypes.UINT
_kernel32.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
_kernel32.QueryInformationJobObject.restype = wintypes.BOOL


def _win_error(call: str) -> OSError:
    code = ctypes.get_last_error()
    return ctypes.WinError(code, f"{call}: {ctypes.FormatError(code).strip()}")


def _create_job():
    job = _kernel32.CreateJobObjectW(None, None)
    if not job:
        raise _win_error("CreateJobObjectW")
    limits = _JobExtendedLimits()
    limits.BasicLimitInformation.LimitFlags = (
        _JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | _JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
    )
    if not _kernel32.SetInformationJobObject(
        job, _JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, ctypes.byref(limits), ctypes.sizeof(limits)
    ):
        error = _win_error("SetInformationJobObject")
        _kernel32.CloseHandle(job)
        raise error
    return job


def _assign(job, pid: int) -> None:
    process = _kernel32.OpenProcess(_PROCESS_SET_QUOTA | _PROCESS_TERMINATE, False, pid)
    if not process:
        raise _win_error("OpenProcess")
    try:
        if not _kernel32.AssignProcessToJobObject(job, process):
            raise _win_error("AssignProcessToJobObject")
    finally:
        _kernel32.CloseHandle(process)


def _resume(pid: int) -> None:
    """Resume every thread of the suspended child; it has exactly its primary thread."""
    snapshot = _kernel32.CreateToolhelp32Snapshot(_TH32CS_SNAPTHREAD, 0)
    if snapshot == _INVALID_HANDLE_VALUE:
        raise _win_error("CreateToolhelp32Snapshot")
    try:
        entry = _ThreadEntry()
        entry.dwSize = ctypes.sizeof(entry)
        resumed = 0
        found = _kernel32.Thread32First(snapshot, ctypes.byref(entry))
        while found:
            if entry.th32OwnerProcessID == pid:
                thread = _kernel32.OpenThread(_THREAD_SUSPEND_RESUME, False, entry.th32ThreadID)
                if not thread:
                    raise _win_error("OpenThread")
                try:
                    if _kernel32.ResumeThread(thread) == _RESUME_FAILED:
                        raise _win_error("ResumeThread")
                finally:
                    _kernel32.CloseHandle(thread)
                resumed += 1
            found = _kernel32.Thread32Next(snapshot, ctypes.byref(entry))
        if resumed == 0:
            raise OSError("Thread32Next: no thread of the suspended child was found")
    finally:
        _kernel32.CloseHandle(snapshot)


class JobTree:
    """Owns one child process tree through an anonymous job object."""

    def __init__(self, cleanup_seconds: float):
        self._cleanup_seconds = cleanup_seconds
        self._job = None
        self._proc = None
        self.launch_cleanup_ok = True
        self._cleanup_attempted = False

    def launch(self, argv, cwd, env):
        self._job = _create_job()
        # Children inherit the process error mode; a missing DLL must fail, not block on a dialog.
        _kernel32.SetErrorMode(
            _kernel32.GetErrorMode()
            | _SEM_FAILCRITICALERRORS
            | _SEM_NOGPFAULTERRORBOX
            | _SEM_NOOPENFILEERRORBOX
        )
        proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            bufsize=0,
            creationflags=_CREATE_SUSPENDED,
        )
        self._proc = proc
        try:
            _assign(self._job, proc.pid)
            _resume(proc.pid)
        except BaseException:
            self._cleanup_attempted = True
            deadline = time.monotonic() + self._cleanup_seconds
            try:
                proc.kill()
                proc.wait(timeout=max(0.0, deadline - time.monotonic()))
            except (OSError, subprocess.TimeoutExpired):
                self.launch_cleanup_ok = False
            finally:
                proc.stdout.close()
                proc.stderr.close()
            raise
        return proc

    def await_exit(self, deadline) -> bool:
        try:
            self._proc.wait(timeout=max(0.0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            return False
        return True
    def terminate(self) -> bool:
        return bool(_kernel32.TerminateJobObject(self._job, TERMINATED_EXIT_CODE))

    def await_cleanup(self, deadline) -> bool:
        try:
            self._cleanup_attempted = True
            self._proc.wait(timeout=max(0.0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            return False
        # CPython owns this handle after Popen discards the primary-thread handle.
        # Cache returncode before closing it: Job accounting can retain its process reference.
        self._proc._handle.Close()
        while time.monotonic() < deadline:
            counts = _JobAccounting()
            if not _kernel32.QueryInformationJobObject(self._job, 1, ctypes.byref(counts), ctypes.sizeof(counts), None):
                return False
            if counts.active == 0:
                return True
            time.sleep(min(0.01, max(0.0, deadline - time.monotonic())))
        return False

    @staticmethod
    def is_kill_status(status) -> bool:
        return (status & 0xFFFFFFFF) == TERMINATED_EXIT_CODE

    def close(self):
        if self._job is not None:
            try:
                if self._proc is not None and not self._cleanup_attempted:
                    self.terminate()
                    self.await_cleanup(time.monotonic() + self._cleanup_seconds)
            finally:
                _kernel32.CloseHandle(self._job)
                self._job = None
