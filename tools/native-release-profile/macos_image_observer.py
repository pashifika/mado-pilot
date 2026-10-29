"""Owned macOS dyld load-log discovery and closure observation for one component process.

Scope is the single-process component benchmark lane only: the child owns no
descendants, startup is outside its timed intervals, and the receipt claims
nothing about consumer startup, UI or multiprocess lanes. ``discover`` and
``observe`` run the exact child through the existing process runner with a
child-only ``DYLD_PRINT_LIBRARIES=1`` and parse the retained stderr.

The host's dyld(1) contract documents one log line per loaded Mach-O image.
Qualification binds that manual and OS identity; it does not assume the public
dyld-1231.3 source reference below matches the running dyld. The reference
describes UUID/path lines, delay-init notes and fd 2 output. Missing root
logging is unsupported, not an empty inventory. Manual mappings and unload
chronology are outside coverage.

Every observed image must be bound: on-disk images by file identity, SHA-256
and the Mach-O slice whose LC_UUID equals the logged UUID; shared-cache membership
is observed through libdyld and bound to its cache UUID, architecture and OS identity.
Hashing happens before launch and after owned exit, never inside the child's lifetime. The root pid is
the runner record's ``process_id`` (the exec'd root's Popen pid) and must equal
the pid every dyld line declares; a record without a positive ``process_id``
leaves the attempt incomplete, and the log pid is never substituted.
Descendants that never log stay invisible: no-spawn is a declared lane assumption.
"""

from __future__ import annotations

import ctypes
import hashlib
import importlib.util
import json
import os
import platform
import posixpath
from pathlib import Path
import re
import stat
import struct
import sys
import time
import uuid as uuid_module

FORMAT_VERSION = 1
METHOD = "dyld-print-libraries-stderr-v1"
LANE = "single-process-component-benchmark"
LANE_NOT_COVERED = (
    "consumer startup, UI and multiprocess lanes",
    "descendant processes: only the single logging pid is claimed",
    "image unloads and manual mappings: dyld logs Loader creation only",
    "loads in any later run that is not itself observed",
)
PROCESS_SCOPE = {
    "claim": "one process: the root the runner exec'd, whose runner-owned process_id must "
             "equal the pid every dyld line declares",
    "root_pid": "record.process_id is the only pid authority; a missing or non-positive "
                "value leaves the attempt incomplete and the log pid is never substituted",
    "root_line_source_note": (
        "dyld logs the main executable in RuntimeState::setMainLoader during prepare(), "
        "before dependents, initializers or user code, with pid = getpid() "
        "(DyldProcessConfig.cpp Process ctor); a restricted root has every DYLD_* "
        "variable pruned (Security::pruneEnvVars), so descendants cannot inherit logging"),
    "descendants": "not observed: a descendant that does not log (restricted, or forked "
                   "without exec) leaves no line; the component lane assumes no process "
                   "spawn and this helper cannot verify that assumption",
}
SOURCE = {
    "role": "non-host implementation reference; coverage uses the pinned host dyld(1) contract",
    "repository": "https://github.com/apple-oss-distributions/dyld",
    "tag": "dyld-1231.3",
    "load_line": "dyld/Loader.cpp Loader::logLoad emits '<UUID> path' per Loader creation",
    "load_sites": [
        "dyld/DyldRuntimeState.cpp RuntimeState::setMainLoader",
        "dyld/JustInTimeLoader.cpp JustInTimeLoader::makeJustInTimeLoaderDisk",
        "dyld/JustInTimeLoader.cpp JustInTimeLoader::makeJustInTimeLoaderDyldCache",
        "dyld/PrebuiltLoader.cpp PrebuiltLoader::map",
    ],
    "informational_sites": [
        "dyld/DyldRuntimeState.cpp RuntimeState::partitionDelayLoads",
        "dyld/DyldRuntimeState.cpp RuntimeState::recursiveMarkNonDelayed",
        "dyld/dyldMain.cpp prepare inserted dylib duplicates",
    ],
    "unload_logging": "none in RuntimeState::removeLoaders/garbageCollectInner",
    "gate": "ProcessConfig::Logging: allowEnvVarsPrint (AMFI) && DYLD_PRINT_LIBRARIES",
    "output": "RuntimeState::vlog: 'dyld[pid]: ' prefix then message on STDERR_FILENO",
}
OBSERVER_EFFECTS = (
    "dyld log formatting, stderr writes and possible pipe/scheduling contention inside the child",
    "child-only DYLD_PRINT_LIBRARIES=1 added to the supplied environment",
    "file stat/hash/Mach-O reads before launch and after owned exit only",
)
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}
SYSTEM_PREFIXES = ("/usr/lib/", "/System/Library/")
REJECTED_LOADER_KEYS = frozenset((
    "DYLD_INSERT_LIBRARIES", "DYLD_SHARED_REGION", "DYLD_SHARED_CACHE_DIR", "DYLD_ROOT_PATH",
))
SYSTEM_KEYS = (
    "os_product_version",
    "os_build",
    "kernel_uuid",
    "kernel_version",
    "hardware_machine",
    "hardware_model",
    "cpu_brand",
    "shared_cache_uuid",
    "observer_architecture",
)
_SYSCTL_NAMES = {
    "os_product_version": "kern.osproductversion",
    "os_build": "kern.osversion",
    "kernel_uuid": "kern.uuid",
    "kernel_version": "kern.version",
    "hardware_machine": "hw.machine",
    "hardware_model": "hw.model",
    "cpu_brand": "machdep.cpu.brand_string",
}
DYLD_PATH = "/usr/lib/dyld"
MAX_OUTPUT = 8 << 20
MAX_IMAGES = 4096
MAX_EVENTS = 16384
MAX_FILE = 1 << 30
MAX_HASH_BYTES = 8 << 30
MAX_PATH = 1024
MAX_ARGV = 256
MAX_ARG = 4096
MAX_ENV = 512
MAX_RETAINED_UNKNOWN = 64
MAX_SLICES = 64
MAX_LOAD_COMMANDS = 4096
MAX_COMMAND_BYTES = 1 << 20
CHUNK = 65536
LC_UUID = 0x1B
_CPU_NAMES = {
    (0x0100000C, 0): "arm64",
    (0x0100000C, 1): "arm64v8",
    (0x0100000C, 2): "arm64e",
    (0x01000007, 3): "x86_64",
    (0x01000007, 8): "x86_64h",
}
_UUID_TEXT = r"[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}"
_UUID = re.compile("^" + _UUID_TEXT + "$")
_PREFIX = re.compile(r"^dyld\[(\d{1,10})\]: (.*)$")
_LOAD = re.compile("^<(" + _UUID_TEXT + r"|no uuid)> (/.*)$")
_INFORMATIONAL = (
    ("delayed", re.compile(r"^move loaded to delayed: (.+)$")),
    ("undelayed", re.compile(r"^move delayed to loaded: (.+)$")),
    ("interposing", re.compile(r"^has interposing tuples so cannot be delayed: (.+)$")),
    ("weak_def", re.compile(
        r"^(.+) has weak-def \(or flat lookup\) symbol used by (.+), so cannot be delayed$")),
    ("inserted_duplicate", re.compile(r"^skipping duplicate inserted dylib '(.*)'$")),
)
_HEX64 = re.compile(r"^[0-9a-f]{64}$")
TIMING_PROTOCOL = "mado-component-timing-v1"
_TIMING_MARKER = re.compile(
    r"mado-component-timing-v1\[([0-9]{1,10})\]: ([A-Za-z0-9-]{1,128}) "
    r"(row-start|samples-begin|samples-end)")

