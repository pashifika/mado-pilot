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
from pathlib import PurePosixPath, PureWindowsPath

ROOT = Path(__file__).resolve().parents[1]
RUNNER_DIR = ROOT / "tools/native-release-profile"
sys.path.insert(0, str(RUNNER_DIR))
from process_runner import run_process  # noqa: E402
import macos_image_observer as macos_images  # noqa: E402

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

# Prospective component apparatus only; this never regrades format-one evidence.
V2_POLICY = {**POLICY, "id": "template-efficiency-v2-prospective", "processes": 6,
             "control_pairs": 6, "comparison_pairs": 6,
             "pair_order": ["AB", "BA", "AB", "BA", "AB", "BA"],
             "timing_ceiling": "ceil(max(calibration6) + max(spread, 2 * clock_allowance))",
             "benefit": "normal median(baseline6) - median(candidate6) > frozen allowance, jointly p95 and CPU",
             "noise": "spread / max(median(cohort6), 2 * clock_allowance) <= noise_cap",
             "resource_ceiling": "max(calibration6) + max(spread, fixed slack); live cache only on native RSS peak",
             "control": "all processes within ceilings; order and half-cohort median differences <= allowance",
             "scope": "component only; no actual-consumer coverage or accepted numerical qualification"}
NORMAL_UNAVAILABLE_METRICS = (
    "allocations", "heap_before_bytes", "heap_after_bytes", "heap_after_drop_bytes",
    "heap_steady_before_bytes", "heap_steady_after_bytes", "peak_heap_bytes", "peak_heap_scope",
    "score_copied_bytes", "score_offsets_scanned", "bgr_conversions", "bgr_converted_bytes",
    "bgr_retained_bytes", "bgr_retained_peak_bytes", "bgr_retained_after_drop_bytes",
    "bgr_evicted_bytes", "bgr_evicted_peak_bytes", "bgr_evicted_after_drop_bytes",
    "mapping_copies", "mapping_copied_bytes", "logical_mapped_bytes", "backend_find_calls",
    "backend_find_failures", "peak_active_finds", "peak_active_finds_scope", "final_owner_correct",
)
V2_COMMON_METRICS = (
    "iterations", "cpu_ns", "cpu_reason", "native_resource_reason",
    "peak_rss_bytes", "rss_before_bytes", "rss_after_bytes", "rss_after_drop_bytes",
    "rss_steady_before_bytes", "rss_steady_after_bytes", "private_after_bytes", "footprint_after_bytes",
    "private_reason", "footprint_reason", "native_allocation_scope", "peak_rss_scope",
    "mapping_owner_live_before_drop",
)
NORMAL_UNAVAILABLE_WORKLOADS = (
    "contract-roi-format-transition", "watch-analysis-always", "watch-exact-rgba",
)


def definitions_for(build_mode):
    require(build_mode in ("normal", "instrumented"), "v2 requires --build-mode normal|instrumented")
    return {key: value for key, value in DEFINITIONS.items()
            if build_mode == "instrumented" or key not in NORMAL_UNAVAILABLE_WORKLOADS}


def mode_identity(value, build_mode):
    definitions_for(build_mode)
    require(value.get("format_version") == 2 and value.get("build_mode") == build_mode,
            "compiled build mode/protocol mismatch")
    require(value.get("global_allocator") == (
        "system" if build_mode == "normal" else "accounting-over-system"), "allocator mode mismatch")
    unavailable = value.get("unavailable")
    require(isinstance(unavailable, dict), "mode unavailable declaration missing")
    reason = nonempty(unavailable.get("reason"), "unavailable reason") if build_mode == "normal" else None
    expected = ({"reason": reason, "metrics": list(NORMAL_UNAVAILABLE_METRICS),
                 "workloads": list(NORMAL_UNAVAILABLE_WORKLOADS)} if build_mode == "normal"
                else {"reason": None, "metrics": [], "workloads": []})
    require(value.get("unavailable") == expected, "mode unavailable declaration mismatch")
    return {key: value[key] for key in ("format_version", "build_mode", "global_allocator", "unavailable")}


def execution_limits(mode, slots):
    require(mode in ("smoke", "calibration", "control", "comparison"), "invalid v2 cohort")
    return {"child_timeout_seconds": 90 if mode == "smoke" else 300,
            "campaign_timeout_seconds": 900 if mode == "smoke" else 21600 if mode == "calibration" else 43200,
            "cleanup_seconds": 5, "output_limit_bytes": 8 * MIB,
            "runtime_binding_seconds": 0 if mode == "smoke" else 120,
            "max_attempts": slots, "campaign_output_limit_bytes": slots * 8 * MIB}


def validate_v2_build(identity, target, build_mode):
    build = identity["build"]
    validate_build(build, target)
    require(build.get("protocol_version") == 2 and build.get("build_mode") == build_mode,
            "v2 build attestation mode/protocol mismatch")
    probe = build.get("provenance_commands")
    require(isinstance(probe, list) and len(probe) == 5 and all(
        isinstance(item, dict) and isinstance(item.get("record"), dict)
        and successful(item["record"]) for item in probe),
        "v2 source-bound manifest/source probes required")
    require(probe[0].get("argv") == [identity["path"], "--protocol-version", "2", "--list"]
            and successful(probe[0].get("record", {})), "v2 attested manifest command mismatch")
    for index, key in ((1, "source_revision"), (2, "source_tree"), (3, "source_status_porcelain"), (4, "rustc")):
        actual = probe[index]["record"]["stdout"]
        require(build.get(key) == (actual.strip() if index in (1, 2) else actual),
                f"attested source probe differs: {key}")
    require([item["argv"][1:] for item in probe[1:]] ==
            [["rev-parse", "HEAD"], ["rev-parse", "HEAD^{tree}"], ["status", "--porcelain"], ["-vV"]],
            "source provenance command mismatch")
    observed = strict_json(probe[0]["record"]["stdout"])
    frozen = manifest_identity(observed, target, build["opencv_version"], 2, build_mode)
    require(build.get("benchmark_manifest") == frozen, "attested source/mode manifest mismatch")
    loaded_runtime(observed, identity)
    return frozen


OPENCV_THREAD_METHOD = "opencv-runtime-backend-and-thread-setting-v1"


def effective_threads(value):
    observation = value.get("opencv_parallel_threads")
    require(isinstance(observation, dict) and observation.get("method") == OPENCV_THREAD_METHOD,
            "effective OpenCV runtime backend/thread observation missing")
    before, after = observation.get("before"), observation.get("after")
    require(isinstance(before, dict) and before == after and observation.get("changed") is False,
            "effective OpenCV thread setting changed or unavailable")
    require(set(before) == {"threads", "framework", "reason"}
            and type(before.get("threads")) is int and before["threads"] > 0
            and before.get("reason") is None, "effective OpenCV thread setting unavailable")
    framework = nonempty(before.get("framework"), "OpenCV parallel framework")
    require(framework.casefold() != "gcd", "GCD does not expose a supported OpenCV thread setting")
    return before


