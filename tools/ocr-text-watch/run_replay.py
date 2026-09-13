#!/usr/bin/env python3
"""Run the fixed real-CPU replay cohort once after explicit execution authorization."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/native-release-profile"))
from process_runner import run_process

_GIT_PROGRAM = shutil.which("git")
GIT_EXECUTABLE = str(Path(_GIT_PROGRAM).resolve()) if _GIT_PROGRAM else None
MAX_IDENTITY_BYTES = 1 << 30
MAX_DOCUMENT_BYTES = 1 << 20
MAX_NATIVE_IMAGES = 256
LOADER_VARIABLES = (
    "PATH", "PATHEXT", "SystemRoot", "WINDIR", "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH", "MADO_PILOT_G004_MODEL_ROOT", "MADO_PILOT_ONNX_RUNTIME",
)

SCENARIOS = ("transition", "negative", "retina")
TIMEOUT_SECONDS = 90
CLEANUP_SECONDS = 10
OUTPUT_LIMIT_BYTES = 65536
PIXELS = {
    "hud.bgra": "3ca2f418f6fb083e49a679638017609e11fe825ef4a1e6f56dc0e572c74f75e6",
    "blank.bgra": "c216e1ed5c0b833b02a2e42bc95e2ccd6afe291b9e71b9ad69bb9bd8a3228d1f",
}
DOCUMENTS = {
    "madopilot-replay.json": "a589997024b9d8cecb72147dc9b053c19e93a7ab4431585a5aaf4812ee761c24",
    "oracle.json": "906f787178acea1e967e3a3989f1e7c50b54b3bb6a1ec41af3e80980128a63bf",
}
MODELS = {
    "rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx": "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9",
    "rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx": "6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884",
}


def _read_regular(path: Path, maximum_bytes: int, collect: bool) -> tuple[bytes | None, dict]:
    canonical = path.resolve(strict=True)
    before = canonical.stat()
    if not stat.S_ISREG(before.st_mode) or not 0 <= before.st_size <= maximum_bytes:
        raise ValueError("bounded regular identity file required")
    signature = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
    digest = hashlib.sha256()
    data = bytearray() if collect else None
    with open(canonical, "rb", opener=lambda name, flags: os.open(name, flags | getattr(os, "O_NONBLOCK", 0))) as stream:
        opened = os.fstat(stream.fileno())
        if not stat.S_ISREG(opened.st_mode) or signature(before) != signature(opened):
            raise ValueError("identity file changed before read")
        remaining = opened.st_size
        while remaining:
            chunk = stream.read(min(1 << 20, remaining))
            if not chunk:
                raise ValueError("identity file shortened")
            remaining -= len(chunk)
            digest.update(chunk)
            if data is not None:
                data.extend(chunk)
        if stream.read(1) or signature(opened) != signature(os.fstat(stream.fileno())):
            raise ValueError("identity file changed during read")
    if signature(opened) != signature(canonical.stat()):
        raise ValueError("identity path changed during read")
    return (bytes(data) if data is not None else None), {
        "path": str(canonical), "bytes": opened.st_size, "sha256": digest.hexdigest(),
    }


def identity(path: Path) -> dict:
    return _read_regular(path, MAX_IDENTITY_BYTES, False)[1]


def read_document(path: Path, maximum_bytes: int = MAX_DOCUMENT_BYTES) -> tuple[bytes, dict]:
    data, observed = _read_regular(path, maximum_bytes, True)
    return data, observed


def _unique_pairs(pairs):
    value = {}
    for name, item in pairs:
        if name in value:
            raise ValueError("duplicate manifest entry")
        value[name] = item
    return value


def verify_native_manifest(approved_map: dict[str, str]) -> dict[str, str]:
    if not isinstance(approved_map, dict) or not 1 <= len(approved_map) <= MAX_NATIVE_IMAGES:
        raise ValueError("bounded native image manifest required")
    for name, expected in approved_map.items():
        if not isinstance(name, str) or not isinstance(expected, str) or "\n" in name or "\r" in name:
            raise ValueError("invalid native image entry")
        if not Path(name).is_absolute():
            raise ValueError("canonical absolute native image path required")
        if platform.system() == "Darwin" and name.startswith(("/System/Library/", "/usr/lib/")):
            raise ValueError("system shared-cache images bind to the OS, not this manifest")
        actual = identity(Path(name))
        if actual["path"] != name or actual["sha256"] != expected:
            raise ValueError("native image identity mismatch")
    return dict(approved_map)


def native_manifest(path: Path) -> tuple[dict[str, str], dict]:
    data, observed = read_document(path)
    approved = json.loads(data, object_pairs_hook=_unique_pairs)
    return verify_native_manifest(approved), observed


def dependency_environment(base_env: dict[str, str], report_path: Path) -> dict[str, str]:
    if any(base_env.get(name) for name in ("DYLD_INSERT_LIBRARIES", "LD_PRELOAD", "LD_AUDIT")):
        raise ValueError("injected native libraries are outside this procedure")
    child = dict(base_env)
    if platform.system() == "Windows":
        child["MADO_PILOT_OCR_DEPENDENCY_REPORT"] = str(report_path.resolve())
    elif platform.system() == "Darwin":
        child["DYLD_PRINT_LIBRARIES"] = "1"
    else:
        raise ValueError("native dependency observation is unavailable on this target")
    return child


def observe_dependencies(process_record: dict, report_path: Path, approved_map: dict[str, str]) -> dict:
    try:
        if platform.system() == "Windows":
            data, _ = read_document(report_path)
            if not data.endswith(b"\n"):
                raise ValueError("incomplete native image report")
            names = data.decode("utf-8", "strict").splitlines()
        else:
            names = [line.split("> ", 1)[1].strip() for line in process_record["stderr"].splitlines()
                     if line.startswith("dyld[") and "> /" in line]
        if not names or len(set(names)) > MAX_NATIVE_IMAGES:
            raise ValueError("missing or oversized native image observation")
        observed = {}
        for name in sorted(set(names)):
            if platform.system() == "Darwin" and name.startswith(("/System/Library/", "/usr/lib/")):
                continue
            entry = identity(Path(name))
            observed[entry["path"]] = entry["sha256"]
        unchanged = verify_native_manifest(approved_map)
        return {"matched": observed == unchanged, "observed": observed}
    except (OSError, ValueError, KeyError) as error:
        return {"matched": False, "observed": {}, "error_kind": type(error).__name__}


def git(*args: str) -> str:
    if GIT_EXECUTABLE is None:
        raise ValueError("git unavailable")
    if any(name in os.environ for name in (
        "GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    )):
        raise ValueError("ambient Git repository selection is refused")
    observed = run_process(
        [GIT_EXECUTABLE, *args], cwd=ROOT, env=dict(os.environ), timeout_seconds=10,
        output_limit_bytes=65536, cleanup_seconds=5,
    )
    if observed["exit_code"] != 0 or observed["timed_out"] or observed["output_limited"] or not observed["cleanup_ok"]:
        raise ValueError("bounded source identity command failed")
    return observed["stdout"].strip()


def selected_target() -> str:
    pair = platform.system(), platform.machine().lower()
    if pair == ("Darwin", "arm64"):
        return "aarch64-apple-darwin"
    if pair[0] == "Windows" and pair[1] in ("amd64", "x86_64"):
        return "x86_64-pc-windows-msvc"
    raise ValueError("host is outside the two release targets")


def canonical_environment() -> dict[str, str]:
    environment = dict(os.environ)
    for variable in ("MADO_PILOT_G004_MODEL_ROOT", "MADO_PILOT_ONNX_RUNTIME"):
        environment[variable] = str(Path(environment[variable]).resolve(strict=True))
    return environment


def inputs(executable: Path, corpus: Path, environment: dict[str, str]) -> dict:
    root = Path(environment["MADO_PILOT_G004_MODEL_ROOT"])
    files = {"executable": identity(executable), "runtime": identity(Path(environment["MADO_PILOT_ONNX_RUNTIME"]))}
    for name, expected in MODELS.items():
        entry = identity(root / name)
        if entry["sha256"] != expected:
            raise ValueError("controlled model identity differs")
        files[name] = entry
    for name, expected in PIXELS.items():
        entry = identity(corpus / name)
        if entry["bytes"] != 2073600 or entry["sha256"] != expected:
            raise ValueError("fixed replay pixel identity differs")
        files[name] = entry
    for name, expected in DOCUMENTS.items():
        entry = identity(corpus / name)
        if entry["sha256"] != expected:
            raise ValueError("fixed replay document identity differs")
        files[name] = entry
    files["procedure"] = identity(Path(__file__))
    files["supervisor"] = identity(ROOT / "tools/native-release-profile/process_runner.py")
    return files


def write_record(path: Path, value: dict) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def execute(executable: Path, corpus: Path, output: Path, native_images: Path) -> bool:
    executable, corpus = executable.resolve(strict=True), corpus.resolve(strict=True)
    target = selected_target()
    if Path(git("rev-parse", "--show-toplevel")).resolve() != ROOT or git("status", "--porcelain"):
        raise ValueError("the candidate source must be committed and clean")
    source = {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}")}
    environment = canonical_environment()
    approved, manifest_identity = native_manifest(native_images)
    before = inputs(executable, corpus, environment)
    for key in ("executable", "runtime"):
        if approved.get(before[key]["path"]) != before[key]["sha256"]:
            raise ValueError("required native image is not approved")
    # Validate selection before any cohort child can run.
    dependency_environment(environment, output / "preflight.images")
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    write_record(output / "plan.json", {
        "schema_version": 2, "lane": "real-cpu-replay", "source": source, "target": target,
        "authority": "explicit --execute after approval of the exact prospective inputs",
        "host": {"uname": list(platform.uname()), "cpu_count": os.cpu_count()},
        "inputs": before, "native_manifest": manifest_identity, "approved_native_images": approved,
        "loader_environment": {name: environment.get(name) for name in LOADER_VARIABLES},
        "profile": "phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v2",
        "scenarios": SCENARIOS, "processes": 3, "warmups": 0, "retries": 0,
        "timeout_seconds_per_process": TIMEOUT_SECONDS, "cleanup_seconds_per_process": CLEANUP_SECONDS,
        "output_limit_bytes_per_process": OUTPUT_LIMIT_BYTES,
        "scope": "Public Rust CPU replay only; no native capture or workload budget qualification",
        "private_output": "Contains controlled paths and complete process output; review before publication",
    })
    results = []
    apparatus_error = None
    for scenario in SCENARIOS:
        try:
            if (before != inputs(executable, corpus, environment)
                    or identity(native_images) != manifest_identity
                    or verify_native_manifest(approved) != approved
                    or source != {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}")}
                    or git("status", "--porcelain")):
                raise ValueError("cohort identity changed")
            report_path = output / f"{scenario}.images"
            argv = [str(executable), str(corpus), scenario]
            observed = run_process(
                argv, cwd=ROOT, env=dependency_environment(environment, report_path),
                timeout_seconds=TIMEOUT_SECONDS, output_limit_bytes=OUTPUT_LIMIT_BYTES,
                cleanup_seconds=CLEANUP_SECONDS,
            )
            dependencies = observe_dependencies(observed, report_path, approved)
            marker = f"ocr-text-watch: scenario={scenario} terminal="
            semantic = observed["exit_code"] == 0 and marker in observed["stdout"]
            passed = (semantic and dependencies["matched"] and observed["cleanup_ok"]
                      and not observed["timed_out"] and not observed["output_limited"])
            row = {"scenario": scenario, "argv": argv, "semantic_passed": semantic,
                   "passed": passed, "dependencies": dependencies, "observed": observed}
            write_record(output / f"{scenario}.json", row)
            results.append(row)
            if not passed:
                break
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            apparatus_error = {"scenario": scenario, "stage": "preflight-or-observation", "error_kind": type(error).__name__}
            write_record(output / "apparatus-invalid.json", apparatus_error)
            break
    unchanged = False
    try:
        unchanged = (before == inputs(executable, corpus, environment)
                     and manifest_identity == identity(native_images)
                     and approved == verify_native_manifest(approved)
                     and source == {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}")}
                     and not git("status", "--porcelain"))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        if apparatus_error is None:
            apparatus_error = {"stage": "final-identity", "error_kind": type(error).__name__}
            write_record(output / "apparatus-invalid.json", apparatus_error)
    passed = unchanged and apparatus_error is None and len(results) == len(SCENARIOS) and all(row["passed"] for row in results)
    write_record(output / "result.json", {
        "schema_version": 2, "target": target, "source": source,
        "identity_unchanged": unchanged, "passed": passed, "apparatus_error": apparatus_error,
        "executed": [row["scenario"] for row in results], "not_run": list(SCENARIOS[len(results):]),
        "native_capture": "not-run", "workload_budgets": "not-qualified",
    })
    return passed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="new private evidence directory")
    parser.add_argument("--native-images", required=True, type=Path, help="prospectively approved canonical native image manifest")
    parser.add_argument("--execute", action="store_true", help="explicitly execute exactly this fixed real-model cohort")
    args = parser.parse_args()
    if not args.execute:
        parser.error("--execute is required; request fresh real-model execution authority first")
    if sys.version_info < (3, 13):
        parser.error("Python 3.13+ is required for private evidence directories")
    try:
        passed = execute(args.executable, args.corpus, args.output, args.native_images)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        print("OCR replay procedure failed; inspect retained private evidence", file=sys.stderr)
        return 1
    print("OCR replay cohort: passed" if passed else "OCR replay cohort: failed; later rows were not run")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