# The existing runner is imported by file, like the Windows preflight helper.
_TOOLS = str(Path(__file__).resolve().parent)
if _TOOLS not in sys.path:
    sys.path.insert(0, _TOOLS)


class _Refusal(Exception):
    pass


def _require(ok, reason):
    if not ok:
        raise _Refusal(reason)


def _sibling(name):
    spec = importlib.util.spec_from_file_location(
        "_macos_image_observer_" + name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _sha256_file_bytes(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(CHUNK), b""):
            digest.update(block)
    return digest.hexdigest()


def method_identity():
    """Identity of this exact helper file plus the dyld source it was read against."""
    return {"module": Path(__file__).name, "sha256": _sha256_file_bytes(__file__),
            "method": METHOD, "source": SOURCE}


def canonical_sha256(value):
    """SHA-256 of the canonical JSON encoding used for closure and receipt identity."""
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(encoded.encode("ascii")).hexdigest()


def _digest(value, reason):
    try:
        return canonical_sha256(value)
    except (TypeError, ValueError):
        raise _Refusal(reason) from None


def _empty_record(launch_error):
    return {"exit_code": None, "timed_out": False, "output_limited": False,
            "cleanup_ok": True, "stdout": "", "stderr": "", "duration_seconds": 0.0,
            "launch_error": launch_error, "process_id": None}


def _receipt(kind):
    return {
        "format_version": FORMAT_VERSION, "kind": kind, "target": None, "method": METHOD,
        "lane": LANE, "lane_not_covered": list(LANE_NOT_COVERED),
        "process_scope": PROCESS_SCOPE, "source": SOURCE,
        "method_identity": method_identity(), "observer_effects": list(OBSERVER_EFFECTS),
        "result": {"status": "refused", "reason": None, "exit_code": None,
                   "observed_images": 0, "observed_events": 0, "unbound": []},
        "record": _empty_record(None),
        "loader": {"env": {}, "pid": None, "main_executable": None, "lines": 0,
                   "application_stderr_lines": 0, "informational": [], "unknown": [],
                   "foreign_pids": []},
        "system": None, "dyld": None, "images": [], "events": [],
        "provenance": {"complete": False, "raw_retained": False},
        "cleanup_ok": True, "duration_seconds": 0.0,
    }


def _refused(receipt, reason, started):
    receipt["result"]["status"] = "refused"
    receipt["result"]["reason"] = reason
    receipt["record"] = _empty_record(reason)
    receipt["duration_seconds"] = time.monotonic() - started
    return receipt


def _prepare(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds,
             binding_seconds):
    runner = _sibling("process_runner")
    runner._validate(argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds)
    if (timeout_seconds > 3600 or cleanup_seconds > 60 or output_limit_bytes > MAX_OUTPUT
            or not runner._positive_finite(binding_seconds) or binding_seconds > 3600
            or len(argv) > MAX_ARGV or len(env) > MAX_ENV or len(argv[0]) > MAX_PATH
            or any(len(item) > MAX_ARG for item in argv) or len(os.fspath(cwd)) > MAX_PATH):
        raise ValueError("observer input exceeds fixed bounds")
    for key in env:
        other_print = key.startswith("DYLD_PRINT_") and key != "DYLD_PRINT_LIBRARIES"
        if other_print or key in REJECTED_LOADER_KEYS:
            raise ValueError("child env must not redirect loader logging, inserted images or the shared cache")
    child_env = dict(env)
    child_env["DYLD_PRINT_LIBRARIES"] = "1"
    loader_env = {key: child_env[key] for key in sorted(child_env) if key.startswith("DYLD_")}
    return runner, child_env, loader_env


def _sysctl_string(name):
    libc = ctypes.CDLL(None, use_errno=True)
    call = libc.sysctlbyname
    call.argtypes = [ctypes.c_char_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t),
                     ctypes.c_void_p, ctypes.c_size_t]
    call.restype = ctypes.c_int
    size = ctypes.c_size_t(0)
    if call(name.encode("ascii"), None, ctypes.byref(size), None, 0) != 0 or size.value == 0:
        return None
    buffer = ctypes.create_string_buffer(size.value)
    if call(name.encode("ascii"), buffer, ctypes.byref(size), None, 0) != 0:
        return None
    text = buffer.raw[:size.value].split(b"\0", 1)[0].decode("utf-8", "replace").strip()
    return text or None


def _shared_cache_uuid():
    # Source-bound libdyld SPI; absence refuses this observer rather than guessing.
    try:
        call = ctypes.CDLL(None)._dyld_get_shared_cache_uuid
        call.argtypes = [ctypes.POINTER(ctypes.c_ubyte)]
        call.restype = ctypes.c_bool
        value = (ctypes.c_ubyte * 16)()
        return str(uuid_module.UUID(bytes=bytes(value))).upper() if call(value) else None
    except (AttributeError, OSError):
        return None


def _cache_contains_path(path):
    try:
        call = ctypes.CDLL(None)._dyld_shared_cache_contains_path
        call.argtypes = [ctypes.c_char_p]
        call.restype = ctypes.c_bool
        return call(os.fsencode(path))
    except (AttributeError, OSError):
        return False


def _system_identity():
    identity = {key: _sysctl_string(name) for key, name in _SYSCTL_NAMES.items()}
    identity["shared_cache_uuid"] = _shared_cache_uuid()
    identity["observer_architecture"] = platform.machine()
    if any(identity[key] is None for key in SYSTEM_KEYS):
        return None
    return identity


def _dyld_identity():
    try:
        with open(DYLD_PATH, "rb") as handle:
            slices = _macho_slices(handle, os.fstat(handle.fileno()).st_size)
    except (OSError, _Refusal):
        return None
    return {"path": DYLD_PATH,
            "slices": [{"arch": item["arch"], "uuid": item["uuid"]} for item in slices]}


def _cpu_name(cputype, cpusubtype):
    return _CPU_NAMES.get((cputype, cpusubtype & 0x00FFFFFF),
                          "cputype-%#x-%#x" % (cputype, cpusubtype))