def validate_runtime_bundle(bundle, identity, target, build_mode):
    """Revalidate retained representative runs; no live target or dependency reads."""
    require(isinstance(bundle, dict) and bundle.get("kind") == "component-runtime-preflight"
            and bundle.get("format_version") == 2, "runtime preflight bundle missing")
    plan = bundle.get("plan")
    require(isinstance(plan, dict) and plan.get("target") == target
            and plan.get("build_mode") == build_mode, "runtime preflight target/mode mismatch")
    base = plan.get("binary")
    require(isinstance(base, dict) and base.get("path") == identity["path"]
            and base.get("sha256") == identity["sha256"]
            and base.get("build") == {key: value for key, value in identity["build"].items()
                                     if key != "runtime_image_preflight"},
            "runtime preflight executable/source binding mismatch")
    selected = list(definitions_for(build_mode))
    expected_schedule = schedule("smoke", selected, 2)
    limits = {**execution_limits("smoke", len(expected_schedule)), "runtime_binding_seconds": 120}
    require(plan.get("schedule") == expected_schedule and plan.get("execution_limits") == limits,
            "runtime preflight selected paths or bounds differ")
    require(plan.get("collector") == collector_identity()
            and plan.get("observer") == macos_images.method_identity(),
            "runtime preflight observer/source changed")
    require(isinstance(plan.get("environment"), dict) and isinstance(plan.get("host"), dict)
            and plan["host"].get("target") == target, "runtime preflight host/environment missing")
    runs = bundle.get("attempts")
    require(isinstance(runs, list) and len(runs) == len(expected_schedule),
            "runtime preflight has unrun paths")
    manifest = validate_v2_build(base, target, build_mode)
    threads = effective_threads(strict_json(base["build"]["provenance_commands"][0]["record"]["stdout"]))
    receipts = []
    for task, attempt in zip(expected_schedule, runs):
        require(isinstance(attempt, dict) and attempt.get("task") == task
                and attempt.get("argv") == command(base, task, "smoke", 2),
                "runtime preflight path/order mismatch")
        require(attempt.get("identity_before") == base and attempt.get("identity_after") == base
                and attempt.get("identity_error") is None and attempt.get("interruption") is None,
                "runtime preflight file identity or completion failed")
        before, after = attempt.get("campaign_elapsed_before"), attempt.get("campaign_elapsed_after")
        timeout = attempt.get("timeout_seconds")
        require(all(type(item) in (int, float) and math.isfinite(item) for item in (before, after, timeout))
                and 0 <= before <= after <= limits["campaign_timeout_seconds"]
                and 0 < timeout <= limits["child_timeout_seconds"]
                and timeout + limits["runtime_binding_seconds"] + limits["cleanup_seconds"]
                <= limits["campaign_timeout_seconds"] - before,
                "runtime preflight deadline missing or exceeded")
        receipt = attempt.get("receipt")
        require(isinstance(receipt, dict) and receipt.get("method_identity") == plan["observer"],
                "runtime preflight receipt source mismatch")
        record = receipt.get("record")
        require(isinstance(record, dict) and successful(record), "runtime preflight process or cleanup failed")
        duration = receipt.get("duration_seconds")
        require(type(duration) in (int, float) and math.isfinite(duration) and 0 <= duration
                <= timeout + limits["cleanup_seconds"] + limits["runtime_binding_seconds"],
                "runtime preflight observer exceeded its reserved execution budget")
        value = strict_json(record["stdout"])
        loaded_runtime(value, base)
        require(effective_threads(value) == threads, "runtime preflight native thread setting changed")
        if task["workload"] is None:
            require(manifest_identity(value, target, base["build"]["opencv_version"], 2, build_mode) == manifest,
                    "runtime preflight manifest changed")
        else:
            observations(value, manifest, task["workload"], True)
        receipts.append(receipt)
    derived = macos_images.validate_closure(receipts)
    require(derived["ok"], "runtime preflight closure incomplete: " + str(derived["reason"]))
    require(bundle.get("closure") == derived["closure"] and bundle.get("threads") == threads,
            "runtime preflight closure/thread summary mismatch")
    require(derived["closure"]["target"] == target
            and derived["closure"]["main_executable"] == identity["path"],
            "runtime preflight main identity mismatch")
    required_files = {native_path(row["path"], target): row["sha256"] for row in [
        {"path": identity["path"], "sha256": identity["sha256"]}, *identity["build"]["dependencies"]]}
    for receipt in receipts:
        files = {native_path(row["path"], target): row["sha256"]
                 for row in receipt["images"] if row["class"] == "file"}
        require(all(files.get(path) == digest for path, digest in required_files.items()),
                "runtime preflight path lacks the attested executable/dependency bytes")
    expected_loader_env = {key: value for key, value in plan["environment"].items() if key.startswith("DYLD_")}
    expected_loader_env["DYLD_PRINT_LIBRARIES"] = "1"
    require(derived["closure"]["loader_env"] == expected_loader_env, "runtime preflight loader environment differs")
    return {"closure": derived["closure"], "threads": threads, "plan": plan}


def validate_runtime_attempt(attempt, runtime):
    receipt = attempt.get("runtime_observation")
    require(isinstance(receipt, dict) and "record" not in receipt
            and receipt.get("method_identity") == runtime["plan"]["observer"],
            "measurement-time runtime observation missing or changed")
    verdict = macos_images.validate_observation({**receipt, "record": attempt["record"]}, runtime["closure"])
    require(verdict["ok"] and verdict["cleanup_ok"],
            "measurement-time runtime observation incomplete: " + str(verdict["reason"]))
    expected_paths = {row["path"] for row in runtime["plan"]["binary"]["build"]["dependencies"]}
    expected_paths.add(runtime["plan"]["binary"]["path"])
    require(expected_paths <= {row["path"] for row in receipt["images"]},
            "measurement-time loader inventory omits an attested image")
    duration = receipt.get("duration_seconds")
    require(type(duration) in (int, float) and math.isfinite(duration) and 0 <= duration
            <= attempt["timeout_seconds"] + attempt["cleanup_seconds"] + 2 * attempt["runtime_binding_seconds"],
            "runtime observer exceeded its reserved execution budget")
    require(effective_threads(strict_json(attempt["record"]["stdout"])) == runtime["threads"],
            "measurement-time native thread setting changed")


def runtime_closure_admission(identity, target, build_mode, bundle_path=None):
    validate_v2_build(identity, target, build_mode)
    reference = identity["build"].get("runtime_image_preflight")
    require(target == TARGETS[0] and isinstance(reference, dict),
            "missing runtime closure prerequisite: full native image preflight required; zero launches")
    require(identity["build"].get("source_status_porcelain") == "",
            "dirty source is not a qualification artifact; zero launches")
    digest(reference.get("sha256"), "runtime preflight sha256")
    native_path(reference.get("path"), target)
    with Path(reference["path"] if bundle_path is None else bundle_path).open("rb") as stream:
        raw = stream.read(64 * MIB + 1)
    require(len(raw) <= 64 * MIB and hashlib.sha256(raw).hexdigest() == reference["sha256"],
            "runtime preflight size/hash mismatch")
    result = validate_runtime_bundle(strict_json(raw.decode("utf-8")), identity, target, build_mode)
    return {**result, "raw": raw}


def reviewed_method(path, expected_sha256, plan):
    digest(expected_sha256, "method_review_sha256")
    require(path is not None and sha256(path) == expected_sha256, "method review hash mismatch")
    review = load(path)
    require(isinstance(review, dict) and review.get("format_version") == 2
            and review.get("kind") == "component-method-review",
            "prospective source/host/cost method review required")
    require(review.get("decision") == "accepted", "method review has not accepted this lane")
    for key in ("target", "build_mode", "host", "environment", "collector", "policy", "workloads"):
        require(review.get(key) == plan[key], f"method review {key} mismatch")
    require(review.get("baseline") == plan["binaries"]["baseline"], "method review baseline/source mismatch")
    if "candidate" in plan["binaries"]:
        require(review.get("candidate") == plan["binaries"]["candidate"],
                "method review candidate/source mismatch")
    for key in ("reviewer", "rationale", "source_host_cost_review", "runtime_observer_eligibility_review"):
        nonempty(review.get(key), key)
    require(review.get("stage_limits") == {
        mode: execution_limits(mode, len(schedule(mode, plan["workloads"], 2)))
        for mode in ("calibration", "control", "comparison")}, "method review finite cost/bounds mismatch")
    return {"path": str(Path(path).resolve()), "sha256": expected_sha256}
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
    require((PureWindowsPath(path) if target == TARGETS[1] else PurePosixPath(path)).is_absolute(),
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
             RUNNER_DIR / "_process_group.py", RUNNER_DIR / "_windows_process.py",
             RUNNER_DIR / "macos_image_observer.py")
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
        path = native_path(dependency.get("path"), target)
        require(path not in paths, "dependency paths must be unique and absolute")
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


