#!/usr/bin/env python3
"""Run reviewed native-profile rows; preserve failures without selecting a profile."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import tempfile

from process_runner import run_process
from staging import _resolve_directory, snapshot_executable, stage_files

CANDIDATES = {
    "cpu-host-windows": ("x86_64-pc-windows-msvc", []),
    "cpu-bundled-windows": ("x86_64-pc-windows-msvc", []),
    "cpu-host-macos": ("aarch64-apple-darwin", []),
    "cpu-bundled-macos": ("aarch64-apple-darwin", []),
    "cuda-host-windows": ("x86_64-pc-windows-msvc", ["mado-pilot/cuda-provider", "mado-pilot-capi/cuda-provider"]),
}
LIMITS = {"manifest_bytes": 1048576, "stage_bytes": 4294967296,
          "output_bytes": 1048576, "row_seconds": 120, "cleanup_seconds": 5,
          "max_rows": 64, "max_files": 512}
IDENTIFIER = re.compile(r"[a-z0-9][a-z0-9-]{0,79}\Z")
DIGEST = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
TOKEN = re.compile(r"\{([a-z][a-z0-9_-]*)\}")
ENVIRONMENT_KEYS = {"PATH", "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH",
                    "MADO_PILOT_ONNX_RUNTIME", "MADO_PILOT_G004_MODEL_ROOT",
                    "MADO_PROFILE_LIBRARY"}


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1048576), b""):
            digest.update(chunk)
    return digest.hexdigest()


def exact(value: object, keys: set[str], label: str) -> dict:
    if not isinstance(value, dict) or set(value) != keys:
        raise ValueError(f"invalid {label} fields")
    return value


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def load_manifest(path: Path, expected_digest: str, candidate: str) -> tuple[dict, bytes]:
    if not DIGEST.fullmatch(expected_digest):
        raise ValueError("manifest digest must be lowercase SHA-256")
    with path.open("rb") as stream:
        data = stream.read(LIMITS["manifest_bytes"] + 1)
    if len(data) > LIMITS["manifest_bytes"] or hashlib.sha256(data).hexdigest() != expected_digest:
        raise ValueError("manifest size or digest mismatch")
    manifest = exact(json.loads(data, object_pairs_hook=unique_object), {"schema_version", "candidate", "source_commit", "source_tree",
                                      "features", "artifacts", "rows", "admission"}, "manifest")
    if type(manifest["schema_version"]) is not int or manifest["schema_version"] != 1:
        raise ValueError("unsupported manifest version")
    if manifest["candidate"] != candidate or manifest["features"] != CANDIDATES[candidate][1]:
        raise ValueError("candidate or feature selection mismatch")
    for field in ("source_commit", "source_tree"):
        if not isinstance(manifest[field], str) or not COMMIT.fullmatch(manifest[field]):
            raise ValueError("source requires full commit and tree identities")
    if not isinstance(manifest["artifacts"], list) or len(manifest["artifacts"]) > LIMITS["max_files"]:
        raise ValueError("invalid artifact count")
    admission = exact(manifest["admission"], {"kind", "record_sha256"}, "admission")
    if admission["kind"] not in ("development-host", "clean-consumer"):
        raise ValueError("unknown admission kind")
    if admission["kind"] == "clean-consumer":
        if not isinstance(admission["record_sha256"], str) or not DIGEST.fullmatch(admission["record_sha256"]):
            raise ValueError("clean admission needs separately reviewed record digest")
    elif admission["record_sha256"] is not None:
        raise ValueError("development admission cannot supply clean evidence")
    rows = manifest["rows"]
    if not isinstance(rows, list) or not 1 <= len(rows) <= LIMITS["max_rows"]:
        raise ValueError("invalid row count")
    seen = set()
    for row in rows:
        exact(row, {"id", "argv", "executable_sha256", "environment", "expected_exit",
                    "required_stdout", "unexecuted_reason"}, "row")
        if not isinstance(row["id"], str) or not IDENTIFIER.fullmatch(row["id"]) or row["id"] in seen:
            raise ValueError("invalid or duplicate row identity")
        seen.add(row["id"])
        reason = row["unexecuted_reason"]
        if reason is not None:
            if not isinstance(reason, str) or not IDENTIFIER.fullmatch(reason):
                raise ValueError("unexecuted reason must be a content-free token")
            if any(row[key] is not None for key in ("argv", "executable_sha256", "expected_exit", "required_stdout")) or row["environment"] != {}:
                raise ValueError("unexecuted rows cannot carry a runnable command")
            continue
        if not isinstance(row["argv"], list) or not row["argv"] or len(row["argv"]) > 64 or any(not isinstance(arg, str) or len(arg) > 4096 or "\0" in arg for arg in row["argv"]):
            raise ValueError("invalid command array")
        if not isinstance(row["executable_sha256"], str) or not DIGEST.fullmatch(row["executable_sha256"]):
            raise ValueError("command executable must be pinned")
        if not isinstance(row["environment"], dict) or set(row["environment"]) - ENVIRONMENT_KEYS:
            raise ValueError("unapproved environment key")
        if any(not isinstance(value, str) or len(value) > 4096 or "\0" in value for value in row["environment"].values()):
            raise ValueError("invalid environment value")
        if type(row["expected_exit"]) is not int or not 0 <= row["expected_exit"] <= 255:
            raise ValueError("expected exit must be an explicit ordinary exit code")
        if not isinstance(row["required_stdout"], str) or not row["required_stdout"] or len(row["required_stdout"]) > 256 or any(char in row["required_stdout"] for char in "\r\n\0"):
            raise ValueError("mandatory terminal observation required")
    return manifest, data


def expand(value: str, roots: dict[str, Path]) -> str:
    def replacement(match: re.Match) -> str:
        if match[1] not in roots:
            raise ValueError("unknown root token")
        return str(roots[match[1]])
    return TOKEN.sub(replacement, value)


def native_target() -> str:
    machine = platform.machine().lower()
    if sys.platform == "darwin" and machine in ("arm64", "aarch64"):
        return "aarch64-apple-darwin"
    if sys.platform == "win32" and machine in ("amd64", "x86_64"):
        return "x86_64-pc-windows-msvc"
    return "unsupported"


def write_bytes(path: Path, data: bytes) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(data)


def write_record(path: Path, value: dict) -> None:
    write_bytes(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8"))


def private_directory(path: Path) -> None:
    # Python 3.13+ applies a caller/administrator-only DACL to mode 0700 on Windows.
    if os.name == "nt" and sys.version_info < (3, 13):
        raise ValueError("private Windows evidence requires Python 3.13 or newer")
    path.mkdir(mode=0o700, parents=True)


def execute(manifest: dict, *, manifest_bytes: bytes, manifest_digest: str, attempt: str,
            roots: dict[str, Path], output: Path, admission_path: Path | None = None) -> tuple[dict, int]:
    if not IDENTIFIER.fullmatch(attempt):
        raise ValueError("invalid attempt identity")
    if set(roots) & {"stage", "scratch"}:
        raise ValueError("stage and scratch are reserved roots")
    output.mkdir(mode=0o700, parents=True, exist_ok=True)
    output = output.resolve(strict=True)
    candidate_output = output / manifest["candidate"]
    candidate_output.mkdir(mode=0o700, exist_ok=True)
    candidate_output = _resolve_directory(candidate_output, "candidate-output")
    attempt_path = candidate_output / attempt
    private_directory(attempt_path)  # Exclusive reservation preserves earlier incomplete attempts too.
    write_bytes(attempt_path / "manifest.json", manifest_bytes)
    record = {"schema_version": 1, "candidate": manifest["candidate"], "attempt": attempt,
              "manifest_sha256": manifest_digest, "source_commit": manifest["source_commit"],
              "source_tree": manifest["source_tree"], "features": manifest["features"],
              "procedure_sha256": {file.name: digest_file(file) for file in sorted(Path(__file__).parent.glob("*.py"))},
              "target": native_target(), "admission": manifest["admission"],
              "inventory": None, "rows": [], "cleanup_ok": False,
              "procedure_status": "failed", "qualification": "not-selected",
              "failure": None}
    scratch = None
    completed = False
    interruption = None
    try:
        if native_target() != CANDIDATES[manifest["candidate"]][0]:
            raise ValueError("target-mismatch")
        if manifest["admission"]["kind"] == "clean-consumer":
            if admission_path is None or digest_file(admission_path) != manifest["admission"]["record_sha256"]:
                raise ValueError("admission-record-missing-or-mismatched")
        scratch = Path(tempfile.mkdtemp(prefix="scratch-", dir=attempt_path))
        stage = scratch / "stage"
        stage.mkdir()
        record["inventory"] = stage_files(manifest["artifacts"], roots, stage, max_bytes=LIMITS["stage_bytes"])
        expanded_roots = dict(roots, stage=stage, scratch=scratch)
        snapshot_bytes = sum(row["bytes"] for row in manifest["artifacts"])
        for row in manifest["rows"]:
            observed = {"id": row["id"], "status": "unexecuted", "command": row["argv"],
                        "environment": row["environment"], "expected_exit": row["expected_exit"],
                        "required_stdout": row["required_stdout"], "reason": row["unexecuted_reason"]}
            record["rows"].append(observed)
            if row["unexecuted_reason"] is not None:
                continue
            try:
                argv = [expand(arg, expanded_roots) for arg in row["argv"]]
                expanded_environment = {key: expand(value, expanded_roots) for key, value in row["environment"].items()}
            except ValueError:
                observed.update(status="failed", reason="row-binding-invalid")
                continue
            executable = Path(argv[0])
            try:
                if not executable.is_absolute():
                    raise ValueError("executable must be absolute")
                if executable.resolve(strict=True).is_relative_to(stage):
                    snapshot_executable(executable, None, row["executable_sha256"], max_bytes=LIMITS["stage_bytes"])
                    snapshot = executable
                else:
                    launch_root = scratch / ("executable-" + row["id"])
                    private_directory(launch_root)
                    snapshot = launch_root / executable.name
                    snapshot_bytes += snapshot_executable(executable, snapshot, row["executable_sha256"],
                                                          max_bytes=LIMITS["stage_bytes"] - snapshot_bytes)
            except (OSError, ValueError):
                observed.update(status="failed", reason="executable-identity-mismatch")
                continue
            argv[0] = str(snapshot)
            observed["executed_sha256"] = row["executable_sha256"]
            environment = {key: os.environ[key] for key in ("SystemRoot", "WINDIR") if key in os.environ}
            environment.update({"HOME": str(scratch), "TMPDIR": str(scratch), "TMP": str(scratch), "TEMP": str(scratch)})
            environment.update(expanded_environment)
            result = run_process(argv, cwd=stage, env=environment,
                                 timeout_seconds=LIMITS["row_seconds"],
                                 output_limit_bytes=LIMITS["output_bytes"],
                                 cleanup_seconds=LIMITS["cleanup_seconds"])
            stdout = result.pop("stdout")
            stderr = result.pop("stderr")
            # Raw bounded logs are private attempt files, never public evidence by default.
            for label, text in (("stdout", stdout), ("stderr", stderr)):
                log_path = attempt_path / f"{row['id']}.{label}.log"
                write_bytes(log_path, text.encode("utf-8"))
            marker_seen = stdout.splitlines().count(row["required_stdout"]) == 1
            passed = (result["exit_code"] == row["expected_exit"] and marker_seen
                      and not result["timed_out"] and not result["output_limited"]
                      and result["cleanup_ok"] and result["launch_error"] is None)
            observed.update(status="passed" if passed else "failed", process=result,
                            mandatory_output_seen=marker_seen)
            if not result["cleanup_ok"]:
                raise ValueError("owned-process-cleanup-failed")
        completed = True
    except (OSError, ValueError) as error:
        record["failure"] = type(error).__name__
        write_bytes(attempt_path / "failure.log", str(error).encode("utf-8"))
    except BaseException as error:
        interruption = error
        record["failure"] = "attempt-interrupted"
        for observed in record["rows"]:
            if observed["status"] == "unexecuted" and observed["reason"] is None:
                observed.update(status="failed", reason="attempt-interrupted")
    finally:
        try:
            if scratch is not None:
                shutil.rmtree(scratch)
            record["cleanup_ok"] = True
        except BaseException as error:
            record["failure"] = "scratch-cleanup-failed"
            write_bytes(attempt_path / "cleanup.log", type(error).__name__.encode("utf-8"))
            if not isinstance(error, OSError):
                interruption = error
        seen = {row["id"] for row in record["rows"]}
        for row in manifest["rows"]:
            if row["id"] not in seen:
                record["rows"].append({"id": row["id"], "status": "unexecuted", "reason": "attempt-interrupted"})
        if completed and record["failure"] is None and record["cleanup_ok"]:
            statuses = {row["status"] for row in record["rows"]}
            record["procedure_status"] = "passed" if statuses == {"passed"} else "failed" if "failed" in statuses else "unexecuted"
        write_record(attempt_path / "result.json", record)
    if interruption is not None:
        raise interruption
    return record, 0 if record["procedure_status"] == "passed" else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True, choices=CANDIDATES)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--attempt", required=True)
    parser.add_argument("--root", action="append", default=[], metavar="ALIAS=PATH")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--admission", type=Path)
    args = parser.parse_args()
    try:
        roots = {}
        for binding in args.root:
            alias, separator, value = binding.partition("=")
            if not separator or not re.fullmatch(r"[a-z][a-z0-9_-]*", alias) or alias in roots:
                raise ValueError("invalid or duplicate root alias")
            roots[alias] = Path(value).absolute()
            if not roots[alias].is_dir():
                raise ValueError("root must be a directory")
        manifest, manifest_bytes = load_manifest(args.manifest, args.manifest_sha256, args.candidate)
        record, code = execute(manifest, manifest_bytes=manifest_bytes, manifest_digest=args.manifest_sha256,
                               attempt=args.attempt, roots=roots, output=args.output, admission_path=args.admission)
        print(json.dumps({key: record[key] for key in ("candidate", "attempt", "procedure_status", "qualification")}))
        return code
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"profile procedure refused: {type(error).__name__}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
