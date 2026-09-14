#!/usr/bin/env python3
"""Execute one pre-bound native pacing protocol; never build, install, or retry."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import stat
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / "tools/native-release-profile"))
from process_runner import run_process
from inspect_native import inspect_file

sys.path.insert(0, str(ROOT / "tools/ocr-text-watch"))
from run_replay import MAX_IDENTITY_BYTES, identity

sys.path.insert(0, str(HERE))
from report import analyze_case, compare_cases

CASES = ("semantic", "capture-off", "baseline", "cooldown-only", "native-only", "combined")
TARGETS = {"Windows": "x86_64-pc-windows-msvc", "Darwin": "aarch64-apple-darwin"}
SOURCE_IMAGE_SHA256 = "10f0163cf298453e55922cc6104ee3066f59a328a55e9bac6c04f24d36c9e288"
WINDOWS_IMAGE_SHA256 = "3ca2f418f6fb083e49a679638017609e11fe825ef4a1e6f56dc0e572c74f75e6"
OUTPUT_LIMIT = 1024 * 1024
JSON_LIMIT = 256 * 1024
FIXTURE_LIMIT = 128 * 1024
CLEANUP_SECONDS = 10
OS_ENV = {
    "SystemRoot", "SYSTEMROOT", "WINDIR", "COMSPEC", "ComSpec", "USERPROFILE",
    "LOCALAPPDATA", "APPDATA", "PROGRAMDATA", "ProgramData", "HOME", "USER",
    "LOGNAME", "LANG", "LC_ALL", "__CF_USER_TEXT_ENCODING", "SECURITYSESSIONID",
}


class Refusal(ValueError):
    """A fixed, payload-free authority or evidence refusal."""


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise Refusal(reason)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate-json-key")
        result[key] = value
    return result


def regular(path: Path, maximum: int, *, single_link: bool = False) -> os.stat_result:
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and not path.is_symlink(), "not-regular-file")
    require(not getattr(info, "st_file_attributes", 0) & 0x400, "reparse-file")
    require(info.st_size <= maximum, "file-bound")
    require(not single_link or info.st_nlink == 1, "linked-control-file")
    return info


def read_json(path: Path, maximum: int = JSON_LIMIT):
    regular(path, maximum, single_link=True)
    with path.open("rb") as source:
        data = source.read(maximum + 1)
    require(len(data) <= maximum, "json-too-large")
    return json.loads(data, object_pairs_hook=unique_object)


def digest(path: Path) -> tuple[int, str]:
    regular(path, MAX_IDENTITY_BYTES)
    observed = identity(path)
    regular(path, MAX_IDENTITY_BYTES)
    return observed["bytes"], observed["sha256"]


def bound_file(record: dict) -> Path:
    require(isinstance(record, dict) and set(record) == {"path", "bytes", "sha256"}, "file-binding-shape")
    require(type(record["bytes"]) is int and record["bytes"] >= 0, "file-binding-length")
    require(isinstance(record["sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", record["sha256"]), "file-binding-digest")
    path = Path(record["path"])
    require(path.is_absolute() and path == path.resolve(strict=True), "noncanonical-input")
    require(digest(path) == (record["bytes"], record["sha256"]), "file-binding-mismatch")
    return path


def git(*arguments: str) -> str:
    result = subprocess.run(["git", "-C", str(ROOT), *arguments], capture_output=True, timeout=5, check=False)
    require(result.returncode == 0, "source-inspection-failed")
    return result.stdout.decode("ascii").strip()


def source_identity(authority: dict) -> None:
    require(git("rev-parse", "HEAD") == authority["source_commit"], "source-commit-mismatch")
    require(git("rev-parse", "HEAD^{tree}") == authority["source_tree"], "source-tree-mismatch")
    require(not git("status", "--porcelain", "--untracked-files=no"), "tracked-source-dirty")
    git("ls-files", "--error-unmatch", "--", "tools/capture-pacing/run.py",
        "tools/capture-pacing/report.py", "tools/capture-pacing/windows/fixture.cpp",
        "tools/capture-pacing/macos/fixture.m",
        "crates/mado-pilot/examples/capture-pacing-native.rs",
        "crates/mado-pilot/examples/support/capture_pacing_contract.rs",
        "crates/mado-pilot/examples/support/capture_pacing_control.rs",
        "crates/mado-pilot/examples/support/capture_pacing_loop.rs",
        "crates/mado-pilot/examples/support/capture_pacing_metrics.rs",
        "crates/mado-pilot/examples/support/capture_pacing_native.rs",
        "crates/mado-pilot/examples/support/completion_cooldown.rs")


def validate_authority(authority: dict) -> None:
    require(isinstance(authority, dict) and authority.get("schema") == 1, "authority-schema")
    require(authority.get("approved") is True, "execution-not-approved")
    require(authority.get("purpose") == "capture-pacing-native", "authority-purpose")
    require(authority.get("target") == TARGETS.get(platform.system()), "wrong-host-target")
    require(Path(authority["project_root"]).resolve(strict=True) == ROOT, "wrong-project-root")
    require(re.fullmatch(r"[0-9a-f]{40}", authority["source_commit"]) is not None, "source-commit-shape")
    require(re.fullmatch(r"[0-9a-f]{40}", authority["source_tree"]) is not None, "source-tree-shape")
    require(authority.get("cases") == list(CASES), "case-order-or-scope")
    require(authority.get("attempts") == 1 and type(authority["attempts"]) is int, "attempt-bound")
    require(set(authority["nonces"]) == set(CASES), "nonce-scope")
    values = list(authority["nonces"].values())
    require(all(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{16}", value) and int(value, 16) for value in values), "nonce-shape")
    require(len(set(values)) == len(values), "duplicate-nonce")
    require(authority.get("native_interval_ns") == 100_000_000, "native-interval-binding")
    require(authority.get("cooldown_ns") == 250_000_000, "cooldown-binding")
    require(authority.get("warmup_seconds") == 2 and authority.get("measurement_seconds") == 6, "measurement-binding")
    require(authority.get("input_authorized") is False and authority.get("permission_changes_authorized") is False, "forbidden-capability")
    require(isinstance(authority.get("loader_directories"), list) and 1 <= len(authority["loader_directories"]) <= 4, "loader-directory-bound")
    for directory in authority["loader_directories"]:
        path = Path(directory)
        require(path.is_absolute() and path.is_dir() and path == path.resolve(strict=True), "noncanonical-loader-directory")
    output = Path(authority["output_root"])
    require(output.is_absolute(), "relative-output-root")
    resolved = output.resolve()
    require(resolved.is_relative_to(ROOT / ".rasen") and resolved != ROOT / ".rasen", "output-outside-private-root")
    require(not output.exists() and not output.is_symlink(), "attempt-root-already-exists")


def host_snapshot() -> dict:
    """Observe only stable host identity; never enumerate or activate windows."""
    system = platform.system()
    require(system in TARGETS, "unsupported-host")
    if system == "Darwin":
        def query(*arguments):
            result = subprocess.run(arguments, capture_output=True, timeout=5, check=True)
            require(len(result.stdout) <= 128 * 1024, "host-output-bound")
            return result.stdout.decode("utf-8").strip()
        identity = query("/usr/sbin/ioreg", "-rd1", "-c", "IOPlatformExpertDevice")
        matches = re.findall(r'"IOPlatformUUID"\s*=\s*"([^"]+)"', identity)
        require(len(matches) == 1, "host-identity-unavailable")
        machine_id = matches[0]
        version = query("/usr/bin/sw_vers", "-productVersion")
        build = query("/usr/bin/sw_vers", "-buildVersion")
        cpu = query("/usr/sbin/sysctl", "-n", "machdep.cpu.brand_string")
        memory = int(query("/usr/sbin/sysctl", "-n", "hw.memsize"))
        processors = int(query("/usr/sbin/sysctl", "-n", "hw.logicalcpu"))
    else:
        import ctypes
        import winreg
        def registry(path, name):
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, path, access=winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
                return winreg.QueryValueEx(key, name)[0]
        current = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"
        machine_id = registry(r"SOFTWARE\Microsoft\Cryptography", "MachineGuid")
        version = registry(current, "DisplayVersion")
        build = f'{registry(current, "CurrentBuildNumber")}.{registry(current, "UBR")}'
        cpu = registry(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").strip()
        system_root = registry(current, "SystemRoot")
        class MemoryStatus(ctypes.Structure):
            _fields_ = [("length", ctypes.c_uint32), ("load", ctypes.c_uint32),
                        *[(name, ctypes.c_uint64) for name in
                          ("total", "available", "page_total", "page_available",
                           "virtual_total", "virtual_available", "extended_available")]]
        status = MemoryStatus()
        status.length = ctypes.sizeof(status)
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GlobalMemoryStatusEx.argtypes = [ctypes.POINTER(MemoryStatus)]
        kernel.GlobalMemoryStatusEx.restype = ctypes.c_int
        require(bool(kernel.GlobalMemoryStatusEx(ctypes.byref(status))), "host-memory-unavailable")
        memory = status.total
        processors = os.cpu_count()
    require(memory > 0 and type(processors) is int and processors > 0, "host-resources-unavailable")
    result = {"system": system, "architecture": platform.machine(), "os_version": version,
              "os_build": build, "cpu": cpu, "memory_bytes": memory, "logical_processors": processors,
              "machine_id_sha256": hashlib.sha256(machine_id.upper().encode("utf-8")).hexdigest()}
    if system == "Windows":
        result["system_root"] = system_root
    return result


def verify_host(authority: dict) -> dict:
    observed = host_snapshot()
    require(authority.get("host") == observed, "host-binding-mismatch")
    return observed


def windows_dll_directory_length() -> int:
    import ctypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetDllDirectoryW.argtypes = [ctypes.c_uint32, ctypes.c_wchar_p]
    kernel.GetDllDirectoryW.restype = ctypes.c_uint32
    ctypes.set_last_error(0)
    count = kernel.GetDllDirectoryW(0, None)
    require(count != 0 or ctypes.get_last_error() == 0, "dll-directory-query-failed")
    require(count <= 32768, "dll-directory-bound")
    buffer = ctypes.create_unicode_buffer(max(count, 1))
    ctypes.set_last_error(0)
    copied = kernel.GetDllDirectoryW(len(buffer), buffer)
    require(copied < len(buffer) and (copied != 0 or ctypes.get_last_error() == 0)
            and buffer[copied] == "\0", "dll-directory-readback-failed")
    return copied


def verify_native_bindings(authority: dict, inputs: dict[str, Path]) -> dict:
    """Bind static imports and lookup candidates, not unobserved actual loads."""
    extras = authority.get("native_inputs")
    require(isinstance(extras, list) and 1 <= len(extras) <= 64, "native-input-bound")
    native = [bound_file(record) for record in extras]
    require(len(set(native)) == len(native), "duplicate-native-input")
    allowed = set(native) | {inputs[name] for name in ("consumer", "fixture", "runtime")}
    inspector = bound_file(authority["native_inspector"])
    directories = [Path(item) for item in authority["loader_directories"]]
    system = platform.system()
    windows_root = Path(authority["host"]["system_root"]) if system == "Windows" else None
    lookup = set(directories) | {inputs[name].parent for name in ("consumer", "fixture", "runtime")}
    if system == "Windows":
        require(windows_dll_directory_length() == 0, "inherited-dll-directory")
        for executable in (inputs["consumer"], inputs["fixture"], inspector):
            for suffix in (".local", ".manifest"):
                redirection = executable.with_name(executable.name + suffix)
                try:
                    redirection.lstat()
                except FileNotFoundError:
                    pass
                else:
                    raise Refusal("external-loader-redirection")
        lookup.add(ROOT)  # The consumer's working directory is a loader search location.
    for directory in lookup:
        entries = list(directory.iterdir())
        require(len(entries) <= 4096, "native-directory-bound")
        for entry in entries:
            is_library = entry.name.lower().endswith(".dll" if system == "Windows" else ".dylib")
            if is_library:
                require(entry.is_file() and entry.resolve(strict=True) in allowed, "unbound-loader-candidate")
    def is_system(path: Path) -> bool:
        if system == "Windows":
            return path.is_relative_to(windows_root / "System32") or path.parent == windows_root
        return str(path).startswith(("/usr/lib/", "/System/Library/"))
    def expand(token: str, owner: Path, executable: Path) -> Path:
        if token == "@loader_path":
            return owner.parent
        if token == "@executable_path":
            return executable.parent
        if token.startswith("@loader_path/"):
            return owner.parent / token[len("@loader_path/"):]
        if token.startswith("@executable_path/"):
            return executable.parent / token[len("@executable_path/"):]
        path = Path(token)
        require(path.is_absolute(), "unsupported-native-rpath")
        return path
    inspected = {}
    checked = set()
    queue = [(inputs["consumer"], inputs["consumer"], ()), (inputs["fixture"], inputs["fixture"], ()),
             (inputs["runtime"], inputs["runtime"] if system == "Windows" else inputs["consumer"], ())]
    queue.extend((path, inputs["runtime"] if system == "Windows" and path.parent == inputs["runtime"].parent
                  else inputs["consumer"], ()) for path in native)
    deadline = time.monotonic() + 120
    edges = []
    while queue:
        owner, executable, inherited = queue.pop()
        identity = (owner, executable, inherited)
        if identity in checked:
            continue
        require(len(checked) < 256 and time.monotonic() < deadline, "native-inspection-bound")
        checked.add(identity)
        if owner not in inspected:
            require(len(inspected) < 67, "native-closure-bound")
            inspected[owner] = inspect_file(owner, inspector)
        metadata = inspected[owner]
        expected_arch = "x64" if system == "Windows" else "ARM64"
        require(metadata["architecture"] == expected_arch, "native-architecture-mismatch")
        require(len(metadata["imports"]) <= 256, "native-import-bound")
        rpaths = tuple(dict.fromkeys(
            [expand(item, owner, executable) for item in metadata.get("rpaths", [])] + list(inherited)))
        require(len(rpaths) <= 32, "native-rpath-bound")
        for imported in metadata["imports"]:
            token = imported["name"]
            if system == "Windows":
                require(Path(token).name == token and token.lower().endswith(".dll"), "native-import-name")
                if token.lower().startswith(("api-ms-win-", "ext-ms-win-")):
                    continue  # OS API-set contracts are bound by the exact OS build.
                if executable == inputs["runtime"]:
                    # The ONNX loader uses DLL_LOAD_DIR | SYSTEM32, not ambient PATH.
                    candidates = [inputs["runtime"].parent / token, windows_root / "System32" / token]
                else:
                    candidates = [executable.parent / token, windows_root / "System32" / token,
                                  windows_root / token, ROOT / token, *[path / token for path in directories]]
            elif token.startswith(("/usr/lib/", "/System/Library/")):
                continue
            else:
                candidates = [path / Path(token).name for path in directories]
                if token.startswith("@rpath/"):
                    candidates.extend(path / token[len("@rpath/"):] for path in rpaths)
                else:
                    candidates.append(expand(token, owner, executable))
            selected = next((path for path in candidates if path.is_file()
                             or (system != "Windows" and is_system(path))), None)
            require(selected is not None, "unresolved-native-import")
            if is_system(selected):
                if system == "Windows":
                    require(selected.is_file(), "missing-system-import")
                continue
            selected = selected.resolve(strict=True)
            require(selected in allowed, "unbound-native-import")
            edges.append({"owner": str(owner), "import": token, "resolved": str(selected)})
            queue.append((selected, executable, rpaths))
    require(time.monotonic() <= deadline, "native-inspection-bound")
    return {"scope": "Static imports and controlled lookup candidates; actual process images not observed.",
            "files": len(inspected), "edges": edges}

def child_environment(authority: dict, scratch: Path) -> dict[str, str]:
    environment = {key: value for key, value in os.environ.items() if key in OS_ENV}
    directories = authority["loader_directories"]
    if platform.system() == "Windows":
        system_root = authority["host"]["system_root"]
        environment = {key: value for key, value in environment.items() if key.upper() not in ("SYSTEMROOT", "WINDIR")}
        environment.update({"SystemRoot": system_root, "WINDIR": system_root})
        environment["PATH"] = os.pathsep.join([*directories, str(Path(system_root) / "System32"), system_root])
    else:
        environment["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
        environment["DYLD_LIBRARY_PATH"] = os.pathsep.join(directories)
        environment["DYLD_FALLBACK_LIBRARY_PATH"] = "/usr/lib"
        environment["DYLD_FALLBACK_FRAMEWORK_PATH"] = "/System/Library/Frameworks"
    environment.update({"TMP": str(scratch), "TEMP": str(scratch), "TMPDIR": str(scratch), "PYTHONDONTWRITEBYTECODE": "1"})
    return environment


def save(path: Path, value) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(value, output, ensure_ascii=False, indent=2, allow_nan=False)
        output.write("\n")
    temporary.replace(path)


def operational_success(result: dict) -> bool:
    return (result["exit_code"] == 0 and not result["timed_out"] and not result["output_limited"]
            and result["cleanup_ok"] and result["launch_error"] is None)


def aggregate_status(rows: list[dict], comparison: dict, binding_unchanged: bool) -> str:
    statuses = [row["analysis"].get("status") for row in rows]
    if not binding_unchanged or "fail" in statuses or comparison.get("status") == "fail":
        return "fail"
    if any(status not in ("pass", "not-run", "incomplete") for status in statuses):
        return "fail"
    if "incomplete" in statuses:
        return "incomplete"
    if "not-run" in statuses:
        return "incomplete" if "pass" in statuses else "not-run"
    if comparison.get("status") != "pass":
        return "incomplete"
    return "pass"

def execute(authority_path: Path) -> tuple[dict, int]:
    if os.name != "nt":
        os.umask(0o077)
    authority = read_json(authority_path)
    validate_authority(authority)
    observed_host = verify_host(authority)
    source_identity(authority)
    inputs = {name: bound_file(authority[name]) for name in ("consumer", "fixture", "runtime", "detector", "recognizer", "image")}
    model_root = Path(authority["model_root"]).resolve(strict=True)
    require(model_root.is_dir(), "model-root-invalid")
    require(inputs["detector"] == model_root / "rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx", "detector-root-binding")
    require(inputs["recognizer"] == model_root / "rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx", "recognizer-root-binding")
    expected_image = WINDOWS_IMAGE_SHA256 if platform.system() == "Windows" else SOURCE_IMAGE_SHA256
    require(authority["image"]["sha256"] == expected_image, "fixture-image-binding")
    native_binding = verify_native_bindings(authority, inputs)
    output_root = Path(authority["output_root"])
    output_root.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    output_root.mkdir(mode=0o700)  # Exclusive root is the one-attempt claim.
    save(output_root / "authority.json", authority)
    rows = []
    halt_reason = None
    started = time.monotonic()
    for case in CASES:
        if halt_reason:
            rows.append({"case": case, "consumer": None, "fixture": None, "stderr": "", "process": None,
                         "analysis": {"status": "not-run", "reason": halt_reason}})
            continue
        control_root = output_root / case
        control_root.mkdir(mode=0o700)
        scratch = control_root / "scratch"
        scratch.mkdir(mode=0o700)
        argv = [str(inputs["consumer"]), "--case", case, "--fixture", str(inputs["fixture"]),
                "--control-root", str(control_root), "--nonce", authority["nonces"][case],
                "--image", str(inputs["image"]), "--model-root", str(model_root), "--runtime", str(inputs["runtime"])]
        process = run_process(argv, cwd=ROOT, env=child_environment(authority, scratch),
                              timeout_seconds=180 if case == "semantic" else 120,
                              output_limit_bytes=OUTPUT_LIMIT, cleanup_seconds=CLEANUP_SECONDS)
        consumer = None
        fixture = None
        parse_reason = None
        try:
            consumer = json.loads(process["stdout"], object_pairs_hook=unique_object)
            require(isinstance(consumer, dict) and consumer.get("case") == case, "consumer-case-mismatch")
            if (control_root / "fixture-metrics.json").exists():
                fixture = read_json(control_root / "fixture-metrics.json", FIXTURE_LIMIT)
                require(fixture.get("nonce") == authority["nonces"][case], "fixture-nonce-mismatch")
            analysis = analyze_case(consumer, fixture, process["stderr"])
        except (OSError, ValueError, TypeError, KeyError):
            parse_reason = "invalid-or-missing-case-report"
            analysis = {"status": "fail", "reason": parse_reason}
        if not operational_success(process):
            permission_refusal = (
                parse_reason is None and consumer is not None
                and analysis.get("status") == "not-run"
                and consumer.get("permission") == "denied-or-undetermined"
                and consumer.get("semantic_status") == "not-run"
                and consumer.get("cleanup_status") == "pass"
                and process["exit_code"] in (1, 2) and process["launch_error"] is None
                and process["cleanup_ok"] and not process["timed_out"]
                and not process["output_limited"]
            )
            analysis = {"status": "not-run" if permission_refusal else "fail", "reason":
                        "permission-denied-or-undetermined" if permission_refusal else "process-or-cleanup-failed",
                        "case_analysis": analysis}
        row = {"case": case, "consumer": consumer, "fixture": fixture, "stderr": process["stderr"],
               "process": process, "analysis": analysis}
        rows.append(row)
        save(control_root / "result.json", row)
        save(control_root / "command.json", {"argv": argv, "parse_reason": parse_reason})
        save(output_root / "records.json", rows)
        if analysis.get("status") != "pass":
            halt_reason = analysis.get("reason", "prior-case-not-passed")
    save(output_root / "records.json", rows)
    comparison = compare_cases([row for row in rows if row["case"] != "semantic"])
    binding_unchanged = True
    try:
        source_identity(authority)
        for name in ("consumer", "fixture", "runtime", "detector", "recognizer", "image"):
            bound_file(authority[name])
        require(verify_host(authority) == observed_host, "host-changed")
        require(verify_native_bindings(authority, inputs) == native_binding, "native-binding-changed")
    except (OSError, ValueError, subprocess.SubprocessError):
        binding_unchanged = False
    status = aggregate_status(rows, comparison, binding_unchanged)
    report = {"schema": 2, "status": status,
              "source_commit": authority["source_commit"], "source_tree": authority["source_tree"],
              "bound_host": observed_host, "target": authority["target"], "binding_unchanged": binding_unchanged,
              "native_binding": native_binding,
              "elapsed_seconds": time.monotonic() - started, "halt_reason": halt_reason,
              "comparison": comparison, "case_statuses": {row["case"]: row["analysis"].get("status") for row in rows},
              "scope": "Owned GDI/AppKit HUD fixture only; unavailable metrics do not establish savings or broader support."}
    save(output_root / "run.json", report)
    return report, 0 if status == "pass" else 2 if status == "not-run" else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--authority", type=Path)
    mode.add_argument("--host-snapshot", action="store_true")
    args = parser.parse_args()
    try:
        if args.host_snapshot:
            print(json.dumps(host_snapshot(), sort_keys=True))
            return 0
        report, status = execute(args.authority)
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError):
        print(json.dumps({"status": "fail", "reason": "authority-or-runner-refused"}))
        return 1
    print(json.dumps({key: report[key] for key in ("status", "case_statuses", "binding_unchanged", "halt_reason")}))
    return status


if __name__ == "__main__":
    raise SystemExit(main())
