"""Portable loader identity negatives and Windows-only same-handle file fences."""

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import time
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "windows_loader_observer", Path(__file__).resolve().parents[1]
    / "native-release-profile/windows_loader_observer.py")
observer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(observer)

MAIN = r"C:\app\bench.exe"
DLL = r"C:\app\known.dll"
NT_MAIN = r"\Device\HarddiskVolume3\app\bench.exe"
NT_DLL = r"\Device\HarddiskVolume3\app\known.dll"


def pe_bytes():
    data = bytearray(1024)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 64)
    data[64:68] = b"PE\0\0"
    struct.pack_into("<HHI", data, 68, 0x8664, 1, 7)
    struct.pack_into("<HH", data, 84, 112, 0x2022)
    struct.pack_into("<H", data, 88, 0x20b)
    struct.pack_into("<I", data, 104, 0x1000)
    struct.pack_into("<Q", data, 112, 0x140000000)
    struct.pack_into("<II", data, 144, 0x2000, 512)
    struct.pack_into("<HH", data, 156, 3, 0x160)
    struct.pack_into("<IIII", data, 208, 512, 0x1000, 512, 512)
    struct.pack_into("<I", data, 236, 0x60000020)
    return bytes(data)


def facts(path=NT_MAIN, identity="01" * 16):
    return {"volume_serial": 42, "file_id_hex": identity,
            "final_nt_path_utf16_hex": observer._hexwide(path),
            "creation_time": 1, "last_access_time": 2, "last_write_time": 3,
            "change_time": 4, "attributes": 0x20, "allocation_size": 4096,
            "file_size": 1024, "links": 1, "delete_pending": 0, "directory": 0}


def counter(n=0):
    return {"first": n, "last": n, "incomplete_bits": 0, "pending_sequences": [], "clean": True}


def module(path=MAIN, base=0x140000000):
    return {"base": base, "size": 0x2000, "path_utf16_hex": observer._hexwide(path)}


def census(modules, n=0):
    return {"before": counter(n), "after": counter(n), "needed_bytes": len(modules) * 8,
            "failure": None, "modules": copy.deepcopy(modules)}


def event(sequence, reason, image):
    return {"sequence": sequence, "complete": True, "reason": reason, "flags": 0,
            "base": image["base"], "size": image["size"], "invalid_bits": 0,
            "full_name_utf16_hex": image["path_utf16_hex"],
            "base_name_utf16_hex": observer._hexwide(observer.ntpath.basename(observer._wide(image["path_utf16_hex"])))}


def bridge(image, index, stage, native=NT_MAIN):
    pe = observer._parse_pe(lambda offset, count: pe_bytes()[offset:offset + count], 1024)
    return {"stage": stage, "census_index": index, "module_index": 0, "base": image["base"],
            "mapped_nt_path_utf16_hex": observer._hexwide(native),
            "mapped_nt_path_after_utf16_hex": observer._hexwide(native),
            "file_before": facts(native), "file_after": facts(native),
            "file_before_complete": True, "file_after_complete": True,
            "memory_pe_complete": True, "file_pe_complete": True,
            "memory_pe": copy.deepcopy(pe), "file_pe": pe, "failure": None, "close_success": True}



class FileApi:
    def __init__(self):
        self.next_handle = 100
        self.opened, self.closed = {}, []
        self.data = pe_bytes()
        self.file_facts = facts()
        self.fail_close = set()
        self.fail_read = False

    def open(self, path, directory=False):
        self.next_handle += 1
        self.opened[self.next_handle] = (path, directory)
        return self.next_handle

    def directory(self, handle):
        if not self.opened[handle][1]:
            raise ValueError("not_directory")
        return {"file_type": 1, "attributes": 0x10}

    def facts(self, handle):
        path = self.opened[handle][0]
        return copy.deepcopy(self.file_facts if path == MAIN else facts(r"\Device\HarddiskVolume3" + path[2:]))

    def read(self, handle, offset, length):
        if self.fail_read:
            raise OSError("read_denied")
        return self.data[offset:offset + length]

    def close(self, handle):
        self.closed.append(handle)
        if handle in self.fail_close:
            raise OSError("close_failed")


