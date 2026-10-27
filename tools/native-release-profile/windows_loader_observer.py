"""Component-only Windows loader correlation under the existing owned runner.

This is not atomic loader history, backing-object/code-byte proof, KnownDLL
membership, or observer-off equivalence. Pre-registration effects can persist;
notifications can be delayed, censuses are non-atomic, and process-static callback
storage has no unregister/final-drain or serialization-to-exit coverage. Discovery
hashes are post-exit candidates only. Measurement pins precede the child.
The cumulative 129 bridges permit at most 43 steady modules. Binding deadlines
are checked between bounded local synchronous calls, not OS-call preemption.
"""

from __future__ import annotations

import ctypes
import hashlib
import json
import math
import ntpath
import os
from pathlib import Path
import re
import struct
import sys
import time

_TOOLS = str(Path(__file__).resolve().parent)
if _TOOLS not in sys.path:
    sys.path.insert(0, _TOOLS)
import process_runner
import windows_loader_callback

METHOD = "windows-ldr-observer-on-warm-v1"
TIMING_PROTOCOL = "mado-component-timing-v1"
PROTOCOL = "mado-component-windows-loader-v1"
TARGET = "x86_64-pc-windows-msvc"
LANE = "single-process-component-benchmark"
SOURCE = {"protocol": PROTOCOL, "correlation": "mapped-name-non-address-PE-open-file",
          "file_pin": "FileIdInfo128-finalNT-SHA256-share-read-only-v1"}
LIMITATIONS = ["pre-registration history absent; prior effects may persist",
              "delayed callbacks and non-atomic census", "no final callback drain",
              "serialization-to-exit unobserved", "no backing-object or code-byte proof",
              "no KnownDLL membership proof", "observer-on costs not subtracted"]
MAX_OUTPUT = 8 << 20
MAX_FILE = 512 << 20
MAX_HASH_BYTES = 8 << 30
MAX_IMAGES = 128
MAX_BRIDGES = 129
MAX_EVENTS = 256
MAX_PATH = 1024
MAX_HANDLES = 4096
CHUNK = 65536
_HEX = re.compile(r"[0-9a-f]{64}\Z")
_ROW = re.compile(r"[A-Za-z0-9-]{1,128}\Z")
_MARKER = re.compile(r"mado-component-timing-v1\[([0-9]{1,10})\]: ([A-Za-z0-9-]{1,128}) (row-start|samples-begin|samples-end)\Z")


class _Refusal(ValueError):
    pass


def _require(value, reason):
    if not value:
        raise _Refusal(reason)


def canonical_sha256(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                     ensure_ascii=True, allow_nan=False).encode("ascii")).hexdigest()


