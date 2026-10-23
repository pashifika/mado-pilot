"""Load-log closure, Mach-O binding, closure derivation and refusal boundaries.

Deterministic tests use synthetic logs and synthetic Mach-O files only. The
transient-dylib smoke compiles a throwaway loader with ``cc`` in a temporary
directory and runs the real helper; it is opt-in on macOS:

    MADO_MACOS_IMAGE_OBSERVER_SMOKE=1 python3 -m unittest discover \\
        -s tools/tests -p test_macos_image_observer.py -v
"""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import unittest
import uuid as uuid_module

from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "macos_image_observer",
    Path(__file__).resolve().parents[1] / "native-release-profile/macos_image_observer.py",
)
observer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(observer)

ARM64 = 0x0100000C
X86_64 = 0x01000007
UUID_A = "7179E75C-3DE4-3C2D-B14E-A85F8A4E0BCE"
UUID_B = "3F34D467-DCC9-311D-B99A-44FC7F5DBAD3"
UUID_C = "4FDC9AA6-B344-37FE-B8C6-A4C94A038F57"
UUID_D = "D94BD0C0-BE2C-3A1B-9D01-FD318CC6439F"
SYSTEM_IMAGE = "/usr/lib/mado-observer-test/libFake.dylib"
SYSTEM = {"os_product_version": "27.0.1", "os_build": "26A434", "kernel_uuid": UUID_C,
          "kernel_version": "Darwin Kernel Version 27.0.0", "hardware_machine": "arm64",
          "hardware_model": "MacBookPro18,3", "cpu_brand": "Apple M1 Pro",
          "shared_cache_uuid": UUID_D, "observer_architecture": "arm64"}
DYLD = {"path": "/usr/lib/dyld", "slices": [{"arch": "arm64e", "uuid": UUID_D}]}
LOADER_ENV = {"DYLD_LIBRARY_PATH": "/opt/lib", "DYLD_PRINT_LIBRARIES": "1"}


def macho(uuid_text, *, cputype=ARM64, cpusubtype=0, filetype=6, order="<", commands=None,
          ncmds=1):
    if commands is None:
        commands = struct.pack(order + "II", 0x1B, 24) + uuid_module.UUID(uuid_text).bytes
    header = struct.pack(order + "IiIIIIII", 0xFEEDFACF, cputype, cpusubtype, filetype,
                         ncmds, len(commands), 0, 0)
    return header + commands + b"\0" * 64


def fat(*slices):
    table_size = 8 + 20 * len(slices)
    offset = table_size
    entries = b""
    payload = b""
    for cputype, blob in slices:
        entries += struct.pack(">iIIII", cputype, 0, offset, len(blob), 0)
        payload += blob
        offset += len(blob)
    return struct.pack(">II", 0xCAFEBABE, len(slices)) + entries + payload


def load_line(pid, uuid_text, path):
    return "dyld[%d]: <%s> %s\n" % (pid, uuid_text or "no uuid", path)


def record(stderr, **overrides):
    value = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
             "stdout": "", "stderr": stderr, "duration_seconds": 0.1, "launch_error": None,
             "process_id": 7}
    value.update(overrides)
    return value


def timing_line(row, phase, pid=7):
    return f"mado-component-timing-v1[{pid}]: {row} {phase}\n"


def timing_row(row, *, inside="", before="", after=""):
    return (timing_line(row, "row-start") + before + timing_line(row, "samples-begin")
            + inside + timing_line(row, "samples-end") + after)


class TimingBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.startup = load_line(7, UUID_A, "/app/main") + load_line(7, UUID_B, "/opt/lib/liba.dylib")

    def validate(self, text, rows=None, **overrides):
        return observer.validate_timing_boundaries(record(text, **overrides), ["row-a"] if rows is None else rows)

    def test_warm_and_late_records_are_located_outside_the_sample_window(self):
        delayed = "dyld[7]: move loaded to delayed: liba.dylib\n"
        text = self.startup + timing_row("row-a", before=delayed, after=delayed)
        result = self.validate(text)
        self.assertTrue(result["ok"], result["reason"])
        self.assertEqual(result["dyld_lines"], [1, 2, 4, 7])
        self.assertEqual(result["rows"], [{
            "id": "row-a", "row_start_line": 3, "samples_begin_line": 5, "samples_end_line": 6,
            "pre_window_dyld_records": 3, "in_window_dyld_records": 0, "post_window_dyld_records": 1}])

    def test_transient_duplicate_loads_and_informational_records_each_block_the_window(self):
        duplicate = load_line(7, UUID_B, "/opt/lib/liba.dylib")
        note = "dyld[7]: move loaded to delayed: liba.dylib\n"
        for inside, count in ((duplicate, 1), (duplicate * 2, 2), (note, 1), (duplicate * 2 + note, 3)):
            with self.subTest(inside=inside):
                text = self.startup + timing_row("row-a", inside=inside)
                result = self.validate(text)
                self.assertTrue(result["complete"])
                self.assertFalse(result["ok"])
                self.assertEqual(result["reason"], "timing_in_window_dyld_records")
                self.assertEqual(result["rows"][0]["in_window_dyld_records"], count)
                self.assertEqual(len(observer._parse_log(text)["images"]), 2)

    def test_invalid_transitions_never_become_empty_windows(self):
        start = timing_line("row-a", "row-start")
        begin = timing_line("row-a", "samples-begin")
        end = timing_line("row-a", "samples-end")
        for markers in ("", start, start + begin, begin + end, start + end + begin,
                        start + start + begin + end, start + begin + begin + end,
                        start + begin + end + end, start + begin + end + start,
                        start + timing_row("row-b") + begin + end):
            with self.subTest(markers=markers):
                result = self.validate(self.startup + markers)
                self.assertFalse(result["complete"])
                self.assertFalse(result["ok"])
                self.assertEqual(result["rows"], [])

    def test_pid_row_and_wire_corruption_are_incomplete(self):
        valid = timing_row("row-a")
        for markers in (
                valid.replace("[7]", "[8]"), valid.replace("[7]", "[07]"),
                valid.replace("row-a", "row-b"), valid.replace("-v1[", "-v2["),
                valid.replace(": row-a", ": row_a"), valid.replace("row-a", "a" * 129),
                valid.replace("row-start\n", "row-start app text\n"),
                valid.replace("mado-component", "app mado-component", 1),
                valid.replace("row-start\n", "row-start" + load_line(7, UUID_B, "/opt/lib/liba.dylib")),
                valid[:-1], valid.replace("\n", "\r\n")):
            with self.subTest(markers=markers):
                result = self.validate(self.startup + markers)
                self.assertFalse(result["complete"])
                self.assertFalse(result["ok"])
        for pid in (None, True, 0, 8):
            with self.subTest(process_id=pid):
                self.assertFalse(self.validate(self.startup + valid, process_id=pid)["complete"])

    def test_rows_must_match_the_entire_expected_sequence(self):
        text = self.startup + timing_row("row-a") + timing_row("row-b")
        self.assertTrue(self.validate(text, ["row-a", "row-b"])["ok"])
        for rows in (["row-b", "row-a"], ["row-a"], ["row-a", "row-b", "row-c"], ["row-a", "row-a"], []):
            with self.subTest(rows=rows):
                self.assertFalse(self.validate(text, rows)["complete"])
        self.assertTrue(self.validate(self.startup, [])["ok"])

    def test_loader_unknown_foreign_pid_and_truncated_output_remain_incomplete(self):
        for inside in ("dyld[7]: unknown event\n", load_line(8, UUID_B, "/opt/lib/liba.dylib"),
                       "app dyld[7]: interleaved\n"):
            with self.subTest(inside=inside):
                self.assertFalse(self.validate(self.startup + timing_row("row-a", inside=inside))["complete"])
        for overrides in ({"timed_out": True}, {"output_limited": True}, {"launch_error": "failed"}):
            with self.subTest(overrides=overrides):
                self.assertFalse(self.validate(self.startup + timing_row("row-a"), **overrides)["complete"])