class PinTests(unittest.TestCase):
    def test_hash_and_pe_are_read_from_the_opened_handle(self):
        api = FileApi()
        pins = observer._Pins(api, time.monotonic() + 10)
        before = pins.open(MAIN)
        self.assertEqual(before["sha256"], hashlib.sha256(pe_bytes()).hexdigest())
        self.assertEqual(before["pe"]["machine"], 0x8664)
        api.file_facts["last_access_time"] += 1
        after = pins.snapshot(MAIN)
        self.assertEqual(observer._binding(before), observer._binding(after))
        self.assertTrue(all(row["success"] for row in pins.close()))
        self.assertEqual(len(api.closed), len(api.opened))

    def test_same_handle_hash_change_is_not_hidden_by_matching_metadata(self):
        api = FileApi()
        pins = observer._Pins(api, time.monotonic() + 10)
        before = pins.open(MAIN)
        api.data = api.data[:-1] + b"x"
        after = pins.snapshot(MAIN)
        self.assertNotEqual(observer._binding(before), observer._binding(after))
        pins.close()

    def test_failure_keeps_all_opened_handles_available_for_cleanup(self):
        api = FileApi()
        api.fail_read = True
        pins = observer._Pins(api, time.monotonic() + 10)
        with self.assertRaisesRegex(OSError, "read_denied"):
            pins.open(MAIN)
        api.fail_close.add(102)
        cleanup = pins.close()
        self.assertEqual(api.closed, list(reversed(api.opened)))
        self.assertEqual([row["success"] for row in cleanup], [True, False, True])
        self.assertIn("close_failed", cleanup[1]["error"])

    def test_read_budget_and_deadline_are_finite(self):
        for budget in ("bytes", "deadline"):
            api = FileApi()
            pins = observer._Pins(api, time.monotonic() + 10)
            if budget == "bytes":
                pins.bytes = observer.MAX_HASH_BYTES
            else:
                pins.deadline = 0
            with self.assertRaises(observer._Refusal):
                pins.open(MAIN)
            pins.close()
            self.assertEqual(len(api.closed), len(api.opened))

    def test_remote_reparse_and_metadata_ambiguity_refuse(self):
        for path in (r"\\server\share\image.dll", r"\\?\C:\image.dll", r"C:\app\..\image.dll",
                     r"C:\app\image.dll:stream", r"C:\app\image.dll ", r"\Device\Mup\server\image.dll"):
            with self.subTest(path=path), self.assertRaises(observer._Refusal):
                observer._path(path)
        for key, value in (("attributes", 0x400), ("directory", 1), ("delete_pending", 1),
                           ("file_id_hex", "00"), ("file_size", observer.MAX_FILE + 1)):
            with self.subTest(field=key), self.assertRaises(observer._Refusal):
                observer._facts(dict(facts(), **{key: value}))


class PublicationTests(unittest.TestCase):
    def test_pending_and_overflow_are_rederived_not_certified_by_clean(self):
        cases = [dict(counter(), first=1, last=2), dict(counter(1), pending_sequences=[1]),
                 dict(counter(), incomplete_bits=1), counter(257), dict(counter(), last=True)]
        for value in cases:
            with self.subTest(value=value), self.assertRaises(observer._Refusal):
                observer._counter(value)

    def test_all_callback_slots_require_publication_and_identity_fields(self):
        image = module()
        good = event(1, 1, image)
        self.assertEqual(observer._events([good], 1), [good])
        for key, value in (("complete", False), ("sequence", 2), ("invalid_bits", 1),
                           ("reason", 3), ("size", 0), ("base_name_utf16_hex", observer._hexwide("other.dll"))):
            with self.subTest(key=key), self.assertRaises(observer._Refusal):
                observer._events([dict(good, **{key: value})], 1)

    def test_transient_event_cannot_borrow_a_later_same_base_bridge(self):
        image = module()
        current = bridge(image, 1, "initial")
        values = [event(1, 1, image), event(2, 2, image), event(3, 1, image)]
        censuses = [census([image], 3), census([image], 3)]
        with self.assertRaisesRegex(observer._Refusal, "without_observed_bridge"):
            observer._instances(censuses, [current], values)

    def test_initial_instance_unload_is_bound_to_prior_bridge(self):
        main, dll = module(), module(DLL, 0x180000000)
        values = [event(1, 2, dll)]
        samples = [census([main, dll]), census([main, dll]), census([main], 1)]
        bridges = [bridge(main, 1, "initial"), bridge(dll, 1, "initial", NT_DLL),
                   bridge(main, 2, "end")]
        instances = observer._instances(samples, bridges, values)
        self.assertEqual(len(instances), 2)
        self.assertEqual(next(row for row in instances if row["base"] == dll["base"])["end"], 1)

    def test_census_disappearance_without_callback_is_not_unchanged(self):
        main, dll = module(), module(DLL, 0x180000000)
        with self.assertRaisesRegex(observer._Refusal, "lifetime_mismatch"):
            observer._instances([census([main, dll]), census([main])], [], [])


