#!/usr/bin/env python3
"""Observe one explicitly authorized real replay process; never approve its image set."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys

import measurements
import run_replay
import workloads

ROOT = run_replay.ROOT


def execute(args: argparse.Namespace) -> bool:
    if not args.execute_binding:
        raise ValueError("one fresh real-model binding execution must be explicitly authorized")
    if sys.platform == "win32" and sys.version_info < (3, 13):
        raise ValueError("private Windows evidence requires Python 3.13 or newer")
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {
        "schema_version": 1, "purpose": "real-cpu-replay-binding-only",
        "execution_state": "not-attempted", "observed": None, "measurements": None,
        "observed_native_images": None, "identity_unchanged": False,
        "binding_complete": False, "image_set_approved": False,
        "qualification_passed": False, "retries": 0, "failure": None,
    }
    stage = "preflight"
    try:
        executable = args.executable.resolve(strict=True)
        corpus = args.corpus.resolve(strict=True)
        source = workloads.source()
        environment = run_replay.canonical_environment()
        if environment.get("MADO_PILOT_ORT_PROFILE_DIR"):
            raise ValueError("binding must not inherit ORT profiling")
        inputs = run_replay.inputs(executable, corpus, environment)
        procedure = run_replay.identity(Path(__file__))
        target = run_replay.selected_target()
        host = workloads.observed_host()
        image_report = output / "native-images.txt"
        child = run_replay.dependency_environment(environment, image_report)
        required_images = {inputs[name]["path"]: inputs[name]["sha256"]
                           for name in ("executable", "runtime")}
        command = run_replay.observed_command([str(executable), str(corpus), "transition"])
        run_replay.write_record(output / "plan.json", {
            "schema_version": 1, "purpose": result["purpose"],
            "authority": "explicit --execute-binding; one real-model process only",
            "source": source, "procedure": procedure, "inputs": inputs,
            "target": target, "host": host, "command": command,
            "selection_environment": workloads.selection_environment(child),
            "required_images": required_images,
            "timeout_seconds": run_replay.TIMEOUT_SECONDS,
            "cleanup_seconds": run_replay.CLEANUP_SECONDS,
            "output_limit_bytes": run_replay.OUTPUT_LIMIT_BYTES,
            "native_image_report_limit_bytes": run_replay.MAX_DOCUMENT_BYTES,
            "retries": 0, "native_capture": False,
            "image_policy": "discovery only; extra images are hashed after execution, not preapproved or proven stable during this binding run",
        })
        stage = "before-process"
        if workloads.source() != source or run_replay.inputs(executable, corpus, environment) != inputs:
            raise ValueError("binding source or inputs changed before launch")
        result["execution_state"] = "attempted-unconfirmed"
        stage = "process-supervision"
        observed = run_replay.run_process(
            command, cwd=ROOT, env=child, timeout_seconds=run_replay.TIMEOUT_SECONDS,
            output_limit_bytes=run_replay.OUTPUT_LIMIT_BYTES,
            cleanup_seconds=run_replay.CLEANUP_SECONDS,
        )
        result["observed"] = observed
        result["execution_state"] = "started" if observed["launch_error"] is None else "not-started"
        stage = "process-result"
        if (observed["exit_code"] != 0 or observed["launch_error"] is not None
                or not observed["cleanup_ok"] or observed["timed_out"] or observed["output_limited"]):
            raise ValueError("binding process or owned cleanup failed")
        stage = "measurement-observation"
        result["measurements"] = measurements.real_measurements(observed["stdout"], target)
        if not result["measurements"]["complete"]:
            raise ValueError("binding semantic or measurement observations are incomplete")
        stage = "image-observation"
        images = run_replay.observe_dependencies(image_report, required_images)
        result["observed_native_images"] = images
        if "error_kind" in images or not images["observed"]:
            raise ValueError("binding image observation failed")
        if any(images["observed"].get(path) != digest for path, digest in required_images.items()):
            raise ValueError("required executable or runtime image differs")
        stage = "final-bindings"
        if (workloads.source() != source or workloads.observed_host() != host
                or run_replay.inputs(executable, corpus, environment) != inputs
                or run_replay.identity(Path(__file__)) != procedure):
            raise ValueError("binding source, host or inputs changed")
        result["identity_unchanged"] = True
        run_replay.write_record(output / "observed-native-images.json", images["observed"])
        result["binding_complete"] = True
    except (Exception, KeyboardInterrupt) as error:
        result["failure"] = {"stage": stage, "exception_type": type(error).__name__,
                             "reason": str(error)[:4096]}
    run_replay.write_record(output / "result.json", result)
    return result["binding_complete"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True,
                        help="exact public example built with ocr-text-watch-qualification")
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new private evidence directory")
    parser.add_argument("--execute-binding", action="store_true")
    args = parser.parse_args()
    if not args.execute_binding:
        parser.error("--execute-binding requires fresh authorization of one real-model process")
    try:
        complete = execute(args)
    except (OSError, ValueError):
        print("OCR binding preflight failed; no execution or approval is implied", file=sys.stderr)
        return 1
    print("OCR binding observed; image approval and qualification remain separate" if complete
          else "OCR binding failed; retain first output without retry")
    return 0 if complete else 1


if __name__ == "__main__":
    raise SystemExit(main())