class ParseTests(unittest.TestCase):
    def test_load_lines_and_delay_notes_close_the_inventory(self):
        text = (load_line(7, UUID_A, "/app/main") + load_line(7, UUID_B, "/opt/lib/liba.dylib")
                + load_line(7, UUID_C, "/usr/lib/libSystem.B.dylib")
                + "dyld[7]: move loaded to delayed: liba.dylib\n"
                + "dyld[7]: libSystem.B.dylib has weak-def (or flat lookup) symbol used by "
                  "liba.dylib, so cannot be delayed\n")
        parsed = observer._parse_log(text)
        self.assertIsNone(parsed["reason"])
        self.assertEqual(parsed["pid"], 7)
        self.assertEqual(parsed["main_executable"], "/app/main")
        self.assertEqual([event["image"] for event in parsed["events"]], [0, 1, 2])
        self.assertEqual(len(parsed["images"]), 3)
        self.assertEqual([note["kind"] for note in parsed["informational"]],
                         ["delayed", "weak_def"])
        self.assertEqual(parsed["lines"], 5)

    def test_reload_after_unmap_is_a_second_event_of_the_same_image(self):
        text = load_line(7, UUID_A, "/app/main") + load_line(7, UUID_B, "/t/lib.dylib") * 2
        parsed = observer._parse_log(text)
        self.assertIsNone(parsed["reason"])
        self.assertEqual(len(parsed["images"]), 1 + 1)
        self.assertEqual([event["image"] for event in parsed["events"]], [0, 1, 1])

    def test_same_path_with_new_uuid_is_a_distinct_image(self):
        text = (load_line(7, UUID_A, "/app/main") + load_line(7, UUID_B, "/t/lib.dylib")
                + load_line(7, UUID_C, "/t/lib.dylib"))
        parsed = observer._parse_log(text)
        self.assertEqual([image["uuid"] for image in parsed["images"]], [UUID_A, UUID_B, UUID_C])

    def test_unrecognized_dyld_message_breaks_closure(self):
        text = load_line(7, UUID_A, "/app/main") + "dyld[7]: terminating because of x\n"
        parsed = observer._parse_log(text)
        self.assertEqual(parsed["reason"], "unknown_record")
        self.assertEqual(parsed["unknown"][0]["line"], 2)
        self.assertEqual(len(parsed["events"]), 1)

    def test_foreign_pid_lines_are_not_claimed(self):
        text = (load_line(7, UUID_A, "/app/main") + load_line(9, UUID_B, "/app/child")
                + load_line(7, UUID_C, "/usr/lib/libSystem.B.dylib"))
        parsed = observer._parse_log(text)
        self.assertEqual(parsed["reason"], "foreign_pid")
        self.assertEqual(parsed["foreign_pids"], [9])
        self.assertEqual([event["path"] for event in parsed["events"]],
                         ["/app/main", "/usr/lib/libSystem.B.dylib"])

    def test_absent_loader_output_is_unsupported_not_empty(self):
        parsed = observer._parse_log("warning: application text\nmore\n")
        self.assertEqual(parsed["reason"], "loader_logging_unavailable")
        self.assertEqual(parsed["application_stderr_lines"], 2)
        self.assertEqual(parsed["events"], [])

    def test_interleaved_and_unterminated_lines_are_unknown(self):
        interleaved = load_line(7, UUID_A, "/app/main") + "app dyld[7]: <x> /y\n"
        self.assertEqual(observer._parse_log(interleaved)["reason"], "unknown_record")
        unterminated = load_line(7, UUID_A, "/app/main") + load_line(7, UUID_B, "/t/l")[:-1]
        parsed = observer._parse_log(unterminated)
        self.assertEqual(parsed["reason"], "unknown_record")
        self.assertEqual(parsed["unknown"][0]["kind"], "unterminated")
        self.assertEqual(len(parsed["events"]), 1)

    def test_informational_note_must_reference_a_logged_image(self):
        text = load_line(7, UUID_A, "/app/main") + "dyld[7]: move loaded to delayed: Ghost\n"
        self.assertEqual(observer._parse_log(text)["reason"], "unknown_informational_reference")
        early = "dyld[7]: move loaded to delayed: main\n" + load_line(7, UUID_A, "/app/main")
        self.assertEqual(observer._parse_log(early)["reason"], "main_executable_not_first")

    def test_inserted_dylib_notes_and_missing_uuid_are_recorded(self):
        text = (load_line(7, UUID_A, "/app/main") + load_line(7, None, "/t/nouuid.dylib")
                + "dyld[7]: skipping duplicate inserted dylib '/t/x.dylib'\n")
        parsed = observer._parse_log(text)
        self.assertEqual(parsed["reason"], "inserted_dylib_present")
        self.assertIsNone(parsed["images"][1]["uuid"])

    def test_item_limits_mark_the_inventory_incomplete(self):
        text = load_line(7, UUID_A, "/app/main") + load_line(7, UUID_B, "/t/lib.dylib") * 2
        self.assertEqual(observer._parse_log(text, image_limit=1)["reason"], "image_limit")
        limited = observer._parse_log(text, event_limit=2)
        self.assertEqual(limited["reason"], "event_limit")
        self.assertEqual(len(limited["events"]), 2)


class MachoTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def write(self, name, blob):
        path = self.root / name
        path.write_bytes(blob)
        return path

    def slices(self, blob):
        path = self.write("image", blob)
        with open(path, "rb") as handle:
            return observer._macho_slices(handle, path.stat().st_size)

    def test_thin_images_in_both_byte_orders_expose_uuid_and_arch(self):
        little = self.slices(macho(UUID_A))
        self.assertEqual((little[0]["uuid"], little[0]["arch"]), (UUID_A, "arm64"))
        big = self.slices(macho(UUID_B, cputype=X86_64, cpusubtype=3, order=">"))
        self.assertEqual((big[0]["uuid"], big[0]["arch"], big[0]["slice_offset"]),
                         (UUID_B, "x86_64", 0))

    def test_fat_images_bind_the_slice_whose_uuid_was_logged(self):
        blob = fat((ARM64, macho(UUID_A)), (X86_64, macho(UUID_B, cputype=X86_64, cpusubtype=3)))
        path = self.write("fat", blob)
        bound = observer._bind_file(str(path), UUID_B, time.monotonic() + 5,
                                   {"bytes": observer.MAX_HASH_BYTES})
        self.assertTrue(bound["bound"])
        self.assertEqual(bound["macho"]["arch"], "x86_64")
        self.assertEqual(bound["macho"]["slice_offset"], 8 + 20 * 2 + len(macho(UUID_A)))
        self.assertEqual(bound["sha256"], hashlib.sha256(blob).hexdigest())

    def test_malformed_images_are_refused_not_guessed(self):
        truncated = macho(UUID_A)[:40]
        with self.assertRaises(observer._Refusal):
            self.slices(truncated)
        twice = struct.pack("<II", 0x1B, 24) + uuid_module.UUID(UUID_A).bytes
        with self.assertRaises(observer._Refusal):
            self.slices(macho(UUID_A, commands=twice * 2, ncmds=2))
        with self.assertRaises(observer._Refusal):
            self.slices(struct.pack(">II", 0xCAFEBABE, 1)
                        + struct.pack(">iIIII", ARM64, 0, 28, 4096, 0))
        with self.assertRaises(observer._Refusal):
            self.slices(b"#!/bin/sh\nexit 0\n")

    def test_image_without_uuid_cannot_match_a_logged_uuid(self):
        blob = macho(UUID_A, commands=struct.pack("<II", 0x2A, 8))
        self.assertIsNone(self.slices(blob)[0]["uuid"])
        path = self.write("plain", blob)
        bound = observer._bind_file(str(path), UUID_A, time.monotonic() + 5,
                                   {"bytes": observer.MAX_HASH_BYTES})
        self.assertEqual((bound["bound"], bound["reason"]), (False, "uuid_mismatch"))


class BindTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "lib.dylib"
        self.blob = macho(UUID_A)
        self.path.write_bytes(self.blob)

    def bind(self, path=None, uuid_text=UUID_A, deadline=None, budget=None):
        return observer._bind_file(str(path or self.path), uuid_text,
                                   time.monotonic() + 5 if deadline is None else deadline,
                                   {"bytes": observer.MAX_HASH_BYTES} if budget is None else budget)

    def test_bound_image_carries_identity_hash_and_slice(self):
        bound = self.bind()
        status = self.path.stat()
        self.assertEqual(bound["reason"], None)
        self.assertEqual(bound["file"], {"size": status.st_size, "inode": status.st_ino,
                                         "device": status.st_dev,
                                         "mtime_ns": status.st_mtime_ns})
        self.assertEqual(bound["sha256"], hashlib.sha256(self.blob).hexdigest())
        self.assertEqual(bound["macho"]["uuid"], UUID_A)

    def test_replaced_missing_and_oversized_files_do_not_bind(self):
        self.assertEqual(self.bind(uuid_text=UUID_B)["reason"], "uuid_mismatch")
        self.assertEqual(self.bind(path=self.path.with_name("gone"))["reason"], "file_missing")
        self.assertEqual(self.bind(budget={"bytes": 3})["reason"], "hash_byte_limit")
        self.assertTrue(self.bind(path=self.directory.name)["reason"].startswith("file_unreadable"))

    def test_expired_deadline_refuses_instead_of_hashing(self):
        self.assertEqual(self.bind(deadline=time.monotonic() - 1)["reason"],
                         "binding_deadline_exceeded")

    def test_last_hash_read_cannot_extend_the_binding_deadline(self):
        with mock.patch.object(observer.time, "monotonic", side_effect=[0, 0, 2]):
            bound = self.bind(deadline=1)
        self.assertFalse(bound["bound"])
        self.assertEqual(bound["reason"], "binding_deadline_exceeded")

    def test_post_open_io_failure_refuses_the_binding(self):
        for operation in ("_macho_slices", "fstat"):
            target = observer if operation == "_macho_slices" else observer.os
            with self.subTest(operation=operation), mock.patch.object(
                    target, operation, side_effect=OSError(5, "I/O error")):
                bound = self.bind()
                self.assertFalse(bound["bound"])
                self.assertIsNone(bound["sha256"])

    def test_system_prefix_without_cache_membership_requires_file_hashing(self):
        with mock.patch.object(observer, "_cache_contains_path", return_value=False):
            self.assertEqual(observer._image_class("/usr/lib/on-disk.dylib"), "file")
        for path in ("/System/Volumes/Data/tmp/lib.dylib", "/System/Library/../Volumes/Data/lib.dylib"):
            with self.subTest(path=path), self.assertRaises(observer._Refusal):
                observer._check_image_binding({"bound": True, "path": path, "uuid": UUID_A, "class": "system"})


