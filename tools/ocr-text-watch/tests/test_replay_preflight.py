"""Model-free regressions for replay evidence input binding."""

from __future__ import annotations

from contextlib import contextmanager
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "run_replay.py"
SPEC = importlib.util.spec_from_file_location("ocr_replay_preflight", SCRIPT)
replay = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(replay)


class ReplayInputBinding(unittest.TestCase):
    def test_relative_dependencies_select_the_same_files_after_child_cwd_change(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            caller, child = base / "caller", base / "child"
            for directory, contents in ((caller, b"reviewed"), (child, b"unreviewed")):
                (directory / "models").mkdir(parents=True)
                (directory / "models" / "sentinel").write_bytes(contents)
                (directory / "runtime").write_bytes(contents)
            previous = Path.cwd()
            try:
                os.chdir(caller)
                with patch.dict(os.environ, {
                    "MADO_PILOT_G004_MODEL_ROOT": "models",
                    "MADO_PILOT_ONNX_RUNTIME": "runtime",
                }):
                    environment = replay.canonical_environment()
                command = (
                    "import os; from pathlib import Path; "
                    "runtime=Path(os.environ['MADO_PILOT_ONNX_RUNTIME']).read_bytes(); "
                    "model=(Path(os.environ['MADO_PILOT_G004_MODEL_ROOT'])/'sentinel').read_bytes(); "
                    "print((runtime == b'reviewed') and (model == b'reviewed'))"
                )
                result = replay.run_process(
                    [sys.executable, "-c", command], cwd=child, env=environment,
                    timeout_seconds=5, output_limit_bytes=1024, cleanup_seconds=5,
                )
            finally:
                os.chdir(previous)
            self.assertEqual(result["exit_code"], 0)
            self.assertEqual(result["stdout"].strip(), "True")
            self.assertTrue(result["cleanup_ok"])

    def test_changed_manifest_or_oracle_is_refused_before_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runtime = root / "runtime"
            runtime.write_bytes(b"inert fixture, not a native library")
            original = {
                "madopilot-replay.json": b'{"targets":["reviewed"]}',
                "oracle.json": b'{"terminal":"SessionClosed"}',
            }
            expected = {name: hashlib.sha256(data).hexdigest() for name, data in original.items()}
            environment = {
                "MADO_PILOT_G004_MODEL_ROOT": str(root),
                "MADO_PILOT_ONNX_RUNTIME": str(runtime),
            }
            with patch.object(replay, "MODELS", {}), patch.object(replay, "PIXELS", {}), patch.object(replay, "DOCUMENTS", expected):
                for name, data in original.items():
                    (root / name).write_bytes(data)
                replay.inputs(Path(sys.executable), root, environment)
                for name, data in original.items():
                    with self.subTest(document=name):
                        changed = json.loads(data)
                        changed[next(iter(changed))] = "unreviewed"
                        (root / name).write_text(json.dumps(changed), encoding="utf-8")
                        with self.assertRaises(ValueError):
                            replay.inputs(Path(sys.executable), root, environment)
                        (root / name).write_bytes(data)


    def test_native_metadata_helpers_work_in_fresh_processes(self):
        for target in ("windows", "macos"):
            with self.subTest(target=target):
                native = SCRIPT.parent / target / "run.py"
                call = (
                    "assert module.Path(module.git(module.runner(),'rev-parse','--show-toplevel')).resolve() == module.ROOT"
                    if target == "windows" else
                    "result=module.bounded_reader()([module.GIT_EXECUTABLE,'rev-parse','--show-toplevel'],"
                    "cwd=module.ROOT,env=dict(os.environ),timeout_seconds=5,output_limit_bytes=4096,cleanup_seconds=5); "
                    "assert result['exit_code']==0 and result['cleanup_ok']; "
                    "assert module.Path(result['stdout'].strip()).resolve()==module.ROOT"
                )
                program = (
                    "import importlib.util,os,sys; "
                    "spec=importlib.util.spec_from_file_location('native_preflight',sys.argv[1]); "
                    "module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module); "
                    + call
                )
                result = subprocess.run(
                    [sys.executable, "-c", program, str(native)], check=False,
                    capture_output=True, text=True, timeout=20,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_unchanged_file_returns_exact_bytes_and_identity_after_metadata_update(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "document"
            content = b"unchanged reviewed bytes"
            path.write_bytes(content)
            os.utime(path, ns=(1_000_000_000, 1_000_000_000))
            data, observed = replay.read_document(path)
            self.assertEqual(data, content)
            self.assertEqual(observed, {
                "path": str(path.resolve()), "bytes": len(content),
                "sha256": hashlib.sha256(content).hexdigest(),
            })
            self.assertEqual(replay.identity(path), observed)

    def test_same_size_mutation_with_restored_mtime_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "document"
            original, changed = b"reviewed", b"replaced"
            path.write_bytes(original)
            before = path.stat()

            def changing_reader(*args, **kwargs):
                stream = open(*args, **kwargs)
                read = stream.read

                def read_then_mutate(size=-1):
                    data = read(size)
                    if data:
                        with path.open("r+b") as writer:
                            writer.write(changed)
                        os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
                    return data

                stream.read = read_then_mutate
                return stream

            with patch.object(replay, "open", side_effect=changing_reader, create=True):
                with self.assertRaises(ValueError):
                    replay.read_document(path)
            self.assertEqual(path.read_bytes(), changed)
            self.assertEqual(path.stat().st_mtime_ns, before.st_mtime_ns)

    def test_identical_content_path_replacement_after_read_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path, replacement, displaced = (root / name for name in ("document", "replacement", "displaced"))
            content = b"identical content in distinct files"
            path.write_bytes(content)
            replacement.write_bytes(content)
            before = path.stat()
            os.utime(replacement, ns=(before.st_atime_ns, before.st_mtime_ns))

            @contextmanager
            def replacing_reader(*args, **kwargs):
                with open(*args, **kwargs) as stream:
                    yield stream
                path.rename(displaced)
                replacement.rename(path)

            with patch.object(replay, "open", side_effect=replacing_reader, create=True):
                with self.assertRaises(ValueError):
                    replay.read_document(path)
            self.assertEqual(path.read_bytes(), content)
            self.assertEqual(path.stat().st_mtime_ns, before.st_mtime_ns)
            self.assertFalse(path.samefile(displaced))

    def test_native_dependency_changes_require_new_approval(self):
        spec = importlib.util.spec_from_file_location("apple_preflight", SCRIPT.parent / "macos/run.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as temporary:
            path = (Path(temporary) / "inert-image").resolve()
            path.write_bytes(b"approved bytes; not an executable")
            approved = {str(path): hashlib.sha256(path.read_bytes()).hexdigest()}
            module.validate_native_images(approved)
            path.write_bytes(b"changed native dependency")
            with self.assertRaises(ValueError):
                module.validate_native_images(approved)
            with self.assertRaises(ValueError):
                module.validate_native_images({})


if __name__ == "__main__":
    unittest.main()
