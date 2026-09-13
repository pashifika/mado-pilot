"""Private image-channel bounds using inert files and owned Python children only."""

import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import run_replay


class ImageReport(unittest.TestCase):
    @unittest.skipUnless(sys.platform == "win32", "Windows extended path identity")
    def test_normal_and_extended_paths_share_the_approved_image_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = root / "inert-image"
            image.write_bytes(b"reviewed bytes, never loaded")
            (root / "apphelp.dll").write_bytes(b"owned OS-location stand-in, never loaded")
            extended = Path("\\\\?\\" + str(image.resolve()))
            required = run_replay.identity(image)
            approved = {required["path"]: required["sha256"]}
            manifest = root / "approved.json"
            run_replay.write_record(manifest, approved)
            approved, _ = run_replay.native_manifest(manifest)
            report = root / "images"
            report.write_text(f"{image.resolve()}\n{extended}\n", encoding="utf-8")

            with patch.object(run_replay, "_windows_system_directory", return_value=root):
                observed = run_replay.observe_dependencies(report, approved)
            self.assertEqual(observed["observed"], approved)
            self.assertTrue(observed["matched"], observed)
            self.assertEqual(run_replay.identity(extended), required)
            with self.assertRaises(ValueError):
                run_replay.verify_native_manifest({
                    str(image.resolve()): required["sha256"], str(extended): required["sha256"],
                })

    @unittest.skipUnless(sys.platform == "win32", "Windows literal trailing name components")
    def test_extended_trailing_dot_and_space_keep_distinct_file_identities(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            normal = root / "inert-image"
            normal.write_bytes(b"ordinary name")
            (root / "apphelp.dll").write_bytes(b"owned OS-location stand-in, never loaded")
            extended_root = Path("\\\\?\\" + str(root.resolve()))
            literal_images = (
                (extended_root / "inert-image.", b"literal trailing dot"),
                (extended_root / "inert-image ", b"literal trailing space"),
            )
            required = run_replay.identity(normal)
            approved = {required["path"]: required["sha256"]}
            try:
                for image, content in literal_images:
                    image.write_bytes(content)
                    data, entry = run_replay.read_document(image)
                    self.assertEqual(data, content)
                    self.assertFalse(image.samefile(normal))
                    self.assertTrue(Path(entry["path"]).samefile(image))
                    self.assertNotIn(entry["path"], approved)
                    approved[entry["path"]] = entry["sha256"]
                report = root / "images"
                report.write_text("\n".join(approved) + "\n", encoding="utf-8")
                with patch.object(run_replay, "_windows_system_directory", return_value=root):
                    observed = run_replay.observe_dependencies(report, approved)
                self.assertTrue(observed["matched"], observed)
                self.assertEqual(observed["observed"], approved)
            finally:
                for image, _ in literal_images:
                    image.unlink(missing_ok=True)

    @unittest.skipUnless(sys.platform == "win32", "Windows read-only system-directory lookup")
    def test_os_system_directory_lookup_returns_an_existing_absolute_directory(self):
        directory = run_replay._windows_system_directory()
        self.assertTrue(directory.is_absolute())
        self.assertTrue(directory.is_dir())

    @unittest.skipUnless(sys.platform == "darwin", "Darwin image paths require native absolute paths")
    def test_system_images_do_not_consume_the_non_system_manifest_limit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = root / "inert-image"
            image.write_bytes(b"identity only, never loaded")
            report = root / "images"
            lines = [f"dyld[1]: <uuid> /usr/lib/system-{index}.dylib" for index in range(600)]
            lines.append(f"dyld[1]: <uuid> {image.resolve()}")
            report.write_text("\n".join(lines) + "\n", encoding="utf-8")
            approved = {str(image.resolve()): hashlib.sha256(image.read_bytes()).hexdigest()}
            with patch.object(run_replay.platform, "system", return_value="Darwin"):
                observed = run_replay.observe_dependencies(report, approved)
            self.assertTrue(observed["matched"], observed)
            self.assertEqual(observed["observed"], approved)

    def test_saturated_or_partial_report_is_not_dependency_proof(self):
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "images"
            for data in (b"dyld[1]: <uuid> /incomplete", b"\n" * run_replay.MAX_DOCUMENT_BYTES):
                report.write_bytes(data)
                with patch.object(run_replay.platform, "system", return_value="Darwin"):
                    observed = run_replay.observe_dependencies(report, {"/unused": "a" * 64})
                self.assertFalse(observed["matched"])
                self.assertEqual(observed["observed"], {})

    def test_ambient_dyld_diagnostics_are_not_inherited(self):
        with self.assertRaises(ValueError):
            run_replay.dependency_environment({"DYLD_PRINT_ENV": "1"}, Path("unused"))

    @unittest.skipUnless(sys.platform == "darwin", "Darwin exec/report contract")
    def test_owned_child_keeps_dyld_out_of_ordinary_output_and_enforces_report_cap(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith(("DYLD_PRINT_", "MADO_PILOT_"))}
            environment["PYTHONDONTWRITEBYTECODE"] = "1"
            report = root / "normal.images"
            child = run_replay.dependency_environment(environment, report)
            command = run_replay.observed_command([
                sys.executable, "-c", "import os; print('ordinary-out'); os.write(2,b'ordinary-err\\n')",
            ])
            result = run_replay.run_process(command, cwd=run_replay.ROOT, env=child,
                                           timeout_seconds=10, output_limit_bytes=4096, cleanup_seconds=5)
            self.assertEqual(result["exit_code"], 0, result)
            self.assertTrue(result["cleanup_ok"], result)
            self.assertEqual(result["stdout"], "ordinary-out\n")
            self.assertEqual(result["stderr"], "ordinary-err\n")
            self.assertIn(b"dyld[", report.read_bytes())
            self.assertEqual(stat.S_IMODE(report.stat().st_mode), 0o600)

            capped = root / "capped.images"
            child = run_replay.dependency_environment(environment, capped)
            command = run_replay.observed_command([
                sys.executable, "-c",
                "import os; f=open(os.environ['MADO_PILOT_OCR_DEPENDENCY_REPORT'],'ab'); f.write(b'x'*(1048576+64)); f.close()",
            ])
            result = run_replay.run_process(command, cwd=run_replay.ROOT, env=child,
                                           timeout_seconds=10, output_limit_bytes=4096, cleanup_seconds=5)
            self.assertNotEqual(result["exit_code"], 0, result)
            self.assertTrue(result["cleanup_ok"], result)
            self.assertLessEqual(capped.stat().st_size, run_replay.MAX_DOCUMENT_BYTES)
            observed = run_replay.observe_dependencies(capped, {"/unused": "a" * 64})
            self.assertFalse(observed["matched"])

    @unittest.skipUnless(sys.platform == "darwin", "Darwin report isolation")
    def test_report_budget_preserves_inherited_limits_and_unrelated_writes(self):
        import resource

        inherited = resource.getrlimit(resource.RLIMIT_FSIZE)
        size = 2 * run_replay.MAX_DOCUMENT_BYTES
        if any(limit != resource.RLIM_INFINITY and limit < size for limit in inherited):
            self.skipTest("inherited host limit already forbids the unrelated write")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith(("DYLD_PRINT_", "MADO_PILOT_"))}
            environment["PYTHONDONTWRITEBYTECODE"] = "1"
            report, unrelated = root / "images", root / "unrelated"
            child = run_replay.dependency_environment(environment, report)
            command = run_replay.observed_command([
                sys.executable, "-c",
                "import json,resource,signal,sys; from pathlib import Path; "
                "signal.signal(signal.SIGXFSZ,signal.SIG_DFL); "
                "print(json.dumps(resource.getrlimit(resource.RLIMIT_FSIZE)),flush=True); "
                "Path(sys.argv[1]).write_bytes(b'x'*int(sys.argv[2]))",
                str(unrelated), str(size),
            ])
            result = run_replay.run_process(command, cwd=run_replay.ROOT, env=child,
                                           timeout_seconds=10, output_limit_bytes=4096, cleanup_seconds=5)
            self.assertEqual(result["exit_code"], 0, result)
            self.assertTrue(result["cleanup_ok"], result)
            self.assertEqual(json.loads(result["stdout"]), list(inherited))
            self.assertEqual(unrelated.read_bytes(), b"x" * size)
            self.assertIn(b"dyld[", report.read_bytes())
            self.assertLess(report.stat().st_size, run_replay.MAX_DOCUMENT_BYTES)

    @unittest.skipUnless(sys.platform == "darwin", "Darwin collector process ownership")
    def test_native_signal_and_outer_timeout_remain_distinct(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith(("DYLD_PRINT_", "MADO_PILOT_"))}
            environment["PYTHONDONTWRITEBYTECODE"] = "1"
            for label, code, timeout in (
                ("signal", "import os,signal; os.kill(os.getpid(),signal.SIGTERM)", 10),
                ("timeout", "import time; time.sleep(20)", 0.5),
            ):
                with self.subTest(label=label):
                    child = run_replay.dependency_environment(environment, root / f"{label}.images")
                    command = run_replay.observed_command([sys.executable, "-c", code])
                    result = run_replay.run_process(command, cwd=run_replay.ROOT, env=child,
                                                   timeout_seconds=timeout, output_limit_bytes=4096,
                                                   cleanup_seconds=5)
                    self.assertTrue(result["cleanup_ok"], result)
                    self.assertEqual(result["timed_out"], label == "timeout", result)
                    if label == "signal":
                        self.assertEqual(result["exit_code"], -signal.SIGTERM, result)


    @unittest.skipUnless(sys.platform == "darwin", "Darwin final pipe drain")
    def test_final_records_after_select_timeout_are_drained(self):
        import textwrap

        child_code = textwrap.dedent("""
            import os, sys, time
            from pathlib import Path
            gate = Path(sys.argv[1])
            deadline = time.monotonic() + 3
            while not gate.exists():
                if time.monotonic() >= deadline:
                    raise SystemExit(9)
                time.sleep(0.001)
            os.write(int(os.environ["MADO_PILOT_OCR_DEPENDENCY_REPORT"].rsplit("/", 1)[1]),
                     b"collector-tail\\n")
        """)
        driver_code = textwrap.dedent("""
            import importlib.util, subprocess, sys
            from pathlib import Path
            spec = importlib.util.spec_from_file_location("collector", sys.argv[1])
            collector = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(collector)
            gate = Path(sys.argv[2])
            processes = []
            popen, select = subprocess.Popen, collector.select.select
            def launch(*args, **kwargs):
                process = popen(*args, **kwargs)
                processes.append(process)
                return process
            def delayed_select(*args):
                ready = select(*args)
                if not ready[0] and not gate.exists():
                    gate.touch()
                    processes[0].wait(timeout=3)
                return ready
            collector.subprocess.Popen = launch
            collector.select.select = delayed_select
            sys.argv = [sys.argv[1], sys.executable, "-c", sys.argv[3], str(gate)]
            raise SystemExit(collector.main())
        """)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith(("DYLD_PRINT_", "MADO_PILOT_"))}
            environment["PYTHONDONTWRITEBYTECODE"] = "1"
            report = root / "images"
            environment = run_replay.dependency_environment(environment, report)
            command = [
                sys.executable, "-c", driver_code,
                str(run_replay.ROOT / "tools/ocr-text-watch/darwin_image_report.py"),
                str(root / "gate"), child_code,
            ]
            result = run_replay.run_process(command, cwd=run_replay.ROOT, env=environment,
                                           timeout_seconds=10, output_limit_bytes=4096, cleanup_seconds=5)
            self.assertEqual(result["exit_code"], 0, result)
            self.assertTrue(result["cleanup_ok"], result)
            self.assertTrue(report.read_bytes().endswith(b"collector-tail\n"))