class SyntheticRun:
    """Synthetic discovery over temp Mach-O files; no child is launched."""

    def __init__(self, root):
        self.root = Path(root)
        self.main = self.root / "main"
        self.lib = self.root / "liba.dylib"
        self.extra = self.root / "libextra.dylib"
        self.main.write_bytes(macho(UUID_A, filetype=2))
        self.lib.write_bytes(macho(UUID_B))
        self.extra.write_bytes(macho(UUID_D))

    def log(self, *, pid=7, extra=False, repeat_lib=1, lib_uuid=UUID_B, tail=""):
        text = load_line(pid, UUID_A, str(self.main))
        text += load_line(pid, lib_uuid, str(self.lib)) * repeat_lib
        text += load_line(pid, UUID_C, SYSTEM_IMAGE)
        if extra:
            text += load_line(pid, UUID_D, str(self.extra))
        return text + tail

    def discovery(self, stderr=None, **overrides):
        receipt = observer._receipt("macos-runtime-image-discovery")
        receipt["loader"]["env"] = dict(LOADER_ENV)
        receipt["system"] = dict(SYSTEM)
        receipt["dyld"] = json.loads(json.dumps(DYLD))
        run = record(self.log() if stderr is None else stderr, **overrides)
        with mock.patch.object(observer, "_cache_contains_path", side_effect=lambda path: path == SYSTEM_IMAGE):
            observer._complete_discovery(
                receipt, run, parsed=observer._parse_log(run["stderr"]),
                main_identity=observer._main_identity(str(self.main)), loader_env=dict(LOADER_ENV),
                deadline=time.monotonic() + 5)
        return json.loads(json.dumps(receipt))

    def observation(self, closure, stderr=None, *, between=None, **overrides):
        receipt = observer._receipt("macos-runtime-image-observation")
        frozen = observer._check_closure(closure)
        receipt["closure_sha256"] = frozen["sha256"]
        receipt["target"] = frozen["target"]
        receipt["system"] = dict(SYSTEM)
        receipt["dyld"] = json.loads(json.dumps(DYLD))
        bindings, reason = observer._recheck_files(frozen["rows"].values(),
                                                   time.monotonic() + 5, "pre")
        receipt["provenance"]["pre"] = {"checked": len(bindings), "reason": reason,
                                        "bindings": bindings}
        if between is not None:
            between()
        run = record(self.log() if stderr is None else stderr, **overrides)
        observer._complete_observation(
            receipt, run, parsed=observer._parse_log(run["stderr"]), frozen=frozen,
            main_identity=observer._main_identity(str(self.main)), loader_env=dict(LOADER_ENV),
            deadline=time.monotonic() + 5)
        return json.loads(json.dumps(receipt))


