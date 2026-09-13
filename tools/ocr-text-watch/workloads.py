#!/usr/bin/env python3
"""Run an explicitly selected OCR workload cohort; never enforce unaccepted budgets."""

from __future__ import annotations

import argparse
import json
import os
import platform
from pathlib import Path
import subprocess
import sys
import tomllib

import run_replay

ROOT = run_replay.ROOT
WORKLOADS = (
    "steady-nonmatch", "positive-consecutive", "slow-backend-saturation",
    "mixed-two-session", "retained-results", "cancellation-close", "cold-startup",
)
HARNESS = (
    "crates/mado-pilot/benches/ocr-text-watch-query.rs",
    "crates/mado-pilot/benches/support/ocr_text_watch.rs",
    "crates/mado-pilot/benches/support/ocr_text_watch_cases.rs",
    "crates/mado-pilot/benches/support/ocr_text_watch_resources.rs",
    "crates/support/testkit/src/bench_harness.rs",
    "crates/support/testkit/src/controlled_capture.rs",
    "crates/support/testkit/src/controlled_ocr.rs",
    "crates/support/testkit/src/controlled_storage.rs",
    "crates/mado-pilot/examples/ocr-text-watch.rs",
    "crates/mado-pilot/examples/support/ocr_dependency_images.rs",
    "tools/ocr-text-watch/run_replay.py",
    "tools/native-release-profile/_process_group.py",
    "tools/native-release-profile/_windows_process.py",
)


def profile(path: Path) -> tuple[dict, dict]:
    data, document_identity = run_replay.read_document(path)
    value = tomllib.loads(data.decode("utf-8"))
    expected_target = run_replay.selected_target()
    canonical = ROOT / "docs/benchmarks" / f"ocr-text-watch-{expected_target}.toml"
    canonical_bytes, _ = run_replay.read_document(canonical)
    if data != canonical_bytes:
        raise ValueError("profile differs from the reviewed source-bound workload")
    facts = value["profile"]
    if not isinstance(facts, dict):
        raise ValueError("profile facts must be a table")
    if facts["release_target"] != expected_target:
        raise ValueError("profile and execution host target differ")
    expected = {
        "source_width": 960, "source_height": 540, "source_bytes": 2073600,
        "source_format": "bgra8", "roi": [0, 0, 960, 540],
        "query_minimum_interval_ms": 50, "capture_period_ms": 16,
        "process_count": 3, "warmup_iterations": 2, "sample_count": 20,
        "process_timeout_seconds": 300, "cleanup_timeout_seconds": 15,
        "output_limit_bytes": 1048576,
    }
    if any(type(facts.get(key)) is not type(setting) or facts[key] != setting
           for key, setting in expected.items()):
        raise ValueError("prospective controlled workload bounds changed; review apparatus applicability")
    workloads = value["workload"]
    if (not isinstance(workloads, list) or len(workloads) != len(WORKLOADS)
            or not all(isinstance(row, dict) for row in workloads)
            or tuple(row.get("id") for row in workloads) != WORKLOADS):
        raise ValueError("prospective workload order differs")
    if any(type(workloads[-1].get(key)) is not type(setting) or workloads[-1][key] != setting
           for key, setting in {
        "process_count": 5, "warmup_iterations": 0, "sample_count": 1,
    }.items()):
        raise ValueError("cold-startup cohort differs from its fixed five fresh processes")
    return value, document_identity


def host(path: Path, target: str) -> tuple[dict, dict]:
    data, document_identity = run_replay.read_document(path)
    value = json.loads(data.decode("utf-8"))
    if not isinstance(value, dict):
        raise ValueError("host record must be an object")
    if value.get("release_target") != target:
        raise ValueError("host record target differs")
    for key in ("host_id", "cpu", "os_version"):
        if not isinstance(value.get(key), str) or not value[key].strip():
            raise ValueError("explicit host identity, CPU and OS/build are required")
    if type(value.get("memory_bytes")) is not int or value["memory_bytes"] <= 0:
        raise ValueError("explicit positive host memory size is required")
    observed = observed_host()
    if value["host_id"] != observed["host_id"] or target != observed["release_target"]:
        raise ValueError("declared host identity differs from the actual execution host")
    verified = ("host_id", "release_target")
    return {
        "declared": value,
        "observed": observed,
        "verified_fields": list(verified),
        "unverified_declared_fields": sorted(set(value) - set(verified)),
    }, document_identity


def observed_host() -> dict:
    return {
        "host_id": platform.node(),
        "release_target": run_replay.selected_target(),
        "system": platform.system(),
        "release": platform.release(),
        "version": platform.version(),
        "machine": platform.machine(),
    }


