"""Build and run external public consumers on a development host, not a clean-system gate."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import sys

from process_runner import run_process
from qualify import digest_file, native_target, private_directory, write_bytes, write_record


def checked(argv: list[str], *, cwd: Path, environment: dict, output: Path, label: str,
            rows: list[dict], marker: str | None = None, expected_exit: int = 0,
            observe_modules: bool = False, missing_preload: bool = False,
            required_modules: tuple[Path, ...] = ()) -> dict:
    executable_digest = digest_file(Path(argv[0]))
    loaded = environment.get("MADO_PROFILE_LIBRARY")
    loaded_identity = digest_file(Path(loaded)) if loaded and Path(loaded).is_file() else None
    result = run_process(argv, cwd=cwd, env=environment, timeout_seconds=120,
                         output_limit_bytes=1048576)
    row = {"id": label, "argv": argv, "executable_sha256": executable_digest,
           "process": {key: value for key, value in result.items() if key not in ("stdout", "stderr")},
           "expected_exit": expected_exit, "required_stdout": marker, "status": "failed"}
    row["loaded_artifact_sha256"] = loaded_identity
    rows.append(row)
    for stream in ("stdout", "stderr"):
        write_bytes(output / f"{label}.{stream}.log", result[stream].encode("utf-8"))
    passed = (result["exit_code"] == expected_exit and not result["timed_out"] and not result["output_limited"]
              and result["cleanup_ok"] and result["launch_error"] is None)
    if marker is not None:
        passed = passed and result["stdout"].splitlines().count(marker) == 1
    if observe_modules:
        observed = (result["stdout"].splitlines().count("MADO_PROFILE_MODULES=complete") == 1
                    and "MADO_PROFILE_MODULES=incomplete" not in result["stdout"].splitlines())
        row["module_observation"] = "complete" if observed else "incomplete"
        passed = passed and observed
    if required_modules:
        modules = {os.path.normcase(line.removeprefix("MADO_PROFILE_MODULE="))
                   for line in result["stdout"].splitlines() if line.startswith("MADO_PROFILE_MODULE=")}
        retained = all(os.path.normcase(str(path)) in modules for path in required_modules)
        row["required_modules_retained"] = retained
        passed = passed and retained
    if missing_preload:
        lines = result["stdout"].splitlines()
        modules = [line.removeprefix("MADO_PROFILE_MODULE=") for line in lines
                   if line.startswith("MADO_PROFILE_MODULE=")]
        refused = ("MADO_PROFILE_LOAD=loaded" not in lines
                   and "MADO_PROFILE_PRELOAD=loaded" not in lines
                   and all(os.path.normcase(path) != os.path.normcase(loaded) for path in modules))
        row["preload_refused_before_candidate"] = refused
        passed = passed and refused
    if not passed:
        raise ValueError(f"{label} failed; see bounded attempt logs")
    row["status"] = "passed"
    return result


def build_and_run(root: Path, libraries: Path, output: Path, model_root: Path, runtime: Path,
                  opencv_runtime: Path | None = None) -> dict:
    root, libraries = root.resolve(strict=True), libraries.resolve(strict=True)
    model_root, runtime = model_root.resolve(strict=True), runtime.resolve(strict=True)
    windows = sys.platform == "win32"
    if windows:
        if opencv_runtime is None or not opencv_runtime.is_absolute():
            raise ValueError("--opencv-runtime requires an absolute Windows DLL path")
        opencv_runtime = opencv_runtime.resolve(strict=True)
        if not opencv_runtime.is_file():
            raise ValueError("--opencv-runtime requires a file")
    elif opencv_runtime is not None:
        raise ValueError("--opencv-runtime is supported only on Windows")
    private_directory(output)  # Never replace a previous consumer attempt.
    output = output.resolve(strict=True)
    if native_target() == "unsupported":
        raise ValueError("external consumers require a native release target")
    environment = dict(os.environ)
    if not windows:
        environment["MACOSX_DEPLOYMENT_TARGET"] = "26.5.2"
    suffix = ".exe" if windows else ""
    library = libraries / ("madopilot.dll" if windows else "libmadopilot.dylib")
    import_library = libraries / "madopilot.dll.lib" if windows else library
    include = root / "crates/bindings/capi/include"
    scene = root / "crates/bindings/capi/examples"
    sources = root / "tools/native-release-profile/consumers"
    compiler = shutil.which("cl" if windows else "clang")
    cpp_compiler = compiler if windows else shutil.which("clang++")
    rustc = shutil.which("rustc")
    if not all((compiler, cpp_compiler, rustc)):
        raise ValueError("native C/C++ and pinned Rust compilers required")
    rows = []
    record = {"schema_version": 1, "target": native_target(), "admission": "development-host",
              "qualification": "not-selected", "library_sha256": digest_file(library),
              "library_bytes": library.stat().st_size, "rows": rows, "status": "failed"}
    if windows:
        record["opencv_runtime"] = {"bytes": opencv_runtime.stat().st_size,
                                    "sha256": digest_file(opencv_runtime)}
    inputs = [sources / name for name in ("rust_facade.rs", "c_abi.c", "cpp_wrapper.cpp")]
    inputs += [root / "tools/native-release-profile/host_load.c", include / "madopilot/madopilot.h",
               include / "madopilot/madopilot.hpp", scene / "deterministic-scene.h"]
    utf8_entry = root / "tools/native-release-profile/windows_utf8_entry.h"
    inputs.append(utf8_entry)
    for version in ("v1", "v1_2", "v1_3", "v1_4"):
        prefix = root / "crates/bindings/capi/tests/abi-compat" / version
        inputs += [prefix / "old-prefix.c", prefix / "madopilot/madopilot.h"]
    record["source_files"] = [{"file": path.relative_to(root).as_posix(), "sha256": digest_file(path)}
                              for path in inputs]
    record["checkout_commit"] = environment.get("GITHUB_SHA")
    record["product_features"] = "not-observed"

    def compile_c(source: Path, name: str, *, cpp=False, headers=include, host_load=False):
        binary = output / (name + suffix)
        driver = cpp_compiler if cpp else compiler
        if windows:
            args = [driver, "/nologo", "/W3", "/WX", "/utf-8", "/EHsc", "/std:c++17" if cpp else "/std:c11",
                    "/I" + str(headers), "/I" + str(scene), str(source), "/Fe:" + str(binary)]
            args.append("/FI" + str(utf8_entry))
            if host_load:
                args += ["/DMADOPILOT_BUILDING", "/Dmadopilot_get_api=qualification_get_api",
                         str(output / "host_load.obj"), "psapi.lib"]
            else:
                args.append(str(import_library))
        else:
            args = [driver, "-std=c++17" if cpp else "-std=c11", "-Wall", "-Wextra", "-Werror",
                    "-I", str(headers), "-I", str(scene), str(source), "-o", str(binary)]
            if host_load:
                args += ["-Dmadopilot_get_api=qualification_get_api", str(output / "host_load.o")]
            else:
                args.append(str(import_library))
        checked(args, cwd=output, environment=environment, output=output, label="build-" + name, rows=rows)
        return binary

    try:
        shim = root / "tools/native-release-profile/host_load.c"
        if windows:
            shim_args = [compiler, "/nologo", "/W3", "/WX", "/utf-8", "/std:c11", "/c", str(shim),
                         "/I" + str(include), "/Fo:" + str(output / "host_load.obj")]
            bootstrap_args = [compiler, "/nologo", "/W3", "/WX", "/utf-8", "/std:c11", str(shim),
                              "/DPROFILE_RUST_MAIN", "/I" + str(include), "psapi.lib",
                              "/Fo:" + str(output / "rust-bootstrap.obj"),
                              "/Fe:" + str(output / ("rust-bootstrap" + suffix))]
        else:
            shim_args = [compiler, "-std=c11", "-Wall", "-Wextra", "-Werror", "-I", str(include),
                         "-c", str(shim), "-o", str(output / "host_load.o")]
            bootstrap_args = [compiler, "-std=c11", "-Wall", "-Wextra", "-Werror", "-I", str(include),
                              "-DPROFILE_RUST_MAIN", str(shim), "-o", str(output / "rust-bootstrap")]
        checked(shim_args, cwd=output, environment=environment, output=output, label="build-host-shim", rows=rows)
        checked(bootstrap_args, cwd=output, environment=environment, output=output, label="build-rust-bootstrap", rows=rows)
        facade = libraries / "libmado_pilot.rlib"
        rust_binary = output / ("rust-facade" + suffix)
        rust_args = [rustc, "--edition=2024", "-C", "opt-level=3", "-D", "warnings",
                     "--extern", "mado_pilot=" + str(facade), "-L", "dependency=" + str(libraries / "deps")]
        link_paths = environment.get("OPENCV_LINK_PATHS")
        if not link_paths and not windows:
            pkg_config = shutil.which("pkg-config")
            if not pkg_config:
                raise ValueError("explicit OPENCV_LINK_PATHS or builder pkg-config required")
            discovery = checked([pkg_config, "--variable=libdir", "opencv4"], cwd=output,
                                environment=environment, output=output, label="discover-opencv",
                                rows=rows)
            link_paths = discovery["stdout"].strip()
        for link_path in (link_paths or "").split(","):
            if link_path:
                rust_args += ["-L", "native=" + link_path]
        rust_source = sources / "rust_facade.rs"
        checked(rust_args + [str(rust_source), "-o", str(rust_binary)], cwd=output, environment=environment,
                output=output, label="build-rust-facade", rows=rows)
        rust_module = output / ("rust_probe.dll" if windows else "librust_probe.dylib")
        checked(rust_args + ["--crate-type", "cdylib", "--cfg", "qualification_module",
                            str(rust_source), "-o", str(rust_module)], cwd=output, environment=environment,
                output=output, label="build-rust-module", rows=rows)
        consumers = [("rust-facade", rust_binary, "MADO_PROFILE_RESULT=passed")]
        consumers += [("c-current", compile_c(sources / "c_abi.c", "c-current"), "MADO_PROFILE_RESULT=passed"),
                      ("cpp-current", compile_c(sources / "cpp_wrapper.cpp", "cpp-current", cpp=True), "MADO_PROFILE_RESULT=passed")]
        for version in ("v1", "v1_2", "v1_3", "v1_4"):
            headers = root / "crates/bindings/capi/tests/abi-compat" / version
            binary = compile_c(headers / "old-prefix.c", "prefix-" + version, headers=headers)
            consumers.append(("prefix-" + version, binary, f"madopilot-abi-compat-{version} complete"))
        if windows:
            environment["PATH"] = str(libraries) + os.pathsep + environment.get("PATH", "")
        package = root / "fixtures/assets/phase1-slice"
        for name, binary, marker in consumers:
            args = [str(binary), "--package", str(package)]
            if not name.startswith("prefix-"):
                args += ["--model-root", str(model_root), "--runtime", str(runtime)]
            checked(args, cwd=output, environment=environment, output=output, label="run-" + name,
                    rows=rows, marker=marker)
            record.setdefault("binaries", []).append({"file": binary.name, "bytes": binary.stat().st_size,
                                                       "sha256": digest_file(binary)})
        prototype_consumers = [
            ("c-host-load", compile_c(sources / "c_abi.c", "c-host-load", host_load=True), library),
            ("cpp-host-load", compile_c(sources / "cpp_wrapper.cpp", "cpp-host-load", cpp=True, host_load=True), library),
            ("rust-deferred", output / ("rust-bootstrap" + suffix), rust_module),
        ]
        common_args = ["--package", str(package), "--model-root", str(model_root), "--runtime", str(runtime)]
        if windows:
            environment["MADO_PROFILE_OPENCV_RUNTIME"] = str(opencv_runtime)
        for name, binary, loaded_library in prototype_consumers:
            environment["MADO_PROFILE_LIBRARY"] = str(loaded_library)
            required_modules = (loaded_library, opencv_runtime) if windows else (loaded_library,)
            checked([str(binary)] + common_args, cwd=output, environment=environment, output=output,
                    label="run-" + name, rows=rows, marker="MADO_PROFILE_RESULT=passed",
                    observe_modules=True, required_modules=required_modules)
            environment["MADO_PROFILE_LIBRARY"] = str(output / "absent-library")
            checked([str(binary)] + common_args, cwd=output, environment=environment, output=output,
                    label="missing-" + name, rows=rows, marker="MADO_PROFILE_LOAD=unavailable", expected_exit=1)
            if windows:
                environment["MADO_PROFILE_LIBRARY"] = str(loaded_library)
                environment["MADO_PROFILE_OPENCV_RUNTIME"] = str(output / "absent-opencv.dll")
                checked([str(binary)] + common_args, cwd=output, environment=environment, output=output,
                        label="missing-preload-" + name, rows=rows,
                        marker="MADO_PROFILE_PRELOAD=unavailable", expected_exit=1,
                        observe_modules=True, missing_preload=True)
                environment["MADO_PROFILE_OPENCV_RUNTIME"] = str(opencv_runtime)
        record["status"] = "passed"
    finally:
        write_record(output / "result.json", record)
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("root", "library-dir", "output", "model-root", "runtime"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--opencv-runtime", type=Path, required=sys.platform == "win32")
    args = parser.parse_args()
    try:
        record = build_and_run(args.root, args.library_dir, args.output, args.model_root, args.runtime,
                               args.opencv_runtime)
        print(json.dumps({key: record[key] for key in ("status", "target", "admission", "qualification")}))
        return 0
    except (OSError, ValueError) as error:
        detail = str(error) if isinstance(error, ValueError) else type(error).__name__
        print(f"consumer lane failed: {detail}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