def method_identity():
    return {"module": Path(__file__).name, "method": METHOD, "source": SOURCE,
            "sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "callback_sha256": hashlib.sha256(Path(windows_loader_callback.__file__).read_bytes()).hexdigest()}


def _keys(value, keys, reason):
    _require(isinstance(value, dict) and set(value) == set(keys.split()), reason)
    return value


def _uint(value, bits=64, minimum=0):
    _require(type(value) is int and minimum <= value < 1 << bits, "integer_out_of_bounds")
    return value


def _finite(value, maximum, minimum=0):
    _require(type(value) in (int, float) and math.isfinite(value)
             and minimum <= value <= maximum, "duration_out_of_bounds")
    return value


def _wide(text):
    _require(isinstance(text, str) and 0 < len(text) < MAX_PATH * 4
             and len(text) % 4 == 0 and re.fullmatch(r"[0-9a-f]+", text), "utf16_path_invalid")
    try:
        result = b"".join(int(text[i:i + 4], 16).to_bytes(2, "little")
                          for i in range(0, len(text), 4)).decode("utf-16-le", "strict")
    except UnicodeError:
        raise _Refusal("utf16_path_invalid") from None
    _require(not any(ord(c) < 32 for c in result), "path_control_character")
    return result


def _hexwide(text):
    data = text.encode("utf-16-le", "strict")
    return "".join(f"{int.from_bytes(data[i:i + 2], 'little'):04x}" for i in range(0, len(data), 2))


def _path(path, *, native=False):
    _require(isinstance(path, str) and 0 < len(path.encode("utf-16-le")) < MAX_PATH * 2
             and not any(ord(c) < 32 for c in path) and "/" not in path, "path_invalid")
    if native:
        match = re.fullmatch(r"\\Device\\HarddiskVolume[0-9]+\\(.+)", path)
    else:
        match = re.fullmatch(r"[A-Za-z]:\\(.+)", path)
    _require(match is not None, "nonlocal_or_unbounded_path")
    parts = match[1].split("\\")
    _require(all(part and part not in (".", "..") and not part.endswith((".", " "))
                 and not any(c in part for c in ':*?"<>|') for part in parts), "path_component_invalid")
    return path


def loader_environment(env):
    """Retain the complete child environment; Windows names are case-insensitive."""
    _require(isinstance(env, dict) and len(env) <= 512, "loader_environment_invalid")
    result = {}
    for name, value in env.items():
        _require(isinstance(name, str) and name and len(name) <= 4096
                 and "=" not in name and "\0" not in name
                 and isinstance(value, str) and len(value) <= 32767 and "\0" not in value,
                 "loader_environment_invalid")
        key = name.upper()
        _require(key not in result, "loader_environment_duplicate")
        result[key] = value
    _require(sum(len(k) + len(v) + 2 for k, v in result.items()) <= 32767,
             "loader_environment_limit")
    return dict(sorted(result.items()))


_FACT_KEYS = "volume_serial file_id_hex final_nt_path_utf16_hex creation_time last_access_time last_write_time change_time attributes allocation_size file_size links delete_pending directory"


def _facts(value):
    _keys(value, _FACT_KEYS, "file_facts_invalid")
    _uint(value["volume_serial"])
    _require(isinstance(value["file_id_hex"], str)
             and re.fullmatch(r"[0-9a-f]{32}", value["file_id_hex"]), "file_id_invalid")
    _path(_wide(value["final_nt_path_utf16_hex"]), native=True)
    for key in ("creation_time", "last_access_time", "last_write_time", "change_time"):
        _require(type(value[key]) is int and -(1 << 63) <= value[key] < 1 << 63, "file_time_invalid")
    for key in ("attributes", "links", "delete_pending", "directory"):
        _uint(value[key], 32)
    _require(not value["attributes"] & 0x410 and value["directory"] == 0
             and value["delete_pending"] == 0 and value["links"] > 0, "file_not_regular")
    _require(type(value["file_size"]) is int and 0 < value["file_size"] <= MAX_FILE
             and type(value["allocation_size"]) is int
             and 0 <= value["allocation_size"] < 1 << 63, "file_extent_invalid")
    return value


def _stable_facts(value):
    return {k: v for k, v in _facts(value).items() if k != "last_access_time"}


_PE_KEYS = "machine timestamp characteristics magic entry_rva preferred_base image_size headers_size checksum subsystem dll_characteristics sections debug_entries codeview_status codeview"


def _pe(value, file_size):
    _keys(value, _PE_KEYS, "pe_fields_invalid")
    for key in ("machine", "characteristics", "magic", "subsystem", "dll_characteristics"):
        _uint(value[key], 16)
    for key in ("timestamp", "entry_rva", "image_size", "headers_size", "checksum"):
        _uint(value[key], 32)
    _uint(value["preferred_base"])
    _require(value["machine"] == 0x8664 and value["magic"] == 0x20b
             and 0 < value["image_size"] <= MAX_FILE
             and 64 <= value["headers_size"] <= min(65536, file_size, value["image_size"])
             and value["entry_rva"] < value["image_size"], "pe_extent_invalid")
    sections, debug, cv = value["sections"], value["debug_entries"], value["codeview"]
    _require(isinstance(sections, list) and 0 < len(sections) <= 96
             and isinstance(debug, list) and len(debug) <= 32
             and isinstance(cv, list) and len(cv) <= len(debug), "pe_counts_invalid")
    for section in sections:
        _keys(section, "virtual_size rva raw_size raw_offset characteristics", "section_invalid")
        for field in section.values():
            _uint(field, 32)
        _require(section["rva"] + max(section["virtual_size"], section["raw_size"]) <= value["image_size"]
                 and section["raw_offset"] + section["raw_size"] <= file_size, "section_extent_invalid")
    indices = []
    for i, entry in enumerate(debug):
        _keys(entry, "type size rva raw_offset timestamp major minor", "debug_entry_invalid")
        for k, field in entry.items():
            _uint(field, 16 if k in ("major", "minor") else 32)
        if entry["type"] == 2:
            _require(24 <= entry["size"] <= 4096 and entry["rva"] > 0
                     and entry["rva"] + entry["size"] <= value["image_size"]
                     and 0 < entry["raw_offset"] <= file_size - entry["size"], "codeview_extent_invalid")
            indices.append(i)
    _require(value["codeview_status"] == ("PRESENT_RSDS" if indices else "ABSENT")
             and len(cv) == len(indices), "codeview_status_invalid")
    for item, index in zip(cv, indices):
        _keys(item, "debug_index guid_raw_hex age", "codeview_invalid")
        _require(type(item["debug_index"]) is int and item["debug_index"] == index
                 and isinstance(item["guid_raw_hex"], str)
                 and re.fullmatch(r"[0-9a-f]{32}", item["guid_raw_hex"]), "codeview_invalid")
        _uint(item["age"], 32)
    return value


def _nonaddress_pe(value):
    return {k: v for k, v in value.items() if k != "preferred_base"}


def _parse_pe(read, size):
    """Parse the same bounded PE fields as the native bridge, from the pinned handle."""
    dos = read(0, 64)
    _require(dos[:2] == b"MZ", "dos_signature")
    nt = struct.unpack_from("<I", dos, 60)[0]
    _require(64 <= nt <= 65536 - 24, "pe_offset")
    coff = read(nt, 24)
    _require(coff[:4] == b"PE\0\0", "pe_signature")
    machine, count, stamp = struct.unpack_from("<HHI", coff, 4)
    optional_size, flags = struct.unpack_from("<HH", coff, 20)
    section_start = nt + 24 + optional_size
    _require(0 < count <= 96 and 112 <= optional_size <= 4096
             and section_start + count * 40 <= 65536, "pe_header_counts")
    optional = read(nt + 24, optional_size)
    u16 = lambda at: struct.unpack_from("<H", optional, at)[0]
    u32 = lambda at: struct.unpack_from("<I", optional, at)[0]
    directories = u32(108)
    _require(directories <= (optional_size - 112) // 8, "pe_directory_count")
    pe = {"machine": machine, "timestamp": stamp, "characteristics": flags, "magic": u16(0),
          "entry_rva": u32(16), "preferred_base": struct.unpack_from("<Q", optional, 24)[0],
          "image_size": u32(56), "headers_size": u32(60), "checksum": u32(64),
          "subsystem": u16(68), "dll_characteristics": u16(70), "sections": [],
          "debug_entries": [], "codeview_status": "ABSENT", "codeview": []}
    _require(section_start + count * 40 <= pe["headers_size"] <= min(size, 65536), "pe_header_extent")
    for index in range(count):
        section = read(section_start + index * 40, 40)
        values = struct.unpack_from("<IIII", section, 8)
        pe["sections"].append(dict(zip(("virtual_size", "rva", "raw_size", "raw_offset"), values),
                                   characteristics=struct.unpack_from("<I", section, 36)[0]))

    def rva_offset(rva, length):
        _require(rva + length <= pe["image_size"], "rva_image_extent")
        found = [rva] if rva + length <= pe["headers_size"] else []
        for section in pe["sections"]:
            relative = rva - section["rva"]
            if 0 <= relative and relative + length <= max(section["virtual_size"], section["raw_size"]):
                _require(relative + length <= section["raw_size"], "rva_zero_fill_not_file")
                found.append(section["raw_offset"] + relative)
        _require(len(found) == 1 and found[0] + length <= size, "rva_ambiguous_or_unmapped")
        return found[0]

    if directories > 6:
        rva, length = u32(160), u32(164)
        if rva or length:
            _require(rva > 0 and 0 < length <= 32 * 28 and length % 28 == 0, "debug_directory_extent")
            offset = rva_offset(rva, length)
            for index in range(length // 28):
                raw = read(offset + index * 28, 28)
                _, stamp, major, minor, kind, count, address, pointer = struct.unpack("<IIHHIIII", raw)
                entry = {"type": kind, "size": count, "rva": address, "raw_offset": pointer,
                         "timestamp": stamp, "major": major, "minor": minor}
                pe["debug_entries"].append(entry)
                if kind == 2:
                    _require(24 <= count <= 4096 and address > 0 and pointer > 0
                             and rva_offset(address, count) == pointer, "codeview_extent")
                    cv = read(pointer, 24)
                    _require(cv[:4] == b"RSDS", "codeview_signature")
                    pe["codeview"].append({"debug_index": index, "guid_raw_hex": cv[4:20].hex(),
                                           "age": struct.unpack_from("<I", cv, 20)[0]})
    pe["codeview_status"] = "PRESENT_RSDS" if pe["codeview"] else "ABSENT"
    return _pe(pe, size)


class _FileId(ctypes.Structure):
    _fields_ = [("volume", ctypes.c_uint64), ("id", ctypes.c_ubyte * 16)]


class _Basic(ctypes.Structure):
    _fields_ = [(name, ctypes.c_int64) for name in ("creation", "access", "write", "change")] + [("attributes", ctypes.c_uint32)]


class _Standard(ctypes.Structure):
    _fields_ = [("allocation", ctypes.c_int64), ("size", ctypes.c_int64),
                ("links", ctypes.c_uint32), ("deleted", ctypes.c_ubyte), ("directory", ctypes.c_ubyte)]


class _Api:
    """Synchronous local fixed-volume file reads; no process or debugger ownership."""
    def __init__(self):
        _require(sys.platform == "win32" and ctypes.sizeof(ctypes.c_void_p) == 8, "requires_native_windows_x64")
        self.k = ctypes.WinDLL("kernel32", use_last_error=True)
        p, d, h, b = ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_int32
        signatures = {
            "CreateFileW": (h, [ctypes.c_wchar_p, d, d, p, d, d, h]),
            "CloseHandle": (b, [h]), "GetFileType": (d, [h]),
            "GetFileInformationByHandleEx": (b, [h, ctypes.c_int, p, d]),
            "GetFinalPathNameByHandleW": (d, [h, ctypes.c_wchar_p, d, d]),
            "GetDriveTypeW": (d, [ctypes.c_wchar_p]),
            "SetFilePointerEx": (b, [h, ctypes.c_int64, ctypes.POINTER(ctypes.c_int64), d]),
            "ReadFile": (b, [h, p, d, ctypes.POINTER(d), p]),
        }
        for name, (restype, argtypes) in signatures.items():
            function = getattr(self.k, name)
            function.restype, function.argtypes = restype, argtypes

    def check(self, ok, operation):
        _require(ok, f"{operation}: {ctypes.get_last_error()}")

    def open(self, path, directory=False):
        _path(path) if len(path) > 3 else _require(re.fullmatch(r"[A-Za-z]:\\", path), "drive_root_invalid")
        _require(self.k.GetDriveTypeW(path[:3]) == 3, "nonlocal_fixed_volume")
        handle = self.k.CreateFileW(path, 0x80 if directory else 0x80000000,
                                    1, None, 3, 0x00200000 | (0x02000000 if directory else 0), None)
        self.check(handle not in (None, 0, ctypes.c_void_p(-1).value), "CreateFileW")
        return handle

    def info(self, handle, kind, structure):
        value = structure()
        self.check(self.k.GetFileInformationByHandleEx(handle, kind, ctypes.byref(value), ctypes.sizeof(value)),
                   "GetFileInformationByHandleEx")
        return value

    def directory(self, handle):
        basic = self.info(handle, 0, _Basic)
        file_type = self.k.GetFileType(handle)
        _require(file_type == 1 and basic.attributes & 0x10
                 and not basic.attributes & 0x400, "directory_reparse_or_not_disk")
        return {"file_type": file_type, "attributes": basic.attributes}

    def facts(self, handle):
        _require(self.k.GetFileType(handle) == 1, "file_not_disk")
        identity = self.info(handle, 18, _FileId)
        basic, standard = self.info(handle, 0, _Basic), self.info(handle, 1, _Standard)
        buffer = ctypes.create_unicode_buffer(MAX_PATH)
        length = self.k.GetFinalPathNameByHandleW(handle, buffer, MAX_PATH, 2)
        self.check(0 < length < MAX_PATH, "GetFinalPathNameByHandleW")
        return _facts({"volume_serial": identity.volume, "file_id_hex": bytes(identity.id).hex(),
                       "final_nt_path_utf16_hex": _hexwide(buffer.value),
                       "creation_time": basic.creation, "last_access_time": basic.access,
                       "last_write_time": basic.write, "change_time": basic.change,
                       "attributes": basic.attributes, "allocation_size": standard.allocation,
                       "file_size": standard.size, "links": standard.links,
                       "delete_pending": standard.deleted, "directory": standard.directory})

    def read(self, handle, offset, length):
        position, done = ctypes.c_int64(-1), ctypes.c_uint32()
        self.check(self.k.SetFilePointerEx(handle, offset, ctypes.byref(position), 0)
                   and position.value == offset, "SetFilePointerEx")
        buffer = ctypes.create_string_buffer(length)
        self.check(self.k.ReadFile(handle, buffer, length, ctypes.byref(done), None)
                   and done.value == length, "ReadFile")
        return buffer.raw

    def close(self, handle):
        self.check(self.k.CloseHandle(handle), "CloseHandle")


class _Pins:
    def __init__(self, api, deadline):
        self.api, self.deadline = api, deadline
        self.bytes = 0
        self.handles, self.files, self.directories, self.cleanup = [], {}, set(), []

    def budget(self):
        _require(time.monotonic() < self.deadline, "binding_deadline_exceeded")

    def _open(self, path, directory):
        self.budget()
        _require(len(self.handles) < MAX_HANDLES, "pin_handle_limit")
        handle = self.api.open(path, directory)
        item = {"handle": handle, "path": path, "directory": directory, "directory_facts": None}
        self.handles.append(item)
        if directory:
            item["directory_facts"] = self.api.directory(handle)
        return handle

    def open(self, path):
        _path(path)
        _require(path not in self.files and len(self.files) < MAX_IMAGES, "duplicate_or_excess_pin")
        parts = path.split("\\")
        for index in range(1, len(parts)):
            directory = "\\".join(parts[:index]) + ("\\" if index == 1 else "")
            if directory not in self.directories:
                self._open(directory, True)
                self.directories.add(directory)
        handle = self._open(path, False)
        self.files[path] = handle
        return self.snapshot(path)

    def snapshot(self, path):
        self.budget()
        handle = self.files[path]
        before = self.api.facts(handle)
        size = before["file_size"]

        def read(offset, length):
            self.budget()
            _require(0 <= offset <= size and 0 <= length <= CHUNK and offset + length <= size,
                     "pin_read_bounds")
            self.bytes += length
            _require(self.bytes <= MAX_HASH_BYTES, "pin_total_read_limit")
            value = self.api.read(handle, offset, length)
            _require(len(value) == length, "pin_short_read")
            return value

        digest = hashlib.sha256()
        for offset in range(0, size, CHUNK):
            digest.update(read(offset, min(CHUNK, size - offset)))
        pe = _parse_pe(read, size)
        after = self.api.facts(handle)
        _require(_stable_facts(before) == _stable_facts(after), "pin_file_changed")
        self.budget()
        return {"path": path, "handle": handle, "file_before": before, "file_after": after,
                "sha256": digest.hexdigest(), "pe": pe}

    def close(self):
        for item in reversed(self.handles):
            row = dict(item, operation="CloseHandle", success=False, error=None)
            try:
                self.api.close(item["handle"])
                row["success"] = True
            except (OSError, ValueError) as exc:
                row["error"] = str(exc)
            self.cleanup.append(row)
        self.handles.clear()
        return self.cleanup


def _system_identity():
    import winreg

    class Version(ctypes.Structure):
        _fields_ = [("size", ctypes.c_uint32), ("major", ctypes.c_uint32),
                    ("minor", ctypes.c_uint32), ("build", ctypes.c_uint32),
                    ("platform", ctypes.c_uint32), ("service_pack", ctypes.c_wchar * 128)]

    ntdll = ctypes.WinDLL("ntdll")
    version = Version()
    version.size = ctypes.sizeof(version)
    query = ntdll.RtlGetVersion
    query.argtypes, query.restype = [ctypes.POINTER(Version)], ctypes.c_int32
    _require(query(ctypes.byref(version)) == 0, "os_version_unavailable")
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    query = k.IsWow64Process2
    query.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint16), ctypes.POINTER(ctypes.c_uint16)]
    query.restype = ctypes.c_int32
    process, native = ctypes.c_uint16(), ctypes.c_uint16()
    _require(query(ctypes.c_void_p(-1), ctypes.byref(process), ctypes.byref(native))
             and process.value == 0 and native.value == 0x8664, "requires_native_windows_x64")
    with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", 0,
                        winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
        build = winreg.QueryValueEx(key, "CurrentBuildNumber")[0]
        revision = winreg.QueryValueEx(key, "UBR")[0]
        build_lab = winreg.QueryValueEx(key, "BuildLabEx")[0]
    _require(build == str(version.build), "os_build_identity_differs")
    return _check_system({"major": version.major, "minor": version.minor, "build": version.build,
                          "revision": revision, "build_lab": build_lab,
                          "native_machine": native.value, "process_machine": process.value})


def _check_system(value):
    _keys(value, "major minor build revision build_lab native_machine process_machine", "system_identity_invalid")
    for key in ("major", "minor", "build", "revision", "native_machine", "process_machine"):
        _uint(value[key], 32)
    _require(value["major"] > 0 and value["build"] > 0 and value["native_machine"] == 0x8664
             and value["process_machine"] == 0 and isinstance(value["build_lab"], str)
             and 0 < len(value["build_lab"]) <= 512 and "\0" not in value["build_lab"],
             "system_identity_invalid")
    return value


def _invocation(argv):
    _require(isinstance(argv, list) and 0 < len(argv) <= 256
             and all(isinstance(a, str) and len(a) <= 4096 and "\0" not in a for a in argv),
             "argv_invalid")
    _path(argv[0])
    _require("--list" not in argv and argv.count("--workload") == 1
             and argv.count("--timing-boundaries") == 1
             and argv.count("--windows-loader-observer") == 1, "one_observed_workload_required")
    at = argv.index("--workload")
    _require(at + 1 < len(argv) and _ROW.fullmatch(argv[at + 1]), "workload_argument_invalid")
    return argv[at + 1]


def _prepare(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds, binding_seconds):
    process_runner._validate(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds)
    _invocation(argv)
    _require(len(os.fspath(cwd)) < MAX_PATH, "cwd_limit")
    _finite(timeout_seconds, 3600, 0.001)
    _finite(cleanup_seconds, 60, 0.001)
    _finite(binding_seconds, 3600, 0.001)
    _require(type(output_limit_bytes) is int and 0 < output_limit_bytes <= MAX_OUTPUT, "output_limit")
    return loader_environment(env)


def _json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            _require(key not in result, "duplicate_json_key")
            result[key] = value
        return result

    def constant(_):
        raise _Refusal("nonfinite_json_number")

    return json.loads(text, object_pairs_hook=pairs, parse_constant=constant)


def _record(record):
    _require(isinstance(record, dict), "record_invalid")
    for key in ("stdout", "stderr"):
        _require(isinstance(record.get(key), str) and "\ufffd" not in record[key], "raw_output_invalid")
    _require(sum(len(record[k].encode("utf-8")) for k in ("stdout", "stderr")) <= MAX_OUTPUT,
             "raw_output_limit")
    _uint(record.get("process_id"), 32, 1)
    _require(record.get("launch_error") is None and "launch_error" in record
             and record.get("timed_out") is False and record.get("output_limited") is False
             and type(record.get("exit_code")) is int, "record_incomplete")
    _require(type(record.get("cleanup_ok")) is bool, "runner_cleanup_unknown")
    _finite(record.get("duration_seconds"), 3660)
    return _json(record["stdout"])


def _markers(record, expected_rows):
    _require(isinstance(expected_rows, list) and len(expected_rows) == 1
             and isinstance(expected_rows[0], str) and _ROW.fullmatch(expected_rows[0]),
             "one_actual_timing_row_required")
    pid = _uint(record.get("process_id"), 32, 1)
    text = record["stderr"]
    markers = []
    for number, line in enumerate(text.splitlines(keepends=True), 1):
        if "mado-component-timing" not in line:
            continue
        _require(line.endswith("\n"), "timing_marker_unterminated")
        marker = _MARKER.fullmatch(line[:-1])
        _require(marker is not None, "timing_marker_malformed")
        wire_pid, row, phase = marker.groups()
        _require(wire_pid == str(pid), "timing_marker_pid_mismatch")
        _require(row == expected_rows[0], "timing_marker_row_mismatch")
        markers.append((phase, number))
    _require([phase for phase, _ in markers] == ["row-start", "samples-begin", "samples-end"],
             "timing_marker_transition_invalid")
    return {"id": expected_rows[0], "row_start_line": markers[0][1],
            "samples_begin_line": markers[1][1], "samples_end_line": markers[2][1]}


def _binding(value):
    _keys(value, "path handle file_before file_after sha256 pe", "pin_binding_invalid")
    _path(value["path"])
    _uint(value["handle"], 64, 1)
    _require(value["handle"] != (1 << 64) - 1, "pin_invalid_handle")
    _require(_stable_facts(value["file_before"]) == _stable_facts(value["file_after"]),
             "pin_same_handle_identity_changed")
    _require(isinstance(value["sha256"], str) and _HEX.fullmatch(value["sha256"]), "pin_hash_invalid")
    _pe(value["pe"], value["file_before"]["file_size"])
    return {"class": "file", "path": value["path"], "sha256": value["sha256"],
            "file": _stable_facts(value["file_before"]), "pe": value["pe"]}


def _cleanup(receipt):
    """Cleanup is a separate fence, even when a primary observation failed."""
    record = receipt.get("record")
    owned = record.get("cleanup_ok") if isinstance(record, dict) else None
    rows = receipt.get("pin_cleanup")
    opened = receipt.get("pin_handles")
    if not isinstance(rows, list) or not isinstance(opened, list) or len(rows) != len(opened):
        return None
    if len(opened) > MAX_HANDLES:
        return None
    for item, row in zip(reversed(opened), rows):
        if not isinstance(item, dict) or not isinstance(row, dict):
            return None
        if set(item) != {"handle", "path", "directory", "directory_facts"} or set(row) != {
                "handle", "path", "directory", "directory_facts", "operation", "success", "error"}:
            return None
        if any(row.get(k) != v for k, v in item.items()) or row["operation"] != "CloseHandle":
            return None
        if type(row["success"]) is not bool or (row["success"] and row["error"] is not None):
            return None
        if not row["success"]:
            return False
    if owned is False:
        return False
    if owned is not True:
        return None
    if record.get("process_id") is None and record.get("launch_error") is not None:
        return True
    try:
        envelope = _record(record)
        _require(type(envelope["process_id"]) is int and envelope["process_id"] == record["process_id"]
                 and len(envelope["workloads"]) == 1, "cleanup_row_unbound")
        report = envelope["workloads"][0]["windows_loader_observer"]
        _require(type(report["pid"]) is int and report["pid"] == record["process_id"], "cleanup_pid_mismatch")
        _require(isinstance(report.get("final_owned_cleanup"), dict)
                 and isinstance(report.get("cleanup"), list) and isinstance(report.get("bridges"), list)
                 and all(isinstance(row, dict) for row in report["cleanup"] + report["bridges"]),
                 "cleanup_shape_invalid")
        if (report.get("final_owned_cleanup", {}).get("ok") is False
                or any(row.get("success") is False for row in report.get("cleanup", []))
                or any(row.get("close_success") is False for row in report.get("bridges", []))):
            return False
        rows = _native_cleanup(report)
        _require(len(rows) == sum(b["close_success"] is True for b in report["bridges"]),
                 "cleanup_bridge_count_mismatch")
    except _INVALID:
        return None
    return True


def _receipt(kind, argv, env):
    return {"format_version": 1, "kind": "windows-runtime-image-" + kind,
            "target": TARGET, "method": METHOD, "lane": LANE, "source": SOURCE,
            "method_identity": method_identity(), "limitations": list(LIMITATIONS),
            "argv": list(argv) if isinstance(argv, list) else argv,
            "loader": {"env": env, "pid": None, "main_executable": None},
            "system": None, "images": [], "events": [], "bindings": {"pre": [], "post": []},
            "pin_handles": [], "pin_cleanup": [], "pin_read_bytes": 0, "cleanup_ok": True,
            "raw_sha256": {},
            "provenance": {"complete": False, "raw_retained": False},
            "record": {"exit_code": None, "timed_out": False, "output_limited": False,
                       "cleanup_ok": True, "stdout": "", "stderr": "", "duration_seconds": 0,
                       "launch_error": "not_launched", "process_id": None},
            "result": {"status": "refused", "reason": None, "exit_code": None,
                       "observed_images": 0, "observed_events": 0, "unbound": []},
            "duration_seconds": 0}


def _image(value):
    _keys(value, "class path sha256 file pe", "closure_image_invalid")
    _require(value["class"] == "file", "unknown_image_class")
    _path(value["path"])
    _require(isinstance(value["sha256"], str) and _HEX.fullmatch(value["sha256"]), "image_hash_invalid")
    _require(isinstance(value["file"], dict) and "last_access_time" not in value["file"], "image_file_invalid")
    _facts(dict(value["file"], last_access_time=0))
    _pe(value["pe"], value["file"]["file_size"])
    return value


def _union(identities):
    _require(identities, "no_discovery_receipts")
    first = identities[0]
    for key in ("target", "main_executable", "loader_env", "system", "method_identity", "capability"):
        _require(all(item[key] == first[key] for item in identities), key + "_differs")
    images, workloads = {}, {}
    for identity in identities:
        workload = identity["workload"]
        _require(workload["id"] not in workloads or workloads[workload["id"]] == workload["statistical"],
                 "workload_policy_differs")
        workloads[workload["id"]] = workload["statistical"]
        for image in identity["images"]:
            key = ntpath.normcase(image["path"])
            _require(key not in images or images[key] == image, "conflicting_file_binding")
            images[key] = image
    _require(0 < len(images) <= MAX_IMAGES, "closure_image_limit")
    result = {"format_version": 1, "kind": "windows-runtime-image-closure", "method": METHOD,
              "lane": LANE, "source": SOURCE, "method_identity": first["method_identity"],
              "target": TARGET, "main_executable": first["main_executable"],
              "loader_env": first["loader_env"], "system": first["system"],
              "capability": first["capability"],
              "images": sorted(images.values(), key=lambda row: row["path"]),
              "workloads": [{"id": row, "statistical": workloads[row]} for row in sorted(workloads)],
              "origins": [item["origin"] for item in identities]}
    result["sha256"] = canonical_sha256(result)
    return result


def _check_closure(closure):
    _keys(closure, "format_version kind method lane source method_identity target main_executable loader_env system capability workloads images origins sha256",
          "closure_fields_invalid")
    _require(type(closure["format_version"]) is int and closure["format_version"] == 1
             and closure["kind"] == "windows-runtime-image-closure" and closure["method"] == METHOD
             and closure["lane"] == LANE and closure["target"] == TARGET
             and closure["source"] == SOURCE and closure["method_identity"] == method_identity(),
             "closure_source_mismatch")
    _require(closure["sha256"] == canonical_sha256({k: v for k, v in closure.items() if k != "sha256"}),
             "closure_hash_mismatch")
    _path(closure["main_executable"])
    _require(loader_environment(closure["loader_env"]) == closure["loader_env"], "closure_environment_invalid")
    _check_system(closure["system"])
    windows_loader_callback.validate_capability(closure["capability"])
    policies = closure["workloads"]
    _require(isinstance(policies, list) and 0 < len(policies) <= 4096, "closure_workloads_invalid")
    seen = set()
    for policy in policies:
        _keys(policy, "id statistical", "closure_workload_invalid")
        _require(isinstance(policy["id"], str) and _ROW.fullmatch(policy["id"])
                 and type(policy["statistical"]) is bool and policy["id"] not in seen,
                 "closure_workload_invalid")
        seen.add(policy["id"])
    _require(isinstance(closure["images"], list) and 0 < len(closure["images"]) <= MAX_IMAGES,
             "closure_images_invalid")
    rows = {}
    for value in closure["images"]:
        image = _image(value)
        key = ntpath.normcase(image["path"])
        _require(key not in rows, "closure_duplicate_image")
        rows[key] = image
    _require(ntpath.normcase(closure["main_executable"]) in rows, "closure_main_unbound")
    origins = closure["origins"]
    _require(isinstance(origins, list) and 0 < len(origins) <= 4096, "closure_origins_invalid")
    for origin in origins:
        _keys(origin, "receipt_sha256 pid events cleanup_ok record_duration_seconds", "closure_origin_invalid")
        _require(isinstance(origin["receipt_sha256"], str) and _HEX.fullmatch(origin["receipt_sha256"]),
                 "closure_origin_hash_invalid")
        _uint(origin["pid"], 32, 1)
        _require(type(origin["events"]) is int and 0 <= origin["events"] <= MAX_EVENTS
                 and origin["cleanup_ok"] is True, "closure_origin_invalid")
        _finite(origin["record_duration_seconds"], 3660)
    return rows


def _check_pin_evidence(receipt, expected, measurement):
    bindings = _keys(receipt.get("bindings"), "pre post", "bindings_invalid")
    for phase in ("pre", "post"):
        _require(isinstance(bindings[phase], list) and len(bindings[phase]) <= MAX_IMAGES,
                 "binding_count_invalid")
    _require(measurement or not bindings["pre"], "discovery_prelaunch_binding_fabricated")
    opened = receipt.get("pin_handles")
    _require(isinstance(opened, list) and len(opened) <= MAX_HANDLES, "pin_handles_invalid")
    file_handles, directory_paths, handles = {}, set(), set()
    for item in opened:
        _keys(item, "handle path directory directory_facts", "pin_handle_invalid")
        _uint(item["handle"], 64, 1)
        _require(item["handle"] != (1 << 64) - 1, "pin_invalid_handle")
        _require(item["handle"] not in handles and type(item["directory"]) is bool, "pin_handle_duplicate")
        handles.add(item["handle"])
        if item["directory"]:
            facts = _keys(item["directory_facts"], "file_type attributes", "pin_directory_facts_missing")
            _uint(facts["attributes"], 32)
            _require(type(facts["file_type"]) is int and facts["file_type"] == 1
                     and facts["attributes"] & 0x10 and not facts["attributes"] & 0x400,
                     "pin_directory_not_regular")
            path = item["path"]
            _require(isinstance(path, str), "pin_directory_invalid")
            _path(path) if len(path) != 3 else _require(re.fullmatch(r"[A-Za-z]:\\", path), "pin_root_invalid")
            directory_paths.add(path)
        else:
            _require(item["directory_facts"] is None, "pin_file_directory_facts_invalid")
            _path(item["path"])
            _require(item["path"] not in file_handles, "pin_path_duplicate")
            file_handles[item["path"]] = item["handle"]
            parts = item["path"].split("\\")
            _require(all(("\\".join(parts[:i]) + ("\\" if i == 1 else "")) in directory_paths
                         for i in range(1, len(parts))), "pin_ancestors_missing")
    phases = {}
    for phase in ("pre", "post"):
        rows = {}
        for binding in bindings[phase]:
            image = _binding(binding)
            path = image["path"]
            _require(path not in rows and file_handles.get(path) == binding["handle"], "binding_handle_mismatch")
            rows[path] = image
        phases[phase] = rows
    _require(set(phases["post"]) == set(file_handles), "post_bindings_missing")
    if measurement:
        _require(phases["pre"] == phases["post"], "same_handle_post_binding_differs")
        _require(phases["pre"] == {row["path"]: row for row in expected.values()}, "prelaunch_closure_binding_differs")
    snapshots = bindings["pre"] + bindings["post"]
    minimum = sum(b["file_before"]["file_size"] + 200 + len(b["pe"]["sections"]) * 40
                  + len(b["pe"]["debug_entries"]) * 28 + len(b["pe"]["codeview"]) * 24 for b in snapshots)
    maximum = minimum + len(snapshots) * (4096 - 112)
    _require(type(receipt.get("pin_read_bytes")) is int
             and minimum <= receipt["pin_read_bytes"] <= min(maximum, MAX_HASH_BYTES), "pin_read_budget_invalid")
    _require(_cleanup(receipt) is True, "owned_cleanup_incomplete")
    return phases["pre"] if measurement else phases["post"]


def _counter(value):
    _keys(value, "first last incomplete_bits pending_sequences clean", "counter_fields_invalid")
    first, last = _uint(value["first"]), _uint(value["last"])
    _uint(value["incomplete_bits"], 32)
    pending = value["pending_sequences"]
    _require(isinstance(pending, list) and len(pending) <= MAX_EVENTS
             and all(type(i) is int and 1 <= i <= min(first, MAX_EVENTS) for i in pending)
             and pending == sorted(set(pending)), "counter_pending_invalid")
    clean = first == last and last <= MAX_EVENTS and value["incomplete_bits"] == 0 and not pending
    _require(type(value["clean"]) is bool and value["clean"] == clean, "counter_summary_mismatch")
    _require(clean, "counter_incomplete_or_overflow")
    return last


def _census(value):
    _keys(value, "before after needed_bytes failure modules", "census_fields_invalid")
    sequence = _counter(value["before"])
    _require(_counter(value["after"]) == sequence and value["failure"] is None, "census_changed")
    modules = value["modules"]
    _require(isinstance(modules, list) and 0 < len(modules) <= MAX_IMAGES
             and type(value["needed_bytes"]) is int and value["needed_bytes"] == len(modules) * 8,
             "census_extent_invalid")
    bases = []
    for module in modules:
        _keys(module, "base size path_utf16_hex", "module_fields_invalid")
        base = _uint(module["base"], 64, 1)
        size = _uint(module["size"], 32, 1)
        _require(size <= MAX_FILE and base + size < 1 << 64, "module_extent_invalid")
        _path(_wide(module["path_utf16_hex"]))
        bases.append(base)
    _require(bases == sorted(set(bases)), "module_order_or_duplicate")
    return sequence


def _events(values, snapshot):
    _require(isinstance(values, list) and len(values) == snapshot, "event_snapshot_count_mismatch")
    for sequence, event in enumerate(values, 1):
        _keys(event, "sequence complete reason flags base size invalid_bits full_name_utf16_hex base_name_utf16_hex",
              "event_fields_invalid")
        _require(type(event["sequence"]) is int and event["sequence"] == sequence
                 and event["complete"] is True, "event_partial_publication")
        _require(type(event["reason"]) is int and event["reason"] in (1, 2), "event_reason_invalid")
        _require(_uint(event["flags"], 32) == 0, "event_flags_invalid")
        _require(type(event["invalid_bits"]) is int and event["invalid_bits"] == 0, "event_invalid")
        base, size = _uint(event["base"], 64, 1), _uint(event["size"], 32, 1)
        _require(size <= MAX_FILE and base + size < 1 << 64, "event_extent_invalid")
        full = _path(_wide(event["full_name_utf16_hex"]))
        name = _wide(event["base_name_utf16_hex"])
        # ASCII-insensitive basename comparison does not supply identity.
        ascii_fold = lambda s: s.translate(str.maketrans("ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz"))
        _require(ascii_fold(ntpath.basename(full)) == ascii_fold(name), "event_basename_mismatch")
    return values


def _instances(censuses, bridges, events):
    """Associate each notification with one lifetime containing an observed bridge.

    A transient load/unload without a bridge, or reuse without an intervening
    notification, cannot be promoted by matching names/base to some other load.
    This conditional sequence linkage does not rule out delayed notifications.
    """
    intervals, active, known = [], {}, set()
    for event in events:
        base, sequence = event["base"], event["sequence"]
        if event["reason"] == 1:
            _require(base not in active, "event_load_over_live_instance")
            instance = {"base": base, "size": event["size"], "path_utf16_hex": event["full_name_utf16_hex"],
                        "start": sequence, "end": MAX_EVENTS + 1, "bridges": []}
            intervals.append(instance)
            active[base] = instance
        else:
            if base not in active:
                _require(base not in known, "event_unload_without_instance")
                instance = {"base": base, "size": event["size"], "path_utf16_hex": event["full_name_utf16_hex"],
                            "start": 0, "end": MAX_EVENTS + 1, "bridges": []}
                intervals.append(instance)
                active[base] = instance
            instance = active.pop(base)
            _require(instance["size"] == event["size"]
                     and instance["path_utf16_hex"] == event["full_name_utf16_hex"], "event_instance_mismatch")
            instance["end"] = sequence
        known.add(base)
    for census in censuses:
        for module in census["modules"]:
            if module["base"] not in known:
                instance = dict(module, start=0, end=MAX_EVENTS + 1, bridges=[])
                intervals.append(instance)
                known.add(module["base"])
    for index, census in enumerate(censuses):
        sequence = census["after"]["last"]
        live = [item for item in intervals if item["start"] <= sequence < item["end"]]
        expected = sorted(({k: item[k] for k in ("base", "size", "path_utf16_hex")} for item in live),
                          key=lambda row: row["base"])
        _require(expected == census["modules"], "census_event_lifetime_mismatch")
        for bridge in bridges:
            if bridge["census_index"] != index:
                continue
            matches = [item for item in live if item["base"] == bridge["base"]]
            _require(len(matches) == 1, "bridge_instance_ambiguous")
            matches[0]["bridges"].append(bridge)
    for instance in intervals:
        _require(instance["bridges"], "event_instance_without_observed_bridge")
        first = instance["bridges"][0]
        for bridge in instance["bridges"][1:]:
            _require(_stable_facts(bridge["file_before"]) == _stable_facts(first["file_before"])
                     and bridge["file_pe"] == first["file_pe"], "instance_file_identity_changed")
    return intervals


def _bound_images(native, bindings, main):
    rows = {}
    by_path = {ntpath.normcase(path): value for path, value in bindings.items()}
    for instance in native["instances"]:
        path = _wide(instance["path_utf16_hex"])
        image = by_path.get(ntpath.normcase(path))
        _require(image is not None, "observed_image_not_prebound: " + path)
        for bridge in instance["bridges"]:
            _require(_stable_facts(bridge["file_before"]) == image["file"]
                     and _stable_facts(bridge["file_after"]) == image["file"]
                     and bridge["file_pe"] == image["pe"], "bridge_pin_identity_mismatch: " + path)
        key = ntpath.normcase(path)
        _require(key not in rows or rows[key] == image, "conflicting_observed_image")
        rows[key] = image
    _require(ntpath.normcase(main) in rows, "main_executable_unobserved")
    return sorted(rows.values(), key=lambda row: row["path"])


def _derive(receipt, closure=None, *, check_summaries=True):
    measurement = closure is not None
    expected = _check_closure(closure) if measurement else {}
    _require(isinstance(receipt, dict) and type(receipt.get("format_version")) is int
             and receipt["format_version"] == 1 and receipt.get("kind") == "windows-runtime-image-"
             + ("observation" if measurement else "discovery")
             and receipt.get("method") == METHOD and receipt.get("lane") == LANE
             and receipt.get("target") == TARGET and receipt.get("source") == SOURCE
             and receipt.get("method_identity") == method_identity()
             and receipt.get("limitations") == LIMITATIONS, "receipt_source_mismatch")
    row = _invocation(receipt.get("argv"))
    main = receipt["argv"][0]
    loader = _keys(receipt.get("loader"), "env pid main_executable", "loader_invalid")
    _require(loader_environment(loader["env"]) == loader["env"], "loader_environment_invalid")
    system = _check_system(receipt.get("system"))
    native = _native_record(receipt.get("record"), [row])
    _require(receipt.get("raw_sha256") == {
        key: hashlib.sha256(receipt["record"][key].encode("utf-8")).hexdigest()
        for key in ("stdout", "stderr")}, "raw_output_hash_mismatch")
    _require(type(loader["pid"]) is int and loader["pid"] == receipt["record"]["process_id"]
             and loader["main_executable"] == main, "loader_summary_mismatch")
    if measurement:
        _require(receipt.get("closure_sha256") == closure["sha256"], "closure_sha256_mismatch")
        _require(main == closure["main_executable"] and loader["env"] == closure["loader_env"]
                 and system == closure["system"] and native["capability"] == closure["capability"],
                 "measurement_context_changed")
        _require({"id": row, "statistical": native["statistical"]} in closure["workloads"],
                 "measurement_workload_policy_changed")
    bindings = _check_pin_evidence(receipt, expected, measurement)
    images = _bound_images(native, bindings, main)
    _require(measurement or len(images) == len(bindings), "discovery_extra_binding")
    _require(not native["statistical"] or native["window_unchanged"], "statistical_window_changed")
    if check_summaries:
        _require(receipt.get("images") == images and receipt.get("events") == native["events"],
                 "observation_summary_mismatch")
    return {"target": TARGET, "main_executable": main, "loader_env": loader["env"], "system": system,
            "method_identity": receipt["method_identity"], "images": images, "events": native["events"],
            "capability": native["capability"],
            "workload": {"id": row, "statistical": native["statistical"]},
            "origin": {"receipt_sha256": canonical_sha256(receipt),
                       "pid": receipt["record"]["process_id"], "events": len(native["events"]),
                       "cleanup_ok": _cleanup(receipt), "record_duration_seconds": receipt["record"]["duration_seconds"]}}


_INVALID = (ValueError, TypeError, KeyError, OverflowError, RecursionError)


def validate_discovery(receipt, *, preceding=()):
    """Re-read raw row identity; a nonzero child exit cannot create a closure."""
    try:
        _require(isinstance(preceding, (list, tuple)) and len(preceding) <= 4096, "preceding_receipts_invalid")
        identity = _derive(receipt)
        identities = [_derive(item) for item in preceding] + [identity]
        _union(identities)
        _require(all(item["record"]["exit_code"] == 0 for item in preceding), "preceding_child_failed")
    except _INVALID as exc:
        return {"provenance_complete": False, "reason": str(exc), "identity": None,
                "cleanup_ok": _cleanup(receipt) if isinstance(receipt, dict) else None, "exit_code": None}
    return {"provenance_complete": True, "reason": None, "identity": identity,
            "cleanup_ok": all(item["origin"]["cleanup_ok"] for item in identities),
            "exit_code": receipt["record"]["exit_code"]}


def validate_closure(receipts):
    """Derive future representative bindings from retained raw discovery evidence."""
    try:
        items = receipts if isinstance(receipts, list) else [receipts]
        _require(0 < len(items) <= 4096, "discovery_receipt_count_invalid")
        identities = []
        for receipt in items:
            identities.append(_derive(receipt))
            _require(receipt["record"]["exit_code"] == 0, "discovery_child_failed")
        closure = _union(identities)
    except _INVALID as exc:
        return {"ok": False, "reason": str(exc), "closure": None}
    return {"ok": True, "reason": None, "closure": closure}


def validate_observation(receipt, closure):
    """Never accept cached complete/window/cleanup summaries as identity evidence."""
    cleanup = _cleanup(receipt) if isinstance(receipt, dict) else None
    try:
        derived = _derive(receipt, closure)
        code = receipt["record"]["exit_code"]
    except _INVALID as exc:
        return {"ok": False, "reason": str(exc), "provenance_complete": False, "cleanup_ok": cleanup}
    return {"ok": code == 0, "reason": None if code == 0 else f"child_exit_{code}",
            "provenance_complete": True, "cleanup_ok": cleanup,
            "observed_images": len(derived["images"]), "observed_events": len(derived["events"])}


def _run(argv, *, cwd, env, closure, timeout_seconds, output_limit_bytes, cleanup_seconds, binding_seconds):
    started = time.monotonic()
    receipt = _receipt("observation" if closure is not None else "discovery", argv, {})
    pins, reason = None, None
    try:
        loader_env = _prepare(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds, binding_seconds)
        receipt["loader"] = {"env": loader_env, "pid": None, "main_executable": argv[0]}
        expected = _check_closure(closure) if closure is not None else {}
        _require(sys.platform == "win32", "requires_native_windows_x64")
        receipt["system"] = _system_identity()
        if closure is not None:
            receipt["closure_sha256"] = closure["sha256"]
            _require(closure["main_executable"] == argv[0] and closure["loader_env"] == loader_env
                     and closure["system"] == receipt["system"], "prelaunch_context_changed")
            _require(any(policy["id"] == _invocation(argv) for policy in closure["workloads"]),
                     "prelaunch_workload_not_discovered")
        pins = _Pins(_Api(), time.monotonic() + binding_seconds)
        if closure is not None:
            for image in expected.values():
                binding = pins.open(image["path"])
                receipt["bindings"]["pre"].append(binding)
                _require(_binding(binding) == image, "prelaunch_file_changed")
        # The existing runner alone owns process, output and child deadlines.
        receipt["record"]["cleanup_ok"] = None
        receipt["record"]["launch_error"] = "runner_did_not_return"
        receipt["record"] = process_runner.run_process(
            argv, cwd=cwd, env=env, timeout_seconds=timeout_seconds,
            output_limit_bytes=output_limit_bytes, cleanup_seconds=cleanup_seconds)
        receipt["loader"]["pid"] = receipt["record"].get("process_id")
        pins.deadline = time.monotonic() + binding_seconds
        if closure is not None:
            # Check every prior handle, including closure images absent from this row.
            for path in pins.files:
                try:
                    receipt["bindings"]["post"].append(pins.snapshot(path))
                except (OSError, ValueError) as exc:
                    reason = reason or str(exc)
        else:
            native = _native_record(receipt["record"], [_invocation(argv)])
            paths = {ntpath.normcase(_wide(item["path_utf16_hex"])): _wide(item["path_utf16_hex"])
                     for item in native["instances"]}
            _require(ntpath.normcase(argv[0]) in paths, "main_executable_unobserved")
            paths[ntpath.normcase(argv[0])] = argv[0]
            for path in sorted(paths.values()):
                receipt["bindings"]["post"].append(pins.open(path))
    except (OSError, *_INVALID) as exc:
        reason = reason or str(exc)
    finally:
        if pins is not None:
            receipt["pin_handles"] = list(pins.handles)
            receipt["pin_cleanup"] = pins.close()
            receipt["pin_read_bytes"] = pins.bytes
        receipt["cleanup_ok"] = _cleanup(receipt)
        receipt["duration_seconds"] = time.monotonic() - started
    receipt["raw_sha256"] = {key: hashlib.sha256(receipt["record"][key].encode("utf-8")).hexdigest()
                             for key in ("stdout", "stderr")}
    receipt["provenance"]["raw_retained"] = receipt["record"].get("output_limited") is False
    if reason is None:
        try:
            derived = _derive(receipt, closure, check_summaries=False)
            receipt["images"], receipt["events"] = derived["images"], derived["events"]
            receipt["provenance"]["complete"] = True
        except _INVALID as exc:
            reason = str(exc)
    code = receipt["record"].get("exit_code")
    if reason is None and code != 0:
        reason = f"child_exit_{code}"
    receipt["result"].update(status="complete" if reason is None else "incomplete", reason=reason,
                             exit_code=code, observed_images=len(receipt["images"]),
                             observed_events=len(receipt["events"]))
    receipt["duration_seconds"] = time.monotonic() - started
    return receipt


def discover(argv, *, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds, binding_seconds=120.0):
    """Run one actual row once; post-exit file bindings are future discovery only."""
    return _run(argv, cwd=cwd, env=env, closure=None, timeout_seconds=timeout_seconds,
                output_limit_bytes=output_limit_bytes, cleanup_seconds=cleanup_seconds, binding_seconds=binding_seconds)


def observe(argv, *, cwd, env, closure, timeout_seconds, output_limit_bytes, cleanup_seconds, binding_seconds=120.0):
    """Hold all prior closure identities before launch, through exit and same-handle rehash."""
    return _run(argv, cwd=cwd, env=env, closure=closure, timeout_seconds=timeout_seconds,
                output_limit_bytes=output_limit_bytes, cleanup_seconds=cleanup_seconds, binding_seconds=binding_seconds)


def _bridge_rows(report):
    censuses, bridges = report["censuses"], report["bridges"]
    _require(isinstance(bridges, list) and 0 < len(bridges) <= MAX_BRIDGES, "bridge_count_invalid")
    expected = [(stage, index, module_index) for stage, index in (("initial", 1), ("begin", 3), ("end", 5))
                for module_index in range(len(censuses[index]["modules"]))]
    _require(len(bridges) == len(expected), "bridge_coverage_missing")
    for bridge, (stage, index, module_index) in zip(bridges, expected):
        _keys(bridge, "stage census_index module_index base mapped_nt_path_utf16_hex mapped_nt_path_after_utf16_hex file_before file_after file_before_complete file_after_complete memory_pe file_pe memory_pe_complete file_pe_complete failure close_success",
              "bridge_fields_invalid")
        _require(bridge["stage"] == stage and type(bridge["census_index"]) is int
                 and bridge["census_index"] == index and type(bridge["module_index"]) is int
                 and bridge["module_index"] == module_index, "bridge_census_association_invalid")
        module = censuses[index]["modules"][module_index]
        _require(type(bridge["base"]) is int and bridge["base"] == module["base"]
                 and bridge["failure"] is None and bridge["close_success"] is True, "bridge_incomplete")
        _require(all(bridge[key] is True for key in (
            "file_before_complete", "file_after_complete", "memory_pe_complete", "file_pe_complete")),
            "bridge_partial_fields")
        before, after = _facts(bridge["file_before"]), _facts(bridge["file_after"])
        _require(_stable_facts(before) == _stable_facts(after), "bridge_file_changed")
        _require(bridge["mapped_nt_path_utf16_hex"] == before["final_nt_path_utf16_hex"]
                 == bridge["mapped_nt_path_after_utf16_hex"], "bridge_mapped_path_mismatch")
        memory = _pe(bridge["memory_pe"], before["file_size"])
        file = _pe(bridge["file_pe"], before["file_size"])
        _require(memory["image_size"] == module["size"]
                 and _nonaddress_pe(memory) == _nonaddress_pe(file), "bridge_pe_correlation_mismatch")
    return bridges


def _native_record(record, expected_rows):
    envelope = _record(record)
    _require(record["cleanup_ok"] is True, "runner_cleanup_incomplete")
    _require(isinstance(envelope, dict) and envelope.get("kind") == "observations"
             and envelope.get("target") == TARGET
             and type(envelope.get("process_id")) is int
             and envelope["process_id"] == record["process_id"]
             and envelope.get("windows_loader_observer_enabled") is True
             and envelope.get("timing_boundaries_enabled") is True
             and envelope.get("timing_boundary_protocol") == TIMING_PROTOCOL, "native_envelope_mismatch")
    windows_loader_callback.validate_capability(envelope.get("windows_loader_observer"))
    workloads = envelope.get("workloads")
    _require(isinstance(workloads, list) and len(workloads) == 1 and isinstance(workloads[0], dict),
             "one_native_row_required")
    row = workloads[0]
    _require(isinstance(expected_rows, list) and expected_rows == [row.get("id")]
             and type(row.get("statistical")) is bool, "native_workload_mismatch")
    report = row.get("windows_loader_observer")
    _require(isinstance(report, dict) and report.get("protocol") == PROTOCOL
             and report.get("method") == METHOD and type(report.get("pid")) is int
             and report["pid"] == record["process_id"] and report.get("workload_id") == row["id"]
             and type(report.get("statistical")) is bool and report["statistical"] == row["statistical"],
             "native_report_identity_mismatch")
    result = _native(report)
    result["markers"] = _markers(record, expected_rows)
    result["statistical"] = row["statistical"]
    result["report_sha256"] = canonical_sha256(report)
    result["capability"] = envelope["windows_loader_observer"]
    return result


def validate_timing_boundaries(record, expected_rows):
    """The shared marker protocol binds one native row; Windows has no dyld log."""
    result = {"protocol": TIMING_PROTOCOL, "complete": False, "ok": False, "reason": None,
              "process_id": None, "rows": []}
    try:
        native = _native_record(record, expected_rows)
        begin, end = native["window"]
        result["process_id"] = record["process_id"]
        result["rows"] = [dict(native["markers"], pre_window_loader_events=begin,
                               in_window_loader_events=end - begin,
                               post_window_loader_events=len(native["events"]) - end,
                               window_unchanged=native["window_unchanged"],
                               native_report_sha256=native["report_sha256"])]
        result["complete"] = True
        result["ok"] = native["window_unchanged"]
        if not result["ok"]:
            result["reason"] = "timing_observed_window_change"
    except _INVALID as exc:
        result["reason"] = str(exc)
    return result


NATIVE_LIMITS = {
    "modules": 128, "events": 256, "name_utf16_units": 1024, "censuses": 20,
    "total_census_path_units": 131072, "image_file_bytes": 536870912, "header_bytes": 65536,
    "sections": 96, "debug_entries": 32, "total_sections": 4096, "total_debug_entries": 4096,
    "codeview_bytes": 4096, "read_bytes": 67108864, "read_calls": 32768, "bridges": 129,
    "stdout_bytes": 8388608, "worst_case_stdout_bytes": 7788672, "cleanup_records": 258,
    "workload_id_bytes": 256, "callback_storage_bytes": 1058828,
    "fixed_storage_bytes": 3493341, "observation_storage_bytes": 2434512, "owner_bytes": 56,
    "storage_bytes_scope": "sum_of_static_objects_excludes_linker_padding_code_and_native_cost",
}
NATIVE_SCOPE = {
    "qualification": False, "claim": "conditional_observed_window_and_path_header_correlation_only",
    "backing_object_proof": False, "code_bytes_proof": False,
    "path_comparison": "exact_utf16_no_normalization", "owned_cleanup_scope": "file_handles_only",
    "callback_lifetime": "process_exit_no_unregister_no_drain", "outer_process_cleanup": "supervisor_only",
    "serialization_exit_observed": False, "records_complete_scope": "snapshot_n_only",
}


def _native_cleanup(report):
    rows = report.get("cleanup")
    _require(isinstance(rows, list) and len(rows) <= 258, "native_cleanup_missing")
    failed = []
    for row in rows:
        _keys(row, "operation handle success failure", "native_cleanup_invalid")
        _uint(row["handle"], 64, 1)
        _require(row["operation"] in ("CloseHandle", "file_post_observation")
                 and type(row["success"]) is bool, "native_cleanup_invalid")
        if not row["success"]:
            failed.append(row["handle"])
        _require(row["success"] is True and row["failure"] is None
                 and row["operation"] == "CloseHandle", "native_cleanup_failed")
    final = _keys(report.get("final_owned_cleanup"), "ok remaining_owned_dll failed_close_handles",
                  "native_final_cleanup_invalid")
    _require(final["ok"] is True and type(final["remaining_owned_dll"]) is int
             and final["remaining_owned_dll"] == 0 and final["failed_close_handles"] == failed,
             "native_final_cleanup_incomplete")
    return rows


def _native(report):
    _keys(report, "protocol method pid workload_id statistical complete window_unchanged primary_failure registration cuts censuses bridges cleanup cut_counters window_counter final_counter snapshot_n events records_complete final_owned_cleanup limits read_bytes read_calls census_path_units total_sections_reserved total_debug_entries_reserved "
          + " ".join(NATIVE_SCOPE), "native_report_fields_invalid")
    for key, value in NATIVE_SCOPE.items():
        _require(type(report[key]) is type(value) and report[key] == value, "native_scope_mismatch")
    _require(report["primary_failure"] is None, "native_primary_failure")
    limits = report["limits"]
    _require(isinstance(limits, dict) and set(limits) == set(NATIVE_LIMITS), "native_limits_invalid")
    for key, value in NATIVE_LIMITS.items():
        _require(type(limits[key]) is type(value) and limits[key] == value, "native_limits_mismatch")
    cuts = _keys(report["cuts"], "initial begin end", "native_cuts_invalid")
    _require(cuts == {"initial": [0, 1], "begin": [2, 3], "end": [4, 5]}
             and all(type(i) is int for pair in cuts.values() for i in pair), "native_six_cuts_required")
    censuses = report["censuses"]
    _require(isinstance(censuses, list) and len(censuses) == 6, "native_six_censuses_required")
    sequences = [_census(value) for value in censuses]
    post = _keys(report["cut_counters"], "initial begin end", "cut_counters_invalid")
    for stage, (a, b) in cuts.items():
        _require(sequences[a] == sequences[b] == _counter(post[stage])
                 and censuses[a]["modules"] == censuses[b]["modules"], "cut_changed_or_partial")
    window = _keys(report["window_counter"], "begin end", "window_counter_invalid")
    begin, end = _counter(window["begin"]), _counter(window["end"])
    final = _counter(report["final_counter"])
    _require(window["begin"] == post["begin"] and report["final_counter"] == post["end"]
             and end == sequences[4] and sequences == sorted(sequences)
             and sequences[1] <= begin <= end <= final, "counter_cut_order_invalid")
    _require(type(report["snapshot_n"]) is int and report["snapshot_n"] == final,
             "snapshot_counter_mismatch")
    events = _events(report["events"], final)
    _require(report["records_complete"] is True, "records_complete_summary_mismatch")
    bridges = _bridge_rows(report)
    cleanup = _native_cleanup(report)
    _require(len(cleanup) == len(bridges), "native_close_coverage_missing")
    registration = _keys(report["registration"], "module export ntstatus cookie unregister_called",
                         "registration_fields_invalid")
    for key in ("module", "export", "cookie"):
        _uint(registration[key], 64, 1)
    _require(type(registration["ntstatus"]) is int and registration["ntstatus"] == 0
             and registration["unregister_called"] is False, "registration_failed_or_unregistered")
    loader = [b for b in bridges if b["stage"] == "initial" and b["base"] == registration["module"]
              and ntpath.basename(_wide(censuses[1]["modules"][b["module_index"]]["path_utf16_hex"])).lower() == "ntdll.dll"]
    _require(len(loader) == 1, "registration_module_unbound")
    offset = registration["export"] - registration["module"]
    _require(any(s["rva"] <= offset < s["rva"] + s["virtual_size"] and s["characteristics"] & 0x20000000
                 for s in loader[0]["memory_pe"]["sections"]), "registration_export_outside_code")
    pes = [bridge[key] for bridge in bridges for key in ("memory_pe", "file_pe")]
    sections = sum(len(pe["sections"]) for pe in pes)
    debug = sum(len(pe["debug_entries"]) for pe in pes)
    cv = sum(len(pe["codeview"]) for pe in pes)
    paths = sum(len(module["path_utf16_hex"]) // 4 for c in censuses for module in c["modules"])
    for key, expected, cap in (("total_sections_reserved", sections, 4096),
                               ("total_debug_entries_reserved", debug, 4096),
                               ("census_path_units", paths, 131072),
                               ("read_calls", len(pes) * 3 + sections + debug + cv, 32768)):
        _require(type(report[key]) is int and report[key] == expected <= cap, "native_budget_mismatch: " + key)
    # OptionalHeader byte counts are not serialized. Retain the raw count and
    # check the complete bounded range, never reconstruct an invented exact value.
    minimum = len(pes) * (64 + 24 + 112) + sections * 40 + debug * 28 + cv * 24
    maximum = minimum + len(pes) * (4096 - 112)
    _require(type(report["read_bytes"]) is int and minimum <= report["read_bytes"] <= min(maximum, 67108864),
             "native_read_byte_budget_invalid")
    instances = _instances(censuses, bridges, events)
    unchanged = begin == end and censuses[3]["modules"] == censuses[4]["modules"]
    _require(type(report["window_unchanged"]) is bool and report["window_unchanged"] == unchanged
             and report["complete"] is True, "native_summary_mismatch")
    return {"instances": instances, "events": events, "window": (begin, end), "window_unchanged": unchanged}
