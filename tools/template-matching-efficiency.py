#!/usr/bin/env python3
"""Collect source-bound template benchmarks from explicit prebuilt executables only.

Run ``policy`` before calibration; ``freeze`` reviews a successful calibration;
``compare`` consumes that immutable, SHA-256-pinned profile. No command builds.
Build metadata is an operator-supplied provenance attestation, not a reproducible
build proof. Keep the calibration directory with its reviewed profile.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import math
import os
import platform
import re
import statistics
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUNNER_DIR = ROOT / "tools/native-release-profile"
sys.path.insert(0, str(RUNNER_DIR))
from process_runner import run_process  # noqa: E402

TARGETS = ("aarch64-apple-darwin", "x86_64-pc-windows-msvc")
# Order is part of the corpus contract, not a user-selectable qualification subset.
WORKLOADS = (
    ("single-small96x64", "guard", False, True),
    ("single-1080p", "guard", False, True),
    ("single-4k", "guard", False, True),
    ("selection-k1-large", "selection", False, True),
    ("selection-k16-dense1080p-keepall", "selection", True, True),
    ("selection-k16-dense1080p-dropoverlapping", "selection", True, True),
    ("selection-k256-small-dense-tied", "selection", True, True),
    ("selection-k256-large-sparse", "selection", True, True),
    ("batch8-cold-full1080p", "repeated-preparation", True, True),
    ("batch8-cold-roi320x180", "repeated-preparation", True, True),
    ("batch8-live-owner-roi320x180", "repeated-preparation", False, True),
    ("batch8-warm-full1080p", "repeated-preparation", False, True),
    ("concurrent2-cold-full1080p", "guard", False, True),
    ("contract-different-frame", "contract", False, False),
    ("contract-equal-stamp", "contract", False, False),
    ("contract-expired-mapping", "contract", False, False),
    ("contract-eviction", "contract", False, False),
    ("contract-oversized-bgr", "contract", False, False),
    ("contract-roi-format-transition", "contract", False, False),
    ("watch-analysis-always", "contract", False, False),
    ("watch-exact-rgba", "contract", False, False),
)
DEFINITIONS = {row[0]: dict(zip(("id", "category", "improvement", "statistical"), row))
               for row in WORKLOADS}
MIB = 1024 * 1024
POLICY = {
    "id": "template-efficiency-v1",
    "processes": 5,
    "warmups": 20,
    "samples": 200,
    "smoke_warmups": 1,
    "smoke_samples": 3,
    "noise_cap": 0.15,
    "timing_ceiling": "ceil(max(calibration5) + max(spread, 2 * clock_allowance))",
    "benefit": "median(comparison-baseline5) - median(candidate5) > frozen allowance",
    "noise": "spread / max(median(calibration5), 2 * clock_allowance) <= noise_cap; zero/quantized guards retain an absolute ceiling",
    "cpu_metric": "sum retained sample CPU deltas per process, not sample p95",
    "cpu_accounting_floor_ns": {TARGETS[0]: 1000, TARGETS[1]: 15625000},
    "clock_allowance": "maximum observed calibration resolution, CPU at least fixed accounting floor; not clock units or a proof of accuracy",
    "resource_ceiling": "max(calibration5) + max(spread, fixed slack); native RSS peak only additionally permits one live-cache allowance",
    "rss_slack_bytes": MIB,
    "heap_slack_bytes": 256 * 1024,
    "live_cache_bytes": 32 * MIB,
    "growth_and_residual": "positive endpoint difference, one total bound per process, no live-cache or per-cycle exemption",
    "timeout_seconds": 300,
    "output_limit_bytes": 8 * MIB,
    "cleanup_seconds": 5,
    "percentile": "nearest rank ceil(0.95 * count), no sample exclusion",
}
COUNTERS = ("allocations", "score_copied_bytes", "score_offsets_scanned", "bgr_conversions",
            "bgr_converted_bytes", "bgr_retained_bytes", "bgr_retained_peak_bytes",
            "bgr_retained_after_drop_bytes", "bgr_evicted_bytes", "bgr_evicted_peak_bytes",
            "bgr_evicted_after_drop_bytes", "mapping_copies", "mapping_copied_bytes")
MEMORY = ("peak_rss_bytes", "rss_before_bytes", "rss_after_bytes", "rss_after_drop_bytes",
          "rss_steady_before_bytes", "rss_steady_after_bytes", "heap_before_bytes",
          "heap_after_bytes", "heap_after_drop_bytes", "peak_heap_bytes",
          "heap_steady_before_bytes", "heap_steady_after_bytes")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, name, minimum=0):
    require(type(value) is int and minimum <= value <= (1 << 64) - 1,
            f"{name}: expected bounded integer >= {minimum}")
    return value


def digest(value, name, size=64):
    require(isinstance(value, str) and re.fullmatch(f"[0-9a-f]{{{size}}}", value),
            f"{name}: expected {size}-character lowercase digest")
    return value


def nonempty(value, name):
    require(isinstance(value, str) and value.strip(), f"{name}: expected nonempty text")
    return value


def strict_json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def invalid(value):
        raise ValueError(f"nonfinite JSON number: {value}")

    return json.loads(text, object_pairs_hook=pairs, parse_constant=invalid)


def document(value):
    return (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")


def load(path):
    return strict_json(Path(path).read_text(encoding="utf-8"))


def save(path, value):
    # Exclusive creation preserves every prior campaign and failed attempt.
    with Path(path).open("xb") as stream:
        stream.write(document(value))


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def collector_identity():
    paths = (Path(__file__).resolve(), RUNNER_DIR / "process_runner.py",
             RUNNER_DIR / "_process_group.py", RUNNER_DIR / "_windows_process.py")
    return {path.relative_to(ROOT).as_posix(): sha256(path) for path in paths}


def host_identity():
    machine = platform.machine()
    target = None
    if sys.platform == "darwin" and machine == "arm64":
        target = TARGETS[0]
    elif sys.platform == "win32" and machine.upper() in ("AMD64", "X86_64"):
        target = TARGETS[1]
    return {"target": target, "node": platform.node(), "system": platform.system(),
            "release": platform.release(), "version": platform.version(), "machine": machine,
            "processor": platform.processor(), "python": platform.python_version()}


def environment():
    names = ("PATH", "HOME", "USERPROFILE", "SystemRoot", "SYSTEMROOT", "WINDIR",
             "TMPDIR", "TEMP", "TMP", "DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH",
             "OMP_NUM_THREADS", "OPENCV_FOR_THREADS_NUM", "OPENBLAS_NUM_THREADS",
             "MKL_NUM_THREADS", "VECLIB_MAXIMUM_THREADS")
    return {name: os.environ[name] for name in names if name in os.environ}


def validate_build(build, target):
    require(isinstance(build, dict), "build metadata must be an object")
    require(build.get("target") == target, "build target mismatch")
    for name in ("source_revision", "source_tree"):
        digest(build.get(name), name, 40)
    for name in ("binary_sha256", "cargo_lock_sha256"):
        digest(build.get(name), name)
    nonempty(build.get("rustc"), "rustc")
    argv = build.get("build_argv")
    require(isinstance(argv, list) and argv and all(isinstance(arg, str) and arg for arg in argv),
            "build_argv must retain the exact build command")
    nonempty(build.get("opencv_version"), "opencv_version")
    dependencies = build.get("dependencies")
    require(isinstance(dependencies, list) and dependencies, "native dependency hashes required")
    paths = set()
    for dependency in dependencies:
        require(isinstance(dependency, dict), "invalid dependency")
        path = nonempty(dependency.get("path"), "dependency path")
        require(Path(path).is_absolute() and path not in paths, "dependency paths must be unique and absolute")
        paths.add(path)
        digest(dependency.get("sha256"), "dependency sha256")


def binary_identity(binary, metadata, target):
    path = Path(binary).resolve(strict=True)
    require(path.is_file(), "binary must be an existing file")
    metadata_path = Path(metadata).resolve(strict=True)
    build = load(metadata_path)
    validate_build(build, target)
    require(sha256(path) == build["binary_sha256"], "binary does not match build attestation")
    identity = {"path": str(path), "sha256": build["binary_sha256"], "build": build,
                "metadata_path": str(metadata_path), "metadata_sha256": sha256(metadata_path)}
    check_files(identity)
    return identity


def check_files(identity):
    require(sha256(identity["path"]) == identity["sha256"], "binary changed")
    require(sha256(identity["metadata_path"]) == identity["metadata_sha256"], "source metadata changed")
    for dependency in identity["build"]["dependencies"]:
        require(sha256(dependency["path"]) == dependency["sha256"], "native dependency changed")


def successful(record):
    duration = record.get("duration_seconds")
    return (type(record.get("exit_code")) is int and record["exit_code"] == 0
            and record.get("timed_out") is False and record.get("output_limited") is False
            and record.get("cleanup_ok") is True and "launch_error" in record
            and record["launch_error"] is None
            and isinstance(record.get("stdout"), str) and isinstance(record.get("stderr"), str)
            and type(duration) in (int, float) and math.isfinite(duration) and duration >= 0)


def manifest_identity(value, target, opencv_version):
    require(isinstance(value, dict) and value.get("format_version") == 1, "invalid protocol version")
    require(value.get("kind") == "manifest", "expected manifest")
    require(value.get("target") == target, "manifest target mismatch")
    require(value.get("opencv_version") == opencv_version, "manifest OpenCV identity mismatch")
    fixture = digest(value.get("fixture_sha256"), "fixture_sha256")
    rows = value.get("workloads")
    require(isinstance(rows, list) and len(rows) == len(WORKLOADS), "incomplete workload manifest")
    frozen = []
    for row, expected in zip(rows, DEFINITIONS.values()):
        require(isinstance(row, dict), "invalid manifest row")
        require(all(type(row.get(key)) is type(want) and row.get(key) == want
                    for key, want in expected.items()), f"manifest row mismatch: {expected['id']}")
        nonempty(row.get("cache_state"), "cache_state")
        nonempty(row.get("timing_boundary"), "timing_boundary")
        for key in ("width", "height", "result_limit"):
            integer(row.get(key), key, 1)
        require(row.get("suppression") in ("KeepAll", "DropOverlapping"), "suppression missing")
        require(row.get("warmups") == (20 if expected["statistical"] else 1)
                and row.get("sample_count") == (200 if expected["statistical"] else 3),
                "manifest sampling plan mismatch")
        frozen.append(dict(row))
    return {"fixture_sha256": fixture, "target": target, "opencv_version": opencv_version,
            "workloads": frozen}


def observations(value, manifest, workload, smoke):
    require(isinstance(value, dict) and value.get("format_version") == 1
            and value.get("kind") == "observations", "invalid observation protocol")
    for key in ("target", "opencv_version", "fixture_sha256"):
        require(value.get(key) == manifest[key], f"observation {key} mismatch")
    rows = value.get("workloads")
    require(isinstance(rows, list) and len(rows) == 1, "one fresh process must report exactly one workload")
    row = rows[0]
    expected = next(item for item in manifest["workloads"] if item["id"] == workload)
    require(isinstance(row, dict) and all(type(row.get(key)) is type(want) and row.get(key) == want
                                        for key, want in expected.items()
                                        if key not in ("warmups", "sample_count")), "observation workload mismatch")
    short = smoke or not expected["statistical"]
    warmups, count = (1, 3) if short else (20, 200)
    require(row.get("warmups") == warmups and row.get("sample_count") == count,
            "row sampling plan mismatch")
    # Top-level counts select the campaign; contract rows declare their short plan.
    require(value.get("warmups") == (1 if smoke else 20)
            and value.get("sample_count") == (3 if smoke else 200),
            "process sampling plan mismatch")
    metrics = row.get("metrics")
    require(isinstance(metrics, dict), "missing metrics")
    unavailable = []
    for reason in ("cpu_reason", "native_resource_reason"):
        if metrics.get(reason) is not None:
            unavailable.append(nonempty(metrics[reason], reason))
    require(metrics.get("peak_rss_scope") == "process-high-water"
            and metrics.get("peak_heap_scope") == "row-including-setup-and-warmup",
            "peak measurements must include setup and warmup")
    integer(metrics.get("allocations"), "allocations")
    for name in COUNTERS:
        integer(metrics.get(name), name)
    for name in MEMORY:
        if metrics.get(name) is None:
            nonempty(metrics.get("native_resource_reason"), "native_resource_reason")
            unavailable.append(name)
        else:
            integer(metrics[name], name)
    require(integer(metrics.get("iterations"), "iterations", 1) == warmups + count,
            "counter iteration denominator mismatch")
    require(integer(row.get("oracle_failures"), "oracle_failures") == 0, "oracle failures")
    retained = []
    for name, expected_count in (("warmup_samples", warmups), ("samples", count)):
        samples = row.get(name)
        require(isinstance(samples, list) and len(samples) == expected_count, f"missing {name}")
        for index, sample in enumerate(samples):
            require(isinstance(sample, dict), f"invalid {name}[{index}]")
            require(sample.get("correct") is True, f"incorrect {name}[{index}]")
            integer(sample.get("elapsed_ns"), "elapsed_ns")
            if sample.get("cpu_ns") is None:
                nonempty(metrics.get("cpu_reason"), "cpu_reason")
                unavailable.append(f"{name}[{index}].cpu_ns")
            else:
                integer(sample["cpu_ns"], "cpu_ns")
        if name == "samples":
            retained = samples
    timer = integer(value.get("timer_resolution_ns"), "timer_resolution_ns", 1)
    nonempty(value.get("timer_resolution_method"), "timer_resolution_method")
    cpu_resolution = value.get("cpu_timer_resolution_ns")
    if cpu_resolution is None:
        unavailable.append("cpu_timer_resolution_ns")
    else:
        integer(cpu_resolution, "cpu_timer_resolution_ns", 1)
    nonempty(value.get("cpu_resolution_method"), "cpu_resolution_method")
    cpu = None if any(sample["cpu_ns"] is None for sample in retained) else sum(sample["cpu_ns"] for sample in retained)
    require(metrics.get("cpu_ns") == cpu, "aggregate CPU disagrees with retained samples")
    require(metrics["bgr_retained_bytes"] <= POLICY["live_cache_bytes"]
            and metrics["bgr_retained_peak_bytes"] <= POLICY["live_cache_bytes"], "live BGR retention bound")
    require(metrics["bgr_retained_after_drop_bytes"] == 0, "BGR payload survives final owner")
    require(metrics["bgr_evicted_after_drop_bytes"] == 0, "evicted BGR payload survives final owner")
    for state in ("retained", "evicted"):
        require(metrics[f"bgr_{state}_peak_bytes"] >= metrics[f"bgr_{state}_bytes"],
                f"BGR {state} peak below measured endpoint")
    for prefix in ("rss", "heap"):
        peak = metrics[f"peak_{prefix}_bytes"]
        endpoints = [metrics[f"{prefix}_{part}_bytes"] for part in
                     ("before", "after", "after_drop", "steady_before", "steady_after")]
        if peak is not None and all(item is not None for item in endpoints):
            require(peak >= max(endpoints), f"{prefix} peak below measured endpoint")
    elapsed = sorted(sample["elapsed_ns"] for sample in retained)
    summary = {"p50_ns": elapsed[(count - 1) // 2], "p95_ns": elapsed[math.ceil(0.95 * count) - 1],
               "max_ns": elapsed[-1], "cpu_ns": cpu, "timer_resolution_ns": timer,
               "cpu_timer_resolution_ns": cpu_resolution, "metrics": metrics, "unavailable": unavailable}
    for prefix in ("rss", "heap"):
        summary[f"peak_{prefix}_bytes"] = metrics[f"peak_{prefix}_bytes"]
        for kind, first, last in (("growth", "steady_before", "steady_after"),
                                  ("residual", "before", "after_drop")):
            a, b = metrics[f"{prefix}_{first}_bytes"], metrics[f"{prefix}_{last}_bytes"]
            summary[f"{prefix}_{kind}_bytes"] = None if a is None or b is None else max(0, b - a)
    return summary


def schedule(mode, selected):
    roles = ("baseline", "candidate") if mode == "comparison" else ("baseline",)
    result = [{"role": role, "workload": None, "process": None} for role in roles]
    for workload in selected:
        for process in range(1 if mode == "smoke" else 5):
            for role in roles:
                result.append({"role": role, "workload": workload, "process": process})
    for index, item in enumerate(result):
        item["id"] = f"{index:04d}"
    return result


def validate_plan(plan):
    require(plan.get("format_version") == 1 and plan.get("kind") == "campaign", "invalid campaign")
    require(plan.get("mode") in ("calibration", "comparison", "smoke"), "invalid cohort")
    require(plan.get("policy") == POLICY, "policy differs from predeclared collector policy")
    target = plan.get("target")
    require(target in TARGETS and plan.get("host", {}).get("target") == target, "host/target mismatch")
    require(plan.get("collector") == collector_identity(), "collector source differs; use the retained collector revision")
    selected = plan.get("workloads")
    require(isinstance(selected, list) and selected, "missing workload selection")
    require(selected == list(DEFINITIONS) if plan["mode"] != "smoke" else
            len(selected) == len(set(selected)) and all(row in DEFINITIONS for row in selected),
            "qualification workload coverage mismatch")
    require(plan.get("schedule") == schedule(plan["mode"], selected), "cohort order/count mismatch")
    require(plan.get("exclusive_host") is True or plan["mode"] == "smoke", "exclusive host declaration required")
    roles = ("baseline", "candidate") if plan["mode"] == "comparison" else ("baseline",)
    require(set(plan.get("binaries", {})) == set(roles), "missing role provenance")
    for identity in plan["binaries"].values():
        validate_build(identity.get("build"), target)
        require(identity.get("sha256") == identity["build"]["binary_sha256"], "binary/source binding mismatch")
        digest(identity.get("metadata_sha256"), "metadata_sha256")
        require(Path(identity.get("path", "")).is_absolute(), "binary path missing")
    require(isinstance(plan.get("environment"), dict), "environment missing")


def evaluate(directory):
    directory = Path(directory).resolve(strict=True)
    plan = load(directory / "plan.json")
    validate_plan(plan)
    manifests, observations_by_role, attempts, issues = {}, {}, [], []
    expected_files = {f"{task['id']}.json" for task in plan["schedule"]}
    files = {}
    for folder in ("attempts", "started"):
        files[folder] = {path.name for path in (directory / folder).glob("*.json")}
        require(not files[folder] - expected_files, f"unexpected {folder}: hidden retry or extra process")
    plan_hash = sha256(directory / "plan.json")
    for task in plan["schedule"]:
        filename = f"{task['id']}.json"
        label = f"{task['id']}:{task['role']}:{task['workload'] or 'manifest'}"
        completed = filename in files["attempts"]
        if not completed and filename not in files["started"]:
            issues.append({"attempt": label, "status": "not-run", "reason": "no started or completed attempt record"})
            continue
        path = directory / ("attempts" if completed else "started") / filename
        relative = path.relative_to(directory).as_posix()
        retained = {"file": filename if completed else relative, "sha256": None}
        attempts.append(retained)
        try:
            raw = path.read_bytes()
            retained["sha256"] = hashlib.sha256(raw).hexdigest()
            attempt = strict_json(raw.decode("utf-8"))
            require(isinstance(attempt, dict), "attempt record must be an object")
            require(attempt.get("task") == task, "attempt identity/order mismatch")
            require(attempt.get("plan_sha256") == plan_hash, "attempt plan binding mismatch")
            binary = plan["binaries"][task["role"]]
            argv = command(binary, task, plan["mode"])
            require(attempt.get("argv") == argv, "attempt argv mismatch")
            if not completed:
                issues.append({"attempt": label, "file": relative, "status": "incomplete",
                               "reason": "attempt started without a completed record"})
                continue
            require(attempt.get("interruption") is None, f"attempt interrupted: {attempt.get('interruption')}")
            require(attempt.get("identity_before") == binary and attempt.get("identity_after") == binary,
                    "binary/dependency identity changed or unavailable")
            require(attempt.get("identity_error") is None, "binary/dependency identity check failed")
            record = attempt.get("record")
            require(isinstance(record, dict) and successful(record), "process failed, timed out, truncated, or cleanup failed")
            value = strict_json(record["stdout"])
            if task["workload"] is None:
                manifests[task["role"]] = manifest_identity(value, plan["target"], binary["build"]["opencv_version"])
            else:
                require(task["role"] in manifests, "role has no valid manifest")
                summary = observations(value, manifests[task["role"]], task["workload"], plan["mode"] == "smoke")
                summary["process"] = task["process"]
                observations_by_role.setdefault(task["role"], {}).setdefault(task["workload"], []).append(summary)
                if plan["mode"] != "smoke" and summary["unavailable"]:
                    issues.append({"attempt": label, "file": relative, "status": "incomplete",
                                   "reason": "unavailable observations: " + ", ".join(summary["unavailable"])})
        except (OSError, ValueError, TypeError, KeyError, StopIteration) as error:
            issues.append({"attempt": label, "file": relative, "status": "failed",
                           "reason": f"{relative}: {type(error).__name__}: {error}"})
    if "candidate" in manifests and manifests.get("baseline") != manifests["candidate"]:
        issues.append({"attempt": "manifests", "status": "failed", "reason": "role fixture/workload/dependency identity mismatch"})
    return {"plan": plan, "plan_sha256": plan_hash, "attempts": attempts,
            "manifest": manifests.get("baseline"), "observations": observations_by_role, "issues": issues}


def derive_envelopes(evidence):
    require(evidence["plan"]["mode"] == "calibration", "not a baseline-only calibration")
    require(not evidence["issues"], "calibration has failed, unavailable, or unrun attempts")
    result = {}
    for workload, definition in DEFINITIONS.items():
        rows = evidence["observations"]["baseline"][workload]
        require(len(rows) == 5, "calibration requires five fresh processes")
        limits, allowances = {}, {}
        if definition["statistical"]:
            for metric, clock in (("p95_ns", "timer_resolution_ns"), ("cpu_ns", "cpu_timer_resolution_ns")):
                values = [row[metric] for row in rows]
                median = statistics.median(values)
                resolution = max(row[clock] for row in rows)
                if metric == "cpu_ns":
                    resolution = max(resolution, POLICY["cpu_accounting_floor_ns"][evidence["plan"]["target"]])
                spread = max(values) - min(values)
                allowance = max(spread, 2 * resolution)
                require(spread / max(median, 2 * resolution) <= POLICY["noise_cap"],
                        f"noisy calibration: {workload}/{metric}; retain the whole campaign")
                limits[metric] = math.ceil(max(values) + allowance)
                allowances[metric] = allowance
        for prefix in ("rss", "heap"):
            for metric in (f"peak_{prefix}_bytes", f"{prefix}_growth_bytes", f"{prefix}_residual_bytes"):
                values = [row[metric] for row in rows]
                slack = max(max(values) - min(values), POLICY[f"{prefix}_slack_bytes"])
                # OpenCV payload is native: no allowance for Rust heap, growth or residual.
                live = POLICY["live_cache_bytes"] if metric == "peak_rss_bytes" and definition["category"] != "selection" else 0
                limits[metric] = max(values) + slack + live
        result[workload] = {"limits": limits, "benefit_allowances": allowances}
    return result


def profile_from(evidence, calibration, reviewer, rationale):
    return {"format_version": 1, "kind": "frozen-profile", "target": evidence["plan"]["target"],
            "policy": POLICY, "collector": evidence["plan"]["collector"], "host": evidence["plan"]["host"],
            "environment": evidence["plan"]["environment"], "baseline": evidence["plan"]["binaries"]["baseline"],
            "manifest": evidence["manifest"], "envelopes": derive_envelopes(evidence),
            "calibration": {"path": str(Path(calibration).resolve()), "plan_sha256": evidence["plan_sha256"],
                            "attempts": evidence["attempts"]},
            "reviewer": nonempty(reviewer, "reviewer"), "rationale": nonempty(rationale, "rationale")}


def validate_profile(profile):
    require(profile.get("format_version") == 1 and profile.get("kind") == "frozen-profile", "invalid profile")
    calibration = profile.get("calibration", {})
    evidence = evaluate(calibration["path"])
    expected = profile_from(evidence, calibration["path"], profile.get("reviewer"), profile.get("rationale"))
    require(profile == expected, "profile/provenance/envelopes differ from frozen calibration")
    return profile


def compare_evidence(evidence, profile):
    plan = evidence["plan"]
    require(plan["mode"] == "comparison", "not a comparison cohort")
    for name in ("target", "collector", "host", "environment"):
        require(plan[name] == profile[name], f"comparison {name} differs from calibration")
    require(plan["binaries"]["baseline"] == profile["baseline"], "comparison baseline source/binary changed")
    if evidence["manifest"] is not None:
        require(evidence["manifest"] == profile["manifest"], "comparison fixture/manifest changed")
    issues = list(evidence["issues"])
    benefits = {}
    for workload, definition in DEFINITIONS.items():
        cohorts = {role: evidence["observations"].get(role, {}).get(workload, [])
                   for role in ("baseline", "candidate")}
        complete = all(len(rows) == 5 and all(not row["unavailable"] for row in rows)
                       for rows in cohorts.values())
        envelope = profile["envelopes"][workload]
        for role, rows in cohorts.items():
            for row in rows:
                process = row["process"]
                for metric, limit in envelope["limits"].items():
                    if row[metric] is not None and row[metric] > limit:
                        issues.append({"attempt": f"{role}:{workload}:{process}", "status": "failed",
                                       "reason": f"{metric}={row[metric]} exceeds {limit}"})
                if role == "candidate":
                    metrics = row["metrics"]
                    failures = []
                    if metrics["score_copied_bytes"] != 0:
                        failures.append("dense score copy remains")
                    if workload.startswith("batch8-cold-") and metrics["bgr_conversions"] != metrics["iterations"]:
                        failures.append("cold batch must convert once per iteration")
                    if workload == "batch8-live-owner-roi320x180" and metrics["mapping_copies"] != metrics["iterations"]:
                        failures.append("live mapping owner must avoid duplicate ROI copies")
                    if len(cohorts["baseline"]) == 5 and definition["category"] == "selection" and definition["improvement"]:
                        reference = min(item["metrics"]["score_offsets_scanned"] for item in cohorts["baseline"])
                        if metrics["score_offsets_scanned"] >= reference:
                            failures.append("selection offsets did not decrease")
                    for reason in failures:
                        issues.append({"attempt": f"{role}:{workload}:{process}", "status": "failed", "reason": reason})
        if not complete:
            issues.append({"attempt": workload, "status": "incomplete", "reason": "five complete processes per role required"})
        if definition["improvement"] and all(len(rows) == 5 for rows in cohorts.values()):
            if complete:
                benefits[workload] = {}
            for metric, allowance in envelope["benefit_allowances"].items():
                if any(row[metric] is None for rows in cohorts.values() for row in rows):
                    continue
                baseline = statistics.median(row[metric] for row in cohorts["baseline"])
                candidate = statistics.median(row[metric] for row in cohorts["candidate"])
                passed = baseline - candidate > allowance
                if complete:
                    benefits[workload][metric] = {"baseline_median": baseline, "candidate_median": candidate,
                                                 "allowance": allowance, "passed": passed}
                if not passed:
                    issues.append({"attempt": workload, "status": "failed", "reason": f"{metric} benefit does not exceed frozen noise"})
    return issues, benefits


def report(directory):
    directory = Path(directory)
    evidence = evaluate(directory)
    mode = evidence["plan"]["mode"]
    issues, benefits = list(evidence["issues"]), {}
    if mode == "calibration" and not issues:
        try:
            derive_envelopes(evidence)
        except ValueError as error:
            issues.append({"attempt": "calibration", "status": "failed", "reason": str(error)})
    if mode == "comparison":
        try:
            profile_path = directory / "profile.json"
            require(sha256(profile_path) == evidence["plan"].get("profile_sha256"), "frozen profile hash mismatch")
            profile = validate_profile(load(profile_path))
            issues, benefits = compare_evidence(evidence, profile)
        except (OSError, ValueError, KeyError, TypeError) as error:
            issues.append({"attempt": "profile", "status": "failed", "reason": str(error)})
    status = "failed" if any(item["status"] == "failed" for item in issues) else "incomplete" if issues else "passed"
    target = evidence["plan"]["target"]
    qualified = mode == "comparison" and status == "passed"
    return {"format_version": 1, "mode": mode, "target": target, "status": status,
            "performance_qualified": qualified, "whole_change_acceptance": "incomplete",
            "targets": {name: (status if mode == "comparison" else "incomplete") if name == target else "not-run"
                        for name in TARGETS},
            "plan_sha256": evidence["plan_sha256"], "issues": issues, "benefits": benefits,
            "attempts": evidence["attempts"],
            "process_observations": evidence["observations"],
            "note": "smoke and calibration do not establish performance qualification; contract rows are non-statistical"}


def command(binary, task, mode):
    if task["workload"] is None:
        return [binary["path"], "--list"]
    result = [binary["path"], "--workload", task["workload"]]
    if mode == "smoke":
        result.append("--smoke")
    return result


@contextlib.contextmanager
def exclusive_collector():
    path = Path(tempfile.gettempdir()) / "mado-template-matching-efficiency.lock"
    try:
        stream = path.open("x", encoding="utf-8")
    except FileExistsError as error:
        raise ValueError(f"collector lock exists: {path}; inspect its owner before removal") from error
    try:
        with stream:
            stream.write(f"pid={os.getpid()}\n")
        yield
    finally:
        path.unlink()


def collect(args, mode):
    host = host_identity()
    require(host["target"] == args.target, "actual host cannot execute the selected qualification target")
    baseline = binary_identity(args.baseline, args.baseline_metadata, args.target)
    binaries = {"baseline": baseline}
    profile, profile_bytes = None, None
    if mode == "comparison":
        profile_path = Path(args.profile).resolve(strict=True)
        digest(args.profile_sha256, "profile_sha256")
        require(sha256(profile_path) == args.profile_sha256, "reviewed profile hash mismatch")
        profile_bytes = profile_path.read_bytes()
        profile = validate_profile(strict_json(profile_bytes.decode("utf-8")))
        require(host == profile["host"] and environment() == profile["environment"], "host/environment differs from reviewed profile")
        require(baseline == profile["baseline"], "baseline differs from reviewed profile")
        require(collector_identity() == profile["collector"], "collector differs from reviewed profile")
        binaries["candidate"] = binary_identity(args.candidate, args.candidate_metadata, args.target)
    selected = [args.workload] if mode == "smoke" and args.workload else list(DEFINITIONS)
    destination = Path(args.output).resolve()
    if profile:
        calibration = Path(profile["calibration"]["path"]).resolve()
        require(destination != calibration and calibration not in destination.parents,
                "comparison must not write into calibration")
    with exclusive_collector():
        destination.mkdir(parents=True, exist_ok=False)
        (destination / "attempts").mkdir()
        (destination / "started").mkdir()
        if profile_bytes is not None:
            with (destination / "profile.json").open("xb") as stream:
                stream.write(profile_bytes)
        plan = {"format_version": 1, "kind": "campaign", "mode": mode, "target": args.target,
                "host": host, "collector": collector_identity(), "binaries": binaries,
                "environment": environment(), "policy": POLICY, "workloads": selected,
                "schedule": schedule(mode, selected), "exclusive_host": args.exclusive_host,
                "cwd": str(ROOT), "created_unix_ns": time.time_ns(),
                "profile_sha256": args.profile_sha256 if profile else None}
        save(destination / "plan.json", plan)
        plan_hash = sha256(destination / "plan.json")
        for task in plan["schedule"]:
            identity = binaries[task["role"]]
            argv = command(identity, task, mode)
            attempt = {"task": task, "argv": argv, "plan_sha256": plan_hash,
                       "identity_before": None, "identity_after": None, "identity_error": None,
                       "record": None, "started_unix_ns": time.time_ns()}
            save(destination / "started" / f"{task['id']}.json", attempt)
            try:
                check_files(identity)
                attempt["identity_before"] = identity
                attempt["record"] = run_process(argv, cwd=ROOT, env=plan["environment"],
                                                 timeout_seconds=POLICY["timeout_seconds"],
                                                 output_limit_bytes=POLICY["output_limit_bytes"],
                                                 cleanup_seconds=POLICY["cleanup_seconds"])
                check_files(identity)
                attempt["identity_after"] = identity
            except (OSError, ValueError) as error:
                attempt["identity_error"] = str(error)
            except BaseException as error:
                attempt["interruption"] = type(error).__name__
                try:
                    save(destination / "attempts" / f"{task['id']}.json", attempt)
                except BaseException:
                    # Evidence persistence must not replace the original interruption.
                    pass
                raise
            save(destination / "attempts" / f"{task['id']}.json", attempt)
            # A failed row is never retried or omitted. Unsafe cleanup stops remaining
            # processes, which stay visibly not-run in the immutable schedule.
            if attempt["record"] is None or not attempt["record"]["cleanup_ok"] or attempt["identity_error"]:
                break
            if task["workload"] is None:
                try:
                    require(successful(attempt["record"]), "manifest process failed")
                    manifest = manifest_identity(strict_json(attempt["record"]["stdout"]), args.target,
                                                 identity["build"]["opencv_version"])
                    if profile:
                        require(manifest == profile["manifest"], "manifest differs from frozen profile")
                except (ValueError, KeyError, TypeError):
                    # Invalid corpus/target discovery is not permission to measure a
                    # different matrix. The raw attempt and unrun schedule survive.
                    break
        result = report(destination)
        save(destination / "report.json", result)
        return result


def parser():
    result = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Build metadata JSON: target, binary_sha256, source_revision (40 hex), source_tree (40 hex), cargo_lock_sha256, rustc, build_argv (array), opencv_version, dependencies ([{path: absolute, sha256}]). Metadata attests the build; collector verifies the named executable/dependency bytes, never rebuilds or infers source from the current checkout.")
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("policy", help="print the predeclared workload/noise/envelope policy")
    for name, description in (("calibrate", "five baseline-only processes per row"),
                              ("compare", "separate alternating baseline5/candidate5 per row"),
                              ("smoke", "one short process per row; never performance qualification")):
        sub = commands.add_parser(name, help=description)
        sub.add_argument("--target", choices=TARGETS, required=True)
        sub.add_argument("--baseline", required=True, help="explicit prebuilt executable")
        sub.add_argument("--baseline-metadata", required=True, help="hash-bound build provenance JSON")
        sub.add_argument("--output", required=True, help="new evidence directory (must not exist)")
        sub.add_argument("--exclusive-host", action="store_true", required=name != "smoke",
                         help="attest no other builds/benchmarks run during collection; lock only excludes this collector")
        if name == "compare":
            sub.add_argument("--candidate", required=True)
            sub.add_argument("--candidate-metadata", required=True)
            sub.add_argument("--profile", required=True)
            sub.add_argument("--profile-sha256", required=True, help="reviewed, frozen profile digest")
        if name == "smoke":
            sub.add_argument("--workload", choices=list(DEFINITIONS))
    freeze = commands.add_parser("freeze", help="review calibration and exclusively write a numeric target profile")
    freeze.add_argument("--calibration", required=True)
    freeze.add_argument("--reviewer", required=True)
    freeze.add_argument("--rationale", required=True)
    freeze.add_argument("--output", required=True, help="new immutable profile JSON")
    validation = commands.add_parser("report", help="revalidate retained evidence without executing any binary")
    validation.add_argument("evidence")
    return result


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        if args.command == "policy":
            result = {"policy": POLICY, "workloads": list(DEFINITIONS.values())}
        elif args.command == "freeze":
            evidence = evaluate(args.calibration)
            result = profile_from(evidence, args.calibration, args.reviewer, args.rationale)
            output = Path(args.output).resolve()
            require(Path(args.calibration).resolve() not in output.parents,
                    "profile must be separate from frozen calibration")
            save(output, result)
            result = {"profile": str(output), "sha256": sha256(output), "target": result["target"]}
        elif args.command == "report":
            result = report(args.evidence)
        else:
            result = collect(args, {"calibrate": "calibration", "compare": "comparison", "smoke": "smoke"}[args.command])
        print(document(result).decode("utf-8"), end="")
        return 0 if result.get("status", "passed") == "passed" else 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"template-matching-efficiency: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