def manifest_identity(value, target, opencv_version, protocol_version=1, build_mode=None):
    require(isinstance(value, dict) and value.get("format_version") == protocol_version, "invalid protocol version")
    mode_fields = mode_identity(value, build_mode) if protocol_version == 2 else {}
    if protocol_version == 2:
        effective_threads(value)
    require(value.get("kind") == "manifest", "expected manifest")
    require(value.get("target") == target, "manifest target mismatch")
    require(value.get("opencv_version") == opencv_version, "manifest OpenCV identity mismatch")
    loaded_runtime(value)
    fixture = digest(value.get("fixture_sha256"), "fixture_sha256")
    if protocol_version == 2:
        corpus = value.get("fixture")
        require(isinstance(corpus, dict) and corpus.get("license") == "repository-first-party",
                "source-bound synthetic fixture declaration required")
        for section, keys in (("direct_formats", ("helper_source_sha256", "fixture_source_sha256")),
                              ("watcher", ("helper_source_sha256", "generator_source_sha256"))):
            for key in keys:
                digest(corpus.get(section, {}).get(key), f"fixture.{section}.{key}")
        mode_fields["fixture"] = corpus
    rows = value.get("workloads")
    definitions = definitions_for(build_mode) if protocol_version == 2 else DEFINITIONS
    require(isinstance(rows, list) and len(rows) == len(definitions), "incomplete workload manifest")
    frozen = []
    for row, expected in zip(rows, definitions.values()):
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
            "workloads": frozen, **mode_fields}


def observations(value, manifest, workload, smoke):
    version = manifest.get("format_version", 1)
    normal = version == 2 and manifest["build_mode"] == "normal"
    require(isinstance(value, dict), "invalid observation protocol")
    if version == 2:
        require(mode_identity(value, manifest["build_mode"]) == {
            key: manifest[key] for key in ("format_version", "build_mode", "global_allocator", "unavailable")},
            "observation mode mismatch")
        effective_threads(value)
    require(isinstance(value, dict) and value.get("format_version") == version
            and value.get("kind") == "observations", "invalid observation protocol")
    for key in ("target", "opencv_version", "fixture_sha256"):
        require(value.get(key) == manifest[key], f"observation {key} mismatch")
    if version == 2:
        require(value.get("fixture") == manifest["fixture"], "observation source/fixture declaration changed")
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
    if version == 2:
        require(set(metrics) == set(V2_COMMON_METRICS) | set(NORMAL_UNAVAILABLE_METRICS),
                "v2 observed/unavailable metric key set mismatch")
        require(type(metrics["mapping_owner_live_before_drop"]) is bool, "mapping owner observation missing")
        nonempty(metrics["native_allocation_scope"], "native_allocation_scope")
        for name, reason in (("private_after_bytes", "private_reason"), ("footprint_after_bytes", "footprint_reason")):
            if metrics[name] is None:
                nonempty(metrics[reason], reason)
            else:
                integer(metrics[name], name)
    for reason in ("cpu_reason", "native_resource_reason"):
        if metrics.get(reason) is not None:
            unavailable.append(nonempty(metrics[reason], reason))
    require(metrics.get("peak_rss_scope") == "process-high-water"
            and (normal or metrics.get("peak_heap_scope") == "row-including-setup-and-warmup"),
            "peak measurements must include setup and warmup")
    if normal:
        require(all(name in metrics and metrics[name] is None for name in NORMAL_UNAVAILABLE_METRICS),
                "normal unobserved metrics must be explicit null, never zero")
    else:
        for name in COUNTERS:
            integer(metrics.get(name), name)
        if version == 2:
            for name in ("logical_mapped_bytes", "backend_find_calls", "backend_find_failures", "peak_active_finds"):
                integer(metrics.get(name), name)
            require(metrics["backend_find_failures"] == 0, "backend find failures")
            require(metrics.get("final_owner_correct") is True, "final owner/cache oracle failed")
            require(metrics.get("peak_active_finds_scope") == "process-high-water", "active-find scope mismatch")
    for name in MEMORY:
        if normal and name in NORMAL_UNAVAILABLE_METRICS:
            continue
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
    if not normal:
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


def schedule(mode, selected, protocol_version=1):
    roles = ("baseline", "candidate") if mode == "comparison" else (
        ("baseline", "control") if protocol_version == 2 and mode == "control" else ("baseline",))
    result = [{"role": role, "workload": None, "process": None} for role in roles]
    for workload in selected:
        for process in range(1 if mode == "smoke" else 6 if protocol_version == 2 else 5):
            positions = tuple(reversed(roles)) if protocol_version == 2 and process % 2 else roles
            for position, role in enumerate(positions):
                task = {"role": role, "workload": workload, "process": process}
                if protocol_version == 2:
                    task["position"] = position
                result.append(task)
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
    version = plan.get("format_version")
    require(version in (1, 2) and plan.get("kind") == "campaign", "invalid campaign")
    modes = ("calibration", "comparison", "smoke", "control") if version == 2 else ("calibration", "comparison", "smoke")
    require(plan.get("mode") in modes, "invalid cohort")
    require(plan.get("policy") == (V2_POLICY if version == 2 else POLICY),
            "policy differs from predeclared collector policy")
    if version == 2:
        require(plan.get("execution_limits") == execution_limits(plan["mode"], len(plan.get("schedule", []))),
                "bounded v2 execution policy mismatch")
    elif plan["mode"] == "smoke":
        require(plan.get("execution_limits") == SMOKE_LIMITS, "bounded smoke execution policy mismatch")
    target = plan.get("target")
    require(target in TARGETS and plan.get("host", {}).get("target") == target, "host/target mismatch")
    require(plan.get("collector") == collector_identity(), "collector source differs; use the retained collector revision")
    selected = plan.get("workloads")
    require(isinstance(selected, list) and selected, "missing workload selection")
    definitions = definitions_for(plan.get("build_mode")) if version == 2 else DEFINITIONS
    require(selected == list(definitions) if plan["mode"] != "smoke" else
            len(selected) == len(set(selected)) and all(row in definitions for row in selected),
            "qualification workload coverage mismatch")
    require(plan.get("schedule") == schedule(plan["mode"], selected, version), "cohort order/count mismatch")
    require(plan.get("exclusive_host") is True or plan["mode"] == "smoke", "exclusive host declaration required")
    roles = ("baseline", "candidate") if plan["mode"] == "comparison" else (
        ("baseline", "control") if plan["mode"] == "control" else ("baseline",))
    require(set(plan.get("binaries", {})) == set(roles), "missing role provenance")
    for identity in plan["binaries"].values():
        validate_build(identity.get("build"), target)
        require(identity.get("sha256") == identity["build"]["binary_sha256"], "binary/source binding mismatch")
        digest(identity.get("metadata_sha256"), "metadata_sha256")
        native_path(identity.get("path"), target)
        if version == 2:
            validate_v2_build(identity, target, plan["build_mode"])
    require(isinstance(plan.get("environment"), dict), "environment missing")
    if version == 2 and plan["mode"] == "control":
        require(plan["binaries"]["baseline"] == plan["binaries"]["control"],
                "A/A control must use the unchanged identical baseline executable/source")
    if version == 2 and plan["mode"] != "smoke":
        review = plan.get("method_review", {})
        require(reviewed_method(review.get("path"), review.get("sha256"), plan) == review,
                "source/host/cost review binding mismatch")


