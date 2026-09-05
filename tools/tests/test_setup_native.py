"""Guard setup failure boundaries and child/CI environment precedence without native dependencies."""

from contextlib import redirect_stderr, redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import sys
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location("setup_native", Path(__file__).resolve().parents[1] / "setup-native.py")
setup_native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(setup_native)


class SetupNativeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / "OpenCV 選択"
        self.clang = self.base / "LLVM 選択"
        self.tools = self.base / "tools"
        self.sdk = self.base / "SDK"
        self.sdk.mkdir()
        self.file(self.sdk / "usr/include/c++/v1/limits")
        self.file(self.sdk / "include/limits")
        self.file(self.sdk / "lib/kernel32.lib")
        self.windows = os.name == "nt"
        self.runtime_version = "4.14.0"
        self.failure = None
        self.marker = self.base / "child-result.json"
        self.env_file = self.base / "github-env"
        self.path_file = self.base / "github-path"
        self.stdout = io.StringIO()
        self.environment = dict(os.environ)
        for name in ("OPENCV4_STATIC", "OPENCV4_NO_PKG_CONFIG", "SDKROOT", "DYLD_LIBRARY_PATH"):
            self.environment.pop(name, None)
        self.environment.update(PATH=str(self.tools) + os.pathsep + os.environ.get("PATH", os.defpath),
                                VSCMD_ARG_TGT_ARCH="x64", INCLUDE=str(self.sdk / "include"),
                                LIB=str(self.sdk / "lib"), LIBPATH=str(self.sdk / "lib"))
        for name in ("cl.exe", "clang", "clang++", "xcrun", "pkg-config"):
            self.file(self.tools / name).chmod(0o755)
        for name in ("libclang.dll", "libclang.dylib", "clang.exe"):
            self.file(self.clang / name).chmod(0o755)
        self.version(4)
        for include in (self.root / "include/opencv4", self.root / "build/include"):
            for name in ("core.hpp", "imgproc.hpp", "imgcodecs.hpp", "cvconfig.h", "opencv_modules.hpp"):
                self.file(include / "opencv2" / name)
        for module in ("core", "imgproc", "imgcodecs"):
            self.file(self.root / "lib" / f"libopencv_{module}.dylib")
        self.file(self.root / "lib/pkgconfig/opencv4.pc")
        self.file(self.root / "build/x64/vc16/lib/opencv_world4140.lib", "release import")
        self.file(self.root / "build/x64/vc16/lib/opencv_world4140d.lib", "debug import")
        self.file(self.root / "build/x64/vc16/bin/opencv_world4140.dll")

    def file(self, path, text=""):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def version(self, major):
        text = f"#define CV_VERSION_MAJOR {major}\n#define CV_VERSION_MINOR 14\n#define CV_VERSION_REVISION 0\n"
        for include in (self.root / "include/opencv4", self.root / "build/include"):
            self.file(include / "opencv2/core/version.hpp", text)

    def probe(self, argv, **kwargs):
        name = Path(argv[0]).name
        output, stage = "", "compile"
        if name == "xcrun":
            output = str(self.sdk if argv[-1] == "--show-sdk-path" else self.tools / argv[-1])
            stage = "discovery"
        elif name == "pkg-config":
            stage = "pkg-config"
            argument = argv[1]
            output = {
                "--variable=pcfiledir": str(self.root / "lib/pkgconfig"),
                "--variable=includedir": str(self.root / "include/opencv4"),
                "--variable=libdir": str(self.root / "lib"),
                "--modversion": "4.14.0",
                "--cflags": "-I" + shlex.quote(str(self.root / "include/opencv4")),
                "--libs": "-L" + shlex.quote(str(self.root / "lib")) + " -lopencv_core -lopencv_imgproc -lopencv_imgcodecs",
            }[argument]
        elif argv[1:3] == ["-I", "-c"]:
            output, stage = "fixture libclang version", "libclang"
        elif name in ("opencv-probe", "opencv-probe.exe"):
            output, stage = self.runtime_version, "runtime"
        failed = stage == self.failure
        if stage == "compile":
            environment = kwargs["env"]
            if self.windows:
                headers = environment.get("INCLUDE", "").split(";")
                libraries = environment.get("LIB", "").split(";")
                failed |= not (any((Path(path) / "limits").is_file() for path in headers if path)
                               and any((Path(path) / "kernel32.lib").is_file() for path in libraries if path))
                opencv_import = Path(environment.get("OPENCV_LINK_PATHS", "")) / (environment.get("OPENCV_LINK_LIBS", "") + ".lib")
                failed |= not opencv_import.is_file() or opencv_import.read_text(encoding="utf-8") != "release import"
            else:
                sdk = environment.get("SDKROOT")
                failed |= sdk is None or not (Path(sdk) / "usr/include/c++/v1/limits").is_file()
        return {"exit_code": 9 if failed else 0, "timed_out": False, "output_limited": False,
                "cleanup_ok": True, "stdout": output, "stderr": "native probe failed" if failed else "",
                "launch_error": None, "duration_seconds": 0.01}

    def invoke(self, command=None, *, root=None, exports=True):
        arguments = ["--opencv-root", str(self.root if root is None else root), "--libclang-path", str(self.clang)]
        if exports:
            arguments += ["--github-env", str(self.env_file), "--github-path", str(self.path_file)]
        if command is None:
            command = [sys.executable, "-c", "from pathlib import Path; import sys; Path(sys.argv[1]).write_text('ran')", str(self.marker)]
        target = "x86_64-pc-windows-msvc" if self.windows else "aarch64-apple-darwin"
        with mock.patch.object(setup_native, "native_target", return_value=target), \
                mock.patch.object(setup_native, "run_process", side_effect=self.probe), \
                mock.patch.object(sys, "version_info", (3, 13)), \
                mock.patch.dict(os.environ, self.environment, clear=True), \
                redirect_stderr(io.StringIO()), redirect_stdout(self.stdout):
            before = dict(os.environ)
            status = setup_native.main([*arguments, *(["--", *command] if command else [])])
            self.assertEqual(dict(os.environ), before, "setup must not mutate the caller environment")
        return status

    def assert_unprepared(self, status):
        self.assertNotEqual(status, 0)
        self.assertFalse(self.marker.exists(), "the downstream command must not run")
        self.assertFalse(self.env_file.exists(), "a failed preparation must not export settings")
        self.assertFalse(self.path_file.exists(), "a failed preparation must not export PATH")

    def test_invalid_root_never_runs_child_or_exports(self):
        self.assert_unprepared(self.invoke(root=self.base / "not-installed"))

    def test_wrong_header_major_never_runs_child_or_exports(self):
        self.version(5)
        self.assert_unprepared(self.invoke())

    def test_wrong_loaded_version_never_runs_child_or_exports(self):
        self.runtime_version = "4.13.0"
        self.assert_unprepared(self.invoke())

    def test_failed_libclang_load_preserves_existing_exports(self):
        self.failure = "libclang"
        self.file(self.env_file, "EXISTING=value\n")
        self.file(self.path_file, "/existing/path\n")
        self.assertNotEqual(self.invoke(), 0)
        self.assertFalse(self.marker.exists())
        self.assertEqual(self.env_file.read_text(encoding="utf-8"), "EXISTING=value\n")
        self.assertEqual(self.path_file.read_text(encoding="utf-8"), "/existing/path\n")

    def test_missing_windows_runtime_never_runs_child_or_exports(self):
        self.windows = True
        (self.root / "build/x64/vc16/bin/opencv_world4140.dll").unlink()
        self.assert_unprepared(self.invoke())

    def test_selected_roots_override_stale_child_and_ci_discovery(self):
        self.environment.update({name: "stale-discovery" for name in setup_native.OPENCV_DISCOVERY})
        self.environment.update(LIBCLANG_PATH="stale-clang", CXX="stale-compiler", CLANG_PATH="stale-clang",
                                OPENCV_MSVC_CRT="static", MADO_PILOT_ONNX_RUNTIME="caller-owned-ort",
                                CUDA_PATH="caller-owned-cuda", OPENCV_DNN_CUDA="caller-choice")
        for name in ("PKG_CONFIG", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR"):
            self.environment.update(setup_native.target_settings(name, "stale-pkg-config", "aarch64-apple-darwin"))
        names = [*setup_native.OPENCV_DISCOVERY, "LIBCLANG_PATH", "PATH", "MADO_PILOT_ONNX_RUNTIME",
                 "CUDA_PATH", "OPENCV_DNN_CUDA", "SDKROOT", "INCLUDE", "LIB", "LIBPATH", "VSCMD_ARG_TGT_ARCH",
                 *setup_native.target_settings("PKG_CONFIG_PATH", "", "aarch64-apple-darwin")]
        script = ("import json, os, pathlib, sys; "
                  "values = {name: os.environ.get(name) for name in json.loads(sys.argv[2])}; "
                  "pathlib.Path(sys.argv[1]).write_text(json.dumps(values), encoding='utf-8')")
        self.assertEqual(self.invoke([sys.executable, "-c", script, str(self.marker), json.dumps(names)]), 0)
        child = json.loads(self.marker.read_text(encoding="utf-8"))
        exported = dict(line.split("=", 1) for line in self.env_file.read_text(encoding="utf-8").splitlines() if line)
        ci = {name: self.environment[name] for name in (
            "PATH", "SYSTEMROOT", "SystemRoot", "WINDIR", "TEMP", "TMP", "HOME", "USERPROFILE",
            "MADO_PILOT_ONNX_RUNTIME", "CUDA_PATH", "OPENCV_DNN_CUDA",
        ) if name in self.environment}
        ci.update(exported)
        for path in self.path_file.read_text(encoding="utf-8").splitlines():
            if path:
                ci["PATH"] = path + os.pathsep + ci.get("PATH", os.defpath)
        include = self.root / ("build/include" if self.windows else "include/opencv4")
        libraries = self.root / ("build/x64/vc16/lib" if self.windows else "lib")
        for observed in (child, ci):
            compiler = "cl.exe" if self.windows else "clang++"
            self.assertEqual(self.probe([compiler], env=observed)["exit_code"], 0,
                             "a fresh consumer must find the selected compiler's standard headers and libraries")
            self.assertEqual(observed["OPENCV_INCLUDE_PATHS"], str(include))
            self.assertEqual(observed["OPENCV_LINK_PATHS"], str(libraries))
            self.assertEqual(observed["LIBCLANG_PATH"], str(self.clang))
            self.assertEqual(observed["MADO_PILOT_ONNX_RUNTIME"], "caller-owned-ort")
            self.assertEqual(observed["CUDA_PATH"], "caller-owned-cuda")
            self.assertEqual(observed["OPENCV_DNN_CUDA"], "caller-choice")
            if self.windows:
                self.assertNotIn("environment", observed["OPENCV_DISABLE_PROBES"].split(","))
            else:
                self.assertNotIn("pkg_config", observed["OPENCV_DISABLE_PROBES"].split(","))
                for name in setup_native.target_settings("PKG_CONFIG_PATH", "", "aarch64-apple-darwin"):
                    self.assertEqual(observed[name], str(self.root / "lib/pkgconfig"))
        if self.windows:
            runtime = str(self.root / "build/x64/vc16/bin")
            self.assertEqual(child["PATH"].split(";")[0], runtime)
            ci_paths = self.path_file.read_text(encoding="utf-8").splitlines()
            self.assertEqual(next(path for path in reversed(ci_paths) if path), runtime)

    def test_inherited_secrets_and_path_are_not_disclosed(self):
        self.environment["SETUP_TEST_TOKEN"] = "private-token-value"
        self.environment["PATH"] += os.pathsep + str(self.base / "private-path-value")
        self.assertEqual(self.invoke([]), 0)
        output = self.stdout.getvalue() + self.env_file.read_text(encoding="utf-8") + self.path_file.read_text(encoding="utf-8")
        self.assertNotIn("private-token-value", output)
        self.assertNotIn("private-path-value", output)
        report = json.loads(self.stdout.getvalue())
        self.assertNotIn("PATH", report["environment"])

    @unittest.skipIf(os.name == "nt", "macOS library search")
    def test_macos_library_search_preserves_child_paths_without_exporting_them(self):
        libraries = str(self.root / "lib")
        caller = self.base / "private-caller-libraries"
        self.file(caller / "caller-required.dylib")
        self.file(caller / "libopencv_core.dylib")
        self.environment["DYLD_LIBRARY_PATH"] = ":".join((libraries, str(caller), libraries))
        script = ("import os, pathlib, sys; "
                  "paths = [pathlib.Path(path) for path in os.environ['DYLD_LIBRARY_PATH'].split(':')]; "
                  "assert any((path / 'caller-required.dylib').is_file() for path in paths); "
                  "assert next(path for path in paths if (path / 'libopencv_core.dylib').is_file()) == pathlib.Path(sys.argv[1])")
        self.assertEqual(self.invoke([sys.executable, "-c", script, libraries], exports=False), 0)
        self.assertEqual(self.invoke([], exports=False), 0)
        self.assertNotIn(str(caller), self.stdout.getvalue())
        self.file(self.env_file, "EXISTING=value\n")
        self.file(self.path_file, "/existing/path\n")
        self.assertNotEqual(self.invoke(), 0)
        self.assertFalse(self.marker.exists())
        self.assertEqual(self.env_file.read_text(encoding="utf-8"), "EXISTING=value\n")
        self.assertEqual(self.path_file.read_text(encoding="utf-8"), "/existing/path\n")
        alias = self.base / "selected-library-alias"
        alias.symlink_to(libraries, target_is_directory=True)
        self.environment["DYLD_LIBRARY_PATH"] = ":".join((libraries + "/", str(alias), libraries))
        self.assertEqual(self.invoke(), 0)
        self.assertNotIn(str(caller), self.env_file.read_text(encoding="utf-8"))

    def test_export_injection_does_not_append_either_file(self):
        self.file(self.env_file, "EXISTING=value\n")
        self.file(self.path_file, "/existing/path\n")
        for report in (
            {"environment": {"LIBCLANG_PATH": "valid\nINJECTED=value"}, "path_prepend": ["/valid"]},
            {"environment": {"LIBCLANG_PATH": "/valid"}, "path_prepend": ["/valid\n::add-mask::injected"]},
        ):
            with self.subTest(report=report), self.assertRaises(ValueError):
                setup_native.export_github(report, self.env_file, self.path_file)
            self.assertEqual(self.env_file.read_text(encoding="utf-8"), "EXISTING=value\n")
            self.assertEqual(self.path_file.read_text(encoding="utf-8"), "/existing/path\n")

    def test_child_failure_is_propagated(self):
        self.assertEqual(self.invoke([sys.executable, "-c", "raise SystemExit(23)"], exports=False), 23)

    @unittest.skipIf(os.name == "nt", "creating symlinks requires a Windows developer privilege")
    def test_child_failure_preserves_multicall_executable_name(self):
        dispatcher = self.file(self.tools / "rustup", "#!/usr/bin/env python3\n"
                               "from pathlib import Path\nimport sys\n"
                               "raise SystemExit(23 if Path(sys.argv[0]).name == 'cargo' else 24)\n")
        dispatcher.chmod(0o755)
        (self.tools / "cargo").symlink_to(dispatcher)
        self.assertEqual(self.invoke(["cargo"], exports=False), 23)


if __name__ == "__main__":
    unittest.main()
