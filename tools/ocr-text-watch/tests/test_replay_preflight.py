"""Model-free regressions for replay evidence input binding."""

from __future__ import annotations

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
                    "assert module.git(module.runner(),'rev-parse','--show-toplevel') == str(module.ROOT)"
                    if target == "windows" else
                    "result=module.bounded_reader()([module.GIT_EXECUTABLE,'rev-parse','--show-toplevel'],"
                    "cwd=module.ROOT,env=dict(os.environ),timeout_seconds=5,output_limit_bytes=4096,cleanup_seconds=5); "
                    "assert result['exit_code']==0 and result['cleanup_ok']; "
                    "assert result['stdout'].strip()==str(module.ROOT)"
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
