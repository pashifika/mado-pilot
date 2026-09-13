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
            extended = Path("\\\\?\\" + str(image.resolve()))
            required = run_replay.identity(image)
            approved = {required["path"]: required["sha256"]}
            manifest = root / "approved.json"
            run_replay.write_record(manifest, approved)
            approved, _ = run_replay.native_manifest(manifest)
            report = root / "images"
            report.write_text(f"{image.resolve()}\n{extended}\n", encoding="utf-8")

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
                observed = run_replay.observe_dependencies(report, approved)
                self.assertTrue(observed["matched"], observed)
                self.assertEqual(observed["observed"], approved)
            finally:
                for image, _ in literal_images:
                    image.unlink(missing_ok=True)

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


if __name__ == "__main__":
    unittest.main()
