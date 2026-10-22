"""Owned Windows x64 loader-event discovery, never a normal timing observer.

``observe`` launches an isolated debugger through the existing process runner.
Its outer Job bounds even a blocked filesystem/hash call. The debugger owns a
nested nonbreakaway Job before resuming the target and observes its descendants.
Only loader-backed PE images are covered: not manual mappings, datafile mappings,
relocated memory integrity, or loads in a subsequent unobserved measurement.

All images, including system DLLs, receive file identities and pre/post hashes.
Discovery does not admit an image against a policy: the caller must reconcile the
returned closure before qualification. Missing identities fail closed. Image file
handles deny writes/deletion until unload/exit; this and debug stops perturb the
workload. Only the initial system-loader breakpoint is consumed; application
exceptions are passed through. Debug strings and debuggee memory are not read.

Win32 contracts:
https://learn.microsoft.com/en-us/windows/win32/debug/debugging-events
https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-waitfordebugevent
"""

from __future__ import annotations

import base64
import ctypes
from ctypes import wintypes as W
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import time

# The existing runner imports its adjacent platform ownership helper by name.
_TOOLS = str(Path(__file__).resolve().parent)
if _TOOLS not in sys.path:
    sys.path.insert(0, _TOOLS)

MAX_OUTPUT = 8 << 20
MAX_TRANSPORT = 64 << 20
MAX_FILE = 1 << 30
MAX_HASH_BYTES = 8 << 30
MAX_PATH = 4096
MAX_PROCESSES = 256
CHUNK = 65536
CONTINUE = 0x00010002
NOT_HANDLED = 0x80010001
INVALID_HANDLE = ctypes.c_void_p(-1).value
EVENT_NAMES = {1: "exception", 2: "create_thread", 3: "create_process",
               4: "exit_thread", 5: "exit_process", 6: "load_dll",
               7: "unload_dll", 8: "debug_string_ignored", 9: "rip"}


