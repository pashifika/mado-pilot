"""Observe only the owned PGID; zombies cannot execute and are reaped by their parent."""

from __future__ import annotations

import ctypes
import errno
from pathlib import Path
import sys

MAX_PROCESSES = 4096

if sys.platform == "darwin":
    # SDK sys/proc_info.h: proc_bsdshortinfo, PROC_PIDT_SHORTBSDINFO (13).
    class _BsdShortInfo(ctypes.Structure):
        _fields_ = [("pid", ctypes.c_uint32), ("ppid", ctypes.c_uint32),
                    ("pgid", ctypes.c_uint32), ("status", ctypes.c_uint32),
                    ("comm", ctypes.c_char * 16), ("credentials", ctypes.c_uint32 * 8)]

    _libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    _libproc.proc_listpids.argtypes = [ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_int]
    _libproc.proc_listpids.restype = ctypes.c_int
    _libproc.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
    _libproc.proc_pidinfo.restype = ctypes.c_int

    def no_executing_members(pgid: int) -> bool:
        pids = (ctypes.c_int * MAX_PROCESSES)()
        ctypes.set_errno(0)
        size = _libproc.proc_listpids(2, pgid, pids, ctypes.sizeof(pids))
        if size < 0 or size >= ctypes.sizeof(pids) or size % ctypes.sizeof(ctypes.c_int):
            return False
        if size == 0 and ctypes.get_errno():
            return False
        for pid in pids[:size // ctypes.sizeof(ctypes.c_int)]:
            if pid <= 0:
                continue
            info = _BsdShortInfo()
            ctypes.set_errno(0)
            length = _libproc.proc_pidinfo(pid, 13, 0, ctypes.byref(info), ctypes.sizeof(info))
            if length == 0 and ctypes.get_errno() == errno.ESRCH:
                continue
            if length != ctypes.sizeof(info):
                return False
            if info.pgid == pgid and info.status != 5:  # SZOMB
                return False
        return True

elif sys.platform.startswith("linux"):
    def no_executing_members(pgid: int) -> bool:
        # Linux is the procedure-test host only. Names are never retained or reported.
        count = 0
        try:
            for entry in Path("/proc").iterdir():
                if not entry.name.isdecimal():
                    continue
                count += 1
                if count > MAX_PROCESSES:
                    return False
                try:
                    with (entry / "stat").open("rb") as stream:
                        data = stream.read(4096)
                except FileNotFoundError:
                    continue
                end = data.rfind(b") ")
                fields = data[end + 2:].split()
                if end < 0 or len(fields) < 3 or len(data) == 4096:
                    return False
                if int(fields[2]) == pgid and fields[0] not in (b"Z", b"X"):
                    return False
            return True
        except (OSError, ValueError):
            return False
else:
    def no_executing_members(pgid: int) -> bool:
        return False