def source() -> dict:
    if Path(run_replay.git("rev-parse", "--show-toplevel")).resolve(strict=True) != ROOT:
        raise ValueError("Git source root differs from the workload apparatus root")
    snapshot = {
        "commit": run_replay.git("rev-parse", "HEAD"),
        "tree": run_replay.git("rev-parse", "HEAD^{tree}"),
        "status": run_replay.git("status", "--porcelain=v1", "--untracked-files=all", "--ignore-submodules=none"),
    }
    if snapshot["status"]:
        raise ValueError("every workload candidate source must be committed and clean")
    return snapshot


def bindings(executable: Path, corpus: Path | None, environment: dict[str, str],
             approved_images: dict[str, str]) -> dict:
    files = {name: run_replay.identity(ROOT / name) for name in HARNESS}
    files.update({
        "executable": run_replay.identity(executable),
        "native_images": run_replay.verify_native_manifest(approved_images),
        "selection_environment": selection_environment(environment),
        "procedure": run_replay.identity(Path(__file__)),
        "supervisor": run_replay.identity(ROOT / "tools/native-release-profile/process_runner.py"),
    })
    if corpus is not None:
        files["real_cpu_inputs"] = run_replay.inputs(executable, corpus, environment)
    required = [files["executable"]]
    if corpus is not None:
        required.append(files["real_cpu_inputs"]["runtime"])
    if any(approved_images.get(entry["path"]) != entry["sha256"] for entry in required):
        raise ValueError("required executable or runtime image is not approved")
    return files


def selection_environment(environment: dict[str, str]) -> dict[str, str]:
    """Record selection inputs from the frozen environment actually passed to children."""
    names = {
        "PATH", "PATHEXT", "SYSTEMROOT", "WINDIR", "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH", "MADO_PILOT_G004_MODEL_ROOT", "MADO_PILOT_ONNX_RUNTIME",
    }
    return {key: value for key, value in sorted(environment.items())
            if key.upper() in names}


def controlled_semantics(stdout: str) -> bool:
    starts = [line for line in stdout.splitlines() if line.startswith("# ocr-start ")]
    rows = [line for line in stdout.splitlines() if line.startswith("# ocr-raw ")]
    expected = [(name, "warmup" if index < 2 else "sample", index)
                for name in WORKLOADS[:-1] for index in range(22)]
    expected.append(("cold-startup", "sample", 0))
    if len(starts) != len(expected) or len(rows) != len(expected):
        return False
    for start, row, (name, phase, index) in zip(starts, rows, expected):
        suffix = f"workload={name} phase={phase} iteration={index}"
        if start != f"# ocr-start {suffix}" or not row.startswith(f"# ocr-raw {suffix} "):
            return False
        if " semantic=passed " not in row:
            return False
    return ("ocr-text-watch-query: 6 workloads, 20 samples each, 0 oracle failure(s)" in stdout
            and "ocr-text-watch-controlled-startup: 1 workloads, 1 samples each, 0 oracle failure(s)" in stdout)


