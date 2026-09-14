#!/usr/bin/env python3
"""Observe one explicitly authorized binding process; never approve its image set."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import sys

import measurements
import run_replay
import workloads

ROOT = run_replay.ROOT


def execute(args: argparse.Namespace) -> bool:
    if not args.execute_binding:
        raise ValueError("one fresh binding execution must be explicitly authorized")
    if args.mode not in ("real-cpu", "controlled"):
        raise ValueError("unknown binding mode")
    real = args.mode == "real-cpu"
    if real and args.corpus is None:
        raise ValueError("real CPU binding requires the fixed prepared corpus")
    if not real and args.corpus is not None:
        raise ValueError("controlled binding does not accept a corpus")
    if sys.platform == "win32" and sys.version_info < (3, 13):
        raise ValueError("private Windows evidence requires Python 3.13 or newer")
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {
        "schema_version": 2, "mode": args.mode,
        "purpose": "real-cpu-replay-binding-only" if real else "controlled-binding-only",
        "execution_state": "not-attempted", "observed": None, "measurements": None,
        "observed_native_images": None, "identity_unchanged": False,
        "binding_complete": False, "image_set_approved": False,
        "qualification_passed": False, "retries": 0, "failure": None,
    }
    stage = "preflight"
    try:
        executable = args.executable.resolve(strict=True)
        corpus = args.corpus.resolve(strict=True) if real else None
        source = workloads.source()
        environment = run_replay.canonical_environment() if real else dict(os.environ)
        if real and environment.get("MADO_PILOT_ORT_PROFILE_DIR"):
            raise ValueError("binding must not inherit ORT profiling")
        procedure = run_replay.identity(Path(__file__))
        target = run_replay.selected_target()
        host = workloads.observed_host()
        image_report = output / "native-images.txt"
        child = run_replay.dependency_environment(environment, image_report)
        selection = workloads.selection_environment(child)
        if real:
            inputs = run_replay.inputs(executable, corpus, environment)
            required_images = {inputs[name]["path"]: inputs[name]["sha256"]
                               for name in ("executable", "runtime")}
            timeout, cleanup, output_limit = (
                run_replay.TIMEOUT_SECONDS, run_replay.CLEANUP_SECONDS,
                run_replay.OUTPUT_LIMIT_BYTES,
            )
        else:
            selected, profile_identity = workloads.profile(
                ROOT / "docs/benchmarks" / f"ocr-text-watch-{target}.toml"
            )
            entry = run_replay.identity(executable)
            required_images = {entry["path"]: entry["sha256"]}
            inputs = {
                "profile": profile_identity,
                "artifacts": workloads.bindings(executable, None, environment, required_images),
            }
            facts = selected["profile"]
            timeout, cleanup, output_limit = (
                facts["process_timeout_seconds"], facts["cleanup_timeout_seconds"],
                facts["output_limit_bytes"],
            )
        command = run_replay.observed_command(
            [str(executable), str(corpus), "transition"] if real else [str(executable), "--semantic"]
        )

        def check_bindings() -> None:
            current = run_replay.inputs(executable, corpus, environment) if real else {
                "profile": run_replay.read_document(Path(inputs["profile"]["path"]))[1],
                "artifacts": workloads.bindings(executable, None, environment, required_images),
            }
            if (workloads.source() != source or workloads.observed_host() != host
                    or current != inputs or run_replay.identity(Path(__file__)) != procedure
                    or workloads.selection_environment(child) != selection):
                raise ValueError("binding source, host, inputs, procedure or selection changed")

        run_replay.write_record(output / "plan.json", {
            "schema_version": 2, "mode": args.mode, "purpose": result["purpose"],
            "authority": "explicit --execute-binding; one selected-mode process only",
            "source": source, "procedure": procedure, "inputs": inputs,
            "target": target, "host": host, "command": command,
            "selection_environment": selection,
            "required_images": required_images,
            "timeout_seconds": timeout,
            "cleanup_seconds": cleanup,
            "output_limit_bytes": output_limit,
            "native_image_report_limit_bytes": run_replay.MAX_DOCUMENT_BYTES,
            "retries": 0, "native_capture": False,
            "process_count": 1, "precursor_sample": False,
            "image_policy": "discovery only; extra images are hashed after execution, not preapproved or proven stable during this binding run",
        })
        stage = "before-process"
        check_bindings()
        result["execution_state"] = "attempted-unconfirmed"
        stage = "process-supervision"
        observed = run_replay.run_process(
            command, cwd=ROOT, env=child, timeout_seconds=timeout,
            output_limit_bytes=output_limit, cleanup_seconds=cleanup,
        )
        result["observed"] = observed
        result["execution_state"] = "started" if observed["launch_error"] is None else "not-started"
        stage = "process-result"
        if (observed["exit_code"] != 0 or observed["launch_error"] is not None
                or not observed["cleanup_ok"] or observed["timed_out"] or observed["output_limited"]):
            raise ValueError("binding process or owned cleanup failed")
        stage = "measurement-observation"
        measure = measurements.real_measurements if real else measurements.controlled_measurements
        result["measurements"] = measure(observed["stdout"], target)
        if (not result["measurements"]["complete"]
                or not real and not workloads.controlled_semantics(observed["stdout"])):
            raise ValueError("binding semantic or measurement observations are incomplete")
        stage = "image-observation"
        images = run_replay.observe_dependencies(image_report, required_images)
        result["observed_native_images"] = images
        if "error_kind" in images or not images["observed"]:
            raise ValueError("binding image observation failed")
        if any(images["observed"].get(path) != digest for path, digest in required_images.items()):
            raise ValueError("required executable or runtime image differs")
        stage = "final-bindings"
        check_bindings()
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
    parser.add_argument("--mode", choices=("real-cpu", "controlled"), required=True)
    parser.add_argument("--executable", type=Path, required=True,
                        help="exact qualification-enabled public example or controlled bench")
    parser.add_argument("--corpus", type=Path, help="required for real-cpu; forbidden for controlled")
    parser.add_argument("--output", type=Path, required=True, help="new private evidence directory")
    parser.add_argument("--execute-binding", action="store_true")
    args = parser.parse_args()
    if not args.execute_binding:
        parser.error("--execute-binding requires fresh authorization of one selected-mode process")
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