@unittest.skipIf(sys.platform == "win32", "synthetic logs name POSIX absolute image paths")
class ClosureTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.run = SyntheticRun(self.directory.name)

    def test_discovery_binds_every_image_and_separates_provenance(self):
        receipt = self.run.discovery(cleanup_ok=False)
        self.assertEqual((receipt["result"]["status"], receipt["result"]["reason"]),
                         ("complete", None))
        self.assertEqual(receipt["target"], "aarch64-apple-darwin")
        self.assertEqual([image["class"] for image in receipt["images"]],
                         ["file", "file", "system"])
        self.assertEqual(receipt["images"][1]["sha256"],
                         hashlib.sha256(self.run.lib.read_bytes()).hexdigest())
        self.assertFalse(receipt["cleanup_ok"])
        self.assertTrue(receipt["provenance"]["complete"])
        self.assertEqual(receipt["record"]["stderr"], self.run.log())

    def test_post_exit_io_failure_preserves_child_outcome_and_log(self):
        with mock.patch.object(observer, "_macho_slices", side_effect=OSError(5, "I/O error")):
            receipt = self.run.discovery(exit_code=23, cleanup_ok=True)
        self.assertEqual(receipt["record"]["exit_code"], 23)
        self.assertEqual(receipt["record"]["stderr"], self.run.log())
        self.assertTrue(receipt["cleanup_ok"])
        self.assertFalse(receipt["provenance"]["complete"])
        self.assertFalse(receipt["images"][0]["bound"])

    def test_missing_transient_file_leaves_discovery_incomplete(self):
        receipt = self.run.discovery(self.run.log(extra=True, tail=""))
        self.assertEqual(receipt["result"]["status"], "complete")
        self.run.extra.unlink()
        receipt = self.run.discovery(self.run.log(extra=True))
        self.assertEqual(receipt["result"]["reason"], "image_unbound")
        self.assertEqual(receipt["result"]["unbound"],
                         [{"path": str(self.run.extra), "uuid": UUID_D, "reason": "file_missing"}])
        self.assertEqual(len(receipt["events"]), 4)

    def test_main_executable_must_be_the_logged_first_image(self):
        other = self.run.root / "other"
        shutil.copy(self.run.main, other)
        text = load_line(7, UUID_A, str(other)) + load_line(7, UUID_C, SYSTEM_IMAGE)
        receipt = self.run.discovery(text)
        self.assertEqual(receipt["result"]["reason"], "main_executable_mismatch")

    def test_runner_process_id_is_the_only_root_pid_authority(self):
        owned = self.run.discovery()
        self.assertEqual((owned["result"]["status"], owned["loader"]["root_pid"]),
                         ("complete", 7))
        self.assertTrue(observer.validate_closure(owned)["ok"])
        for process_id in (None, 0, -1, "7"):
            missing = self.run.discovery(process_id=process_id)
            self.assertEqual(missing["result"]["reason"], "root_pid_unavailable")
            self.assertIsNone(missing["loader"]["root_pid"])
            self.assertEqual(observer.validate_closure(missing)["reason"], "root_pid_unavailable")
        stranger = self.run.discovery(process_id=8)
        self.assertEqual((stranger["result"]["reason"], stranger["loader"]["pid"]),
                         ("root_pid_mismatch", 7))
        forged = json.loads(json.dumps(owned))
        forged["record"]["process_id"] = 9
        self.assertEqual(observer.validate_closure(forged)["reason"], "root_pid_mismatch")
        forged = json.loads(json.dumps(owned))
        forged["loader"]["root_pid"] = 9
        self.assertEqual(observer.validate_closure(forged)["reason"], "root_pid_mismatch")

    def test_closure_is_derived_from_the_raw_log_not_from_status(self):
        receipt = self.run.discovery()
        derived = observer.validate_closure(receipt)
        self.assertTrue(derived["ok"], derived["reason"])
        closure = derived["closure"]
        self.assertEqual([row["path"] for row in closure["images"]],
                         [str(self.run.main), str(self.run.lib), SYSTEM_IMAGE])
        self.assertEqual(closure["origins"][0]["receipt_sha256"],
                         observer.canonical_sha256(receipt))
        forged = json.loads(json.dumps(receipt))
        forged["result"]["status"] = "complete"
        forged["events"].pop()
        self.assertEqual(observer.validate_closure(forged)["reason"], "events_mismatch")
        forged = json.loads(json.dumps(receipt))
        forged["images"][1]["uuid"] = UUID_D
        self.assertEqual(observer.validate_closure(forged)["reason"], "images_mismatch")
        forged = json.loads(json.dumps(receipt))
        forged["images"][1]["macho"]["uuid"] = UUID_D
        self.assertTrue(observer.validate_closure(forged)["reason"]
                        .startswith("image_macho_uuid_mismatch"))
        forged = json.loads(json.dumps(receipt))
        forged["record"]["exit_code"] = 1
        self.assertEqual(observer.validate_closure(forged)["reason"], "record_not_successful")
        forged = json.loads(json.dumps(receipt))
        forged["record"]["stderr"] += load_line(8, UUID_D, str(self.run.extra))
        self.assertTrue(observer.validate_closure(forged)["reason"]
                        .startswith("raw_log_not_closed"))

    def test_unbound_or_foreign_receipts_yield_no_closure(self):
        self.run.extra.unlink()
        incomplete = self.run.discovery(self.run.log(extra=True))
        self.assertTrue(observer.validate_closure(incomplete)["reason"]
                        .startswith("image_unbound"))
        self.assertEqual(observer.validate_closure([])["reason"], "no_receipts")
        self.assertEqual(observer.validate_closure({"kind": "other"})["reason"],
                         "receipt_kind_invalid")

    def test_union_of_representative_receipts_without_conflicting_bindings(self):
        first = self.run.discovery()
        second = self.run.discovery(self.run.log(extra=True))
        union = observer.validate_closure([first, second])
        self.assertTrue(union["ok"], union["reason"])
        self.assertEqual([row["uuid"] for row in union["closure"]["images"]],
                         [UUID_A, UUID_B, UUID_C, UUID_D])
        self.assertEqual(len(union["closure"]["origins"]), 2)
        self.run.lib.write_bytes(macho(UUID_D))
        replaced = self.run.discovery(self.run.log(lib_uuid=UUID_D))
        self.assertEqual(replaced["result"]["status"], "complete")
        conflict = observer.validate_closure([first, replaced])
        self.assertTrue(conflict["reason"].startswith("conflicting_image_uuid"))
        other_host = json.loads(json.dumps(second))
        other_host["system"]["os_build"] = "26A999"
        self.assertEqual(observer.validate_closure([first, other_host])["reason"],
                         "system_differs")
        other_env = json.loads(json.dumps(second))
        other_env["loader"]["env"]["DYLD_LIBRARY_PATH"] = "/elsewhere"
        self.assertEqual(observer.validate_closure([first, other_env])["reason"],
                         "loader_env_differs")