class PeTests(unittest.TestCase):
    def test_only_preferred_base_is_excluded_from_memory_file_correlation(self):
        pe = observer._parse_pe(lambda offset, size: pe_bytes()[offset:offset + size], 1024)
        moved = dict(pe, preferred_base=0x7ff000000000)
        self.assertEqual(observer._nonaddress_pe(pe), observer._nonaddress_pe(moved))
        for field in ("timestamp", "checksum", "entry_rva", "image_size"):
            self.assertNotEqual(observer._nonaddress_pe(pe), observer._nonaddress_pe(dict(moved, **{field: pe[field] + 1})))

    def test_malformed_pe_extents_refuse(self):
        for offset, format_, value in ((60, "<I", 65536), (68, "<H", 0x14c), (70, "<H", 97),
                                       (84, "<H", 0), (144, "<I", 0), (220, "<I", 0xffffff00)):
            data = bytearray(pe_bytes())
            struct.pack_into(format_, data, offset, value)
            with self.subTest(offset=offset), self.assertRaises(observer._Refusal):
                observer._parse_pe(lambda start, size: data[start:start + size], len(data))


@unittest.skipUnless(sys.platform == "win32", "requires real Windows file handles")
class NativeFileTests(unittest.TestCase):
    def test_pin_denies_write_delete_and_checks_same_handle_afterward(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "observed.exe"
            path.write_bytes(pe_bytes())
            pins = observer._Pins(observer._Api(), time.monotonic() + 10)
            try:
                before = pins.open(str(path))
                with self.assertRaises(OSError):
                    with path.open("r+b"):
                        pass
                with self.assertRaises(OSError):
                    path.rename(path.with_suffix(".moved"))
                self.assertEqual(observer._binding(before), observer._binding(pins.snapshot(str(path))))
            finally:
                cleanup = pins.close()
            self.assertTrue(all(row["success"] for row in cleanup), cleanup)
            path.write_bytes(b"released")

    def test_actual_invalid_handle_close_failure_is_explicit(self):
        with self.assertRaisesRegex(observer._Refusal, "CloseHandle"):
            observer._Api().close(0)


class InputTests(unittest.TestCase):
    def test_environment_retains_all_values_and_rejects_case_aliases(self):
        self.assertEqual(observer.loader_environment({"SystemRoot": r"C:\Windows", "Path": r"C:\native",
                                                      "CUSTOM_LOADER_INPUT": "on"}),
                         {"CUSTOM_LOADER_INPUT": "on", "PATH": r"C:\native", "SYSTEMROOT": r"C:\Windows"})
        for env in ({"PATH": "x", "Path": "y"}, {"X": "a\0b"}, {"X": "a" * 32767}):
            with self.assertRaises(observer._Refusal):
                observer.loader_environment(env)

    def test_metadata_only_and_ambiguous_invocations_are_not_observations(self):
        argv = [MAIN, "--workload", "hit-full", "--timing-boundaries", "--windows-loader-observer"]
        self.assertEqual(observer._invocation(argv), "hit-full")
        for args in (argv + ["--list"], argv + ["--workload", "other"], argv[:-1],
                     [MAIN, "--list", "--timing-boundaries", "--windows-loader-observer"]):
            with self.assertRaises(observer._Refusal):
                observer._invocation(args)

    def test_marker_order_pid_and_exact_row_are_owned(self):
        text = "".join(f"{observer.TIMING_PROTOCOL}[7]: hit-full {phase}\n"
                       for phase in ("row-start", "samples-begin", "samples-end"))
        record = {"process_id": 7, "stderr": text}
        self.assertEqual(observer._markers(record, ["hit-full"])["samples_end_line"], 3)
        for invalid in (text[:-1], text.replace("[7]", "[8]"), text.replace("[7]", "[07]"),
                        text.replace("hit-full", "other"), text.replace("samples-begin", "samples-end"),
                        text + text, text.replace("\n", "\r\n")):
            with self.subTest(text=invalid), self.assertRaises(observer._Refusal):
                observer._markers(dict(record, stderr=invalid), ["hit-full"])
        with self.assertRaises(observer._Refusal):
            observer._markers(record, [])

    def test_duplicate_json_and_nonfinite_numbers_are_rejected(self):
        for text in ('{"pid":7,"pid":8}', '{"duration":NaN}', '{"duration":Infinity}'):
            with self.assertRaises(ValueError):
                observer._json(text)


NTDLL = r"C:\Windows\System32\ntdll.dll"
NT_NTDLL = r"\Device\HarddiskVolume3\Windows\System32\ntdll.dll"
SYSTEM = {"major": 10, "minor": 0, "build": 26300, "revision": 9550, "build_lab": "fixture",
          "native_machine": 0x8664, "process_machine": 0}
CAPABILITY = {"protocol": "mado-component-windows-loader-v1",
              "callback": {"rva": 4096, "statics": {"sequence": {"rva": 8192, "bytes": 8},
                  "incomplete": {"rva": 8200, "bytes": 4}, "records": {"rva": 8208, "bytes": 1058816}}}}
ARGV = [MAIN, "--workload", "hit-full", "--timing-boundaries", "--windows-loader-observer"]


def native_report(*, statistical=True):
    images = [module(), module(NTDLL, 0x180000000)]
    bridges = []
    for stage, index in (("initial", 1), ("begin", 3), ("end", 5)):
        for position, (image, path) in enumerate(zip(images, (NT_MAIN, NT_NTDLL))):
            bridges.append(dict(bridge(image, index, stage, path), module_index=position))
    return {
        "protocol": "mado-component-windows-loader-v1", "method": "windows-ldr-observer-on-warm-v1",
        "pid": 7, "workload_id": "hit-full", "statistical": statistical,
        "complete": True, "window_unchanged": True, "primary_failure": None,
        "registration": {"module": images[1]["base"], "export": images[1]["base"] + 4096,
                         "ntstatus": 0, "cookie": 64, "unregister_called": False},
        "cuts": {"initial": [0, 1], "begin": [2, 3], "end": [4, 5]},
        "cut_counters": {"initial": counter(), "begin": counter(), "end": counter()},
        "window_counter": {"begin": counter(), "end": counter()},
        "censuses": [census(images) for _ in range(6)], "bridges": bridges,
        "cleanup": [{"operation": "CloseHandle", "handle": 60, "success": True, "failure": None}
                    for _ in bridges],
        "final_counter": counter(), "snapshot_n": 0, "events": [], "records_complete": True,
        "records_complete_scope": "snapshot_n_only",
        "final_owned_cleanup": {"ok": True, "remaining_owned_dll": 0, "failed_close_handles": []},
        "limits": {"modules": 128, "events": 256, "name_utf16_units": 1024, "censuses": 20,
            "total_census_path_units": 131072, "image_file_bytes": 536870912, "header_bytes": 65536,
            "sections": 96, "debug_entries": 32, "total_sections": 4096, "total_debug_entries": 4096,
            "codeview_bytes": 4096, "read_bytes": 67108864, "read_calls": 32768, "bridges": 129,
            "stdout_bytes": 8388608, "worst_case_stdout_bytes": 7788672, "cleanup_records": 258,
            "workload_id_bytes": 256, "fixed_storage_bytes": 3493341, "observation_storage_bytes": 2434512,
            "callback_storage_bytes": 1058828, "owner_bytes": 56,
            "storage_bytes_scope": "sum_of_static_objects_excludes_linker_padding_code_and_native_cost"},
        "read_bytes": 2880, "read_calls": 48,
        "census_path_units": sum(len(image["path_utf16_hex"]) // 4 for image in images) * 6,
        "total_sections_reserved": 12, "total_debug_entries_reserved": 0,
        "qualification": False, "claim": "conditional_observed_window_and_path_header_correlation_only",
        "backing_object_proof": False, "code_bytes_proof": False,
        "path_comparison": "exact_utf16_no_normalization", "owned_cleanup_scope": "file_handles_only",
        "callback_lifetime": "process_exit_no_unregister_no_drain", "outer_process_cleanup": "supervisor_only",
        "serialization_exit_observed": False,
    }


def raw_record(report=None, **changes):
    report = native_report() if report is None else report
    envelope = {"kind": "observations", "target": "x86_64-pc-windows-msvc", "process_id": 7,
                "windows_loader_observer_enabled": True, "windows_loader_observer": CAPABILITY,
                "timing_boundary_protocol": "mado-component-timing-v1", "timing_boundaries_enabled": True,
                "workloads": [{"id": "hit-full", "statistical": report["statistical"],
                               "windows_loader_observer": report}]}
    value = {"exit_code": 0, "timed_out": False, "output_limited": False, "cleanup_ok": True,
             "stdout": json.dumps(envelope), "stderr": "".join(
                 f"mado-component-timing-v1[7]: hit-full {phase}\n"
                 for phase in ("row-start", "samples-begin", "samples-end")),
             "duration_seconds": 0.1, "launch_error": None, "process_id": 7}
    value.update(changes)
    return value


def run_observer(*, closure=None, record=None, api=None, child=None):
    api = FileApi() if api is None else api
    record = raw_record() if record is None else record
    with mock.patch.object(observer.sys, "platform", "win32"), \
            mock.patch.object(observer, "_system_identity", return_value=SYSTEM), \
            mock.patch.object(observer, "_Api", return_value=api), \
            mock.patch.object(observer.process_runner, "_validate"), \
            mock.patch.object(observer.process_runner, "run_process", side_effect=child,
                              return_value=record) as runner:
        kwargs = dict(cwd=Path.cwd(), env={"PATH": r"C:\app"}, timeout_seconds=1,
                      output_limit_bytes=8 << 20, cleanup_seconds=1, binding_seconds=10)
        receipt = (observer.discover(ARGV, **kwargs) if closure is None
                   else observer.observe(ARGV, closure=closure, **kwargs))
    return receipt, api, runner


class NativeContractTests(unittest.TestCase):
    def test_raw_complete_report_has_six_owned_cuts_and_no_invented_dyld_records(self):
        result = observer.validate_timing_boundaries(raw_record(), ["hit-full"])
        self.assertTrue(result["ok"], result["reason"])
        self.assertTrue(result["complete"])
        self.assertEqual(result["rows"][0]["in_window_loader_events"], 0)
        self.assertNotIn("dyld_lines", result)

    def test_wrong_owner_row_enablement_and_raw_decode_refuse(self):
        for mutate in (
                lambda v: v.update(process_id=8),
                lambda v: v.update(windows_loader_observer_enabled=False),
                lambda v: v["workloads"][0].update(id="other"),
                lambda v: v["workloads"][0].update(statistical=False),
                lambda v: v["workloads"][0]["windows_loader_observer"].update(pid=8),
                lambda v: v.update(kind="manifest"),
                lambda v: v.update(workloads=[])):
            record = raw_record()
            value = json.loads(record["stdout"])
            mutate(value)
            record["stdout"] = json.dumps(value)
            self.assertFalse(observer.validate_timing_boundaries(record, ["hit-full"])["complete"])
        for override in ({"process_id": None}, {"process_id": True}, {"timed_out": True},
                         {"output_limited": True}, {"cleanup_ok": False}, {"stderr": ""}, {"stdout": "{}\ufffd"}):
            self.assertFalse(observer.validate_timing_boundaries(raw_record(**override), ["hit-full"])["complete"])

    def test_raw_counter_bridges_and_cleanup_override_complete_summary(self):
        changes = [
            lambda r: r["censuses"].pop(),
            lambda r: r["cuts"].update(begin=[1, 2]),
            lambda r: r["censuses"][0]["before"].update(pending_sequences=[1], first=1, last=1),
            lambda r: r["censuses"][0]["after"].update(first=257, last=257),
            lambda r: r["cut_counters"]["begin"].update(first=1, last=1),
            lambda r: r.update(final_counter=counter(1)),
            lambda r: r.update(snapshot_n=1),
            lambda r: r["bridges"].pop(),
            lambda r: r["bridges"][0].update(census_index=0),
            lambda r: r["bridges"][0].update(file_before_complete=False),
            lambda r: r["bridges"][0].update(close_success=False),
            lambda r: r["bridges"][0]["file_after"].update(file_id_hex="ff" * 16),
            lambda r: r["bridges"][0]["memory_pe"].update(checksum=1),
            lambda r: r["bridges"][0].update(mapped_nt_path_after_utf16_hex=observer._hexwide(NT_DLL)),
            lambda r: r["cleanup"].pop(),
            lambda r: r["cleanup"][0].update(success=False),
            lambda r: r["final_owned_cleanup"].update(remaining_owned_dll=1),
            lambda r: r["limits"].update(bridges=384),
            lambda r: r.update(read_calls=1),
            lambda r: r.update(read_bytes=1),
            lambda r: r.update(census_path_units=0),
            lambda r: r["registration"].update(export=1),
            lambda r: r.update(serialization_exit_observed=True),
        ]
        for change in changes:
            report = native_report()
            change(report)
            with self.subTest(change=change):
                self.assertFalse(observer.validate_timing_boundaries(raw_record(report), ["hit-full"])["complete"])

    def test_contract_row_cannot_waive_unknown_event_identity(self):
        report = native_report(statistical=False)
        image = module(DLL, 0x190000000)
        report["events"] = [event(1, 1, image), event(2, 2, image)]
        for index in (4, 5):
            report["censuses"][index]["before"] = counter(2)
            report["censuses"][index]["after"] = counter(2)
        report["cut_counters"]["end"] = counter(2)
        report["window_counter"]["end"] = counter(2)
        report["final_counter"] = counter(2)
        report["snapshot_n"] = 2
        report["window_unchanged"] = False
        result = observer.validate_timing_boundaries(raw_record(report), ["hit-full"])
        self.assertFalse(result["complete"])
        self.assertIn("without_observed_bridge", result["reason"])


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.discovery, _, _ = run_observer()

    def closure(self):
        result = observer.validate_closure([self.discovery])
        self.assertTrue(result["ok"], result["reason"])
        return result["closure"]

    def test_discovery_and_measurement_bind_files_without_relaunch_or_late_repair(self):
        self.assertTrue(observer.validate_discovery(self.discovery)["provenance_complete"],
                        self.discovery["result"]["reason"])
        closure = self.closure()
        api = FileApi()

        def child(*args, **kwargs):
            files = [path for path, directory in api.opened.values() if not directory]
            self.assertEqual(set(files), {MAIN, NTDLL})
            self.assertFalse(api.closed)
            return raw_record()

        receipt, _, runner = run_observer(closure=closure, api=api, child=child)
        runner.assert_called_once()
        self.assertEqual(len(api.closed), len(api.opened))
        verdict = observer.validate_observation(receipt, closure)
        self.assertTrue(verdict["ok"], verdict["reason"])
        self.assertEqual({image["class"] for image in receipt["images"]}, {"file"})

    def test_prelaunch_hash_change_runs_no_child_and_closes_every_handle(self):
        api = FileApi()
        api.data = api.data[:-1] + b"x"
        receipt, _, runner = run_observer(closure=self.closure(), api=api)
        runner.assert_not_called()
        self.assertEqual(len(api.closed), len(api.opened))
        self.assertIn("prelaunch_file_changed", receipt["result"]["reason"])

    def test_postlaunch_hash_change_and_native_bridge_rebinding_refuse(self):
        closure = self.closure()
        api = FileApi()

        def child(*args, **kwargs):
            api.data = api.data[:-1] + b"x"
            return raw_record()

        receipt, _, runner = run_observer(closure=closure, api=api, child=child)
        runner.assert_called_once()
        self.assertFalse(observer.validate_observation(receipt, closure)["ok"])
        self.assertEqual(len(api.closed), len(api.opened))
        report = native_report()
        for b in report["bridges"]:
            b["file_before"]["file_id_hex"] = "ee" * 16
            b["file_after"]["file_id_hex"] = "ee" * 16
        receipt, _, _ = run_observer(closure=closure, record=raw_record(report))
        self.assertIn("bridge_pin_identity_mismatch", receipt["result"]["reason"])

    def test_re_read_rejects_tampered_source_raw_hash_and_pin_cleanup(self):
        for change in (
                lambda r: r["method_identity"].update(sha256="0" * 64),
                lambda r: r["raw_sha256"].update(stdout="0" * 64),
                lambda r: r["record"].update(process_id=8),
                lambda r: r["bindings"]["post"].pop(),
                lambda r: r["bindings"]["post"][0].update(sha256="bad"),
                lambda r: r["pin_cleanup"].pop(),
                lambda r: r["pin_handles"][0]["directory_facts"].update(attributes=0x410),
                lambda r: r["pin_cleanup"][0].update(success=False, error="close_failed"),
                lambda r: r["images"][0].update(sha256="0" * 64)):
            receipt = copy.deepcopy(self.discovery)
            change(receipt)
            receipt["result"]["status"] = "complete"
            receipt["provenance"]["complete"] = True
            receipt["cleanup_ok"] = True
            with self.subTest(change=change):
                self.assertFalse(observer.validate_discovery(receipt)["provenance_complete"])

    def test_close_failure_and_unknown_cleanup_never_become_clean(self):
        api = FileApi()
        api.fail_close.add(102)
        receipt, _, _ = run_observer(api=api)
        self.assertFalse(receipt["cleanup_ok"])
        self.assertEqual(len(api.closed), len(api.opened))
        report = native_report()
        report["cleanup"][0].update(success=False, failure={"operation": "CloseHandle", "raw_code": 6, "detail": 0})
        receipt, _, _ = run_observer(record=raw_record(report))
        self.assertFalse(receipt["cleanup_ok"])
        receipt, _, _ = run_observer(record=raw_record(stdout="{}"))
        self.assertIsNone(receipt["cleanup_ok"])

    def test_nonzero_exit_can_retain_identity_but_never_forms_a_closure(self):
        receipt, _, runner = run_observer(record=raw_record(exit_code=3))
        runner.assert_called_once()
        self.assertTrue(observer.validate_discovery(receipt)["provenance_complete"])
        self.assertEqual(observer.validate_discovery(receipt)["exit_code"], 3)
        self.assertFalse(observer.validate_closure(receipt)["ok"])


def known_unload_report(*, statistical):
    report = native_report(statistical=statistical)
    dll = module(DLL, 0x190000000)
    for index in range(4):
        report["censuses"][index]["modules"].append(copy.deepcopy(dll))
        report["censuses"][index]["needed_bytes"] += 8
    for index in (4, 5):
        report["censuses"][index]["before"] = counter(1)
        report["censuses"][index]["after"] = counter(1)
    bridges = []
    for stage, index in (("initial", 1), ("begin", 3), ("end", 5)):
        for position, image in enumerate(report["censuses"][index]["modules"]):
            path = observer._wide(image["path_utf16_hex"])
            native = r"\Device\HarddiskVolume3" + path[2:]
            bridges.append(dict(bridge(image, index, stage, native), module_index=position))
    report["bridges"] = bridges
    report["cleanup"] = [{"operation": "CloseHandle", "handle": 60, "success": True, "failure": None}
                         for _ in bridges]
    report["cut_counters"]["end"] = counter(1)
    report["window_counter"]["end"] = counter(1)
    report["final_counter"] = counter(1)
    report["snapshot_n"] = 1
    report["events"] = [event(1, 2, dll)]
    report["window_unchanged"] = False
    report["read_calls"] = 64
    report["read_bytes"] = 3840
    report["total_sections_reserved"] = 16
    report["census_path_units"] = sum(len(m["path_utf16_hex"]) // 4
                                     for c in report["censuses"] for m in c["modules"])
    return report


class RowPolicyTests(unittest.TestCase):
    def test_known_contract_change_keeps_identity_but_statistical_change_refuses(self):
        report = known_unload_report(statistical=False)
        timing = observer.validate_timing_boundaries(raw_record(report), ["hit-full"])
        self.assertTrue(timing["complete"], timing["reason"])
        self.assertFalse(timing["ok"])
        self.assertEqual(timing["rows"][0]["in_window_loader_events"], 1)
        discovery, _, _ = run_observer(record=raw_record(report))
        closure_result = observer.validate_closure(discovery)
        self.assertTrue(closure_result["ok"], closure_result["reason"])
        closure = closure_result["closure"]
        receipt, _, runner = run_observer(record=raw_record(report), closure=closure)
        runner.assert_called_once()
        verdict = observer.validate_observation(receipt, closure)
        self.assertTrue(verdict["ok"], verdict["reason"])
        statistical = known_unload_report(statistical=True)
        receipt, _, _ = run_observer(record=raw_record(statistical), closure=closure)
        self.assertFalse(observer.validate_observation(receipt, closure)["ok"])
        self.assertEqual(receipt["result"]["reason"], "measurement_workload_policy_changed")
        receipt, _, _ = run_observer(record=raw_record(statistical))
        self.assertFalse(observer.validate_discovery(receipt)["provenance_complete"])
        self.assertEqual(receipt["result"]["reason"], "statistical_window_changed")

    def test_nonzero_raw_event_flags_cannot_be_certified_by_clean_counters(self):
        report = known_unload_report(statistical=False)
        discovery, _, _ = run_observer(record=raw_record(report))
        closure = observer.validate_closure(discovery)["closure"]
        report["events"][0]["flags"] = 1
        receipt, _, _ = run_observer(record=raw_record(report), closure=closure)
        verdict = observer.validate_observation(receipt, closure)
        self.assertFalse(verdict["provenance_complete"])
        self.assertFalse(verdict["ok"])
        self.assertIn("event_flags_invalid", verdict["reason"])
        self.assertFalse(observer.validate_timing_boundaries(raw_record(report), ["hit-full"])["complete"])

    def test_new_file_is_not_late_bound_during_measurement(self):
        discovery, _, _ = run_observer(record=raw_record(native_report(statistical=False)))
        closure = observer.validate_closure(discovery)["closure"]
        receipt, api, runner = run_observer(
            record=raw_record(known_unload_report(statistical=False)), closure=closure)
        runner.assert_called_once()
        self.assertNotIn(DLL, [path for path, directory in api.opened.values() if not directory])
        self.assertFalse(observer.validate_observation(receipt, closure)["ok"])
        self.assertIn("observed_image_not_prebound", receipt["result"]["reason"])

    def test_postbinding_and_closure_hash_tampering_do_not_recertify_measurement(self):
        discovery, _, _ = run_observer()
        closure = observer.validate_closure(discovery)["closure"]
        receipt, _, _ = run_observer(closure=closure)
        for change in (
                lambda r: r["bindings"]["pre"].clear(),
                lambda r: r["bindings"]["post"][0].update(handle=999),
                lambda r: r["bindings"]["post"][0]["file_after"].update(change_time=99),
                lambda r: r.update(pin_read_bytes=0),
                lambda r: r.update(closure_sha256="f" * 64)):
            mutated = copy.deepcopy(receipt)
            change(mutated)
            self.assertFalse(observer.validate_observation(mutated, closure)["ok"])
        changed = copy.deepcopy(closure)
        changed["images"][0]["sha256"] = "f" * 64
        self.assertFalse(observer.validate_observation(receipt, changed)["ok"])


class PeBridgeTests(unittest.TestCase):
    def test_relocation_sensitive_imagebase_alone_does_not_refuse_a_complete_native_row(self):
        report = native_report()
        for value in report["bridges"]:
            value["memory_pe"]["preferred_base"] += 0x100000
        result = observer.validate_timing_boundaries(raw_record(report), ["hit-full"])
        self.assertTrue(result["ok"], result["reason"])

    def test_each_nonaddress_pe_scalar_and_section_identity_remains_required(self):
        for field in ("machine", "timestamp", "characteristics", "magic", "entry_rva", "image_size",
                      "headers_size", "checksum", "subsystem", "dll_characteristics"):
            report = native_report()
            report["bridges"][0]["memory_pe"][field] += 1
            with self.subTest(field=field):
                self.assertFalse(observer.validate_timing_boundaries(raw_record(report), ["hit-full"])["complete"])
        report = native_report()
        report["bridges"][0]["memory_pe"]["sections"][0]["characteristics"] ^= 0x10000000
        self.assertFalse(observer.validate_timing_boundaries(raw_record(report), ["hit-full"])["complete"])


if __name__ == "__main__":
    unittest.main()
