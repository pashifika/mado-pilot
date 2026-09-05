#!/usr/bin/env python3
"""Validate installed native development prerequisites; configure only a child or CI step."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent / "native-release-profile"))
from process_runner import run_process  # noqa: E402


OPENCV_DISCOVERY = (
    "OPENCV_PACKAGE_NAME", "OPENCV_PKGCONFIG_NAME", "OPENCV_CMAKE_NAME",
    "OPENCV_CMAKE_BIN", "OPENCV_VCPKG_NAME", "OPENCV_LINK_LIBS",
    "OPENCV_LINK_PATHS", "OPENCV_INCLUDE_PATHS", "OPENCV_DISABLE_PROBES",
    "OPENCV_CMAKE_TOOLCHAIN_FILE", "OPENCV_CMAKE_ARGS",
)
CPP_PROBE = r"""
#include <opencv2/core.hpp>
#include <opencv2/core/version.hpp>
#include <opencv2/imgproc.hpp>
#include <opencv2/imgcodecs.hpp>
#include <iostream>
#include <vector>
#if defined(_WIN32)
# if !defined(_MSC_VER) || !defined(_M_X64)
#  error An x64 MSVC development environment is required
# endif
#elif !defined(__APPLE__) || !defined(__aarch64__)
# error A native arm64 macOS development environment is required
#endif
static_assert(sizeof(void*) == 8 && CV_VERSION_MAJOR == 4);
int main() {
    cv::Mat image(2, 3, CV_8UC3, cv::Scalar(0, 0, 255)), gray;
    cv::cvtColor(image, gray, cv::COLOR_BGR2GRAY);
    if (gray.type() != CV_8UC1 || gray.at<unsigned char>(0, 0) != 76) return 1;
    std::vector<unsigned char> encoded;
    if (!cv::imencode(".png", gray, encoded)) return 2;
    const cv::Mat decoded = cv::imdecode(encoded, cv::IMREAD_GRAYSCALE);
    if (decoded.size() != gray.size() || decoded.type() != gray.type()
        || cv::countNonZero(decoded != gray) != 0) return 3;
    std::cout << cv::getVersionString() << '\n';
}
"""
LIBCLANG_PROBE = """
import ctypes
import os
from pathlib import Path
import sys
path = Path(sys.argv[1])
directory = os.add_dll_directory(str(path.parent)) if os.name == "nt" else None
library = ctypes.CDLL(str(path))
class CXString(ctypes.Structure):
    _fields_ = [("data", ctypes.c_void_p), ("private_flags", ctypes.c_uint)]
library.clang_getClangVersion.argtypes = []
library.clang_getClangVersion.restype = CXString
library.clang_getCString.argtypes = [CXString]
library.clang_getCString.restype = ctypes.c_char_p
library.clang_disposeString.argtypes = [CXString]
library.clang_disposeString.restype = None
library.clang_createIndex.argtypes = [ctypes.c_int, ctypes.c_int]
library.clang_createIndex.restype = ctypes.c_void_p
library.clang_disposeIndex.argtypes = [ctypes.c_void_p]
library.clang_disposeIndex.restype = None
version = library.clang_getClangVersion()
try:
    print(library.clang_getCString(version).decode("utf-8"))
finally:
    library.clang_disposeString(version)
index = library.clang_createIndex(0, 0)
if not index:
    raise RuntimeError("libclang could not create an index")