def execute(args: argparse.Namespace) -> bool:
    if args.enforce_budgets:
        raise ValueError("numeric enforcement requires an accepting target-specific budget ADR")
    if not args.execute:
        raise ValueError("explicit execution authority is required")
    if args.mode not in ("controlled", "real-cpu-cold-startup"):
        raise ValueError("unknown workload mode")
    if sys.platform == "win32" and sys.version_info < (3, 13):
        raise ValueError("private Windows evidence directories require Python 3.13 or newer")
    real = args.mode == "real-cpu-cold-startup"
    count = 5 if real else 3
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    rows = []
    failures = []
    first_apparatus_failure = None
    admitted = False
    unchanged = False
    final_failure = None

    def retain_failure(stage: str, process: int | None, error: BaseException) -> dict:
        nonlocal first_apparatus_failure
        failure = {
            "kind": "apparatus-invalid", "stage": stage, "process": process,
            "exception_type": type(error).__name__, "reason": str(error)[:4096],
        }
        failures.append(failure)
        if first_apparatus_failure is None:
            first_apparatus_failure = failure
            run_replay.write_record(output / "apparatus-invalid.json", failure)
        return failure

    stage = "preflight-inputs"
    try:
        if real and args.corpus is None:
            raise ValueError("real CPU cold startup requires the fixed prepared corpus")
        if not real and args.corpus is not None:
            raise ValueError("controlled semantic mode does not accept model or corpus selection")
        executable = args.executable.resolve(strict=True)
        corpus = args.corpus.resolve(strict=True) if real else None
        stage = "preflight-profile"
        selected, profile_identity = profile(args.profile)
        facts = selected["profile"]
        stage = "preflight-host"
        host_facts, host_identity = host(args.host_record, facts["release_target"])
        stage = "preflight-source"
        original_source = source()
        stage = "preflight-native-manifest"
        approved_images, manifest_identity = run_replay.native_manifest(args.native_images)
        documents = {
            "profile": profile_identity, "host_record": host_identity,
            "native_manifest": manifest_identity,
        }
        stage = "preflight-environment"
        environment = run_replay.canonical_environment() if real else dict(os.environ)
        report_paths = [output / f"process-{index + 1}.native-images.txt" for index in range(count)]
        child_environments = [
            run_replay.dependency_environment(environment, report_path) for report_path in report_paths
        ]
        stage = "preflight-bindings"
        before = {
            "documents": documents,
            "artifacts": bindings(executable, corpus, environment, approved_images),
        }
        argv = [str(executable), str(corpus), "transition"] if real else [str(executable), "--semantic"]
        stage = "plan-evidence"
        run_replay.write_record(output / "plan.json", {
            "schema_version": 2, "mode": args.mode, "authority": "explicit --execute; this exact prospective cohort requires operator authorization",
            "target": facts["release_target"], "host": host_facts, "source": original_source,
            "bindings": before, "argv": argv, "process_count": count,
            "child_selection_environments": [selection_environment(child) for child in child_environments],
            "dependency_report_paths": [str(path) for path in report_paths],
            "native_dependency_policy": "preapproved exact loaded-image set; rehashed before every process and finally; missing observation is nonpass",
            "warmups": 0 if real else 2, "samples_per_workload": 1 if real else 20,
            "controlled_startup_override": {"warmups": 0, "samples": 1} if not real else None,
            "process_timeout_seconds": facts["process_timeout_seconds"],
            "cleanup_timeout_seconds": facts["cleanup_timeout_seconds"],
            "output_limit_bytes": facts["output_limit_bytes"], "retries": 0, "exclusions": 0,
            "order": ["cold-startup"] if real else list(WORKLOADS),
            "clock": "supervisor monotonic launch-attempt through process-tree cleanup; Rust per-sample Instant is separate",
            "cold_startup_scope": "real mode executes the existing public example once in each fresh process; no OS cache flush or internal startup timestamp is implied",
            "real_example_overrides": {"query_interval_ms": 1, "confirmation_count": 1} if real else None,
            "resource_scope": "controlled rows expose process RSS and logical cache/result extents, not native fixture allocation; real example has no RSS or native-pair counter hook",
            "numeric_enforcement": "refused: no accepted budget ADR", "native_capture": "unexecuted-separate-target-procedure",
            "private_record": "Contains declared and observed host facts, controlled path/digest identities, loader-selection environment and complete raw process output",
        })
        admitted = True
    except (Exception, KeyboardInterrupt) as error:
        retain_failure(stage, None, error)

    if admitted:
        def check_bindings() -> None:
            if source() != original_source:
                raise ValueError("prospective source identity changed")
            if observed_host() != host_facts["observed"]:
                raise ValueError("observed execution host changed")
            current_documents = {
                name: run_replay.read_document(Path(entry["path"]))[1]
                for name, entry in documents.items()
            }
            if current_documents != before["documents"]:
                raise ValueError("profile, host record or native manifest differs from parsed bytes")
            if bindings(executable, corpus, environment, approved_images) != before["artifacts"]:
                raise ValueError("prospective executable, apparatus, corpus or native identity changed")

        for index in range(count):
            row = {
                "process": index + 1, "mode": args.mode, "execution_state": "not-attempted",
                "semantic_passed": False, "cleanup_passed": False, "dependencies_passed": False,
                "observed": None, "native_dependencies": None,
                "dependency_report_path": str(report_paths[index]),
                "selection_environment": selection_environment(child_environments[index]),
                "elapsed_scope": "whole fresh process including driver and teardown; never held-double OCR performance",
                "rss": None if real else "see raw target snapshots; missing values are nonpass, not zero",
                "internal_startup_endpoints": "unmeasured-nonpass" if real else "controlled-only raw endpoints",
                "native_session_pair_count": "unmeasured-nonpass" if real else "not-applicable-controlled-backend",
                "physical_mapping_total_bytes": None,
                "physical_mapping_total_status": "unmeasured-nonpass; backend input/cache/accessor extents are not a complete physical-map ledger",
                "required_measurements_complete": False, "budget_passed": False,
            }
            stage = "before-process-bindings"
            try:
                check_bindings()
                if report_paths[index].is_symlink() or report_paths[index].exists():
                    raise ValueError("dependency observation output already exists before launch")
                stage = "process-supervision"
                row["execution_state"] = "attempted-unconfirmed"
                observed = run_replay.run_process(
                    argv, cwd=ROOT, env=child_environments[index],
                    timeout_seconds=facts["process_timeout_seconds"],
                    output_limit_bytes=facts["output_limit_bytes"],
                    cleanup_seconds=facts["cleanup_timeout_seconds"],
                )
                row["observed"] = observed
                row["execution_state"] = "started" if observed["launch_error"] is None else "not-started"
                stage = "process-result"
                if real:
                    semantic = "ocr-text-watch: scenario=transition terminal=Matched sequence=1 regions=8 satisfying=1 confirmations=1 retained_bytes=2073600 cleanup=returned" in observed["stdout"]
                else:
                    semantic = controlled_semantics(observed["stdout"])
                row["semantic_passed"] = semantic and observed["exit_code"] == 0 and observed["launch_error"] is None
                row["cleanup_passed"] = observed["cleanup_ok"] and not observed["timed_out"] and not observed["output_limited"]
                if not row["semantic_passed"] or not row["cleanup_passed"]:
                    failures.append({
                        "kind": "process-failure", "stage": stage, "process": index + 1,
                        "semantic_passed": row["semantic_passed"], "cleanup_passed": row["cleanup_passed"],
                    })
                stage = "dependency-observation"
                dependencies = run_replay.observe_dependencies(observed, report_paths[index], approved_images)
                row["native_dependencies"] = dependencies
                row["dependencies_passed"] = dependencies["matched"] is True
                if not row["dependencies_passed"]:
                    raise ValueError("actual loaded native images do not match the approved manifest")
            except (Exception, KeyboardInterrupt) as error:
                row["apparatus_failure"] = retain_failure(stage, index + 1, error)
            run_replay.write_record(output / f"process-{index + 1}.json", row)
            rows.append(row)
            if failures:
                break

        try:
            check_bindings()
            unchanged = True
        except (Exception, KeyboardInterrupt) as error:
            final_failure = retain_failure("final-bindings", None, error)

    semantic_complete = (
        admitted and unchanged and not failures and len(rows) == count
        and all(row["semantic_passed"] and row["cleanup_passed"] and row["dependencies_passed"] for row in rows)
    )
    states = {row["process"]: row["execution_state"] for row in rows}
    run_replay.write_record(output / "result.json", {
        "schema_version": 2, "mode": args.mode, "admitted": admitted, "process_count": count,
        "identity_unchanged": unchanged, "semantic_cohort_passed": semantic_complete,
        "executed_processes": sum(state == "started" for state in states.values()),
        "attempted_processes": sum(state != "not-attempted" for state in states.values()),
        "unexecuted_processes": [
            number for number in range(1, count + 1)
            if states.get(number, "not-attempted") in ("not-attempted", "not-started")
        ],
        "execution_unknown_processes": [number for number, state in states.items() if state == "attempted-unconfirmed"],
        "first_failure": failures[0] if failures else None, "failures": failures,
        "apparatus_invalid": first_apparatus_failure is not None,
        "final_binding_check": {"performed": admitted, "matched": unchanged, "failure": final_failure},
        "required_measurements_complete": False, "passed": False,
        "task_8_2": "not-passed", "task_8_3": "not-passed", "numeric_budgets": "unaccepted",
        "native_scope": "unexecuted; no fixture, capture, signature, permission or input operation is delegated",
    })
    return semantic_complete


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", required=True, choices=("controlled", "real-cpu-cold-startup"))
    parser.add_argument("--executable", required=True, type=Path, help="bench executable for controlled mode; public example executable for real CPU mode")
    parser.add_argument("--profile", required=True, type=Path)
    parser.add_argument("--host-record", required=True, type=Path, help="private JSON containing release_target, host_id matching platform.node(), cpu, os_version, memory_bytes; CPU/OS/memory declarations remain unverified")
    parser.add_argument("--native-images", required=True, type=Path, help="independently approved canonical native-image path to SHA-256 JSON; never derived from this execution")
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--output", required=True, type=Path, help="new private evidence directory outside tracked source or under an ignored output path, never overwritten")
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--enforce-budgets", action="store_true")
    args = parser.parse_args()
    if args.enforce_budgets:
        parser.error("numeric enforcement is refused: no accepting target-specific budget ADR exists")
    if not args.execute:
        parser.error("--execute is required after authorization of the exact prospective cohort")
    try:
        complete = execute(args)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
        print("OCR workload procedure failed; retain and inspect private evidence without retry", file=sys.stderr)
        return 1
    print("OCR semantic cohort completed; workload qualification remains nonpass" if complete else "OCR cohort failed; later processes remain unexecuted")
    return 0 if complete else 1


if __name__ == "__main__":
    raise SystemExit(main())