def evaluate(directory):
    directory = Path(directory).resolve(strict=True)
    plan = load(directory / "plan.json")
    validate_plan(plan)
    version = plan["format_version"]
    manifests, observations_by_role, attempts, issues = {}, {}, [], []
    runtimes, prerequisite_issues = {}, []
    if version == 2 and plan["mode"] != "smoke":
        for role, binary in plan["binaries"].items():
            try:
                runtime = runtime_closure_admission(binary, plan["target"], plan["build_mode"],
                                                    directory / f"runtime-{role}.json")
                require(runtime["plan"]["host"] == plan["host"]
                        and runtime["plan"]["environment"] == plan["environment"],
                        "runtime preflight host/environment changed")
                runtimes[role] = runtime
            except (OSError, ValueError, KeyError, TypeError) as error:
                prerequisite_issues.append({"attempt": role, "status": "incomplete", "reason": str(error)})
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
            argv = command(binary, task, plan["mode"], version)
            require(attempt.get("argv") == argv, "attempt argv mismatch")
            if version == 2:
                started_path = directory / "started" / filename
                receipt = load(started_path)
                retained["started_sha256"] = sha256(started_path)
                require(all(receipt.get(key) == attempt.get(key) for key in
                            ("task", "argv", "plan_sha256", "started_unix_ns", "campaign_elapsed_before")),
                        "started receipt differs from completed attempt")
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
            if version == 2 or plan["mode"] == "smoke":
                limits = plan["execution_limits"]
                timeout = attempt.get("timeout_seconds")
                require(type(timeout) in (int, float) and math.isfinite(timeout)
                        and 0 < timeout <= limits["child_timeout_seconds"],
                        "child deadline missing or relaxed")
                if version == 2:
                    require(attempt.get("output_limit_bytes") == limits["output_limit_bytes"]
                            and attempt.get("cleanup_seconds") == limits["cleanup_seconds"]
                            and attempt.get("runtime_binding_seconds") == limits["runtime_binding_seconds"],
                            "child output/cleanup/runtime bounds differ from plan")
                    # UTF-8 replacement can expand retained raw bytes by up to 3x.
                    minimum_bytes = sum(len(record[key].encode("utf-8")) - 2 * record[key].count("\ufffd")
                                        for key in ("stdout", "stderr"))
                    require(minimum_bytes <= limits["output_limit_bytes"], "retained output exceeds child bound")
                    before, after = attempt.get("campaign_elapsed_before"), attempt.get("campaign_elapsed_after")
                    require(all(type(item) in (int, float) and math.isfinite(item) for item in (before, after))
                            and 0 <= before <= after <= limits["campaign_timeout_seconds"],
                            "campaign deadline missing or exceeded")
                    require(timeout + limits["cleanup_seconds"] + 2 * limits["runtime_binding_seconds"]
                            <= limits["campaign_timeout_seconds"] - before,
                            "child and runtime binding exceed remaining independent campaign deadline")
            value = strict_json(record["stdout"])
            loaded_runtime(value, binary)
            if task["role"] in runtimes:
                validate_runtime_attempt(attempt, runtimes[task["role"]])
            if task["workload"] is None:
                manifests[task["role"]] = manifest_identity(value, plan["target"], binary["build"]["opencv_version"],
                                                           version, plan.get("build_mode"))
                if version == 2:
                    require(manifests[task["role"]] == binary["build"]["benchmark_manifest"],
                            "observed manifest differs from source/mode attestation")
            else:
                require(task["role"] in manifests, "role has no valid manifest")
                summary = observations(value, manifests[task["role"]], task["workload"], plan["mode"] == "smoke")
                summary["process"] = task["process"]
                if version == 2:
                    summary["position"] = task["position"]
                observations_by_role.setdefault(task["role"], {}).setdefault(task["workload"], []).append(summary)
                if plan["mode"] != "smoke" and summary["unavailable"]:
                    issues.append({"attempt": label, "file": relative, "status": "incomplete",
                                   "reason": "unavailable observations: " + ", ".join(summary["unavailable"])})
        except (OSError, ValueError, TypeError, KeyError, StopIteration) as error:
            issues.append({"attempt": label, "file": relative, "status": "failed",
                           "reason": f"{relative}: {type(error).__name__}: {error}"})
    for role in ("candidate", "control"):
        if role in manifests and manifests.get("baseline") != manifests[role]:
            issues.append({"attempt": "manifests", "status": "failed",
                           "reason": "role fixture/workload/dependency identity mismatch"})
    return {"plan": plan, "plan_sha256": plan_hash, "attempts": attempts,
            "manifest": manifests.get("baseline"), "observations": observations_by_role, "issues": issues,
            **({"prerequisite_issues": prerequisite_issues} if version == 2 else {})}


def derive_envelopes(evidence):
    require(evidence["plan"]["mode"] == "calibration", "not a baseline-only calibration")
    require(not evidence["issues"], "calibration has failed, unavailable, or unrun attempts")
    version = evidence["plan"]["format_version"]
    count = 6 if version == 2 else 5
    definitions = definitions_for(evidence["plan"]["build_mode"]) if version == 2 else DEFINITIONS
    result = {}
    for workload, definition in definitions.items():
        rows = evidence["observations"]["baseline"][workload]
        require(len(rows) == count and (version == 1 or
                sorted(row["process"] for row in rows) == list(range(count))),
                "calibration requires five fresh processes" if version == 1 else "calibration requires 6 fresh processes")
        limits, allowances = {}, {}
        if definition["statistical"]:
            for metric, clock in (("p95_ns", "timer_resolution_ns"), ("cpu_ns", "cpu_timer_resolution_ns")):
                values = [row[metric] for row in rows]
                require(all(value is not None for value in values)
                        and all(row[clock] is not None for row in rows), f"unavailable calibration {workload}/{metric}")
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
        prefixes = ("rss",) if version == 2 and evidence["plan"]["build_mode"] == "normal" else ("rss", "heap")
        for prefix in prefixes:
            for metric in (f"peak_{prefix}_bytes", f"{prefix}_growth_bytes", f"{prefix}_residual_bytes"):
                values = [row[metric] for row in rows]
                require(all(value is not None for value in values), f"unavailable calibration {workload}/{metric}")
                slack = max(max(values) - min(values), POLICY[f"{prefix}_slack_bytes"])
                # OpenCV payload is native: no allowance for Rust heap, growth or residual.
                live = POLICY["live_cache_bytes"] if metric == "peak_rss_bytes" and definition["category"] != "selection" else 0
                limits[metric] = max(values) + slack + live
        result[workload] = {"limits": limits, "benefit_allowances": allowances}
    return result


def cohort_binding(evidence):
    return {"plan_sha256": evidence["plan_sha256"], "attempts": evidence["attempts"]}


def same_v2_context(evidence, calibration):
    plan, base = evidence["plan"], calibration["plan"]
    require(plan.get("format_version") == base.get("format_version") == 2, "mixed protocol cohorts")
    for key in ("target", "build_mode", "collector", "host", "environment", "policy", "workloads", "method_review"):
        require(plan.get(key) == base.get(key), f"cohort {key} differs from calibration")
    require(plan["binaries"]["baseline"] == base["binaries"]["baseline"], "cohort baseline/source changed")
    require(evidence["manifest"] == calibration["manifest"], "cohort manifest/corpus changed")
    require(evidence["plan_sha256"] != calibration["plan_sha256"], "calibration/control cohorts must be independent")
    prior = {item["sha256"] for item in calibration["attempts"]}
    require(not prior.intersection(item["sha256"] for item in evidence["attempts"]),
            "shared raw calibration/control attempts")


def control_evidence(calibration, control):
    require(control["plan"]["mode"] == "control", "not an independent A/A control cohort")
    same_v2_context(control, calibration)
    binding = control["plan"].get("calibration")
    require(isinstance(binding, dict) and all(
        binding.get(key) == value for key, value in cohort_binding(calibration).items()),
        "control calibration receipts changed")
    require(control["plan"]["binaries"]["control"] == control["plan"]["binaries"]["baseline"],
            "A/A control executable/source changed")
    envelopes = derive_envelopes(calibration)
    issues = (list(calibration.get("prerequisite_issues", []))
              + list(control["issues"]) + control.get("prerequisite_issues", []))
    for workload, envelope in envelopes.items():
        for role in ("baseline", "control"):
            rows = control["observations"].get(role, {}).get(workload, [])
            complete = (len(rows) == 6 and sorted(row["process"] for row in rows) == list(range(6))
                        and all(row.get("position") == (row["process"] % 2 if role == "baseline"
                                                       else 1 - row["process"] % 2) for row in rows))
            if not complete or any(row["unavailable"] for row in rows):
                issues.append({"attempt": f"{role}:{workload}", "status": "incomplete",
                               "reason": "six independent complete A/A processes per role required"})
            for row in rows:
                for metric, limit in envelope["limits"].items():
                    value = row[metric]
                    if value is not None and value > limit:
                        issues.append({"attempt": f"{role}:{workload}:{row['process']}", "status": "failed",
                                       "reason": f"{metric}={value} exceeds {limit}"})
            for metric, allowance in envelope["benefit_allowances"].items():
                clock = "timer_resolution_ns" if metric == "p95_ns" else "cpu_timer_resolution_ns"
                if not complete or any(row[metric] is None or row[clock] is None for row in rows):
                    continue
                values = [row[metric] for row in rows]
                resolution = max(row[clock] for row in rows)
                if metric == "cpu_ns":
                    resolution = max(resolution, POLICY["cpu_accounting_floor_ns"][control["plan"]["target"]])
                checks = {
                    "noise": (max(values) - min(values)) / max(statistics.median(values), 2 * resolution)
                             <= V2_POLICY["noise_cap"],
                    "order": abs(statistics.median(row[metric] for row in rows if row["position"] == 0)
                                 - statistics.median(row[metric] for row in rows if row["position"] == 1)) <= allowance,
                    "drift": abs(statistics.median(row[metric] for row in rows if row["process"] < 3)
                                 - statistics.median(row[metric] for row in rows if row["process"] >= 3)) <= allowance,
                }
                for name, passed in checks.items():
                    if not passed:
                        issues.append({"attempt": f"{role}:{workload}", "status": "failed",
                                       "reason": f"A/A {metric} {name} exceeds frozen control tolerance"})
    return issues