class OsManagedImagePresence(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.system_directory = self.root / "owned-system"
        self.system_directory.mkdir()
        self.paths = {
            "executable": self.root / "required.exe",
            "apphelp": self.system_directory / "apphelp.dll",
            "other": self.system_directory / "other.dll",
            "outside": self.root / "apphelp.dll",
        }
        self.contents = {
            "executable": b"owned executable, never executed",
            "apphelp": b"owned apphelp stand-in, never loaded",
            "other": b"owned other image, never loaded",
            "outside": b"owned apphelp stand-in, never loaded",
        }
        for name, path in self.paths.items():
            path.write_bytes(self.contents[name])
        self.identities = {name: run_replay.identity(path) for name, path in self.paths.items()}
        self.exclusions = [self.identities["apphelp"]["path"]]
        self.report = self.root / "images"
        location = patch.object(run_replay, "_windows_system_directory", return_value=self.system_directory)
        self.location = location.start()
        self.addCleanup(location.stop)
        system = patch.object(run_replay.platform, "system", return_value="Windows")
        self.system = system.start()
        self.addCleanup(system.stop)

    def images(self, *names):
        return {self.identities[name]["path"]: self.identities[name]["sha256"] for name in names}

    def observe(self, approved, *names, darwin=False):
        paths = [self.identities[name]["path"] for name in names]
        if darwin:
            paths = [f"dyld[1]: <uuid> {path}" for path in paths]
        self.report.write_text("\n".join(paths) + "\n", encoding="utf-8")
        return run_replay.observe_dependencies(self.report, approved)

    def test_missing_os_apphelp_preserves_observed_images_and_matches(self):
        approved = self.images("executable", "apphelp")
        observed = self.observe(approved, "executable")
        self.assertTrue(observed["matched"], observed)
        self.assertEqual(observed["observed"], self.images("executable"))
        self.assertEqual(observed["os_managed_presence_exclusions"], self.exclusions)
        self.assertEqual(observed["report"], run_replay.identity(self.report))
        self.assertEqual(approved, self.images("executable", "apphelp"))

    def test_extra_os_apphelp_preserves_observed_images_and_matches(self):
        approved = self.images("executable")
        observed = self.observe(approved, "executable", "apphelp")
        self.assertTrue(observed["matched"], observed)
        self.assertEqual(observed["observed"], self.images("executable", "apphelp"))
        self.assertEqual(observed["os_managed_presence_exclusions"], self.exclusions)
        self.assertEqual(approved, self.images("executable"))

    def test_same_named_dll_elsewhere_is_not_presence_exempt(self):
        for direction, approved, reported in (
            ("missing", ("executable", "outside"), ("executable", "apphelp")),
            ("extra", ("executable", "apphelp"), ("executable", "outside")),
        ):
            with self.subTest(direction=direction):
                observed = self.observe(self.images(*approved), *reported)
                self.assertFalse(observed["matched"], observed)
                self.assertNotIn("error_kind", observed)
                self.assertEqual(observed["observed"], self.images(*reported))
                self.assertEqual(observed["os_managed_presence_exclusions"], self.exclusions)

    def test_other_system_image_presence_remains_strict(self):
        for direction, approved, reported in (
            ("missing", ("executable", "other"), ("executable", "apphelp")),
            ("extra", ("executable", "apphelp"), ("executable", "other")),
        ):
            with self.subTest(direction=direction):
                observed = self.observe(self.images(*approved), *reported)
                self.assertFalse(observed["matched"], observed)
                self.assertNotIn("error_kind", observed)
                self.assertEqual(observed["observed"], self.images(*reported))

    def test_missing_required_executable_is_not_excused_by_apphelp_presence(self):
        observed = self.observe(self.images("executable"), "apphelp")
        self.assertFalse(observed["matched"], observed)
        self.assertNotIn("error_kind", observed)
        self.assertEqual(observed["observed"], self.images("apphelp"))

    def test_common_image_hash_mismatch_is_not_presence_exempt(self):
        for name in ("apphelp", "other"):
            with self.subTest(image=name):
                approved = self.images("executable", name)
                changed = b"changed owned image before observation"
                self.paths[name].write_bytes(changed)

                def restore_declared_bytes():
                    # Keep both real hash reads: the OS-location boundary lets
                    # the final manifest match while the observed bytes differ.
                    self.paths[name].write_bytes(self.contents[name])
                    return self.system_directory

                self.location.side_effect = restore_declared_bytes
                observed = self.observe(approved, "executable", name)
                self.assertFalse(observed["matched"], observed)
                self.assertNotIn("error_kind", observed)
                self.assertEqual(observed["observed"][self.identities[name]["path"]],
                                 hashlib.sha256(changed).hexdigest())
                self.assertEqual(run_replay.verify_native_manifest(approved), approved)

    def test_declared_apphelp_mutation_still_fails_when_not_observed(self):
        approved = self.images("executable", "apphelp")
        self.paths["apphelp"].write_bytes(b"mutated owned declaration, never loaded")
        with self.assertRaises(ValueError):
            run_replay.verify_native_manifest(approved)
        observed = self.observe(approved, "executable")
        self.assertFalse(observed["matched"], observed)
        self.assertEqual(observed["error_kind"], "ValueError")
        self.assertEqual(observed["observed"], self.images("executable"))
        self.assertEqual(observed["os_managed_presence_exclusions"], self.exclusions)

    def test_os_location_failure_fails_closed_without_discarding_observed_facts(self):
        self.location.side_effect = OSError("owned OS-location failure")
        approved = self.images("executable")
        observed = self.observe(approved, "executable")
        self.assertFalse(observed["matched"], observed)
        self.assertEqual(observed["error_kind"], "OSError")
        self.assertEqual(observed["observed"], approved)
        self.assertEqual(observed["report"], run_replay.identity(self.report))
        self.assertEqual(observed["os_managed_presence_exclusions"], [])

    def test_uncanonicalizable_os_apphelp_fails_closed(self):
        self.paths["apphelp"].unlink()
        approved = self.images("executable")
        observed = self.observe(approved, "executable")
        self.assertFalse(observed["matched"], observed)
        self.assertEqual(observed["error_kind"], "FileNotFoundError")
        self.assertEqual(observed["observed"], approved)
        self.assertEqual(observed["os_managed_presence_exclusions"], [])

    @unittest.skipUnless(sys.platform == "darwin", "Darwin report paths require native absolute paths")
    def test_darwin_does_not_exempt_apphelp_presence(self):
        self.system.return_value = "Darwin"
        self.location.side_effect = AssertionError("Darwin must not request a Windows system directory")
        for direction, approved, reported in (
            ("missing", ("executable", "apphelp"), ("executable",)),
            ("extra", ("executable",), ("executable", "apphelp")),
        ):
            with self.subTest(direction=direction):
                observed = self.observe(self.images(*approved), *reported, darwin=True)
                self.assertFalse(observed["matched"], observed)
                self.assertNotIn("error_kind", observed)
                self.assertEqual(observed["observed"], self.images(*reported))
                self.assertEqual(observed["os_managed_presence_exclusions"], [])

    @unittest.skipUnless(sys.platform == "win32", "Windows literal trailing name components")
    def test_literal_apphelp_suffixes_are_not_presence_exempt(self):
        extended_root = Path("\\\\?\\" + str(self.system_directory.resolve()))
        for suffix in (".", " "):
            with self.subTest(suffix=suffix):
                literal = extended_root / f"apphelp.dll{suffix}"
                self.addCleanup(literal.unlink, missing_ok=True)
                literal.write_bytes(self.contents["apphelp"])
                self.identities["literal"] = run_replay.identity(literal)
                observed = self.observe(self.images("executable"), "executable", "literal")
                self.assertFalse(observed["matched"], observed)
                self.assertNotIn("error_kind", observed)
                self.assertEqual(observed["observed"], self.images("executable", "literal"))
                self.assertNotIn(self.identities["literal"]["path"], observed["os_managed_presence_exclusions"])


if __name__ == "__main__":
    unittest.main()