def _sibling(name):
    spec = importlib.util.spec_from_file_location(
        "_image_preflight_" + name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _result(reason=None):
    return {"format_version": 1, "kind": "windows-runtime-image-preflight",
            "target": "x86_64-pc-windows-msvc", "method": "win32-owned-debug-events-v1",
            "observer_effects": ["debug stops at every loader/process/thread event",
                                 "file hashing outside normal timing",
                                 "image files held without write/delete sharing until release",
                                 "Windows may create an owned console-host descendant"],
            "coverage": "owned Win32 loader events in this preflight only",
            "measurement_time_identity": "not-observed",
            "reason": reason, "record": {"exit_code": None, "timed_out": False,
                "output_limited": False, "cleanup_ok": True, "stdout": "", "stderr": "",
                "duration_seconds": 0.0, "launch_error": reason},
            "processes": [], "images": [], "events": [],
            "provenance_complete": False, "cleanup_ok": True}


def observe(argv, *, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds,
            image_limit=4096, event_limit=16384):
    """Discover actual loader-backed images using child-only ``env``.

    Return a refusal receipt without launching for invalid/unsupported inputs.
    Primary observation is bounded by timeout_seconds, inner cleanup by
    cleanup_seconds. The outer watchdog allows their sum, then at most another
    cleanup_seconds to stop its owned debugger/tree. Temporary protocol files
    contain only command configuration, never captured application content.
    A truncated worker receipt preserves the already emitted event/output prefix
    and is incomplete, even when outer Job cleanup succeeds.
    Root exit permits at most cleanup_seconds of natural descendant drain,
    still within the primary deadline; it does not imply immediate tree exit.
    Use from a serialized coordinator: the reused runner temporarily sets the
    caller's error mode for inheritance, restored before this call returns.
    """
    result = _result()
    runner = _sibling("process_runner")
    try:
        runner._validate(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds)
        if (timeout_seconds > 3600 or cleanup_seconds > 60 or output_limit_bytes > MAX_OUTPUT
                or type(image_limit) is not int or not 1 <= image_limit <= 4096
                or type(event_limit) is not int or not 1 <= event_limit <= 16384
                or len(argv) > 256 or len(env) > 512
                or len({key.upper() for key in env}) != len(env)
                or "\0" in os.fspath(cwd) or len(os.fspath(cwd)) > MAX_PATH
                or len(subprocess.list2cmdline(argv).encode("utf-16-le")) >= 65534
                or sum(len((key + "=" + value).encode("utf-16-le")) + 2
                       for key, value in env.items()) + 2 > 65534):
            raise ValueError("preflight input exceeds fixed bounds")
    except (ValueError, TypeError, UnicodeError) as exc:
        return _result("invalid_arguments: " + str(exc))
    if sys.platform != "win32" or ctypes.sizeof(ctypes.c_void_p) != 8:
        return _result("requires_windows_x64_python")
    plan = dict(argv=argv, cwd=os.fspath(cwd), env=env, timeout_seconds=timeout_seconds,
                output_limit_bytes=output_limit_bytes, cleanup_seconds=cleanup_seconds,
                image_limit=image_limit, event_limit=event_limit)
    started = time.monotonic()
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetErrorMode.argtypes, kernel.GetErrorMode.restype = [], W.UINT
    kernel.SetErrorMode.argtypes, kernel.SetErrorMode.restype = [W.UINT], W.UINT
    error_mode = kernel.GetErrorMode()
    outer_started = False
    try:
        with tempfile.TemporaryDirectory(prefix="mado-image-preflight-") as directory:
            plan_path = Path(directory) / "plan.json"
            plan_path.write_text(json.dumps(plan, ensure_ascii=True), encoding="utf-8")
            # -I isolates Python startup, not the target's explicitly supplied environment.
            outer_started = True
            outer = runner.run_process(
                [str(Path(sys.executable).resolve()), "-I", "-B", str(Path(__file__).resolve()),
                 "--worker", str(plan_path)], cwd=Path(directory), env=dict(os.environ),
                timeout_seconds=timeout_seconds + cleanup_seconds,
                output_limit_bytes=MAX_TRANSPORT, cleanup_seconds=cleanup_seconds)
            result = _decode(outer, output_limit_bytes, image_limit, event_limit)
    except OSError as exc:
        # A temporary-directory cleanup error must not erase a completed target
        # receipt or turn uncertain runner cleanup into a successful empty run.
        result["reason"] = result["reason"] or "observer_io_failed"
        result["observer_error"] = str(exc)
        result["provenance_complete"] = False
        result["cleanup_ok"] = not outer_started
        result["record"]["cleanup_ok"] = not outer_started
        if not outer_started:
            result["record"]["launch_error"] = result["reason"]
    finally:
        # The reused runner sets its parent's process error mode for inheritance.
        # Keep that existing behavior confined to this serialized preflight call.
        kernel.SetErrorMode(error_mode)
    result["record"]["duration_seconds"] = time.monotonic() - started
    return result


def _decode(outer, output_limit, image_limit, event_limit):
    result = _result("observer_receipt_incomplete")
    streams = [bytearray(), bytearray()]
    complete = False
    received_done = False
    try:
        for line in outer["stdout"].splitlines():
            value = json.loads(line)
            kind = value.pop("type")
            if complete:
                raise ValueError("data after final receipt")
            if kind == "output":
                index = value["stream"]
                if type(index) is not int or index not in (0, 1):
                    raise ValueError("invalid output stream")
                block = base64.b64decode(value["bytes"], validate=True)
                if sum(map(len, streams)) + len(block) > output_limit:
                    raise ValueError("child output exceeds bound")
                streams[index].extend(block)
            elif kind in ("image", "process", "event"):
                key = {"image": "images", "process": "processes", "event": "events"}[kind]
                limit = {"image": image_limit, "process": MAX_PROCESSES,
                         "event": event_limit}[kind]
                if len(result[key]) >= limit:
                    raise ValueError("receipt item limit")
                result[key].append(value)
            elif kind == "release":
                image = result["images"][value.pop("index")]
                if image["release_event"] is not None:
                    raise ValueError("duplicate image release")
                image.update(value)
            elif kind == "exit":
                result["processes"][value["index"]].update(
                    exit_event=value["event"], exit_code=value["exit_code"])
            elif kind == "done":
                result.update(value)
                complete = True
                received_done = True
            else:
                raise ValueError("unknown observer record")
    except (ValueError, TypeError, KeyError, IndexError):
        complete = False
    outer_ok = (outer["exit_code"] == 0 and not outer["timed_out"]
                and not outer["output_limited"] and outer["cleanup_ok"])
    if not complete or not outer_ok:
        result["reason"] = "observer_receipt_incomplete"
        result["provenance_complete"] = False
        if not received_done:
            result["record"] = {**outer, "stdout": "", "stderr": ""}
            # Without a final target receipt, containment cannot invent its exit.
            result["record"]["exit_code"] = None
            result["cleanup_ok"] = outer["cleanup_ok"]
        else:
            result["cleanup_ok"] = result["cleanup_ok"] and outer["cleanup_ok"]
    else:
        result["cleanup_ok"] = result["cleanup_ok"] and outer["cleanup_ok"]
    result["record"]["stdout"] = streams[0].decode("utf-8", errors="replace")
    result["record"]["stderr"] = streams[1].decode("utf-8", errors="replace")
    result["record"]["cleanup_ok"] = result["cleanup_ok"]
    result["observer_record"] = {key: value for key, value in outer.items() if key != "stdout"}
    return result


class _Refusal(Exception):
    pass


def _require(ok, reason):
    if not ok:
        raise _Refusal(reason)


class _ExceptionRecord(ctypes.Structure):
    _fields_ = [("code", W.DWORD), ("flags", W.DWORD), ("record", W.LPVOID),
                ("address", W.LPVOID), ("count", W.DWORD), ("parameters", ctypes.c_size_t * 15)]


class _ExceptionInfo(ctypes.Structure):
    _fields_ = [("record", _ExceptionRecord), ("first_chance", W.DWORD)]


class _CreateThread(ctypes.Structure):
    _fields_ = [("thread", W.HANDLE), ("local", W.LPVOID), ("start", W.LPVOID)]


class _CreateProcess(ctypes.Structure):
    _fields_ = [("file", W.HANDLE), ("process", W.HANDLE), ("thread", W.HANDLE),
                ("base", W.LPVOID), ("debug_offset", W.DWORD), ("debug_size", W.DWORD),
                ("local", W.LPVOID), ("start", W.LPVOID), ("image_name", W.LPVOID),
                ("unicode", W.WORD)]


class _LoadDll(ctypes.Structure):
    _fields_ = [("file", W.HANDLE), ("base", W.LPVOID), ("debug_offset", W.DWORD),
                ("debug_size", W.DWORD), ("image_name", W.LPVOID), ("unicode", W.WORD)]


class _Exit(ctypes.Structure):
    _fields_ = [("code", W.DWORD)]


class _Unload(ctypes.Structure):
    _fields_ = [("base", W.LPVOID)]


class _DebugUnion(ctypes.Union):
    _fields_ = [("exception", _ExceptionInfo), ("create_thread", _CreateThread),
                ("create_process", _CreateProcess), ("exit", _Exit),
                ("load", _LoadDll), ("unload", _Unload)]


class _DebugEvent(ctypes.Structure):
    _fields_ = [("code", W.DWORD), ("pid", W.DWORD), ("tid", W.DWORD), ("u", _DebugUnion)]


class _FileInfo(ctypes.Structure):
    _fields_ = [("attributes", W.DWORD), ("creation", W.FILETIME), ("access", W.FILETIME),
                ("write", W.FILETIME), ("volume", W.DWORD), ("size_high", W.DWORD),
                ("size_low", W.DWORD), ("links", W.DWORD),
                ("id_high", W.DWORD), ("id_low", W.DWORD)]


class _Api:
    def __init__(self):
        _require(ctypes.sizeof(_DebugEvent) == 176 and _DebugEvent.u.offset == 16,
                 "unsupported_debug_event_layout")
        self.job = _sibling("_windows_process")
        self.k = ctypes.WinDLL("kernel32", use_last_error=True)
        p, d, h = W.LPVOID, W.DWORD, W.HANDLE
        signatures = {
            "CloseHandle": (W.BOOL, [h]),
            "CreateFileW": (h, [W.LPCWSTR, d, d, p, d, d, h]),
            "ReOpenFile": (h, [h, d, d, d]),
            "GetFileType": (d, [h]),
            "GetFileInformationByHandle": (W.BOOL, [h, ctypes.POINTER(_FileInfo)]),
            "GetFinalPathNameByHandleW": (d, [h, W.LPWSTR, d, d]),
            "ReadFile": (W.BOOL, [h, p, d, ctypes.POINTER(d), p]),
            "SetFilePointerEx": (W.BOOL, [h, ctypes.c_int64, p, d]),
            "PeekNamedPipe": (W.BOOL, [h, p, d, p, ctypes.POINTER(d), p]),
            "WaitForDebugEvent": (W.BOOL, [ctypes.POINTER(_DebugEvent), d]),
            "ContinueDebugEvent": (W.BOOL, [d, d, d]),
            "IsProcessInJob": (W.BOOL, [h, h, ctypes.POINTER(W.BOOL)]),
            "QueryInformationJobObject": (W.BOOL, [h, ctypes.c_int, p, d, p]),
            "TerminateJobObject": (W.BOOL, [h, W.UINT]),
            "GetSystemDirectoryW": (W.UINT, [W.LPWSTR, W.UINT]),
        }
        for name, (result, args) in signatures.items():
            function = getattr(self.k, name)
            function.restype, function.argtypes = result, args

    def close(self, handle):
        if handle not in (None, 0, INVALID_HANDLE):
            _require(self.k.CloseHandle(handle), "handle_close_failed")

    def path(self, handle):
        buffer = ctypes.create_unicode_buffer(MAX_PATH + 1)
        size = self.k.GetFinalPathNameByHandleW(handle, buffer, len(buffer), 0)
        _require(0 < size < len(buffer), "image_path_unavailable_or_too_long")
        path = buffer.value
        _require(path.startswith("\\\\?\\") and not path.startswith("\\\\?\\UNC\\"),
                 "nonlocal_image_path_unsupported")
        return path[4:]

    def identity(self, handle):
        info = _FileInfo()
        _require(self.k.GetFileType(handle) == 1, "image_not_disk_file")
        _require(self.k.GetFileInformationByHandle(handle, ctypes.byref(info)),
                 "image_identity_unavailable")
        _require(not info.attributes & (0x10 | 0x400), "image_not_regular_file")
        return {"volume": info.volume, "file_id": (info.id_high << 32) | info.id_low,
                "bytes": (info.size_high << 32) | info.size_low,
                "write_time": (info.write.dwHighDateTime << 32) | info.write.dwLowDateTime}


class _Pin:
    def __init__(self, owner, handle):
        self.owner, self.handle = owner, handle
        self.path = owner.api.path(handle)
        self.identity = owner.api.identity(handle)
        _require(0 < self.identity["bytes"] <= MAX_FILE, "image_file_limit")
        self.sha256 = self.hash()
        self.ranges = []
        header = self.read(0, 64)
        _require(header[:2] == b"MZ", "image_not_pe")
        pe = struct.unpack_from("<I", header, 60)[0]
        _require(pe <= 1 << 20, "pe_header_limit")
        header = self.read(pe, 24)
        _require(header[:4] == b"PE\0\0" and struct.unpack_from("<H", header, 4)[0] == 0x8664,
                 "image_not_x64_pe")
        if os.path.normcase(self.path) == owner.ntdll:
            count, optional = struct.unpack_from("<H", header, 6)[0], struct.unpack_from("<H", header, 20)[0]
            _require(1 <= count <= 96 and 64 <= optional <= 4096, "loader_pe_bounds")
            table = self.read(pe + 24 + optional, count * 40)
            for offset in range(0, len(table), 40):
                size, address = struct.unpack_from("<II", table, offset + 8)
                if struct.unpack_from("<I", table, offset + 36)[0] & 0x20000000:
                    self.ranges.append((address, address + size))

    def read(self, offset, size):
        self.owner.budget()
        _require(0 <= offset and 0 <= size <= CHUNK and offset + size <= self.identity["bytes"],
                 "image_read_bounds")
        self.owner.hash_bytes += size
        _require(self.owner.hash_bytes <= MAX_HASH_BYTES, "image_hash_byte_limit")
        k = self.owner.api.k
        _require(k.SetFilePointerEx(self.handle, offset, None, 0), "image_seek_failed")
        buffer, done = ctypes.create_string_buffer(size), W.DWORD()
        _require(k.ReadFile(self.handle, buffer, size, ctypes.byref(done), None)
                 and done.value == size, "image_read_failed")
        return buffer.raw

    def hash(self):
        digest = hashlib.sha256()
        for offset in range(0, self.identity["bytes"], CHUNK):
            digest.update(self.read(offset, min(CHUNK, self.identity["bytes"] - offset)))
        _require(self.owner.api.identity(self.handle) == self.identity
                 and self.owner.api.path(self.handle) == self.path, "image_identity_changed")
        return digest.hexdigest()


class _Observer:
    def __init__(self, plan):
        self.plan, self.api = plan, _Api()
        self.started = time.monotonic()
        self.deadline = self.started + plan["timeout_seconds"]
        self.cleanup_deadline = None
        self.root_drain_deadline = None
        self.job = self.proc = self.root_pin = None
        self.job_accounting = None
        self.live, self.pins = {}, {}
        self.process_count = self.image_count = self.events = self.continued = 0
        self.hash_bytes = self.output_bytes = 0
        self.eof = [False, False]
        self.reason = None
        self.root_exit = None
        self.root_reaped = False
        self.assigned = self.resumed = self.killed = False
        self.root_forced_cleanup = False
        self.cleanup_ok = True
        self.output_limited = self.timed_out = False
        buffer = ctypes.create_unicode_buffer(MAX_PATH)
        size = self.api.k.GetSystemDirectoryW(buffer, len(buffer))
        _require(0 < size < len(buffer), "system_directory_unavailable")
        self.ntdll = os.path.normcase(os.path.join(buffer.value, "ntdll.dll"))

    @staticmethod
    def emit(record_type, **fields):
        print(json.dumps({"type": record_type, **fields}, ensure_ascii=True, separators=(",", ":")), flush=True)

    def budget(self):
        now = time.monotonic()
        _require(now < (self.cleanup_deadline or self.deadline), "deadline_exceeded")
        _require(self.cleanup_deadline is not None or self.root_drain_deadline is None
                 or now < self.root_drain_deadline, "root_cleanup_deadline_exceeded")
        self.pump()
        _require(not self.output_limited, "output_limit_exceeded")

    def pump(self):
        if self.proc is None:
            return
        import msvcrt
        for index, stream in enumerate((self.proc.stdout, self.proc.stderr)):
            if self.eof[index]:
                continue
            handle = msvcrt.get_osfhandle(stream.fileno())
            available = W.DWORD()
            if not self.api.k.PeekNamedPipe(handle, None, 0, None, ctypes.byref(available), None):
                _require(ctypes.get_last_error() == 109, "output_pipe_query_failed")
                self.eof[index] = True
                continue
            if not available.value:
                continue
            block = os.read(stream.fileno(), min(CHUNK, available.value))
            _require(bool(block), "output_pipe_short_read")
            remaining = self.plan["output_limit_bytes"] - self.output_bytes
            kept = block[:remaining]
            if kept:
                self.output_bytes += len(kept)
                self.emit("output", stream=index, bytes=base64.b64encode(kept).decode("ascii"))
            if len(block) > remaining:
                self.output_limited = True

    def fail(self, reason):
        if self.reason is None:
            self.reason = reason
        self.timed_out |= reason == "deadline_exceeded"
        if self.cleanup_deadline is None:
            self.cleanup_deadline = time.monotonic() + self.plan["cleanup_seconds"]
        if self.proc is not None and not self.killed:
            if self.assigned:
                self.killed = bool(self.api.k.TerminateJobObject(self.job, 0xC000013A))
            else:
                self.proc.kill()
                self.killed = True
            self.cleanup_ok &= self.killed
            self.root_forced_cleanup |= self.killed and self.root_exit is None

    def launch(self):
        handle = self.api.k.CreateFileW(self.plan["argv"][0], 0x80000000, 1, None, 3, 0x08000000, None)
        _require(handle not in (0, INVALID_HANDLE), "root_image_open_failed")
        try:
            self.root_pin = _Pin(self, handle)
        except BaseException:
            self.api.close(handle)
            raise
        self.job = self.api.job._create_job()
        self.proc = subprocess.Popen(self.plan["argv"], cwd=self.plan["cwd"], env=self.plan["env"],
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     bufsize=0, creationflags=0x1 | 0x4 | 0x08000000)
        self.api.job._assign(self.job, self.proc.pid)
        self.assigned = True
        self.api.job._resume(self.proc.pid)
        self.resumed = True

    def bind(self, file, process, base, kind):
        _require(file not in (None, 0, INVALID_HANDLE), "missing_image_file_handle")
        _require(self.image_count < self.plan["image_limit"], "image_limit_exceeded")
        _require(base and base not in process["images"], "duplicate_image_base")
        if kind == "executable" and process["root"]:
            pin = self.root_pin
            _require(self.api.identity(file) == pin.identity and self.api.path(file) == pin.path,
                     "root_image_identity_mismatch")
            self.root_pin = None
        else:
            handle = self.api.k.ReOpenFile(file, 0x80000000, 1, 0x08000000)
            _require(handle not in (0, INVALID_HANDLE), "image_reopen_failed")
            try:
                pin = _Pin(self, handle)
                _require(self.api.identity(file) == pin.identity and self.api.path(file) == pin.path,
                         "image_event_identity_mismatch")
            except BaseException:
                self.api.close(handle)
                raise
        index = self.image_count
        self.image_count += 1
        self.pins[index] = pin
        process["images"][base] = index
        self.emit("image", index=index, process_index=process["index"], kind=kind,
                  path=pin.path, architecture="x64", file_identity=pin.identity,
                  sha256=pin.sha256, post_sha256=None, load_event=self.events,
                  release_event=None, release_kind=None, identity_verified_at_release=False)

    def release(self, index, why):
        pin = self.pins[index]
        try:
            post = pin.hash()
            _require(post == pin.sha256, "image_digest_changed")
            self.emit("release", index=index, post_sha256=post, release_event=self.events,
                      release_kind=why, identity_verified_at_release=True)
        finally:
            self.pins.pop(index)
            self.api.close(pin.handle)

    def event(self, event):
        code, pid, tid = event.code, event.pid, event.tid
        _require(code in EVENT_NAMES, "unknown_debug_event")
        _require(self.events <= self.plan["event_limit"], "event_limit_exceeded")
        self.emit("event", sequence=self.events, kind=EVENT_NAMES[code], pid=pid, tid=tid)
        if code == 3:
            _require(pid not in self.live and self.process_count < MAX_PROCESSES,
                     "process_limit_or_duplicate")
            owned = W.BOOL()
            _require(self.assigned and self.api.k.IsProcessInJob(
                event.u.create_process.process, self.job, ctypes.byref(owned)) and owned.value,
                "process_outside_owned_job")
            root = pid == self.proc.pid and self.process_count == 0
            _require(self.process_count > 0 or root, "first_process_not_root")
            process = {"index": self.process_count, "root": root, "images": {},
                       "threads": {tid}, "initial_tid": tid, "exception_seen": False}
            self.process_count += 1
            self.live[pid] = process
            self.emit("process", index=process["index"], pid=pid, is_root=root,
                      create_event=self.events, exit_event=None, exit_code=None, owned_job=True)
        else:
            _require(pid in self.live, "event_for_unknown_process")
            process = self.live[pid]
        status = CONTINUE
        if code == 3:
            self.bind(event.u.create_process.file, process, event.u.create_process.base, "executable")
        elif code == 2:
            _require(tid not in process["threads"], "duplicate_thread")
            process["threads"].add(tid)
        else:
            _require(tid in process["threads"], "event_for_unknown_thread")
            if code == 4:
                process["threads"].remove(tid)
            elif code == 5:
                for index in list(process["images"].values()):
                    self.release(index, "process_exit")
                self.emit("exit", index=process["index"], event=self.events, exit_code=event.u.exit.code)
                del self.live[pid]
            elif code == 6:
                self.bind(event.u.load.file, process, event.u.load.base, "dll")
            elif code == 7:
                _require(event.u.unload.base in process["images"], "unload_for_unknown_image")
                self.release(process["images"].pop(event.u.unload.base), "unload")
            elif code == 1:
                info = event.u.exception
                initial = not process["exception_seen"] and tid == process["initial_tid"]
                process["exception_seen"] = True
                loader = (initial and info.first_chance and info.record.code == 0x80000003
                          and any(os.path.normcase(self.pins[index].path) == self.ntdll
                                  and any(lo <= (info.record.address or 0) - base < hi
                                          for lo, hi in self.pins[index].ranges)
                                  for base, index in process["images"].items()))
                status = CONTINUE if loader else NOT_HANDLED
            elif code == 9:
                raise _Refusal("debug_rip_event")
        return status

    def counts(self):
        counts = self.api.job._JobAccounting()
        _require(self.api.k.QueryInformationJobObject(self.job, 1, ctypes.byref(counts),
                                                      ctypes.sizeof(counts), None), "job_query_failed")
        return counts

    def reap(self):
        if self.root_exit is not None and not self.root_reaped and self.proc.poll() is not None:
            # Do not retain the root process object while asking Job active accounting.
            self.proc._handle.Close()
            self.root_reaped = True

    def loop(self):
        while True:
            try:
                self.pump()
                if self.output_limited:
                    self.fail("output_limit_exceeded")
                if self.cleanup_deadline is None and time.monotonic() >= self.deadline:
                    self.fail("deadline_exceeded")
                if self.cleanup_deadline is not None and time.monotonic() >= self.cleanup_deadline:
                    self.cleanup_ok = False
                    break
                self.reap()
                if (self.reason is None and self.root_drain_deadline is not None
                        and time.monotonic() >= self.root_drain_deadline):
                    self.fail("descendants_survived_root" if self.live else "root_cleanup_deadline_exceeded")
                if self.root_reaped and not self.live and all(self.eof):
                    # ContinueDebugEvent releases a terminated debuggee, but Job
                    # accounting need not have settled at that exact instant.
                    if self.counts().active == 0:
                        break
                event = _DebugEvent()
                if not self.api.k.WaitForDebugEvent(ctypes.byref(event), 10):
                    _require(ctypes.get_last_error() in (121, 258), "debug_wait_failed")
                    continue
                self.events += 1
                if event.code == 5 and event.pid == self.proc.pid:
                    self.root_exit = event.u.exit.code
                    # Start before release hashing, not after the final image.
                    if self.root_drain_deadline is None:
                        self.root_drain_deadline = time.monotonic() + self.plan["cleanup_seconds"]
                status = NOT_HANDLED if event.code == 1 else CONTINUE
                try:
                    if self.reason is None:
                        status = self.event(event)
                    elif event.code == 5:
                        self.live.pop(event.pid, None)
                except (OSError, _Refusal) as exc:
                    self.fail(str(exc))
                    if event.code == 5:
                        self.live.pop(event.pid, None)
                finally:
                    handle = (event.u.create_process.file if event.code == 3
                              else event.u.load.file if event.code == 6 else None)
                    self.api.close(handle)
                    _require(self.api.k.ContinueDebugEvent(event.pid, event.tid, status),
                             "debug_continue_failed")
                    self.continued += 1
                _require(self.events <= self.plan["event_limit"] + 16384, "cleanup_event_limit")
            except (OSError, _Refusal) as exc:
                self.fail(str(exc))
                # A broken debugger API cannot be retried into a passing receipt.
                self.cleanup_ok = False
                break

    def run(self):
        try:
            self.launch()
            self.loop()
        except (OSError, _Refusal) as exc:
            self.fail(str(exc))
            if self.proc is not None:
                self.loop()
        finally:
            if self.proc is not None:
                self.reap()
                self.cleanup_ok &= self.root_reaped and not self.live and all(self.eof)
            if self.job is not None:
                try:
                    counts = self.counts()
                    self.cleanup_ok &= counts.active == 0
                    self.job_accounting = {"total": counts.total, "active": counts.active,
                                           "terminated": counts.terminated}
                    if counts.active and self.reason is None:
                        self.reason = "owned_job_not_empty"
                    if counts.total != self.process_count and self.reason is None:
                        self.reason = "job_debug_process_count_mismatch"
                except _Refusal as exc:
                    self.reason = self.reason or str(exc)
                    self.cleanup_ok = False
            if self.pins and self.reason is None:
                self.reason = "image_lifetimes_incomplete"
            for pin in list(self.pins.values()) + ([self.root_pin] if self.root_pin else []):
                try:
                    self.api.close(pin.handle)
                except _Refusal:
                    self.cleanup_ok = False
            if self.job is not None:
                self.api.close(self.job)  # kill-on-close is a final safety net, not proof of cleanup.
            if self.proc is not None:
                for stream in (self.proc.stdout, self.proc.stderr):
                    stream.close()
                if not self.root_reaped:
                    self.proc._handle.Close()
        complete = (self.reason is None and self.cleanup_ok and self.root_reaped
                    and self.events == self.continued and not self.pins)
        record = {"exit_code": self.root_exit if self.root_reaped and not self.root_forced_cleanup else None,
                  "timed_out": self.timed_out, "output_limited": self.output_limited,
                  "cleanup_ok": self.cleanup_ok, "stdout": "", "stderr": "",
                  "duration_seconds": time.monotonic() - self.started,
                  "launch_error": self.reason if not self.resumed else None}
        self.emit("done", reason=self.reason, record=record, provenance_complete=complete,
                  cleanup_ok=self.cleanup_ok, events_observed=self.events,
                  events_continued=self.continued, hash_read_bytes=self.hash_bytes,
                  forced_owned_cleanup=self.killed, job_accounting=self.job_accounting)


def _worker(path):
    plan = json.loads(Path(path).read_text(encoding="utf-8"))
    _Observer(plan).run()


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] != "--worker":
        raise SystemExit("Internal preflight worker; use observe()")
    _worker(sys.argv[2])