def profile_from_v2(evidence, calibration, control, control_path, reviewer, rationale,
                    profile_review, profile_review_sha256):
    issues = control_evidence(evidence, control)
    require(not issues, "independent A/A control failed or incomplete; profile/comparison denied")
    envelopes = derive_envelopes(evidence)
    plan = evidence["plan"]
    digest(profile_review_sha256, "profile_review_sha256")
    require(profile_review is not None and sha256(profile_review) == profile_review_sha256, "profile review hash mismatch")
    review = load(profile_review)
    expected_review = {"format_version": 2, "kind": "component-profile-review", "decision": "accepted",
                       "target": plan["target"], "build_mode": plan["build_mode"],
                       "method_review": plan["method_review"], "calibration": cohort_binding(evidence),
                       "control": cohort_binding(control), "envelopes": envelopes}
    require(isinstance(review, dict) and all(review.get(key) == value for key, value in expected_review.items()),
            "external profile review does not bind the exact method/cohorts/envelopes")
    nonempty(review.get("reviewer"), "profile review reviewer")
    nonempty(review.get("rationale"), "profile review rationale")
    for role, binary in plan["binaries"].items():
        runtime_closure_admission(binary, plan["target"], plan["build_mode"],
                                  Path(calibration) / f"runtime-{role}.json")
    return {"format_version": 2, "kind": "frozen-profile",
            **{key: plan[key] for key in ("target", "build_mode", "policy", "collector", "host", "environment", "method_review")},
            "baseline": plan["binaries"]["baseline"], "manifest": evidence["manifest"], "envelopes": envelopes,
            "calibration": {"path": str(Path(calibration).resolve()), **cohort_binding(evidence)},
            "control": {"path": str(Path(control_path).resolve()), **cohort_binding(control)},
            "profile_review": {"path": str(Path(profile_review).resolve()), "sha256": profile_review_sha256},
            "reviewer": nonempty(reviewer, "reviewer"), "rationale": nonempty(rationale, "rationale")}


def profile_from(evidence, calibration, reviewer, rationale):
    require(evidence["plan"]["format_version"] == 1, "v2 profile requires independent control and external profile review")
    return {"format_version": 1, "kind": "frozen-profile", "target": evidence["plan"]["target"],
            "policy": POLICY, "collector": evidence["plan"]["collector"], "host": evidence["plan"]["host"],
            "environment": evidence["plan"]["environment"], "baseline": evidence["plan"]["binaries"]["baseline"],
            "manifest": evidence["manifest"], "envelopes": derive_envelopes(evidence),
            "calibration": {"path": str(Path(calibration).resolve()), "plan_sha256": evidence["plan_sha256"],
                            "attempts": evidence["attempts"]},
            "reviewer": nonempty(reviewer, "reviewer"), "rationale": nonempty(rationale, "rationale")}


def validate_profile(profile):
    require(profile.get("format_version") in (1, 2) and profile.get("kind") == "frozen-profile", "invalid profile")
    calibration = profile.get("calibration", {})
    evidence = evaluate(calibration["path"])
    if profile["format_version"] == 2:
        control = profile.get("control", {})
        review = profile.get("profile_review", {})
        expected = profile_from_v2(evidence, calibration["path"], evaluate(control["path"]), control["path"],
                                   profile.get("reviewer"), profile.get("rationale"),
                                   review.get("path"), review.get("sha256"))
    else:
        expected = profile_from(evidence, calibration["path"], profile.get("reviewer"), profile.get("rationale"))
    require(profile == expected, "profile/provenance/envelopes differ from frozen calibration")
    return profile


def compare_evidence(evidence, profile):
    plan = evidence["plan"]
    require(plan["mode"] == "comparison", "not a comparison cohort")
    version = plan["format_version"]
    require(version == profile["format_version"], "mixed profile protocol")
    count = 6 if version == 2 else 5
    normal = version == 2 and plan["build_mode"] == "normal"
    if version == 2:
        for name in ("build_mode", "policy", "method_review"):
            require(plan.get(name) == profile.get(name), f"comparison {name} differs from calibration")
        require(plan["profile_sha256"] is not None, "comparison requires pinned control-accepted profile")
    for name in ("target", "collector", "host", "environment"):
        require(plan[name] == profile[name], f"comparison {name} differs from calibration")
    require(plan["binaries"]["baseline"] == profile["baseline"], "comparison baseline source/binary changed")
    if evidence["manifest"] is not None:
        require(evidence["manifest"] == profile["manifest"], "comparison fixture/manifest changed")
    issues = list(evidence["issues"]) + evidence.get("prerequisite_issues", [])
    benefits = {}
    definitions = definitions_for(plan["build_mode"]) if version == 2 else DEFINITIONS
    for workload, definition in definitions.items():
        cohorts = {role: evidence["observations"].get(role, {}).get(workload, [])
                   for role in ("baseline", "candidate")}
        complete = all(len(rows) == count and all(not row["unavailable"] for row in rows)
                       and (version == 1 or sorted(row["process"] for row in rows) == list(range(count)))
                       for rows in cohorts.values())
        envelope = profile["envelopes"][workload]
        for role, rows in cohorts.items():
            for row in rows:
                process = row["process"]
                for metric, limit in envelope["limits"].items():
                    if row[metric] is not None and row[metric] > limit:
                        issues.append({"attempt": f"{role}:{workload}:{process}", "status": "failed",
                                       "reason": f"{metric}={row[metric]} exceeds {limit}"})
                if role == "candidate" and not normal:
                    metrics = row["metrics"]
                    failures = []
                    if metrics["score_copied_bytes"] != 0:
                        failures.append("dense score copy remains")
                    if workload.startswith("batch8-cold-") and metrics["bgr_conversions"] != metrics["iterations"]:
                        failures.append("cold batch must convert once per iteration")
                    if workload == "batch8-live-owner-roi320x180" and metrics["mapping_copies"] != metrics["iterations"]:
                        failures.append("live mapping owner must avoid duplicate ROI copies")
                    if len(cohorts["baseline"]) == count and definition["category"] == "selection" and definition["improvement"]:
                        reference = min(item["metrics"]["score_offsets_scanned"] for item in cohorts["baseline"])
                        if metrics["score_offsets_scanned"] >= reference:
                            failures.append("selection offsets did not decrease")
                    for reason in failures:
                        issues.append({"attempt": f"{role}:{workload}:{process}", "status": "failed", "reason": reason})
        if not complete:
            issues.append({"attempt": workload, "status": "incomplete",
                           "reason": f"{'six' if version == 2 else 'five'} complete processes per role required"})
        if definition["improvement"] and (version == 1 or normal) and all(len(rows) == count for rows in cohorts.values()):
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
    issues, benefits = list(evidence["issues"]) + evidence.get("prerequisite_issues", []), {}
    version = evidence["plan"]["format_version"]
    if version == 2 and mode == "control":
        try:
            binding = evidence["plan"]["calibration"]
            calibration = evaluate(binding["path"])
            issues = control_evidence(calibration, evidence)
        except (OSError, ValueError, KeyError, TypeError) as error:
            issues.append({"attempt": "control", "status": "failed", "reason": str(error)})
    if mode == "calibration" and not evidence["issues"]:
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
    qualified = mode == "comparison" and status == "passed" and (
        version == 1 or evidence["plan"]["build_mode"] == "normal")
    return {"format_version": version, "mode": mode, "target": target, "status": status,
            "performance_qualified": qualified, "whole_change_acceptance": "incomplete",
            "targets": {name: (status if mode == "comparison" else "incomplete") if name == target else "not-run"
                        for name in TARGETS},
            "plan_sha256": evidence["plan_sha256"], "issues": issues, "benefits": benefits,
            "attempts": evidence["attempts"],
            "process_observations": evidence["observations"],
            **({"build_mode": evidence["plan"]["build_mode"], "lane": "component",
                "normal_timing_verdict": ("unavailable" if evidence["plan"]["build_mode"] == "instrumented"
                                          else status if mode == "comparison" else "not-qualified"),
                "heap_and_work_verdict": ("unavailable" if evidence["plan"]["build_mode"] == "normal"
                                          else status if mode == "comparison" else "descriptive"),
                "actual_consumer_coverage": "not-implemented"} if version == 2 else {}),
            "note": "smoke and calibration do not establish performance qualification; contract rows are non-statistical"}


