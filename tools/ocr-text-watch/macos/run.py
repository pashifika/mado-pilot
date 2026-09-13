"""Explicitly authorized, one-shot private task 7.4 execution of the 7.3 procedure.

Preparation/inspection alone does not authorize invoking this file. No build,
permission request, focus, input, signing, download, retry, or process-name kill.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import secrets
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
_GIT_PROGRAM = shutil.which("git")
GIT_EXECUTABLE = str(Path(_GIT_PROGRAM).resolve()) if _GIT_PROGRAM else None
HUD_SHA256 = "10f0163cf298453e55922cc6104ee3066f59a328a55e9bac6c04f24d36c9e288"
ROWS = ("acknowledged-transition", "retained-producer-progress", "resize-reset",
        "resize-invalidity", "exact-window-loss", "parent-close-retention")
BOUNDS = {"construction_seconds": 45, "permission_and_launch_seconds": 65,
          "fixture_launch_seconds": 10, "control_seconds": 5,
          "semantic_wait_seconds": 15, "query_lifetime_seconds": 100,
          "session_close_seconds": 5, "physical_drain_seconds": 5,
          "consumer_seconds": 200, "fixture_self_fuse_seconds": 210,
          "graceful_exit_seconds": 5, "terminate_seconds": 2, "kill_reap_seconds": 2,
          "each_output_file_bytes": 65536, "total_output_bytes": 262144,
          "consumer_processes": 1, "fixture_processes": 1, "warmups": 0, "samples": 1}


def digest(path: Path) -> dict:
    path = path.resolve(strict=True)
    if not path.is_file():
        raise ValueError("prerequisite-not-file")
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": hasher.hexdigest()}


def validate_native_images(value) -> dict[str, str]:
    if not isinstance(value, dict) or not 1 <= len(value) <= 128:
        raise ValueError("bounded-native-image-manifest-required")
    approved = {}
    for name, expected in value.items():
        if not isinstance(name, str) or not isinstance(expected, str):
            raise ValueError("invalid-native-image-entry")
        path = Path(name)
        if not path.is_absolute() or name.startswith(("/System/Library/", "/usr/lib/")):
            raise ValueError("noncanonical-nonsystem-image-required")
        observed = digest(path)
        if observed["path"] != name or observed["sha256"] != expected:
            raise ValueError("native-image-authority-mismatch")
        approved[name] = expected
    return approved


def bounded_reader():
    # Existing generic native setup utility, not the frozen foreign controller.
    # Retain the path for the runner's lazy POSIX cleanup dependency too.
    sys.path.insert(0, str(ROOT / "tools/native-release-profile"))
    from process_runner import run_process
    return run_process


def child_limits():
    import resource
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_FSIZE, (65536, 65536))


def stop_owned(child: subprocess.Popen, graceful: float) -> dict:
    result = {"pid": child.pid, "forced": False, "reaped": False, "exit_code": None}
    try:
        child.wait(timeout=graceful)
    except subprocess.TimeoutExpired:
        result["forced"] = True
        # Popen owns this exact child; poll/wait serializes reaping. No PID lookup,
        # process-name search, process group, LaunchServices, or descendant sweep.
        child.terminate()
        try:
            child.wait(timeout=2)
        except subprocess.TimeoutExpired:
            child.kill()
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                return result
    result.update(reaped=True, exit_code=child.returncode)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--consumer", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--authority-file", type=Path, required=True)
    parser.add_argument("--build-record", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    try:
        args.evidence.mkdir(mode=0o700, parents=False, exist_ok=False)
    except OSError:
        print("semantic=not-run resource=not-run cleanup=not-run reason=new-private-directory-required")
        return 2
    evidence = args.evidence.resolve()
    record = {"procedure": "ocr-text-watch-apple-v1", "task": "7.4",
              "bounds": BOUNDS, "semantic": "not-run", "resource": "not-run",
              "cleanup": "not-run", "reason": "prerequisites",
              "rows": {name: "not-run" for name in ROWS}, "commands": [], "children": []}
    consumer = fixture = None
    files = []
    started = time.monotonic()
    try:
        if platform.system() != "Darwin" or platform.machine() != "arm64":
            record["reason"] = "unsupported-host"
            return 2
        run_process = bounded_reader()
        environment = dict(os.environ)

        def command(argv: list[str], limit: int = 65536) -> str:
            result = run_process(argv, cwd=ROOT, env=environment, timeout_seconds=10,
                                 output_limit_bytes=limit, cleanup_seconds=5)
            record["commands"].append({"argv": argv, **result})
            if (result["exit_code"] != 0 or result["timed_out"] or result["output_limited"]
                    or not result["cleanup_ok"] or result["launch_error"]):
                raise ValueError("provenance-command-failed")
            return result["stdout"].strip()

        host = {"uname": list(platform.uname()),
                "product_version": command(["/usr/bin/sw_vers", "-productVersion"]),
                "build": command(["/usr/bin/sw_vers", "-buildVersion"]),
                "cpu": command(["/usr/sbin/sysctl", "-n", "machdep.cpu.brand_string"]),
                "sdk": command(["/usr/bin/xcrun", "--show-sdk-version"])}
        machine_ids = re.findall(
            r'"IOPlatformUUID"\s*=\s*"([0-9a-fA-F-]{36})"',
            command(["/usr/sbin/ioreg", "-rd1", "-c", "IOPlatformExpertDevice"]),
        )
        if len(machine_ids) != 1:
            raise ValueError("exact-machine-identity-unavailable")
        host["machine_uuid"] = machine_ids[0].lower()
        record["host"] = host
        # A fresh approval pins this one host; no historical campaign host or
        # permission grant is inherited. Existing adapter availability still wins.
        record["authority"] = digest(args.authority_file)
        record["build_record"] = digest(args.build_record)
        if not 0 < record["authority"]["bytes"] <= 65536 or not record["build_record"]["bytes"]:
            raise ValueError("invalid-authority-or-build-record")
        authority = json.loads(args.authority_file.read_text())
        if not isinstance(authority, dict):
            raise ValueError("authority-not-object")
        required_host = {name: host[name] for name in ("machine_uuid", "uname", "product_version", "build", "cpu", "sdk")}
        if (authority.get("approved") is not True or authority.get("task") != "7.4"
                or authority.get("target") != "aarch64-apple-darwin"
                or authority.get("host") != required_host or authority.get("bounds") != BOUNDS):
            raise ValueError("fresh-host-authority-mismatch")
        if GIT_EXECUTABLE is None:
            raise ValueError("git-unavailable")
        record["git_executable"] = digest(Path(GIT_EXECUTABLE))
        record["source_status"] = command([GIT_EXECUTABLE, "status", "--porcelain", "--untracked-files=normal"])
        if record["source_status"]:
            raise ValueError("execution-source-not-clean")
        record["git_head"] = command([GIT_EXECUTABLE, "rev-parse", "HEAD"])
        record["git_tree"] = command([GIT_EXECUTABLE, "rev-parse", "HEAD^{tree}"])
        if (authority.get("source_head") != record["git_head"]
                or authority.get("source_tree") != record["git_tree"]):
            raise ValueError("source-authority-mismatch")
        model = Path(environment["MADO_PILOT_G004_MODEL_ROOT"])
        runtime = Path(environment["MADO_PILOT_ONNX_RUNTIME"])
        if not model.is_absolute() or not runtime.is_absolute():
            raise ValueError("nonabsolute-native-prerequisite")
        model = model.resolve(strict=True)
        runtime = runtime.resolve(strict=True)
        if not model.is_dir():
            raise ValueError("model-root-not-directory")
        environment["MADO_PILOT_G004_MODEL_ROOT"] = str(model)
        environment["MADO_PILOT_ONNX_RUNTIME"] = str(runtime)
        consumer_path, fixture_path = args.consumer.resolve(strict=True), args.fixture.resolve(strict=True)
        hud = ROOT / "fixtures/ocr/g-004/hud.png"
        bound_files = [consumer_path, fixture_path, hud, runtime,
                       model / "rapidocr-v3.9.2/ch_PP-OCRv4_det_mobile.onnx",
                       model / "rapidocr-v3.9.2/PP-OCRv6_rec_small.onnx",
                       ROOT / "Cargo.lock", ROOT / "rust-toolchain.toml",
                       ROOT / "crates/mado-pilot/examples/ocr-text-watch-native-macos.rs",
                       ROOT / "tools/ocr-text-watch/macos/fixture.m", Path(__file__),
                       ROOT / "docs/ocr-text-watch-macos.md",
                       ROOT / "tools/native-release-profile/process_runner.py"]
        record["files"] = [digest(path) for path in bound_files]
        if digest(hud)["sha256"] != HUD_SHA256:
            raise ValueError("immutable-fixture-mismatch")
        for name, index in (("consumer_sha256", 0), ("fixture_sha256", 1), ("runtime_sha256", 3),
                            ("detector_sha256", 4), ("recognizer_sha256", 5)):
            if authority.get(name) != record["files"][index]["sha256"]:
                raise ValueError("artifact-authority-mismatch")
        if authority.get("build_record_sha256") != record["build_record"]["sha256"]:
            raise ValueError("build-record-authority-mismatch")
        procedure_hashes = {str(Path(row["path"]).relative_to(ROOT)): row["sha256"]
                            for row in record["files"][6:]}
        if authority.get("procedure_sha256") != procedure_hashes:
            raise ValueError("procedure-source-authority-mismatch")
        approved_images = validate_native_images(authority.get("native_images"))
        required_images = {record["files"][index]["path"]: record["files"][index]["sha256"] for index in (0, 1, 3)}
        if any(approved_images.get(path) != expected for path, expected in required_images.items()):
            raise ValueError("required-native-image-not-approved")
        record["approved_native_images"] = approved_images
        # Validity is checked before signing metadata; identifiers stay private.
        for executable in (consumer_path, fixture_path):
            command(["/usr/bin/codesign", "--verify", "--strict", str(executable)])
            command(["/usr/bin/codesign", "-d", "--verbose=4", str(executable)])
            command(["/usr/bin/otool", "-L", str(executable)])
        nonce = secrets.token_hex(8)
        record["fixture_nonce"] = nonce
        environment["MADO_PILOT_OCR_PRIVATE_SUPERVISED"] = "1"
        environment["DYLD_PRINT_LIBRARIES"] = "1"
        record["loader_environment"] = {name: environment.get(name) for name in (
            "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH", "MADO_PILOT_ONNX_RUNTIME",
            "MADO_PILOT_G004_MODEL_ROOT")}

        def spawn(executable: Path, argv: list[str], name: str) -> subprocess.Popen:
            out = (evidence / f"{name}.stdout").open("xb")
            err = (evidence / f"{name}.stderr").open("xb")
            files.extend((out, err))
            row = [str(executable), *argv]
            record["commands"].append({"argv": row, "role": name})
            return subprocess.Popen(row, cwd=ROOT, env=environment, stdin=subprocess.DEVNULL,
                                    stdout=out, stderr=err, preexec_fn=child_limits)

        launch_start = time.monotonic()
        consumer = spawn(consumer_path, [str(evidence), nonce], "consumer")
        record["reason"] = "running"
        deadline = launch_start + BOUNDS["consumer_seconds"]
        while consumer.poll() is None:
            now = time.monotonic()
            if now >= deadline:
                record["reason"] = "consumer-timeout"
                break
            output_size = sum(path.stat().st_size for path in evidence.iterdir() if path.is_file())
            if output_size > BOUNDS["total_output_bytes"]:
                record["reason"] = "output-limit"
                break
            permission = evidence / "permission"
            if fixture is None and permission.exists():
                if permission.stat().st_size > 128:
                    raise ValueError("permission-output-limit")
                decision = permission.read_text().strip()
                if decision not in ("granted", "denied-or-undetermined", "unsupported", "undetermined"):
                    raise ValueError("invalid-permission-decision")
                record["permission"] = decision
                if decision != "granted":
                    record["reason"] = "permission-" + decision
                    break
                fixture = spawn(fixture_path, [str(evidence), nonce, str(hud)], "fixture")
                owner_path = evidence / "fixture-owner.tmp"
                owner_path.write_text(str(fixture.pid) + "\n")
                owner_path.replace(evidence / "fixture-owner")
            if fixture is None and now - launch_start >= BOUNDS["permission_and_launch_seconds"]:
                record["reason"] = "permission-or-launch-timeout"
                break
            if (evidence / "fixture-failure").exists():
                record["reason"] = "fixture-failed"
                break
            # Success exits after state 11; an explicit out-of-band stop is the
            # consumer's failure cleanup, not a second semantic experiment.
            if fixture is not None and fixture.poll() is not None:
                ack = evidence / "ack"
                words = ack.read_text().split() if ack.exists() and ack.stat().st_size <= 512 else []
                if (len(words) != 8 or words[:3] != [nonce, "11", "exit"]) and not (evidence / "stop").exists():
                    record["reason"] = "fixture-premature-exit"
                    break
            time.sleep(0.02)
        if consumer.poll() is not None and record["reason"] == "running":
            record["reason"] = "consumer-exited"
    except (OSError, ValueError, KeyError, KeyboardInterrupt) as error:
        record["reason"] = "interrupted" if isinstance(error, KeyboardInterrupt) else "prerequisite-or-control-failure"
        record["private_error"] = str(error)
    finally:
        # First ask only the owned fixture to stop. The consumer always owns its
        # public close path; termination is a failed cleanup endpoint, never a pass.
        if fixture is not None and fixture.poll() is None:
            try:
                (evidence / "stop").write_text("stop\n")
            except OSError:
                record["reason"] = "fixture-stop-write-failed"
        for child, name in ((consumer, "consumer"), (fixture, "fixture")):
            if child is not None:
                result = stop_owned(child, BOUNDS["graceful_exit_seconds"])
                record["children"].append({"role": name, **result})
        for stream in files:
            stream.close()
        permission_path = evidence / "permission"
        if permission_path.exists() and permission_path.stat().st_size <= 128:
            decision = permission_path.read_text(errors="replace").strip()
            if decision in ("granted", "denied-or-undetermined", "unsupported", "undetermined"):
                record["permission"] = decision
                if decision != "granted" and record["reason"] == "consumer-exited":
                    record["reason"] = "permission-" + decision
        error_path = evidence / "consumer.stderr"
        if error_path.exists() and error_path.stat().st_size <= 65536:
            errors = error_path.read_text(errors="replace").splitlines()
            if "consumer-failed reason=unsupported" in errors and record["reason"] == "consumer-exited":
                record["reason"] = "unsupported"
        report = evidence / "consumer.report"
        lines = report.read_text(errors="replace").splitlines() if report.exists() and report.stat().st_size <= 65536 else []
        for line in lines:
            parts = dict(word.split("=", 1) for word in line.split() if "=" in word)
            if parts.get("row") in record["rows"]:
                name = parts["row"]
                if record["rows"][name] != "failed":
                    record["rows"][name] = parts.get("status", "failed")
            if "semantic" in parts:
                record["semantic"] = parts["semantic"]
            if "resource" in parts:
                record["resource"] = parts["resource"]
            if "cleanup" in parts:
                record["consumer_cleanup"] = parts["cleanup"]
        all_reaped = bool(record["children"]) and all(child["reaped"] for child in record["children"])
        clean_exits = all_reaped and all(not child["forced"] and
            (child["exit_code"] in (0, 1) if child["role"] == "consumer" else child["exit_code"] == 0)
            for child in record["children"])
        if fixture is not None:
            record["cleanup"] = "passed" if (clean_exits and
                record.get("consumer_cleanup") == "consumer-endpoints-passed") else "failed"
            if record["semantic"] == "not-run" and fixture is not None:
                record["semantic"] = "failed"
            if record["resource"] == "not-run" and fixture is not None:
                record["resource"] = "unproven"
        # The loader's actual paths (not otool's install-name strings) bind every
        # developer-owned dynamic image. System shared-cache images bind to OS build.
        loaded = set()
        for name in ("consumer.stderr", "fixture.stderr"):
            path = evidence / name
            if path.exists() and path.stat().st_size <= 65536:
                for line in path.read_text(errors="replace").splitlines():
                    if line.startswith("dyld[") and "> /" in line:
                        loaded.add(line.split("> ", 1)[1].strip())
        record["loaded_images"] = []
        for value in sorted(loaded):
            path = Path(value)
            row = {"path": value, "system_cache": value.startswith(("/System/Library/", "/usr/lib/"))}
            if not row["system_cache"]:
                try:
                    row.update(digest(path))
                except (OSError, ValueError):
                    row["identity"] = "unresolved"
            record["loaded_images"].append(row)
        runtime_path = next((row["path"] for row in record.get("files", [])[3:4]), None)
        record["runtime_image_observed"] = runtime_path is not None and any(
            row["path"] == runtime_path for row in record["loaded_images"])
        observed_images = {row["path"]: row.get("sha256") for row in record["loaded_images"] if not row["system_cache"]}
        approved_images = record.get("approved_native_images")
        image_binding_matches = approved_images is not None and observed_images == approved_images
        if approved_images is not None:
            try:
                image_binding_matches = image_binding_matches and validate_native_images(approved_images) == approved_images
            except (OSError, ValueError):
                image_binding_matches = False
        record["native_image_binding_matches"] = image_binding_matches
        if fixture is not None and (not record["runtime_image_observed"] or not image_binding_matches):
            record["resource"] = "failed"
        record["duration_seconds"] = time.monotonic() - started
        record["passed"] = (record["semantic"] == record["resource"] == record["cleanup"] == "passed"
                            and all(value == "passed" for value in record["rows"].values())
                            and record["reason"] == "consumer-exited"
                            and consumer is not None and consumer.returncode == 0)
        (evidence / "record.json").write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n")
        print("semantic={semantic} resource={resource} cleanup={cleanup} reason={reason}".format(**record))
    return 0 if record["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
