"""Controlled external-consumer commands bind to the repository pin, never ambient rustc."""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_consumers


class CompilerBindingTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / "checkout"
        self.root.mkdir()
        (self.root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.97.1"\n', encoding="utf-8")
        inputs = ["tools/native-release-profile/consumers/" + name
                  for name in ("rust_facade.rs", "c_abi.c", "cpp_abi.cpp")]
        inputs += ["tools/native-release-profile/host_load.c", "tools/native-release-profile/windows_utf8_entry.h",
                   "crates/bindings/capi/include/madopilot/madopilot.h",
                   "crates/bindings/capi/examples/deterministic-scene.h"]
        for version in ("v1", "v1_2", "v1_3", "v1_4"):
            inputs += [f"crates/bindings/capi/tests/abi-compat/{version}/" + name
                       for name in ("old-prefix.c", "madopilot/madopilot.h")]
        for name in inputs:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"synthetic consumer input")
        self.libraries = self.base / "libraries"
        self.libraries.mkdir()
        for name in ("libmadopilot.dylib", "libmado_pilot.rlib"):
            (self.libraries / name).write_bytes(b"synthetic artifact")
        self.models = self.base / "models"
        self.models.mkdir()
        self.runtime = self.base / "runtime.dylib"
        self.runtime.write_bytes(b"synthetic runtime")
        self.output = self.base / "external-output"
        self.tools = {}
        for name in ("clang", "clang++", "rustup", "ambient-rustc", "pinned-rustc"):
            path = self.base / "tools" / name
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(name.encode())
            self.tools[name] = str(path)
        self.selected = self.tools["pinned-rustc"]
        self.version = "rustc 1.97.1 (synthetic)\nrelease: 1.97.1\nhost: aarch64-apple-darwin\n"
        self.calls = []

    def process(self, argv, **options):
        self.calls.append((list(argv), options))
        result = {"exit_code": 0, "timed_out": False, "output_limited": False,
                  "cleanup_ok": True, "launch_error": None, "stdout": "", "stderr": "", "duration_seconds": 0.01}
        if argv[0] == self.tools["rustup"]:
            result["stdout"] = self.selected + "\n"
        elif argv[0] == self.tools["pinned-rustc"] and argv[1:] == ["-Vv"]:
            result["stdout"] = self.version
        elif "-o" in argv:
            Path(argv[argv.index("-o") + 1]).write_bytes(b"synthetic compiler output")
        else:
            name = Path(argv[0]).name
            loaded = options["env"].get("MADO_PROFILE_LIBRARY")
            if loaded and Path(loaded).name == "absent-library":
                result.update(exit_code=1, stdout="MADO_PROFILE_LOAD=unavailable\n")
            elif name.startswith("prefix-"):
                result["stdout"] = "madopilot-abi-compat-" + name.removeprefix("prefix-") + " complete\n"
            else:
                result["stdout"] = "MADO_PROFILE_RESULT=passed\nMADO_PROFILE_MODULES=complete\n"
                if loaded:
                    result["stdout"] += "MADO_PROFILE_MODULE=" + loaded + "\n"
        return result

    def build(self):
        def which(name):
            return self.tools["ambient-rustc"] if name == "rustc" else self.tools.get(name)

        with patch.object(check_consumers.sys, "platform", "darwin"), \
                patch.object(check_consumers, "native_target", return_value="aarch64-apple-darwin"), \
                patch.object(check_consumers.shutil, "which", side_effect=which), \
                patch.dict(os.environ, OPENCV_LINK_PATHS="/synthetic/opencv", RUSTUP_TOOLCHAIN="ambient-default"), \
                patch.object(check_consumers, "run_process", side_effect=self.process):
            return check_consumers.build_and_run(self.root, self.libraries, self.output, self.models, self.runtime)

    def test_external_executable_and_module_use_absolute_pinned_compiler_despite_ambient_default(self):
        record = self.build()
        self.assertEqual(record["status"], "passed")
        selection = self.calls[0]
        self.assertEqual(selection[0], [self.tools["rustup"], "which", "--toolchain", "1.97.1", "rustc"])
        self.assertEqual(selection[1]["cwd"], self.root)
        self.assertEqual(self.calls[1][0], [self.selected, "-Vv"])
        self.assertEqual(self.calls[1][1]["cwd"], self.root)
        builds = [row for row in record["rows"] if row["id"] in ("build-rust-facade", "build-rust-module")]
        self.assertEqual(len(builds), 2)
        for row in builds:
            self.assertEqual(row["argv"][0], self.selected)
            call = next(call for call in self.calls if call[0] == row["argv"])
            self.assertEqual(call[1]["cwd"], self.output)
        self.assertTrue(any("cdylib" in row["argv"] for row in builds))
        self.assertEqual(record["rust_compiler"], {"toolchain": "1.97.1", "path": self.selected,
                         "sha256": check_consumers.digest_file(Path(self.selected)), "version": self.version.strip()})
        self.assertFalse(any(call[0][0] == self.tools["ambient-rustc"] for call in self.calls))
        self.assertEqual(json.loads((self.output / "result.json").read_text())["rust_compiler"], record["rust_compiler"])

    def test_missing_selected_compiler_stops_before_any_consumer_build(self):
        self.selected = str(self.base / "missing-rustc")
        with self.assertRaisesRegex(ValueError, "installed pinned Rust compiler unavailable"):
            self.build()
        self.assertEqual(len(self.calls), 1)
        record = json.loads((self.output / "result.json").read_text())
        self.assertEqual(record["status"], "failed")
        self.assertEqual([row["id"] for row in record["rows"]], ["select-rustc"])

    def test_selected_compiler_version_must_match_pin(self):
        self.version = "rustc 1.98.1 (synthetic)\nrelease: 1.98.1\n"
        with self.assertRaisesRegex(ValueError, "version differs from rust-toolchain.toml"):
            self.build()
        self.assertEqual(len(self.calls), 2)
        self.assertFalse(any("--edition=2024" in call[0] for call in self.calls))

    def test_missing_rustup_does_not_fall_back_to_ambient_rustc(self):
        del self.tools["rustup"]
        with self.assertRaisesRegex(ValueError, "installed pinned Rust toolchain required"):
            self.build()
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
