"""Explicitly authorized, one-shot private task 7.4 execution of the 7.3 procedure.

Preparation/inspection alone does not authorize invoking this file. No build,
permission request, focus, input, signing, download, retry, or process-name kill.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import platform
import re
import secrets
import select
import signal
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
          "each_native_image_file_bytes": 1048576, "total_native_image_bytes": 2097152,
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
    sys.path.insert(0, str(ROOT / "tools/native-release-profile"))
    from process_runner import run_process
    return run_process


def child_limits():
    import resource
    # Pipe sinks own evidence budgets; unrelated writes inherit RLIMIT_FSIZE.
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


class NativeCapture:
    """Private, direct-child capture; every pump reads at most 64KiB per pipe.

    Writers stay blocking, so dyld cannot silently lose records to EAGAIN. Only
    the parent read ends are nonblocking. Failed sinks continue to be drained,
    with discarded bytes and the first failure made explicit.
    """

    def __init__(self, evidence: Path):
        self.evidence = evidence
        self.children = {}
        self.channels = {}
        self.failure = None
        self.cleanup_results = None
        self._closed = False
        self._accept_interrupts = True

    def fail(self, reason: str, detail: str | None = None):
        if self.failure is None:
            self.failure = {"reason": reason}
            if detail is not None:
                self.failure["detail"] = detail

    def interrupt(self, signum, _frame=None):
        if self._accept_interrupts:
            self.fail("interrupted", signal.Signals(signum).name)

    def _channel_failure(self, channel: dict, reason: str, error=None):
        if channel["failure"] is None:
            channel["failure"] = reason
            if error is not None:
                channel["private_error"] = str(error)
        self.fail(reason, channel["name"])

    def spawn(self, executable: Path, argv: list[str], name: str, *,
              cwd: Path, env: dict[str, str]) -> subprocess.Popen:
        if self._closed or self.failure is not None:
            raise ValueError("native-capture-not-launchable")
        if name not in ("consumer", "fixture") or name in self.children:
            raise ValueError("native-child-role-already-owned-or-invalid")
        if any(key.startswith("DYLD_PRINT_") and value for key, value in env.items()):
            raise ValueError("ambient-dyld-diagnostics-refused")
        prepared = []
        # A failed file/pipe/Popen setup closes every endpoint, including a pipe
        # created before its stream wrapper could be constructed.
        try:
            with contextlib.ExitStack() as resources, contextlib.ExitStack() as writers:
                for kind in ("stdout", "stderr", "native-images"):
                    path = self.evidence / f"{name}.{kind}"
                    output = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                    resources.callback(os.close, output)
                    reader, writer = os.pipe()
                    resources.callback(os.close, reader)
                    writers.callback(os.close, writer)
                    os.set_blocking(reader, False)
                    channel = {"name": path.name, "role": name, "kind": kind,
                               "reader": reader, "output": output, "writer": writer,
                               "observed_bytes": 0, "retained_bytes": 0, "eof": False,
                               "saturated": False, "failure": None}
                    prepared.append(channel)
                environment = dict(env)
                image_writer = prepared[2]["writer"]
                environment["DYLD_PRINT_LIBRARIES"] = "1"
                environment["DYLD_PRINT_TO_FILE"] = f"/dev/fd/{image_writer}"
                child = subprocess.Popen(
                    [str(executable), *argv], cwd=cwd, env=environment,
                    stdin=subprocess.DEVNULL, stdout=prepared[0]["writer"],
                    stderr=prepared[1]["writer"], pass_fds=(image_writer,),
                    preexec_fn=child_limits,
                )
                # Register the actual executable PID before any post-launch
                # operation can fail. Cleanup never looks up a process by name.
                self.children[name] = child
                self.channels.update((row["name"], row) for row in prepared)
                resources.pop_all()
                return child
        except (Exception, KeyboardInterrupt) as error:
            self.fail("native-child-launch-failed", str(error))
            raise

    def ordinary_bytes(self) -> int:
        excluded = {self.evidence / f"{name}.native-images" for name in ("consumer", "fixture")}
        total = 0
        for path in self.evidence.iterdir():
            if path not in excluded:
                try:
                    if path.is_file():
                        total += path.stat().st_size
                except FileNotFoundError:
                    # The owned atomic control-file publisher can rename a
                    # temporary between directory enumeration and stat.
                    continue
        return total

    def check_output(self) -> int:
        total = self.ordinary_bytes()
        if total > BOUNDS["total_output_bytes"]:
            self.fail("output-limit", "ordinary/control/report aggregate")
        return total

    def poll(self, name: str) -> int | None:
        child = self.children[name]
        code = child.poll()
        if code is not None and code != 0:
            self.fail(f"{name}-exited", f"exit_code={code}")
        return code

    def _close_descriptor(self, channel: dict, field: str):
        descriptor = channel[field]
        channel[field] = None
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError as error:
                self._channel_failure(channel, "channel-close-failed", error)

    def pump(self, timeout: float = 0.02):
        if self._closed:
            return
        image_bytes = sum(row["retained_bytes"] for row in self.channels.values()
                          if row["kind"] == "native-images")
        readers = [row["reader"] for row in self.channels.values() if row["reader"] is not None]
        timeout = min(0.02, max(0.0, timeout))
        if readers:
            try:
                ready, _, _ = select.select(readers, [], [], timeout)
            except (OSError, ValueError) as error:
                self.fail("channel-pump-failed", str(error))
                # All reads are nonblocking. A failed readiness notification
                # must not prevent draining or exact-child reaping.
                ready = readers
                time.sleep(timeout)
        else:
            ready = []
            time.sleep(timeout)
        try:
            # Recount after select: a child may have published a control/report
            # file before its ordinary pipe became readable.
            ordinary = self.check_output()
        except OSError as error:
            self.fail("channel-pump-failed", str(error))
            ordinary = BOUNDS["total_output_bytes"]
        for row in self.channels.values():
            if row["reader"] not in ready:
                continue
            try:
                chunk = os.read(row["reader"], 65536)
            except BlockingIOError:
                continue
            except OSError as error:
                self._channel_failure(row, "channel-read-failed", error)
                self._close_descriptor(row, "reader")
                continue
            if not chunk:
                row["eof"] = True
                self._close_descriptor(row, "reader")
                continue
            row["observed_bytes"] += len(chunk)
            image = row["kind"] == "native-images"
            limit = BOUNDS["each_native_image_file_bytes" if image else "each_output_file_bytes"]
            aggregate_room = (BOUNDS["total_native_image_bytes"] - image_bytes if image
                              else BOUNDS["total_output_bytes"] - ordinary)
            room = max(0, min(limit - row["retained_bytes"], aggregate_room))
            if row["failure"] is None and room:
                piece = chunk[:room]
                try:
                    written = os.write(row["output"], piece)
                    row["retained_bytes"] += written
                    if image:
                        image_bytes += written
                    else:
                        ordinary += written
                    if written != len(piece):
                        self._channel_failure(row, "channel-write-failed", "short write")
                except OSError as error:
                    self._channel_failure(row, "channel-write-failed", error)
            if row["observed_bytes"] >= limit:
                row["saturated"] = True
                self._channel_failure(row, "native-image-limit" if image else "output-limit")
            elif len(chunk) > room:
                self._channel_failure(row, "native-image-limit" if image else "output-limit")
        # Observe exits only after this pass has captured ready failure output.
        for name in self.children:
            self.poll(name)

    def _cleanup_pump(self, timeout: float):
        try:
            self.pump(timeout)
        except (Exception, KeyboardInterrupt) as error:
            self.fail("channel-pump-failed", str(error))
            # An unexpected pump failure cannot bypass the cleanup deadline or
            # exact-child polling/signalling.
            time.sleep(min(0.02, max(0.0, timeout)))

    def _wait_children(self, seconds: float):
        deadline = time.monotonic() + seconds
        while any(self.poll(name) is None for name in self.children):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            self._cleanup_pump(remaining)

    def cleanup(self, graceful: float | None = None) -> list[dict]:
        """Drain all roles together while stopping/reaping only their Popen PIDs."""
        if self.cleanup_results is not None:
            return self.cleanup_results
        rows = {name: {"role": name, "pid": child.pid, "forced": False,
                       "reaped": False, "exit_code": None, "errors": []}
                for name, child in self.children.items()}
        try:
            self._wait_children(BOUNDS["graceful_exit_seconds"] if graceful is None else graceful)
        except (Exception, KeyboardInterrupt) as error:
            self.fail("child-cleanup-failed", str(error))
        finally:
            for operation, bound in (("terminate", "terminate_seconds"), ("kill", "kill_reap_seconds")):
                for name, child in self.children.items():
                    try:
                        if self.poll(name) is None:
                            rows[name]["forced"] = True
                            self.fail(f"{name}-forced-cleanup", operation)
                            getattr(child, operation)()
                    except ProcessLookupError:
                        pass
                    except (Exception, KeyboardInterrupt) as error:
                        rows[name]["errors"].append(str(error))
                        self.fail("child-cleanup-failed", str(error))
                try:
                    self._wait_children(BOUNDS[bound])
                except (Exception, KeyboardInterrupt) as error:
                    self.fail("child-cleanup-failed", str(error))
            for name in self.children:
                try:
                    code = self.poll(name)
                    rows[name].update(reaped=code is not None, exit_code=code)
                    if code is None:
                        self.fail("child-reap-timeout", name)
                except (Exception, KeyboardInterrupt) as error:
                    rows[name]["errors"].append(str(error))
                    self.fail("child-cleanup-failed", str(error))
            try:
                # Poll is not EOF: an exited child may leave a full final pipe.
                deadline = time.monotonic() + BOUNDS["physical_drain_seconds"]
                while any(row["reader"] is not None for row in self.channels.values()):
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        break
                    self._cleanup_pump(remaining)
            finally:
                self.close()
                self.cleanup_results = list(rows.values())
        return self.cleanup_results

    def close(self):
        for row in self.channels.values():
            if not row["eof"]:
                self._channel_failure(row, "channel-incomplete")
            self._close_descriptor(row, "reader")
            self._close_descriptor(row, "output")
        self._closed = True

    def facts(self) -> dict:
        return {name: {key: row[key] for key in (
                    "observed_bytes", "retained_bytes", "eof", "saturated", "failure"
                )} | {"complete": row["eof"] and row["failure"] is None
                                  and row["observed_bytes"] == row["retained_bytes"]}
                | ({"private_error": row["private_error"]} if "private_error" in row else {})
                for name, row in self.channels.items()}

    def image_paths(self) -> set[str]:
        paths = set()
        for name, child in self.children.items():
            row = self.channels[f"{name}.native-images"]
            try:
                if not self._closed or not self.facts()[row["name"]]["complete"]:
                    raise ValueError("native-image-incomplete")
                path = self.evidence / row["name"]
                if path.stat().st_size != row["retained_bytes"]:
                    raise ValueError("native-image-size-changed")
                with path.open("rb") as stream:
                    data = stream.read(BOUNDS["each_native_image_file_bytes"])
                if (not data or len(data) != row["retained_bytes"]
                        or len(data) >= BOUNDS["each_native_image_file_bytes"] or not data.endswith(b"\n")):
                    raise ValueError("native-image-incomplete-or-saturated")
                for line in data.decode("utf-8", errors="strict").split("\n")[:-1]:
                    # dyld also reports a loaded-to-delayed state transition.
                    # It names no path/UUID and is not an image identity row.
                    delayed = re.fullmatch(
                        r"dyld\[([1-9][0-9]*)\]: move loaded to delayed: [^/\x00-\x1f\x7f]+",
                        line,
                    )
                    if delayed is not None and int(delayed[1]) == child.pid:
                        continue
                    match = re.fullmatch(
                        r"dyld\[([1-9][0-9]*)\]: <[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-"
                        r"[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}> (/[^\x00-\x1f\x7f]+)",
                        line,
                    )
                    if match is None or int(match[1]) != child.pid:
                        raise ValueError("native-image-malformed-record")
                    value = match[2]
                    path = Path(value)
                    if str(path) != value or ".." in path.parts:
                        raise ValueError("native-image-noncanonical-path")
                    paths.add(value)
            except (OSError, ValueError) as error:
                self._channel_failure(row, "native-image-observation-failed", error)
                raise
        return paths


def finalize_record(record: dict, capture: NativeCapture, started: float):
    """Interpret only fully drained evidence; a report cannot clear a prior failure."""
    evidence = capture.evidence
    consumer, fixture = (capture.children.get(name) for name in ("consumer", "fixture"))
    record["children"] = capture.cleanup_results or []
    if capture.failure is not None:
        record["reason"] = capture.failure["reason"]
    permission_path = evidence / "permission"
    if permission_path.exists() and permission_path.stat().st_size <= 128:
        decision = permission_path.read_text(errors="replace").strip()
        if decision in ("granted", "denied-or-undetermined", "unsupported", "undetermined"):
            record["permission"] = decision
            if (decision != "granted" and record["reason"] == "consumer-exited"
                    and consumer is not None and consumer.returncode == 1):
                record["reason"] = "permission-" + decision
    error_path = evidence / "consumer.stderr"
    if error_path.exists() and error_path.stat().st_size <= BOUNDS["each_output_file_bytes"]:
        errors = error_path.read_text(errors="replace").splitlines()
        if ("consumer-failed reason=unsupported" in errors and record["reason"] == "consumer-exited"
                and consumer is not None and consumer.returncode == 1):
            record["reason"] = "unsupported"
    report = evidence / "consumer.report"
    if report.exists() and report.stat().st_size > BOUNDS["each_output_file_bytes"]:
        capture.fail("output-limit", "consumer.report")
        lines = []
    else:
        lines = report.read_text(errors="replace").splitlines() if report.exists() else []
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
    clean_exits = all_reaped and all(not child["forced"] and not child["errors"] and
        (child["exit_code"] in (0, 1) if child["role"] == "consumer" else child["exit_code"] == 0)
        for child in record["children"])
    if fixture is not None:
        record["cleanup"] = "passed" if (clean_exits and
            all(row["complete"] for row in capture.facts().values()) and
            record.get("consumer_cleanup") == "consumer-endpoints-passed") else "failed"
        if record["semantic"] == "not-run":
            record["semantic"] = "failed"
        if record["resource"] == "not-run":
            record["resource"] = "unproven"
    # No stderr scraping, replacement decoding, basename allowance or observed
    # auto-approval. Shared-cache paths alone bind to the approved OS build.
    loaded = set()
    if capture.children:
        try:
            loaded = capture.image_paths()
        except (OSError, ValueError):
            pass  # image_paths records the failed channel and first failure.
    record["loaded_images"] = []
    for value in sorted(loaded):
        row = {"path": value, "system_cache": value.startswith(("/System/Library/", "/usr/lib/"))}
        if not row["system_cache"]:
            try:
                identity = digest(Path(value))
                if identity["path"] != value:
                    raise ValueError("noncanonical-observed-native-image")
                row.update(identity)
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
        capture.fail("native-image-binding-failed")
    if any(row["failure"] is not None for row in capture.channels.values()):
        record["resource"] = "failed"
    capture.check_output()
    record["duration_seconds"] = time.monotonic() - started
    update_capture_record(record, capture)
    record["passed"] = (record["semantic"] == record["resource"] == record["cleanup"] == "passed"
                        and all(value == "passed" for value in record["rows"].values())
                        and record["reason"] == "consumer-exited"
                        and consumer is not None and consumer.returncode == 0
                        and capture.failure is None)


def update_capture_record(record: dict, capture: NativeCapture):
    record["private_channels"] = capture.facts()
    record["private_first_failure"] = capture.failure
    if capture.failure is not None:
        # A normal consumer failure may be refined from its ordinary diagnostics;
        # a signal, timeout, channel failure or cleanup failure cannot be relabelled.
        if not (capture.failure["reason"] == "consumer-exited"
                and capture.children["consumer"].returncode == 1
                and (record["reason"] == "unsupported" or record["reason"].startswith("permission-"))):
            record["reason"] = capture.failure["reason"]
        record["passed"] = False


def write_record(record: dict, capture: NativeCapture) -> bool:
    """Publish the complete JSON only if it fits the ordinary aggregate too."""
    try:
        ordinary = capture.check_output()
        # Python signal handlers run on this same main thread. This assignment
        # closes interruption admission before the immutable verdict snapshot;
        # every earlier accepted signal is included by the following refresh.
        capture._accept_interrupts = False
        update_capture_record(record, capture)
        payload = (json.dumps(record, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
        if ordinary + len(payload) > BOUNDS["total_output_bytes"]:
            raise ValueError("record.json exceeds ordinary/control/report aggregate")
        with (capture.evidence / "record.json").open("xb") as stream:
            if stream.write(payload) != len(payload):
                raise OSError("short record write")
        return True
    except (OSError, ValueError) as error:
        capture.fail("record-output-failed", str(error))
        record["private_error"] = str(error)
        update_capture_record(record, capture)
        return False


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
    record = {"procedure": "ocr-text-watch-apple-v2", "task": "7.4",
              "bounds": BOUNDS, "semantic": "not-run", "resource": "not-run",
              "cleanup": "not-run", "reason": "prerequisites",
              "rows": {name: "not-run" for name in ROWS}, "commands": [], "children": []}
    consumer = fixture = None
    capture = NativeCapture(evidence)
    handlers = {number: signal.getsignal(number) for number in (signal.SIGTERM, signal.SIGINT)}
    started = time.monotonic()
    try:
        for number in handlers:
            signal.signal(number, capture.interrupt)
        if platform.system() != "Darwin" or platform.machine() != "arm64":
            record["reason"] = "unsupported-host"
            return 2
        run_process = bounded_reader()
        environment = dict(os.environ)

        def command(argv: list[str], limit: int = 65536) -> str:
            if capture.failure is not None:
                raise ValueError("native-run-stopped")
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
        record["loader_environment"] = {name: environment.get(name) for name in (
            "DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH", "MADO_PILOT_ONNX_RUNTIME",
            "MADO_PILOT_G004_MODEL_ROOT")}

        def spawn(executable: Path, argv: list[str], name: str) -> subprocess.Popen:
            if capture.failure is not None:
                raise ValueError("native-run-stopped")
            row = [str(executable), *argv]
            record["commands"].append({"argv": row, "role": name})
            return capture.spawn(executable, argv, name, cwd=ROOT, env=environment)

        launch_start = time.monotonic()
        consumer = spawn(consumer_path, [str(evidence), nonce], "consumer")
        record["reason"] = "running"
        deadline = launch_start + BOUNDS["consumer_seconds"]
        while True:
            capture.pump()
            if capture.failure is not None or capture.poll("consumer") is not None:
                break
            now = time.monotonic()
            if now >= deadline:
                capture.fail("consumer-timeout")
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
                    capture.fail("permission-" + decision)
                    break
                fixture = spawn(fixture_path, [str(evidence), nonce, str(hud)], "fixture")
                owner_path = evidence / "fixture-owner.tmp"
                owner_path.write_text(str(fixture.pid) + "\n")
                owner_path.replace(evidence / "fixture-owner")
            if fixture is None and now - launch_start >= BOUNDS["permission_and_launch_seconds"]:
                capture.fail("permission-or-launch-timeout")
                break
            if (evidence / "fixture-failure").exists():
                capture.fail("fixture-failed")
                break
            # Success exits after state 11; an explicit out-of-band stop is the
            # consumer's failure cleanup, not a second semantic experiment.
            if fixture is not None and capture.poll("fixture") is not None:
                ack = evidence / "ack"
                words = ack.read_text().split() if ack.exists() and ack.stat().st_size <= 512 else []
                if (len(words) != 8 or words[:3] != [nonce, "11", "exit"]) and not (evidence / "stop").exists():
                    capture.fail("fixture-premature-exit")
                    break
        if capture.poll("consumer") is not None and capture.failure is None:
            record["reason"] = "consumer-exited"
    except (Exception, KeyboardInterrupt) as error:
        capture.fail("interrupted" if isinstance(error, KeyboardInterrupt) else "prerequisite-or-control-failure", str(error))
        record["private_error"] = str(error)
    finally:
        try:
            # Even a launch that fails after Popen registration is still owned.
            fixture = capture.children.get("fixture")
            try:
                if fixture is not None and fixture.poll() is None:
                    (evidence / "stop").write_text("stop\n")
            except (Exception, KeyboardInterrupt) as error:
                capture.fail("fixture-stop-write-failed", str(error))
            finally:
                record["children"] = capture.cleanup()
            try:
                finalize_record(record, capture, started)
            except (Exception, KeyboardInterrupt) as error:
                capture.fail("evidence-finalization-failed", str(error))
                record["private_error"] = str(error)
                record["passed"] = False
                record["duration_seconds"] = time.monotonic() - started
                update_capture_record(record, capture)
            if not write_record(record, capture):
                print("record=not-written reason=bounded-record-output-failure", file=sys.stderr)
            print("semantic={semantic} resource={resource} cleanup={cleanup} reason={reason}".format(**record))
        finally:
            for number, handler in handlers.items():
                signal.signal(number, handler)
    return 0 if record["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