@unittest.skipIf(sys.platform == "win32", "synthetic logs name POSIX absolute image paths")
class ObservationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.run = SyntheticRun(self.directory.name)
        first = self.run.discovery()
        second = self.run.discovery(self.run.log(extra=True))
        self.closure = observer.validate_closure([first, second])["closure"]

    def test_subset_of_closure_is_complete_and_new_images_are_not(self):
        receipt = self.run.observation(self.closure, self.run.log(repeat_lib=2))
        self.assertEqual((receipt["result"]["status"], receipt["result"]["reason"]),
                         ("complete", None))
        self.assertEqual([image["closure_index"] for image in receipt["images"]], [0, 1, 2])
        self.assertEqual(len(receipt["events"]), 4)
        self.assertTrue(receipt["provenance"]["complete"])
        stranger = self.run.root / "libnew.dylib"
        stranger.write_bytes(macho(UUID_C))
        receipt = self.run.observation(
            self.closure, self.run.log() + load_line(7, UUID_C, str(stranger)))
        self.assertEqual(receipt["result"]["reason"], "image_unbound")
        self.assertEqual(receipt["result"]["unbound"][0]["reason"], "image_not_in_closure")
        receipt = self.run.observation(self.closure, self.run.log(lib_uuid=UUID_D))
        self.assertEqual(receipt["result"]["unbound"][0]["reason"], "uuid_changed")
        receipt = self.run.observation(
            self.closure, self.run.log() + load_line(7, UUID_D, "/usr/lib/libz.1.dylib"))
        self.assertFalse(receipt["provenance"]["complete"])
        self.assertFalse(observer.validate_observation(receipt, self.closure)["ok"])

    def test_post_exit_rebinding_detects_replacement_apart_from_cleanup(self):
        receipt = self.run.observation(
            self.closure, cleanup_ok=False,
            between=lambda: self.run.lib.write_bytes(macho(UUID_B) + b"\0"))
        self.assertIsNone(receipt["provenance"]["pre"]["reason"])
        self.assertEqual(receipt["result"]["status"], "incomplete")
        self.assertTrue(receipt["result"]["reason"].startswith("post_file_changed"))
        self.assertEqual(receipt["provenance"]["post"]["checked"], 2)
        self.assertFalse(receipt["provenance"]["complete"])
        self.assertFalse(receipt["cleanup_ok"])
        verdict = observer.validate_observation(receipt, self.closure)
        self.assertTrue(verdict["reason"].startswith("post_binding_mismatch"))
        self.assertIs(verdict["cleanup_ok"], False)

    def test_failed_execution_keeps_independent_provenance_and_cleanup(self):
        for overrides, complete in (
                ({"exit_code": 23}, True),
                ({"exit_code": 23, "cleanup_ok": False}, True),
                ({"exit_code": -15, "timed_out": True}, False),
                ({"exit_code": -15, "output_limited": True}, False)):
            with self.subTest(overrides=overrides):
                receipt = self.run.observation(self.closure, **overrides)
                verdict = observer.validate_observation(receipt, self.closure)
                self.assertEqual(verdict["cleanup_ok"], receipt["record"]["cleanup_ok"])
                self.assertFalse(verdict["ok"])
                self.assertEqual(verdict["provenance_complete"], complete)

    def test_retained_observation_is_independent_of_current_loader(self):
        receipt = self.run.observation(self.closure, self.run.log())
        self.run.lib.unlink()
        with mock.patch.object(observer, "_cache_contains_path",
                               side_effect=AssertionError("retained evidence queried the live cache")):
            verdict = observer.validate_observation(receipt, self.closure)
        self.assertTrue(verdict["ok"], verdict["reason"])

    def test_image_only_history_does_not_claim_a_missing_timing_interval(self):
        receipt = self.run.observation(self.closure)
        self.assertTrue(observer.validate_observation(receipt, self.closure)["ok"])
        timing = observer.validate_timing_boundaries(receipt["record"], ["row-a"])
        self.assertFalse(timing["complete"])
        self.assertEqual(timing["reason"], "timing_markers_missing")

    def test_retained_observation_is_revalidated_from_raw_data_only(self):
        receipt = self.run.observation(self.closure, self.run.log(repeat_lib=2), cleanup_ok=False)
        verdict = observer.validate_observation(receipt, self.closure)
        self.assertEqual((verdict["ok"], verdict["reason"]), (True, None))
        self.assertEqual((verdict["observed_events"], verdict["cleanup_ok"]), (4, False))
        for mutate, prefix in (
                (lambda r: r["events"].pop(), "events_mismatch"),
                (lambda r: r["images"][1].update(bound=False), "images_mismatch"),
                (lambda r: r["record"].update(exit_code=2), "child_exit_2"),
                (lambda r: r["record"].update(timed_out=True), "timed_out"),
                (lambda r: r["record"].update(process_id=None), "root_pid_unavailable"),
                (lambda r: r["record"].update(process_id=8), "root_pid_mismatch"),
                (lambda r: r["loader"]["env"].update(DYLD_LIBRARY_PATH="/x"), "loader_env_mismatch"),
                (lambda r: r["system"].update(os_build="26A999"), "system_identity_mismatch"),
                (lambda r: r["provenance"]["pre"]["bindings"].pop(), "pre_bindings_incomplete"),
                (lambda r: r["provenance"]["post"]["bindings"][1].update(sha256="0" * 64),
                 "post_binding_mismatch"),
                (lambda r: r["record"].update(
                    stderr=r["record"]["stderr"] + load_line(7, UUID_D, "/t/late.dylib")),
                 "events_mismatch")):
            forged = json.loads(json.dumps(receipt))
            mutate(forged)
            reason = observer.validate_observation(forged, self.closure)["reason"]
            self.assertTrue(reason.startswith(prefix), reason)
        other = json.loads(json.dumps(self.closure))
        other["origins"][0]["receipt_sha256"] = "0" * 64
        self.assertEqual(observer.validate_observation(receipt, other)["reason"],
                         "closure_sha256_mismatch")
        stranger = self.run.root / "libnew.dylib"
        stranger.write_bytes(macho(UUID_C))
        forged = self.run.observation(self.closure, self.run.log() + load_line(7, UUID_C, str(stranger)))
        forged["result"].update(status="complete", reason=None, unbound=[])
        self.assertEqual(observer.validate_observation(forged, self.closure)["reason"],
                         "image_unbound")

    def test_unhashable_binding_paths_return_a_structured_refusal(self):
        receipt = self.run.observation(self.closure)
        for phase in ("pre", "post"):
            for path in ([], {}):
                with self.subTest(phase=phase, path=path):
                    broken = json.loads(json.dumps(receipt))
                    broken["provenance"][phase]["bindings"][0]["path"] = path
                    self.assertFalse(observer.validate_observation(broken, self.closure)["ok"])

    def test_closure_origins_require_complete_discovery_references(self):
        good = self.closure["origins"][0]
        variants = [None, {}, *({**good, field: value} for field, value in (
            ("receipt_sha256", "x"), ("pid", 0), ("pid", True), ("events", -1),
            ("cleanup_ok", "yes"), ("record_duration_seconds", -1),
            ("record_duration_seconds", float("nan"))))]
        for origin in variants:
            with self.subTest(origin=origin):
                broken = json.loads(json.dumps(self.closure))
                broken["origins"] = [origin]
                with self.assertRaises(observer._Refusal):
                    observer._check_closure(broken)

    def test_closure_structure_is_validated_not_trusted(self):
        frozen = observer._check_closure(self.closure)
        self.assertEqual(len(frozen["rows"]), 4)
        for mutate, prefix in (
                (lambda c: c.update(complete=True), "closure_kind_invalid"),
                (lambda c: c["images"][1].update(bound=True), "closure_image_row_invalid"),
                (lambda c: c["images"].append(dict(c["images"][1])), "closure_image_duplicate"),
                (lambda c: c["images"].pop(0), "closure_main_executable_unbound"),
                (lambda c: c["images"][1].update(sha256="x"), "image_sha256_invalid"),
                (lambda c: c["loader_env"].update(DYLD_PRINT_APIS="1"), "loader_env_invalid"),
                (lambda c: c.update(target="x86_64-apple-darwin"), "closure_target_mismatch")):
            broken = json.loads(json.dumps(self.closure))
            mutate(broken)
            with self.assertRaises(observer._Refusal) as raised:
                observer._check_closure(broken)
            self.assertTrue(str(raised.exception).startswith(prefix), str(raised.exception))

    def test_observe_refuses_invalid_closure_before_any_launch(self):
        receipt = observer.observe([str(self.run.main)], cwd=self.run.root, env=dict(LOADER_ENV),
                                   closure={"kind": "x"}, timeout_seconds=1,
                                   output_limit_bytes=4096, cleanup_seconds=1)
        self.assertEqual(receipt["result"]["status"], "refused")
        self.assertTrue(receipt["result"]["reason"].startswith("closure_invalid"))
        self.assertIsNone(receipt["record"]["exit_code"])
        self.assertIsNotNone(receipt["record"]["launch_error"])
        self.assertEqual(receipt["images"], [])

    @unittest.skipUnless(sys.platform == "darwin", "actual system identity comparison")
    def test_observe_rejects_a_closure_from_another_system_before_launch(self):
        receipt = observer.observe([str(self.run.main)], cwd=self.run.root, env=dict(LOADER_ENV),
                                   closure=self.closure, timeout_seconds=1,
                                   output_limit_bytes=4096, cleanup_seconds=1)
        self.assertEqual(receipt["result"]["status"], "refused")
        self.assertIn(receipt["result"]["reason"],
                      ("system_identity_differs", "dyld_identity_differs"))
        self.assertIsNone(receipt["record"]["exit_code"])


