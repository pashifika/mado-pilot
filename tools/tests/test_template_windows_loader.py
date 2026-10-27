"""Loader capability provenance stays separate from workload semantics."""

import copy
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

from tools.tests import test_template_matching_efficiency as fixtures
from tools.tests import test_windows_loader_callback as linked
from tools.tests import test_windows_loader_observer as native

collector = fixtures.collector
TARGET = "x86_64-pc-windows-msvc"
WORKLOAD = "selection-k16-dense1080p-keepall"


def windows(value, *, enabled=False):
    value = copy.deepcopy(value)
    value.update(target=TARGET, process_id=7,
                 windows_loader_observer=linked.fixture()[1],
                 windows_loader_observer_enabled=enabled,
                 timing_boundary_protocol="mado-component-timing-v1",
                 timing_boundaries_enabled=enabled)
    value["apparatus"]["source_files"][collector.WINDOWS_OBSERVER_SOURCE] = "f" * 64
    value["loaded_native_runtime"] = {
        "method": "K32EnumProcessModules-GetModuleFileNameW",
        "images": [{"path": r"C:\app\opencv_world.dll", "sha256": "e" * 64}]}
    for row in value["workloads"]:
        row["windows_loader_observer"] = None
        if "retained_loop_cpu" in row:
            row["retained_loop_cpu"].update(source="GetProcessTimes-current-process", source_unit_ns=100)
        if enabled:
            report = native.native_report(statistical=row["statistical"])
            report["workload_id"] = row["id"]
            row["windows_loader_observer"] = report
    return value


def manifest(value):
    return collector.manifest_identity(value, value["target"], "4.12.0", 4, "normal")


class EnvironmentTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == "win32", "Windows case-insensitive process environment")
    def test_windows_environment_reaches_strict_loader_binding(self):
        with mock.patch.dict(os.environ, {
            "SystemRoot": r"C:\Windows", "Path": r"C:\Tools", "SECRET_TOKEN": "excluded",
        }, clear=True):
            environment = collector.environment()
        self.assertEqual(environment, {"SYSTEMROOT": r"C:\Windows", "PATH": r"C:\Tools"})
        self.assertEqual(collector.windows_images.loader_environment(environment), environment)

    @unittest.skipIf(sys.platform == "win32", "POSIX case-sensitive process environment")
    def test_posix_environment_preserves_distinct_authored_names(self):
        with mock.patch.dict(os.environ, {
            "SystemRoot": "mixed", "SYSTEMROOT": "upper", "PATH": "/usr/bin",
            "SECRET_TOKEN": "excluded",
        }, clear=True):
            environment = collector.environment()
        self.assertEqual(environment, {
            "SystemRoot": "mixed", "SYSTEMROOT": "upper", "PATH": "/usr/bin",
        })


