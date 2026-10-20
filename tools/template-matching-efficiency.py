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
import shutil
import statistics
import sys
import tempfile
import time
from pathlib import Path
from pathlib import PureWindowsPath

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

# Diagnostic execution bounds do not alter the frozen v1 numerical policy.
SMOKE_LIMITS = {"child_timeout_seconds": 90, "campaign_timeout_seconds": 900,
                "cleanup_seconds": 5, "output_limit_bytes": 8 * MIB}
DIRECT_FORMAT_WORKLOAD = "contract-roi-format-transition"
DIRECT_FORMAT_CASES = (
    ("rgba-full", "rgba8", 0, False, False),
    ("bgra-full", "bgra8", 0, False, False),
    ("rgba-padded-full", "rgba8", 16, False, False),
    ("bgra-padded-full", "bgra8", 16, False, False),
    ("rgba-roi", "rgba8", 0, True, False),
    ("bgra-roi", "bgra8", 0, True, False),
    ("rgba-padded-roi", "rgba8", 16, True, False),
    ("bgra-padded-roi", "bgra8", 16, True, False),
    ("rgba-forced-bgra-full", "rgba8", 0, False, True),
    ("rgba-forced-bgra-roi", "rgba8", 0, True, True),
)
DIRECT_FORMAT_WORK = {"backend_find_calls": 18, "bgr_conversions": 17,
                      "bgr_converted_bytes": 57_346_368, "mapping_copies": 10,
                      "mapping_copied_bytes": 10_337_024}
PERSISTENT_ORDER = ("mapping-live", "cold-result", "persistent-hit", "mapping-released",
                    "frame-released", "result-released", "final-backend-released")
ACTIVE_ORDER = ("prepared-reader-paused", "active-image-evicted", "final-reader-completed",
                "frames-released", "results-released", "final-backend-released")


def direct_format_contract():
    cases = []
    for name, source, padding, roi, forced in DIRECT_FORMAT_CASES:
        pixels = 320 * 180 if roi else 1920 * 1080
        cases.append({"id": name, "source_format": source,
                      "selected_format": "bgra8" if forced else source,
                      "row_padding_bytes": padding, "nonzero_roi": roi,
                      "forced_bgra_public_mapping": forced,
                      "expected_matches": 1, "expected_bounds": [80, 64, 92, 74],
                      "expected_mapping_copies": int(roi or forced),
                      "expected_mapping_copied_bytes": pixels * 4 if roi or forced else 0,
                      "expected_bgr_conversions": 1, "expected_bgr_converted_bytes": pixels * 3})
    transitions = [
        {"id": "same-source-roi", "selected_format": "rgba8", "region": [64, 48, 384, 228], "expected_bounds": [80, 64, 92, 74],
         "mapping_copies": 1, "mapping_copied_bytes": 230400, "bgr_conversions": 1, "bgr_converted_bytes": 172800},
        {"id": "same-source-alternate-roi", "selected_format": "rgba8", "region": [72, 56, 376, 220], "expected_bounds": [152, 64, 164, 74],
         "mapping_copies": 1, "mapping_copied_bytes": 199424, "bgr_conversions": 1, "bgr_converted_bytes": 149568},
        {"id": "distinct-bgra-full", "selected_format": "bgra8", "region": [0, 0, 1920, 1080], "expected_bounds": [80, 64, 92, 74],
         "mapping_copies": 0, "mapping_copied_bytes": 0, "bgr_conversions": 1, "bgr_converted_bytes": 6220800},
    ]
    return {"cases": cases, "transition_cases": transitions, "lifetime_order": ["persistent-hit", "active-evicted"],
            "persistent_checkpoint_order": list(PERSISTENT_ORDER),
            "active_checkpoint_order": list(ACTIVE_ORDER),
            "expected_per_iteration": dict(DIRECT_FORMAT_WORK),
            "operation_timeout_seconds": 5, "prepared_reader_pause_timeout_seconds": 5,
            "resource_scope": "logical mapping owners and owned BGR payload; not total native allocation or RSS release",
            "active_owner_order": "final production reader completes before final backend release"}


def native_path(path, target):
    path = nonempty(path, "loaded runtime path")
    require((PureWindowsPath(path) if target == TARGETS[1] else Path(path)).is_absolute(),
            "loaded runtime path must be absolute")
    return path.replace("\\", "/").removeprefix("//?/").casefold() if target == TARGETS[1] else path