class SettlementTests(unittest.TestCase):
    def settle(self, **overrides):
        receipt = observer._receipt("macos-runtime-image-discovery")
        parsed = observer._parse_log(load_line(7, UUID_A, "/app/main"))
        observer._settle(receipt, record("", **overrides), parsed, None)
        return receipt["result"]

    def test_owned_record_facts_precede_log_and_binding(self):
        self.assertEqual(self.settle(timed_out=True, exit_code=0)["reason"], "timed_out")
        self.assertEqual(self.settle(output_limited=True)["reason"], "output_limited")
        self.assertEqual(self.settle(exit_code=None)["reason"], "child_exit_not_owned")
        self.assertEqual(self.settle(exit_code=3)["reason"], "child_exit_3")
        self.assertEqual(self.settle(launch_error="boom", exit_code=None)["reason"],
                         "launch_failed: boom")
        self.assertEqual(self.settle()["status"], "complete")


class RefusalTests(unittest.TestCase):
    def discover(self, argv=None, env=None, **overrides):
        arguments = dict(cwd=Path(tempfile.gettempdir()).resolve(), env=env or {},
                         timeout_seconds=1, output_limit_bytes=4096, cleanup_seconds=1)
        arguments.update(overrides)
        return observer.discover(argv or [str(Path(sys.executable).resolve()), "-c", "pass"],
                                 **arguments)

    def assert_refused_without_launch(self, receipt, prefix):
        self.assertEqual(receipt["result"]["status"], "refused", receipt["result"]["reason"])
        self.assertTrue(receipt["result"]["reason"].startswith(prefix), receipt["result"]["reason"])
        self.assertEqual(receipt["record"]["launch_error"], receipt["result"]["reason"])
        self.assertEqual((receipt["images"], receipt["events"]), ([], []))
        self.assertTrue(receipt["cleanup_ok"])

    def test_other_print_variables_and_insertions_are_refused(self):
        self.assert_refused_without_launch(
            self.discover(env={"DYLD_PRINT_SEGMENTS": "1"}), "invalid_arguments")
        self.assert_refused_without_launch(
            self.discover(env={"DYLD_INSERT_LIBRARIES": "/x.dylib"}), "invalid_arguments")
        for key in ("DYLD_SHARED_REGION", "DYLD_SHARED_CACHE_DIR", "DYLD_ROOT_PATH"):
            with self.subTest(key=key):
                self.assert_refused_without_launch(self.discover(env={key: "/alternate"}), "invalid_arguments")
        self.assert_refused_without_launch(
            self.discover(binding_seconds=0), "invalid_arguments")
        self.assert_refused_without_launch(
            self.discover(argv=["python3", "-c", "pass"]), "invalid_arguments")

    @unittest.skipIf(sys.platform == "darwin", "unsupported-platform boundary")
    def test_unsupported_host_refuses_without_launch(self):
        self.assert_refused_without_launch(self.discover(), "requires_macos")

    @unittest.skipUnless(sys.platform == "darwin", "actual loader refusal")
    def test_missing_main_executable_refuses_without_launch(self):
        self.assert_refused_without_launch(
            self.discover(argv=[str(Path(tempfile.gettempdir()).resolve() / "absent-main")]),
            "main_executable_unreadable")