class ManifestTests(unittest.TestCase):
    def test_runtime_null_fields_do_not_change_apple_session_declarations(self):
        declaration = fixtures.v4_manifest()
        declaration.update(windows_loader_observer=None, windows_loader_observer_enabled=False, process_id=7)
        for row in declaration["workloads"]:
            row["windows_loader_observer"] = None
        parsed = manifest(declaration)
        self.assertEqual(parsed, manifest(fixtures.v4_manifest()))
        self.assertEqual(len(parsed["workloads"]), 54)
        declaration["workloads"][-1]["windows_loader_observer"] = {"complete": True}
        with self.assertRaisesRegex(ValueError, "manifest cannot contain"):
            manifest(declaration)

    def test_windows_runtime_report_is_not_a_frozen_workload_field(self):
        declaration = windows(fixtures.v4_manifest())
        frozen = manifest(declaration)
        observed = windows(fixtures.v4_payload(WORKLOAD, warmups=1, count=3), enabled=True)
        raw = native.raw_record()
        raw["stdout"] = json.dumps(observed)
        raw["stderr"] = raw["stderr"].replace("hit-full", WORKLOAD)
        attempt = {"task": {"workload": WORKLOAD},
                   "argv": [native.MAIN, "--timing-boundaries", "--windows-loader-observer",
                            "--workload", WORKLOAD]}
        collector.validate_timing_attempt(attempt, frozen, raw, retain=True)
        result = collector.observations(observed, frozen, WORKLOAD, True, timing_boundaries=True)
        self.assertEqual(result["retained_loop_cpu"]["sample_count"], 3)
        self.assertNotIn("windows_loader_observer", frozen["workloads"][0])
        self.assertTrue(attempt["timing_observation"]["complete"])
        observed["workloads"][0]["windows_loader_observer"] = None
        with self.assertRaisesRegex(ValueError, "report presence mismatch"):
            collector.observation_identity(observed, frozen, WORKLOAD, timing_boundaries=True)

    def test_default_off_rejects_an_injected_runtime_report(self):
        frozen = manifest(windows(fixtures.v4_manifest()))
        value = windows(fixtures.v4_payload(WORKLOAD, warmups=1, count=3))
        value["workloads"][0]["windows_loader_observer"] = native.native_report()
        with self.assertRaisesRegex(ValueError, "report presence mismatch"):
            collector.observation_identity(value, frozen, WORKLOAD)

    def test_callback_addresses_are_role_specific_but_layout_is_not(self):
        baseline = manifest(windows(fixtures.v4_manifest()))
        candidate = copy.deepcopy(baseline)
        capability = candidate["windows_loader_observer"]["callback"]
        capability["rva"] += 4096
        for static in capability["statics"].values():
            static["rva"] += 8192
        self.assertNotEqual(baseline, candidate)
        self.assertEqual(collector.common_manifest(baseline), collector.common_manifest(candidate))
        capability["statics"]["records"]["bytes"] -= 1
        with self.assertRaisesRegex(ValueError, "callback-static-layout"):
            collector.common_manifest(candidate)

    def test_missing_source_or_capability_cannot_create_a_windows_method(self):
        declaration = windows(fixtures.v4_manifest())
        for remove_source in (True, False):
            value = copy.deepcopy(declaration)
            if remove_source:
                del value["apparatus"]["source_files"][collector.WINDOWS_OBSERVER_SOURCE]
            else:
                value["windows_loader_observer"] = None
            with self.subTest(remove_source=remove_source), self.assertRaisesRegex(ValueError, "source set differ"):
                manifest(value)
        value = copy.deepcopy(declaration)
        value["workloads"][0].pop("windows_loader_observer")
        with self.assertRaisesRegex(ValueError, "manifest observer field missing"):
            manifest(value)

    def test_missing_capability_refuses_before_a_qualifying_command(self):
        value = windows(fixtures.v4_manifest())
        value.pop("windows_loader_observer")
        value["apparatus"]["source_files"].pop(collector.WINDOWS_OBSERVER_SOURCE)
        frozen = manifest(value)
        identity = {"path": r"C:\app\bench.exe", "build": {"target": TARGET, "benchmark_manifest": frozen}}
        with self.assertRaisesRegex(ValueError, "callback-capability-protocol"):
            collector.command(identity, {"workload": WORKLOAD}, "smoke", 4, timing_boundaries=True)


class ExecutableAdmissionTests(unittest.TestCase):
    def test_actual_executable_hash_is_checked_before_linked_byte_inspection(self):
        data, capability, _ = linked.fixture()
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "bench.exe"
            executable.write_bytes(data)
            identity = {"path": str(executable), "sha256": hashlib.sha256(data).hexdigest(),
                        "build": {"benchmark_manifest": {"windows_loader_observer": capability}}}
            original = collector.inspect_windows_callback(identity)
            changed = bytearray(data)
            changed[0x40] = 1
            executable.write_bytes(changed)
            with self.assertRaisesRegex(ValueError, "executable.*changed|hash"):
                collector.inspect_windows_callback(identity)
            identity["sha256"] = hashlib.sha256(changed).hexdigest()
            current = collector.inspect_windows_callback(identity)
            self.assertNotEqual(current["executable_sha256"], original["executable_sha256"])
            self.assertEqual(current["callback_sha256"], original["callback_sha256"])


if __name__ == "__main__":
    unittest.main()