def _read_at(handle, offset, length):
    handle.seek(offset)
    return handle.read(length)


def _macho_header(handle, offset, size):
    head = _read_at(handle, offset, 32)
    _require(len(head) >= 28, "macho_header_short")
    magic = head[:4]
    if magic == b"\xcf\xfa\xed\xfe":
        order, header_size = "<", 32
    elif magic == b"\xfe\xed\xfa\xcf":
        order, header_size = ">", 32
    elif magic == b"\xce\xfa\xed\xfe":
        order, header_size = "<", 28
    elif magic == b"\xfe\xed\xfa\xce":
        order, header_size = ">", 28
    else:
        raise _Refusal("macho_magic_unknown")
    _require(len(head) >= header_size, "macho_header_short")
    cputype, cpusubtype, filetype, ncmds, sizeofcmds = struct.unpack(
        order + "iIIII", head[4:24])
    _require(ncmds <= MAX_LOAD_COMMANDS and sizeofcmds <= MAX_COMMAND_BYTES
             and header_size + sizeofcmds <= size, "macho_load_commands_out_of_bounds")
    commands = _read_at(handle, offset + header_size, sizeofcmds)
    _require(len(commands) == sizeofcmds, "macho_load_commands_short")
    position = 0
    found = None
    for _ in range(ncmds):
        _require(position + 8 <= sizeofcmds, "macho_load_command_truncated")
        cmd, cmdsize = struct.unpack(order + "II", commands[position:position + 8])
        _require(cmdsize >= 8 and position + cmdsize <= sizeofcmds, "macho_load_command_size")
        if cmd == LC_UUID:
            _require(cmdsize == 24 and found is None, "macho_uuid_command_malformed")
            found = str(uuid_module.UUID(bytes=commands[position + 8:position + 24])).upper()
        position += cmdsize
    return {"uuid": found, "arch": _cpu_name(cputype, cpusubtype), "cputype": cputype,
            "cpusubtype": cpusubtype, "filetype": filetype, "slice_offset": offset,
            "slice_size": size}