def command(binary, task, mode, protocol_version=1):
    prefix = [binary["path"]] + (["--protocol-version", "2"] if protocol_version == 2 else [])
    if task["workload"] is None:
        return prefix + ["--list"]
    result = prefix + ["--workload", task["workload"]]
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


def runtime_preflight(args):
    """Discover every selected component path without collecting acceptance samples."""
    require(args.protocol_version == 2 and args.target == TARGETS[0]
            and host_identity()["target"] == args.target,
            "runtime preflight requires protocol 2 on native Apple Silicon; Windows remains incomplete")
    identity = binary_identity(args.binary, args.build_metadata, args.target)
    require("runtime_image_preflight" not in identity["build"], "preflight requires the original build attestation")
    manifest = validate_v2_build(identity, args.target, args.build_mode)
    threads = effective_threads(strict_json(identity["build"]["provenance_commands"][0]["record"]["stdout"]))
    tasks = schedule("smoke", list(definitions_for(args.build_mode)), 2)
    limits = {**execution_limits("smoke", len(tasks)), "runtime_binding_seconds": 120}
    plan = {"target": args.target, "build_mode": args.build_mode, "binary": identity,
            "host": host_identity(), "environment": environment(), "collector": collector_identity(),
            "observer": macos_images.method_identity(), "schedule": tasks, "execution_limits": limits}
    destination = Path(args.output).resolve()
    with exclusive_collector():
        destination.mkdir(parents=True, exist_ok=False)
        (destination / "attempts").mkdir()
        (destination / "started").mkdir()
        save(destination / "plan.json", plan)
        started, attempts, issues, retained_bytes = time.monotonic(), [], [], 0
        deadline = started + limits["campaign_timeout_seconds"]
        for task in tasks:
            before = time.monotonic() - started
            timeout = min(limits["child_timeout_seconds"],
                          deadline - time.monotonic() - limits["cleanup_seconds"] - limits["runtime_binding_seconds"])
            if timeout <= 0:
                issues.append("campaign deadline reached; remaining representative paths not-run")
                break
            argv = command(identity, task, "smoke", 2)
            attempt = {"task": task, "argv": argv, "identity_before": None, "identity_after": None,
                       "identity_error": None, "receipt": None, "campaign_elapsed_before": before,
                       "timeout_seconds": timeout}
            save(destination / "started" / f"{task['id']}.json", attempt)
            try:
                check_files(identity)
                attempt["identity_before"] = identity
                timeout = min(timeout, deadline - time.monotonic()
                              - limits["cleanup_seconds"] - limits["runtime_binding_seconds"])
                require(timeout > 0, "preflight deadline reached before child launch")
                attempt["timeout_seconds"] = timeout
                receipt = macos_images.discover(
                    argv, cwd=ROOT, env=plan["environment"], timeout_seconds=timeout,
                    output_limit_bytes=limits["output_limit_bytes"], cleanup_seconds=limits["cleanup_seconds"],
                    binding_seconds=limits["runtime_binding_seconds"])
                attempt["receipt"] = receipt
                check_files(identity)
                attempt["identity_after"] = identity
                require(successful(receipt["record"]), "preflight process or owned cleanup failed")
                closed = macos_images.validate_closure(receipt)
                require(closed["ok"], "preflight image binding failed: " + str(closed["reason"]))
                value = strict_json(receipt["record"]["stdout"])
                loaded_runtime(value, identity, verify_files=True)
                require(effective_threads(value) == threads, "preflight native thread setting changed")
                if task["workload"] is None:
                    require(manifest_identity(value, args.target, identity["build"]["opencv_version"],
                                              2, args.build_mode) == manifest, "preflight manifest changed")
                else:
                    observations(value, manifest, task["workload"], True)
            except (OSError, ValueError, KeyError, TypeError) as error:
                attempt["identity_error"] = str(error)
                issues.append(str(error))
            except BaseException as error:
                attempt["interruption"] = type(error).__name__
                attempt["campaign_elapsed_after"] = time.monotonic() - started
                try:
                    save(destination / "attempts" / f"{task['id']}.json", attempt)
                except (OSError, ValueError):
                    pass  # Preserve the primary interruption after best-effort receipt retention.
                raise
            attempt["campaign_elapsed_after"] = time.monotonic() - started
            save(destination / "attempts" / f"{task['id']}.json", attempt)
            attempts.append(attempt)
            retained_bytes += len(document(attempt))
            if retained_bytes > 64 * MIB:
                issues.append("runtime preflight bundle exceeds 64 MiB")
            if issues:
                break
        bundle = {"format_version": 2, "kind": "component-runtime-preflight", "plan": plan,
                  "attempts": attempts, "threads": threads, "closure": None}
        if not issues:
            derived = macos_images.validate_closure([attempt["receipt"] for attempt in attempts])
            bundle["closure"] = derived["closure"]
            try:
                validate_runtime_bundle(bundle, identity, args.target, args.build_mode)
                require(len(document(bundle)) <= 64 * MIB, "runtime preflight bundle exceeds 64 MiB")
            except (ValueError, KeyError, TypeError) as error:
                issues.append(str(error))
        result = {"status": "incomplete" if issues else "passed", "issues": issues,
                  "attempts": len(attempts), "scheduled": len(tasks), "performance_qualified": False}
        if not issues:
            save(destination / "bundle.json", bundle)
            build = {**identity["build"], "runtime_image_preflight": {
                "path": str(destination / "bundle.json"), "sha256": sha256(destination / "bundle.json")}}
            save(destination / "build.json", build)
            result.update(metadata=str(destination / "build.json"), sha256=sha256(destination / "build.json"))
        save(destination / "report.json", result)
        return result