library.clang_disposeIndex(index)
"""


def native_target() -> str:
    machine = platform.machine().lower()
    if struct.calcsize("P") == 8:
        if sys.platform == "darwin" and machine == "arm64":
            return "aarch64-apple-darwin"
        if sys.platform == "win32" and machine == "amd64":
            return "x86_64-pc-windows-msvc"
    raise ValueError("use native arm64 macOS or AMD64 Windows with 64-bit Python >=3.13 (not an emulated Python)")


def single_line(value: str, label: str) -> str:
    if any(ord(character) < 32 or ord(character) == 127 for character in value):
        raise ValueError(f"{label} must not contain control characters; use a single-line path or value")
    return value


def installed(path: Path, label: str, remedy: str, *, directory: bool = False) -> Path:
    single_line(str(path), label)
    resolved = path.expanduser().resolve()
    single_line(str(resolved), label)
    if not (resolved.is_dir() if directory else resolved.is_file()):
        raise ValueError(f"missing {label}: {resolved}; {remedy}")
    return resolved


def executable(name: str, environment: dict[str, str], remedy: str) -> Path:
    single_line(name, "executable")
    found = shutil.which(name, path=environment.get("PATH", os.defpath))
    if found is None:
        raise ValueError(f"missing executable {name}; {remedy}")
    installed(Path(found), name, remedy)
    # Keep argv[0]'s basename: clang++ and rustup's cargo proxy may be symlinks.
    return Path(found).absolute()


def checked(argv: list[str], scratch: Path, environment: dict[str, str], label: str,
            remedy: str, *, seconds: int = 30) -> str:
    result = run_process(argv, cwd=scratch, env=environment, timeout_seconds=seconds,
                         output_limit_bytes=131072)
    if (result["exit_code"] != 0 or result["timed_out"] or result["output_limited"]
            or not result["cleanup_ok"] or result["launch_error"] is not None):
        detail = result["launch_error"] or result["stderr"] or result["stdout"]
        raise ValueError(
            f"{label} failed (exit={result['exit_code']}, timeout={result['timed_out']}, "
            f"output_limit={result['output_limited']}, cleanup={result['cleanup_ok']}); "
            f"{remedy}; diagnostic={json.dumps(detail)}"
        )
    return result["stdout"].strip()


def header_version(include: Path) -> tuple[int, int, int]:
    header = installed(include / "opencv2/core/version.hpp", "OpenCV version header",
                       "select an OpenCV 4 development installation with --opencv-root")
    text = header.read_text(encoding="utf-8")
    numbers = [re.search(rf"^\s*#\s*define\s+CV_VERSION_{part}\s+(\d+)\s*$", text, re.MULTILINE)
               for part in ("MAJOR", "MINOR", "REVISION")]
    if not all(numbers):
        raise ValueError(f"unrecognized OpenCV version header: {header}; select an intact OpenCV 4 installation")
    version = tuple(int(number.group(1)) for number in numbers)
    if version[0] != 4:
        raise ValueError(f"OpenCV {version[0]} is unsupported; select OpenCV 4 with --opencv-root")
    return version


def matching_version(value: str, expected: tuple[int, int, int], label: str) -> None:
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+][A-Za-z0-9.-]+)?", value)
    if match is None or tuple(map(int, match.group(1, 2, 3))) != expected:
        raise ValueError(f"{label} version {value!r} does not match OpenCV {'.'.join(map(str, expected))}; "
                         "select matching OpenCV 4 headers, libraries and pkg-config metadata")


def target_settings(name: str, value: str, target: str) -> dict[str, str]:
    return {key: value for key in (name, f"{name}_{target}", f"{name}_{target.replace('-', '_')}",
                                   f"HOST_{name}", f"TARGET_{name}")}


def prepare(opencv_root: Path | None, libclang_path: Path | None,
            base_environment: dict[str, str]) -> tuple[dict, dict[str, str]]:
    target = native_target()
    windows = target == "x86_64-pc-windows-msvc"
    environment = dict(base_environment)
    if not windows:
        # Probes change directory; resolve inherited paths from the caller first.
        inherited_library_paths = [
            Path(path).resolve()
            for path in base_environment.get("DYLD_LIBRARY_PATH", "").split(":") if path
        ]
        environment["DYLD_LIBRARY_PATH"] = ":".join(map(str, inherited_library_paths))
    settings = {key: "" for key in OPENCV_DISCOVERY}
    settings.update(OPENCV_PACKAGE_NAME="opencv4", OPENCV_PKGCONFIG_NAME="opencv4",
                    OPENCV_CMAKE_NAME="OpenCV", OPENCV_VCPKG_NAME="opencv4")
    with tempfile.TemporaryDirectory(prefix="mado-native-") as temporary:
        scratch = Path(temporary).resolve()
        if windows:
            if opencv_root is None or libclang_path is None:
                raise ValueError("Windows requires --opencv-root DIR and --libclang-path DIR; "
                                 "provide an extracted OpenCV 4 archive and an installed LLVM bin directory")
            if (environment.get("VSCMD_ARG_TGT_ARCH", "").lower() not in ("x64", "amd64")
                    or not environment.get("INCLUDE") or not environment.get("LIB")):
                raise ValueError("missing x64 MSVC developer environment; run from an x64 Native Tools "
                                 "Command Prompt or call vcvars64.bat before running this script")
            compiler = executable("cl.exe", environment, "initialize an x64 MSVC developer prompt with C++ build tools")
        else:
            xcrun = executable("xcrun", environment, "install and select Xcode Command Line Tools")
            sdk = installed(Path(checked([str(xcrun), "--sdk", "macosx", "--show-sdk-path"], scratch, environment,
                                         "macOS SDK discovery", "install and select Xcode Command Line Tools")),
                            "macOS SDK", "install and select a macOS SDK", directory=True)
            settings["SDKROOT"] = str(sdk)
            environment["SDKROOT"] = str(sdk)
            clang = executable(checked([str(xcrun), "--find", "clang"], scratch, environment,
                                       "Clang discovery", "install and select Xcode Command Line Tools"),
                               environment, "select an intact Xcode toolchain")
            compiler = executable(checked([str(xcrun), "--find", "clang++"], scratch, environment,
                                          "C++ compiler discovery", "install and select Xcode Command Line Tools"),
                                  environment, "select an intact Xcode toolchain")
            if opencv_root is None:
                brew = executable("brew", environment, "install OpenCV 4 separately or supply --opencv-root DIR")
                opencv_root = Path(checked([str(brew), "--prefix", "opencv@4"], scratch, environment,
                                          "OpenCV discovery", "install opencv@4 separately or supply --opencv-root DIR"))
            if libclang_path is None:
                libclang_path = clang.parent.parent / "lib"
        root = installed(opencv_root, "OpenCV root", "supply --opencv-root with an installed OpenCV 4 root", directory=True)
        # OpenCV's discovery lists are comma-separated; PATH is platform-separated.
        separator = ";" if windows else ":"
        if "," in str(root) or separator in str(root):
            raise ValueError("OpenCV root contains a discovery-list separator; use an installation path without commas "
                             f"or {separator!r}")
        clang_dir = installed(libclang_path, "libclang directory", "supply --libclang-path DIR from LLVM or Xcode", directory=True)
        if separator in str(clang_dir):
            raise ValueError(f"libclang directory contains {separator!r}; use a path without PATH separators")
        libclang = installed(clang_dir / ("libclang.dll" if windows else "libclang.dylib"), "libclang shared library",
                             "install a native LLVM or Xcode toolchain separately and select its libclang directory")
        include = root / ("build/include" if windows else "include/opencv4")
        version = header_version(include)
        for module in ("core", "imgproc", "imgcodecs"):
            installed(include / f"opencv2/{module}.hpp", f"OpenCV {module} header", "select a complete OpenCV 4 development installation")
        for name in ("cvconfig.h", "opencv_modules.hpp"):
            installed(include / "opencv2" / name, f"OpenCV {name}", "select a complete OpenCV 4 development installation")
        libraries = root / ("build/x64/vc16/lib" if windows else "lib")
        runtime = root / "build/x64/vc16/bin" if windows else libraries
        settings.update(LIBCLANG_PATH=str(clang_dir), OPENCV_INCLUDE_PATHS=str(include), OPENCV_LINK_PATHS=str(libraries))
        if windows:
            clang = installed(clang_dir / "clang.exe", "LLVM clang.exe", "select the bin directory of a complete native LLVM installation")
            library_name = "opencv_world" + "".join(map(str, version))
            link_flags = [str(installed(libraries / f"{library_name}.lib", "OpenCV x64 import library",
                                        "select the OpenCV 4 archive root containing build/x64/vc16/lib"))]
            installed(runtime / f"{library_name}.dll", "OpenCV x64 runtime DLL",
                      "select the matching OpenCV 4 archive root containing build/x64/vc16/bin")
            settings.update(OPENCV_LINK_LIBS=library_name, OPENCV_MSVC_CRT="dynamic",
                            OPENCV_DISABLE_PROBES="pkg_config,cmake,vcpkg_cmake,vcpkg", CL="", _CL_="", LINK="")
            settings.update(INCLUDE=environment["INCLUDE"], LIB=environment["LIB"],
                            LIBPATH=environment.get("LIBPATH", ""),
                            VSCMD_ARG_TGT_ARCH=environment["VSCMD_ARG_TGT_ARCH"])
            paths = [str(runtime), str(clang_dir), str(compiler.parent)]
            cflags = []
        else:
            for module in ("core", "imgproc", "imgcodecs"):
                installed(libraries / f"libopencv_{module}.dylib", f"OpenCV {module} shared library",
                          "select a native shared OpenCV 4 installation")
            pkg_dir = installed(libraries / "pkgconfig", "OpenCV pkg-config directory",
                                "select an OpenCV 4 installation providing lib/pkgconfig/opencv4.pc", directory=True)
            installed(pkg_dir / "opencv4.pc", "opencv4.pc", "install OpenCV 4 development metadata separately")
            pkg_config = executable("pkg-config", environment, "install pkg-config separately and add it to PATH")
            settings.update(OPENCV_DISABLE_PROBES="environment,cmake,vcpkg_cmake,vcpkg", OPENCV_LINK_LIBS="+",
                            OPENCV_MSVC_CRT="dynamic", OPENCV4_DYNAMIC="1")
            settings.update(target_settings("PKG_CONFIG", str(pkg_config), target))
            for name, value in (("PKG_CONFIG_PATH", str(pkg_dir)), ("PKG_CONFIG_LIBDIR", str(pkg_dir)),
                                ("PKG_CONFIG_SYSROOT_DIR", "/")):
                settings.update(target_settings(name, value, target))
            for name in ("OPENCV4_STATIC", "OPENCV4_NO_PKG_CONFIG"):
                if name in environment:
                    raise ValueError(f"{name} conflicts with shared OpenCV 4 discovery; unset it before running setup")
            settings["DYLD_LIBRARY_PATH"] = str(libraries)
            paths = [str(compiler.parent), str(pkg_config.parent)]
        settings.update(CLANG_PATH=str(clang))
        settings.update(target_settings("CC", str(clang if not windows else compiler), target))
        settings.update(target_settings("CXX", str(compiler), target))
        for name, value in settings.items():
            single_line(value, name)
        environment.update(settings)
        environment["PATH"] = separator.join(dict.fromkeys(paths + [environment.get("PATH", os.defpath)]))
        if not windows:
            library_paths = [str(libraries)]
            library_identities = {libraries.resolve()}
            for identity in inherited_library_paths:
                if identity not in library_identities:
                    library_identities.add(identity)
                    library_paths.append(str(identity))
            environment["DYLD_LIBRARY_PATH"] = ":".join(library_paths)
            def pkg(*arguments: str) -> str:
                return checked([str(pkg_config), *arguments, "opencv4"], scratch, environment,
                               "OpenCV pkg-config probe", "select matching OpenCV 4 development metadata and dependencies")
            for variable, expected in (("pcfiledir", pkg_dir), ("includedir", include), ("libdir", libraries)):
                if Path(pkg("--variable=" + variable)).resolve() != expected.resolve():
                    raise ValueError(f"opencv4.pc {variable} points outside the selected OpenCV root; "
                                     "select an intact installation or repair its pkg-config metadata")
            matching_version(pkg("--modversion"), version, "pkg-config")
            cflags = shlex.split(pkg("--cflags"))
            link_flags = ["-L", str(libraries), *shlex.split(pkg("--libs")),
                          "-Xlinker", "-rpath", "-Xlinker", str(libraries)]
        clang_version = checked([str(Path(sys.executable).resolve()), "-I", "-c", LIBCLANG_PROBE, str(libclang)],
                                scratch, environment, "libclang load/API probe",
                                "select native loadable libclang and install its matching dependencies separately")
        source = scratch / "opencv-probe.cpp"
        source.write_text(CPP_PROBE, encoding="utf-8")
        binary = scratch / ("opencv-probe.exe" if windows else "opencv-probe")
        if windows:
            argv = [str(compiler), "/nologo", "/utf-8", "/MD", "/EHsc", "/std:c++17", "/I" + str(include),
                    str(source), "/Fo:" + str(scratch / "opencv-probe.obj"), "/Fe:" + str(binary), *link_flags]
        else:
            argv = [str(compiler), "-std=c++17", "-arch", "arm64", "-I", str(include), *cflags,
                    str(source), "-o", str(binary), *link_flags]
        checked(argv, scratch, environment, "OpenCV C++ compile/link probe",
                "install matching native C++ tools, OpenCV 4 headers and shared libraries separately", seconds=120)
        runtime_version = checked([str(binary)], scratch, environment, "OpenCV image/runtime probe",
                                  "select matching native OpenCV 4 shared libraries with PNG support and their dependencies")
        matching_version(runtime_version, version, "loaded OpenCV")
    report = {"target": target, "opencv_version": runtime_version, "opencv_root": str(root),
              "libclang_path": str(clang_dir), "libclang_version": clang_version,
              "environment": settings, "path_prepend": list(dict.fromkeys(paths))}
    return report, environment


def export_github(report: dict, env_file: Path, path_file: Path) -> None:
    for path in (env_file, path_file):
        single_line(str(path), "GitHub output file")
    if env_file.resolve() == path_file.resolve():
        raise ValueError("--github-env and --github-path must name different files")
    lines = []
    for name, value in report["environment"].items():
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", name):
            raise ValueError("invalid GitHub environment variable name")
        single_line(value, name)
        if name != "PATH":
            lines.append(f"{name}={value}\n")
    # Actions prepends the last path first; preserve the child's search order.
    paths = [single_line(value, "GitHub PATH entry") + "\n" for value in reversed(report["path_prepend"])]
    with env_file.open("a", encoding="utf-8", newline="\n") as env_stream, path_file.open("a", encoding="utf-8", newline="\n") as path_stream:
        env_stream.write("\n" + "".join(lines))
        path_stream.write("\n" + "".join(paths))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     epilog="With no command, print selected settings as JSON. Append -- COMMAND ARG... to run a child.")
    parser.add_argument("--opencv-root", type=Path, metavar="DIR", help="installed OpenCV 4 root (Windows: contains build/)")
    parser.add_argument("--libclang-path", type=Path, metavar="DIR", help="directory containing libclang.dylib or libclang.dll")
    parser.add_argument("--github-env", type=Path, metavar="FILE", help="append validated settings to this GitHub environment file")
    parser.add_argument("--github-path", type=Path, metavar="FILE", help="append validated search paths; requires --github-env")
    arguments = list(sys.argv[1:] if argv is None else argv)
    command = []
    if "--" in arguments:
        index = arguments.index("--")
        arguments, command = arguments[:index], arguments[index + 1:]
        if not command:
            parser.error("-- requires a command")
    args = parser.parse_args(arguments)
    if (args.github_env is None) != (args.github_path is None):
        parser.error("--github-env and --github-path must be supplied together")
    try:
        if sys.version_info < (3, 13):
            raise ValueError("Python >=3.13 is required; run this script with a current native Python installation")
        report, environment = prepare(args.opencv_root, args.libclang_path, dict(os.environ))
        if command:
            program = executable(command[0], environment, "provide an executable command after --")
            if sys.platform == "win32" and program.suffix.lower() in (".bat", ".cmd"):
                raise ValueError("commands after -- must be executables, not .bat/.cmd files; invoke a shell explicitly if needed")
        if args.github_env is not None:
            if ("DYLD_LIBRARY_PATH" in report["environment"]
                    and environment["DYLD_LIBRARY_PATH"] != report["environment"]["DYLD_LIBRARY_PATH"]):
                raise ValueError("GitHub export cannot retain inherited DYLD_LIBRARY_PATH entries without disclosing them; "
                                 "use -- COMMAND without GitHub export options")
            export_github(report, args.github_env, args.github_path)
        if command:
            status = subprocess.call([str(program), *command[1:]], env=environment)
            return 128 - status if status < 0 and sys.platform != "win32" else status
        print(json.dumps(report, ensure_ascii=True, indent=2))
        return 0
    except (OSError, ValueError) as error:
        print(f"native setup failed: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