def loaded_runtime(value, identity=None, *, verify_files=False):
    target = value.get("target")
    runtime = value.get("loaded_native_runtime")
    require(isinstance(runtime, dict), "actual loaded native runtime identity required")
    require(runtime.get("method") == ("dyld-loaded-image-paths" if target == TARGETS[0]
                                      else "K32EnumProcessModules-GetModuleFileNameW"),
            "loaded runtime path evidence method mismatch")
    images = runtime.get("images")
    require(isinstance(images, list) and 0 < len(images) <= 4096, "bounded loaded runtime images required")
    actual = {}
    for image in images:
        require(isinstance(image, dict), "invalid loaded image")
        path = native_path(image.get("path"), target)
        require(path not in actual, "duplicate loaded runtime image")
        actual[path] = digest(image.get("sha256"), "loaded runtime sha256")
        if verify_files:
            require(sha256(image["path"]) == actual[path], "actual loaded runtime bytes changed")
    require(any("opencv_core" in path.rsplit("/", 1)[-1].lower()
                or "opencv_world" in path.rsplit("/", 1)[-1].lower() for path in actual),
            "actual loaded OpenCV core/world path required")
    if identity is not None:
        expected = {native_path(item["path"], target): item["sha256"]
                    for item in identity["build"]["dependencies"]}
        require(actual == expected, "actual loaded runtime path/hash differs from build attestation")
    return runtime


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
    loaded_runtime(value)
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
        if expected["id"] == DIRECT_FORMAT_WORKLOAD:
            require(row.get("format_contract") == direct_format_contract(),
                    "direct-format declaration/order/work mismatch")
        frozen.append(dict(row))
    return {"fixture_sha256": fixture, "target": target, "opencv_version": opencv_version,
            "workloads": frozen}


def observations(value, manifest, workload, smoke):
    require(isinstance(value, dict) and value.get("format_version") == 1
            and value.get("kind") == "observations", "invalid observation protocol")
    for key in ("target", "opencv_version", "fixture_sha256"):
        require(value.get(key) == manifest[key], f"observation {key} mismatch")
    loaded_runtime(value)
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
    if workload == DIRECT_FORMAT_WORKLOAD:
        validate_direct_formats(row, warmups + count)
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


