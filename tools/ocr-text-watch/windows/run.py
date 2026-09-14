#!/usr/bin/env python3
"""Prepare or execute exactly one privately owned Windows WGC OCR procedure."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import secrets
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
_GIT_PROGRAM = shutil.which("git")
GIT_EXECUTABLE = str(Path(_GIT_PROGRAM).resolve()) if _GIT_PROGRAM else None
PROFILE = "ocr-text-watch-windows-v1"
GRANT = "owned-fixture-wgc-real-cpu-ocr"
HUD = "3ca2f418f6fb083e49a679638017609e11fe825ef4a1e6f56dc0e572c74f75e6"
PNG = "10f0163cf298453e55922cc6104ee3066f59a328a55e9bac6c04f24d36c9e288"
ORACLE = "906f787178acea1e967e3a3989f1e7c50b54b3bb6a1ec41af3e80980128a63bf"
SOURCE_FILES = (
    "tools/ocr-text-watch/windows/run.py",
    "tools/ocr-text-watch/windows/fixture.cpp",
    "crates/mado-pilot/examples/ocr-text-watch-native-windows.rs",
    "docs/ocr-text-watch-windows.md",
    "tools/ocr-text-watch/replay_corpus.py",
    "fixtures/ocr/g-004/hud.png",
    "tools/native-release-profile/process_runner.py",
    "tools/native-release-profile/_windows_process.py",
    "Cargo.lock",
)


def digest(path: Path) -> str:
    if not path.is_file() or path.is_symlink():
        raise ValueError("identity-file-not-regular")
    value = hashlib.sha256()
    with path.open("rb") as stream:
        while data := stream.read(1024 * 1024):
            value.update(data)
    return value.hexdigest()


def bounded_text(path: Path, maximum: int = 1024 * 1024) -> str:
    with path.open("rb") as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("evidence-file-limit")
    return data.decode("utf-8", "strict")


def store(path: Path, value: dict) -> None:
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def runner():
    # Existing ordinary native-setup process ownership, not a frozen campaign controller.
    sys.path.insert(0, str(ROOT / "tools/native-release-profile"))
    from process_runner import run_process
    return run_process


def git(run_process, *args: str) -> str:
    if GIT_EXECUTABLE is None:
        raise ValueError("git-unavailable")
    result = run_process([GIT_EXECUTABLE, *args], cwd=ROOT, env=dict(os.environ),
                         timeout_seconds=10, output_limit_bytes=65536, cleanup_seconds=5)
    if (result["exit_code"] != 0 or result["timed_out"] or result["output_limited"]
            or not result["cleanup_ok"] or result["launch_error"]):
        raise ValueError("source-identity-command-failed")
    return result["stdout"].strip()


def identities(args, run_process) -> dict:
    import winreg

    model = Path(os.environ["MADO_PILOT_G004_MODEL_ROOT"]).resolve(strict=True)
    runtime = Path(os.environ["MADO_PILOT_ONNX_RUNTIME"]).resolve(strict=True)
    consumer = args.consumer.resolve(strict=True)
    fixture = args.fixture.resolve(strict=True)
    corpus = args.corpus.resolve(strict=True)
    if not model.is_dir():
        raise ValueError("model-root-not-directory")
    if git(run_process, "status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("execution-source-not-clean")
    model_files = []
    # A deliberately finite controlled package, not arbitrary directory ingestion.
    for path in model.rglob("*"):
        if path.is_symlink():
            raise ValueError("model-link-refused")
        if path.is_file():
            model_files.append(path)
            if len(model_files) > 128:
                raise ValueError("model-file-count-limit")
    if not model_files:
        raise ValueError("model-files-absent")
    paths = [consumer, fixture, runtime, corpus / "hud.bgra", corpus / "oracle.json", Path(GIT_EXECUTABLE)]
    paths.extend(ROOT / name for name in SOURCE_FILES)
    paths.extend(model_files)
    files = {str(path.resolve(strict=True)): digest(path) for path in sorted(set(paths))}
    if files[str((corpus / "hud.bgra").resolve())] != HUD:
        raise ValueError("hud-raw-identity-mismatch")
    if files[str((ROOT / "fixtures/ocr/g-004/hud.png").resolve())] != PNG:
        raise ValueError("hud-source-identity-mismatch")
    if files[str((corpus / "oracle.json").resolve())] != ORACLE:
        raise ValueError("fixed-oracle-document-mismatch")
    oracle = json.loads(bounded_text(corpus / "oracle.json"))
    if (oracle["source_sha256"] != PNG or oracle["files"]["hud.bgra"] != {
            "bytes": 2073600, "sha256": HUD}):
        raise ValueError("corpus-oracle-identity-mismatch")
    with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") as key:
        ubr = winreg.QueryValueEx(key, "UBR")[0]
    version = sys.getwindowsversion()
    return {
        "source_commit": git(run_process, "rev-parse", "HEAD"),
        "source_tree": git(run_process, "rev-parse", "HEAD^{tree}"),
        "git_executable": GIT_EXECUTABLE,
        "source_status": "clean",
        "host": {"computer": platform.node(), "system": list(platform.win32_ver()),
                 "build": version.build, "ubr": ubr, "architecture": platform.machine(),
                 "processor": platform.processor(),
                 "sdk": os.environ.get("WindowsSDKVersion"),
                 "msvc": os.environ.get("VCToolsVersion")},
        "consumer": str(consumer), "fixture": str(fixture), "corpus": str(corpus),
        "model_root": str(model), "runtime": str(runtime), "sha256": files,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--consumer", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--prepare", type=Path, help="new prospective identity file; launches no fixture/model")
    mode.add_argument("--authority", type=Path, help="separately reviewed explicit execution authority")
    parser.add_argument("--evidence", type=Path, help="new private execution evidence directory")
    args = parser.parse_args()
    record = {"profile": PROFILE, "execution": "not-run", "semantic": "not-run",
              "resource": "not-run", "cleanup": "not-run", "reason": None,
              "recorded_at": datetime.now(timezone.utc).isoformat()}
    output = None
    try:
        if args.authority:
            if args.evidence is None:
                raise ValueError("private-evidence-directory-required")
            destination = args.evidence.resolve()
            destination.mkdir(parents=True, exist_ok=False)
            output = destination
        if sys.platform != "win32" or platform.machine().lower() not in ("amd64", "x86_64"):
            raise ValueError("unsupported-host-windows-x64-required")
        run_process = runner()
        if args.authority:
            approval = json.loads(bounded_text(args.authority))
            record["authority"] = approval
            if not isinstance(approval, dict) or approval.get("authorization") != GRANT:
                raise ValueError("fresh-native-execution-authority-absent")
        current = identities(args, run_process)
        if args.prepare:
            store(args.prepare, {
                "schema_version": 1, "profile": PROFILE, "identities": current,
                "authorization": None, "authority_id": None, "host_id": None,
                "planning_revision": None, "build_commands": [],
                "display_scope": None,
                "limits": {"process_seconds": 300, "output_bytes": 1048576, "cleanup_seconds": 5},
                "note": "Preparation is not an execution grant; fill and review every authority field separately.",
            })
            print("ocr-text-watch-windows: execution=not-run reason=prepared-awaiting-authority")
            return 0
        record["authority"] = approval
        record["identities_before"] = current
        if (approval.get("schema_version") != 1 or approval.get("profile") != PROFILE
                or approval.get("authorization") != GRANT
                or approval.get("identities") != current
                or approval.get("limits") != {"process_seconds": 300, "output_bytes": 1048576, "cleanup_seconds": 5}):
            raise ValueError("execution-authority-or-identity-mismatch")
        for field in ("authority_id", "host_id", "planning_revision", "display_scope"):
            if not isinstance(approval.get(field), str) or not approval[field].strip():
                raise ValueError("execution-authority-incomplete")
        if (not isinstance(approval.get("build_commands"), list) or not approval["build_commands"]
                or not all(isinstance(command, str) and command.strip() for command in approval["build_commands"])):
            raise ValueError("build-command-evidence-absent")
        nonce = secrets.token_hex(16)
        env = dict(os.environ)
        env["MADO_PILOT_OCR_WGC_RUN_NONCE"] = nonce
        env["MADO_PILOT_G004_MODEL_ROOT"] = current["model_root"]
        env["MADO_PILOT_ONNX_RUNTIME"] = current["runtime"]
        command = [current["consumer"], "--owned-wgc", current["fixture"],
                   str(Path(current["corpus"]) / "hud.bgra"), str(output / "consumer.txt"), nonce]
        record["command"] = command
        store(output / "before.json", record)
        observed = run_process(command, cwd=ROOT, env=env, timeout_seconds=300,
                               output_limit_bytes=1048576, cleanup_seconds=5)
        record["process"] = observed
        record["execution"] = "not-run" if observed["launch_error"] else "executed"
        if not observed["launch_error"]:
            record.update(semantic="failed", resource="unresolved", cleanup="failed")
        raw = bounded_text(output / "consumer.txt") if (output / "consumer.txt").exists() else ""
        record["consumer_raw"] = raw
        final = [line for line in raw.splitlines() if line.startswith("final ")]
        if len(final) == 1:
            facts = dict(field.split("=", 1) for field in final[0].split()[1:])
            if set(facts) != {"semantic", "resource", "cleanup"} or any(
                    value not in ("passed", "failed") for value in facts.values()):
                raise ValueError("consumer-summary-invalid")
            record.update(facts)
        elif not observed["launch_error"]:
            record.update(semantic="failed", resource="unresolved", cleanup="failed")
        record["phase_facts"] = [line for line in raw.splitlines() if line in (
            "transition=passed", "resize_reset=passed", "target_loss=passed",
            "producer_progress_while_result_retained=passed", "retained_after_parents=passed")]
        if not observed["cleanup_ok"] or observed["timed_out"] or observed["output_limited"]:
            record["cleanup"] = "failed"
        after = identities(args, run_process)
        record["identities_after"] = after
        stable = after == current
        record["identity_stable"] = stable
        passed = (observed["exit_code"] == 0 and not observed["launch_error"]
                  and not observed["timed_out"] and not observed["output_limited"]
                  and observed["cleanup_ok"] and stable
                  and all(record[field] == "passed" for field in ("semantic", "resource", "cleanup")))
        record["status"] = "passed" if passed else "failed"
        record["reason"] = None if passed else "mandatory-procedure-gate-failed"
        store(output / "evidence.json", record)
        print(f"ocr-text-watch-windows: semantic={record['semantic']} resource={record['resource']} cleanup={record['cleanup']} status={record['status']}")
        return 0 if passed else 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        # Details and actual identities stay only in the explicitly requested private file.
        record["reason"] = str(error)
        record["status"] = "failed" if record["execution"] == "executed" else "not-run"
        closed_reason = "authority-or-procedure-refused"
        try:
            if output is not None and not (output / "evidence.json").exists():
                store(output / "evidence.json", record)
        except OSError:
            closed_reason = "private-evidence-write-failed"
        print(f"ocr-text-watch-windows: execution={record['execution']} status={record['status']} reason={closed_reason}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