def collect(args, mode):
    version = args.protocol_version
    definitions = definitions_for(args.build_mode) if version == 2 else DEFINITIONS
    require(version == 2 or mode != "control", "control requires --protocol-version 2")
    host = host_identity()
    require(host["target"] == args.target, "actual host cannot execute the selected qualification target")
    baseline = binary_identity(args.baseline, args.baseline_metadata, args.target)
    binaries = {"baseline": baseline}
    runtimes = {}
    if version == 2:
        validate_v2_build(baseline, args.target, args.build_mode)
    if mode == "control":
        binaries["control"] = baseline
    profile, profile_bytes = None, None
    if mode == "comparison":
        profile_path = Path(args.profile).resolve(strict=True)
        digest(args.profile_sha256, "profile_sha256")
        require(sha256(profile_path) == args.profile_sha256, "reviewed profile hash mismatch")
        profile_bytes = profile_path.read_bytes()
        profile = validate_profile(strict_json(profile_bytes.decode("utf-8")))
        require(profile["format_version"] == version, "mixed profile protocol")
        require(host == profile["host"] and environment() == profile["environment"], "host/environment differs from reviewed profile")
        require(baseline == profile["baseline"], "baseline differs from reviewed profile")
        require(collector_identity() == profile["collector"], "collector differs from reviewed profile")
        binaries["candidate"] = binary_identity(args.candidate, args.candidate_metadata, args.target)
    selected = [args.workload] if mode == "smoke" and args.workload else list(definitions)
    require(all(workload in definitions for workload in selected), "workload unavailable in selected build mode")
    preflight = {"target": args.target, "build_mode": args.build_mode, "host": host,
                 "collector": collector_identity(), "environment": environment(), "policy": V2_POLICY,
                 "workloads": selected, "binaries": binaries}
    method, calibration_binding = None, None
    if version == 2:
        for binary in binaries.values():
            validate_v2_build(binary, args.target, args.build_mode)
        if mode != "smoke":
            method = reviewed_method(args.method_review, args.method_review_sha256, preflight)
            preflight["method_review"] = method
            if mode == "control":
                require(args.calibration is not None, "control requires immutable calibration")
                calibration = evaluate(args.calibration)
                require(calibration["plan_sha256"] == digest(args.calibration_plan_sha256, "calibration_plan_sha256"),
                        "control calibration plan hash mismatch")
                require(not calibration.get("prerequisite_issues"),
                        "calibration runtime prerequisites are incomplete; control not started")
                derive_envelopes(calibration)
                for key in ("target", "build_mode", "host", "collector", "environment", "policy", "workloads", "method_review"):
                    require(preflight[key] == calibration["plan"][key], f"control {key} differs from calibration")
                require(baseline == calibration["plan"]["binaries"]["baseline"], "control baseline/source changed")
                calibration_binding = {"path": str(Path(args.calibration).resolve()), **cohort_binding(calibration)}
            if profile:
                require(args.build_mode == profile["build_mode"] and method == profile["method_review"],
                        "comparison build mode/method review changed")
            for role, binary in binaries.items():
                runtime = runtime_closure_admission(binary, args.target, args.build_mode)
                require(runtime["plan"]["host"] == host and runtime["plan"]["environment"] == environment(),
                        "runtime preflight host/environment changed")
                runtimes[role] = runtime
    destination = Path(args.output).resolve()
    if profile:
        calibration = Path(profile["calibration"]["path"]).resolve()
        require(destination != calibration and calibration not in destination.parents,
                "comparison must not write into calibration")
        if version == 2:
            control_path = Path(profile["control"]["path"]).resolve()
            require(destination != control_path and control_path not in destination.parents,
                    "comparison must not write into control")
    if calibration_binding:
        calibration_path = Path(calibration_binding["path"])
        require(destination != calibration_path and calibration_path not in destination.parents,
                "control must not write into calibration")
    with exclusive_collector():
        destination.mkdir(parents=True, exist_ok=False)
        (destination / "attempts").mkdir()
        (destination / "started").mkdir()
        if profile_bytes is not None:
            with (destination / "profile.json").open("xb") as stream:
                stream.write(profile_bytes)
        plan = {"format_version": version, "kind": "campaign", "mode": mode, "target": args.target,
                "host": host, "collector": collector_identity(), "binaries": binaries,
                "environment": environment(), "policy": V2_POLICY if version == 2 else POLICY, "workloads": selected,
                "schedule": schedule(mode, selected, version), "exclusive_host": args.exclusive_host,
                "cwd": str(ROOT), "created_unix_ns": time.time_ns(),
                "profile_sha256": args.profile_sha256 if profile else None}
        if version == 2:
            plan.update(build_mode=args.build_mode, method_review=method, calibration=calibration_binding,
                        execution_limits=execution_limits(mode, len(plan["schedule"])))
        elif mode == "smoke":
            plan["execution_limits"] = SMOKE_LIMITS
        for role, runtime in runtimes.items():
            with (destination / f"runtime-{role}.json").open("xb") as stream:
                stream.write(runtime["raw"])
        save(destination / "plan.json", plan)
        plan_hash = sha256(destination / "plan.json")
        limits = plan.get("execution_limits")
        campaign_started = time.monotonic() if limits is not None else None
        deadline = campaign_started + limits["campaign_timeout_seconds"] if limits is not None else None
        binding_seconds = limits.get("runtime_binding_seconds", 0) if limits else 0
        for task in plan["schedule"]:
            remaining = None if deadline is None else deadline - time.monotonic() - limits["cleanup_seconds"]
            if remaining is not None and remaining <= 2 * binding_seconds:
                break
            identity = binaries[task["role"]]
            argv = command(identity, task, mode, version)
            attempt = {"task": task, "argv": argv, "plan_sha256": plan_hash,
                       "identity_before": None, "identity_after": None, "identity_error": None,
                       "record": None, "started_unix_ns": time.time_ns()}
            if version == 2:
                attempt["campaign_elapsed_before"] = limits["campaign_timeout_seconds"] - remaining - limits["cleanup_seconds"]
                attempt.update(output_limit_bytes=limits["output_limit_bytes"], cleanup_seconds=limits["cleanup_seconds"],
                               runtime_binding_seconds=binding_seconds)
            save(destination / "started" / f"{task['id']}.json", attempt)
            try:
                check_files(identity)
                attempt["identity_before"] = identity
                if deadline is not None:
                    remaining = deadline - time.monotonic() - limits["cleanup_seconds"] - 2 * binding_seconds
                    require(remaining > 0, "campaign deadline reached before child launch or runtime binding")
                timeout = POLICY["timeout_seconds"] if remaining is None else min(limits["child_timeout_seconds"], remaining)
                attempt["timeout_seconds"] = timeout
                if task["role"] in runtimes:
                    receipt = macos_images.observe(
                        argv, cwd=ROOT, env=plan["environment"], closure=runtimes[task["role"]]["closure"],
                        timeout_seconds=timeout, output_limit_bytes=limits["output_limit_bytes"],
                        cleanup_seconds=limits["cleanup_seconds"], binding_seconds=binding_seconds)
                    attempt["record"] = receipt["record"]
                    attempt["runtime_observation"] = {key: value for key, value in receipt.items() if key != "record"}
                    validate_runtime_attempt(attempt, runtimes[task["role"]])
                else:
                    attempt["record"] = run_process(argv, cwd=ROOT, env=plan["environment"],
                                                     timeout_seconds=timeout,
                                                     output_limit_bytes=POLICY["output_limit_bytes"],
                                                     cleanup_seconds=POLICY["cleanup_seconds"])
                check_files(identity)
                attempt["identity_after"] = identity
                if successful(attempt["record"]):
                    loaded_runtime(strict_json(attempt["record"]["stdout"]), identity, verify_files=True)
                    if version == 2:
                        value = strict_json(attempt["record"]["stdout"])
                        if task["workload"] is None:
                            require(manifest_identity(value, args.target, identity["build"]["opencv_version"],
                                                      2, args.build_mode) == identity["build"]["benchmark_manifest"],
                                    "observed source/mode manifest changed")
                        else:
                            observations(value, identity["build"]["benchmark_manifest"], task["workload"], mode == "smoke")
            except (OSError, ValueError) as error:
                attempt["identity_error"] = str(error)
            except BaseException as error:
                attempt["interruption"] = type(error).__name__
                if version == 2:
                    attempt["campaign_elapsed_after"] = time.monotonic() - campaign_started
                try:
                    save(destination / "attempts" / f"{task['id']}.json", attempt)
                except BaseException:
                    # Evidence persistence must not replace the original interruption.
                    pass
                raise
            if version == 2:
                attempt["campaign_elapsed_after"] = time.monotonic() - campaign_started
            save(destination / "attempts" / f"{task['id']}.json", attempt)
            # A failed row is never retried or omitted. Unsafe cleanup stops remaining
            # processes, which stay visibly not-run in the immutable schedule.
            if attempt["record"] is None or not attempt["record"]["cleanup_ok"] or attempt["identity_error"]:
                break
            if task["workload"] is None:
                try:
                    require(successful(attempt["record"]), "manifest process failed")
                    manifest = manifest_identity(strict_json(attempt["record"]["stdout"]), args.target,
                                                 identity["build"]["opencv_version"], version, args.build_mode)
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
    version = args.protocol_version
    if version == 2:
        definitions_for(args.build_mode)
    require(not Path(args.output).exists() and not Path(str(args.output) + ".probe.json").exists(),
            "attestation destination already exists")
    require(host_identity()["target"] == args.target, "attestation must run on the actual native target")
    binary = Path(args.binary).resolve(strict=True)
    binary_hash = sha256(binary)
    git, rustc = shutil.which("git"), shutil.which("rustc")
    require(git is not None and rustc is not None, "attestation requires git and rustc on PATH")
    git, rustc = str(Path(git).absolute()), str(Path(rustc).absolute())
    commands = []
    probe_schedule = (command({"path": str(binary)}, {"workload": None}, "smoke", version),
                      [git, "rev-parse", "HEAD"], [git, "rev-parse", "HEAD^{tree}"],
                      [git, "status", "--porcelain"], [rustc, "-vV"])
    deadline = time.monotonic() + SMOKE_LIMITS["campaign_timeout_seconds"] if version == 2 else None
    value = runtime = manifest = None
    try:
        for argv in probe_schedule:
            timeout = SMOKE_LIMITS["child_timeout_seconds"]
            if deadline is not None:
                timeout = min(timeout, deadline - time.monotonic() - SMOKE_LIMITS["cleanup_seconds"])
                require(timeout > 0, "attestation campaign deadline reached; remaining probe slots not-run")
            attempt = {"argv": argv, "record": None}
            if version == 2:
                commands.append(attempt)
            record = run_process(argv, cwd=ROOT, env=environment(),
                                 timeout_seconds=timeout,
                                 output_limit_bytes=SMOKE_LIMITS["output_limit_bytes"],
                                 cleanup_seconds=SMOKE_LIMITS["cleanup_seconds"])
            attempt["record"] = record
            if version == 1:
                commands.append(attempt)
            require(successful(record), f"attestation command failed: {argv}")
            if version == 2 and len(commands) == 1:
                value = strict_json(record["stdout"])
                manifest = manifest_identity(value, args.target, value["opencv_version"], version, args.build_mode)
                runtime = loaded_runtime(value, verify_files=True)
                require(sha256(binary) == binary_hash, "binary changed during runtime probe")
    finally:
        save(str(args.output) + ".probe.json", {"commands": commands, **({
            "format_version": 2, "build_mode": args.build_mode, "schedule": list(probe_schedule),
            "execution_limits": execution_limits("smoke", len(probe_schedule))} if version == 2 else {})})
    if version == 1:
        value = strict_json(commands[0]["record"]["stdout"])
        runtime = loaded_runtime(value, verify_files=True)
        manifest = manifest_identity(value, args.target, value["opencv_version"], version, args.build_mode)
    require(sha256(binary) == binary_hash, "binary changed during runtime probe")
    build = {"target": args.target, "source_revision": commands[1]["record"]["stdout"].strip(),
             "source_tree": commands[2]["record"]["stdout"].strip(),
             "source_status_porcelain": commands[3]["record"]["stdout"],
             "binary_sha256": binary_hash, "cargo_lock_sha256": sha256(ROOT / "Cargo.lock"),
             "rustc": commands[4]["record"]["stdout"], "build_argv": strict_json(args.build_argv_json),
             "opencv_version": value["opencv_version"], "dependencies": runtime["images"],
             "loaded_native_runtime": runtime, "provenance_commands": commands,
             "attestation_scope": "operator-supplied build command; source commit/tree and dirty status observed, not a reproducible build proof"}
    if version == 2:
        build.update(protocol_version=2, build_mode=args.build_mode, benchmark_manifest=manifest,
                     runtime_observation_scope="OpenCV endpoint only; not full transitive/short-lived closure; nonqualifying")
    validate_build(build, args.target)
    save(args.output, build)
    return {"metadata": str(Path(args.output).resolve()), "sha256": sha256(args.output)}