def validate_direct_formats(row, iterations):
    observations = row.get("format_observations")
    require(isinstance(observations, list) and len(observations) == iterations,
            "all warmup/retained format observations required")
    declared = direct_format_contract()
    for observed in observations:
        require(observed.get("correct") is True, "incorrect direct-format observation")
        cases = observed.get("cases")
        require(isinstance(cases, list) and [item.get("id") for item in cases] ==
                [item["id"] for item in declared["cases"]], "direct-format case order/count mismatch")
        for case, expected in zip(cases, declared["cases"]):
            require(case.get("correct") is True and case.get("source_format") == expected["source_format"]
                    and case.get("selected_format") == expected["selected_format"], "actual selected format mismatch")
            source_stride = 1920 * 4 + expected["row_padding_bytes"]
            mapped_stride = 320 * 4 if expected["nonzero_roi"] else 1920 * 4 if expected["forced_bgra_public_mapping"] else source_stride
            require(case.get("source_stride") == source_stride and case.get("mapped_stride") == mapped_stride
                    and case.get("mapping_shared") is (not expected["nonzero_roi"] and not expected["forced_bgra_public_mapping"]),
                    "actual padded/shared/packed mapping layout mismatch")
            for key in ("mapping_copies", "mapping_copied_bytes", "bgr_conversions", "bgr_converted_bytes"):
                require(integer(case.get("work", {}).get(key), key) == expected[f"expected_{key}"],
                        f"direct-format case work mismatch: {case['id']}/{key}")
        transitions = observed.get("transitions")
        require(isinstance(transitions, list) and len(transitions) == 3, "existing ROI/format transitions missing")
        for actual, expected in zip(transitions, declared["transition_cases"]):
            require(actual.get("correct") is True and all(actual.get(key) == expected[key]
                    for key in ("id", "selected_format", "region")), "ROI/format transition order/outcome mismatch")
            require(actual.get("work") == {key: expected[key] for key in
                    ("mapping_copies", "mapping_copied_bytes", "bgr_conversions", "bgr_converted_bytes")},
                    "ROI/format transition work mismatch")
        require(observed.get("work") == {key: value for key, value in DIRECT_FORMAT_WORK.items()
                                        if key != "backend_find_calls"}
                and observed.get("backend_find_calls") == 18
                and observed.get("backend_find_completions") == 18, "direct-format iteration work mismatch")
        lifetimes = observed.get("lifetimes")
        require(isinstance(lifetimes, list) and [item.get("id") for item in lifetimes] ==
                declared["lifetime_order"], "lifetime observation order/count mismatch")
        for lifetime, order, held, evicted, active in (
            (lifetimes[0], PERSISTENT_ORDER, (0, 172800, 172800, 172800, 172800, 172800, 0),
             (0,) * 7, (0,) * 7),
            (lifetimes[1], ACTIVE_ORDER, (6220800, 12441600, 12441600, 12441600, 12441600, 0),
             (0, 6220800, 0, 0, 0, 0), (1, 1, 0, 0, 0, 0)),
        ):
            checkpoints = lifetime.get("checkpoints")
            require(isinstance(checkpoints, list) and [point.get("checkpoint") for point in checkpoints] ==
                    list(order), "lifetime checkpoint order/count mismatch")
            for point, cache_bytes, evicted_bytes, finds in zip(checkpoints, held, evicted, active):
                for name in ("cache_held_bgr_bytes", "active_evicted_bgr_bytes", "active_backend_finds"):
                    integer(point.get(name), name)
                require(point.get("cache_held_bgr_bytes") == cache_bytes
                        and point.get("active_evicted_bgr_bytes") == evicted_bytes
                        and point.get("active_backend_finds") == finds, "finite BGR/active-reader checkpoint mismatch")
            mapping = (True, True, True, False, False, False, False) if lifetime["id"] == "persistent-hit" else (True, True, False, False, False, False)
            sources = (True, True, True, True, False, False, False) if lifetime["id"] == "persistent-hit" else (True, True, True, False, False, False)
            results = (False, True, True, True, True, False, False) if lifetime["id"] == "persistent-hit" else (False, False, True, True, False, False)
            for point, mapped, source, result in zip(checkpoints, mapping, sources, results):
                require(point.get("mapping_owner_live") is mapped and point.get("source_frame_alive") is source
                        and point.get("result_owner_live") is result, "logical owner checkpoint mismatch")
    for key, per_iteration in DIRECT_FORMAT_WORK.items():
        require(integer(row["metrics"].get(key), key) == per_iteration * iterations,
                f"direct-format aggregate work mismatch: {key}")
    require(row["metrics"].get("final_owner_correct") is True, "direct-format final owner failure")


def validate_plan(plan):
    require(plan.get("format_version") == 1 and plan.get("kind") == "campaign", "invalid campaign")
    require(plan.get("mode") in ("calibration", "comparison", "smoke"), "invalid cohort")
    require(plan.get("policy") == POLICY, "policy differs from predeclared collector policy")
    if plan["mode"] == "smoke":
        require(plan.get("execution_limits") == SMOKE_LIMITS, "bounded smoke execution policy mismatch")
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
            if plan["mode"] == "smoke":
                timeout = attempt.get("timeout_seconds")
                require(type(timeout) in (int, float) and math.isfinite(timeout)
                        and 0 < timeout <= SMOKE_LIMITS["child_timeout_seconds"],
                        "smoke child deadline missing or relaxed")
            value = strict_json(record["stdout"])
            loaded_runtime(value, binary)
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
        if mode == "smoke":
            plan["execution_limits"] = SMOKE_LIMITS
        save(destination / "plan.json", plan)
        plan_hash = sha256(destination / "plan.json")
        deadline = time.monotonic() + SMOKE_LIMITS["campaign_timeout_seconds"] if mode == "smoke" else None
        for task in plan["schedule"]:
            remaining = None if deadline is None else deadline - time.monotonic() - SMOKE_LIMITS["cleanup_seconds"]
            if remaining is not None and remaining <= 0:
                break
            identity = binaries[task["role"]]
            argv = command(identity, task, mode)
            attempt = {"task": task, "argv": argv, "plan_sha256": plan_hash,
                       "identity_before": None, "identity_after": None, "identity_error": None,
                       "record": None, "started_unix_ns": time.time_ns()}
            save(destination / "started" / f"{task['id']}.json", attempt)
            try:
                check_files(identity)
                attempt["identity_before"] = identity
                if deadline is not None:
                    remaining = deadline - time.monotonic() - SMOKE_LIMITS["cleanup_seconds"]
                    require(remaining > 0, "campaign deadline reached before child launch")
                timeout = POLICY["timeout_seconds"] if remaining is None else min(SMOKE_LIMITS["child_timeout_seconds"], remaining)
                attempt["timeout_seconds"] = timeout
                attempt["record"] = run_process(argv, cwd=ROOT, env=plan["environment"],
                                                 timeout_seconds=timeout,
                                                 output_limit_bytes=POLICY["output_limit_bytes"],
                                                 cleanup_seconds=POLICY["cleanup_seconds"])
                check_files(identity)
                attempt["identity_after"] = identity
                if successful(attempt["record"]):
                    loaded_runtime(strict_json(attempt["record"]["stdout"]), identity, verify_files=True)
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