TRANSIENT_SOURCE = "int transient_value(void) { return %d; }\n"
LOADER_SOURCE = r"""
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 3) return 64;
    const char *mode = argv[1];
    const char *path = argv[2];
    void *handle = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (handle == NULL) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 65; }
    int (*value)(void) = (int (*)(void))dlsym(handle, "transient_value");
    if (value == NULL) return 66;
    int observed = value();
    if (strcmp(mode, "vanish") == 0 && unlink(path) != 0) return 67;
    dlclose(handle);
    void *still = dlopen(path, RTLD_NOW | RTLD_LOCAL | RTLD_NOLOAD);
    int unloaded = still == NULL;
    if (still != NULL) dlclose(still);
    int reloaded = 0;
    if (strcmp(mode, "transient") == 0) {
        void *again = dlopen(path, RTLD_NOW | RTLD_LOCAL);
        if (again == NULL) return 68;
        reloaded = 1;
        dlclose(again);
    }
    if (strcmp(mode, "spawn") == 0) {
        pid_t pid = fork();
        if (pid == 0) {
            char *child[] = {argv[0], "transient", argv[2], NULL};
            execv(argv[0], child);
            _exit(70);
        }
        int status = 0;
        if (waitpid(pid, &status, 0) != pid || !WIFEXITED(status) || WEXITSTATUS(status) != 0)
            return 69;
    }
    printf("{\"value\":%d,\"unloaded\":%d,\"reloaded\":%d}\n", observed, unloaded, reloaded);
    return 0;
}
"""


@unittest.skipUnless(sys.platform == "darwin" and os.environ.get("MADO_MACOS_IMAGE_OBSERVER_SMOKE") == "1",
                     "opt-in transient dylib smoke on macOS")
class TransientDylibSmoke(unittest.TestCase):
    """Compiles a throwaway loader with cc and proves transient loads are observed."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="mado-image-observer-smoke-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.loader = self.root / "loader"
        self.dylib = self.root / "libtransient.dylib"
        self.compile(LOADER_SOURCE, "loader.c", [], self.loader)
        self.build_dylib(1)

    def compile(self, source, name, flags, output):
        path = self.root / name
        path.write_text(source, encoding="utf-8")
        subprocess.run(["cc", *flags, "-o", str(output), str(path)], check=True,
                       capture_output=True, timeout=60)

    def build_dylib(self, value):
        self.compile(TRANSIENT_SOURCE % value, "transient.c", ["-dynamiclib"], self.dylib)

    def run_helper(self, function, mode, dylib, **extra):
        receipt = function([str(self.loader), mode, str(dylib)], cwd=self.root, env={},
                           timeout_seconds=30, output_limit_bytes=1 << 20, cleanup_seconds=5,
                           **extra)
        if receipt["result"]["reason"] == "system_identity_unavailable":
            self.fail(json.dumps({
                "receipt": receipt,
                "sysctl": {key: observer._sysctl_string(name)
                           for key, name in observer._SYSCTL_NAMES.items()},
                "shared_cache_uuid": observer._shared_cache_uuid(),
                "observer_architecture": observer.platform.machine(),
            }))
        return receipt

    def events_for(self, receipt, path):
        return [event for event in receipt["events"]
                if Path(event["path"]).resolve() == path.resolve()]

    def test_transient_load_unload_reload_is_fully_observed_and_closure_round_trips(self):
        receipt = self.run_helper(observer.discover, "transient", self.dylib)
        self.assertEqual((receipt["result"]["status"], receipt["result"]["reason"]),
                         ("complete", None), json.dumps(receipt["result"]))
        outcome = json.loads(receipt["record"]["stdout"])
        self.assertEqual(outcome["value"], 1)
        self.assertEqual(len(self.events_for(receipt, self.dylib)),
                         1 + (outcome["unloaded"] and outcome["reloaded"]))
        image = next(image for image in receipt["images"]
                     if Path(image["path"]).resolve() == self.dylib.resolve())
        self.assertEqual(image["sha256"], hashlib.sha256(self.dylib.read_bytes()).hexdigest())
        self.assertEqual(image["macho"]["uuid"], image["uuid"])
        self.assertEqual(receipt["events"][0]["path"], receipt["loader"]["main_executable"])
        self.assertGreater(receipt["record"]["process_id"], 0)
        self.assertEqual((receipt["loader"]["pid"], receipt["loader"]["root_pid"]),
                         (receipt["record"]["process_id"], receipt["record"]["process_id"]))
        derived = observer.validate_closure(receipt)
        self.assertTrue(derived["ok"], derived["reason"])
        observed = self.run_helper(observer.observe, "transient", self.dylib,
                                   closure=derived["closure"])
        self.assertEqual((observed["result"]["status"], observed["result"]["reason"]),
                         ("complete", None), json.dumps(observed["result"]))
        self.assertTrue(observed["provenance"]["complete"])
        self.assertEqual(observed["closure_sha256"],
                         observer.canonical_sha256(derived["closure"]))
        verdict = observer.validate_observation(json.loads(json.dumps(observed)),
                                                derived["closure"])
        self.assertEqual((verdict["ok"], verdict["reason"]), (True, None))
        self.build_dylib(2)
        refused = self.run_helper(observer.observe, "transient", self.dylib,
                                  closure=derived["closure"])
        self.assertEqual(refused["result"]["status"], "refused")
        self.assertTrue(refused["result"]["reason"].startswith("pre_"), refused["result"]["reason"])
        self.assertIsNone(refused["record"]["exit_code"])

    def test_vanished_transient_file_cannot_be_bound(self):
        copy = self.root / "libvanish.dylib"
        shutil.copy(self.dylib, copy)
        receipt = self.run_helper(observer.discover, "vanish", copy)
        self.assertEqual(receipt["record"]["exit_code"], 0)
        self.assertEqual(receipt["result"]["reason"], "image_unbound")
        self.assertEqual([item["reason"] for item in receipt["result"]["unbound"]], ["file_missing"])
        self.assertEqual(len(self.events_for(receipt, copy)), 1)

    def test_descendant_loader_output_is_foreign_not_claimed(self):
        receipt = self.run_helper(observer.discover, "spawn", self.dylib)
        self.assertEqual(receipt["record"]["exit_code"], 0)
        self.assertEqual(receipt["result"]["reason"], "foreign_pid")
        self.assertEqual(len(receipt["loader"]["foreign_pids"]), 1)


if __name__ == "__main__":
    unittest.main()