def protocol_arguments(sub):
    sub.add_argument("--protocol-version", type=int, choices=(1, 2), default=1)
    sub.add_argument("--build-mode", choices=("normal", "instrumented"),
                     help="required for protocol 2; verified from compiled executable manifest")


def parser():
    result = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Build metadata JSON: target, binary_sha256, source_revision (40 hex), source_tree (40 hex), cargo_lock_sha256, rustc, build_argv (array), opencv_version, dependencies ([{path: absolute, sha256}]). Metadata attests the build; collector verifies the named executable/dependency bytes, never rebuilds or infers source from the current checkout.")
    commands = result.add_subparsers(dest="command", required=True)
    policy = commands.add_parser("policy", help="print the predeclared workload/noise/envelope policy")
    protocol_arguments(policy)
    binding = commands.add_parser("attest", help="observe a prebuilt executable's actual loaded OpenCV paths; never build")
    protocol_arguments(binding)
    binding.add_argument("--target", choices=TARGETS, required=True)
    binding.add_argument("--binary", required=True)
    binding.add_argument("--build-argv-json", required=True, help="exact completed build command as a JSON array")
    binding.add_argument("--output", required=True, help="new build metadata JSON")
    discovery = commands.add_parser("runtime-preflight", help="v2 macOS only: bind all representative image loads; non-acceptance")
    protocol_arguments(discovery)
    discovery.add_argument("--target", choices=TARGETS, required=True)
    discovery.add_argument("--binary", required=True)
    discovery.add_argument("--build-metadata", required=True)
    discovery.add_argument("--output", required=True, help="new evidence directory")
    for name, description in (("calibrate", "baseline-only calibration: v1 five, v2 six processes per row"),
                              ("control", "v2 only: six independent balanced unchanged A/A pairs per row"),
                              ("compare", "profile-bound comparison; v2 uses six balanced pairs per row"),
                              ("smoke", "one short process per row; never performance qualification")):
        sub = commands.add_parser(name, help=description)
        protocol_arguments(sub)
        sub.add_argument("--method-review", help="external prospective source/host/cost review JSON (v2)")
        sub.add_argument("--method-review-sha256", help="pinned method review bytes (v2)")
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
        if name == "control":
            sub.add_argument("--calibration", required=True)
            sub.add_argument("--calibration-plan-sha256", required=True)
        if name == "smoke":
            sub.add_argument("--workload", choices=list(DEFINITIONS))
    freeze = commands.add_parser("freeze", help="review calibration and exclusively write a numeric target profile")
    protocol_arguments(freeze)
    freeze.add_argument("--control", help="separate unchanged control evidence directory (v2)")
    freeze.add_argument("--profile-review", help="external review binding exact method/cohorts/envelopes (v2)")
    freeze.add_argument("--profile-review-sha256", help="pinned external numerical profile review bytes (v2)")
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
            if args.protocol_version == 2:
                definitions = definitions_for(args.build_mode)
                result = {"format_version": 2, "build_mode": args.build_mode, "policy": V2_POLICY,
                          "workloads": list(definitions.values()), "execution_limits": {
                              mode: execution_limits(mode, len(schedule(mode, list(definitions), 2)))
                              for mode in ("smoke", "calibration", "control", "comparison")},
                          "qualification": "requires source-bound runtime closure, observer eligibility review, calibration, control, and paired comparison"}
            else:
                result = {"policy": POLICY, "workloads": list(DEFINITIONS.values()),
                          "smoke_execution_limits": SMOKE_LIMITS, "direct_format_contract": direct_format_contract()}
        elif args.command == "attest":
            result = attest(args)
        elif args.command == "runtime-preflight":
            result = runtime_preflight(args)
        elif args.command == "freeze":
            evidence = evaluate(args.calibration)
            require(evidence["plan"]["format_version"] == args.protocol_version, "freeze protocol mismatch")
            if args.protocol_version == 2:
                require(args.build_mode == evidence["plan"]["build_mode"], "freeze build mode mismatch")
                require(args.control is not None, "v2 freeze requires independent unchanged control")
                result = profile_from_v2(evidence, args.calibration, evaluate(args.control), args.control,
                                         args.reviewer, args.rationale, args.profile_review, args.profile_review_sha256)
            else:
                result = profile_from(evidence, args.calibration, args.reviewer, args.rationale)
            output = Path(args.output).resolve()
            require(Path(args.calibration).resolve() not in output.parents,
                    "profile must be separate from frozen calibration")
            if args.protocol_version == 2:
                require(Path(args.control).resolve() not in output.parents,
                        "profile must be separate from frozen control")
            save(output, result)
            result = {"profile": str(output), "sha256": sha256(output), "target": result["target"]}
        elif args.command == "report":
            result = report(args.evidence)
        else:
            result = collect(args, {"calibrate": "calibration", "control": "control",
                                    "compare": "comparison", "smoke": "smoke"}[args.command])
        print(document(result).decode("utf-8"), end="")
        return 0 if result.get("status", "passed") == "passed" else 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"template-matching-efficiency: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