def attest(args):
    """Bind an explicit prebuilt executable to its actual loaded OpenCV images."""
    require(not Path(args.output).exists() and not Path(str(args.output) + ".probe.json").exists(),
            "attestation destination already exists")
    require(host_identity()["target"] == args.target, "attestation must run on the actual native target")
    binary = Path(args.binary).resolve(strict=True)
    binary_hash = sha256(binary)
    git, rustc = shutil.which("git"), shutil.which("rustc")
    require(git is not None and rustc is not None, "attestation requires git and rustc on PATH")
    git, rustc = str(Path(git).absolute()), str(Path(rustc).absolute())
    commands = []
    try:
        for argv in ([str(binary), "--list"], [git, "rev-parse", "HEAD"],
                     [git, "rev-parse", "HEAD^{tree}"], [git, "status", "--porcelain"],
                     [rustc, "-vV"]):
            record = run_process(argv, cwd=ROOT, env=environment(),
                                 timeout_seconds=SMOKE_LIMITS["child_timeout_seconds"],
                                 output_limit_bytes=SMOKE_LIMITS["output_limit_bytes"],
                                 cleanup_seconds=SMOKE_LIMITS["cleanup_seconds"])
            commands.append({"argv": argv, "record": record})
            require(successful(record), f"attestation command failed: {argv}")
    finally:
        save(str(args.output) + ".probe.json", {"commands": commands})
    value = strict_json(commands[0]["record"]["stdout"])
    runtime = loaded_runtime(value, verify_files=True)
    manifest_identity(value, args.target, value["opencv_version"])
    require(sha256(binary) == binary_hash, "binary changed during runtime probe")
    build = {"target": args.target, "source_revision": commands[1]["record"]["stdout"].strip(),
             "source_tree": commands[2]["record"]["stdout"].strip(),
             "source_status_porcelain": commands[3]["record"]["stdout"],
             "binary_sha256": binary_hash, "cargo_lock_sha256": sha256(ROOT / "Cargo.lock"),
             "rustc": commands[4]["record"]["stdout"], "build_argv": strict_json(args.build_argv_json),
             "opencv_version": value["opencv_version"], "dependencies": runtime["images"],
             "loaded_native_runtime": runtime, "provenance_commands": commands,
             "attestation_scope": "operator-supplied build command; source commit/tree and dirty status observed, not a reproducible build proof"}
    validate_build(build, args.target)
    save(args.output, build)
    return {"metadata": str(Path(args.output).resolve()), "sha256": sha256(args.output)}


def parser():
    result = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Build metadata JSON: target, binary_sha256, source_revision (40 hex), source_tree (40 hex), cargo_lock_sha256, rustc, build_argv (array), opencv_version, dependencies ([{path: absolute, sha256}]). Metadata attests the build; collector verifies the named executable/dependency bytes, never rebuilds or infers source from the current checkout.")
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("policy", help="print the predeclared workload/noise/envelope policy")
    binding = commands.add_parser("attest", help="observe a prebuilt executable's actual loaded OpenCV paths; never build")
    binding.add_argument("--target", choices=TARGETS, required=True)
    binding.add_argument("--binary", required=True)
    binding.add_argument("--build-argv-json", required=True, help="exact completed build command as a JSON array")
    binding.add_argument("--output", required=True, help="new build metadata JSON")
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
            result = {"policy": POLICY, "workloads": list(DEFINITIONS.values()),
                      "smoke_execution_limits": SMOKE_LIMITS, "direct_format_contract": direct_format_contract()}
        elif args.command == "attest":
            result = attest(args)
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
