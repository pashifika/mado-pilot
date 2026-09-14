"""Bounded native-runner evidence using owned inert Python children, never capture/UI."""

import errno
import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import sys
import tempfile
import textwrap
import time
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "macos/run.py"
SPEC = importlib.util.spec_from_file_location("native_macos_runner", SCRIPT)
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)

CHILD = r"""
import json, os, signal, sys, time
from pathlib import Path
image_fd = int(os.environ["DYLD_PRINT_TO_FILE"].rsplit("/", 1)[1])
def write_all(fd, data):
    data = memoryview(data)
    while data:
        data = data[os.write(fd, data):]
def image_line(path):
    return (f"dyld[{os.getpid()}]: <00000000-0000-0000-0000-000000000001> {path}\n").encode()
write_all(image_fd, image_line("/usr/lib/owned-inert-probe.dylib"))
write_all(image_fd, (f"dyld[{os.getpid()}]: move loaded to delayed: owned-inert-probe.dylib\n").encode())
"""


@unittest.skipUnless(os.name == "posix", "anonymous inherited descriptors and POSIX child limits")
class NativeMacosRunner(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.environment = {key: value for key, value in os.environ.items()
                            if not key.startswith(("DYLD_PRINT_", "MADO_PILOT_"))}
        self.environment["PYTHONDONTWRITEBYTECODE"] = "1"
        self.capture = self.new_capture("evidence")

    def new_capture(self, name):
        evidence = self.root / name
        evidence.mkdir(mode=0o700)
        capture = runner.NativeCapture(evidence)
        self.addCleanup(capture.cleanup)
        return capture

    def launch(self, code, *args, role="consumer", capture=None):
        capture = self.capture if capture is None else capture
        return capture.spawn(Path(sys.executable).resolve(),
                             ["-c", CHILD + textwrap.dedent(code), *map(str, args)], role,
                             cwd=self.root, env=self.environment)

    def record(self):
        return {"procedure": "ocr-text-watch-apple-v2", "task": "7.4", "bounds": runner.BOUNDS,
                "semantic": "not-run", "resource": "not-run", "cleanup": "not-run",
                "reason": "consumer-exited", "rows": {name: "not-run" for name in runner.ROWS},
                "commands": [], "children": []}

    def test_84029_image_bytes_reach_main_without_spending_ordinary_quota(self):
        child = self.launch(r"""
            line = image_line("/usr/lib/owned-large-probe.dylib")
            prefix = image_line("/usr/lib/")[:-1]
            payload = line * (84029 // len(line) - 1)
            payload += prefix + b"x" * (84029 - len(payload) - len(prefix) - 1) + b"\n"
            write_all(image_fd, payload)
            print(json.dumps({"pid": os.getpid(), "main": True, "image_bytes": len(payload)}), flush=True)
            write_all(2, b"ordinary-error\n")
        """)
        self.capture.cleanup()
        evidence = self.capture.evidence
        ordinary = json.loads((evidence / "consumer.stdout").read_text())
        self.assertEqual(ordinary, {"pid": child.pid, "main": True, "image_bytes": 84029})
        self.assertEqual(child.returncode, 0)
        self.assertIsNone(self.capture.failure)
        self.assertEqual((evidence / "consumer.stderr").read_bytes(), b"ordinary-error\n")
        image = self.capture.facts()["consumer.native-images"]
        self.assertGreaterEqual(image["retained_bytes"], 84029)
        self.assertLess(image["retained_bytes"], runner.BOUNDS["each_native_image_file_bytes"])
        self.assertEqual(image["observed_bytes"], image["retained_bytes"])
        self.assertTrue(image["complete"], image)
        self.assertIn("/usr/lib/owned-large-probe.dylib", self.capture.image_paths())
        self.assertEqual(self.capture.ordinary_bytes(), sum(
            (evidence / f"consumer.{kind}").stat().st_size for kind in ("stdout", "stderr")))
        for kind in ("stdout", "stderr", "native-images"):
            self.assertEqual(stat.S_IMODE((evidence / f"consumer.{kind}").stat().st_mode), 0o600)

    def test_ordinary_stdout_and_stderr_saturate_independently(self):
        child = self.launch(r"""
            write_all(1, b"o" * 73728)
            write_all(2, b"e" * 73728)
        """)
        self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        self.assertEqual(self.capture.failure["reason"], "output-limit")
        for kind, byte in (("stdout", b"o"), ("stderr", b"e")):
            with self.subTest(channel=kind):
                row = self.capture.facts()[f"consumer.{kind}"]
                self.assertEqual((self.capture.evidence / f"consumer.{kind}").read_bytes(), byte * 65536)
                self.assertEqual(row["observed_bytes"], 73728)
                self.assertTrue(row["saturated"])
                self.assertTrue(row["eof"])
                self.assertFalse(row["complete"])

    def test_image_saturation_retains_one_mib_and_cannot_be_cleared_by_clean_exit(self):
        child = self.launch(r"""
            write_all(image_fd, b"x" * 1048576)
            write_all(2, b"ordinary-tail\n")
        """)
        cleanup = self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        self.assertTrue(cleanup[0]["reaped"])
        self.assertFalse(cleanup[0]["forced"])
        row = self.capture.facts()["consumer.native-images"]
        self.assertEqual(row["retained_bytes"], 1048576)
        self.assertEqual((self.capture.evidence / "consumer.native-images").stat().st_size, 1048576)
        self.assertGreaterEqual(row["observed_bytes"], 1048576)
        self.assertTrue(row["saturated"])
        self.assertFalse(row["complete"])
        with self.assertRaises(ValueError):
            self.capture.image_paths()
        self.assertEqual(self.capture.failure["reason"], "native-image-limit")
        self.assertEqual((self.capture.evidence / "consumer.stderr").read_bytes(), b"ordinary-tail\n")

    def test_failed_image_sink_still_drains_and_reaps_the_executable(self):
        child = self.launch(r"""
            write_all(image_fd, image_line("/usr/lib/owned-sink-failure.dylib") * 2000)
            print("reached-end-of-main", flush=True)
        """)
        output = self.capture.channels["consumer.native-images"]["output"]
        real_write = os.write

        def exhausted_sink(descriptor, data):
            if descriptor == output:
                raise OSError(errno.ENOSPC, "injected full evidence device")
            return real_write(descriptor, data)

        with patch.object(runner.os, "write", side_effect=exhausted_sink):
            cleanup = self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        self.assertTrue(cleanup[0]["reaped"])
        self.assertFalse(cleanup[0]["forced"])
        self.assertEqual((self.capture.evidence / "consumer.stdout").read_text(), "reached-end-of-main\n")
        row = self.capture.facts()["consumer.native-images"]
        self.assertTrue(row["eof"])
        self.assertGreater(row["observed_bytes"], 65536)
        self.assertEqual(row["retained_bytes"], 0)
        self.assertFalse(row["complete"])
        self.assertEqual(self.capture.failure["reason"], "channel-write-failed")

    def test_unrelated_regular_file_keeps_inherited_limits_and_no_core_policy(self):
        import resource

        inherited = resource.getrlimit(resource.RLIMIT_FSIZE)
        size = 2 * 1048576
        if any(limit != resource.RLIM_INFINITY and limit < size for limit in inherited):
            self.skipTest("inherited OS limit already prevents the unrelated 2MiB write")
        unrelated = self.root / "unrelated-file"
        child = self.launch(r"""
            import resource
            signal.signal(signal.SIGXFSZ, signal.SIG_DFL)
            print(json.dumps({"file": resource.getrlimit(resource.RLIMIT_FSIZE),
                              "core": resource.getrlimit(resource.RLIMIT_CORE)}), flush=True)
            Path(sys.argv[1]).write_bytes(b"u" * int(sys.argv[2]))
        """, unrelated, size)
        self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        self.assertIsNone(self.capture.failure)
        limits = json.loads((self.capture.evidence / "consumer.stdout").read_text())
        self.assertEqual(limits["file"], list(inherited))
        self.assertEqual(limits["core"], [0, 0])
        self.assertEqual(unrelated.read_bytes(), b"u" * size)
        self.assertEqual(resource.getrlimit(resource.RLIMIT_FSIZE), inherited)

    def test_cleanup_drains_all_six_channels_without_serial_wait_deadlock(self):
        children = []
        for role in ("consumer", "fixture"):
            children.append(self.launch(r"""
                write_all(1, b"o" * 48000 + b"stdout-tail\n")
                write_all(2, b"e" * 48000 + b"stderr-tail\n")
                write_all(image_fd, image_line("/usr/lib/owned-cleanup-probe.dylib") * 5000)
            """, role=role))
        cleanup = self.capture.cleanup()
        self.assertEqual({row["pid"] for row in cleanup}, {child.pid for child in children})
        self.assertIsNone(self.capture.failure)
        for row in cleanup:
            self.assertTrue(row["reaped"], row)
            self.assertFalse(row["forced"], row)
            self.assertEqual(row["exit_code"], 0, row)
            evidence = self.capture.evidence
            self.assertEqual((evidence / f"{row['role']}.stdout").read_bytes(), b"o" * 48000 + b"stdout-tail\n")
            self.assertEqual((evidence / f"{row['role']}.stderr").read_bytes(), b"e" * 48000 + b"stderr-tail\n")
        self.assertTrue(all(row["complete"] for row in self.capture.facts().values()))
        self.assertIn("/usr/lib/owned-cleanup-probe.dylib", self.capture.image_paths())
        self.assertEqual(self.capture.ordinary_bytes(), 2 * (96000 + len(b"stdout-tail\nstderr-tail\n")))

    def test_exit_after_empty_select_still_drains_every_tail(self):
        gate = self.root / "exit-gate"
        ready_path = self.root / "exit-ready"
        child = self.launch(r"""
            gate = Path(sys.argv[1])
            Path(sys.argv[2]).write_text("ready")
            deadline = time.monotonic() + 5
            while not gate.exists():
                if time.monotonic() >= deadline:
                    raise SystemExit(9)
                time.sleep(0.001)
            write_all(1, b"stdout-exit-tail\n")
            write_all(2, b"stderr-exit-tail\n")
            write_all(image_fd, image_line("/usr/lib/owned-exit-tail.dylib"))
        """, gate, ready_path)
        real_select = runner.select.select

        def exit_after_empty_select(*args):
            ready = real_select(*args)
            if not ready[0] and ready_path.exists() and not gate.exists():
                gate.touch()
                # All three final tails are smaller than one pipe capacity. Force
                # the exit/select race, not an unsafe wait behind a full pipe.
                child.wait(timeout=3)
            return ready

        with patch.object(runner.select, "select", side_effect=exit_after_empty_select):
            deadline = time.monotonic() + 5
            while child.poll() is None and time.monotonic() < deadline:
                self.capture.pump()
        self.assertTrue(gate.exists(), "the empty-select/exit race was not exercised")
        self.capture.cleanup()
        self.assertIsNone(self.capture.failure)
        self.assertEqual((self.capture.evidence / "consumer.stdout").read_bytes(), b"stdout-exit-tail\n")
        self.assertEqual((self.capture.evidence / "consumer.stderr").read_bytes(), b"stderr-exit-tail\n")
        self.assertIn("/usr/lib/owned-exit-tail.dylib", self.capture.image_paths())
        self.assertTrue(all(row["complete"] for row in self.capture.facts().values()))

    def test_aggregate_counts_control_report_and_unrecognized_image_filename(self):
        child = self.launch(r"""
            root = Path(sys.argv[1])
            (root / "command").write_bytes(b"c" * 200000)
            (root / "consumer.report").write_bytes(b"r" * 40000)
            (root / "unapproved.native-images").write_bytes(b"i" * 30000)
        """, self.capture.evidence)
        self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        self.assertEqual(self.capture.ordinary_bytes(), 270000)
        self.assertEqual(self.capture.failure["reason"], "output-limit")

    def test_full_final_record_also_must_fit_ordinary_aggregate(self):
        child = self.launch("print('ordinary')")
        self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        record = self.record()
        record["private_payload"] = "x" * runner.BOUNDS["total_output_bytes"]
        self.assertFalse(runner.write_record(record, self.capture))
        self.assertFalse((self.capture.evidence / "record.json").exists())
        self.assertFalse(record["passed"])
        self.assertEqual(record["reason"], "record-output-failed")

    def test_interruption_fence_keeps_saved_and_returned_verdicts_consistent(self):
        for moment in ("before-commit", "serialization", "write"):
            with self.subTest(moment=moment):
                capture = self.new_capture(moment)
                record = self.record()
                record.update(passed=True, semantic="passed", resource="passed", cleanup="passed")
                original_dumps, original_open = json.dumps, Path.open

                def serialize(*args, **kwargs):
                    if moment == "serialization":
                        signal.raise_signal(signal.SIGTERM)
                    return original_dumps(*args, **kwargs)

                def open_record(path, *args, **kwargs):
                    stream = original_open(path, *args, **kwargs)
                    if moment == "write" and path.name == "record.json":
                        signal.raise_signal(signal.SIGTERM)
                    return stream

                previous = signal.signal(signal.SIGTERM, capture.interrupt)
                try:
                    if moment == "before-commit":
                        signal.raise_signal(signal.SIGTERM)
                    with patch.object(runner.json, "dumps", side_effect=serialize), \
                            patch.object(Path, "open", open_record):
                        self.assertTrue(runner.write_record(record, capture))
                finally:
                    signal.signal(signal.SIGTERM, previous)
                saved = json.loads((capture.evidence / "record.json").read_text())
                self.assertEqual(saved["passed"], moment != "before-commit")
                self.assertEqual(record["passed"], saved["passed"])
                self.assertEqual(saved["private_first_failure"], capture.failure)
                if moment == "before-commit":
                    self.assertEqual(saved["reason"], "interrupted")
                else:
                    self.assertIsNone(capture.failure)

    def test_malformed_or_unterminated_image_records_are_never_dependency_proof(self):
        for name, suffix in (("utf8", b"\xff\n"), ("syntax", b"not-a-dyld-record\n"),
                             ("partial", b"dyld[1]: <00000000-0000-0000-0000-000000000001> /incomplete")):
            with self.subTest(case=name):
                capture = self.new_capture(name)
                child = self.launch("write_all(image_fd, bytes.fromhex(sys.argv[1]))", suffix.hex(), capture=capture)
                capture.cleanup()
                self.assertEqual(child.returncode, 0)
                self.assertIsNone(capture.failure)
                with self.assertRaises(ValueError):
                    capture.image_paths()
                self.assertEqual(capture.failure["reason"], "native-image-observation-failed")
                self.assertFalse(capture.facts()["consumer.native-images"]["complete"])

    def test_images_come_only_from_image_channel_without_basename_exemption(self):
        image = self.root / "libsystem_kernel.dylib"
        ordinary_only = self.root / "ordinary-only-image"
        image.write_bytes(b"owned inert non-system image")
        ordinary_only.write_bytes(b"not a loaded dependency")
        child = self.launch(r"""
            write_all(image_fd, image_line(sys.argv[1]))
            write_all(2, image_line(sys.argv[2]))
        """, image, ordinary_only)
        self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        self.assertIn(str(image), self.capture.image_paths())
        self.assertNotIn(str(ordinary_only), self.capture.image_paths())
        record = self.record()
        record["approved_native_images"] = {str(ordinary_only): runner.digest(ordinary_only)["sha256"]}
        runner.finalize_record(record, self.capture, time.monotonic())
        observed = next(row for row in record["loaded_images"] if row["path"] == str(image))
        self.assertFalse(observed["system_cache"])
        self.assertEqual(observed["sha256"], runner.digest(image)["sha256"])
        self.assertFalse(record["native_image_binding_matches"])
        self.assertNotIn(str(image), record["approved_native_images"])

    def test_observed_symlink_is_not_rewritten_to_an_approved_canonical_identity(self):
        image = self.root / "approved-image"
        alias = self.root / "unapproved-alias"
        image.write_bytes(b"owned inert image identity")
        alias.symlink_to(image)
        child = self.launch("write_all(image_fd, image_line(sys.argv[1]))", alias)
        self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        record = self.record()
        record["approved_native_images"] = {str(image): runner.digest(image)["sha256"]}
        runner.finalize_record(record, self.capture, time.monotonic())
        observed = next(row for row in record["loaded_images"] if row["path"] == str(alias))
        self.assertEqual(observed["identity"], "unresolved")
        self.assertNotIn("sha256", observed)
        self.assertFalse(record["native_image_binding_matches"])

    def test_nonzero_or_signal_exit_cannot_be_relabelled_from_ordinary_error_text(self):
        for label, expected in (("exit", 7), ("signal", -signal.SIGTERM)):
            with self.subTest(case=label):
                capture = self.new_capture(label)
                child = self.launch(r"""
                    Path(sys.argv[1]).write_text("semantic=passed resource=passed cleanup=consumer-endpoints-passed\n")
                    write_all(2, b"consumer-failed reason=unsupported\n")
                    if sys.argv[2] == "signal":
                        os.kill(os.getpid(), signal.SIGTERM)
                    raise SystemExit(7)
                """, capture.evidence / "consumer.report", label, capture=capture)
                cleanup = capture.cleanup()
                self.assertEqual(child.returncode, expected)
                self.assertTrue(cleanup[0]["reaped"])
                self.assertFalse(cleanup[0]["forced"])
                first = dict(capture.failure)
                record = self.record()
                runner.finalize_record(record, capture, time.monotonic())
                self.assertFalse(record["passed"])
                self.assertEqual(record["reason"], "consumer-exited")
                self.assertEqual(record["private_first_failure"], first)
                self.assertEqual(record["children"][0]["exit_code"], expected)

    def test_timeout_is_sticky_even_when_graceful_cleanup_exits_zero(self):
        gate = self.root / "cleanup-gate"
        child = self.launch(r"""
            while not Path(sys.argv[1]).exists():
                time.sleep(0.001)
            Path(sys.argv[2]).write_text("semantic=passed resource=passed cleanup=consumer-endpoints-passed\n")
        """, gate, self.capture.evidence / "consumer.report")
        self.capture.fail("consumer-timeout")
        gate.touch()
        cleanup = self.capture.cleanup()
        self.assertEqual(child.returncode, 0)
        self.assertTrue(cleanup[0]["reaped"])
        self.assertFalse(cleanup[0]["forced"])
        record = self.record()
        runner.finalize_record(record, self.capture, time.monotonic())
        self.assertFalse(record["passed"])
        self.assertEqual(record["reason"], "consumer-timeout")
        self.assertEqual(record["private_first_failure"]["reason"], "consumer-timeout")

    def test_pump_exception_cannot_skip_either_owned_child_reap(self):
        gate = self.root / "pump-failure-gate"
        ready = [self.root / f"{role}-ready" for role in ("consumer", "fixture")]
        children = [self.launch(r"""
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            Path(sys.argv[1]).write_text("ready")
            while not Path(sys.argv[2]).exists():
                time.sleep(0.001)
            while True:
                write_all(1, b"x" * 65536)
                write_all(image_fd, b"i" * 65536)
        """, path, gate, role=role) for role, path in zip(("consumer", "fixture"), ready)]
        deadline = time.monotonic() + 5
        while not all(path.exists() for path in ready) and time.monotonic() < deadline:
            self.capture.pump()
        self.assertTrue(all(path.exists() for path in ready))
        with patch.object(self.capture, "pump", side_effect=OSError("injected pump failure")), \
                patch.dict(runner.BOUNDS, {"physical_drain_seconds": 0.05}):
            gate.touch()
            cleanup = self.capture.cleanup(graceful=0.05)
        self.assertEqual(self.capture.failure["reason"], "channel-pump-failed")
        self.assertEqual({row["pid"] for row in cleanup}, {child.pid for child in children})
        for row in cleanup:
            self.assertTrue(row["forced"], row)
            self.assertTrue(row["reaped"], row)
            self.assertEqual(row["exit_code"], -signal.SIGKILL)
            with self.assertRaises(ChildProcessError):
                os.waitpid(row["pid"], os.WNOHANG)
        self.assertFalse(any(row["complete"] for row in self.capture.facts().values()))

    def test_partial_launch_setup_closes_descriptors_and_keeps_exclusive_file(self):
        sentinel = self.capture.evidence / "consumer.stderr"
        sentinel.write_bytes(b"existing evidence")
        opened = []
        real_open, real_pipe = os.open, os.pipe

        def remember_open(*args, **kwargs):
            descriptor = real_open(*args, **kwargs)
            opened.append(descriptor)
            return descriptor

        def remember_pipe():
            pair = real_pipe()
            opened.extend(pair)
            return pair

        with patch.object(runner.os, "open", side_effect=remember_open), \
                patch.object(runner.os, "pipe", side_effect=remember_pipe):
            with self.assertRaises(FileExistsError):
                self.launch("raise SystemExit(0)")
        self.assertEqual(sentinel.read_bytes(), b"existing evidence")
        self.assertEqual(self.capture.cleanup(), [])
        self.assertEqual(self.capture.failure["reason"], "native-child-launch-failed")
        for descriptor in opened:
            with self.assertRaises(OSError) as caught:
                os.fstat(descriptor)
            self.assertEqual(caught.exception.errno, errno.EBADF)


if __name__ == "__main__":
    unittest.main()