def _macho_slices(handle, size):
    """Parses the header of every architecture slice; raises _Refusal when malformed."""
    head = _read_at(handle, 0, 8)
    _require(len(head) == 8, "macho_file_short")
    if head[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
        wide = head[:4] == b"\xca\xfe\xba\xbf"
        count = struct.unpack(">I", head[4:8])[0]
        _require(1 <= count <= MAX_SLICES, "macho_fat_count")
        entry = 32 if wide else 20
        table = _read_at(handle, 8, count * entry)
        _require(len(table) == count * entry, "macho_fat_table_short")
        slices = []
        for index in range(count):
            raw = table[index * entry:(index + 1) * entry]
            if wide:
                _, _, offset, length = struct.unpack(">iIQQ", raw[:24])
            else:
                _, _, offset, length = struct.unpack(">iIII", raw[:16])
            _require(offset >= 8 + count * entry and length >= 28
                     and offset + length <= size, "macho_fat_slice_out_of_bounds")
            slices.append(_macho_header(handle, offset, length))
        return slices
    return [_macho_header(handle, 0, size)]


def _file_identity(status):
    return {"size": status.st_size, "inode": status.st_ino, "device": status.st_dev,
            "mtime_ns": status.st_mtime_ns}


def _bind_file(path, expected_uuid, deadline, budget):
    """Binds one on-disk image to the logged UUID; returns a plain dict with ``bound``."""
    try:
        _require(expected_uuid is not None, "no_uuid")
        _require(time.monotonic() < deadline, "binding_deadline_exceeded")
        try:
            handle = open(path, "rb")
        except FileNotFoundError:
            raise _Refusal("file_missing") from None
        except OSError as exc:
            raise _Refusal("file_unreadable: " + (exc.strerror or type(exc).__name__)) from None
        with handle:
            fd = handle.fileno()
            before = os.fstat(fd)
            _require(stat.S_ISREG(before.st_mode), "not_regular_file")
            _require(before.st_size <= MAX_FILE, "file_too_large")
            slices = _macho_slices(handle, before.st_size)
            matches = [item for item in slices if item["uuid"] == expected_uuid]
            _require(len(matches) == 1, "uuid_mismatch")
            _require(budget["bytes"] >= before.st_size, "hash_byte_limit")
            budget["bytes"] -= before.st_size
            hasher = hashlib.sha256()
            remaining = before.st_size
            handle.seek(0)
            while remaining > 0:
                _require(time.monotonic() < deadline, "binding_deadline_exceeded")
                block = handle.read(min(CHUNK, remaining))
                _require(block, "file_shrank_during_hash")
                hasher.update(block)
                remaining -= len(block)
            digest = hasher.hexdigest()
            after = os.fstat(fd)
            _require(_file_identity(after) == _file_identity(before), "file_changed_during_binding")
            _require(time.monotonic() < deadline, "binding_deadline_exceeded")
        macho = {key: matches[0][key] for key in
                 ("uuid", "arch", "cputype", "cpusubtype", "filetype", "slice_offset")}
        return {"bound": True, "reason": None, "file": _file_identity(before),
                "sha256": digest, "macho": macho}
    except (_Refusal, OSError) as exc:
        reason = str(exc) if isinstance(exc, _Refusal) else "file_unreadable: " + (exc.strerror or type(exc).__name__)
        return {"bound": False, "reason": reason, "file": None, "sha256": None, "macho": None}


def _image_class(path):
    return "system" if (posixpath.normpath(path) == path and path.startswith(SYSTEM_PREFIXES)
                        and _cache_contains_path(path)) else "file"


def _parse_log(text, *, image_limit=MAX_IMAGES, event_limit=MAX_EVENTS):
    """Classifies every stderr line; the first closure-breaking fact becomes ``reason``."""
    parsed = {"pid": None, "main_executable": None, "events": [], "images": [],
              "informational": [], "unknown": [], "foreign_pids": [], "lines": 0,
              "application_stderr_lines": 0, "reason": None}
    lines = text.split("\n")
    terminated = text.endswith("\n")
    if terminated or lines[-1] == "":
        lines.pop()
    indexes = {}
    problems = []

    def unknown(number, line, why):
        if len(parsed["unknown"]) < MAX_RETAINED_UNKNOWN:
            parsed["unknown"].append({"line": number, "kind": why, "text": line[:MAX_PATH]})
        problems.append("unknown_record")

    for number, line in enumerate(lines, 1):
        parsed["lines"] = number
        last_unterminated = number == len(lines) and not terminated
        if not line.startswith("dyld["):
            if "dyld[" in line:
                unknown(number, line, "interleaved")
            else:
                parsed["application_stderr_lines"] += 1
            continue
        prefix = _PREFIX.match(line)
        if prefix is None or last_unterminated:
            unknown(number, line, "unterminated" if last_unterminated else "malformed_prefix")
            continue
        pid, body = int(prefix.group(1)), prefix.group(2)
        if parsed["pid"] is None:
            parsed["pid"] = pid
        if pid != parsed["pid"]:
            if pid not in parsed["foreign_pids"]:
                parsed["foreign_pids"].append(pid)
            problems.append("foreign_pid")
            continue
        load = _LOAD.match(body)
        if load is not None:
            uuid_text, path = load.group(1), load.group(2)
            uuid = None if uuid_text == "no uuid" else uuid_text
            if len(path) > MAX_PATH or posixpath.normpath(path) != path or "\ufffd" in path:
                unknown(number, line, "noncanonical_path")
                continue
            if parsed["main_executable"] is None:
                parsed["main_executable"] = path
            key = (path, uuid)
            index = indexes.get(key)
            if index is None:
                if len(parsed["images"]) >= image_limit:
                    problems.append("image_limit")
                    continue
                index = len(parsed["images"])
                indexes[key] = index
                parsed["images"].append({"path": path, "uuid": uuid})
            if len(parsed["events"]) >= event_limit:
                problems.append("event_limit")
                continue
            parsed["events"].append({"line": number, "pid": pid, "uuid": uuid, "path": path,
                                     "image": index})
            continue
        if parsed["main_executable"] is None:
            problems.append("main_executable_not_first")
        for kind, pattern in _INFORMATIONAL:
            match = pattern.match(body)
            if match is not None:
                parsed["informational"].append(
                    {"line": number, "kind": kind, "names": list(match.groups())})
                break
        else:
            unknown(number, line, "unrecognized_message")
    if parsed["pid"] is None:
        problems.insert(0, "loader_logging_unavailable")
    leaves = {os.path.basename(image["path"]) for image in parsed["images"]}
    for note in parsed["informational"]:
        if note["kind"] == "inserted_duplicate":
            problems.append("inserted_dylib_present")
        elif any(name not in leaves for name in note["names"]):
            problems.append("unknown_informational_reference")
    parsed["reason"] = problems[0] if problems else None
    return parsed


def validate_timing_boundaries(record, expected_rows):
    """Re-derive byte-order windows, independently of execution success and image binding.

    Zero records is not proof of zero loader activity or zero observer cost.
    """
    result = {"protocol": TIMING_PROTOCOL, "complete": False, "ok": False, "reason": None,
              "process_id": None, "dyld_lines": [], "rows": []}
    try:
        _require(isinstance(record, dict) and isinstance(record.get("stderr"), str),
                 "timing_record_invalid")
        pid = _root_pid(record)
        result["process_id"] = pid
        _require(pid is not None, "root_pid_unavailable")
        _require(isinstance(expected_rows, list) and all(
            isinstance(row, str) and re.fullmatch(r"[A-Za-z0-9-]{1,128}", row)
            for row in expected_rows) and len(set(expected_rows)) == len(expected_rows),
            "timing_expected_rows_invalid")
        text = record["stderr"]
        lines = text.split("\n")
        terminated = text.endswith("\n")
        if lines[-1] == "":
            lines.pop()
        result["dyld_lines"] = [number for number, line in enumerate(lines, 1) if "dyld[" in line]
        phases = ("row-start", "samples-begin", "samples-end")
        row_index, phase_index, positions = 0, 0, []
        windows = []
        for number, line in enumerate(lines, 1):
            if "mado-component-timing" not in line:
                continue
            _require(number != len(lines) or terminated, "timing_marker_unterminated")
            marker = _TIMING_MARKER.fullmatch(line)
            _require(marker is not None, "timing_marker_malformed_or_unsupported")
            marker_pid, row, phase = marker.groups()
            _require(marker_pid == str(pid), "timing_marker_pid_mismatch")
            _require(row_index < len(expected_rows) and row == expected_rows[row_index],
                     "timing_marker_row_mismatch")
            _require(phase == phases[phase_index], "timing_marker_transition_invalid")
            positions.append(number)
            phase_index += 1
            if phase_index == len(phases):
                windows.append((row, *positions))
                row_index += 1
                phase_index, positions = 0, []
        _require(row_index == len(expected_rows) and phase_index == 0, "timing_markers_missing")
        parsed = _parse_log(text)
        _require(parsed["reason"] is None, "timing_raw_log_not_closed: " + str(parsed["reason"]))
        authority = _authority_reason(record, parsed)
        _require(authority is None, authority)
        _require(record.get("timed_out") is False and record.get("output_limited") is False
                 and record.get("launch_error") is None and type(record.get("exit_code")) is int,
                 "timing_record_incomplete")
        for row, start, begin, end in windows:
            result["rows"].append({
                "id": row, "row_start_line": start, "samples_begin_line": begin, "samples_end_line": end,
                "pre_window_dyld_records": sum(line < begin for line in result["dyld_lines"]),
                "in_window_dyld_records": sum(begin < line < end for line in result["dyld_lines"]),
                "post_window_dyld_records": sum(line > end for line in result["dyld_lines"])})
        result["complete"] = True
        _require(not any(row["in_window_dyld_records"] for row in result["rows"]),
                 "timing_in_window_dyld_records")
        result["ok"] = True
    except _Refusal as exc:
        result["reason"] = str(exc)
    return result


def _main_identity(path):
    try:
        status = os.stat(path)
    except OSError as exc:
        raise _Refusal("main_executable_unreadable: " + (exc.strerror or type(exc).__name__))
    _require(stat.S_ISREG(status.st_mode), "main_executable_not_regular_file")
    return (status.st_dev, status.st_ino)


def _outcome_reason(record, log_reason, binding_reason):
    """Owned record facts first, then log closure, then binding, then the child's exit."""
    if record["launch_error"] is not None:
        return "launch_failed: " + str(record["launch_error"])
    if record["timed_out"]:
        return "timed_out"
    if record["output_limited"]:
        return "output_limited"
    if record["exit_code"] is None:
        return "child_exit_not_owned"
    if log_reason is not None:
        return log_reason
    if binding_reason is not None:
        return binding_reason
    if record["exit_code"] != 0:
        return "child_exit_%d" % record["exit_code"]
    return None


def _settle(receipt, record, parsed, binding_reason):
    """Fills the primary result from the owned record, parsed log and binding outcome."""
    result = receipt["result"]
    result["exit_code"] = record["exit_code"]
    result["observed_images"] = len(receipt["images"])
    result["observed_events"] = len(receipt["events"])
    reason = _outcome_reason(record, parsed["reason"], binding_reason)
    result["status"] = "complete" if reason is None else "incomplete"
    result["reason"] = reason


def _root_pid(record):
    """The runner-owned pid of the exec'd root, or None when the record lacks one."""
    process_id = record.get("process_id")
    return process_id if type(process_id) is int and process_id > 0 else None


def _authority_reason(record, parsed, loader=None):
    """Strict pid authority: runner process_id must exist and equal every dyld line's pid."""
    root_pid = _root_pid(record)
    if root_pid is None:
        return "root_pid_unavailable"
    if loader is not None and loader.get("root_pid") != root_pid:
        return "root_pid_mismatch"
    if parsed["pid"] is not None and parsed["pid"] != root_pid:
        return "root_pid_mismatch"
    return None


def _fill_loader(receipt, parsed, loader_env, record):
    authority = _authority_reason(record, parsed)
    if authority is not None:
        parsed["reason"] = authority
    receipt["loader"] = {"env": loader_env, "pid": parsed["pid"], "root_pid": _root_pid(record),
                         "main_executable": parsed["main_executable"], "lines": parsed["lines"],
                         "application_stderr_lines": parsed["application_stderr_lines"],
                         "informational": parsed["informational"], "unknown": parsed["unknown"],
                         "foreign_pids": parsed["foreign_pids"]}
    receipt["events"] = parsed["events"]


def _main_matches(parsed, main_identity):
    if parsed["main_executable"] is None:
        return False
    try:
        return _main_identity(parsed["main_executable"]) == main_identity
    except _Refusal:
        return False


def discover(argv, *, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds,
             binding_seconds=120.0):
    """Runs the exact child once, parses its dyld load inventory and binds every image.

    Non-acceptance: the dyld log writes perturb the run. Binding happens after
    owned exit within ``binding_seconds``; images whose file cannot be bound
    (missing, replaced, malformed, over limits) leave the result incomplete.
    """
    started = time.monotonic()
    receipt = _receipt("macos-runtime-image-discovery")
    try:
        runner, child_env, loader_env = _prepare(
            argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds, binding_seconds)
    except (ValueError, TypeError) as exc:
        return _refused(receipt, "invalid_arguments: " + str(exc), started)
    if sys.platform != "darwin":
        return _refused(receipt, "requires_macos", started)
    receipt["loader"]["env"] = loader_env
    try:
        main_identity = _main_identity(argv[0])
    except _Refusal as exc:
        return _refused(receipt, str(exc), started)
    receipt["system"] = _system_identity()
    receipt["dyld"] = _dyld_identity()
    if receipt["system"] is None:
        return _refused(receipt, "system_identity_unavailable", started)
    record = runner.run_process(argv, cwd=cwd, env=child_env, timeout_seconds=timeout_seconds,
                                output_limit_bytes=output_limit_bytes,
                                cleanup_seconds=cleanup_seconds)
    _complete_discovery(receipt, record, parsed=_parse_log(record["stderr"]),
                        main_identity=main_identity, loader_env=loader_env,
                        deadline=time.monotonic() + binding_seconds)
    receipt["duration_seconds"] = time.monotonic() - started
    return receipt


def _complete_discovery(receipt, record, *, parsed, main_identity, loader_env, deadline):
    receipt["record"] = record
    receipt["cleanup_ok"] = record["cleanup_ok"]
    _fill_loader(receipt, parsed, loader_env, record)
    if parsed["reason"] is None and not _main_matches(parsed, main_identity):
        parsed["reason"] = "main_executable_mismatch"
    budget = {"bytes": MAX_HASH_BYTES}
    hashed = 0
    unbound = []
    images = []
    for item in parsed["images"]:
        image = {"path": item["path"], "uuid": item["uuid"], "class": _image_class(item["path"])}
        if time.monotonic() >= deadline:
            image.update(bound=False, reason="binding_deadline_exceeded", file=None, sha256=None, macho=None)
        elif image["class"] == "system":
            image.update(bound=item["uuid"] is not None,
                         reason=None if item["uuid"] else "no_uuid", file=None, sha256=None, macho=None)
        else:
            image.update(_bind_file(item["path"], item["uuid"], deadline, budget))
            if image["bound"]:
                hashed += image["file"]["size"]
        if not image["bound"]:
            unbound.append({"path": image["path"], "uuid": image["uuid"],
                            "reason": image["reason"]})
        images.append(image)
    receipt["images"] = images
    receipt["result"]["unbound"] = unbound
    if images and images[0]["bound"]:
        receipt["target"] = _target_of(images[0])
    deadline_exceeded = any(item["reason"] == "binding_deadline_exceeded" for item in unbound)
    arch_matches = receipt["target"] == TARGETS.get(receipt["system"]["observer_architecture"])
    receipt["provenance"] = {
        "raw_retained": not record["output_limited"],
        "binding": {"files_hashed": sum(1 for image in images if image["sha256"]),
                    "bytes_hashed": hashed, "deadline_exceeded": deadline_exceeded},
        "complete": (not record["output_limited"] and not record["timed_out"] and not unbound
                     and parsed["reason"] is None and arch_matches and receipt["dyld"] is not None),
    }
    binding_reason = "image_unbound" if unbound else None
    if binding_reason is None and not arch_matches:
        binding_reason = "observer_architecture_mismatch"
    _settle(receipt, record, parsed, binding_reason)


def _target_of(image):
    macho = image.get("macho")
    if not macho:
        return None
    return TARGETS.get(macho["arch"], macho["arch"] + "-apple-darwin")


def _check_str(value, reason, *, pattern=None):
    _require(isinstance(value, str) and value and "\0" not in value
             and (pattern is None or pattern.match(value)), reason)
    return value


def _check_int(value, reason):
    _require(type(value) is int, reason)
    return value


def _check_image_binding(image):
    """Validates one bound image record and returns its frozen closure row."""
    _require(isinstance(image, dict) and image.get("bound") is True, "image_unbound")
    path = _check_str(image.get("path"), "image_path_invalid")
    _require(path.startswith("/") and len(path) <= MAX_PATH and posixpath.normpath(path) == path,
             "image_path_invalid")
    uuid = _check_str(image.get("uuid"), "image_uuid_invalid", pattern=_UUID)
    klass = image.get("class")
    _require(klass in ("file", "system") and (klass == "file" or path.startswith(SYSTEM_PREFIXES)),
             "image_class_invalid")
    row = {"path": path, "uuid": uuid, "class": klass}
    if klass == "system":
        return row
    file_identity = image.get("file")
    _require(isinstance(file_identity, dict), "image_file_identity_missing")
    row["file"] = {key: _check_int(file_identity.get(key), "image_file_identity_invalid")
                   for key in ("size", "inode", "device", "mtime_ns")}
    row["sha256"] = _check_str(image.get("sha256"), "image_sha256_invalid", pattern=_HEX64)
    macho = image.get("macho")
    _require(isinstance(macho, dict) and macho.get("uuid") == uuid, "image_macho_uuid_mismatch")
    row["macho"] = {"uuid": uuid, "arch": _check_str(macho.get("arch"), "image_arch_invalid")}
    for key in ("cputype", "cpusubtype", "filetype", "slice_offset"):
        row["macho"][key] = _check_int(macho.get(key), "image_macho_invalid")
    return row


def _check_loader_env(env):
    _require(isinstance(env, dict) and all(
        isinstance(key, str) and isinstance(value, str) and key.startswith("DYLD_")
        for key, value in env.items()), "loader_env_invalid")
    _require(env.get("DYLD_PRINT_LIBRARIES") == "1", "loader_env_print_libraries_missing")
    _require(not any((key.startswith("DYLD_PRINT_") and key != "DYLD_PRINT_LIBRARIES")
                     or key in REJECTED_LOADER_KEYS for key in env), "loader_env_invalid")
    return dict(sorted(env.items()))


def _check_system(system):
    _require(isinstance(system, dict), "system_identity_invalid")
    result = {key: _check_str(system.get(key), "system_identity_invalid") for key in SYSTEM_KEYS}
    _check_str(result["shared_cache_uuid"], "shared_cache_uuid_invalid", pattern=_UUID)
    _require(result["observer_architecture"] in TARGETS, "observer_architecture_unsupported")
    return result


def _check_dyld(dyld):
    _require(isinstance(dyld, dict) and dyld.get("path") == DYLD_PATH
             and isinstance(dyld.get("slices"), list) and dyld["slices"], "dyld_identity_invalid")
    slices = []
    for item in dyld["slices"]:
        _require(isinstance(item, dict), "dyld_identity_invalid")
        slices.append({"arch": _check_str(item.get("arch"), "dyld_identity_invalid"),
                       "uuid": _check_str(item.get("uuid"), "dyld_identity_invalid")})
    return {"path": DYLD_PATH, "slices": slices}


def _reparse_receipt(receipt, *, require_success=True):
    """Re-derive discovery provenance; qualification also requires a successful exit."""
    _require(isinstance(receipt, dict), "receipt_invalid")
    _require(receipt.get("format_version") == FORMAT_VERSION
             and receipt.get("kind") == "macos-runtime-image-discovery"
             and receipt.get("method") == METHOD and receipt.get("lane") == LANE,
             "receipt_kind_invalid")
    _require(receipt.get("source") == SOURCE, "receipt_source_mismatch")
    record = receipt.get("record")
    _require(isinstance(record, dict), "record_invalid")
    _require("launch_error" in record and record["launch_error"] is None
             and record.get("timed_out") is False and record.get("output_limited") is False
             and type(record.get("exit_code")) is int and (not require_success or record["exit_code"] == 0),
             "record_not_successful")
    _require(isinstance(record.get("stderr"), str), "record_stderr_missing")
    _require(type(record.get("cleanup_ok")) is bool, "record_cleanup_invalid")
    duration = record.get("duration_seconds")
    _require(type(duration) in (int, float) and 0 <= duration < float("inf"), "record_duration_invalid")
    loader = receipt.get("loader")
    _require(isinstance(loader, dict), "loader_invalid")
    loader_env = _check_loader_env(loader.get("env"))
    parsed = _parse_log(record["stderr"])
    _require(parsed["reason"] is None, "raw_log_not_closed: " + str(parsed["reason"]))
    _require(parsed["pid"] == loader.get("pid")
             and parsed["main_executable"] == loader.get("main_executable"),
             "loader_summary_mismatch")
    authority = _authority_reason(record, parsed, loader)
    _require(authority is None, authority)
    _require(parsed["events"] == receipt.get("events"), "events_mismatch")
    images = receipt.get("images")
    _require(isinstance(images, list) and len(images) == len(parsed["images"])
             and all(isinstance(image, dict)
                     and image.get("path") == expected["path"]
                     and image.get("uuid") == expected["uuid"]
                     for image, expected in zip(images, parsed["images"])), "images_mismatch")
    rows = []
    for image in images:
        try:
            rows.append(_check_image_binding(image))
        except _Refusal as exc:
            raise _Refusal("%s: %s" % (exc, image.get("path"))) from None
    _require(rows, "no_images")
    main_row = rows[0]
    _require(main_row["class"] == "file" and main_row["path"] == parsed["main_executable"],
             "main_executable_unbound")
    target = _target_of({"macho": main_row["macho"]})
    _require(receipt.get("target") == target, "target_mismatch")
    system = _check_system(receipt.get("system"))
    _require(target == TARGETS.get(system["observer_architecture"]),
             "observer_architecture_mismatch")
    return {
        "target": target, "main_executable": parsed["main_executable"],
        "loader_env": loader_env, "system": system,
        "dyld": _check_dyld(receipt.get("dyld")), "images": rows,
        "origin": {"receipt_sha256": _digest(receipt, "receipt_not_serializable"),
                   "pid": parsed["pid"],
                   "events": len(parsed["events"]), "cleanup_ok": record["cleanup_ok"],
                   "record_duration_seconds": record.get("duration_seconds")},
    }


def validate_discovery(receipt, *, preceding=()):
    """Read-only provenance and cleanup verdict, including an owned nonzero exit; no qualification."""
    try:
        identity = _reparse_receipt(receipt, require_success=False)
        identities = [*(_reparse_receipt(item) for item in preceding), identity]
        _union(identities)
    except _Refusal as exc:
        return {"provenance_complete": False, "reason": str(exc), "identity": None,
                "cleanup_ok": None, "exit_code": None}
    record = receipt["record"]
    return {"provenance_complete": True, "reason": None, "identity": identity,
            "cleanup_ok": all(item["origin"]["cleanup_ok"] for item in identities),
            "exit_code": record["exit_code"]}


def _union(derived):
    first = derived[0]
    for key in ("target", "main_executable", "loader_env", "system", "dyld"):
        _require(all(item[key] == first[key] for item in derived), key + "_differs")
    rows = {}
    by_path = {}
    for item in derived:
        for row in item["images"]:
            key = (row["path"], row["uuid"])
            known = rows.get(key)
            _require(known is None or known == row, "conflicting_file_binding: " + row["path"])
            _require(by_path.get(row["path"], row["uuid"]) == row["uuid"],
                     "conflicting_image_uuid: " + row["path"])
            by_path[row["path"]] = row["uuid"]
            rows.setdefault(key, row)
    images = list(rows.values())
    _require(len(images) <= MAX_IMAGES, "image_limit")
    return {
        "format_version": FORMAT_VERSION, "kind": "macos-runtime-image-closure", "lane": LANE,
        "method": METHOD, "source": SOURCE, "target": first["target"],
        "main_executable": first["main_executable"], "loader_env": first["loader_env"],
        "system": first["system"], "dyld": first["dyld"], "images": images,
        "origins": [item["origin"] for item in derived],
    }


def validate_closure(receipts):
    """Derives the frozen closure from one or more discovery receipts' raw records.

    Retained data only: no launch, no live file or host reads. Each receipt is
    re-parsed from its retained stderr; summaries and status fields are checked
    against that re-derivation, never trusted. Receipts must share target, main
    executable, loader environment and system and dyld identities, and a path
    may bind to one UUID with one file identity.
    """
    try:
        items = receipts if isinstance(receipts, list) else [receipts]
        _require(items, "no_receipts")
        closure = _union([_reparse_receipt(item) for item in items])
    except _Refusal as exc:
        return {"ok": False, "reason": str(exc), "closure": None}
    return {"ok": True, "reason": None, "closure": closure}


CLOSURE_KEYS = frozenset((
    "format_version", "kind", "lane", "method", "source", "target", "main_executable",
    "loader_env", "system", "dyld", "images", "origins",
))


def _check_closure(closure):
    """Validates closure structure and returns indexed rows; never trusts flags."""
    _require(isinstance(closure, dict) and set(closure) == CLOSURE_KEYS
             and closure["format_version"] == FORMAT_VERSION
             and closure["kind"] == "macos-runtime-image-closure"
             and closure["lane"] == LANE and closure["method"] == METHOD
             and closure["source"] == SOURCE, "closure_kind_invalid")
    main = _check_str(closure["main_executable"], "closure_main_executable_invalid")
    loader_env = _check_loader_env(closure["loader_env"])
    system = _check_system(closure["system"])
    dyld = _check_dyld(closure["dyld"])
    target = _check_str(closure["target"], "closure_target_invalid")
    images = closure["images"]
    _require(isinstance(images, list) and 0 < len(images) <= MAX_IMAGES, "closure_images_invalid")
    _require(isinstance(closure["origins"], list) and closure["origins"], "closure_origins_invalid")
    for origin in closure["origins"]:
        _require(isinstance(origin, dict) and set(origin) == {
            "receipt_sha256", "pid", "events", "cleanup_ok", "record_duration_seconds"},
            "closure_origin_invalid")
        _check_str(origin["receipt_sha256"], "closure_origin_invalid", pattern=_HEX64)
        _require(type(origin["pid"]) is int and origin["pid"] > 0
                 and type(origin["events"]) is int and 0 < origin["events"] <= MAX_EVENTS
                 and type(origin["cleanup_ok"]) is bool
                 and type(origin["record_duration_seconds"]) in (int, float)
                 and 0 <= origin["record_duration_seconds"] < float("inf"), "closure_origin_invalid")
    rows = {}
    paths = set()
    for image in images:
        _require(isinstance(image, dict), "closure_image_row_invalid")
        row = _check_image_binding(dict(image, bound=True))
        _require(image == row, "closure_image_row_invalid: " + row["path"])
        _require(row["path"] not in paths, "closure_image_duplicate: " + row["path"])
        paths.add(row["path"])
        rows[(row["path"], row["uuid"])] = row
    main_rows = [row for row in rows.values() if row["path"] == main]
    _require(len(main_rows) == 1 and main_rows[0]["class"] == "file",
             "closure_main_executable_unbound")
    _require(_target_of(main_rows[0]) == target, "closure_target_mismatch")
    _require(target == TARGETS.get(system["observer_architecture"]), "observer_architecture_mismatch")
    digest = _digest(closure, "closure_not_serializable")
    return {"main_executable": main, "loader_env": loader_env, "system": system, "dyld": dyld,
            "target": target, "rows": rows, "sha256": digest}


def _recheck_files(rows, deadline, label):
    """Rebinds every on-disk closure row; returns the retained records and first mismatch."""
    budget = {"bytes": MAX_HASH_BYTES}
    bindings = []
    reason = None
    for row in rows:
        if row["class"] != "file":
            continue
        current = _bind_file(row["path"], row["uuid"], deadline, budget)
        bindings.append({"path": row["path"], "uuid": row["uuid"], **current})
        if not current["bound"]:
            reason = "%s_%s: %s" % (label, current["reason"], row["path"])
        elif _binding_differs(current, row):
            reason = "%s_file_changed: %s" % (label, row["path"])
        if reason is not None:
            break
    return bindings, reason


def _binding_differs(current, row):
    return (current.get("sha256") != row["sha256"] or current.get("file") != row["file"]
            or current.get("macho") != row["macho"])


def _check_bindings(records, rows, label):
    """Every on-disk row must carry exactly one retained binding equal to the closure row."""
    _require(isinstance(records, list), label + "_bindings_invalid")
    expected = {row["path"]: row for row in rows if row["class"] == "file"}
    seen = set()
    for item in records:
        _require(isinstance(item, dict), label + "_bindings_invalid")
        row = expected.get(_check_str(item.get("path"), label + "_binding_path_invalid"))
        _require(row is not None and row["path"] not in seen, label + "_binding_unexpected")
        seen.add(row["path"])
        _require(item.get("bound") is True and item.get("uuid") == row["uuid"]
                 and not _binding_differs(item, row), "%s_binding_mismatch: %s" % (label, row["path"]))
    _require(seen == set(expected), label + "_bindings_incomplete")
    return len(seen)


def observe(argv, *, cwd, env, closure, timeout_seconds, output_limit_bytes, cleanup_seconds,
            binding_seconds=120.0):
    """Runs the exact child under the frozen closure and reconciles every actual load.

    Before launch every closure file is re-bound (identity, SHA-256, Mach-O
    UUID) and the system/dyld identity must equal the closure's. Each observed
    image must be in the closure with the same path and UUID; unobserved
    closure images are allowed (subset), new or changed images are not. After
    owned exit the observed on-disk images are re-bound again. ``result`` is
    the primary outcome; ``provenance`` and ``cleanup_ok`` are reported apart.
    """
    started = time.monotonic()
    receipt = _receipt("macos-runtime-image-observation")
    receipt["closure_sha256"] = None
    try:
        runner, child_env, loader_env = _prepare(
            argv, cwd, env, timeout_seconds, output_limit_bytes, cleanup_seconds, binding_seconds)
    except (ValueError, TypeError) as exc:
        return _refused(receipt, "invalid_arguments: " + str(exc), started)
    try:
        frozen = _check_closure(closure)
    except _Refusal as exc:
        return _refused(receipt, "closure_invalid: " + str(exc), started)
    receipt["closure_sha256"] = frozen["sha256"]
    receipt["target"] = frozen["target"]
    receipt["loader"]["env"] = loader_env
    if sys.platform != "darwin":
        return _refused(receipt, "requires_macos", started)
    try:
        main_identity = _main_identity(argv[0])
        _require(frozen["loader_env"] == loader_env, "loader_env_differs")
        _require(_main_identity(frozen["main_executable"]) == main_identity,
                 "main_executable_differs")
        receipt["system"] = _system_identity()
        receipt["dyld"] = _dyld_identity()
        _require(receipt["system"] is not None, "system_identity_unavailable")
        _require(receipt["system"] == frozen["system"], "system_identity_differs")
        _require(receipt["dyld"] == frozen["dyld"], "dyld_identity_differs")
        _require(frozen["target"] == TARGETS.get(platform.machine()), "observer_architecture_mismatch")
        pre_bindings, pre_reason = _recheck_files(
            frozen["rows"].values(), time.monotonic() + binding_seconds, "pre")
        receipt["provenance"]["pre"] = {"checked": len(pre_bindings), "reason": pre_reason,
                                        "bindings": pre_bindings}
        _require(pre_reason is None, pre_reason)
    except _Refusal as exc:
        return _refused(receipt, str(exc), started)
    record = runner.run_process(argv, cwd=cwd, env=child_env, timeout_seconds=timeout_seconds,
                                output_limit_bytes=output_limit_bytes,
                                cleanup_seconds=cleanup_seconds)
    _complete_observation(receipt, record, parsed=_parse_log(record["stderr"]), frozen=frozen,
                          main_identity=main_identity, loader_env=loader_env,
                          deadline=time.monotonic() + binding_seconds)
    receipt["duration_seconds"] = time.monotonic() - started
    return receipt


def _reconcile(parsed_images, rows):
    """Maps observed (path, uuid) images onto closure rows; subset allowed, new refused."""
    order = {key: index for index, key in enumerate(rows)}
    by_path = {path for path, _ in rows}
    images = []
    unbound = []
    for item in parsed_images:
        path, uuid = item["path"], item["uuid"]
        row = rows.get((path, uuid))
        image = {"path": path, "uuid": uuid, "class": row["class"] if row else None,
                 "closure_index": None, "bound": False, "reason": None}
        if uuid is None:
            image["reason"] = "no_uuid"
        elif (path, uuid) in order:
            image["bound"] = True
            image["closure_index"] = order[(path, uuid)]
        elif path in by_path:
            image["reason"] = "uuid_changed"
        else:
            image["reason"] = "image_not_in_closure"
        if not image["bound"]:
            unbound.append({"path": path, "uuid": uuid, "reason": image["reason"]})
        images.append(image)
    return images, unbound


def _complete_observation(receipt, record, *, parsed, frozen, main_identity, loader_env,
                          deadline):
    receipt["record"] = record
    receipt["cleanup_ok"] = record["cleanup_ok"]
    _fill_loader(receipt, parsed, loader_env, record)
    if parsed["reason"] is None and not _main_matches(parsed, main_identity):
        parsed["reason"] = "main_executable_mismatch"
    images, unbound = _reconcile(parsed["images"], frozen["rows"])
    receipt["images"] = images
    receipt["result"]["unbound"] = unbound
    observed_rows = [frozen["rows"][(image["path"], image["uuid"])]
                     for image in images if image["bound"]]
    post_bindings, post_reason = _recheck_files(observed_rows, deadline, "post")
    receipt["provenance"]["post"] = {"checked": len(post_bindings), "reason": post_reason,
                                     "bindings": post_bindings}
    receipt["provenance"]["raw_retained"] = not record["output_limited"]
    receipt["provenance"]["complete"] = (
        not record["output_limited"] and not record["timed_out"] and not unbound
        and parsed["reason"] is None and post_reason is None
        and receipt["provenance"]["pre"]["reason"] is None)
    _settle(receipt, record, parsed, "image_unbound" if unbound else post_reason)


def validate_observation(receipt, closure):
    """Re-derive loader provenance, child outcome and cleanup independently.

    ``ok`` still requires exit 0. Complete provenance can accompany a nonzero
    exit; neither verdict implies successful cleanup. No live reads occur.
    """
    cleanup_ok = None
    try:
        frozen = _check_closure(closure)
        _require(isinstance(receipt, dict) and receipt.get("format_version") == FORMAT_VERSION
                 and receipt.get("kind") == "macos-runtime-image-observation"
                 and receipt.get("method") == METHOD and receipt.get("lane") == LANE
                 and receipt.get("source") == SOURCE, "receipt_kind_invalid")
        _require(receipt.get("closure_sha256") == frozen["sha256"], "closure_sha256_mismatch")
        _require(receipt.get("target") == frozen["target"], "target_mismatch")
        _require(receipt.get("system") == frozen["system"], "system_identity_mismatch")
        _require(receipt.get("dyld") == frozen["dyld"], "dyld_identity_mismatch")
        loader = receipt.get("loader")
        _require(isinstance(loader, dict), "loader_invalid")
        _require(_check_loader_env(loader.get("env")) == frozen["loader_env"],
                 "loader_env_mismatch")
        record = receipt.get("record")
        _require(isinstance(record, dict) and isinstance(record.get("stderr"), str)
                 and "launch_error" in record
                 and type(record.get("timed_out")) is bool
                 and type(record.get("output_limited")) is bool
                 and type(record.get("cleanup_ok")) is bool
                 and (record.get("exit_code") is None or type(record.get("exit_code")) is int),
                 "record_invalid")
        cleanup_ok = record["cleanup_ok"]
        parsed = _parse_log(record["stderr"])
        _require(parsed["pid"] == loader.get("pid")
                 and parsed["main_executable"] == loader.get("main_executable"),
                 "loader_summary_mismatch")
        authority = _authority_reason(record, parsed, loader)
        _require(authority is None, authority)
        _require(parsed["events"] == receipt.get("events"), "events_mismatch")
        images, unbound = _reconcile(parsed["images"], frozen["rows"])
        _require(images == receipt.get("images"), "images_mismatch")
        if parsed["reason"] is None and parsed["main_executable"] != frozen["main_executable"]:
            parsed["reason"] = "main_executable_mismatch"
        provenance = receipt.get("provenance")
        _require(isinstance(provenance, dict) and isinstance(provenance.get("pre"), dict)
                 and isinstance(provenance.get("post"), dict), "provenance_invalid")
        _check_bindings(provenance["pre"].get("bindings"), frozen["rows"].values(), "pre")
        observed_rows = [frozen["rows"][(image["path"], image["uuid"])]
                         for image in images if image["bound"]]
        _check_bindings(provenance["post"].get("bindings"), observed_rows, "post")
        reason = _outcome_reason(record, parsed["reason"], "image_unbound" if unbound else None)
        complete = (record["launch_error"] is None and record["exit_code"] is not None
                    and not record["timed_out"] and not record["output_limited"]
                    and parsed["reason"] is None and not unbound)
    except _Refusal as exc:
        return {"ok": False, "reason": str(exc), "provenance_complete": False, "cleanup_ok": cleanup_ok}
    return {"ok": reason is None, "reason": reason, "provenance_complete": complete,
            "cleanup_ok": cleanup_ok, "observed_images": len(images),
            "observed_events": len(parsed["events"])}
